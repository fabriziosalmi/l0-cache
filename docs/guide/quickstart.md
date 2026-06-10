# Quick Start

## Basic Usage

Prefix any command with `l0-cache`:

```sh
l0-cache cargo test
l0-cache npm run build
l0-cache git log --oneline -50
l0-cache docker compose logs
l0-cache kubectl get pods -A
```

Output is automatically filtered. Lines beyond the head/tail window are
replaced with a banner:

```
line 1
line 2
...
line 30

... [370 lines omitted for LLM] ...

line 401
line 402
...
line 430
```

## Full Output

When you need the complete output (but still want metrics logged):

```sh
l0-cache --raw cargo test
```

## Interactive Commands

Interactive commands (like `vim`, `htop`, and `ssh`) are automatically detected and run in
passthrough mode. You can also force passthrough:

```sh
l0-cache -i python3    # force interactive mode
```

Auto-detected interactive commands: `vim`, `vi`, `nvim`, `nano`, `emacs`,
`less`, `more`, `man`, `htop`, `top`, `ssh`, `fzf`.

## Token Savings Report

```sh
# All time
l0-cache --stats

# Last 7 days
l0-cache --stats --since 7d

# Last 24 hours
l0-cache --stats --since 24h
```

Example output (colors omitted):

```
┌─ l0-cache TELEMETRY ───────────────────────────────── last 7d ─┐
│ Runs        35                                                 │
│ Saved       12.5k  of 17.4k raw · est. tokens                  │
│ Efficiency   71.7%  █████████████████░░░░░░░                   │
│ Median/run   65.2%  unweighted                                 │
│             cargo accounts for 66% of savings                  │
├────────────────────────────────────────────────────────────────┤
│ COMMAND     RUNS   SAVED  EFFIC. IMPACT                        │
│ cargo         15    8.2k   78.4% ██████████░░  ↑ most saved    │
│ git           12    3.1k   65.2% ██████░░░░░░                  │
│ npm            8    1.2k   54.2% ████░░░░░░░░                  │
├────────────────────────────────────────────────────────────────┤
│ AUTO-TUNING                                                    │
│ Firings     8  22.9% of 35 runs                                │
│   expand_tail_err    1   decay_mod   2   decay_strong   3      │
│   proactive_shrink     1   decay_steady   1   recover   0      │
│   noisy     0   0.0% of firings                                │
│ Top cmds (by firings)                                          │
│   cargo         5   Dm:2 Ds:3                                  │
│   git           2   P:1 Dsy:1                                  │
│   npm           1   E:1                                        │
│   E=expand Dm/Ds/Dsy=decay P=shrink R=recover                  │
└────────────────────────────────────────────────────────────────┘
  metrics ~/.local/share/l0-cache/metrics.jsonl
```

Reading the dashboard:

- **Saved / Efficiency** — token-weighted totals (tokens are estimated as
  bytes ÷ 4). **Median/run** is the unweighted per-run median, the honest
  companion when one command dominates; in that case a dim
  "`<cmd>` accounts for N% of savings" line appears (above 50%).
- **EFFIC.** — per-command savings ratio. `100.0%` is shown only when the
  output was elided entirely; near-perfect values floor at `99.9%`.
- **IMPACT** — the command's share of all tokens saved (square-root scaled so
  small rows stay visible), colored by efficiency.
- **Markers** — `↑ most saved` tags the top row; `⚠ low` flags commands with
  ≥5 runs saving <10% (or producing no output at all — see the footer hints);
  `(n<5)` marks low-efficiency rows still below the 5-run sample gate.

When stdout is not a terminal (piped or redirected, or with `NO_COLOR` set),
the dashboard renders as plain text like above; on a TTY it is colorized, with
the efficiency bars shaded red → orange → green by savings, and the box grows
with the terminal (the COMMAND column absorbs the extra width).

## Error Handling

On non-zero exit, the tail window automatically expands to 120 lines
(configurable with `--tail-error`). This ensures error messages and stack
traces are preserved in full.

```sh
# Default: 120 tail lines on error
l0-cache cargo test

# Custom: 200 tail lines on error
l0-cache --tail-error 200 cargo test
```
