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

Adaptive parameter auto-tuning is enabled by default. Five rules tune `head`,
`tail`, and `tail_error` per `(cmd, args_hash)` bucket — see
[the Configuration guide](../guide/configuration.md#parameter-auto-tuning-enabled-by-default)
for the full semantics, and the `AUTO-TUNING` section of `--stats` (plus
the `auto_tuning` object in `--stats --json`) for what's firing in your
sessions.

Each rule firing is appended to a small sidecar at
`$XDG_DATA_HOME/l0-cache/tuned.jsonl`, which the learner reads on the next
run of the same bucket so the decay/shrink rules **compound** across runs
instead of resetting from CLI defaults each time. Delete this file to reset
all learned tunes.

#### `--no-auto`
Disable adaptive auto-tuning. The learner also stops reading and writing
`tuned.jsonl` for this run.

#### `--auto`
Enable adaptive auto-tuning (redundant as it is now enabled by default, but supported for backward compatibility).

#### `--auto-floor <N>`
Floor limit for the decay rules (`decay_moderate`, `decay_strong`,
`decay_steady`) and `proactive_shrink`. The tuned `head`/`tail` is never
allowed below this. Default: 10.

#### `--auto-ceiling <N>`
Ceiling limit for failure-backoff `tail_error` expansion
(`expand_tail_err`). Default: 1000.

### Metrics

#### `--stats`
Print an aggregated token savings report and exit. Does not run a command.
Renders a boxed dashboard (runs, tokens saved, per-command efficiency and bars)
followed by an `AUTO-TUNING` section: total adaptive firings, per-event
breakdown (`expand_tail_err`, `decay_moderate`, `decay_strong`,
`proactive_shrink`, `decay_steady`), a `noisy` counter (expand firings that
fired on zero-output runs — false-positive surface), and the top commands by
firing count. Color is emitted only on a TTY; piping, redirecting, or
`NO_COLOR` yields plain text, and `FORCE_COLOR` forces it on.

#### `--since <DURATION>`
Filter the stats report to entries within the given time window.
Requires `--stats`. Format: `Nd`, `Nh`, `Nm`, `Ns` (e.g. `7d`, `24h`).
Entries with an unparseable timestamp are excluded from a windowed query.

#### `--discover`
Print an opinionated optimization advisory from your metrics — which prefixed
commands are paying off (keep), which to consider dropping (low savings over enough
runs), and the biggest raw-token footprint — then exit. Honors `--since`.

#### `--json`
Output `--stats` as a single JSON object (totals + per-command array) instead of the
dashboard, for tooling.

#### `--cost-per-mtok <N>`
USD per million tokens. When greater than 0, `--stats` and `--discover` show the
estimated cost saved (and `usd_saved` appears in `--json`). Default: 0 (hidden).

#### `--reset-stats`
Delete **all** recorded telemetry (the `metrics.jsonl` file) and exit. This is
destructive and cannot be undone. **Does not** touch `tuned.jsonl` — delete
that file separately if you also want to drop the persisted adaptive tunes.

#### `--token-factor <N>`
Divisor used for token estimation. The number of bytes is divided by this factor to estimate the token count. Default: 4.

### Safety & Output Control

#### `--guard` / `--no-guard`
Force-enable or force-disable the [Safety Guard](#safety-guard). By default the
guard auto-enables inside detected AI-assistant terminals.

#### `--quiet`, `-q`
Suppress l0-cache's own stderr notices (e.g. auto-tuning messages).

#### `--recover`
On a **failing** command whose output was **truncated**, save the full
(un-truncated) output to a temp file and reference it in the banner, so the agent
can read the omitted middle without re-running. Off by default. Lazy (no disk for
small/under-threshold output), bounded, and fail-safe (any I/O error is ignored).

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
