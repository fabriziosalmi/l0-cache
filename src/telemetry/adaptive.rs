//! Adaptive auto-tuning engine.
//!
//! Learns per-`(cmd, args_hash)` head/tail/tail_error from each bucket's
//! recent metric history (failure backoff, decay tiers, proactive + steady
//! shrink, recover) and persists the result via [`super::save_tuned`].

use super::*;

/// Adaptive parameters computed via historical command executions.
#[derive(Debug, PartialEq, Eq)]
pub struct AdaptiveParams {
    pub head: usize,
    pub tail: usize,
    pub tail_error: usize,
    pub modified: bool,
    pub reason: Option<String>,
    /// Which rule branch fired, `Some` only when the params actually changed.
    /// A trigger whose numeric result equals the seeded default (ceiling/floor
    /// pinned) returns `None`: recording those as events inflated the --stats
    /// Firings counter ~13x for floor-pinned buckets (one no-op event per run).
    pub event: Option<&'static str>,
}

/// Adaptive-tuning event tags written to the metrics file.
pub const ADAPTIVE_EVENT_EXPAND_TAIL_ERR: &str = "expand_tail_err";
pub const ADAPTIVE_EVENT_DECAY_MODERATE: &str = "decay_moderate";
pub const ADAPTIVE_EVENT_DECAY_STRONG: &str = "decay_strong";
/// Step 3: proactive shrink — fires on long clean histories where the cap is
/// demonstrably wasted budget. Max-based (not p95), so it never introduces
/// new truncations vs. observed history.
pub const ADAPTIVE_EVENT_PROACTIVE_SHRINK: &str = "proactive_shrink";
/// Step 4: steady-state decay — catches the "consistently truncated" pattern
/// that the consecutive-counting decay rule misses when the streak is broken
/// by occasional non-truncated runs interleaved with truncated ones.
pub const ADAPTIVE_EVENT_DECAY_STEADY: &str = "decay_steady";
/// Recovery — the un-ratchet. Every other rule moves head/tail down and
/// tail_error up only, compounding across runs with no way back; when the
/// workload changes, a stale tune persisted months ago would otherwise keep
/// truncating output that fits the configured base just fine. Fires when the
/// persisted tune is demonstrably counterproductive (see the rule body).
pub const ADAPTIVE_EVENT_RECOVER: &str = "recover_defaults";

/// Clean (successful, non-truncated) consecutive runs required before an
/// expanded `tail_error` is restored to its configured base.
pub(crate) const RECOVER_CLEAN_MIN_RUNS: usize = 5;

/// The config/CLI-resolved parameters a bucket would use if no tune had ever
/// been persisted — the target the recovery rule restores toward. Captured in
/// `main` BEFORE `lookup_tuned` seeding overwrites the resolved values.
#[derive(Debug, Clone, Copy)]
pub struct BaseParams {
    pub head: usize,
    pub tail: usize,
    pub tail_error: usize,
}

const PROACTIVE_MIN_RUNS: usize = 20;
const PROACTIVE_MAX_SCAN: usize = 50;
const PROACTIVE_MARGIN_LINES: usize = 5;

/// Window size for Step 4's steady-state check.
const STEADY_WINDOW_RUNS: usize = 20;
/// Minimum number of records the bucket must have for Step 4 to even look —
/// fewer than this and there's no steady-state to read.
const STEADY_MIN_RUNS: usize = 20;
/// Fraction of the window that must be truncated successes to fire steady
/// decay, as numerator over denominator (16/20 = 80%).
const STEADY_TRUNCATED_NUM: usize = 16;
const STEADY_TRUNCATED_DEN: usize = 20;
/// Steady decay's shrink factor — 30% reduction, between moderate (20%) and
/// strong (40%). It's a stronger signal than 3 consecutive truncations but
/// weaker than 5+, so it sits in the middle by design.
const STEADY_DECAY_NUM: usize = 70;
const STEADY_DECAY_DEN: usize = 100;

