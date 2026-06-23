//! Subprocess runner: spawn, stream, capture.
//!
//! Merges stderr into stdout (`2>&1`) via a single pipe, read synchronously
//! from the main thread. Zero threads, zero async, zero deadlock.
//!
//! Hardening:
//! - UTF-8 lossy reads (never drops lines on invalid encoding)
//! - Line length cap (1MB) to prevent OOM on binary/minified input
//! - Raw mode memory cap (256MB) to prevent OOM on huge output
//! - Exit code 128+N for signal-killed children (POSIX convention)

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::filter::{self, FilterPipeline, FilterResult};
use crate::recovery::Recovery;

/// Process-group id of the currently running captured child (0 when none).
///
/// The captured child is spawned into its OWN process group, and the parent's
/// SIGINT/SIGTERM handlers forward to this group (see `main::forward_signal`) so
/// the whole child subtree — not just the `sh` wrapper — receives the signal.
/// The idle-timeout watchdog kills the same group.
pub static CHILD_PGID: AtomicI32 = AtomicI32::new(0);

/// Result of running a command through the proxy.
pub struct RunResult {
    pub filter_result: FilterResult,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub strategy: &'static str,
    /// Path to the saved full-output recovery file, when one was kept (only on a
    /// failing + truncated run with `--recover`). `None` otherwise.
    pub recovery_path: Option<PathBuf>,
    /// The actual number of tail lines shown — after the success-vs-error choice
    /// AND the clean-success squelch — so the banner reports the truth, not the
    /// configured cap. `0` in raw mode (which has no head/tail banner).
    pub display_tail: usize,
}

/// Maximum length of a single line before we force-truncate (1MB).
/// Prevents OOM on binary files that slip past detection (no newlines).
const MAX_LINE_BYTES: usize = 1_048_576;

/// Maximum total bytes collected in raw mode (256MB).
/// Prevents OOM when `--raw` is used on commands with massive output.
const RAW_MODE_MAX_BYTES: usize = 256 * 1024 * 1024;

/// Upper bound on how many extra tail lines we retain to honor a large
/// `--threshold` (the "no truncation below threshold" promise). Bounds memory
/// so a pathological `--threshold` cannot make the filtered buffer unbounded.
const RETAIN_COMPLETENESS_CAP: usize = 100_000;

/// Spawn a command via shell with explicit `2>&1` merge.
fn spawn_merged(cmd: &[String]) -> std::io::Result<(Child, BufReader<std::process::ChildStdout>)> {
    // Build a shell command string with proper escaping
    let shell_cmd = cmd
        .iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ");

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("{} 2>&1", shell_cmd))
        .stdout(Stdio::piped())
        .stderr(Stdio::null()); // stderr is already merged into stdout

    // Put the child in its own process group so the parent can deliver SIGINT/
    // SIGTERM (and the watchdog SIGKILL) to the WHOLE subtree via killpg, rather
    // than only the `sh` wrapper. pgid becomes the child pid.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child_res = command.spawn();

    let mut child = match child_res {
        Ok(c) => c,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "l0-cache: 'sh' shell not found in PATH. Cannot merge stderr. Install a POSIX shell or use `l0-cache -i` for passthrough mode.",
            ));
        }
        Err(e) => return Err(e),
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let reader = BufReader::with_capacity(64 * 1024, stdout);
    Ok((child, reader))
}

/// Simple shell escaping: wrap in single quotes, escape existing single quotes.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // If safe, no escaping needed
    if s.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || b == b'_'
            || b == b'-'
            || b == b'.'
            || b == b'/'
            || b == b':'
            || b == b'='
    }) {
        return s.to_string();
    }
    // Wrap in single quotes, escape internal single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Read one line's bytes into `buf`, but NEVER buffer more than `MAX_LINE_BYTES`.
