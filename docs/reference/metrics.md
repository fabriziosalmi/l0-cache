# Metrics Format

## File Location

`$XDG_DATA_HOME/l0-cache/metrics.jsonl` or `~/.local/share/l0-cache/metrics.jsonl`.

## Format

One JSON object per line (JSONL). Each line represents one `l0-cache` invocation.

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

## Fields

| Field | Type | Description |
|---|---|---|
| `ts` | string | RFC 3339 / ISO 8601 UTC timestamp, second resolution (`YYYY-MM-DDTHH:MM:SSZ`) |
| `cmd` | string | Command binary name (basename only) |
| `args` | string | Command arguments joined by space |
| `bytes_raw` | integer | Total bytes of raw output |
| `bytes_final` | integer | Bytes after filtering |
| `tokens_raw` | integer | Estimated tokens (bytes_raw / 4) |
| `tokens_final` | integer | Estimated tokens (bytes_final / 4) |
| `tokens_saved` | integer | tokens_raw - tokens_final |
| `lines_raw` | integer | Total lines of raw output |
| `lines_final` | integer | Lines after filtering |
| `truncated` | boolean | Whether output was truncated |
| `strategy` | string | Filter strategy used |
| `exit_code` | integer | Child process exit code |
| `duration_ms` | integer | Wall-clock execution time in milliseconds |
| `version` | string | Binary version of `l0-cache` |

## Strategy Values

| Value | Meaning |
|---|---|
| `head_tail` | Normal filtered mode |
| `raw` | `--raw` mode (no truncation) |
| `binary_skip` | Binary output detected, passed through |

## Token Estimation

Tokens are estimated as `bytes / 4`, which is a reasonable approximation
for English text with common LLM tokenizers (GPT-4, Claude). The estimate
is used for the savings report, not for billing.

## File Management

- **Permissions**: 0600 (owner read/write only)
- **Write mode**: `O_APPEND` (atomic for lines smaller than PIPE_BUF)
- **Rotation**: auto-renamed to `metrics.jsonl.old` when file exceeds 10 MB
- **Malformed lines**: silently skipped when reading stats
