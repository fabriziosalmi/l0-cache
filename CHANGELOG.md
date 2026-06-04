# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-06-04

### Added
- `claude-hook.sh`: an optional, off-by-default transparent integration for
  Claude Code. It installs a `PreToolUse` hook that routes the *simple* Bash
  commands Claude Code runs through `l0-cache`, so the model never has to prefix
  anything. Conservative (compound/piped/redirected/interactive/stateful commands
  pass through untouched), fail-safe (any error or a missing `l0-cache`/`jq`
  leaves the command unchanged, and it never sets a `permissionDecision`), and
  toggleable at runtime with no restart (`install`/`enable`/`disable`/`status`/
  `uninstall`). See the Claude Code Integration guide.

### Fixed
- Memory is now bounded on a giant newline-free stream (e.g. a minified bundle).
  Line reading no longer uses `read_until`, which buffered the entire line before
  the 1 MB cap could apply; a chunked reader keeps at most ~1 MB and drains the
  rest (still counted for accurate `bytes_raw`). A new RSS stress test pushes
  100 MB as a single line and asserts the resident set stays far below it.

## [0.1.1] - 2026-06-04

### Fixed
- **`--tail-error` / failure backoff now actually works**: the tail buffer retains
  `max(--tail, --tail-error)` lines while streaming and trims at render based on
  exit code, instead of an after-the-fact expansion that did nothing. Also fixed a
  latent silent gap when the tail wrapped below the threshold.
- **`--raw` is now verbatim**: progress-bar/backspace squashing and big-JSON
  truncation are applied in filtered mode only; raw keeps content unchanged.
- **Binary output** is no longer silently truncated to ~8 KB with `truncated:false`;
  it now carries an explicit "binary output detected — showing first N of M bytes"
  banner and `truncated:true`.
- **Metrics `bytes_raw`** is measured from true pre-transform bytes, so reported
  token savings are no longer understated.
- **Metrics rotation** preserves recent history (it was rewriting the pruned log
  into a never-read `.old` file) and is size-capped to avoid re-rotating every run.
- **Safety Guard** is no longer bypassed by `sh -c "rm -rf /etc"` (it now scans the
  shell `-c` payload) or by trailing-slash/glob path variants.
- **`print_stats`** no longer panics on a multibyte command name in the metrics file.
- **Signals**: the captured child runs in its own process group; SIGINT/SIGTERM are
  forwarded to it (a directed `kill`/`timeout`/`docker stop` now terminates the
  child instead of being swallowed), and the `--idle-timeout` watchdog kills the
  whole group rather than only the `sh` wrapper.
- **Installer**: the `curl | bash` one-liner installs correctly (it was a silent
  no-op on the empty-argument path) and pins to the latest release tag.

### Added
- Safety Guard documentation, `L0_CACHE_GUARD` truthy/falsy parsing, and a unified
  enable/disable decision shared by enforcement and `--doctor`.
- Credential redaction for the metrics `args` field (passwords, tokens, auth
  headers, URL userinfo).
- Documentation for the previously undocumented flags (`--guard`/`--no-guard`,
  `--only-errors`, `--idle-timeout`, `--quiet`, `--reset-stats`) and an installer
  CI job (shellcheck + regression checks).

## [0.1.0] - 2026-06-04

### Added
- Rebranded CLI proxy under the name `l0-cache`.
- Single-threaded, synchronous Rust engine for process execution.
- Universal output filtering pipeline:
  - ANSI escape stripping.
  - Squeezing of consecutive blank lines.
  - Identical line collapsing.
  - Prefix-based line collapsing (e.g. for compiler progress).
- Head and tail buffer rendering with a safety floor of 10 lines.
- Failure backoff tail expansion (up to 1000 lines) to prevent AI loops.
- Consecutive success token savings decay (20% and 40% reductions).
- Restricted permission checks on local database metrics log file (`chmod 0600`).
- Local metrics reporting system with `XDG_DATA_HOME` overrides and automated rotation at 10 MB.
- Isolated E2E integration tests and LCG pseudo-random fuzzer testing.
- Static musl cross-compilation pipeline for Alpine and dynamic builds for Linux/macOS.
- Shell completions script generator for Bash, Zsh, Fish, Elvish, and PowerShell.
- VitePress documentation site.
