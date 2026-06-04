//! Build script: embed git commit hash into the binary for `--version`.
//!
//! Produces: `l0-cache 0.1.0 (abc1234)` instead of `l0-cache 0.1.0`.
//! Falls back gracefully if git is not available (containers, tarballs).

use std::process::Command;

fn main() {
    // Tell Cargo to re-run this if the git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");

    let git_hash = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let git_dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    let suffix = if git_dirty { "-dirty" } else { "" };

    println!("cargo:rustc-env=L0_CACHE_GIT_HASH={}{}", git_hash, suffix);
}
