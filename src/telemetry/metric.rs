//! Execution-metric model and the metrics-log append path.
//!
//! One JSONL line per execution to `~/.local/share/l0-compressor/metrics.jsonl`,
//! `O_APPEND` for atomic concurrent writes. Never fails the wrapped command.

use super::*;

/// A single execution record.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct ExecutionMetric {
    pub ts: String,
    pub cmd: String,
    pub args: String,
    pub bytes_raw: usize,
    pub bytes_final: usize,
    pub lines_raw: usize,
    pub lines_final: usize,
    pub tokens_raw: usize,
    pub tokens_final: usize,
    pub tokens_saved: usize,
    pub truncated: bool,
    pub strategy: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    #[serde(alias = "t_version")]
    pub version: String,
    /// Adaptive-tuning event tag for this run; `None` when the tuning rule did
    /// not fire (or was disabled via `--no-auto`). Absent from the JSONL line
    /// when `None`, so back-compat with older records is preserved both ways.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_event: Option<String>,
    /// 8-hex-char FNV-1a hash of the (redacted) args string. Used by the
    /// adaptive learner as the per-bucket key alongside `cmd`. `None` on
    /// pre-Step-2 records — those are ignored by the learner but still
    /// counted in --stats totals, so back-compat works both ways.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_hash: Option<String>,
}

/// Inputs for building an execution metric.
pub struct RunMetrics<'a> {
    pub cmd: &'a str,
    pub args: &'a str,
    pub bytes_raw: usize,
    pub bytes_final: usize,
    pub lines_raw: usize,
    pub lines_final: usize,
    pub truncated: bool,
    pub strategy: &'a str,
    pub exit_code: i32,
    pub duration_ms: u64,
    /// Adaptive-tuning event that fired for this run (one of the
    /// `ADAPTIVE_EVENT_*` constants), or `None` when no rule branch fired.
    pub adaptive_event: Option<&'a str>,
    /// FNV-1a hash of the (redacted) args string, as computed by [`args_hash`].
    /// `Some` in production; `None` only in test fixtures that don't exercise
    /// per-bucket learning.
    pub args_hash: Option<&'a str>,
}

impl ExecutionMetric {
    /// Create a metric from run results with the default token divisor.
    /// Test-only convenience; production passes an explicit `--token-factor`.
    #[cfg(test)]
    pub fn from_run(m: RunMetrics<'_>) -> Self {
        Self::from_run_with_factor(m, 4)
    }

    /// Create a metric from run results with a custom token divisor.
    pub fn from_run_with_factor(m: RunMetrics<'_>, token_factor: usize) -> Self {
        let divisor = if token_factor == 0 { 4 } else { token_factor };
        let tokens_raw = m.bytes_raw / divisor;
        let tokens_final = m.bytes_final / divisor;
        let tokens_saved = tokens_raw.saturating_sub(tokens_final);

        Self {
            ts: rfc3339_now(),
            cmd: m.cmd.to_string(),
            args: m.args.to_string(),
            bytes_raw: m.bytes_raw,
            bytes_final: m.bytes_final,
            lines_raw: m.lines_raw,
            lines_final: m.lines_final,
            tokens_raw,
            tokens_final,
            tokens_saved,
            truncated: m.truncated,
            strategy: m.strategy.to_string(),
            exit_code: m.exit_code,
            duration_ms: m.duration_ms,
            version: env!("CARGO_PKG_VERSION").to_string(),
            adaptive_event: m.adaptive_event.map(|s| s.to_string()),
            args_hash: m.args_hash.map(|s| s.to_string()),
        }
    }
}

/// Delete all recorded telemetry statistics, including the adaptive-tuning
/// sidecar — leaving `tuned.jsonl` behind meant stale adaptive state kept
/// seeding runs while `--stats` reported "No metrics found".
///
/// Lock artifacts (the flock files and any legacy mkdir lock directory) are
/// removed best-effort: reset is an explicit, rare user action, and a writer
/// racing it simply recreates its lock on the next run.
pub fn reset_stats() -> std::io::Result<()> {
    for path in [metrics_path(), tuned_path()].into_iter().flatten() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let _ = std::fs::remove_file(path.with_extension("jsonl.flock"));
        let _ = std::fs::remove_dir(path.with_extension("jsonl.lock"));
    }
    Ok(())
}

