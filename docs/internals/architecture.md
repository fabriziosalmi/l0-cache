# Architecture

## Overview

`l0-cache` is a single-threaded, synchronous Rust binary. It spawns a child process
via `sh -c '<command> 2>&1'`, reads the merged output line by line, filters
it through a streaming pipeline, and prints the result.

```
                    +---------------------+
                    |  l0-cache (parent)  |
                    |                     |
  stdin (inherited) |  +---------------+  |  stdout
  ------------------->  | sh -c '..    |  +----------->
                    |  | 2>&1'         |  |
                    |  +------+--------+  |
                    |         |           |
                    |  pipe (stdout)      |
                    |         |           |
                    |  +------v--------+  |
                    |  | read_line     |  |  (UTF-8 lossy, 1MB cap)
                    |  +------+--------+  |
                    |         |           |
                    |  +------v--------+  |
                    |  | FilterPipe    |  |  (streaming, O(head+tail))
                    |  +------+--------+  |
                    |         |           |
                    |  +------v--------+  |
                    |  | render()      |  |
                    |  +------+--------+  |
                    |         |           |
                    |  +------v--------+  |
                    |  | metrics.jsonl |  |  (O_APPEND, 0600)
                    |  | tuned.jsonl   |  |  (per-bucket adaptive tune,
                    |  +---------------+  |   read at start, append on firing)
                    +---------------------+
```

## Why `sh -c '... 2>&1'`

The `2>&1` merge is the foundation of the architecture. Without it:

- Two pipes (stdout, stderr) require either threads or async to drain
  concurrently, or else the child blocks when one pipe's 64 KB buffer fills.
- Separate pipes destroy the interleaving order, which is exactly what an LLM
  needs to understand *when* an error occurred relative to other output.

By merging at the shell level, we get a single pipe that preserves the real
output order, drained synchronously from the main thread.

## Modules

| Module | File | Responsibility |
|---|---|---|
| `args` | `src/args.rs` | CLI argument parsing via clap |
| `filter` | `src/filter.rs` | ANSI strip, collapse, squeeze, head/tail buffer |
| `runner` | `src/runner.rs` | Process spawning, line reading, exit code |
| `telemetry` | `src/telemetry/` | JSONL metrics, stats reporting, adaptive learner + `tuned.jsonl`; split into focused submodules (`metric`, `stats/`, `adaptive`, `tuned`, `doctor`, `lock`, `paths`) behind `mod.rs` |
| `main` | `src/main.rs` | Orchestration, signal handling, output writing |

## Execution Modes

1. **Filtered** (default): output goes through `FilterPipeline`, truncated
   at head+tail.
2. **Raw** (`--raw`): output collected in full (up to 256 MB), ANSI stripped,
   no truncation.
3. **Passthrough** (`-i` or auto-detected): stdio inherited, no capture, no
   metrics.

## Memory Guarantees

| Mode | Memory bound | Notes |
|---|---|---|
| Filtered | O(head + tail) lines | Constant regardless of output size |
| Raw | 256 MB cap | Drains remaining output without storing |
| Passthrough | O(1) | No buffering at all |
