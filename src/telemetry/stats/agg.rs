//! Metric aggregation: parse `metrics.jsonl` into a sorted, ready-to-render
//! [`StatsAgg`]. The shared stats data model lives here too.

use crate::telemetry::*;
use std::path::PathBuf;

/// Aggregated stats for a single command. Fields are `pub(crate)`: this is the
/// data contract between the aggregator (this module) and the renderer.
#[derive(Debug, Default)]
pub(crate) struct CmdStats {
    pub(crate) runs: usize,
    pub(crate) tokens_saved_total: usize,
    pub(crate) tokens_raw_total: usize,
    /// Times the `expand_tail_err` rule fired for this command.
    pub(crate) auto_expand: usize,
    /// Times the `decay_moderate` rule fired for this command.
    pub(crate) auto_decay_mod: usize,
    /// Times the `decay_strong` rule fired for this command.
    pub(crate) auto_decay_strong: usize,
    /// Times the `proactive_shrink` (Step 3) rule fired for this command.
    pub(crate) auto_proactive_shrink: usize,
    /// Times the `decay_steady` (Step 4) rule fired for this command.
    pub(crate) auto_decay_steady: usize,
    /// Times the `recover_defaults` (un-ratchet) rule fired for this command.
    pub(crate) auto_recover: usize,
    /// Subset of `auto_expand` where the trigger was semantically empty:
    /// failing exit + zero output lines (classic grep/find "no match"). The
    /// expansion did nothing useful — this counter exposes the false-positive
    /// rate the future Step 1 fix is meant to drive to zero.
    pub(crate) auto_noisy: usize,
}

/// Minimum runs before a command can be flagged low-value — below this the
/// sample is too small to advise on. Shared by the `⚠ low` row marker, the
/// stats footer, and `--discover` so the three surfaces can never disagree.
pub(crate) const LOW_VALUE_MIN_RUNS: usize = 5;
/// Savings percentage below which wrapping is considered not worth it.
/// `pub(crate)`: `ui::pct_code` keys its red tier on this same value, so the
/// row color and the `⚠ low` hint can never drift apart again.
pub(crate) const LOW_VALUE_MAX_PCT: f64 = 10.0;

impl CmdStats {
    pub(crate) fn auto_firings(&self) -> usize {
        self.auto_expand
            + self.auto_decay_mod
            + self.auto_decay_strong
            + self.auto_proactive_shrink
            + self.auto_decay_steady
            + self.auto_recover
    }

    /// Enough runs, real output, but savings under 10% — the prefix is
    /// mostly overhead on this command.
    pub(crate) fn is_low_savings(&self) -> bool {
        self.runs >= LOW_VALUE_MIN_RUNS
            && self.tokens_raw_total > 0
            && pct(self.tokens_saved_total, self.tokens_raw_total) < LOW_VALUE_MAX_PCT
    }

    /// Enough runs and never any output — wrapping can't save anything by
    /// definition (shell builtins like `exit`, zero-output commands). These
    /// used to escape every hint because the low-savings predicate required
    /// `tokens_raw_total > 0`.
    pub(crate) fn is_zero_output(&self) -> bool {
        self.runs >= LOW_VALUE_MIN_RUNS && self.tokens_raw_total == 0
    }

    /// Single source of truth for "stop prefixing this command" advice.
    pub(crate) fn is_low_value(&self) -> bool {
        self.is_low_savings() || self.is_zero_output()
    }
}

/// Metrics aggregated and sorted by tokens saved (desc), ready to render.
/// Fields are `pub(crate)` — the aggregator/renderer data contract.
pub(crate) struct StatsAgg {
    pub(crate) path: PathBuf,
    pub(crate) total_runs: usize,
    pub(crate) total_saved: usize,
    pub(crate) total_raw: usize,
    /// Unweighted median of per-run efficiencies (runs with output only).
    /// Complements the token-weighted headline, which a single huge command
    /// can dominate (one dd benchmark made 77%-real-world read as 98%).
    pub(crate) median_run_pct: f64,
    pub(crate) by_cmd: Vec<(String, CmdStats)>,
    /// Sum across all commands of each rule's firings.
    pub(crate) auto_expand_total: usize,
    pub(crate) auto_decay_mod_total: usize,
    pub(crate) auto_decay_strong_total: usize,
    pub(crate) auto_proactive_shrink_total: usize,
    pub(crate) auto_decay_steady_total: usize,
    pub(crate) auto_recover_total: usize,
    pub(crate) auto_noisy_total: usize,
    /// Timestamp of the most recent noisy firing in the window. Rendered next
    /// to the noisy counter so stale pre-fix history (the noisy-skip landed in
    /// 0.1.10) is distinguishable from a live problem in the all-time view.
    pub(crate) auto_noisy_last_ts: Option<String>,
}