/// Maximum metrics file size before rotation (10MB).
const METRICS_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Whether telemetry writes are disabled via `L0_COMPRESSOR_NO_TELEMETRY` (truthy
/// values per `parse_bool_env`: 1/true/yes/on). Used by test harnesses and
/// benchmark scripts so they never pollute the user's real metrics/tuning
/// files, regardless of `XDG_DATA_HOME` isolation. The pre-rename
/// `L0_CACHE_NO_TELEMETRY` name is honored as a deprecated fallback.
pub(crate) fn telemetry_disabled() -> bool {
    std::env::var("L0_COMPRESSOR_NO_TELEMETRY")
        .or_else(|_| std::env::var("L0_CACHE_NO_TELEMETRY"))
        .ok()
        .and_then(|v| super::guard::parse_bool_env(&v))
        .unwrap_or(false)
}

/// Append a metric to the JSONL file. Fail-safe: errors → stderr warning, never panics.
pub fn append_metric(metric: &ExecutionMetric, quiet: bool) {
    if telemetry_disabled() {
        return;
    }
    let path = match metrics_path() {
        Some(p) => p,
        None => {
            if !quiet {
                eprintln!(
                    "l0-compressor: warning: $HOME not set, cannot write metrics (common in containers/cron)"
                );
            }
            return;
        }
    };

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            if !quiet {
                eprintln!(
                    "l0-compressor: warning: cannot create {}: {}",
                    parent.display(),
                    e
                );
            }
            return;
        }
    }

    // Acquire lock for write and rotation
    let mut lock = FileLock::for_data_file(&path);
    let _ = lock.lock(); // best-effort locking

    // Auto-rotate if file is too large
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > METRICS_MAX_BYTES {
            let old = path.with_extension("jsonl.old");
            if fs::rename(&path, &old).is_ok() {
                // Perform housekeeping: filter out entries older than 30 days
                if let Ok(content) = fs::read_to_string(&old) {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let cutoff = now_secs.saturating_sub(30 * 86400); // 30 days

                    let mut kept_lines = Vec::new();
                    for line in content.lines() {
                        if let Ok(metric) = serde_json::from_str::<ExecutionMetric>(line) {
                            if let Some(ts_secs) = parse_rfc3339_to_secs(&metric.ts) {
                                if ts_secs >= cutoff {
                                    kept_lines.push(line.to_string());
                                }
                            }
                        }
                    }

                    // Size cap: if age-pruning alone still leaves the file above the
                    // rotation target (e.g. a heavy user generating >10 MB within 30
                    // days), keep only the most-recent lines that fit in half the max.
                    // Without this, an all-recent oversized file would re-rotate (and
                    // rewrite itself in full) on EVERY subsequent invocation.
                    let target = (METRICS_MAX_BYTES / 2) as usize;
                    let mut start = kept_lines.len();
                    let mut budget = target;
                    for i in (0..kept_lines.len()).rev() {
                        let needed = kept_lines[i].len() + 1; // line + '\n'
                        if needed > budget {
                            break;
                        }
                        budget -= needed;
                        start = i;
                    }
                    let kept_lines = &kept_lines[start..];

                    // Write the pruned history back into the ACTIVE file (`path`),
                    // not the rotated-away copy — otherwise `print_stats` and
                    // `get_adaptive_params` would see an empty file after every
                    // rotation. The new metric is then appended below.
                    let rotate_open = {
                        let mut opts = OpenOptions::new();
                        opts.create(true).write(true).truncate(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            opts.mode(0o600);
                        }
                        opts.open(&path)
                    };
                    if let Ok(mut file) = rotate_open {
                        for line in kept_lines {
                            let _ = writeln!(file, "{}", line);
                        }
                        // History preserved in `path`; discard the rotated copy.
                        let _ = fs::remove_file(&old);
                    }
                }
            }
        }
    }

    // Serialize to a single line
    let mut line = match serde_json::to_string(metric) {
        Ok(s) => s,
        Err(e) => {
            if !quiet {
                eprintln!("l0-compressor: warning: cannot serialize metric: {}", e);
            }
            return;
        }
    };
    line.push('\n');

    // Append atomically (O_APPEND). Create new files 0600 via the open mode so
    // there is no window where the file is world-readable before a chmod.
    let open_result = {
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&path)
    };
    match open_result {
        Ok(mut file) => {
            // Fix up a pre-existing file's perms (open mode only applies on create).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
            }
            if let Err(e) = file.write_all(line.as_bytes()) {
                if !quiet {
                    eprintln!(
                        "l0-compressor: warning: cannot write to {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
        Err(e) => {
            if !quiet {
                eprintln!(
                    "l0-compressor: warning: cannot open {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
}
