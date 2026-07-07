//! Persisted per-bucket adaptive tuning state (`tuned.jsonl`).
//!
//! One line per `(cmd, args_hash)` bucket, compacted on write. Read at the
//! start of a run to seed adaptive params; written at the end if a rule fired.

use super::*;

/// One persisted tune per (cmd, args_hash) bucket — read at the start of a
/// run to seed the adaptive params, written at the end if a rule fired.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TunedParams {
    pub ts: String,
    pub cmd: String,
    pub args_hash: String,
    pub head: usize,
    pub tail: usize,
    pub tail_error: usize,
    pub event: String,
}

/// Scan the tuned-params sidecar for the most recent entry matching this
/// bucket. Fail-open: any I/O error or missing file → `None`.
///
/// Implementation is intentionally simple — scan-and-keep-last. `save_tuned`
/// compacts on write (latest entry per bucket), so the file holds one line
/// per active bucket and stays in the low kilobytes even for a heavy user.
/// Keep-last still matters for files written by pre-compaction versions.
pub fn lookup_tuned(cmd: &str, args_hash: &str) -> Option<TunedParams> {
    let path = tuned_path()?;
    lookup_tuned_at_path(&path, cmd, args_hash)
}

/// How long a persisted tune stays authoritative. Matches the metrics file's
/// 30-day housekeeping window: a tune older than every record that could
/// justify it should not keep seeding runs forever (the one-way-ratchet bug).
pub(crate) const TUNED_TTL_SECS: u64 = 30 * 86400;

/// Tolerance for timestamps slightly in the future (clock skew, NTP steps).
/// Anything further ahead is treated as expired — `saturating_sub` alone made
/// a far-future timestamp (e.g. a corrupted "2099-…" entry) fresh forever,
/// the exact immortal-tune failure mode the TTL exists to eliminate.
pub(crate) const TUNED_FUTURE_SKEW_SECS: u64 = 86400;

/// Whether a tuned entry is still within its TTL. An unparseable timestamp
/// (e.g. garbage from an external writer) counts as expired.
pub(crate) fn tuned_entry_fresh(t: &TunedParams, now_secs: u64) -> bool {
    match parse_rfc3339_to_secs(&t.ts) {
        Some(ts) => {
            ts <= now_secs + TUNED_FUTURE_SKEW_SECS && now_secs.saturating_sub(ts) <= TUNED_TTL_SECS
        }
        None => false,
    }
}

pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Path-explicit variant used by tests so they don't have to mutate the
/// shared `XDG_DATA_HOME` env var (which races under parallel test runs).
pub(crate) fn lookup_tuned_at_path(
    path: &std::path::Path,
    cmd: &str,
    args_hash: &str,
) -> Option<TunedParams> {
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    // TTL during the scan (matching compaction's semantics): an expired tune
    // no longer seeds runs, and a stale/garbage-ts LATER line must not hide a
    // fresh earlier entry by winning keep-last and then failing the filter.
    let now = now_unix_secs();
    let mut found: Option<TunedParams> = None;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(t) = serde_json::from_str::<TunedParams>(line) {
            if t.cmd == cmd && t.args_hash == args_hash && tuned_entry_fresh(&t, now) {
                found = Some(t);
            }
        }
    }
    found
}

/// Upsert the bucket's tune; the file is compacted and atomically rewritten
/// on each write (append-only fallback when degraded). Best-effort: any
/// error → a single stderr warning (silenced by `--quiet`), never a panic,
/// never an effect on the wrapped command's exit code.
pub fn save_tuned(t: &TunedParams, quiet: bool) {
    if telemetry_disabled() {
        return;
    }
    let path = match tuned_path() {
        Some(p) => p,
        None => return,
    };
    save_tuned_at_path(&path, t, quiet);
}

/// Path-explicit variant — see `lookup_tuned_at_path` for rationale.
///
/// Compacts on write: the existing file is reduced to its latest entry per
/// (cmd, args_hash) bucket, this entry is upserted, and the whole file is
/// rewritten via temp-file + rename. The file used to be append-only (one
/// line per FIRING, unbounded growth, contradicting its own docs).
pub(crate) fn save_tuned_at_path(path: &std::path::Path, t: &TunedParams, quiet: bool) {
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
    let mut lock = FileLock::for_data_file(path);
    let locked = lock.lock();

    // The compacting rewrite is a read-modify-replace of the WHOLE file, so
    // it is only safe while holding the lock and only correct from a snapshot
    // we could actually read. In every degraded case, fall back to a plain
    // O_APPEND of this one entry: the keep-last reader semantics tolerate
    // multiple lines per bucket, so an append can duplicate but never lose
    // another writer's tune (the rewrite from a bad snapshot could).
    let read_result = fs::read_to_string(path);
    let degraded =
        !locked || matches!(&read_result, Err(e) if e.kind() != std::io::ErrorKind::NotFound);
    if degraded {
        append_tuned_line(path, t, quiet);
        return;
    }

    // Latest entry per bucket, in first-seen order; then upsert ours.
    // Entries past their TTL are pruned here — they no longer seed runs
    // (lookup filters them) so carrying them forward is dead weight.
    let now_secs = now_unix_secs();
    let mut entries: Vec<TunedParams> = Vec::new();
    if let Ok(content) = &read_result {
        for line in content.lines() {
            if let Ok(parsed) = serde_json::from_str::<TunedParams>(line) {
                if !tuned_entry_fresh(&parsed, now_secs) {
                    continue;
                }
                match entries
                    .iter_mut()
                    .find(|e| e.cmd == parsed.cmd && e.args_hash == parsed.args_hash)
                {
                    Some(slot) => *slot = parsed,
                    None => entries.push(parsed),
                }
            }
        }
    }
    match entries
        .iter_mut()
        .find(|e| e.cmd == t.cmd && e.args_hash == t.args_hash)
    {
        Some(slot) => *slot = t.clone(),
        None => entries.push(t.clone()),
    }

    let mut out = String::new();
    for e in &entries {
        if let Ok(json) = serde_json::to_string(e) {
            out.push_str(&json);
            out.push('\n');
        }
    }
    // Unique tmp name: a shared name would let two writers that BOTH lost the
    // lock race publish each other's half-written file via rename.
    let tmp = path.with_extension(format!(
        "jsonl.tmp.{}.{}",
        std::process::id(),
        now_unix_secs()
    ));
    let write_result = fs::write(&tmp, out).and_then(|_| fs::rename(&tmp, path));
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        if !quiet {
            eprintln!(
                "l0-compressor: warning: cannot write {}: {}",
                path.display(),
                e
            );
        }
    }
}

/// Degraded-path writer: append one tuned line without rewriting the file.
fn append_tuned_line(path: &std::path::Path, t: &TunedParams, quiet: bool) {
    let json = match serde_json::to_string(t) {
        Ok(s) => s,
        Err(_) => return,
    };
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", json);
        }
        Err(e) => {
            if !quiet {
                eprintln!(
                    "l0-compressor: warning: cannot write {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
}
