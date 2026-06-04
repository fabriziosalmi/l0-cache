# Exit Codes

`l0-cache` propagates the child process exit code as its own. The following
codes have special meaning.

## Standard Exit Codes

| Code | Meaning |
|---|---|
| 0 | Child exited successfully |
| 1-125 | Child exited with error (code passed through) |
| 126 | Child found but not executable, **or blocked by the [Safety Guard](./#safety-guard)** |
| 127 | Child command not found, or `/bin/sh` not found |

## Signal Exit Codes (POSIX Convention)

When the child process is killed by a signal, `l0-cache` reports the exit code
as `128 + signal_number`. This follows the POSIX/bash convention.

| Code | Signal | Common Cause |
|---|---|---|
| 130 | SIGINT (2) | User pressed Ctrl-C |
| 131 | SIGQUIT (3) | User pressed Ctrl-\\ |
| 137 | SIGKILL (9) | `kill -9`, OOM killer |
| 139 | SIGSEGV (11) | Segmentation fault in child |
| 141 | SIGPIPE (13) | `l0-cache cmd \| head` (pipe closed) |
| 143 | SIGTERM (15) | `kill` (default signal) |

## Proxy-Specific Codes

| Code | Meaning |
|---|---|
| 127 | `l0-cache` itself failed to start the child (command not found, `/bin/sh` missing) |
| 141 | `l0-cache` detected BrokenPipe on stdout (e.g. `l0-cache cmd \| head`) |

## Behavior on Signals

The captured child runs in its **own process group**. When `l0-cache` receives
SIGINT (Ctrl-C) or SIGTERM (`kill`, `timeout`, systemd, `docker stop`):

1. `l0-cache`'s signal handler **forwards** the signal to the child's process
   group, so the whole child subtree (pipelines, grandchildren) receives it.
2. The child handles the signal (typically exits); its stdout pipe closes.
3. `l0-cache` finishes reading, calls `child.wait()`, and collects the status.
4. `l0-cache` reports `128 + signal_number` (e.g. 130 for SIGINT, 143 for SIGTERM).

This both lets the child terminate cleanly **and** ensures `l0-cache` always
completes its own cleanup (metrics logging) before exiting. SIGPIPE is ignored so
that `l0-cache cmd | head` can log metrics before exiting 141.
