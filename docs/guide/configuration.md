# Configuration

`l0-compressor` is configured by command-line flags. An **optional** per-command config
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
| `--no-squelch` | false | Disable the clean-success squelch (keep the full tail on clean exits) |
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

## Clean-success squelch

On a **zero exit** with **no** error/warning signal anywhere in the stream, the
displayed tail is trimmed further — halved and floored at 5 lines (`--tail 30`
shows 15, a tail already ≤ 5 is left untouched, never expanded). The middle of a
clean build/test/install is almost always progress noise that the agent does not
need to confirm success. Always preserved: the **head** (command echo) and the
**final summary line**.

The moment any signal appears — `error`, `warn`, `fail`, `exception`, `panic`,
`traceback`, `fatal` (the same keyword set as [`--only-errors`](/reference/#only-errors)) —
**or** the command exits non-zero, the full tail is restored. **Failures are
never squelched.** The truncation banner reports the *actual* tail rendered (e.g.
`30 head + 15 tail`), not the configured cap.

On by default. Disable per-run with [`--no-squelch`](/reference/#no-squelch), or
per-command with `squelch = false` in the [config file](#config-file-optional).

## Config file (optional)

Drop a file in `$XDG_CONFIG_HOME/l0-compressor/` (or `~/.config/l0-compressor/`) to set
per-command defaults without recompiling and without per-tool parsers. l0-compressor
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
  `only_errors`, `recover`, `squelch`.
- A section names a command; **`[defaults]`** (or **`[*]`**) apply to every command,
  and a per-command section layers on top (command wins field-by-field).
- Commands are matched by **resolved name**, so `sh -c "cargo test"` matches the
  `cargo` block — the same name used by metrics and auto-tuning.
- **Precedence**: an explicit CLI flag > config file > built-in default.
  Auto-tuning then adjusts from that resolved base.
- A missing/unreadable file is silently ignored; a malformed file is ignored with
  a single stderr note (unless `--quiet`). Unknown keys are skipped, so a config
  written for a newer l0-compressor won't break an older binary.

## Parameter Auto-tuning (Enabled by Default)

By default, `l0-compressor` automatically tunes `head`, `tail`, and `tail_error`
per **bucket** — `(cmd, args_hash)` — based on the execution history in the
local metrics log. Pass `--no-auto` to disable. Each bucket carries its
own learning, so e.g. `curl https://api.openai.com` and `curl https://example.com`
don't pollute each other.

### Rules

Six rules feed off the same per-bucket history. The first one whose
trigger matches wins (top-to-bottom precedence).

1. **`expand_tail_err` — anti-loop backoff**. If the last F consecutive
   bucket records were failures *with output* (`exit_code != 0 && lines_raw > 0`),
   scale `tail_error` by `(1 + F)`, capped by `--auto-ceiling` (default 1000).
   Failing records with `lines_raw == 0` (e.g. `grep` "no match", `find`
   "not found") **break** the streak rather than feed it — their failure
   mode isn't one extra tail of error context can help with. Records with
   `--stats --json` track this distinction under the `noisy` counter.

2. **`decay_moderate` — 3-4 consecutive truncated successes**. Shrink
   `head` and `tail` by 20% (floored at `--auto-floor`, default 10).

3. **`decay_strong` — 5+ consecutive truncated successes**. Shrink `head`
   and `tail` by 40% (same floor).

4. **`recover_defaults` — the un-ratchet**. Every other rule only moves
   `head`/`tail` down and `tail_error` up, compounding across runs. After
   5 consecutive clean (successful, non-truncated) runs, a bucket sitting
   below its configured base restores `head`/`tail` to base — unless the
   tune was seeded by `proactive_shrink`, whose evidence a clean streak
   *confirms* — and a `tail_error` expanded above base returns to base
   once the failures stop. Both restores happen in one firing. Recovery
   is strictly per `(cmd, args_hash)` bucket: the same bucket must start
   producing clean runs (e.g. the same `cat <file>` over a now-smaller
   file), not merely the same command with different args.

5. **`decay_steady` — window-adaptive**. If the bucket has ≥20 records,
   all successful, and ≥80% of the last 20 are truncated, shrink `head`
   and `tail` by 30%. Catches the steady-state pattern that "5 consecutive"
   misses when occasional non-truncated runs break the streak.

6. **`proactive_shrink` — long clean histories**. If the bucket has ≥20
   records, all clean (success + not truncated), and `max(lines_raw) + 5`
   is at most half the current `head + tail` budget, set
   `head = max(lines_raw) + 5` and `tail = default_tail / 4`. Max-based,
   so it never introduces a new truncation vs. observed history.

A trigger whose numeric result equals the seeded values (floor/ceiling
pinned) is **not** a firing: no event is recorded and nothing is persisted.

### Persistence (`tuned.jsonl`)

Each time a rule fires (with a real change to `head`/`tail`/`tail_error`),
the result is upserted into `$XDG_DATA_HOME/l0-compressor/tuned.jsonl`, keyed by
`(cmd, args_hash)` — the file is compacted on write to one line per bucket,
and entries older than 30 days are pruned (an expired tune also stops
seeding runs). The next run of the same bucket starts from the saved
tune instead of the CLI defaults, so the decay/shrink rules **compound**:

```
run 4:  decay_moderate from defaults (30, 30)   → head=24 tail=24 saved
run 5:  decay_moderate from cached  (24, 24)    → head=19 tail=19 saved
run 6:  decay_strong   from cached  (19, 19)    → head=11 tail=11 saved
run 7:  decay_strong   from cached  (11, 11)    → head=10 tail=10  (floor hit)
```

The floor (`--auto-floor`) stops further compounding — and once the
workload changes, `recover_defaults` walks the bucket back to its base.
Persistence is best-effort: a missing or unreadable `tuned.jsonl`
silently degrades to the no-persistence behavior. `--reset-stats`
deletes the file along with the metrics (or delete it by hand to reset
all learned tunes).

### Diagnostic print

When a rule changes the params, `l0-compressor` prints a single note to stderr
(silenced by `--quiet`), e.g.

```
l0-compressor: auto-tuning: 2 consecutive failures detected, expanding tail_error to 720
```

The same firings are aggregated under the `AUTO-TUNING` section of
`l0-compressor --stats` (and the `auto_tuning` block of `--stats --json`).

## Environment Variables

| Variable | Purpose |
|---|---|
| `XDG_DATA_HOME` | Override metrics directory (default: `~/.local/share/l0-compressor/`) |
| `HOME` | Used if `XDG_DATA_HOME` is not set |
| `NO_COLOR` | If set (any value), disables ANSI color in `--stats` / `--doctor` |
| `FORCE_COLOR` / `CLICOLOR_FORCE` | Force color on even when stdout is not a TTY (CI captures, screenshots) |

By default `--stats` and `--doctor` emit color only when stdout is an interactive
terminal, so piping or redirecting them yields clean, escape-free text.

If neither is set (containers, cron), `l0-compressor` falls back to `/etc/passwd` lookup.

## Metrics Location

Metrics are stored at `$XDG_DATA_HOME/l0-compressor/metrics.jsonl` (or
`~/.local/share/l0-compressor/metrics.jsonl` by default).

The file auto-rotates at 10 MB. The previous file is kept as
`metrics.jsonl.old`. When rotating, entries older than 30 days are automatically
filtered out of the `.old` file to keep disk usage in check.
