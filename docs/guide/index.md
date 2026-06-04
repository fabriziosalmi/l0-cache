# Introduction

`l0-cache` is a lightweight CLI proxy that sits between your AI coding assistant and
shell commands. It filters, truncates, and compresses command output to reduce
token consumption by 50-80%.

## Why

AI coding assistants (Claude Code, Gemini CLI, Cursor) read the full output of
every shell command they execute. Most of this output is noise:

- Hundreds of passing test lines when only the failures matter
- ANSI color codes that consume tokens but carry no semantic value
- Repeated identical lines from build tools
- Blank lines that waste token budget

The useful information is almost always concentrated in the first few lines
(headers, command echo) and the last few lines (errors, summary, exit status).

## How

Instead of running:

```sh
cargo test
```

Run:

```sh
l0-cache cargo test
```

The output is filtered through a streaming pipeline:

1. **ANSI stripping** -- removes color codes, cursor movement, etc.
2. **Identical line collapsing** -- repeated lines become `(×N)`
3. **Blank line squeezing** -- consecutive blanks reduced to one
4. **Head/tail buffering** -- keeps first 30 and last 30 lines, discards the middle

The result is a compact output that preserves the information an LLM needs
to understand what happened.

## Design Principles

- **Universal**: works with any command, no per-tool parsers
- **Unintrusive**: the binary never rewrites your shell, aliases, or `$PATH` --
  you invoke it explicitly. An optional, off-by-default
  [Claude Code hook](./claude-code) can do the prefixing for you, and is just as
  conservative about what it touches.
- **Safe**: never modifies the child command's behavior
- **Lightweight**: single Rust binary, zero runtime dependencies
- **Observable**: every invocation logs metrics for analysis