/// FNV-1a 64-bit constants — RFC-stable, no new dependency, fast.
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Per-bucket key for adaptive learning: a deterministic FNV-1a 64-bit hash
/// of the (redacted) args string, rendered as 8 hex chars (low 32 bits).
///
/// Why this exists: before Step 2 the learning bucketed by `cmd` alone, so
/// `curl https://api.openai.com/...` and `curl https://example.com` shared
/// one streak — wildly different output profiles polluting each other's
/// history. With `args_hash`, distinct args land in distinct buckets and the
/// learning sees coherent signal.
///
/// Why not std's DefaultHasher: it's SipHash whose output isn't guaranteed
/// stable across Rust versions. FNV-1a is RFC-pinned and 8 lines of code.
///
/// Why 8 chars: ~4.3 billion buckets — collision-free for any single user's
/// command vocabulary — and keeps the JSONL line short.
pub fn args_hash(args: &str) -> String {
    let mut h = FNV_OFFSET_BASIS;
    for b in args.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", h as u32)
}

/// How many bytes from the end of the metrics file `get_adaptive_params` reads.
/// Adaptive tuning only needs the last few matching entries, so reading the whole
/// (up to 10 MB) file on every wrapped command is wasteful; the tail is enough.
const ADAPTIVE_READ_TAIL_BYTES: u64 = 256 * 1024;

/// Read up to `max_bytes` from the END of a file as (lossy) UTF-8, dropping the
/// partial first line when the read started mid-file. Returns `None` on I/O error.
pub(crate) fn read_tail_lossy(path: &std::path::Path, max_bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(nl) = s.find('\n') {
            s.drain(..=nl); // discard the (possibly partial) first line
        }
    }
    Some(s)
}

/// Analyze historical metrics to compute tuned head/tail parameters.
///
/// `args_hash` is the per-bucket key of the current run; the learner only
/// considers history records whose own `args_hash` matches. Records from
/// before Step 2 (no args_hash field) are skipped — graceful degradation.
///
/// `default_*` are the (possibly tune-seeded) values the run starts from;
/// `base` is the pre-seed config/CLI resolution and `seeded_by` the event tag
/// of the persisted tune that did the seeding — both used by the recovery rule.
#[allow(clippy::too_many_arguments)]
pub fn get_adaptive_params(
    cmd_name: &str,
    args_hash: &str,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
    base: BaseParams,
    seeded_by: Option<&str>,
    auto_floor: usize,
    auto_ceiling: usize,
) -> AdaptiveParams {
    let path = match metrics_path() {
        Some(p) => p,
        None => {
            return AdaptiveParams {
                head: default_head,
                tail: default_tail,
                tail_error: default_tail_error,
                modified: false,
                reason: None,
                event: None,
            };
        }
    };

    if !path.exists() {
        return AdaptiveParams {
            head: default_head,
            tail: default_tail,
            tail_error: default_tail_error,
            modified: false,
            reason: None,
            event: None,
        };
    }

    let content = match read_tail_lossy(&path, ADAPTIVE_READ_TAIL_BYTES) {
        Some(c) => c,
        None => {
            return AdaptiveParams {
                head: default_head,
                tail: default_tail,
                tail_error: default_tail_error,
                modified: false,
                reason: None,
                event: None,
            };
        }
    };

    get_adaptive_params_with_base(
        &content,
        cmd_name,
        args_hash,
        default_head,
        default_tail,
        default_tail_error,
        base,
        seeded_by,
        auto_floor,
        auto_ceiling,
    )
}

const ADAPTIVE_DECAY_MIN_SUCCESSES: usize = 3;
const ADAPTIVE_DECAY_MAX_SUCCESSES: usize = 5;
const DECAY_FACTOR_MODERATE_NUM: usize = 80;
const DECAY_FACTOR_STRONG_NUM: usize = 60;
const DECAY_FACTOR_DENOM: usize = 100;

