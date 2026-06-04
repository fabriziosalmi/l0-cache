# Exit Codes

`l0-cache` propagates the child process exit code as its own. The following
codes have special meaning.

## Standard Exit Codes

| Code | Meaning |
|---|---|
| 0 | Child exited successfully |
| 1-125 | Child exited with error (code passed through) |
| 126 | Child command found but not executable |
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

When the user sends Ctrl-C (SIGINT):

1. SIGINT is delivered to the entire process group by the terminal.
2. `l0-cache` ignores SIGINT (handler installed at startup).
3. The child process receives SIGINT and handles it (typically exits).
4. `l0-cache` calls `child.wait()`, collects the exit status.
5. `l0-cache` reports exit code 130 (128 + SIGINT).

This ensures `l0-cache` always completes its cleanup (metrics logging) before
exiting.
