//! Filter engine: streaming-safe pipeline for output reduction.
//!
//! Pipeline order: ANSI strip → collapse-identical → whitespace squeeze → head/tail buffer.
//! All filters work line-by-line with O(head+tail) RAM.

use std::borrow::Cow;
use std::collections::VecDeque;

// ── Defaults ────────────────────────────────────────────────────────────────

pub const DEFAULT_HEAD: usize = 30;
pub const DEFAULT_TAIL: usize = 30;
pub const DEFAULT_TAIL_ERROR: usize = 120;
/// Only truncate if total lines exceed this threshold.
pub const DEFAULT_THRESHOLD: usize = 100;

/// Clean-success squelch: on a zero exit with no error/warning signal anywhere
/// in the stream, the displayed tail is trimmed to `tail / DIVISOR` — the middle
/// of a clean build/test/install is almost always progress noise. The head
/// (command echo) is left intact and failures are never squelched.
pub const SUCCESS_SQUELCH_TAIL_DIVISOR: usize = 2;
/// Never squelch the tail below this many lines: the final summary line(s) of a
/// clean run (e.g. "Finished in 3.2s", "PASSED") must always survive.
pub const SUCCESS_SQUELCH_MIN_TAIL: usize = 5;

/// Reduced tail to display for a clean, signal-free success. Never larger than
/// `tail_cap`, so an explicitly tiny `--tail` is honored rather than expanded.
pub fn squelched_tail(tail_cap: usize) -> usize {
    (tail_cap / SUCCESS_SQUELCH_TAIL_DIVISOR)
        .max(SUCCESS_SQUELCH_MIN_TAIL)
        .min(tail_cap)
}

// ── ANSI Strip ──────────────────────────────────────────────────────────────

/// Remove all ANSI escape sequences from a byte slice, returning a String.
/// If the input isn't valid UTF-8 after stripping, returns the lossy version.
pub fn strip_ansi(input: &[u8]) -> Cow<'_, str> {
    // Fast path: no ESC byte → no ANSI escapes possible
    if !input.contains(&0x1b) {
        return String::from_utf8_lossy(input);
    }
    let stripped = strip_ansi_escapes::strip(input);
    match String::from_utf8(stripped) {
        Ok(s) => Cow::Owned(s),
        Err(e) => Cow::Owned(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

/// Case-insensitive ASCII substring search that does not allocate.
/// `needle` is expected to be lowercase ASCII. Used on the hot path instead of
/// `line.to_lowercase().contains(..)`, which allocated a String for every line.
fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let (hb, nb) = (haystack.as_bytes(), needle.as_bytes());
    if nb.is_empty() {
        return true;
    }
    if hb.len() < nb.len() {
        return false;
    }
    (0..=hb.len() - nb.len()).any(|i| hb[i..i + nb.len()].eq_ignore_ascii_case(nb))
}

/// Keywords that mark a line as carrying error/warning signal. Shared by the
/// `--only-errors` filter and the clean-success squelch gate so both agree on
/// what "this output looks problematic" means.
const ERROR_SIGNAL_KEYWORDS: [&str; 7] = [
    "error",
    "warn",
    "fail",
    "exception",
    "panic",
    "traceback",
    "fatal",
];

/// Whether `line` carries any error/warning signal keyword (case-insensitive).
pub fn has_error_signal(line: &str) -> bool {
    ERROR_SIGNAL_KEYWORDS
        .iter()
        .any(|kw| contains_ascii_ci(line, kw))
}

// ── Line Collapse (Identical + Prefix-based) ────────────────────────────────

const MIN_PREFIX_LEN: usize = 2;

/// Skip common timestamp prefixes to find the actual "first word".
fn skip_timestamp(line: &str) -> &str {
    let s = line.trim_start();

    // Check syslog: "Oct 12 10:30:00 " (16 chars)
    if s.len() >= 15 {
        let month = &s[0..3];
        let is_month = matches!(
            month,
            "Jan"
                | "Feb"
                | "Mar"
                | "Apr"
                | "May"
                | "Jun"
                | "Jul"
                | "Aug"
                | "Sep"
                | "Oct"
                | "Nov"
                | "Dec"
        );
        if is_month {
            let bytes = s.as_bytes();
            // simple check for time colon: e.g. "Oct 12 10:30:00" -> bytes[12] or bytes[13] == ':'
            if bytes.len() > 14 && (bytes[12] == b':' || bytes[13] == b':') {
                return s[15..].trim_start();
            }
        }
    }

    // Check ISO8601 or similar (starts with digit)
    if let Some(c) = s.chars().next() {
        if c.is_ascii_digit() {
            let mut parts = s.splitn(3, |c: char| c.is_whitespace());
            let t1 = parts.next().unwrap_or("");
            let is_date = t1.contains('-') || t1.contains('/');
            let is_time = t1.contains(':');

            if is_date || is_time {
                // Full ISO timestamp: 2024-10-12T10:30:00Z
                if t1.contains('T') && t1.contains(':') {
                    return s[t1.len()..].trim_start();
                }

                // Just a date, maybe next token is a time
                if let Some(t2) = parts.next() {
                    let is_t2_time = t2.contains(':') && t2.chars().any(|ch| ch.is_ascii_digit());
                    if is_t2_time {
                        let idx = s.find(t2).unwrap() + t2.len();
                        return s[idx..].trim_start();
                    }
                }

                // Else skip just t1
                return s[t1.len()..].trim_start();
            }
        }
    }

    s
}

/// Extract the first whitespace-delimited word from a line (the "prefix"), ignoring timestamps.
/// Returns `None` for blank or whitespace-only lines.
fn first_word(line: &str) -> Option<&str> {
    skip_timestamp(line).split_whitespace().next()
}

/// Extract prefix including leading indentation
fn prefix_with_indent(line: &str) -> Option<&str> {
    let word = line.split_whitespace().next()?;
    let idx = line.find(word)?;
    Some(&line[..idx + word.len()])
}

/// Two-mode collapser for consecutive similar lines:
///
/// 1. **Identical**: exact string match → `line (×N)`
/// 2. **Prefix**: same first word → shows prefix and count
///    e.g. "  Compiling serde v1.0" ... "  Compiling clap v4.0" → "  Compiling ... (×2)"
///
/// Prefix mode catches: cargo Compiling, npm Downloading, test runners,
/// docker log lines with timestamps, CI pipeline stage output, etc.
pub struct CollapseLines {
    last_line: Option<String>,
    repeat_count: usize,
    /// When in prefix-collapse mode, the first line of the run
    first_in_run: Option<String>,
    /// The prefix (first word) that's being collapsed
    run_prefix: Option<String>,
}

impl Default for CollapseLines {
    fn default() -> Self {
        Self::new()
    }
}

impl CollapseLines {
    pub fn new() -> Self {
        Self {
            last_line: None,
            repeat_count: 0,
            first_in_run: None,
            run_prefix: None,
        }
    }

    /// Feed a line. Returns emitted lines (0, 1, or 2) via callback-style Option<String>.
    /// The caller must call `flush()` at EOF to get remaining pending lines.
    /// Takes ownership only when buffering a new line.
    pub fn feed(&mut self, line: Cow<'_, str>) -> Option<String> {
        match &self.last_line {
            // Case 1: Exact identical match (works for both identical-run and prefix-run)
            Some(prev) if prev == line.as_ref() => {
                self.repeat_count += 1;
                None
            }
            // Case 2: Same prefix (first word) — prefix-collapse
            Some(prev) if self.same_prefix(prev, line.as_ref()) => {
                if self.first_in_run.is_none() {
                    // Start prefix run. Move the first line of the run to first_in_run.
                    self.first_in_run = self.last_line.take();
                    self.run_prefix = first_word(self.first_in_run.as_deref().unwrap_or(""))
                        .map(|s| s.to_string());
                }
                self.repeat_count += 1;
                self.last_line = Some(line.into_owned());
                None
            }
            // Case 3: Different line — emit pending, start new
            _ => {
                let emit = self.emit_pending();
                self.last_line = Some(line.into_owned());
                self.repeat_count = 1;
                emit
            }
        }
    }

    /// Flush remaining buffered lines at EOF.
    pub fn flush(&mut self) -> Option<String> {
        self.emit_pending()
    }

    /// Check if two lines share the same first word (prefix) or are fuzzily identical.
    /// Returns false for blank lines or single-char prefixes (too noisy).
    fn same_prefix(&self, a: &str, b: &str) -> bool {
        if let (Some(wa), Some(wb)) = (first_word(a), first_word(b)) {
            if wa.len() >= MIN_PREFIX_LEN && wa == wb {
                return true;
            }
        }

        // --- 80/20 Fuzzy Line Collapse ---
        // If the first word differs (e.g. dynamic hashes or IDs at the start),
        // we extract the first 40 characters, keep ONLY alphabetic letters, and compare.
        // This instantly deduplicates lines like:
        // "[info] 123 processing" vs "[info] 456 processing" -> "infoprocessing"
        let extract_fuzzy = |s: &str| {
            s.chars()
                .take(40)
                .filter(|c| c.is_alphabetic())
                .collect::<String>()
        };

        let fa = extract_fuzzy(a);
        let fb = extract_fuzzy(b);
        fa.len() >= 10 && fa == fb
    }

    fn emit_pending(&mut self) -> Option<String> {
        if let Some(first) = self.first_in_run.take() {
            let count = self.repeat_count;
            self.repeat_count = 0;
            self.last_line = None;
            // Show the meaningful (timestamp-skipped) prefix with the original
            // indentation, e.g. "  Compiling ... (×N)" or "[INFO] ... (×N)" rather
            // than collapsing onto the leading timestamp token.
            let prefix = match self.run_prefix.take() {
                Some(word) => {
                    let indent = &first[..first.len() - first.trim_start().len()];
                    format!("{}{}", indent, word)
                }
                None => prefix_with_indent(&first).unwrap_or(&first).to_string(),
            };
            Some(format!("{} ... (×{})", prefix, count))
        } else if let Some(line) = self.last_line.take() {
            let count = self.repeat_count;
            self.repeat_count = 0;
            if count > 1 {
                Some(format!("{} (×{})", line, count))
            } else {
                Some(line)
            }
        } else {
            None
        }
    }
}

// ── Whitespace Squeeze ──────────────────────────────────────────────────────

/// State tracker for squeezing consecutive blank lines.
pub struct WhitespaceSqueeze {
    consecutive_blanks: usize,
}

impl Default for WhitespaceSqueeze {
    fn default() -> Self {
        Self::new()
    }
}

impl WhitespaceSqueeze {
    pub fn new() -> Self {
        Self {
            consecutive_blanks: 0,
        }
    }

    /// Feed a borrowed line. Returns `Some(line)` if it should be emitted.
    /// Test-only; production uses [`Self::feed_owned`].
    #[cfg(test)]
    pub fn feed<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        if line.trim().is_empty() {
            self.consecutive_blanks += 1;
            if self.consecutive_blanks <= 1 {
                Some(line)
            } else {
                None // suppress extra blanks
            }
        } else {
            self.consecutive_blanks = 0;
            Some(line)
        }
    }

    /// Feed an owned line. Returns `Some(line)` if the line should be emitted.
    pub fn feed_owned(&mut self, line: String) -> Option<String> {
        if line.trim().is_empty() {
            self.consecutive_blanks += 1;
            if self.consecutive_blanks <= 1 {
                Some(line)
            } else {
                None // suppress extra blanks
            }
        } else {
            self.consecutive_blanks = 0;
            Some(line)
        }
    }
}