/// Analyze metrics log content to compute tuned parameters with default limits.
/// Test-only; production calls `_with_limits` with the configured floor/ceiling.
///
/// Tests that don't care about bucketing can pass the empty string as
/// `args_hash`, which matches itself only (any non-empty bucket hash differs).
#[cfg(test)]
pub(crate) fn get_adaptive_params_from_content(
    content: &str,
    cmd_name: &str,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
) -> AdaptiveParams {
    get_adaptive_params_from_content_with_limits(
        content,
        cmd_name,
        "",
        default_head,
        default_tail,
        default_tail_error,
        10,
        1000,
    )
}

/// Back-compat shim for the pre-recovery signature: `base` = the defaults,
/// which makes the recovery rule inert (nothing to recover toward). The
/// production path (`get_adaptive_params`) passes the real pre-seed base.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_adaptive_params_from_content_with_limits(
    content: &str,
    cmd_name: &str,
    args_hash: &str,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
    auto_floor: usize,
    auto_ceiling: usize,
) -> AdaptiveParams {
    get_adaptive_params_with_base(
        content,
        cmd_name,
        args_hash,
        default_head,
        default_tail,
        default_tail_error,
        BaseParams {
            head: default_head,
            tail: default_tail,
            tail_error: default_tail_error,
        },
        None,
        auto_floor,
        auto_ceiling,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn get_adaptive_params_with_base(
    content: &str,
    cmd_name: &str,
    args_hash: &str,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
    base: BaseParams,
    seeded_by: Option<&str>,
    auto_floor: usize,
    auto_ceiling: usize,
) -> AdaptiveParams {
    // 1. Scan and collect the last 5 execution metrics for THIS (cmd, args_hash)
    //    bucket. Records from a different args bucket — or pre-Step-2 records
    //    that have no args_hash at all — are skipped: they can't speak to the
    //    output profile of the current run.
    //
    // Back-compat: when `args_hash` is empty (only happens through the test
    // helper that didn't pass one) we fall back to cmd-only matching, which
    // preserves the pre-Step-2 semantics existing tests were written against.
    let cmd_only_mode = args_hash.is_empty();
    let mut history = Vec::new();
    for line in content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(metric) = serde_json::from_str::<ExecutionMetric>(line) {
            if metric.cmd != cmd_name {
                continue;
            }
            if !cmd_only_mode && metric.args_hash.as_deref() != Some(args_hash) {
                continue;
            }
            history.push(metric);
            if history.len() >= 5 {
                break;
            }
        }
    }

    if history.is_empty() {
        return AdaptiveParams {
            head: default_head,
            tail: default_tail,
            tail_error: default_tail_error,
            modified: false,
            reason: None,
            event: None,
        };
    }

    // 2. Count consecutive recent failures starting from the most recent run (history[0]).
    //
    // Step 1 — noisy-skip: a failing run that produced zero output (classic
    // grep "no match", find "not found", `[ -f missing ]`) is semantically
    // empty. Its failure mode isn't the kind extra error context would help
    // with — the tail to expand wouldn't have any new bytes anyway. So such
    // entries do NOT contribute to the streak: they break it, just like a
    // success would. Conservative on purpose: when the recent history is
    // dominated by no-match-style "failures", the expand rule simply doesn't
    // fire, and the `noisy` counter in --stats drops to ~zero.
    let mut consecutive_failures = 0;
    for metric in &history {
        if metric.exit_code != 0 && metric.lines_raw > 0 {
            consecutive_failures += 1;
        } else {
            break;
        }
    }

    // 3. If there are consecutive failures, apply Adaptive Tail Expansion (Anti-Loop).
    if consecutive_failures > 0 {
        let factor = 1 + consecutive_failures;
        let mut tuned_tail_error = default_tail_error * factor;
        if tuned_tail_error > auto_ceiling {
            tuned_tail_error = auto_ceiling;
        }
        let modified = tuned_tail_error != default_tail_error;
        let reason = format!(
            "{} consecutive failures detected, expanding tail_error to {}",
            consecutive_failures, tuned_tail_error
        );
        return AdaptiveParams {
            head: default_head,
            tail: default_tail,
            tail_error: tuned_tail_error,
            modified,
            reason: Some(reason),
            // No-op discipline: a ceiling-pinned trigger that changed nothing
            // is not a firing. Mirrors check_decay_steady/check_proactive_shrink,
            // and keeps the --stats Firings counter meaning "params changed".
            event: if modified {
                Some(ADAPTIVE_EVENT_EXPAND_TAIL_ERR)
            } else {
                None
            },
        };
    }

    // 4. If no failures, count consecutive recent successful runs that were truncated.
    let mut consecutive_successes_truncated = 0;
    for metric in &history {
        if metric.exit_code == 0 {
            if metric.truncated {
                consecutive_successes_truncated += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // 5. If we have multiple successful truncated runs in a row, decay head/tail boundaries.
    if consecutive_successes_truncated >= ADAPTIVE_DECAY_MIN_SUCCESSES {
        let (factor_num, factor_den, event_tag) =
            if consecutive_successes_truncated >= ADAPTIVE_DECAY_MAX_SUCCESSES {
                (
                    DECAY_FACTOR_STRONG_NUM,
                    DECAY_FACTOR_DENOM,
                    ADAPTIVE_EVENT_DECAY_STRONG,
                ) // 40% reduction
            } else {
                (
                    DECAY_FACTOR_MODERATE_NUM,
                    DECAY_FACTOR_DENOM,
                    ADAPTIVE_EVENT_DECAY_MODERATE,
                ) // 20% reduction
            };

        let mut tuned_head = (default_head * factor_num) / factor_den;
        let mut tuned_tail = (default_tail * factor_num) / factor_den;

        // Enforce safety floor
        if tuned_head < auto_floor {
            tuned_head = auto_floor;
        }
        if tuned_tail < auto_floor {
            tuned_tail = auto_floor;
        }

        let modified = tuned_head != default_head || tuned_tail != default_tail;
        let reason = if modified {
            Some(format!(
                "{} consecutive successful runs, optimizing head={} tail={}",
                consecutive_successes_truncated, tuned_head, tuned_tail
            ))
        } else {
            None
        };

        return AdaptiveParams {
            head: tuned_head,
            tail: tuned_tail,
            tail_error: default_tail_error,
            modified,
            reason,
            // No-op discipline: a floor-pinned decay that changed nothing is
            // not a firing — without this, a bucket sitting at the floor
            // emits a decay event on EVERY run, inflating --stats forever.
            event: if modified { Some(event_tag) } else { None },
        };
    }

    // 5b. Recovery (un-ratchet) — every other rule moves head/tail down and
    // tail_error up only, compounding across runs with no way back. After a
    // full window of clean (successful, non-truncated) runs:
    //   - head/tail sitting below base are restored, UNLESS the bucket was
    //     seeded by proactive_shrink — a clean streak is exactly the evidence
    //     that justifies that rule (restoring would flip-flop with it). The
    //     gate is an exclusion, not a decay allow-list, because tuned.jsonl
    //     keeps ONE event tag per bucket and every firing overwrites it: a
    //     later expand (or a partial recovery) re-tagging a decay-shrunk
    //     bucket used to mask the head/tail restore forever.
    //   - a tail_error expanded by the failure rule is restored to base: the
    //     failures have stopped.
    let clean_streak = history.len() >= RECOVER_CLEAN_MIN_RUNS
        && history
            .iter()
            .take(RECOVER_CLEAN_MIN_RUNS)
            .all(|m| m.exit_code == 0 && !m.truncated);
    if clean_streak {
        let proactive_seeded = seeded_by == Some(ADAPTIVE_EVENT_PROACTIVE_SHRINK);
        let restore_head_tail =
            !proactive_seeded && (default_head < base.head || default_tail < base.tail);
        let restore_tail_error = default_tail_error > base.tail_error;
        if restore_head_tail || restore_tail_error {
            let (head, tail) = if restore_head_tail {
                (base.head, base.tail)
            } else {
                (default_head, default_tail)
            };
            let tail_error = if restore_tail_error {
                base.tail_error
            } else {
                default_tail_error
            };
            let reason = format!(
                "{} clean runs — restoring head={} tail={} tail_error={}",
                RECOVER_CLEAN_MIN_RUNS, head, tail, tail_error
            );
            return AdaptiveParams {
                head,
                tail,
                tail_error,
                modified: true,
                reason: Some(reason),
                event: Some(ADAPTIVE_EVENT_RECOVER),
            };
        }
    }

    // 6. Step 4 — Steady-state decay. Catches the "consistently truncated"
    // pattern that the 3/5-consecutive rule misses when the streak is
    // broken by an occasional non-truncated success interleaved with
    // truncated ones. Window-adaptive: looks at the last 20 records of
    // the bucket, fires at ≥80% truncated successes.
    if let Some(p) = check_decay_steady(
        content,
        cmd_name,
        args_hash,
        cmd_only_mode,
        default_head,
        default_tail,
        default_tail_error,
        auto_floor,
    ) {
        return p;
    }

    // 7. Step 3 — Proactive shrink. Fires when the bucket has accumulated
    // enough clean (non-truncated, non-failing) runs that the current cap
    // is demonstrably wasteful: every observed run fits comfortably under
    // half the budget. Max-based so we never introduce a NEW truncation:
    // tuned_head ≥ max(lines_raw) + margin.
    if let Some(p) = check_proactive_shrink(
        content,
        cmd_name,
        args_hash,
        cmd_only_mode,
        default_head,
        default_tail,
        default_tail_error,
        auto_floor,
    ) {
        return p;
    }

    // Default
    AdaptiveParams {
        head: default_head,
        tail: default_tail,
        tail_error: default_tail_error,
        modified: false,
        reason: None,
        event: None,
    }
}

/// Step 4 helper: look at the last `STEADY_WINDOW_RUNS` records of the bucket
/// and fire a 30% shrink when ≥80% are truncated successes. Complements the
/// consecutive-streak decay: same intent (shrink because the cap keeps
/// triggering) but tolerant to noise within the window.
///
/// Returns `None` when the bucket is too small, when truncation rate is
/// below threshold, when any failure sits in the window (steady-state must
/// also mean steady SUCCESS — failures change the safety calculus), or
/// when the floor squeezes the saving to zero.
#[allow(clippy::too_many_arguments)]
fn check_decay_steady(
    content: &str,
    cmd_name: &str,
    args_hash: &str,
    cmd_only_mode: bool,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
    auto_floor: usize,
) -> Option<AdaptiveParams> {
    let mut window = Vec::with_capacity(STEADY_WINDOW_RUNS);
    for line in content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let metric: ExecutionMetric = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metric.cmd != cmd_name {
            continue;
        }
        if !cmd_only_mode && metric.args_hash.as_deref() != Some(args_hash) {
            continue;
        }
        window.push(metric);
        if window.len() >= STEADY_WINDOW_RUNS {
            break;
        }
    }

    if window.len() < STEADY_MIN_RUNS {
        return None;
    }
    // Any failure in the window disqualifies — steady-state means a stable
    // success pattern, just one that consistently overruns the cap.
    if window.iter().any(|m| m.exit_code != 0) {
        return None;
    }
    let truncated_count = window.iter().filter(|m| m.truncated).count();
    // truncated_count * DEN < NUM * window.len()  ⇔  ratio < NUM/DEN
    if truncated_count * STEADY_TRUNCATED_DEN < STEADY_TRUNCATED_NUM * window.len() {
        return None;
    }

    let mut tuned_head = (default_head * STEADY_DECAY_NUM) / STEADY_DECAY_DEN;
    let mut tuned_tail = (default_tail * STEADY_DECAY_NUM) / STEADY_DECAY_DEN;
    if tuned_head < auto_floor {
        tuned_head = auto_floor;
    }
    if tuned_tail < auto_floor {
        tuned_tail = auto_floor;
    }
    // No-op guard: when the floor sits at/above the default for either
    // dimension, the "shrink" can absorb to a no-op (or even GROW the
    // budget, if floor > default). Require a strict overall reduction —
    // matching Step 3's discipline so --stats only reports real wins.
    if tuned_head + tuned_tail >= default_head + default_tail {
        return None;
    }

    let reason = format!(
        "{}/{} truncated successes in window → steady shrink head={} tail={}",
        truncated_count,
        window.len(),
        tuned_head,
        tuned_tail
    );
    Some(AdaptiveParams {
        head: tuned_head,
        tail: tuned_tail,
        tail_error: default_tail_error,
        modified: true,
        reason: Some(reason),
        event: Some(ADAPTIVE_EVENT_DECAY_STEADY),
    })
}

/// Step 3 helper: scan the bucket for a stable clean pattern and propose a
/// tighter (head, tail) if the data backs it. Returns `None` when the trigger
/// doesn't fit — caller falls through to the default `AdaptiveParams`.
#[allow(clippy::too_many_arguments)]
fn check_proactive_shrink(
    content: &str,
    cmd_name: &str,
    args_hash: &str,
    cmd_only_mode: bool,
    default_head: usize,
    default_tail: usize,
    default_tail_error: usize,
    auto_floor: usize,
) -> Option<AdaptiveParams> {
    // Collect up to PROACTIVE_MAX_SCAN bucket records, most-recent first.
    let mut bucket = Vec::with_capacity(PROACTIVE_MAX_SCAN);
    for line in content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let metric: ExecutionMetric = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metric.cmd != cmd_name {
            continue;
        }
        if !cmd_only_mode && metric.args_hash.as_deref() != Some(args_hash) {
            continue;
        }
        bucket.push(metric);
        if bucket.len() >= PROACTIVE_MAX_SCAN {
            break;
        }
    }

    if bucket.len() < PROACTIVE_MIN_RUNS {
        return None;
    }
    // All observations must be clean: success AND not truncated. A single
    // truncated record is signal that the cap is sometimes load-bearing —
    // proactive shrink would risk introducing new truncations. Bail.
    if !bucket.iter().all(|m| m.exit_code == 0 && !m.truncated) {
        return None;
    }
    let max_lines = bucket.iter().map(|m| m.lines_raw).max().unwrap_or(0);
    let budget = default_head + default_tail;
    // Only shrink when the savings would be meaningful: max observed fits
    // in HALF the current budget (with margin). Otherwise the small win
    // isn't worth the params churn in stats.
    if max_lines + PROACTIVE_MARGIN_LINES > budget / 2 {
        return None;
    }

    // Tuned head bounded below by max-observed + margin (no new truncations)
    // and by auto_floor (operator's safety floor). Tail kept small but
    // non-zero so errors still have a tiny window. Both floored.
    let mut tuned_head = max_lines + PROACTIVE_MARGIN_LINES;
    if tuned_head < auto_floor {
        tuned_head = auto_floor;
    }
    let mut tuned_tail = default_tail / 4;
    if tuned_tail < auto_floor {
        tuned_tail = auto_floor;
    }
    // After flooring it might happen that tuned_head + tuned_tail meets or
    // exceeds the original budget — in which case there is no actual saving
    // to claim. Treat as a no-op so we don't pollute --stats with a firing
    // that didn't actually shrink anything.
    if tuned_head + tuned_tail >= budget {
        return None;
    }

    let reason = format!(
        "{} clean runs, max={} lines → proactive shrink head={} tail={}",
        bucket.len(),
        max_lines,
        tuned_head,
        tuned_tail
    );
    Some(AdaptiveParams {
        head: tuned_head,
        tail: tuned_tail,
        tail_error: default_tail_error,
        modified: true,
        reason: Some(reason),
        event: Some(ADAPTIVE_EVENT_PROACTIVE_SHRINK),
    })
}