impl StatsAgg {
    pub(crate) fn auto_firings_total(&self) -> usize {
        self.auto_expand_total
            + self.auto_decay_mod_total
            + self.auto_decay_strong_total
            + self.auto_proactive_shrink_total
            + self.auto_decay_steady_total
            + self.auto_recover_total
    }
}

/// Outcome of reading the metrics file for a stats/discover query.
pub(crate) enum StatsData {
    NoDataDir,
    NoFile(PathBuf),
    Empty,
    Ready(StatsAgg),
}

/// Savings percentage of `saved` against `raw` (0 when `raw` is 0).
pub(crate) fn pct(saved: usize, raw: usize) -> f64 {
    if raw > 0 {
        (saved as f64 / raw as f64) * 100.0
    } else {
        0.0
    }
}

/// USD value of `tokens` at `cost_per_mtok` dollars per million tokens.
pub(crate) fn usd(tokens: usize, cost_per_mtok: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * cost_per_mtok
}

/// Whether a cost figure should be shown: finite and positive. Rejects `inf`/`nan`
/// (which would otherwise serialize to JSON `null`) and non-positive rates.
pub(crate) fn cost_shown(cost_per_mtok: f64) -> bool {
    cost_per_mtok.is_finite() && cost_per_mtok > 0.0
}

/// Sanitize an externally-sourced command name for terminal display: drop control
/// characters (the metrics file is user-writable, so `cmd` could carry raw ANSI/
/// escapes — the `--json` path is unaffected since serde escapes them), then clamp
/// to `width` columns (char-boundary safe).
pub(crate) fn safe_label(cmd: &str, width: usize) -> String {
    let clean: String = cmd.chars().filter(|c| !c.is_control()).collect();
    if crate::ui::vis_len(&clean) <= width {
        return clean;
    }
    // Truncate by display COLUMNS, not chars: a CJK char occupies two cells,
    // so a char-count clamp let wide names overflow the column and shatter
    // the box alignment. saturating: width 0 must not underflow.
    let budget = width.saturating_sub(1);
    let mut used = 0;
    let mut out = String::new();
    for c in clean.chars() {
        let w = crate::ui::char_cols(c);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// Pad `s` with spaces to `width` display columns. `format!("{:<w$}")` pads
/// by char count, which under-pads names containing double-width chars.
pub(crate) fn pad_cols(s: &str, width: usize) -> String {
    let used = crate::ui::vis_len(s);
    format!("{}{}", s, " ".repeat(width.saturating_sub(used)))
}

/// Read, parse, and aggregate the metrics file for the `since` window. Shared by
/// `--stats`, `--stats --json`, and `--discover`.
pub(crate) fn aggregate_metrics(since: Option<&str>) -> StatsData {
    let path = match metrics_path() {
        Some(p) => p,
        None => return StatsData::NoDataDir,
    };
    if !path.exists() {
        return StatsData::NoFile(path);
    }

    let mut lock = FileLock::for_data_file(&path);
    let _ = lock.lock(); // best-effort locking

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("l0-compressor: error reading {}: {}", path.display(), e);
            return StatsData::Empty;
        }
    };

    let cutoff =
        since.and_then(|s| parse_since(s).map(|secs| now_unix_secs().saturating_sub(secs)));

    match aggregate_content(&content, cutoff) {
        None => StatsData::Empty,
        Some(mut agg) => {
            agg.path = path;
            StatsData::Ready(agg)
        }
    }
}