// ── Head/Tail Ring Buffer ───────────────────────────────────────────────────

/// Streaming head/tail buffer that stores the first `head_cap` lines
/// and the last `tail_cap` lines with O(head+tail) memory.
pub struct HeadTailBuffer {
    head_cap: usize,
    tail_cap: usize,
    head: Vec<String>,
    tail: VecDeque<String>,
    total_lines: usize,
}

const PREALLOCATION_CAP: usize = 1024;

impl HeadTailBuffer {
    pub fn new(head_cap: usize, tail_cap: usize) -> Self {
        // Cap pre-allocation to avoid capacity overflow with usize::MAX (raw mode).
        // The actual buffer grows on-demand; with_capacity is a hint.
        let head_prealloc = head_cap.min(PREALLOCATION_CAP);
        let tail_prealloc = tail_cap.min(PREALLOCATION_CAP);
        Self {
            head_cap,
            tail_cap,
            head: Vec::with_capacity(head_prealloc),
            tail: VecDeque::with_capacity(tail_prealloc.saturating_add(1)),
            total_lines: 0,
        }
    }

    /// Feed a filtered line into the buffer.
    pub fn push(&mut self, line: String) {
        self.total_lines += 1;

        if self.head.len() < self.head_cap {
            self.head.push(line);
        } else if self.tail_cap > 0 {
            if self.tail.len() == self.tail_cap {
                self.tail.pop_front();
            }
            self.tail.push_back(line);
        }
        // else: tail_cap == 0 → drop the line (only counting)
    }

    /// Total lines seen so far.
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    /// Shrink the head budget and apply it RETROACTIVELY: any already-buffered
    /// head lines beyond `new_head_cap` are moved to the front of the tail (in
    /// order), so a mid-stream re-split (e.g. on a detected panic, where the
    /// useful context is at the bottom) actually reshapes what is rendered
    /// instead of only affecting future lines. Bounded by the tail capacity.
    pub fn rebalance_head(&mut self, new_head_cap: usize) {
        if self.head.len() > new_head_cap {
            let excess = self.head.split_off(new_head_cap);
            for line in excess.into_iter().rev() {
                self.tail.push_front(line);
            }
            while self.tail_cap > 0 && self.tail.len() > self.tail_cap {
                self.tail.pop_front();
            }
        }
        self.head_cap = new_head_cap;
    }

