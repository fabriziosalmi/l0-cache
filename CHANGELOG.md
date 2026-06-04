# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