///
/// Unlike `BufRead::read_until`, which would allocate the entire line before any
/// cap could apply, this reads in chunks via `fill_buf`/`consume`: once the kept
/// content reaches the cap, the rest of the line is read and DISCARDED (still
/// counted), so a pathological newline-free stream — a minified bundle, a
/// `tr '\0' a` flood — cannot blow up memory. The trailing `\n` is consumed but
/// not stored.
///
/// Returns `Some((raw_len, capped))`: `raw_len` is the true number of bytes the OS
/// produced for this line including the terminator (feeds an accurate `bytes_raw`);
/// `capped` is true if the line exceeded the cap. Returns `None` only at EOF.
fn read_line_capped(reader: &mut BufReader<impl Read>, buf: &mut Vec<u8>) -> Option<(usize, bool)> {
    buf.clear();
    let mut raw_len = 0usize;
    let mut capped = false;
    let mut saw_any = false;
    loop {
        // Inspect the buffered chunk and copy into `buf` here; end the borrow
        // before calling `consume` (which needs &mut reader).
        let (consumed, found_nl) = {
            let chunk = match reader.fill_buf() {
                Ok(c) => c,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    return if saw_any {
                        Some((raw_len, capped))
                    } else {
                        None
                    }
                }
            };
            if chunk.is_empty() {
                return if saw_any {
                    Some((raw_len, capped))
                } else {
                    None
                };
            }
            saw_any = true;

            let nl = chunk.iter().position(|&b| b == b'\n');
            let take = nl.unwrap_or(chunk.len()); // bytes of this line in the chunk
            let room = MAX_LINE_BYTES.saturating_sub(buf.len());
            let keep = room.min(take);
            buf.extend_from_slice(&chunk[..keep]);
            if keep < take {
                capped = true; // dropped the rest of this line (kept counting)
            }
            match nl {
                Some(i) => (i + 1, true), // include the newline in the consume count
                None => (take, false),
            }
        };
        raw_len += consumed;
        reader.consume(consumed);
        if found_nl {
            return Some((raw_len, capped));
        }
    }
}

/// Read one line from the reader into `buf` (newline stripped), memory-bounded.
///
/// Returns `Some(raw_len)` where `raw_len` is the number of bytes the OS actually
/// produced for this line (including the line terminator), measured BEFORE any
/// transformation — this is what feeds the accurate `bytes_raw` metric. Returns
/// `None` at EOF.
///
/// When `transform` is true (filtered mode) the line is cleaned for LLM
/// consumption: interior carriage-return progress-bar squashing, backspace/bell
/// resolution, and aggressive truncation of giant single-line JSON payloads.
/// When `transform` is false (`--raw`) the content is left verbatim — only line
/// framing and the 1 MB OOM safety cap are applied, so `--raw` is truly raw.
fn read_line_bytes(
    reader: &mut BufReader<impl Read>,
    buf: &mut Vec<u8>,
    transform: bool,
) -> Option<usize> {
    let (raw_len, capped) = read_line_capped(reader, buf)?;

    // Line framing: the newline is already excluded; strip a trailing \r (\r\n).
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }

    let mut json_truncated = false;
    if transform {
        // --- 80/20 Progress Bar Squashing ---
        // If the buffer still contains '\r' (interior carriage returns), it means
        // the command updated the same line multiple times (e.g. progress bars).
        // Emulate terminal behavior by keeping only the part *after* the last '\r'.
        if let Some(last_r) = buf.iter().rposition(|&b| b == b'\r') {
            let keep_len = buf.len() - (last_r + 1);
            buf.copy_within(last_r + 1.., 0);
            buf.truncate(keep_len);
        }

        // --- 80/20 Backspace and Bell Resolution ---
        if buf.contains(&0x08) || buf.contains(&0x7f) || buf.contains(&0x07) {
            let mut write_idx = 0;
            for read_idx in 0..buf.len() {
                let b = buf[read_idx];
                if b == 0x08 || b == 0x7f {
                    // Backspace or DEL: remove previous byte if possible
                    if write_idx > 0 {
                        write_idx -= 1;
                        // Basic UTF-8 continuation byte skip:
                        while write_idx > 0 && (buf[write_idx] & 0xC0) == 0x80 {
                            write_idx -= 1;
                        }
                    }
                } else if b == 0x07 {
                    // Terminal Bell: ignore completely
                    continue;
                } else {
                    buf[write_idx] = b;
                    write_idx += 1;
                }
            }
            buf.truncate(write_idx);
        }

        // --- 80/20 JSON Smart Truncation (Token Shield) ---
        // If the line is large (> 2000 bytes) and looks like JSON, aggressively
        // truncate it to spare the LLM a massive single-line JSON payload.
        // This is destructive, so it is filtered-mode only (never in --raw).
        if buf.len() > 2000 {
            let first_non_whitespace = buf.iter().find(|&&b| b != b' ' && b != b'\t');
            let is_json = matches!(first_non_whitespace, Some(&b'{') | Some(&b'['));
            if is_json {
                buf.truncate(2000);
                buf.extend_from_slice(b"\n... [Large JSON Payload Truncated for LLM] ...");
                json_truncated = true;
            }
        }
    }

    // The line hit the 1 MB cap and its tail was drained — say so (unless JSON
    // truncation already replaced the content with its own marker).
    if capped && !json_truncated {
        buf.extend_from_slice(b"... [line truncated at 1MB]");
    }

    Some(raw_len)
}

