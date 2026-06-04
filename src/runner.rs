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
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::filter::{self, FilterPipeline, FilterResult};

/// Result of running a command through the proxy.
pub struct RunResult {
    pub filter_result: FilterResult,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub strategy: &'static str,
}

/// Maximum length of a single line before we force-truncate (1MB).
/// Prevents OOM on binary files that slip past detection (no newlines).
const MAX_LINE_BYTES: usize = 1_048_576;

/// Maximum total bytes collected in raw mode (256MB).
/// Prevents OOM when `--raw` is used on commands with massive output.
const RAW_MODE_MAX_BYTES: usize = 256 * 1024 * 1024;

/// Spawn a command via shell with explicit `2>&1` merge.
fn spawn_merged(cmd: &[String]) -> std::io::Result<(Child, BufReader<std::process::ChildStdout>)> {
    // Build a shell command string with proper escaping
    let shell_cmd = cmd
        .iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ");

    let child_res = Command::new("sh")
        .arg("-c")
        .arg(format!("{} 2>&1", shell_cmd))
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // stderr is already merged into stdout
        .spawn();

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

/// Read one line from the reader, extracting the bytes up to the newline.
/// Returns `None` at EOF.
/// Truncates lines longer than `MAX_LINE_BYTES` to prevent OOM.
fn read_line_bytes<'a>(
    reader: &mut BufReader<impl Read>,
    buf: &'a mut Vec<u8>,
) -> Option<&'a [u8]> {
    buf.clear();
    match reader.read_until(b'\n', buf) {
        Ok(0) => None, // EOF
        Ok(_) => {
            // Remove trailing newline
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            if buf.last() == Some(&b'\r') {
                buf.pop(); // handle \r\n (Windows line endings from SSH etc.)
            }

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
            // If the line is large (> 2000 bytes) and looks like JSON, aggressively truncate
            // it early to save the LLM from massive single-line JSON payloads.
            if buf.len() > 2000 {
                let first_non_whitespace = buf.iter().find(|&&b| b != b' ' && b != b'\t');
                let is_json = matches!(first_non_whitespace, Some(&b'{') | Some(&b'['));

                if is_json {
                    let keep_bytes = 2000;
                    buf.truncate(keep_bytes);
                    let suffix = b"\n... [Large JSON Payload Truncated for LLM] ...";
                    buf.extend_from_slice(suffix);
                } else if buf.len() > MAX_LINE_BYTES {
                    // Fallback generic truncation
                    buf.truncate(MAX_LINE_BYTES);
                    let suffix = b"... [line truncated at 1MB]";
                    let start = MAX_LINE_BYTES - suffix.len();
                    buf[start..].copy_from_slice(suffix);
                }
            }
            Some(buf.as_slice())
        }
        Err(_) => None, // I/O error = treat as EOF
    }
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
    idle_timeout: u64,
) -> std::io::Result<RunResult> {
    let start = Instant::now();

    let (mut child, mut reader) = spawn_merged(cmd)?;

    // Reusable line buffer for read_line_bytes
    let mut line_buf: Vec<u8> = Vec::with_capacity(4096);

    // Track raw bytes for binary detection on first chunk
    let mut first_chunk = Vec::new();
    let mut is_binary = false;
    let mut raw_bytes_total: usize = 0;

    let mut all_lines: Vec<String> = Vec::new();
    let mut raw_capped = false;
    let mut pipeline = if !raw_mode {
        Some(FilterPipeline::new(head_cap, tail_cap, only_errors))
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
                    let _ = std::process::Command::new("kill")
                        .arg("-9")
                        .arg(child_id.to_string())
                        .status();
                    return true;
                }
            }
        }))
    } else {
        None
    };

    while let Some(line_bytes) = read_line_bytes(&mut reader, &mut line_buf) {
        if idle_timeout > 0 {
            last_output_time.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        raw_bytes_total += line_bytes.len() + 1;

        // Binary detection on first ~8KB
        if first_chunk.len() < 8192 {
            first_chunk.extend_from_slice(line_bytes);
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
            let stripped = filter::strip_ansi(line_bytes);
            all_lines.push(stripped.into_owned());
        } else {
            let stripped = filter::strip_ansi(line_bytes);
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
    let exit_code = exit_code_from_status(status);
    let duration_ms = start.elapsed().as_millis() as u64;

    if is_binary {
        // Binary output: passthrough, no filtering
        return Ok(RunResult {
            filter_result: FilterResult {
                output: String::from_utf8_lossy(&first_chunk).into_owned(),
                lines_raw: 0,
                lines_final: 0,
                bytes_raw: raw_bytes_total,
                bytes_final: raw_bytes_total,
                truncated: false,
            },
            exit_code,
            duration_ms,
            strategy: "binary_skip",
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
        })
    } else {
        let mut pipe = pipeline.unwrap();
        // If error exit, expand tail
        if exit_code != 0 {
            pipe.expand_tail(tail_error_cap);
        }

        let mut filter_result = pipe.finish(threshold, raw_bytes_total);
        if killed_by_watchdog {
            let msg = format!("\n... [l0-cache: Command killed due to {}s output inactivity. Is it waiting for interactive input?] ...\n", idle_timeout);
            filter_result.output.push_str(&msg);
            // Count it as truncated so it triggers banner logic
            filter_result.truncated = true;
        }

        Ok(RunResult {
            filter_result,
            exit_code,
            duration_ms,
            strategy: "head_tail",
        })
    }
}

/// Run a command in passthrough mode: inherit all stdio, no capture.
pub fn run_passthrough(cmd: &[String]) -> std::io::Result<i32> {
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

    #[test]
    fn read_line_bytes_normal() {
        let data = b"hello\nworld\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b"hello"[..]));
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b"world"[..]));
        assert_eq!(read_line_bytes(&mut reader, &mut buf), None);
    }

    #[test]
    fn read_line_bytes_crlf() {
        let data = b"windows\r\nline\r\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line_bytes(&mut reader, &mut buf),
            Some(&b"windows"[..])
        );
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b"line"[..]));
    }

    #[test]
    fn read_line_bytes_no_trailing_newline() {
        let data = b"no newline";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line_bytes(&mut reader, &mut buf),
            Some(&b"no newline"[..])
        );
        assert_eq!(read_line_bytes(&mut reader, &mut buf), None);
    }

    #[test]
    fn read_line_bytes_empty() {
        let mut buf = Vec::new();
        let data = b"";
        let mut reader = BufReader::new(&data[..]);
        let res = read_line_bytes(&mut reader, &mut buf);
        assert_eq!(res, None);
    }

    #[test]
    fn read_line_bytes_progress_bar_squash() {
        let mut buf = Vec::new();
        // Simulates: "Downloading... 10%\rDownloading... 50%\rDownloading... 100%\n"
        let data = b"Downloading... 10%\rDownloading... 50%\rDownloading... 100%\n";
        let mut reader = BufReader::new(&data[..]);
        let res = read_line_bytes(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"Downloading... 100%");
    }

    #[test]
    fn read_line_bytes_backspace_and_bell() {
        let mut buf = Vec::new();
        // "foo\x07bar\x08baz\n" -> "foobabaz"
        let data = b"foo\x07bar\x08baz\n";
        let mut reader = BufReader::new(&data[..]);
        let res = read_line_bytes(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"foobabaz");
    }

    #[test]
    fn read_line_bytes_backspace_utf8() {
        let mut buf = Vec::new();
        // "hello \xF0\x9F\x9A\x80\x08world\n" -> "hello world" (rocket emoji followed by backspace)
        let data = b"hello \xF0\x9F\x9A\x80\x08world\n";
        let mut reader = BufReader::new(&data[..]);
        let res = read_line_bytes(&mut reader, &mut buf);
        assert_eq!(res.unwrap(), b"hello world");
    }

    #[test]
    fn read_line_bytes_blank_lines() {
        let data = b"\n\n\n";
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(read_line_bytes(&mut reader, &mut buf), Some(&b""[..]));
        assert_eq!(read_line_bytes(&mut reader, &mut buf), None);
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
            0,
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.filter_result.output.contains("hello world"));
        assert_eq!(result.strategy, "head_tail");
    }

    #[test]
    fn run_false_returns_nonzero() {
        let result = run_captured(&["false".into()], 30, 30, 120, 100, false, false, 0).unwrap();
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
            0,
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
            0,
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
            0,
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
            0,
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
            0,
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
            0,
        )
        .unwrap();
        assert!(!result.filter_result.truncated);
    }

    #[test]
    fn run_captured_empty_output() {
        let result = run_captured(&["true".into()], 30, 30, 120, 100, false, false, 0).unwrap();
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
            0,
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
            0,
        )
        .unwrap();
        assert_eq!(result.exit_code, 127);
    }
}
