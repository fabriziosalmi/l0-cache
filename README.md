# l0-cache

A lightweight CLI proxy written in Rust that reduces LLM token consumption by
filtering, truncating, and compressing command output. Designed for AI coding
assistants (Claude Code, Gemini CLI, Cursor) running on macOS, Linux, and
remote servers.

## The Problem

AI coding assistants read the full output of every shell command. A single
`cargo test` or `git log` can produce thousands of lines, consuming tokens
that add no value. The relevant information is almost always in the first
few lines (headers, command echo) and the last few lines (errors, summary).

## The Solution

`l0-cache` wraps any command and applies a pipeline of universal filters:

```
command output
  --> ANSI escape stripping
  --> line collapsing (identical & prefix-based) (×N)
  --> blank line squeezing
  --> head/tail buffering (30 + 30 lines)
  --> metrics logging
  --> filtered output
```

Typical savings: 50-80% fewer tokens per command invocation.

## Installation

### Quick Install (non-interactive)

Install `l0-cache` locally to `~/.local/bin/` with a single command:

```sh
curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-cache/master/install.sh | bash
```

### Interactive Installer

Clone the repository and run the setup script for a guided interactive install:

```sh
git clone https://github.com/fabriziosalmi/l0-cache.git
cd l0-cache
./install.sh
```

### Manual Build

```sh
cargo build --release
cp target/release/l0-cache /usr/local/bin/
```

### Verify

```sh
l0-cache --version
# l0-cache 0.1.0 (abc1234)
```

## Usage

```sh
# Filtered output (default: 30 head + 30 tail, threshold 100 lines)
l0-cache cargo test

# Full output, metrics still logged
l0-cache --raw cargo test

# Interactive commands pass through unchanged
l0-cache -i vim file.txt

# Token savings report
l0-cache --stats
l0-cache --stats --since 7d

# Custom head/tail
l0-cache --head 50 --tail 50 cargo build

# More tail lines on error (default: 120)
l0-cache --tail-error 200 cargo test

# Auto-tuning is enabled by default. To disable it:
l0-cache --no-auto cargo test

# Diagnose system installation, shell configuration, and active LLM editors
l0-cache --doctor

# Custom success optimization floor and failure backoff ceiling
l0-cache --auto-floor 15 --auto-ceiling 500 cargo test

# Custom token divisor ratio (e.g. 8 bytes per token)
l0-cache --token-factor 8 cargo test
```

## Options

```
--raw                Print output verbatim (no head/tail truncation, no collapsing
                     or JSON squashing); ANSI is still stripped and a 1 MB/line and
                     256 MB total OOM cap still apply. Metrics are still logged.
-i, --interactive    Passthrough mode (stdin/stdout/stderr inherited)
--head N             Lines to keep from start (default: 30)
--tail N             Lines to keep from end (default: 30)
--tail-error N       Tail lines on non-zero exit (default: 120)
--threshold N        Only truncate if output exceeds N lines (default: 100)
--only-errors        Keep only lines matching error/warn/fail/panic/exception/etc.
--idle-timeout N     SIGKILL the command (and its process group) after N seconds with
                     no output (prevents interactive-prompt deadlocks). 0 = off.
--no-auto            Disable adaptive auto-tuning of parameters
--quiet, -q          Suppress l0-cache's own stderr notices (e.g. auto-tuning)
--guard              Force-enable the safety guard (see "Safety Guard" below)
--no-guard           Force-disable the safety guard
--doctor             Diagnose system installation, shell environment, and active LLM editors
--auto-floor N       Floor for success optimization decay (default: 10)
--auto-ceiling N     Ceiling for failure backoff tail expansion (default: 1000)
--token-factor N     Divisor for token estimation (default: 4)
--stats              Show token savings report
--since DURATION     Filter stats (e.g. 7d, 24h, 30m)
--reset-stats        Delete ALL recorded telemetry (destructive, cannot be undone)
--completions SHELL  Generate shell completions (bash, zsh, fish, elvish, powershell)
--version            Print version with git commit hash
```

