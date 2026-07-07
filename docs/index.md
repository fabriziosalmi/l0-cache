---
layout: home

hero:
  name: l0-compressor
  text: CLI proxy for LLM token savings
  tagline: Reduce AI coding assistant token consumption by 50-80% with a single command prefix.
  image:
    src: /screenshot.png
    alt: l0-compressor --stats telemetry dashboard
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: Architecture
      link: /internals/architecture

features:
  - title: Universal Filtering
    details: Works with any command. No per-tool parsers and no alias rewriting -- just prefix with l0-compressor, or enable the optional Claude Code / Gemini CLI hook.
  - title: Adaptive Auto-tuning
    details: Six rules adjust head/tail budgets per command from execution history, and persist what they learn between runs.
  - title: Safety Guard
    details: Refuses rm -rf on system paths and HOME, reverse shells, credential exfiltration, DROP DATABASE. 161-case adversarial test suite.
  - title: Zero Overhead
    details: Single-threaded, synchronous Rust binary. Sub-millisecond overhead. 700 KB on disk.
  - title: Production Hardened
    details: UTF-8 lossy reads, OOM protection, SIGPIPE handling, POSIX exit codes, metrics rotation, telemetry redaction.
  - title: Cross-Platform
    details: macOS, Ubuntu, Alpine, LXC, Proxmox, VPS. Static musl build for containers.
---
