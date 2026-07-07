//! Data-directory and sidecar path resolution.

use std::path::PathBuf;

/// Get the data directory: `~/.local/share/l0-compressor/`
///
/// Resolution order:
/// 1. `$XDG_DATA_HOME/l0-compressor/`
/// 2. `$HOME/.local/share/l0-compressor/`
/// 3. `/etc/passwd` lookup for home dir (fallback for containers/cron/systemd)
pub(crate) fn data_dir() -> Option<PathBuf> {
    // 1. XDG_DATA_HOME (highest priority)
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("l0-compressor"));
        }
    }

    // 2. $HOME/.local/share
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("l0-compressor"),
            );
        }
    }

    // 3. Fallback: /etc/passwd lookup (for LXC, cron, systemd without User=)
    #[cfg(unix)]
    {
        if let Some(home) = home_from_passwd() {
            return Some(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("l0-compressor"),
            );
        }
    }

    None
}

/// Look up the current user's home directory from /etc/passwd.
/// This works in containers and cron jobs where $HOME is not set.
#[cfg(unix)]
fn home_from_passwd() -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        // /etc/passwd format: username:x:uid:gid:gecos:homedir:shell
        if fields.len() >= 6 {
            if let Ok(entry_uid) = fields[2].parse::<u32>() {
                if entry_uid == uid {
                    let home = fields[5];
                    if !home.is_empty() {
                        return Some(home.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Get the metrics file path: `~/.local/share/l0-compressor/metrics.jsonl`
pub(crate) fn metrics_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("metrics.jsonl"))
}

/// Path to the per-bucket persistence sidecar — written each time an adaptive
/// rule fires + reads to seed the next run of the same bucket. Step 5.
pub(crate) fn tuned_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tuned.jsonl"))
}

/// One-time, best-effort migration of the pre-rename data directory
/// (`…/l0-cache/`) to the current `…/l0-compressor/` location, so a user's
/// accumulated `metrics.jsonl`/`tuned.jsonl` survive the rebrand. Renames ONLY
/// when the new directory does not yet exist and the legacy one does; never
/// deletes anything, and any I/O error is ignored (the caller then degrades to a
/// fresh directory). Safe to call unconditionally on startup.
pub(crate) fn migrate_legacy_data_dir() {
    if let Some(new_dir) = data_dir() {
        if new_dir.exists() {
            return;
        }
        if let Some(legacy) = new_dir.parent().map(|p| p.join("l0-cache")) {
            if legacy.is_dir() {
                let _ = std::fs::rename(&legacy, &new_dir);
            }
        }
    }
}
