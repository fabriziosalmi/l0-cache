//! Data-directory and sidecar path resolution.

use std::path::PathBuf;

/// Get the data directory: `~/.local/share/l0-cache/`
///
/// Resolution order:
/// 1. `$XDG_DATA_HOME/l0-cache/`
/// 2. `$HOME/.local/share/l0-cache/`
/// 3. `/etc/passwd` lookup for home dir (fallback for containers/cron/systemd)
pub(crate) fn data_dir() -> Option<PathBuf> {
    // 1. XDG_DATA_HOME (highest priority)
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("l0-cache"));
        }
    }

    // 2. $HOME/.local/share
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("l0-cache"),
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
                    .join("l0-cache"),
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

/// Get the metrics file path: `~/.local/share/l0-cache/metrics.jsonl`
pub(crate) fn metrics_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("metrics.jsonl"))
}

/// Path to the per-bucket persistence sidecar — written each time an adaptive
/// rule fires + reads to seed the next run of the same bucket. Step 5.
pub(crate) fn tuned_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tuned.jsonl"))
}
