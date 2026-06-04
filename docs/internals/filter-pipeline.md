# Filter Pipeline

The filter pipeline processes lines in streaming fashion. Each line passes
through four stages before reaching the head/tail buffer.

## Stages

### 1. ANSI Stripping

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

### 2. Line Collapsing (Identical & Prefix-based)

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
  Compiling l0-cache v0.1.0
```

Output:
```
  Compiling ... (×3)
```

The collapsed output is emitted when the next *non-matching* line arrives (or at EOF), ensuring streaming correctness.

### 3. Whitespace Squeezing

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

### 4. Head/Tail Buffer

The core data structure. Maintains two fixed-size buffers:

- **head**: first N lines (default 30)
- **tail**: circular buffer of last M lines (default 30)

When the total line count exceeds the threshold (default 100), the middle
is discarded and replaced with a banner:

```
... [370 lines omitted for LLM] ...
```

On non-zero exit, the tail buffer is expanded to 120 lines (configurable)
before rendering, to ensure error messages and stack traces are preserved.

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

Before the pipeline processes any lines, the first 8 KB of output are
checked for null bytes. If null bytes are found, the output is classified
as binary and passed through without filtering (with a lossy UTF-8
conversion for display).
