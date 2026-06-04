---
layout: home

hero:
  name: l0-cache
  text: CLI proxy for LLM token savings
  tagline: Reduce AI coding assistant token consumption by 50-80% with a single command prefix.
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: Architecture
      link: /internals/architecture

features:
  - title: Universal Filtering
    details: Works with any command. No per-tool parsers, no shell hooks, no aliases. Prefix with l0-cache.
  - title: Zero Overhead
    details: Single-threaded, synchronous Rust binary. Sub-millisecond overhead. 700 KB on disk.
  - title: Production Hardened
    details: UTF-8 lossy reads, OOM protection, SIGPIPE handling, POSIX exit codes, metrics rotation.
  - title: Cross-Platform
    details: macOS, Ubuntu, Alpine, LXC, Proxmox, VPS. Static musl build for containers.
---