/// Pure aggregation over JSONL content — separated from the I/O so the
/// renderer can be unit-tested against fixture records.
pub(crate) fn aggregate_content(content: &str, cutoff: Option<u64>) -> Option<StatsAgg> {
    let mut total_runs: usize = 0;
    let mut total_saved: usize = 0;
    let mut total_raw: usize = 0;
    let mut auto_expand_total: usize = 0;
    let mut auto_decay_mod_total: usize = 0;
    let mut auto_decay_strong_total: usize = 0;
    let mut auto_proactive_shrink_total: usize = 0;
    let mut auto_decay_steady_total: usize = 0;
    let mut auto_recover_total: usize = 0;
    let mut auto_noisy_total: usize = 0;
    let mut auto_noisy_last_ts: Option<String> = None;
    let mut by_cmd: std::collections::HashMap<String, CmdStats> = std::collections::HashMap::new();
    // Per-run efficiencies (runs with output only) for the unweighted median —
    // the token-weighted headline alone lets one huge command bury the rest.
    let mut run_pcts: Vec<f64> = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let metric: ExecutionMetric = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue, // skip malformed lines
        };

        // Fail-closed time filter: a missing/unparseable timestamp is excluded
        // from a windowed query rather than silently counted.
        if let Some(cutoff_time) = cutoff {
            match parse_rfc3339_to_secs(&metric.ts) {
                Some(ts_secs) if ts_secs >= cutoff_time => {}
                _ => continue,
            }
        }

        // Clamp at ingestion: the metrics file is externally writable, and a
        // tampered/corrupt record with saved > raw would otherwise leak >100%
        // into the median, the JSON, and the totals while fmt_pct clamps the
        // text view — three surfaces disagreeing on the same input.
        let saved = metric.tokens_saved.min(metric.tokens_raw);

        total_runs += 1;
        total_saved += saved;
        total_raw += metric.tokens_raw;
        if metric.tokens_raw > 0 {
            run_pcts.push(pct(saved, metric.tokens_raw));
        }

        let entry = by_cmd.entry(metric.cmd.clone()).or_default();
        entry.runs += 1;
        entry.tokens_saved_total += saved;
        entry.tokens_raw_total += metric.tokens_raw;

        // Auto-tuning event classification. Unknown tags are ignored (forward-
        // compat: a newer l0-compressor could write a tag this build doesn't know).
        if let Some(tag) = metric.adaptive_event.as_deref() {
            match tag {
                ADAPTIVE_EVENT_EXPAND_TAIL_ERR => {
                    entry.auto_expand += 1;
                    auto_expand_total += 1;
                    // "Noisy" = the failure-expand rule fired on a failing run
                    // that produced no output at all. Semantically empty: the
                    // command's failure mode is "no match" / "not found", not a
                    // truncated error stream, so the expansion was wasted.
                    if metric.exit_code != 0 && metric.lines_raw == 0 {
                        entry.auto_noisy += 1;
                        auto_noisy_total += 1;
                        // RFC3339 strings order lexicographically.
                        if auto_noisy_last_ts.as_deref() < Some(metric.ts.as_str()) {
                            auto_noisy_last_ts = Some(metric.ts.clone());
                        }
                    }
                }
                ADAPTIVE_EVENT_DECAY_MODERATE => {
                    entry.auto_decay_mod += 1;
                    auto_decay_mod_total += 1;
                }
                ADAPTIVE_EVENT_DECAY_STRONG => {
                    entry.auto_decay_strong += 1;
                    auto_decay_strong_total += 1;
                }
                ADAPTIVE_EVENT_PROACTIVE_SHRINK => {
                    entry.auto_proactive_shrink += 1;
                    auto_proactive_shrink_total += 1;
                }
                ADAPTIVE_EVENT_DECAY_STEADY => {
                    entry.auto_decay_steady += 1;
                    auto_decay_steady_total += 1;
                }
                ADAPTIVE_EVENT_RECOVER => {
                    entry.auto_recover += 1;
                    auto_recover_total += 1;
                }
                _ => {}
            }
        }
    }

    if total_runs == 0 {
        return None;
    }

    let median_run_pct = {
        run_pcts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        match run_pcts.len() {
            0 => 0.0,
            n if n % 2 == 1 => run_pcts[n / 2],
            n => (run_pcts[n / 2 - 1] + run_pcts[n / 2]) / 2.0,
        }
    };

    let mut by_cmd: Vec<(String, CmdStats)> = by_cmd.into_iter().collect();
    // Tie-breakers (runs desc, then name) make the order deterministic: the
    // source is a HashMap, so ties — e.g. several 0-saved commands — used to
    // shuffle on every invocation.
    by_cmd.sort_by(|(name_a, a), (name_b, b)| {
        b.tokens_saved_total
            .cmp(&a.tokens_saved_total)
            .then(b.runs.cmp(&a.runs))
            .then(name_a.cmp(name_b))
    });

    Some(StatsAgg {
        path: PathBuf::new(), // set by aggregate_metrics
        total_runs,
        total_saved,
        total_raw,
        median_run_pct,
        by_cmd,
        auto_expand_total,
        auto_decay_mod_total,
        auto_decay_strong_total,
        auto_proactive_shrink_total,
        auto_decay_steady_total,
        auto_recover_total,
        auto_noisy_total,
        auto_noisy_last_ts,
    })
}