/// Extract exit code from ExitStatus, using POSIX 128+N convention for signals.
fn exit_code_from_status(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.code().unwrap_or_else(|| {
            // Killed by signal → 128 + signal_number (POSIX convention)
            // e.g., SIGKILL=9 → 137, SIGSEGV=11 → 139, SIGTERM=15 → 143
            status.signal().map(|s| 128 + s).unwrap_or(1)
        })
    }
    #[cfg(not(unix))]
    {
        status.code().unwrap_or(1)
    }
}

/// Run a command in capture mode: merge streams, filter, return result.
#[allow(clippy::too_many_arguments)]
pub fn run_captured(
    cmd: &[String],
    head_cap: usize,
    tail_cap: usize,
    tail_error_cap: usize,
    threshold: usize,
    raw_mode: bool,
    only_errors: bool,
    squelch: bool,
    idle_timeout: u64,
    recover: bool,
) -> std::io::Result<RunResult> {
    let start = Instant::now();

    let (mut child, mut reader) = spawn_merged(cmd)?;

    // Full-output recovery (best-effort, filtered mode only). Inactive unless
    // `--recover` is set; lazily writes to a temp file only past the threshold.
    let cmd_label = cmd
        .first()
        .map(|s| s.rsplit('/').next().unwrap_or(s))
        .unwrap_or("cmd");
    let mut recovery = Recovery::new(recover && !raw_mode, cmd_label, threshold);

    // Publish the child's process-group id so the parent's signal handlers and
    // the watchdog can target the whole subtree. pgid == pid (own group).
    CHILD_PGID.store(child.id() as i32, Ordering::SeqCst);

    // Reusable line buffer for read_line_bytes
    let mut line_buf: Vec<u8> = Vec::with_capacity(4096);

    // Track raw bytes for binary detection on first chunk
    let mut first_chunk = Vec::new();
    let mut is_binary = false;
    let mut raw_bytes_total: usize = 0;

    let mut all_lines: Vec<String> = Vec::new();
    let mut raw_capped = false;
    // Retain enough tail lines while streaming to serve BOTH a successful exit
    // (show `tail_cap`) and a failing exit (show the larger `tail_error_cap`),
    // since the tail cannot be expanded retroactively. Also retain enough to show
    // every line below `threshold` (the documented "no truncation" promise),
    // bounded so a huge --threshold cannot exhaust memory.
    let retain_tail = tail_cap.max(tail_error_cap).max(
        threshold
            .saturating_sub(head_cap)
            .min(RETAIN_COMPLETENESS_CAP),
    );
    let mut pipeline = if !raw_mode {
        Some(FilterPipeline::new(head_cap, retain_tail, only_errors))
    } else {
        None
    };

    // --- 80/20 Interactive Prompt Deadlock Prevention ---
    let last_output_time = Arc::new(AtomicU64::new(start.elapsed().as_millis() as u64));
    let last_output_time_clone = Arc::clone(&last_output_time);
    let done_flag = Arc::new(AtomicBool::new(false));
    let done_flag_clone = Arc::clone(&done_flag);
    let child_id = child.id();

    let watchdog_handle = if idle_timeout > 0 {
        Some(std::thread::spawn(move || {
            let timeout_ms = idle_timeout * 1000;
            loop {
                if done_flag_clone.load(Ordering::Relaxed) {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                let elapsed = start.elapsed().as_millis() as u64;
                let last = last_output_time_clone.load(Ordering::Relaxed);
                if elapsed.saturating_sub(last) > timeout_ms {
                    // Kill the whole child process group, not just the `sh` wrapper,
                    // so pipelines/grandchildren can't keep the stdout pipe open and
                    // deadlock the read loop.
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(-(child_id as i32), libc::SIGKILL);
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/T", "/PID", &child_id.to_string()])
                            .status();
                    }
                    return true;
                }
            }
        }))
    } else {
        None
    };

    // In --raw mode we read verbatim (no destructive transforms); otherwise we
    // clean each line for LLM consumption. `raw_len` is the true pre-transform
    // byte count, which keeps `bytes_raw` honest.
    while let Some(raw_len) = read_line_bytes(&mut reader, &mut line_buf, !raw_mode) {
        if idle_timeout > 0 {
            last_output_time.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        raw_bytes_total += raw_len;

        // Binary detection on first ~8KB
        if first_chunk.len() < 8192 {
            first_chunk.extend_from_slice(&line_buf);
            first_chunk.push(b'\n');
            if filter::looks_binary(&first_chunk) {
                is_binary = true;
                break;
            }
        }

        if raw_mode {
            // OOM protection: cap total collected bytes
            if raw_bytes_total > RAW_MODE_MAX_BYTES {
                raw_capped = true;
                // Keep draining but don't store (so child doesn't block)
                continue;
            }
            let stripped = filter::strip_ansi(&line_buf);
            all_lines.push(stripped.into_owned());
        } else {
            let stripped = filter::strip_ansi(&line_buf);
            recovery.feed(&stripped);
            if let Some(ref mut pipe) = pipeline {
                pipe.feed(stripped);
            }
        }
    }

    // Drain remaining pipe output after binary-detection break.
    // Without this, the child blocks when the 64KB pipe buffer fills → deadlock.
    if is_binary {
        let mut drain_buf = [0u8; 8192];
        while let Ok(n) = reader.read(&mut drain_buf) {
            if n == 0 {
                break;
            }
            raw_bytes_total += n;
        }
    }

    done_flag.store(true, Ordering::Relaxed);
    let mut killed_by_watchdog = false;
    if let Some(handle) = watchdog_handle {
        killed_by_watchdog = handle.join().unwrap_or(false);
    }

    // Wait for child to finish
    let status: ExitStatus = child.wait()?;
    // Child reaped: stop forwarding signals to a defunct group.
    CHILD_PGID.store(0, Ordering::SeqCst);
    let exit_code = exit_code_from_status(status);
    let duration_ms = start.elapsed().as_millis() as u64;

    if is_binary {
        // Binary output: we deliberately do NOT forward the full stream to the LLM
        // (it would be both useless and token-expensive). We show the first ~8 KB
        // we sniffed and, when the stream was larger, an explicit banner — instead
        // of silently emitting a truncated blob that looks like the whole output.
        let shown_bytes = first_chunk.len();
        let mut output = String::from_utf8_lossy(&first_chunk).into_owned();
        let truncated = raw_bytes_total > shown_bytes;
        if truncated {
            output.push_str(&format!(
                "\n... [l0-cache: binary output detected — showing first {} of {} bytes] ...\n",
                shown_bytes, raw_bytes_total
            ));
        }
        let bytes_final = output.len();
        let _ = recovery.finalize(false); // drop any partial file for binary output
        return Ok(RunResult {
            filter_result: FilterResult {
                output,
                lines_raw: 0,
                lines_final: 0,
                bytes_raw: raw_bytes_total,
                bytes_final,
                truncated,
            },
            exit_code,
            duration_ms,
            strategy: "binary_skip",
            recovery_path: None,
            display_tail: 0,
        });
    }

    if raw_mode {
        if raw_capped {
            all_lines.push(String::new());
            all_lines.push(format!(
                "... [output truncated at {}MB by l0-cache --raw] ...",
                RAW_MODE_MAX_BYTES / (1024 * 1024)
            ));
        }

        let mut output = all_lines.join("\n");
        if killed_by_watchdog {
            let msg = format!("\n... [l0-cache: Command killed due to {}s output inactivity. Is it waiting for interactive input?] ...\n", idle_timeout);
            output.push_str(&msg);
        }
        let bytes_final = output.len();
        let lines_final = all_lines.len();

        Ok(RunResult {
            filter_result: FilterResult {
                output,
                lines_raw: lines_final,
                lines_final,
                bytes_raw: raw_bytes_total,
                bytes_final,
                truncated: raw_capped || killed_by_watchdog,
            },
            exit_code,
            duration_ms,
            strategy: "raw",
            recovery_path: None,
            display_tail: 0,
        })
    } else {
        let pipe = pipeline.unwrap();
        // Show the success tail on exit 0, the (larger) error tail otherwise.
        // The buffer retained `retain_tail` lines, so this trims, never expands.
        //
        // Clean-success squelch: on a zero exit with no error/warning signal
        // anywhere in the stream, trim the success tail further — a clean
        // build/test/install's middle is progress noise. Any signal (or a
        // non-zero exit) keeps the full tail so failures stay fully visible.
        let display_tail = if exit_code == 0 {
            if squelch && !pipe.saw_error_signal() {
                crate::filter::squelched_tail(tail_cap)
            } else {
                tail_cap
            }
        } else {
            tail_error_cap
        };

        let mut filter_result = pipe.finish(threshold, raw_bytes_total, display_tail);
        if killed_by_watchdog {
            let msg = format!("\n... [l0-cache: Command killed due to {}s output inactivity. Is it waiting for interactive input?] ...\n", idle_timeout);
            filter_result.output.push_str(&msg);
            // Count it as truncated so it triggers banner logic
            filter_result.truncated = true;
        }

        // Keep the full-output recovery file only when the agent is likely to need
        // the dropped middle: a failing command whose output was truncated.
        let recovery_path = recovery.finalize(exit_code != 0 && filter_result.truncated);

        Ok(RunResult {
            filter_result,
            exit_code,
            duration_ms,
            strategy: "head_tail",
            recovery_path,
            display_tail,
        })
    }
}

/// Run a command in passthrough mode: inherit all stdio, no capture.
pub fn run_passthrough(cmd: &[String]) -> std::io::Result<i32> {
    if cmd.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no command to execute",
        ));
    }
    let status = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    Ok(exit_code_from_status(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── shell_escape tests ─────────────────────────────────────────────

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("cargo"), "cargo");
    }

    #[test]
    fn shell_escape_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn shell_escape_with_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_escape_empty() {
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn shell_escape_path() {
        assert_eq!(shell_escape("/usr/local/bin/cargo"), "/usr/local/bin/cargo");
    }

    #[test]
    fn shell_escape_equals() {
        assert_eq!(shell_escape("--target=release"), "--target=release");
    }

    #[test]
    fn shell_escape_colon() {
        assert_eq!(shell_escape("host:port"), "host:port");
    }

    #[test]
    fn shell_escape_special_chars() {
        assert_eq!(shell_escape("hello;world"), "'hello;world'");
    }

    #[test]
    fn shell_escape_dollar() {
        assert_eq!(shell_escape("$HOME"), "'$HOME'");
    }

    #[test]
    fn shell_escape_backtick() {
        assert_eq!(shell_escape("cmd `whoami`"), "'cmd `whoami`'");
    }

    #[test]
    fn shell_escape_newline() {
        let input = "line1\nline2";
        let escaped = shell_escape(input);
        assert!(
            escaped.starts_with('\''),
            "should be single-quoted: {escaped}"
        );
        assert!(
            escaped.ends_with('\''),
            "should be single-quoted: {escaped}"
        );
        assert!(escaped.contains("line1"));
        assert!(escaped.contains("line2"));
    }

    #[test]
    fn shell_escape_tabs() {
        let input = "col1\tcol2";
        let escaped = shell_escape(input);
        assert!(
            escaped.starts_with('\''),
            "should be single-quoted: {escaped}"
        );
        assert!(
            escaped.ends_with('\''),
            "should be single-quoted: {escaped}"
        );
    }

    #[test]
    fn shell_escape_double_quotes() {
        let input = "say \"hi\"";
        let escaped = shell_escape(input);
        assert!(
            escaped.starts_with('\''),
            "should be single-quoted: {escaped}"
        );
        assert!(escaped.contains("say"));
        assert!(escaped.contains("hi"));
    }

    // ── read_line_bytes tests ──────────────────────────────────────────

    /// Test adapter: read one transformed (filtered-mode) line, returning its
    /// content slice (or None at EOF) so existing assertions stay readable.
    fn rl<'a>(reader: &mut BufReader<impl Read>, buf: &'a mut Vec<u8>) -> Option<&'a [u8]> {
        match read_line_bytes(reader, buf, true) {
            Some(_) => Some(buf.as_slice()),
            None => None,
        }
    }

    #[test]
    fn read_line_bytes_normal() {
        let data = b"hello\nworld\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(rl(&mut reader, &mut buf), Some(&b"hello"[..]));
        assert_eq!(rl(&mut reader, &mut buf), Some(&b"world"[..]));
        assert_eq!(rl(&mut reader, &mut buf), None);
    }

    #[test]
    fn read_line_bytes_crlf() {
        let data = b"windows\r\nline\r\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(rl(&mut reader, &mut buf), Some(&b"windows"[..]));
        assert_eq!(rl(&mut reader, &mut buf), Some(&b"line"[..]));
    }

    #[test]
    fn read_line_bytes_no_trailing_newline() {
        let data = b"no newline";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(rl(&mut reader, &mut buf), Some(&b"no newline"[..]));
        assert_eq!(rl(&mut reader, &mut buf), None);
    }

    #[test]
    fn read_line_bytes_empty() {
        let mut buf = Vec::new();
        let data = b"";
        let mut reader = BufReader::new(&data[..]);
        let res = rl(&mut reader, &mut buf);
        assert_eq!(res, None);
    }

    #[test]
    fn read_line_bytes_progress_bar_squash() {
        let mut buf = Vec::new();
        // Simulates: "Downloading... 10%\rDownloading... 50%\rDownloading... 100%\n"
        let data = b"Downloading... 10%\rDownloading... 50%\rDownloading... 100%\n";
        let mut reader = BufReader::new(&data[..]);
        let res = rl(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"Downloading... 100%");
    }

    #[test]
    fn read_line_bytes_backspace_and_bell() {
        let mut buf = Vec::new();
        // "foo\x07bar\x08baz\n" -> "foobabaz"
        let data = b"foo\x07bar\x08baz\n";
        let mut reader = BufReader::new(&data[..]);
        let res = rl(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"foobabaz");
    }

    #[test]
    fn read_line_bytes_backspace_utf8() {
        let mut buf = Vec::new();
        // "hello \xF0\x9F\x9A\x80\x08world\n" -> "hello world" (rocket emoji followed by backspace)
        let data = b"hello \xF0\x9F\x9A\x80\x08world\n";
        let mut reader = BufReader::new(&data[..]);
        let res = rl(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"hello world");
    }

    #[test]
    fn read_line_bytes_blank_lines() {
        let data = b"\n\n\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(rl(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(rl(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(rl(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(rl(&mut reader, &mut buf), None);
    }

    #[test]
    fn read_line_bytes_returns_true_raw_len() {
        // raw_len reflects the OS bytes incl. terminator, BEFORE any transform.
        let data = b"abc\ndef\r\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(read_line_bytes(&mut reader, &mut buf, true), Some(4)); // "abc\n"
        assert_eq!(&buf[..], b"abc");
        assert_eq!(read_line_bytes(&mut reader, &mut buf, true), Some(5)); // "def\r\n"
        assert_eq!(&buf[..], b"def");
        assert_eq!(read_line_bytes(&mut reader, &mut buf, true), None);
    }

    #[test]
    fn read_line_bytes_raw_mode_is_verbatim() {
        // A >2000B JSON line is truncated in filtered mode, kept in raw mode.
        let mut payload = vec![b'{'];
        payload.extend(std::iter::repeat_n(b'a', 3000));
        payload.push(b'\n');

        let mut bf = Vec::new();
        read_line_bytes(&mut BufReader::new(&payload[..]), &mut bf, true).unwrap();
        assert!(bf.len() < 3000, "filtered mode should truncate big JSON");

        let mut br = Vec::new();
        read_line_bytes(&mut BufReader::new(&payload[..]), &mut br, false).unwrap();
        assert_eq!(
            br.len(),
            3001,
            "raw mode must keep JSON verbatim (1 '{{' + 3000 'a')"
        );

        // Interior carriage returns: squashed in filtered, preserved in raw.
        let data = b"a\rb\n";
        let mut cf = Vec::new();
        read_line_bytes(&mut BufReader::new(&data[..]), &mut cf, true).unwrap();
        assert_eq!(&cf[..], b"b");
        let mut cr = Vec::new();
        read_line_bytes(&mut BufReader::new(&data[..]), &mut cr, false).unwrap();
        assert_eq!(&cr[..], b"a\rb");
    }

    #[test]
    fn read_line_bytes_caps_giant_newline_free_line() {
        // A 5 MB single "line" with no newline must NOT be buffered whole: `buf`
        // stays ~1 MB, raw_len reports the true size, and a marker is appended.
        let big = vec![b'a'; 5 * 1024 * 1024];
        let mut reader = BufReader::new(&big[..]);
        let mut buf = Vec::new();
        let raw = read_line_bytes(&mut reader, &mut buf, true).unwrap();
        assert_eq!(
            raw,
            5 * 1024 * 1024,
            "raw_len must reflect the true byte count"
        );
        assert!(
            buf.len() <= MAX_LINE_BYTES + 64,
            "buf must stay bounded (~1MB), got {}",
            buf.len()
        );
        assert!(buf.starts_with(b"aaaa"));
        assert!(String::from_utf8_lossy(&buf).ends_with("[line truncated at 1MB]"));
        assert_eq!(read_line_bytes(&mut reader, &mut buf, true), None);
    }

    #[test]
    fn read_line_bytes_cap_does_not_eat_the_next_line() {
        // Giant line, then a normal one: capping the first must not consume the second.
        let mut data = vec![b'x'; 2 * 1024 * 1024];
        data.push(b'\n');
        data.extend_from_slice(b"second\n");
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        let raw1 = read_line_bytes(&mut reader, &mut buf, true).unwrap();
        assert_eq!(raw1, 2 * 1024 * 1024 + 1);
        assert!(buf.len() <= MAX_LINE_BYTES + 64);
        let raw2 = read_line_bytes(&mut reader, &mut buf, true).unwrap();
        assert_eq!(raw2, 7); // "second\n"
        assert_eq!(&buf[..], b"second");
    }

    // ── exit_code_from_status tests ────────────────────────────────────

    #[test]
    fn exit_code_normal() {
        let status = Command::new("sh")
            .arg("-c")
            .arg("exit 42")
            .status()
            .unwrap();
        assert_eq!(exit_code_from_status(status), 42);
    }

    #[test]
    fn exit_code_zero() {
        let status = Command::new("true").status().unwrap();
        assert_eq!(exit_code_from_status(status), 0);
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_signal_killed() {
        // Kill a process with SIGKILL (9) → expect 128+9=137
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        unsafe {
            libc::kill(child.id() as i32, libc::SIGKILL);
        }
        let status = child.wait().unwrap();
        assert_eq!(exit_code_from_status(status), 137); // 128 + 9
    }

    // ── run_captured tests ─────────────────────────────────────────────

    #[test]
    fn run_echo_captured() {
        let result = run_captured(
            &["echo".into(), "hello world".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.filter_result.output.contains("hello world"));
        assert_eq!(result.strategy, "head_tail");
    }

    #[test]
    fn run_false_returns_nonzero() {
        let result = run_captured(
            &["false".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn run_passthrough_echo() {
        let code = run_passthrough(&["echo".into(), "pass".into()]).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn run_captured_multiline() {
        let result = run_captured(
            &["printf".into(), "line1\\nline2\\nline3".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        let lines: Vec<&str> = result.filter_result.output.lines().collect();
        assert!(
            lines.len() >= 3,
            "expected at least 3 lines, got {}: {:?}",
            lines.len(),
            lines
        );
    }

    #[test]
    fn run_captured_stderr_merged() {
        let result = run_captured(
            &["sh".into(), "-c".into(), "echo err >&2; echo out".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        let output = &result.filter_result.output;
        assert!(output.contains("err"), "stderr should be merged: {output}");
        assert!(output.contains("out"), "stdout should be present: {output}");
    }

    #[test]
    fn run_captured_exit_code_propagation() {
        let result = run_captured(
            &["sh".into(), "-c".into(), "exit 42".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn run_captured_raw_mode() {
        let result = run_captured(
            &["echo".into(), "raw test".into()],
            30,
            30,
            120,
            100,
            true,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_eq!(result.strategy, "raw");
        assert!(
            !result.filter_result.truncated,
            "raw mode should never truncate small output"
        );
        assert!(result.filter_result.output.contains("raw test"));
    }

    #[test]
    fn run_captured_large_output_truncation() {
        let result = run_captured(
            &["seq".into(), "1".into(), "500".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert!(result.filter_result.truncated);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.strategy, "head_tail");
    }

    #[test]
    fn run_captured_small_output_no_truncation() {
        let result = run_captured(
            &["echo".into(), "hi".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert!(!result.filter_result.truncated);
    }

    #[test]
    fn run_captured_empty_output() {
        let result = run_captured(
            &["true".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.filter_result.output.trim().is_empty());
    }

    #[test]
    fn run_captured_duration_is_positive() {
        let result = run_captured(
            &["echo".into(), "timing".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert!(result.duration_ms < 10_000, "echo should complete quickly");
    }

    #[test]
    fn run_passthrough_false() {
        let code = run_passthrough(&["false".into()]).unwrap();
        assert_ne!(code, 0);
    }

    #[test]
    fn run_passthrough_nonexistent() {
        let result = run_passthrough(&["__nonexistent_cmd_xyz__".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn run_captured_nonexistent_command() {
        let result = run_captured(
            &["__nonexistent_cmd_xyz__".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            false,
        )
        .unwrap();
        assert_eq!(result.exit_code, 127);
    }

    // ── recovery (--recover) tests ─────────────────────────────────────
    // Distinct argv[0] per test → distinct temp filenames, so parallel runs
    // don't race on a shared recovery file.

    #[test]
    fn recover_keeps_full_output_on_failure() {
        let result = run_captured(
            &["sh".into(), "-c".into(), "seq 1 500; exit 1".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            true,
        )
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(result.filter_result.truncated);
        let path = result
            .recovery_path
            .expect("recovery file kept on failure + truncation");
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 500, "all lines preserved");
        assert!(body.contains("\n250\n"), "the dropped middle is present");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recover_absent_on_success() {
        // `seq` → label "seq", a distinct temp file from the failure test.
        let result = run_captured(
            &["seq".into(), "1".into(), "500".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            true,
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.filter_result.truncated);
        assert!(
            result.recovery_path.is_none(),
            "no recovery file on success"
        );
    }

    #[test]
    fn recover_absent_when_not_truncated() {
        // `false` → fails with no output → nothing was truncated → nothing to recover.
        let result = run_captured(
            &["false".into()],
            30,
            30,
            120,
            100,
            false,
            false,
            false,
            0,
            true,
        )
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(result.recovery_path.is_none());
    }

    // ── clean-success squelch tests ────────────────────────────────────
    // Distinct argv[0] per test → distinct recovery temp filenames (unused
    // here since recover=false, but keeps the pattern consistent).

    #[test]
    fn squelch_trims_clean_success_tail() {
        let args = ["seq".to_string(), "1".into(), "500".into()];
        // Clean exit 0, no error signal → squelch trims the success tail.
        let squelched =
            run_captured(&args, 30, 30, 120, 100, false, false, true, 0, false).unwrap();
        // Same command, squelch off → full success tail (legacy behavior).
        let full = run_captured(&args, 30, 30, 120, 100, false, false, false, 0, false).unwrap();
        assert_eq!(squelched.exit_code, 0);
        assert!(squelched.filter_result.truncated);
        assert!(
            squelched.filter_result.output.lines().count()
                < full.filter_result.output.lines().count(),
            "squelch should keep fewer tail lines on a clean success: squelched={}, full={}",
            squelched.filter_result.output.lines().count(),
            full.filter_result.output.lines().count(),
        );
        // The final summary line must always survive the squelch.
        assert!(
            squelched.filter_result.output.contains("500"),
            "the last line (500) must survive the squelched tail"
        );
    }

    #[test]
    fn squelch_backs_off_on_error_signal() {
        // exit 0, but a warning is printed → the squelch must NOT trim.
        let warn_args = [
            "sh".to_string(),
            "-c".into(),
            "echo 'warning: heads up'; seq 1 500".into(),
        ];
        let with_warn =
            run_captured(&warn_args, 30, 30, 120, 100, false, false, true, 0, false).unwrap();
        let clean_args = ["seq".to_string(), "1".into(), "500".into()];
        let clean =
            run_captured(&clean_args, 30, 30, 120, 100, false, false, true, 0, false).unwrap();
        assert_eq!(with_warn.exit_code, 0);
        assert!(
            with_warn.filter_result.output.lines().count()
                > clean.filter_result.output.lines().count(),
            "an error/warning signal must restore the full success tail"
        );
    }

    #[test]
    fn squelch_never_touches_failures() {
        // Failing command: output is identical with squelch on vs off — failures
        // always use the full error tail and are never squelched.
        let args = ["sh".to_string(), "-c".into(), "seq 1 500; exit 1".into()];
        let on = run_captured(&args, 30, 30, 120, 100, false, false, true, 0, false).unwrap();
        let off = run_captured(&args, 30, 30, 120, 100, false, false, false, 0, false).unwrap();
        assert_ne!(on.exit_code, 0);
        assert_eq!(
            on.filter_result.output.lines().count(),
            off.filter_result.output.lines().count(),
            "squelch must not change failing-command output"
        );
    }
}
