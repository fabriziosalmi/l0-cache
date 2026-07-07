#![allow(clippy::manual_is_multiple_of)]

//! Telemetry: local metrics logging and stats aggregation.
//!
//! Appends one JSONL line per execution to `~/.local/share/l0-compressor/metrics.jsonl`.
//! Uses `O_APPEND` for atomic writes (safe for parallel `l0-compressor` invocations on APFS).
//! **Never** causes the wrapped command to fail — all errors are swallowed
//! after a single warning on stderr.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

mod adaptive;
mod datetime;
mod doctor;
mod guard;
mod lock;
mod metric;
mod paths;
mod stats;
mod tuned;

// Date/time helpers used by metrics, stats, and rotation housekeeping.
use datetime::{parse_rfc3339_to_secs, parse_since, rfc3339_now};

/// Public timestamp helper for callers outside this module (e.g. `main.rs`
/// stamping a `TunedParams` line).
pub fn rfc3339_now_for_pub() -> String {
    rfc3339_now()
}

/// Whether a `--since` value parses to a valid window. Exposed so `main` can
/// reject bad input up front: an unparseable value used to be silently
/// ignored, rendering ALL-TIME data under a header that claimed the window
/// (e.g. `--since 7days` → "last 7days" over all-time totals). A leading `+`
/// is rejected too — `u64::parse` accepts it, but the raw string is echoed
/// into the dashboard header ("last +3d"). Empty strings are also rejected.
pub fn since_is_valid(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && !s.starts_with('+') && parse_since(s).is_some()
}
// Safety guard — re-exported so `main` can reach `telemetry::{...}`.
pub use guard::{check_dangerous_command, guard_enabled};
// Advisory file lock — used by the metric, tuned, stats, and doctor paths.
pub(crate) use lock::FileLock;
// Data-directory + sidecar path resolution.
pub(crate) use paths::{metrics_path, migrate_legacy_data_dir, tuned_path};
// Persisted adaptive-tuning state — `lookup_tuned`/`save_tuned`/`TunedParams`
// reach `main`; helpers + consts feed the adaptive engine, stats, and tests.
pub(crate) use tuned::*;
// Metric model + the metrics-log append path. `ExecutionMetric`/`RunMetrics`/
// `append_metric`/`reset_stats` reach `main`; `telemetry_disabled` feeds tuned.
pub(crate) use metric::*;
// Adaptive auto-tuning engine — `get_adaptive_params`/`args_hash`/`BaseParams`/
// `ADAPTIVE_EVENT_*` reach `main` and the stats renderer; helpers feed tests.
pub(crate) use adaptive::*;
// `--doctor` diagnostics — `run_doctor` reaches `main`.
pub(crate) use doctor::run_doctor;
// Stats/discover command — `print_stats`/`run_discover` reach `main`; the
// aggregation model + helpers feed each other across `agg`/`render` and tests.
pub(crate) use stats::*;
// Brought into scope so the in-module `#[cfg(test)] mod tests` (which uses
// `super::*`) can reach these test-only helpers.
#[cfg(test)]
use datetime::to_rfc3339;
#[cfg(test)]
use guard::{
    check_dangerous_command_with_homes, is_critical_target, normalize_guard_path, parse_bool_env,
};
#[cfg(test)]
mod tests;
