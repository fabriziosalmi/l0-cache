# Metrics Format

## File Location

`$XDG_DATA_HOME/l0-cache/metrics.jsonl` or `~/.local/share/l0-cache/metrics.jsonl`.

## Format

One JSON object per line (JSONL). Each line represents one `l0-cache` invocation.

```json
{
  "ts": "2026-06-10T10:30:00Z",
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
  "version": "0.1.10",
  "adaptive_event": "decay_moderate",
  "args_hash": "a1b2c3d4"
}
```

## Fields

| Field | Type | Description |
|---|---|---|
| `ts` | string | RFC 3339 / ISO 8601 UTC timestamp, second resolution (`YYYY-MM-DDTHH:MM:SSZ`) |
| `cmd` | string | Command binary name (basename only) |
| `args` | string | Command arguments joined by space, with credential-shaped values redacted |
| `bytes_raw` | integer | Total bytes of raw output |
| `bytes_final` | integer | Bytes after filtering |
| `tokens_raw` | integer | Estimated tokens (bytes_raw / `--token-factor`, default 4) |
| `tokens_final` | integer | Estimated tokens (bytes_final / `--token-factor`) |
| `tokens_saved` | integer | tokens_raw - tokens_final |
| `lines_raw` | integer | Total lines of raw output |
| `lines_final` | integer | Lines after filtering |
| `truncated` | boolean | Whether output was truncated |
| `strategy` | string | Filter strategy used |
| `exit_code` | integer | Child process exit code |
| `duration_ms` | integer | Wall-clock execution time in milliseconds |
| `version` | string | Binary version of `l0-cache` |
| `adaptive_event` | string, optional | The adaptive-tuning rule that fired this run, if any (see [Strategy Values](#strategy-values) and below); absent (and `Option::None` in code) when no rule fired or `--no-auto` was passed |
| `args_hash` | string, optional | 8-char FNV-1a 64-bit hash of `args`; the per-bucket key for the adaptive learner. Absent on pre-0.1.10 records |

## Strategy Values

| Value | Meaning |
|---|---|
| `head_tail` | Normal filtered mode |
| `raw` | `--raw` mode (no truncation) |
| `binary_skip` | Binary output detected, passed through |

## Adaptive event tags

When `adaptive_event` is present, it carries one of:

| Tag | Meaning |
|---|---|
| `expand_tail_err` | Tail-error expansion fired this run because of recent failures in this bucket |
| `decay_moderate` | 3-4 consecutive truncated successes → 20% head/tail shrink |
| `decay_strong` | 5+ consecutive truncated successes → 40% head/tail shrink |
| `decay_steady` | ≥80% of last 20 bucket records were truncated successes → 30% shrink |
| `proactive_shrink` | ≥20 clean (success + not truncated) records, `max(lines_raw) + 5` fits in half the current budget → shrink to `max + 5` |

See the [Adaptive auto-tuning section](../guide/configuration.md#parameter-auto-tuning-enabled-by-default)
of the configuration guide for the full rule semantics.

## Token Estimation

Tokens are estimated as `bytes / token_factor` (default `4`), which is a
reasonable approximation for English text with common LLM tokenizers
(GPT-4, Claude). The estimate is used for the savings report, not for billing.

## File Management

- **Permissions**: 0600 (owner read/write only)
- **Write mode**: `O_APPEND` (atomic for lines smaller than PIPE_BUF)
- **Rotation**: auto-renamed to `metrics.jsonl.old` when file exceeds 10 MB;
  entries older than 30 days are dropped at rotation time
- **Malformed lines**: silently skipped when reading stats
- **Back-compat**: pre-0.1.10 records (without `adaptive_event` / `args_hash`)
  still count in `--stats` totals; the adaptive learner ignores them so
  legacy data can't re-introduce noise

## Persistence sidecar (`tuned.jsonl`)

Alongside `metrics.jsonl`, the adaptive learner reads and writes
`$XDG_DATA_HOME/l0-cache/tuned.jsonl` — one JSON line per firing, keyed by
`(cmd, args_hash)`. Schema:

```json
{
  "ts": "2026-06-10T15:55:18Z",
  "cmd": "seq",
  "args_hash": "8cd4b46f",
  "head": 18,
  "tail": 18,
  "tail_error": 120,
  "event": "decay_strong"
}
```

The learner does a last-write-wins lookup for the current bucket on every
run and seeds the rule with those values instead of the CLI defaults — that
makes the decay/shrink rules compound across runs. Delete this file to
reset all learned tunes. Best-effort I/O; a missing or corrupt file is
silently treated as "no prior tune".