    /// How many lines are dropped from the final rendered output, given the
    /// `threshold` gate and how many tail lines we intend to *display*.
    ///
    /// The buffer may physically retain more tail lines than it displays (so that
    /// the same stream can serve a small success tail or a large error tail without
    /// a retroactive — and impossible — expansion). Below the threshold we show
    /// everything we retained; above it we show head + `display_tail`.
    fn omitted_count(&self, threshold: usize, display_tail: usize) -> usize {
        let head_count = self.head.len();
        if self.total_lines <= threshold {
            // Small output: show everything retained. Lines are only missing if the
            // stream exceeded what we could physically retain (head_cap + tail_cap).
            self.total_lines
                .saturating_sub(head_count + self.tail.len())
        } else {
            let shown_tail = display_tail.min(self.tail.len());
            self.total_lines.saturating_sub(head_count + shown_tail)
        }
    }

    /// Was the output actually truncated (any line omitted from what we render)?
    pub fn was_truncated(&self, threshold: usize, display_tail: usize) -> bool {
        self.omitted_count(threshold, display_tail) > 0
    }

    /// Render the final output. Returns (output_string, lines_final, bytes_final).
    /// Consumes self to avoid cloning head/tail lines.
    ///
    /// `display_tail` is how many of the retained tail lines to actually show. The
    /// runner passes the success tail on exit 0 and the (larger) error tail on a
    /// non-zero exit, while the buffer has retained `max(success, error)` all along.
    pub fn render(self, threshold: usize, display_tail: usize) -> (String, usize, usize) {
        let home_path = std::env::var("HOME").ok().filter(|h| !h.is_empty());
        let process_line = |mut s: String| -> String {
            if let Some(home) = &home_path {
                if s.contains(home) {
                    s = s.replace(home, "~");
                }
            }
            s
        };

        let omitted = self.omitted_count(threshold, display_tail);
        let below = self.total_lines <= threshold;
        // Below threshold we show every retained tail line; above it we trim to the
        // last `display_tail` lines (dropping the oldest retained tail entries).
        let shown_tail = if below {
            self.tail.len()
        } else {
            display_tail.min(self.tail.len())
        };
        let drop_from_tail = self.tail.len() - shown_tail;

        let mut parts: Vec<String> = self.head.into_iter().map(process_line).collect();

        if omitted > 0 {
            parts.push(String::new());
            parts.push(format!("... [{} lines omitted for LLM] ...", omitted));
            parts.push(String::new());
        }

        parts.extend(self.tail.into_iter().skip(drop_from_tail).map(process_line));

        let joined = parts.join("\n");
        let lines = parts.len();
        let bytes = joined.len();
        (joined, lines, bytes)
    }
}

// ── Unified-diff context collapsing ─────────────────────────────────────────

/// Keep this many context lines at each edge of a long unchanged run.
const DIFF_CTX_KEEP: usize = 3;
/// Only collapse a context run longer than this (so short runs stay verbatim).
const DIFF_CTX_MIN: usize = 8;
/// Flush the context buffer if it reaches this many lines OR this many bytes,
/// so a pathological hunk can't grow memory unbounded (lines are 1 MB-capped, so a
/// line-only cap could still buffer gigabytes — the byte cap is the real bound).
const DIFF_CTX_CAP: usize = 5000;
const DIFF_CTX_BYTE_CAP: usize = 1024 * 1024;

/// `@@ -a,b +c,d @@` hunk header. Requires the space after `@@` that every real
/// unified-diff hunk header has, so a line like `@@@@` does not falsely activate.
fn is_hunk_header(l: &str) -> bool {
    l.starts_with("@@ ") && l[3..].contains("@@")
}

/// A line that still belongs to a unified diff (so seeing it does not end the diff).
fn is_diffish(l: &str) -> bool {
    matches!(
        l.as_bytes().first(),
        Some(b'+' | b'-' | b' ' | b'@' | b'\\')
    ) || l.starts_with("diff ")
        || l.starts_with("index ")
}

/// Streaming, format-aware collapse of *unchanged context* in unified diffs.
///
/// It activates only after a real hunk header (`@@ … @@`), so non-diff output —
/// even indented text whose lines start with a space — is never touched. Within a
/// diff it keeps the file/hunk headers and every added/removed line, but collapses
/// long runs of unchanged context lines to `… (N unchanged diff lines) …`, which is
/// exactly the noise an agent does not need.
struct DiffCollapse {
    active: bool,
    ctx: Vec<String>,
    ctx_bytes: usize,
}

impl DiffCollapse {
    fn new() -> Self {
        Self {
            active: false,
            ctx: Vec::new(),
            ctx_bytes: 0,
        }
    }

    fn feed(&mut self, line: String) -> Vec<String> {
        let mut out = Vec::new();
        if is_hunk_header(&line) {
            self.flush_into(&mut out);
            self.active = true;
            out.push(line);
            return out;
        }
        if self.active && line.starts_with(' ') {
            self.ctx_bytes += line.len();
            self.ctx.push(line);
            // Bound memory on a pathological hunk (by lines AND bytes).
            if self.ctx.len() >= DIFF_CTX_CAP || self.ctx_bytes >= DIFF_CTX_BYTE_CAP {
                self.flush_into(&mut out);
            }
            return out;
        }
        self.flush_into(&mut out);
        // A non-diff line ends the diff section, so later text is left alone.
        if self.active && !is_diffish(&line) {
            self.active = false;
        }
        out.push(line);
        out
    }

    fn flush(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.flush_into(&mut out);
        out
    }

    fn flush_into(&mut self, out: &mut Vec<String>) {
        let n = self.ctx.len();
        if n == 0 {
            return;
        }
        self.ctx_bytes = 0; // both branches below empty `ctx`
        if n <= DIFF_CTX_MIN {
            out.append(&mut self.ctx);
            return;
        }
        let ctx = std::mem::take(&mut self.ctx);
        out.extend(ctx[..DIFF_CTX_KEEP].iter().cloned());
        out.push(format!(
            " ... ({} unchanged diff lines) ...",
            n - DIFF_CTX_KEEP * 2
        ));
        out.extend(ctx[n - DIFF_CTX_KEEP..].iter().cloned());
    }
}

// ── Streaming Filter Pipeline ───────────────────────────────────────────────

/// The full streaming pipeline. Feed lines one by one, then call `finish()`.
pub struct FilterPipeline {
    diff: DiffCollapse,
    collapse: CollapseLines,
    squeeze: WhitespaceSqueeze,
    buffer: HeadTailBuffer,
    only_errors: bool,
    auto_tuned: bool,
    /// Sticky: set once any error/warning signal is seen in the stream. Gates
    /// the clean-success squelch so a problematic run keeps its full tail.
    saw_error_signal: bool,
}

