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
──────────────────────────────────────────────────────────────────────────────
 ● l0-cache Telemetry Dashboard
──────────────────────────────────────────────────────────────────────────────
 Period:       Last 7d
 Total Runs:   42
 Tokens Saved: 12.5k (71.5% efficiency)

 COMMAND                      RUNS         SAVED EFFICIENCY  IMPACT
──────────────────────────────────────────────────────────────────────────────
 cargo                          15          8.2k      78.4%  ████████████···
 git                            12          3.1k      65.2%  ██████████·····
 npm                             8          1.2k      54.1%  ████████·······
──────────────────────────────────────────────────────────────────────────────
```

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
