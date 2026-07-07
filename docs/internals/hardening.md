# Hardening

`l0-compressor` is designed for unattended operation on servers, containers, and CI
systems. The following protections are in place.

## Memory Safety

| Threat | Protection |
|---|---|
| Binary file with no newlines (single 2 GB "line") | Line length cap: 1 MB per line |
| `--raw` on command producing gigabytes of output | Raw mode cap: 256 MB, then drain without storing |
| `head_cap + tail_cap` arithmetic overflow | `saturating_add` throughout |
| `usize::MAX` passed to `Vec::with_capacity` | Raw mode has dedicated path, no `HeadTailBuffer` |

## Process Safety

| Threat | Protection |
|---|---|
| Child killed by signal (SIGKILL, SIGSEGV) | Exit code 128+N (POSIX convention) |
| User presses Ctrl-C during long command | SIGINT ignored in `l0-compressor`, child receives it via process group |
| `l0-compressor cmd | head` closes pipe early | SIGPIPE ignored, BrokenPipe caught, metrics logged, exit 141 |
| `l0-compressor` killed before `child.wait()` | Signal handlers prevent this scenario |
| `/bin/sh` missing (distroless container) | Pre-spawn check with clear error message |

## I/O Safety

| Threat | Protection |
|---|---|
| Invalid UTF-8 in command output | `read_line_lossy()` with `String::from_utf8_lossy` |
| Multibyte characters straddling a byte offset (`€`, emoji, CJK) | Char-boundary-safe slicing (`get(..)`) in every line transform; regression-tested — a panic here used to swallow the child's real exit code |
| Windows-style `\r\n` line endings (SSH from Windows) | Stripped during line reading |
| `$HOME` not set (containers, cron, systemd) | Fallback to `/etc/passwd` lookup via `getuid()` |
| Metrics file permissions in shared environments | `chmod 0600` on every open |
| Metrics file growing unbounded | Auto-rotation at 10 MB |
| Partial JSON write (process killed mid-write) | Stats reader skips malformed lines |
| Concurrent writes from multiple `l0-compressor` instances | `O_APPEND` mode (atomic for lines < PIPE_BUF) |
| Credential-shaped values in recorded `args` | Redacted before the metric is written |

## Recovery File Safety (`--recover`)

The full-output temp file written on a truncated failure is hardened against
shared-`/tmp` attacks (Unix):

| Threat | Protection |
|---|---|
| Symlink pre-planted at the predictable path (clobbers a victim's file) | Directory rejected if it is a symlink or owned by another user; file opened with `O_NOFOLLOW` |
| Un-redacted output readable by other users | Private `0700` directory, `0600` file |

## Safety Guard

The [safety guard](/reference/#safety-guard) refuses to run a small set of
clearly destructive commands (recursive force-removal of system paths and the
user's HOME, reverse shells, credential exfiltration, `DROP DATABASE`) when an
AI-assistant terminal is detected. Its `rm -rf` matcher resolves `..`
traversal, home parents (`/home`, `/Users`), nested `sh -c` payloads,
`env`/`sudo`/`VAR=x` wrapper prefixes, and `~user` spellings. A 161-case
adversarial suite (`sandbox_rm_rf_in_100_ways_is_always_blocked`) pins every
decision in CI — it calls only the guard's decision function, so it can never
run a real `rm` — together with a benign control set that must never be
blocked. The guard is a seatbelt against accidents, not a security boundary;
its residual limits are documented in the [reference](/reference/#safety-guard).

## What Is Not Protected

- **SSH without PTY**: signals may not reach child. Use `ssh -t`.
- **Binary detection after 8 KB**: late binary data is processed as text
  (lossy, not dangerous, but output may be noisy).
- **NFS/CIFS**: `O_APPEND` atomicity is not guaranteed on network
  filesystems. Metrics may have interleaved lines in rare cases.