impl FilterPipeline {
    /// `tail_cap` is the number of tail lines to *retain* while streaming. The
    /// runner sizes this to `max(success_tail, error_tail)` so the rendered tail
    /// can grow on a non-zero exit without an (impossible) retroactive expansion.
    /// The actual number of tail lines shown is chosen at [`Self::finish`].
    pub fn new(head_cap: usize, tail_cap: usize, only_errors: bool) -> Self {
        Self {
            diff: DiffCollapse::new(),
            collapse: CollapseLines::new(),
            squeeze: WhitespaceSqueeze::new(),
            buffer: HeadTailBuffer::new(head_cap, tail_cap),
            only_errors,
            auto_tuned: false,
            saw_error_signal: false,
        }
    }

    /// Whether any error/warning signal appeared in the stream. The clean-success
    /// squelch consults this: a signal anywhere → keep the full success tail.
    pub fn saw_error_signal(&self) -> bool {
        self.saw_error_signal
    }

    /// Push one already-diff-processed line through collapse → squeeze → buffer.
    fn push_through(&mut self, line: String) {
        if let Some(collapsed) = self.collapse.feed(Cow::Owned(line)) {
            if let Some(squeezed) = self.squeeze.feed_owned(collapsed) {
                self.buffer.push(squeezed);
            }
        }
    }

    /// Feed a raw line (already ANSI-stripped). Applies collapse + squeeze + buffer.
    /// Uses Cow to avoid cloning through the pipeline.
    pub fn feed(&mut self, line: Cow<'_, str>) {
        // Track error/warning signal across the WHOLE stream — even lines later
        // truncated away. The clean-success squelch must back off if anything
        // problematic appeared, so the agent keeps the full tail to inspect it.
        if !self.saw_error_signal && has_error_signal(line.as_ref()) {
            self.saw_error_signal = true;
        }

        // --- 80/20 only_errors Filter ---
        if self.only_errors && !has_error_signal(line.as_ref()) {
            return;
        }

        // --- 80/20 Auto-Tuning Ecosystem Heuristics ---
        // When a crash signature appears, reshape the head/tail split retroactively.
        // The tail is already sized to retain a large error window (see runner),
        // so we only need to decide how much HEAD to keep.
        if !self.auto_tuned {
            let l = line.as_ref();
            if contains_ascii_ci(l, "traceback (most recent call last)")
                || contains_ascii_ci(l, "panicked at")
            {
                // Python/Rust: the useful context (the actual error) is at the bottom.
                // Keep far fewer head lines and let the tail carry the trace.
                let new_head = (self.buffer.head_cap / 5).max(3); // ~20%, floor 3
                self.buffer.rebalance_head(new_head);
                self.auto_tuned = true;
            } else if contains_ascii_ci(l, "exception in thread") {
                // Java: the exception header is at the TOP; the existing head already
                // captures it. Nothing to rebalance — just stop re-checking.
                self.auto_tuned = true;
            }
        }

        // Step 1: Diff-aware context collapsing (one input line may yield several
        // output lines when a buffered context run is flushed). Step 2: collapse
        // identical consecutive. Step 3: whitespace squeeze. Step 4: head/tail buffer.
        for piece in self.diff.feed(line.into_owned()) {
            self.push_through(piece);
        }
    }

    /// Finalize: flush pending collapse state, return rendered output and stats.
    ///
    /// `raw_bytes_override`: true raw byte count from runner (pre-filter), for accurate metrics.
    /// `display_tail`: how many tail lines to show (success tail vs. error tail).
    pub fn finish(
        mut self,
        threshold: usize,
        raw_bytes_override: usize,
        display_tail: usize,
    ) -> FilterResult {
        // Flush any pending diff-context run first, then the collapse buffer.
        for piece in self.diff.flush() {
            self.push_through(piece);
        }
        if let Some(last) = self.collapse.flush() {
            if let Some(squeezed) = self.squeeze.feed_owned(last) {
                self.buffer.push(squeezed);
            }
        }

        let total_lines_raw = self.buffer.total_lines();
        let truncated = self.buffer.was_truncated(threshold, display_tail);
        let (output, lines_final, bytes_final) = self.buffer.render(threshold, display_tail);

        FilterResult {
            output,
            lines_raw: total_lines_raw,
            lines_final,
            bytes_raw: raw_bytes_override,
            bytes_final,
            truncated,
        }
    }
}

/// Result of the full filter pipeline.
pub struct FilterResult {
    pub output: String,
    pub lines_raw: usize,
    pub lines_final: usize,
    pub bytes_raw: usize,
    pub bytes_final: usize,
    pub truncated: bool,
}

// ── Binary Detection ────────────────────────────────────────────────────────

const BINARY_CHECK_BYTES: usize = 8192;

