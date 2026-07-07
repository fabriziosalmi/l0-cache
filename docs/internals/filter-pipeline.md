# Filter Pipeline

The filter pipeline processes lines in streaming fashion. Each line is
normalized at read time, then passes through the pipeline stages —
diff-context collapse, line collapse, whitespace squeeze — before reaching
the head/tail buffer. The stages below are in execution order.

## Stages

### 1. Read-time Line Normalization (filtered mode only)

Applied while reading each line, before anything else, and skipped entirely
in `--raw` mode:

- **Progress-bar squashing** — a line containing interior carriage returns
  (a command redrawing the same line) keeps only the part after the last
  `\r`, emulating what a terminal would display.
- **Backspace / bell resolution** — `\b` and DEL erase the previous
  character (UTF-8 aware); BEL bytes are dropped.
- **Giant JSON line truncation** — a line over 2000 bytes whose first
  non-whitespace byte is `{` or `[` is cut at 2000 bytes with an explicit
  `... [Large JSON Payload Truncated for LLM] ...` marker instead of being
  forwarded whole.

### 2. ANSI Stripping

Removes all ANSI escape sequences (colors, cursor movement, bold, etc.)
using the `strip-ansi-escapes` crate. This is always applied, even in raw
mode.

Input:
```
\x1b[32mPASSED\x1b[0m test_something
```

Output:
```
PASSED test_something
```

### 3. Diff Context Collapsing

Inside a unified diff, long runs of *unchanged context lines* are collapsed to
a `… (N unchanged diff lines) …` marker. The stage activates only after a real
hunk header (`@@ … @@`), so non-diff output — even indented text whose lines
start with a space — is never touched. File and hunk headers and every
added/removed line are kept verbatim; only context runs longer than 8 lines
are collapsed, keeping 3 lines at each edge. The context buffer is bounded by
both a line cap (5000) and a byte cap (1 MB), so a pathological hunk cannot
grow memory unbounded. A non-diff line deactivates the stage.

### 4. Line Collapsing (Identical & Prefix-based)

To handle repetitive noise, the collapser runs in two modes:

#### Exact Identical Collapsing
When consecutive lines are exactly identical, they are collapsed into a single line with a count suffix `(×N)`.

Input:
```
Downloading crate...
Downloading crate...
Downloading crate...
Downloading crate...
```

Output:
```
Downloading crate... (×4)
```

#### Prefix-based Collapsing
When consecutive lines share the same first word (the "prefix"), they are collapsed into a prefix summary showing the prefix, an ellipsis, and the count suffix `... (×N)`. This is highly effective for compiler progress and package downloaders (e.g., `Compiling`, `Downloading`).
Prefix-based collapsing requires the prefix to be at least 2 characters long (to avoid collapsing on bullets like `-` or `*`) and preserves any leading indentation.

Input:
```
  Compiling serde v1.0.1
  Compiling clap v4.0.0
  Compiling l0-compressor v0.2.0
```

Output:
```
  Compiling ... (×3)
```

The collapsed output is emitted when the next *non-matching* line arrives (or at EOF), ensuring streaming correctness.

### 5. Whitespace Squeezing

Consecutive blank lines are reduced to a single blank line. Lines containing
only whitespace are treated as blank.

Input:
```
section 1


section 2




section 3
```

Output:
```
section 1

section 2

section 3
```

### 6. Head/Tail Buffer

The core data structure. Maintains two fixed-size buffers:

- **head**: first N lines (default 30)
- **tail**: circular buffer of last M lines (default 30)

When the total line count exceeds the threshold (default 100), the middle
is discarded and replaced with a banner:

```
... [370 lines omitted for LLM] ...
```

The buffer retains `max(--tail, --tail-error)` lines while streaming (the tail
cannot be expanded retroactively once lines have been evicted). At render time,
on a **non-zero exit** the larger error tail (120 lines, configurable) is shown
so error messages and stack traces are preserved; on success the smaller tail
(30 lines) is shown.

#### Clean-success squelch

One more render-time gate applies to the success tail. If the exit is zero **and**
no error/warning signal was seen anywhere in the stream, the tail is trimmed
further — `squelched_tail(cap)` halves it with a floor of 5 (`30 → 15`, `8 → 5`,
`≤5` unchanged, never expanded). The error-signal tracking is *sticky* across the
whole stream (a signal in an evicted middle line still counts), so a single
`warning:` anywhere backs the squelch off to the full tail, and any non-zero exit
does the same. The head and the final summary line always survive. The behavior is
on by default and shares the [`--only-errors`](/reference/#only-errors) keyword set;
see [Clean-success squelch](/guide/configuration#clean-success-squelch) for the
user-facing controls. The runner returns the *rendered* tail count so the
truncation banner reports the truth (`30 head + 15 tail`), not the configured cap.

### Memory Layout

```
+--------+---------------------------+--------+
| head   |  (discarded, not stored)  |  tail  |
| 30 ln  |                           | 30 ln  |
+--------+---------------------------+--------+
     ^               ^                    ^
     |               |                    |
  Vec<String>    not in memory       VecDeque<String>
  (fixed)        (counter only)      (circular, capped)
```

Total memory: O(head_cap + tail_cap) strings, regardless of whether the
command produces 100 lines or 10 million.

## Binary Detection

The first ~8 KB of output are checked for null bytes **or** invalid UTF-8. If
either is found, the output is classified as binary. Rather than forward a
useless, token-expensive blob, `l0-compressor` emits the sniffed first ~8 KB (lossy
UTF-8) and, when the stream was larger, an explicit banner:

```
... [l0-compressor: binary output detected — showing first 8192 of 1048576 bytes] ...
```

The metric records `strategy: "binary_skip"` and `truncated: true` when bytes
were dropped, so binary output is never silently presented as if complete.
