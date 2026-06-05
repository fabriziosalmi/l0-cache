//! Best-effort full-output recovery.
//!
//! When a captured command **fails** (non-zero exit) *and* its output was
//! truncated, the agent often needs the lines that were dropped from the middle.
//! Re-running the command wastes tokens and may not be idempotent. Instead, this
//! writer tees the un-truncated output to a temp file and the run banner points
//! the agent at it, so it can read the omitted lines without re-executing.
//!
//! It is designed to cost (almost) nothing unless it is needed:
//!   * **Lazy** — output is buffered in memory until truncation becomes possible
//!     (more than `threshold` lines) or the prebuffer grows past a small cap, so
//!     the common case (small output) never touches the disk.
//!   * **Bounded** — the in-memory prebuffer is byte-capped, and the file itself
//!     is size-capped, so neither memory nor disk can blow up.
//!   * **Fail-safe** — any I/O error disables it silently; it never affects the
//!     command's output, exit code, or the metrics.
//!   * **Kept only when useful** — the file is retained only on failure + truncation;
//!     otherwise it is removed on finalize.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

/// Flush the prebuffer to disk early if a few giant lines arrive before the
/// line-count threshold is crossed (keeps extra memory bounded).
const PREBUF_BYTES_CAP: usize = 256 * 1024;

/// Stop writing past this size; the recovery file is a safety net, not an archive.
const FILE_BYTES_CAP: usize = 64 * 1024 * 1024;

/// Tees full output to a temp file for failure recovery. See module docs.
pub struct Recovery {
    active: bool,
    failed: bool,
    threshold: usize,
    path: PathBuf,
    line_count: usize,
    prebuf: Vec<u8>,
    file: Option<BufWriter<File>>,
    written: usize,
    capped: bool,
}

impl Recovery {
    /// `enabled` is the `--recover` switch (and false in raw/binary modes).
    /// `cmd_label` names the temp file; `threshold` matches the truncation threshold.
    pub fn new(enabled: bool, cmd_label: &str, threshold: usize) -> Self {
        Recovery {
            active: enabled,
            failed: false,
            threshold,
            path: recovery_path(cmd_label),
            line_count: 0,
            prebuf: Vec::new(),
            file: None,
            written: 0,
            capped: false,
        }
    }

    /// Feed one output line (newline excluded). Cheap no-op when inactive.
    pub fn feed(&mut self, line: &str) {
        if !self.active || self.failed {
            return;
        }
        self.line_count += 1;
        if self.file.is_some() {
            self.write_line(line.as_bytes());
            return;
        }
        // Still buffering: keep in memory until truncation is possible.
        self.prebuf.extend_from_slice(line.as_bytes());
        self.prebuf.push(b'\n');
        if self.line_count > self.threshold || self.prebuf.len() > PREBUF_BYTES_CAP {
            self.open_and_flush();
        }
    }

    fn open_and_flush(&mut self) {
        let dir = match self.path.parent() {
            Some(d) => d.to_path_buf(),
            None => {
                self.failed = true;
                return;
            }
        };
        if fs::create_dir_all(&dir).is_err() {
            self.failed = true;
            return;
        }
        let file = match File::create(&self.path) {
            Ok(f) => f,
            Err(_) => {
                self.failed = true;
                return;
            }
        };
        let mut w = BufWriter::new(file);
        let prebuf = std::mem::take(&mut self.prebuf);
        if w.write_all(&prebuf).is_err() {
            self.failed = true;
            return;
        }
        self.written = prebuf.len();
        self.file = Some(w);
    }

    fn write_line(&mut self, bytes: &[u8]) {
        if self.written >= FILE_BYTES_CAP {
            self.capped = true;
            return;
        }
        if let Some(w) = self.file.as_mut() {
            if w.write_all(bytes).and_then(|_| w.write_all(b"\n")).is_err() {
                self.failed = true;
                return;
            }
            self.written += bytes.len() + 1;
        }
    }

    /// Finalize. Returns the recovery file path only when `keep` is true (the
    /// caller passes `exit_code != 0 && truncated`) and a file was actually
    /// written; otherwise removes any partial file and returns `None`.
    pub fn finalize(mut self, keep: bool) -> Option<PathBuf> {
        if !self.active || self.failed {
            return None;
        }
        let mut w = self.file.take()?; // None => never crossed threshold => nothing to recover
        if self.capped {
            let _ = w.write_all(b"... [l0-cache: recovery file capped] ...\n");
        }
        if w.flush().is_err() {
            let _ = fs::remove_file(&self.path);
            return None;
        }
        if keep {
            Some(self.path)
        } else {
            let _ = fs::remove_file(&self.path);
            None
        }
    }
}

/// `<tmp>/l0-cache/recovery-<sanitized-cmd>.log`. The stable per-command name
/// means repeated failures of the same command reuse one file (bounded clutter).
fn recovery_path(cmd_label: &str) -> PathBuf {
    let safe: String = cmd_label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let safe = if safe.is_empty() {
        "cmd".to_string()
    } else {
        safe
    };
    std::env::temp_dir()
        .join("l0-cache")
        .join(format!("recovery-{}.log", safe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_never_touches_disk() {
        let mut r = Recovery::new(false, "x", 2);
        for _ in 0..100 {
            r.feed("line");
        }
        assert!(r.finalize(true).is_none());
    }

    #[test]
    fn small_output_creates_no_file() {
        // Stays at/under threshold → never opens a file → nothing to recover.
        let mut r = Recovery::new(true, "small-test", 10);
        for i in 0..5 {
            r.feed(&format!("line {i}"));
        }
        assert!(r.finalize(true).is_none());
    }

    #[test]
    fn keeps_file_on_failure_with_all_lines() {
        let mut r = Recovery::new(true, "keep-test", 3);
        for i in 0..20 {
            r.feed(&format!("line-{i}"));
        }
        let path = r.finalize(true).expect("should keep on failure");
        let body = fs::read_to_string(&path).unwrap();
        // The middle (which head/tail truncation would drop) is present.
        assert!(body.contains("line-0"));
        assert!(body.contains("line-10"));
        assert!(body.contains("line-19"));
        assert_eq!(body.lines().count(), 20);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn removes_file_when_not_kept() {
        let mut r = Recovery::new(true, "drop-test", 3);
        for i in 0..20 {
            r.feed(&format!("l{i}"));
        }
        let path = recovery_path("drop-test");
        assert!(path.exists(), "file should exist mid-run");
        assert!(r.finalize(false).is_none());
        assert!(!path.exists(), "file should be removed when not kept");
    }

    #[test]
    fn sanitizes_command_label() {
        let p = recovery_path("../../etc/passwd; rm -rf /");
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(!name.contains('/'));
        assert!(!name.contains(' '));
        assert!(name.starts_with("recovery-"));
    }
}