/// Check if a byte slice looks like binary content (contains null bytes
/// or isn't valid UTF-8). Check the first ~8KB.
pub fn looks_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(BINARY_CHECK_BYTES);
    let slice = &data[..check_len];

    // Null bytes = almost certainly binary
    if slice.contains(&0) {
        return true;
    }

    // Not valid UTF-8 = probably binary
    std::str::from_utf8(slice).is_err()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ANSI strip tests ────────────────────────────────────────────────

    #[test]
    fn strip_ansi_removes_colors() {
        let input = b"\x1b[31mERROR\x1b[0m: something failed";
        let result = strip_ansi(input);
        assert_eq!(result, "ERROR: something failed");
    }

    #[test]
    fn strip_ansi_passthrough_clean() {
        let input = b"no colors here";
        assert_eq!(strip_ansi(input), "no colors here");
    }

    // ── Collapse identical/prefix tests ──────────────────────────────────

    #[test]
    fn collapse_no_repeats() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("a".into()), None);
        assert_eq!(c.feed("b".into()), Some("a".into()));
        assert_eq!(c.feed("c".into()), Some("b".into()));
        assert_eq!(c.flush(), Some("c".into()));
    }

    #[test]
    fn collapse_with_repeats() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("warn: X".into()), None);
        assert_eq!(c.feed("warn: X".into()), None);
        assert_eq!(c.feed("warn: X".into()), None);
        assert_eq!(c.feed("other".into()), Some("warn: X (×3)".into()));
        assert_eq!(c.flush(), Some("other".into()));
    }

    #[test]
    fn collapse_repeats_at_end() {
        let mut c = CollapseLines::new();
        c.feed("a".into());
        c.feed("a".into());
        c.feed("a".into());
        assert_eq!(c.flush(), Some("a (×3)".into()));
    }

    #[test]
    fn collapse_prefix_simple() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("  Compiling serde v1.0".into()), None);
        assert_eq!(c.feed("  Compiling clap v4.0".into()), None);
        assert_eq!(
            c.feed("Finished dev".into()),
            Some("  Compiling ... (×2)".into())
        );
        assert_eq!(c.flush(), Some("Finished dev".into()));
    }

    #[test]
    fn collapse_prefix_non_matching() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("  Compiling serde v1.0".into()), None);
        assert_eq!(
            c.feed("  Downloading clap v4.0".into()),
            Some("  Compiling serde v1.0".into())
        );
        assert_eq!(c.flush(), Some("  Downloading clap v4.0".into()));
    }

    #[test]
    fn collapse_prefix_single_char_prefix() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("- test 1".into()), None);
        assert_eq!(c.feed("- test 2".into()), Some("- test 1".into()));
        assert_eq!(c.flush(), Some("- test 2".into()));
    }

    #[test]
    fn collapse_prefix_mixed_exact_and_prefix() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("  Compiling serde v1.0".into()), None);
        assert_eq!(c.feed("  Compiling serde v1.0".into()), None);
        assert_eq!(c.feed("  Compiling clap v4.0".into()), None);
        assert_eq!(c.feed("other".into()), Some("  Compiling ... (×3)".into()));
        assert_eq!(c.flush(), Some("other".into()));
    }

    // ── Whitespace squeeze tests ────────────────────────────────────────

    #[test]
    fn squeeze_allows_single_blank() {
        let mut s = WhitespaceSqueeze::new();
        assert!(s.feed("text").is_some());
        assert!(s.feed("").is_some()); // first blank: keep
        assert!(s.feed("more").is_some());
    }

    #[test]
    fn squeeze_removes_extra_blanks() {
        let mut s = WhitespaceSqueeze::new();
        assert!(s.feed("text").is_some());
        assert!(s.feed("").is_some()); // 1st blank: keep
        assert!(s.feed("").is_none()); // 2nd blank: suppress
        assert!(s.feed("  ").is_none()); // 3rd blank (whitespace only): suppress
        assert!(s.feed("more").is_some()); // text resets counter
    }

    // ── Head/Tail buffer tests ──────────────────────────────────────────

    #[test]
    fn buffer_under_threshold_no_truncation() {
        let mut buf = HeadTailBuffer::new(5, 5);
        for i in 0..8 {
            buf.push(format!("line {}", i));
        }
        assert!(!buf.was_truncated(100, 5));
        let (output, lines, _) = buf.render(100, 5);
        assert_eq!(lines, 8);
        assert!(!output.contains("omitted"));
    }

    #[test]
    fn buffer_exactly_head_plus_tail() {
        let mut buf = HeadTailBuffer::new(3, 3);
        for i in 0..6 {
            buf.push(format!("line {}", i));
        }
        assert!(!buf.was_truncated(6, 3));
        let (output, _, _) = buf.render(6, 3);
        assert!(!output.contains("omitted"));
    }

    #[test]
    fn buffer_truncation_works() {
        let mut buf = HeadTailBuffer::new(3, 3);
        for i in 0..100 {
            buf.push(format!("line {}", i));
        }
        assert!(buf.was_truncated(6, 3));
        assert_eq!(buf.total_lines(), 100);

        let (output, _, _) = buf.render(6, 3);
        // Should contain head lines
        assert!(output.contains("line 0"));
        assert!(output.contains("line 1"));
        assert!(output.contains("line 2"));
        // Should contain tail lines
        assert!(output.contains("line 97"));
        assert!(output.contains("line 98"));
        assert!(output.contains("line 99"));
        // Should have banner
        assert!(output.contains("94 lines omitted for LLM"));
        // Should NOT contain middle lines
        assert!(!output.contains("line 50"));
    }

    #[test]
    fn buffer_one_line() {
        let mut buf = HeadTailBuffer::new(30, 30);
        buf.push("solo".into());
        let (output, lines, _) = buf.render(100, 30);
        assert_eq!(output, "solo");
        assert_eq!(lines, 1);
    }

    #[test]
    fn strip_ansi_osc_sequences() {
        let input = b"Hello\x1b]0;Title\x07World";
        let out = strip_ansi(input);
        assert_eq!(out, "HelloWorld");
    }

    #[test]
    fn buffer_empty() {
        let buf = HeadTailBuffer::new(30, 30);
        let (output, lines, _) = buf.render(100, 30);
        assert_eq!(output, "");
        assert_eq!(lines, 0);
    }

    // ── Full pipeline tests ─────────────────────────────────────────────

    #[test]
    fn pipeline_small_output_passthrough() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        pipe.feed("hello".into());
        pipe.feed("world".into());
        let result = pipe.finish(100, 0, 30);
        assert_eq!(result.output, "hello\nworld");
        assert_eq!(result.lines_raw, 2);
        assert!(!result.truncated);
    }

    #[test]
    fn pipeline_truncates_large_output() {
        let mut pipe = FilterPipeline::new(5, 5, false);
        for i in 0..200 {
            pipe.feed(format!("u{}", i).into());
        }
        let result = pipe.finish(10, 0, 5);
        assert!(result.truncated);
        assert!(result.output.contains("u0"));
        assert!(result.output.contains("u199"));
        assert!(result.output.contains("omitted"));
    }

    #[test]
    fn pipeline_collapses_and_squeezes() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        pipe.feed("warning: unused var".into());
        pipe.feed("warning: unused var".into());
        pipe.feed("warning: unused var".into());
        pipe.feed("".into());
        pipe.feed("".into());
        pipe.feed("".into());
        pipe.feed("done".into());
        let result = pipe.finish(100, 0, 30);
        assert!(result.output.contains("warning: unused var (×3)"));
        // Should only have 1 blank line, not 3
        assert!(!result.output.contains("\n\n\n"));
    }

    // ── Binary detection tests ──────────────────────────────────────────

    #[test]
    fn binary_detects_null_bytes() {
        assert!(looks_binary(b"hello\x00world"));
    }

    #[test]
    fn binary_clean_utf8() {
        assert!(!looks_binary(b"hello world"));
    }

    #[test]
    fn binary_invalid_utf8() {
        assert!(looks_binary(&[0xFF, 0xFE, 0x80, 0x81]));
    }

    // ── Additional ANSI strip tests ─────────────────────────────────────

    #[test]
    fn strip_ansi_complex_sequences() {
        // Bold + underline + reset
        let input = b"\x1b[1m\x1b[4mBOLD\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "BOLD");
    }

    #[test]
    fn strip_ansi_empty_input() {
        assert_eq!(strip_ansi(b""), "");
    }

    #[test]
    fn strip_ansi_only_escape_codes() {
        let input = b"\x1b[31m\x1b[0m";
        assert_eq!(strip_ansi(input), "");
    }

    #[test]
    fn strip_ansi_multiline() {
        let input = b"\x1b[32mOK\x1b[0m\nplain\n\x1b[31mERR\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "OK\nplain\nERR");
    }

    // ── Additional collapse-identical tests ─────────────────────────────

    #[test]
    fn collapse_single_line() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("only".into()), None);
        let flushed = c.flush();
        assert_eq!(flushed, Some("only".into()));
        // No "×" annotation for a single occurrence
        assert!(!flushed.unwrap_or_default().contains('×'));
    }

    #[test]
    fn collapse_flush_on_empty() {
        let mut c = CollapseLines::new();
        assert_eq!(c.flush(), None);
    }

    #[test]
    fn collapse_two_different_then_same() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("a".into()), None);
        assert_eq!(c.feed("b".into()), Some("a".into()));
        assert_eq!(c.feed("b".into()), None);
        assert_eq!(c.feed("b".into()), None);
        let flushed = c.flush();
        // same_prefix is false for single-char strings, so it emits exact repeats
        assert_eq!(flushed, Some("b (×3)".into()));
    }

    #[test]
    fn collapse_fuzzy_match() {
        let mut collapse = CollapseLines::new();
        assert_eq!(
            collapse.feed(Cow::Borrowed("[info] 1234a processing item")),
            None
        );
        assert_eq!(
            collapse.feed(Cow::Borrowed("[info] 5678b processing item")),
            None
        );
        assert_eq!(
            collapse.feed(Cow::Borrowed("[info] 9101c processing item")),
            None
        );

        let out = collapse.flush().unwrap();
        assert!(out.contains("... (×3)"));
    }

    #[test]
    fn collapse_alternating() {
        let mut c = CollapseLines::new();
        assert_eq!(c.feed("a".into()), None);
        assert_eq!(c.feed("b".into()), Some("a".into()));
        assert_eq!(c.feed("a".into()), Some("b".into()));
        assert_eq!(c.feed("b".into()), Some("a".into()));
        let flushed = c.flush();
        assert_eq!(flushed, Some("b".into()));
        // None of the emitted lines should have a repeat marker
    }

    // ── Additional whitespace squeeze tests ─────────────────────────────

    #[test]
    fn squeeze_starts_with_blanks() {
        let mut s = WhitespaceSqueeze::new();
        assert!(s.feed("").is_some()); // 1st blank: keep
        assert!(s.feed("").is_none()); // 2nd blank: suppress
        assert!(s.feed("").is_none()); // 3rd blank: suppress
    }

    #[test]
    fn squeeze_interleaved_blanks() {
        let mut s = WhitespaceSqueeze::new();
        assert!(s.feed("text").is_some());
        assert!(s.feed("").is_some()); // 1st blank after text: keep
        assert!(s.feed("more").is_some());
        assert!(s.feed("").is_some()); // 1st blank after more: keep
        assert!(s.feed("").is_none()); // 2nd consecutive blank: suppress
        assert!(s.feed("end").is_some());
    }

    #[test]
    fn squeeze_all_blanks() {
        let mut s = WhitespaceSqueeze::new();
        let mut kept = 0;
        for _ in 0..5 {
            if s.feed("").is_some() {
                kept += 1;
            }
        }
        assert_eq!(kept, 1); // only the first blank passes
    }

    #[test]
    fn squeeze_whitespace_only_lines() {
        let mut s = WhitespaceSqueeze::new();
        assert!(s.feed("text").is_some());
        assert!(s.feed("\t").is_some()); // 1st whitespace-only: keep (counts as blank)
        assert!(s.feed("   ").is_none()); // 2nd whitespace-only: suppress
        assert!(s.feed("\t  ").is_none()); // 3rd: suppress
        assert!(s.feed("back").is_some()); // text resets
    }

    // ── Additional head/tail buffer tests ───────────────────────────────

    #[test]
    fn buffer_head_zero() {
        let mut buf = HeadTailBuffer::new(0, 5);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 10);
        let (output, lines, _) = buf.render(0, 5); // threshold=0 forces banner path
                                                   // Head is empty, so output should only have tail lines
        assert!(!output.contains("line 0"));
        assert!(output.contains("line 9"));
        assert!(output.contains("line 5"));
        assert_eq!(lines, 5 + 3); // 5 tail + 3 banner lines (empty, banner text, empty)
    }

    #[test]
    fn buffer_tail_zero() {
        let mut buf = HeadTailBuffer::new(5, 0);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 10);
        // With tail_cap=0, lines beyond head are dropped entirely
        let (output, _, _) = buf.render(0, 0); // threshold=0 forces banner path
        assert!(output.contains("line 0"));
        assert!(output.contains("line 4"));
        assert!(!output.contains("line 5")); // dropped
        assert!(!output.contains("line 9")); // dropped
                                             // Banner shows omitted count
        assert!(output.contains("5 lines omitted for LLM"));
    }

    #[test]
    fn buffer_head_and_tail_zero() {
        let mut buf = HeadTailBuffer::new(0, 0);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 10);
        let (output, _, _) = buf.render(0, 0);
        // head=0, tail=0 → no lines stored, only banner
        assert!(output.contains("10 lines omitted for LLM"));
        // No actual content lines
        assert!(!output.contains("line 0"));
        assert!(!output.contains("line 9"));
    }

    #[test]
    fn buffer_total_lines_tracking() {
        let mut buf = HeadTailBuffer::new(30, 30);
        buf.push("hello".into());
        buf.push("world".into());
        assert_eq!(buf.total_lines(), 2);
    }

    #[test]
    fn buffer_total_lines_accurate() {
        let mut buf = HeadTailBuffer::new(5, 5);
        for _ in 0..7 {
            buf.push("x".into());
        }
        assert_eq!(buf.total_lines(), 7);
        for _ in 0..3 {
            buf.push("y".into());
        }
        assert_eq!(buf.total_lines(), 10);
    }

    #[test]
    fn buffer_render_threshold_exact_boundary() {
        // total_lines == threshold → no banner
        let mut buf = HeadTailBuffer::new(5, 5);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 10);
        let (output, _, _) = buf.render(10, 5); // threshold == total_lines
        assert!(!output.contains("omitted"));
    }

    #[test]
    fn buffer_render_threshold_plus_one() {
        // total_lines == threshold + 1 → banner appears
        let mut buf = HeadTailBuffer::new(5, 5);
        for i in 0..11 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 11);
        let (output, _, _) = buf.render(10, 5); // threshold < total_lines
        assert!(output.contains("omitted"));
    }

    #[test]
    fn buffer_display_tail_trims_to_window() {
        // Retain a large tail (120) but display only a small window: simulates a
        // successful exit reusing a buffer sized for the error tail.
        let mut buf = HeadTailBuffer::new(5, 120);
        for i in 0..200 {
            buf.push(format!("line {}", i));
        }
        // display_tail = 5 → only the last 5 retained lines are shown.
        let (output, _, _) = buf.render(10, 5);
        assert!(output.contains("line 199"));
        assert!(output.contains("line 195"));
        assert!(!output.contains("line 194"));
        // display_tail = 50 → a wider window, still within the 120 retained.
        let mut buf2 = HeadTailBuffer::new(5, 120);
        for i in 0..200 {
            buf2.push(format!("line {}", i));
        }
        let (output2, _, _) = buf2.render(10, 50);
        assert!(output2.contains("line 150"));
        assert!(output2.contains("line 199"));
        assert!(!output2.contains("line 149"));
    }

    #[test]
    fn buffer_rebalance_head_preserves_order() {
        let mut buf = HeadTailBuffer::new(10, 100);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }
        buf.rebalance_head(3); // keep 3 head lines, move the other 7 to the tail front
        let (output, lines, _) = buf.render(1000, 100); // below threshold → show all
        let expected: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        assert_eq!(output, expected.join("\n"));
        assert_eq!(lines, 10);
    }

    #[test]
    fn pipeline_panic_shrinks_head_retroactively() {
        // Use tokens that neither prefix- nor fuzzy-collapse (distinct first words,
        // <10 alphabetic chars) so each line reaches the buffer individually.
        let mut pipe = FilterPipeline::new(30, 30, false);
        for i in 0..40 {
            pipe.feed(format!("E{}", i).into());
        }
        pipe.feed("thread 'main' panicked at src/x.rs:1:1".into());
        for i in 0..5 {
            pipe.feed(format!("TR{}", i).into());
        }
        let result = pipe.finish(10, 0, 30); // threshold 10 forces truncation
                                             // The crash context (panic + trailing trace) must survive...
        assert!(result.output.contains("panicked at"));
        assert!(result.output.contains("TR4"));
        // ...while the head is shrunk, so an early-middle line is dropped.
        assert!(!result.output.contains("E10"));
    }

    #[test]
    fn buffer_large_scale() {
        let mut buf = HeadTailBuffer::new(5, 5);
        for i in 0..10_000 {
            buf.push(format!("line {}", i));
        }
        assert_eq!(buf.total_lines(), 10_000);
        assert!(buf.was_truncated(10, 5));

        let (output, _, _) = buf.render(10, 5);
        // Head: lines 0-4
        for i in 0..5 {
            assert!(output.contains(&format!("line {}", i)));
        }
        // Tail: lines 9995-9999
        for i in 9995..10_000 {
            assert!(output.contains(&format!("line {}", i)));
        }
        // Middle should not be present
        assert!(!output.contains("line 5000"));
        assert!(output.contains("9990 lines omitted for LLM"));
    }

    // ── Additional full pipeline tests ──────────────────────────────────

    #[test]
    fn pipeline_empty_input() {
        let pipe = FilterPipeline::new(30, 30, false);
        let result = pipe.finish(100, 0, 30);
        assert_eq!(result.output, "");
        assert_eq!(result.lines_raw, 0);
        assert_eq!(result.lines_final, 0);
        assert_eq!(result.bytes_raw, 0);
        assert_eq!(result.bytes_final, 0);
        assert!(!result.truncated);
    }

    #[test]
    fn pipeline_single_line() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        pipe.feed("hello world".into());
        let result = pipe.finish(100, 0, 30);
        assert_eq!(result.output, "hello world");
        assert_eq!(result.lines_raw, 1);
        assert_eq!(result.lines_final, 1);
        assert!(!result.truncated);
    }

    #[test]
    fn pipeline_all_identical() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        for _ in 0..100 {
            pipe.feed("same line".into());
        }
        let result = pipe.finish(100, 0, 30);
        // 100 identical lines should collapse to "same line (×100)"
        assert!(result.output.contains("same line (×100)"));
        assert_eq!(result.lines_final, 1);
    }

    #[test]
    fn pipeline_all_blanks() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        let lines = vec![""; 50];
        for line in lines {
            pipe.feed(line.into());
        }
        let result = pipe.finish(100, 0, 30);
        // 50 identical blanks → collapsed to 1 blank (×50), then squeeze passes it (it's only 1)
        // The collapse makes it "×50" so it becomes non-blank text
        assert_eq!(result.lines_final, 1);
    }

    #[test]
    fn pipeline_mixed_collapse_and_truncation() {
        let mut pipe = FilterPipeline::new(5, 5, false);
        // 50 repeats of "repeat" + 50 unique lines = enough to trigger truncation
        for _ in 0..50 {
            pipe.feed("repeat".into());
        }
        for i in 0..50 {
            pipe.feed(format!("u{}", i).into());
        }
        let result = pipe.finish(10, 0, 5);
        // The 50 repeats should collapse to 1 line "repeat (×50)"
        // Then 50 unique lines → total 51 lines after collapse
        // With head=5, tail=5, threshold=10 → truncation should occur
        assert!(result.truncated);
        assert!(result.output.contains("repeat (×50)"));
    }

    #[test]
    fn pipeline_bytes_raw_vs_final() {
        let mut pipe = FilterPipeline::new(5, 5, false);
        let mut raw_bytes: usize = 0;
        for i in 0..200 {
            let line = format!("u{:05}", i);
            raw_bytes += line.len() + 1;
            pipe.feed(line.into());
        }
        let result = pipe.finish(10, raw_bytes, 5);
        assert!(result.truncated);
        assert!(result.bytes_raw > result.bytes_final);
    }

    #[test]
    fn pipeline_lines_raw_vs_final() {
        let mut pipe = FilterPipeline::new(5, 5, false);
        for i in 0..200 {
            pipe.feed(format!("u{}", i).into());
        }
        let result = pipe.finish(10, 0, 5);
        assert!(result.truncated);
        assert!(result.lines_raw > result.lines_final);
    }

    #[test]
    fn pipeline_finish_flushes_pending_collapse() {
        let mut pipe = FilterPipeline::new(30, 30, false);
        pipe.feed("first".into());
        pipe.feed("last".into());
        pipe.feed("last".into());
        pipe.feed("last".into());
        let result = pipe.finish(100, 0, 30);
        // flush should emit the pending "last (×3)"
        assert!(result.output.contains("first"));
        assert!(result.output.contains("last (×3)"));
    }

    // ── Additional binary detection tests ───────────────────────────────

    #[test]
    fn contains_ascii_ci_works() {
        assert!(contains_ascii_ci("Build FAILED with ERROR", "error"));
        assert!(contains_ascii_ci("thread panicked AT", "panicked at"));
        assert!(contains_ascii_ci("anything", ""));
        assert!(!contains_ascii_ci("all good here", "panic"));
        assert!(!contains_ascii_ci("hi", "longer than haystack"));
    }

    #[test]
    fn binary_empty_input() {
        assert!(!looks_binary(b""));
    }

    #[test]
    fn binary_large_clean_utf8() {
        let data = vec![b'A'; 16 * 1024]; // 16KB of ASCII
        assert!(!looks_binary(&data));
    }

    #[test]
    fn binary_null_at_boundary() {
        // Null byte at exactly position 8191 (within the 8KB window)
        let mut data = vec![b'A'; 8192];
        data[8191] = 0;
        assert!(looks_binary(&data));
    }

    #[test]
    fn binary_null_after_boundary() {
        // Null byte at position 8193 (beyond the 8KB check window)
        let mut data = vec![b'A'; 16 * 1024];
        data[8193] = 0;
        assert!(!looks_binary(&data)); // only first 8KB checked
    }

    #[test]
    fn binary_mixed_utf8_with_high_bytes() {
        // Valid multi-byte UTF-8 (Chinese characters)
        let input = "你好世界 hello 日本語";
        assert!(!looks_binary(input.as_bytes()));
    }

    // ── DiffCollapse tests ──────────────────────────────────────────────

    fn run_diff(lines: &[&str]) -> Vec<String> {
        let mut d = DiffCollapse::new();
        let mut out = Vec::new();
        for l in lines {
            out.extend(d.feed((*l).to_string()));
        }
        out.extend(d.flush());
        out
    }

    #[test]
    fn diff_collapses_long_unchanged_context() {
        let mut lines = vec!["@@ -1,30 +1,30 @@".to_string()];
        for i in 0..20 {
            lines.push(format!(" ctx_{i}")); // distinct context lines
        }
        lines.push("-old".to_string());
        lines.push("+new".to_string());
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let out = run_diff(&refs);
        let joined = out.join("\n");

        assert!(joined.contains("unchanged diff lines"), "{joined}");
        assert!(joined.contains("-old") && joined.contains("+new"));
        assert!(out.len() < lines.len(), "context should shrink");
        assert!(joined.contains("ctx_0") && joined.contains("ctx_19")); // edges kept
        assert!(!joined.contains("ctx_10")); // deep middle dropped
    }

    #[test]
    fn diff_keeps_short_context_verbatim() {
        let out = run_diff(&["@@ -1,3 +1,3 @@", " a", " b", "-x", "+y", " c"]);
        assert_eq!(out, vec!["@@ -1,3 +1,3 @@", " a", " b", "-x", "+y", " c"]);
    }

    #[test]
    fn non_diff_indented_text_is_not_collapsed() {
        // No hunk header → DiffCollapse stays inactive even for space-led lines.
        let mut lines = vec!["Build:".to_string()];
        for i in 0..20 {
            lines.push(format!("   step {i}"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let out = run_diff(&refs);
        assert_eq!(out.len(), lines.len());
        assert!(!out.join("\n").contains("unchanged diff"));
    }

    #[test]
    fn diff_section_ends_on_non_diff_line() {
        // After a diff, normal prose with leading spaces must not be collapsed.
        let mut lines = vec![
            "@@ -1,1 +1,1 @@".to_string(),
            "-a".to_string(),
            "+b".to_string(),
            "Summary follows:".to_string(), // non-diff line ends the section
        ];
        for i in 0..20 {
            lines.push(format!("   note {i}"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let out = run_diff(&refs);
        assert!(!out.join("\n").contains("unchanged diff"));
        assert_eq!(out.len(), lines.len());
    }

    #[test]
    fn diff_hunk_header_requires_space() {
        // `@@@@` (no space) must NOT activate diff mode; following context stays.
        let out = run_diff(&["@@@@", " x", " y"]);
        assert_eq!(out, vec!["@@@@", " x", " y"]);
        // A real header does activate.
        assert!(is_hunk_header("@@ -1,2 +1,2 @@"));
        assert!(!is_hunk_header("@@@@"));
        assert!(!is_hunk_header("@@ no second marker"));
    }

    #[test]
    fn diff_collapse_through_full_pipeline() {
        // Exercise the production wiring: DiffCollapse → CollapseLines → squeeze →
        // buffer. Distinct context (won't prefix-collapse) so the marker survives.
        let mut pipe = FilterPipeline::new(30, 30, false);
        pipe.feed(Cow::Borrowed("@@ -1,40 +1,40 @@"));
        for i in 0..30 {
            pipe.feed(Cow::Owned(format!(" ctx_{i} = value_{}", i * 7)));
        }
        pipe.feed(Cow::Borrowed("-let old = 1;"));
        pipe.feed(Cow::Borrowed("+let new = 2;"));
        let r = pipe.finish(100, 0, 30);
        assert!(
            r.output.contains("unchanged diff lines"),
            "marker should survive the pipeline: {}",
            r.output
        );
        assert!(r.output.contains("-let old = 1;") && r.output.contains("+let new = 2;"));
        // 30 context lines compressed → far fewer than 30 lines reach the buffer.
        assert!(
            r.lines_raw < 30,
            "context collapsed: lines_raw={}",
            r.lines_raw
        );
    }

    // ── Clean-success squelch ───────────────────────────────────────────

    #[test]
    fn has_error_signal_matches_keywords_case_insensitively() {
        assert!(has_error_signal("ERROR: boom"));
        assert!(has_error_signal("a warning here"));
        assert!(has_error_signal("Build FAILED"));
        assert!(has_error_signal("thread 'main' panicked at ..."));
        assert!(has_error_signal("Traceback (most recent call last)"));
        // Clean progress/summary lines carry no signal.
        assert!(!has_error_signal("Compiling foo v0.1.0"));
        assert!(!has_error_signal("Finished in 3.2s"));
        assert!(!has_error_signal("   125"));
        assert!(!has_error_signal(""));
    }

    #[test]
    fn squelched_tail_halves_floors_and_never_expands() {
        assert_eq!(squelched_tail(30), 15); // halved
        assert_eq!(squelched_tail(8), 5); // floored at MIN, not 4
        assert_eq!(squelched_tail(5), 5); // already at floor
        assert_eq!(squelched_tail(3), 3); // tiny --tail honored, never expanded
        assert_eq!(squelched_tail(0), 0); // degenerate: no tail stays no tail
        assert!(squelched_tail(1000) <= 1000);
    }

    #[test]
    fn pipeline_tracks_error_signal_across_whole_stream() {
        // A signal seen mid-stream sticks even if that line is later truncated away.
        let mut p = FilterPipeline::new(2, 2, false);
        for i in 0..50 {
            if i == 25 {
                p.feed(Cow::Borrowed("error: transient hiccup"));
            } else {
                p.feed(Cow::Owned(format!("clean line {i}")));
            }
        }
        assert!(
            p.saw_error_signal(),
            "a mid-stream error keyword must trip the sticky flag"
        );
    }

    #[test]
    fn pipeline_no_error_signal_on_clean_stream() {
        let mut p = FilterPipeline::new(2, 2, false);
        for i in 0..50 {
            p.feed(Cow::Owned(format!("Compiling crate-{i}")));
        }
        assert!(
            !p.saw_error_signal(),
            "a fully clean stream must leave the squelch gate open"
        );
    }
}
