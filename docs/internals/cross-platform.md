# Cross-Platform Support

## Tested Environments

| Environment | Build Target | Shell | HOME | Status |
|---|---|---|---|---|
| macOS arm64 | native | `/bin/sh` (zsh) | always set | Tested |
| macOS x86_64 | native | `/bin/sh` (zsh) | always set | Tested |
| Ubuntu 22.04 | `x86_64-unknown-linux-gnu` | `/bin/sh` (dash) | usually set | Tested |
| Ubuntu 24.04 | `x86_64-unknown-linux-gnu` | `/bin/sh` (dash) | usually set | Tested |
| Alpine 3.x | `x86_64-unknown-linux-musl` | `/bin/sh` (busybox ash) | usually set | Tested |
| LXC container | matches host | varies | often missing | `/etc/passwd` fallback |
| Proxmox VE host | `x86_64-unknown-linux-gnu` | `/bin/sh` (dash) | set | Tested |
| systemd service | matches host | varies | often missing | `/etc/passwd` fallback |
| cron job | matches host | varies | often missing | `/etc/passwd` fallback |
| Windows 10/11 | `x86_64-pc-windows-msvc` | none (direct spawn) | `%LOCALAPPDATA%` | Experimental (CI smoke-tested) |

## Windows (experimental)

On Windows the command is spawned directly via `CreateProcess` — no shell.
stdout and stderr are merged by two drain threads feeding one channel, so
interleaving between the two streams is best-effort (per read chunk) rather
than the kernel-exact ordering `sh -c '… 2>&1'` provides on unix; each
stream's own ordering is always preserved. Consequences:

- Shell syntax needs an explicit shell: `l0-compressor cmd /C "a | b"`.
- Ctrl-C follows default console semantics (delivered to the process group);
  there is no signal forwarding. The `--idle-timeout` watchdog kills the
  child tree with `taskkill /F /T`.
- The data directory resolves to `%XDG_DATA_HOME%`, then `%LOCALAPPDATA%`,
  then `%USERPROFILE%\.local\share`.
- The 0600/0700 permission hardening and the `--recover` symlink defenses
  are unix-only; Windows temp and profile directories are per-user.
- The safety guard's `rm -rf` matcher already understands `C:\Users` /
  `/c/Users` spellings.

Coverage: the unit suite runs on Windows in CI (the E2E suite is unix-gated),
plus a CI smoke test for native spawn, truncation, stderr merge, and
exit-code propagation.

## Build Targets

### macOS (native)

```sh
cargo build --release
# produces: target/release/l0-compressor (arm64 or x86_64, depending on host)
```

### Linux glibc (Ubuntu, Debian, RHEL)

```sh
cargo build --release --target x86_64-unknown-linux-gnu
```

Requires a Linux cross-compiler if building from macOS. The `Makefile`
handles this automatically.

### Linux musl (Alpine, containers)

```sh
CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc \
cargo build --release --target x86_64-unknown-linux-musl
```

Produces a fully static binary with no runtime dependencies. This is the
recommended build for containers and minimal environments.

## Shell Compatibility

The `2>&1` redirect and single-quote escaping used by `l0-compressor` are POSIX sh
compatible. They work identically on:

- **dash** (Ubuntu default `/bin/sh`)
- **bash** (common on RHEL, macOS pre-Catalina)
- **zsh** (macOS default since Catalina)
- **busybox ash** (Alpine default `/bin/sh`)

The only environment where this fails is distroless containers or scratch
images that have no shell at all. In that case, use `l0-compressor -i` for passthrough
mode (no `2>&1` merge).

## HOME Resolution

`l0-compressor` resolves the data directory in this order:

1. `$XDG_DATA_HOME/l0-compressor/`
2. `$HOME/.local/share/l0-compressor/`
3. `/etc/passwd` lookup using the current UID

The `/etc/passwd` fallback handles:

- `lxc exec container -- l0-compressor cargo test` (no `$HOME`)
- Cron jobs without `HOME=` in the crontab
- systemd services without `User=` (runs as root, `$HOME` may not be set)
