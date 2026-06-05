# Configuration

`l0-cache` is configured by command-line flags. An **optional** per-command config
file fills in values you did not set on the command line (see
[Config file](#config-file-optional) below) — there is still no required config,
and explicit flags always win.

## Tuning Parameters

| Parameter | Default | Description |
|---|---|---|
| `--head` | 30 | Lines to keep from the start of output |
| `--tail` | 30 | Lines to keep from the end of output |
| `--tail-error` | 120 | Tail lines when exit code is non-zero |
| `--threshold` | 100 | Minimum lines before truncation kicks in |
| `--no-auto` | false | Disable adaptive parameter auto-tuning |
| `--auto-floor` | 10 | Safety floor limit under `--auto` |
| `--auto-ceiling` | 1000 | Max ceiling limit under `--auto` |
| `--token-factor` | 4 | Divisor for token count estimation |

### Choosing Values

For **build tools** (cargo, npm, gradle): defaults work well. Build output is
typically headers + progress + summary.

For **test runners** (pytest, jest, cargo test): consider `--tail-error 200`
to capture full stack traces on failure.

For **log inspection** (docker logs, journalctl): consider `--head 10 --tail 80`
to prioritize recent entries.

## Config file (optional)

Drop a file in `$XDG_CONFIG_HOME/l0-cache/` (or `~/.config/l0-cache/`) to set
per-command defaults without recompiling and without per-tool parsers. l0-cache
auto-detects, in this order, `config.{json,toml,yaml,yml,conf,ini}` —
**transparent multi-format with zero extra dependencies** (JSON is parsed strictly
by serde; TOML/YAML/INI share a small flat parser, since the schema is flat):

```toml
# config.toml
[defaults]
recover = true

[cargo]
tail_error = 300
head = 50

[git]
head = 10
tail = 40
```

The same configuration in JSON or YAML:

```json
{ "defaults": { "recover": true },
  "commands": { "cargo": { "tail_error": 300, "head": 50 }, "git": { "head": 10, "tail": 40 } } }
```

```yaml
defaults:
  recover: true
cargo:
  tail_error: 300
  head: 50
git:
  head: 10
  tail: 40
```

- **Tunable keys** (all optional): `head`, `tail`, `tail_error`, `threshold`,
  `only_errors`, `recover`.
- A section names a command; **`[defaults]`** (or **`[*]`**) apply to every command,
  and a per-command section layers on top (command wins field-by-field).
- Commands are matched by **resolved name**, so `sh -c "cargo test"` matches the
  `cargo` block — the same name used by metrics and auto-tuning.
- **Precedence**: an explicit CLI flag > config file > built-in default.
  Auto-tuning then adjusts from that resolved base.
- A missing/unreadable file is silently ignored; a malformed file is ignored with
  a single stderr note (unless `--quiet`). Unknown keys are skipped, so a config
  written for a newer l0-cache won't break an older binary.

## Parameter Auto-tuning (Enabled by Default)

By default, `l0-cache` automatically optimizes parameter values based on the execution history of the same command (stored in the local metrics log). Pass `--no-auto` to disable this behavior.

### How It Works

1. **Anti-Loop Backoff (Consecutive Failures)**:
   - If the last $F$ runs of the command failed (exited with a non-zero status), the `--tail-error` parameter is scaled up:
     $$\text{tuned\_tail\_error} = \text{default\_tail\_error} \times (1 + F)$$
   - This ensures that if an LLM gets stuck in an error-fixing loop, `l0-cache` automatically exposes more log/trace context so the LLM has the necessary information to resolve the issue.
   - The expanded error tail is capped at a ceiling of `1000` lines (customizable via `--auto-ceiling <N>`).
   - The backoff resets as soon as the command successfully exits with status 0.

2. **Token Optimization Decay (Consecutive Successes)**:
   - If the last $S$ runs of the command succeeded and were truncated, the head and tail parameters are gradually reduced:
     - $3 \le S < 5$: 20% reduction (e.g. head/tail become 24).
     - $S \ge 5$: 40% reduction (e.g. head/tail become 18).
   - This saves additional token budget when commands are running smoothly.
   - A safety floor of `10` lines (customizable via `--auto-floor <N>`) is always enforced to ensure minimal context is preserved.

3. **Diagnostic Print**:
   - If parameters are adjusted, `l0-cache` prints a subtle note to `stderr` describing the change (e.g., `l0-cache: auto-tuning: 2 consecutive failures detected, expanding tail_error to 360`).

## Environment Variables

| Variable | Purpose |
|---|---|
| `XDG_DATA_HOME` | Override metrics directory (default: `~/.local/share/l0-cache/`) |
| `HOME` | Used if `XDG_DATA_HOME` is not set |
| `NO_COLOR` | If set (any value), disables ANSI color in `--stats` / `--doctor` |
| `FORCE_COLOR` / `CLICOLOR_FORCE` | Force color on even when stdout is not a TTY (CI captures, screenshots) |

By default `--stats` and `--doctor` emit color only when stdout is an interactive
terminal, so piping or redirecting them yields clean, escape-free text.

If neither is set (containers, cron), `l0-cache` falls back to `/etc/passwd` lookup.

## Metrics Location

Metrics are stored at `$XDG_DATA_HOME/l0-cache/metrics.jsonl` (or
`~/.local/share/l0-cache/metrics.jsonl` by default).

The file auto-rotates at 10 MB. The previous file is kept as
`metrics.jsonl.old`. When rotating, entries older than 30 days are automatically
filtered out of the `.old` file to keep disk usage in check.
