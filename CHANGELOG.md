# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.8] - 2026-06-05

### Added
- **Prebuilt release binaries — install without a Rust toolchain.** A CI workflow
  builds and attaches macOS (arm64 + x64) and Linux (x64, static musl) binaries —
  each with a SHA-256 checksum — to every tag's GitHub Release. `install-binary.sh`
  downloads and verifies the right one for your platform:
  `curl -fsSL …/install-binary.sh | sh`.
- **Homebrew** (`Formula/l0-cache.rb`):
  `brew tap fabriziosalmi/l0-cache https://github.com/fabriziosalmi/l0-cache` then
  `brew install l0-cache`.
- Cargo.toml `repository`/`homepage` metadata.

## [0.1.7] - 2026-06-05

### Added
- **Transparent multi-format config — still zero new dependencies.** The optional
  per-command config file is now auto-detected as
  `config.{json,toml,yaml,yml,conf,ini}` in `$XDG_CONFIG_HOME/l0-cache/` (or
  `~/.config/l0-cache/`). JSON is parsed strictly via serde; TOML/YAML/INI share a
  small built-in flat parser (the schema is flat, so all three styles reduce to the
  same shape). `[*]` is an alias for `[defaults]`; unknown keys and unparseable
  lines are skipped. No new crates were added.

## [0.1.6] - 2026-06-05

Hardening pass from a full file-by-file/test-by-test audit.

### Fixed
- **Terminal-injection hardening.** `--stats` and `--discover` now strip control
  characters from the (user-writable) metrics file's command names, so a crafted
  `metrics.jsonl` can no longer drive the terminal with raw ANSI escapes. `--json`
  was already safe (serde escapes them).
- **Recovery file is now PID-scoped** (`recovery-<cmd>-<pid>.log`), so concurrent
  runs of the same command (multi-agent, `make -j`, parallel CI) no longer collide
  on and corrupt one shared file. A mid-stream I/O error no longer leaves a partial
  recovery file behind.
- **Diff-context collapsing is byte-bounded**, not just line-bounded — restoring the
  strict bounded-memory guarantee (1 MB-capped lines could otherwise buffer GBs). The
  hunk-header detector now requires the space real `@@ … @@` headers have (no false
  activation on `@@@@`).
- **Stats robustness:** `--cost-per-mtok inf`/`nan` no longer emit `null` cost fields
  in `--json`; displayed efficiency is clamped to 100% (a corrupt file can't show
  500%); `--discover` truncates long command names so columns stay aligned.
- **Shell installers:** `help` no longer leaks script internals (`set`/vars/dividers);
  `agent-rules.sh` no longer accumulates a blank line per install→remove cycle; the
  hook installers remove their `jq` scratch file even on an early-exit failure.

### Added
- Tests for all of the above, plus the previously-uncovered stats path (`pct`/`usd`/
  `cost_shown`, control-char sanitization), the DiffCollapse↔pipeline integration,
  and config precedence (explicit CLI flag > config > default). 205 unit + 21
  integration tests.

## [0.1.5] - 2026-06-05

### Added
- **Format-aware diff compression.** A new streaming filter stage collapses long
  runs of *unchanged context* in unified diffs (keeping file/hunk headers and every
  `+`/`-` line) to `… (N unchanged diff lines) …`. It activates only after a real
  `@@ … @@` hunk header, so non-diff output — even indented text — is never touched.
- **`--discover`** — an optimization advisory from your metrics: which prefixed
  commands are paying off (keep), which to consider dropping (low savings over
  enough runs), and the biggest raw-token footprint.
- **`--json`** — emit `--stats` as a single JSON object (totals + per-command array)
  for tooling.
- **`--cost-per-mtok <N>`** — when > 0, show estimated USD cost saved in `--stats`
  and `--discover` (and `usd_saved` in `--json`).
- **`agent-rules.sh`** — installs a project rule ("prefix noisy commands with
  `l0-cache`") for agents whose hook cannot rewrite a command (Cursor, Cline,
  Copilot, Codex). Best-effort prompt-injection, complementing the transparent
  `agent-hook.sh` (Claude Code, Gemini CLI).
- README "How it compares" section positioning l0-cache against rtk / snip / Lean Ctx.

## [0.1.4] - 2026-06-05

### Added
- **`--recover`**: on a failing command whose output was truncated, the full
  un-truncated output is saved to a temp file and the banner points the agent at
  it, so it can read the omitted middle without re-running. Lazy (no disk for
  small/under-threshold output), memory- and size-bounded, and fail-safe (any I/O
  error is ignored). Off by default.
- **Per-command config file** at `$XDG_CONFIG_HOME/l0-cache/config.json` (or
  `~/.config/l0-cache/config.json`): optional per-command overrides for `head`,
  `tail`, `tail_error`, `threshold`, `only_errors`, and `recover`, with a
  `defaults` block. Precedence is explicit CLI flag > config > built-in default;
  commands match by resolved name; malformed/unknown keys are ignored gracefully.
- **`agent-hook.sh`**: a generalized transparent-hook installer covering the
  agents whose hook API can rewrite a command — **Claude Code** (`PreToolUse`) and
  **Gemini CLI** (`BeforeTool`/`run_shell_command`) — with the same conservative,
  fail-safe wrapper (and `--recover` enabled). `claude-hook.sh` stays as a
  Claude-only convenience. Cursor's hook can only allow/deny (no rewrite), so it
  cannot be wrapped transparently.

## [0.1.3] - 2026-06-05

### Added
- **Color/UI overhaul.** `--stats` is now a dense, boxed dashboard (summary card +
  per-command table with proportional efficiency bars and `↑ best` / `⚠ low`
  markers), and `--doctor` is re-skinned to share the same boxed visual language.
- **`NO_COLOR` / TTY color guard.** `--stats` and `--doctor` emit ANSI only on an
  interactive terminal; piping or redirecting now yields clean, escape-free text.
  `NO_COLOR` (any value) and `TERM=dumb` force color off; `FORCE_COLOR` /
  `CLICOLOR_FORCE` force it on for CI captures.
- README status badges (CI, latest release, license, MSRV) and a declared
  `rust-version = "1.85"` so the MSRV is enforced by cargo.

### Changed
- `benchmark.sh` rewritten with a boxed header and a comparison table (lines, size,
  estimated tokens, reduction %); `install.sh` and `claude-hook.sh` now honor the
  `NO_COLOR` / TTY color guard.

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
