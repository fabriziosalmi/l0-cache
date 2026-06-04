# Hardening

`l0-cache` is designed for unattended operation on servers, containers, and CI
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
| User presses Ctrl-C during long command | SIGINT ignored in `l0-cache`, child receives it via process group |
| `l0-cache cmd | head` closes pipe early | SIGPIPE ignored, BrokenPipe caught, metrics logged, exit 141 |
| `l0-cache` killed before `child.wait()` | Signal handlers prevent this scenario |
| `/bin/sh` missing (distroless container) | Pre-spawn check with clear error message |

## I/O Safety

| Threat | Protection |
|---|---|
| Invalid UTF-8 in command output | `read_line_lossy()` with `String::from_utf8_lossy` |
| Windows-style `\r\n` line endings (SSH from Windows) | Stripped during line reading |
| `$HOME` not set (containers, cron, systemd) | Fallback to `/etc/passwd` lookup via `getuid()` |
| Metrics file permissions in shared environments | `chmod 0600` on every open |
| Metrics file growing unbounded | Auto-rotation at 10 MB |
| Partial JSON write (process killed mid-write) | Stats reader skips malformed lines |
| Concurrent writes from multiple `l0-cache` instances | `O_APPEND` mode (atomic for lines < PIPE_BUF) |

## What Is Not Protected

- **SSH without PTY**: signals may not reach child. Use `ssh -t`.
- **Binary detection after 8 KB**: late binary data is processed as text
  (lossy, not dangerous, but output may be noisy).
- **NFS/CIFS**: `O_APPEND` atomicity is not guaranteed on network
  filesystems. Metrics may have interleaved lines in rare cases.