## Safety Guard

When `l0-cache` detects it is running inside an AI coding assistant (Claude Code,
Gemini CLI, Cursor/VS Code terminals), it enables a **best-effort** guard that
blocks a few obviously destructive commands before they run, exiting with code
**126**:

- recursive force-removal of a critical system path (`rm -rf /`, `/etc`, `/usr`, …),
  including inside `sh -c "…"` wrappers and with trailing-slash/glob variants;
- reverse shells / socket redirections (`/dev/tcp`, `/dev/udp`);
- credential exfiltration (`curl`/`wget`/`nc`/`ssh` touching `id_rsa`, `.env`, `shadow`, …);
- `DROP DATABASE` via `psql`/`mysql`/`sqlite3`/`sqlcmd`.

Control it explicitly with `--guard` / `--no-guard`, or the `L0_CACHE_GUARD`
environment variable (`1`/`true`/`on` to force on, `0`/`false`/`off` to force off).
Precedence: `--no-guard` → `--guard` → `L0_CACHE_GUARD` → auto-detect.

> This is a guard rail, not a sandbox. It pattern-matches argv and shell payloads
> and can be bypassed by a determined caller — do not rely on it as a security
> boundary. Bypass an intentional command with `--no-guard`.

## Architecture

Single-threaded, synchronous design. Zero async. The only thread ever spawned is
an optional output-inactivity watchdog, and only when `--idle-timeout` is set.

```
l0-cache <command>
  |
  +-- sh -c '<command> 2>&1'     # merge stderr into stdout
  |
  +-- read_line_lossy()          # UTF-8 lossy, 1MB line cap
  |
  +-- FilterPipeline             # streaming, O(head+tail) memory
  |     |-- strip_ansi()
  |     |-- CollapseLines
  |     |-- WhitespaceSqueeze
  |     +-- HeadTailBuffer
  |
  +-- write_output()             # BrokenPipe-safe
  |
  +-- append_metric()            # JSONL, O_APPEND, 0600 perms
  |
  +-- exit(child_exit_code)      # 128+N for signal-killed processes
```

### Memory Model

- Filtered mode: O(head + tail) lines in memory, regardless of output size
- Raw mode: capped at 256 MB, then truncation with warning
- Line length: capped at 1 MB per line to prevent OOM on binary input

### Signal Handling

- The captured child runs in its own process group. `l0-cache` installs SIGINT
  and SIGTERM handlers that **forward** the signal to that group, so Ctrl-C and a
  directed `kill <pid>` (or `timeout`, systemd, `docker stop`) terminate the whole
  child subtree — not just the `sh` wrapper — and `l0-cache` then propagates the
  child's status.
- SIGPIPE: ignored in `l0-cache`, BrokenPipe handled in code so metrics are logged
  before exit
- Exit codes: POSIX 128+N convention for signal-killed children

## Metrics

Each invocation logs a JSON line to `~/.local/share/l0-cache/metrics.jsonl`:

```json
{
  "ts": "2024-01-15T10:30:00Z",
  "cmd": "cargo",
  "args": "test --all",
  "bytes_raw": 15000,
  "bytes_final": 3000,
  "tokens_raw": 3750,
  "tokens_final": 750,
  "tokens_saved": 3000,
  "lines_raw": 500,
  "lines_final": 62,
  "truncated": true,
  "strategy": "head_tail",
  "exit_code": 0,
  "duration_ms": 1234,
  "version": "0.1.0"
}
```

Data directory resolution: `$XDG_DATA_HOME/l0-cache/` then `$HOME/.local/share/l0-cache/`
then `/etc/passwd` lookup (for containers, cron, systemd).

File permissions are set to 0600. Auto-rotation at 10 MB.

## Cross-Platform Support

