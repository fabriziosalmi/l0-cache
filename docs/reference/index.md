# CLI Reference

## Synopsis

```
l0-cache [OPTIONS] [COMMAND]...
```

## Options

### Execution Modes

#### `--raw`
Print the command output verbatim: no head/tail truncation, no line collapsing,
no progress-bar/JSON squashing. ANSI escapes are still stripped, and the safety
caps (1 MB per line, 256 MB total) still apply. Metrics are still logged.

#### `-i`, `--interactive`
Force passthrough mode. stdin, stdout, and stderr are inherited by the
child process. No capture, no filtering, no metrics.

### Filtering

#### `--head <N>`
Number of lines to keep from the start of output. Default: 30.

#### `--tail <N>`
Number of lines to keep from the end of output. Default: 30.

#### `--tail-error <N>`
Number of tail lines to keep when the child exits with non-zero status.
Default: 120. The buffer retains `max(--tail, --tail-error)` lines while
streaming, so the larger error tail is available on failure.

#### `--threshold <N>`
Minimum number of output lines before truncation is applied. If the
total output is below this threshold, it is printed in full. Default: 100.

#### `--only-errors`
Keep only lines that look like problems (`error`, `warn`, `fail`, `exception`,
`panic`, `traceback`, `fatal`). Aggressive; non-matching lines are dropped.

#### `--idle-timeout <N>`
SIGKILL the command and its whole process group after `N` seconds with no
output, to break interactive-prompt deadlocks. `0` (default) disables it.

### Adaptive Tuning

Adaptive parameter auto-tuning is enabled by default.

#### `--no-auto`
Disable adaptive auto-tuning of parameters.

#### `--auto`
Enable adaptive auto-tuning (redundant as it is now enabled by default, but supported for backward compatibility).

#### `--auto-floor <N>`
Floor limit for success optimization decay. Default: 10.

#### `--auto-ceiling <N>`
Ceiling limit for failure backoff tail expansion. Default: 1000.

### Metrics

#### `--stats`
Print an aggregated token savings report and exit. Does not run a command.
Renders a boxed dashboard (runs, tokens saved, per-command efficiency and bars).
Color is emitted only on a TTY; piping, redirecting, or `NO_COLOR` yields plain
text, and `FORCE_COLOR` forces it on.

#### `--since <DURATION>`
Filter the stats report to entries within the given time window.
Requires `--stats`. Format: `Nd`, `Nh`, `Nm`, `Ns` (e.g. `7d`, `24h`).
Entries with an unparseable timestamp are excluded from a windowed query.

#### `--reset-stats`
Delete **all** recorded telemetry (the `metrics.jsonl` file) and exit. This is
destructive and cannot be undone.

#### `--token-factor <N>`
Divisor used for token estimation. The number of bytes is divided by this factor to estimate the token count. Default: 4.

### Safety & Output Control

#### `--guard` / `--no-guard`
Force-enable or force-disable the [Safety Guard](#safety-guard). By default the
guard auto-enables inside detected AI-assistant terminals.

#### `--quiet`, `-q`
Suppress l0-cache's own stderr notices (e.g. auto-tuning messages).

### Utility

#### `--doctor`
Diagnose system installation, PATH resolution, shell configuration, and active LLM editors. Prints a SOTA terminal health report.

#### `--completions <SHELL>`
Generate shell completion script and print to stdout. Valid values:
`bash`, `zsh`, `fish`, `elvish`, `powershell`.

#### `--version`, `-V`
Print version with git commit hash (e.g. `l0-cache 0.1.0 (abc1234)`).

#### `--help`, `-h`
Print help message.

## Safety Guard

When `l0-cache` runs inside a detected AI-assistant terminal (Claude Code, Gemini
CLI, Cursor/VS Code), a **best-effort** guard blocks a few clearly destructive
commands before executing them, exiting with code **126**:

- recursive force-removal of a critical system path (`rm -rf /`, `/etc`, `/usr`,
  …), including inside `sh -c "…"` payloads and with trailing-slash/glob variants;
- reverse shells / socket redirections (`/dev/tcp`, `/dev/udp`);
- credential exfiltration (`curl`/`wget`/`nc`/`ssh` referencing `id_rsa`, `.env`,
  `shadow`, `credentials`, …);
- `DROP DATABASE` via `psql`/`mysql`/`sqlite3`/`sqlcmd`.

**Enabling/disabling** — precedence is `--no-guard` → `--guard` →
`L0_CACHE_GUARD` → auto-detect. The `L0_CACHE_GUARD` environment variable accepts
`1`/`true`/`on` (force on) and `0`/`false`/`off` (force off).

> The guard is a guard rail, not a sandbox: it pattern-matches argv and shell
> payloads and can be bypassed by a determined caller. Bypass an intentional
> command with `--no-guard`.