| Environment | Build target | Status |
|---|---|---|
| macOS arm64/x86_64 | native | Tested |
| Ubuntu 22.04 / 24.04 | `x86_64-unknown-linux-gnu` | Tested |
| Alpine Linux | `x86_64-unknown-linux-musl` | Static binary |
| LXC container | same as host | `/etc/passwd` fallback |
| Proxmox VE | same as host | Works in host and guests |
| cron / systemd | same as host | `/etc/passwd` fallback |
| SSH with PTY | same as host | Signals work normally |
| SSH without PTY | same as host | See known limitations |

### Cross-Compilation from macOS

```sh
# Prerequisites
brew install filosottile/musl-cross/musl-cross
rustup target add x86_64-unknown-linux-gnu
rustup target add x86_64-unknown-linux-musl

# Build
make build-linux    # glibc, dynamic
make build-alpine   # musl, fully static

# Deploy
make deploy-linux HOST=user@server
make deploy-alpine HOST=user@alpine-host
```

## Shell Completions

```sh
# Bash
l0-cache --completions bash > /etc/bash_completion.d/l0-cache

# Zsh
l0-cache --completions zsh > ~/.zsh/completions/_l0-cache

# Fish
l0-cache --completions fish > ~/.config/fish/completions/l0-cache.fish
```

## Claude Code integration (optional)

Normally you prefix a command yourself (`l0-cache cargo test`). To let Claude Code
do that automatically for noisy commands — with nothing to remember — there is an
opt-in [`PreToolUse`](https://docs.claude.com/en/docs/claude-code/hooks) hook,
managed by `claude-hook.sh`:

```sh
./claude-hook.sh install     # write the wrapper + register the hook (needs jq)
./claude-hook.sh enable      # turn it ON  (instant on/off, no restart)
# → start a new Claude Code session so the hook loads
./claude-hook.sh disable     # turn it OFF immediately
./claude-hook.sh status      # installed? registered? on/off + version
./claude-hook.sh uninstall   # remove it
```

The hook is **conservative and fail-safe**: it only wraps simple single commands
and passes through anything risky — pipes, `&&`/`||`, redirects, `cd`/`export`,
command substitution, multi-line, and interactive programs — unchanged. Any error
makes the command run exactly as sent, and it does **not** auto-approve commands
(they still go through your normal permission rules). It is **off by default** and
toggled by a sentinel file (`~/.config/l0-cache/hook.enabled`), so you can disable
it instantly if anything misbehaves. Honors `$CLAUDE_CONFIG_DIR` / `$XDG_CONFIG_HOME`.

## Known Limitations

- **SSH without PTY**: when running `ssh host l0-cache cargo build` (no `-t` flag),
  Ctrl-C may not reach the child process. Use `ssh -t host l0-cache cargo build`
  instead.

- **Binary output**: detected on the first 8 KB. If a command produces text
  followed by binary data after 8 KB, the binary portion is processed as text
  (with UTF-8 lossy conversion).

- **Shell requirement**: `l0-cache` requires `/bin/sh` or `/usr/bin/sh` for the
  `2>&1` merge. In distroless containers without a shell, use `l0-cache -i` for
  passthrough mode (no stderr merge, no filtering).

## Hardening

The following protections are in place for production use across diverse
environments:

- UTF-8 lossy reads: never drops lines on invalid encoding
- Line length cap: 1 MB per line prevents OOM on binary/minified input
- Raw mode cap: 256 MB prevents OOM on massive output
- SIGPIPE handling: metrics are always logged, even when piped to `head`
- Exit code 128+N: POSIX-correct for signal-killed processes
- `$HOME` fallback: `/etc/passwd` lookup for containers, cron, systemd
- Metrics rotation: auto-rename to `.old` at 10 MB
- File permissions: 0600 on metrics file
- Shell check: clear error when `/bin/sh` is missing

## Development

```sh
cargo test           # 186 tests (unit + E2E integration)
cargo clippy         # 0 warnings enforced
cargo build --release
```

## License

MIT. See [LICENSE](LICENSE).
