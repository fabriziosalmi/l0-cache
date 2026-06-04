//! Safety guard: best-effort detection of obviously destructive commands, plus
//! the LLM-environment auto-detection that turns the guard on by default.
//!
//! This is a guard rail, not a sandbox. It pattern-matches argv and shell `-c`
//! payloads and can be bypassed by a determined caller (`--no-guard` /
//! `L0_CACHE_GUARD=0`).

/// Detects if the current process is running inside an active LLM editor environment.
pub(crate) fn is_llm_environment() -> bool {
    if std::env::var("CLAUDE_CODE").is_ok() || std::env::var("GEMINI_CLI").is_ok() {
        return true;
    }
    if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
        let tp = term_prog.to_lowercase();
        if tp.contains("vscode") || tp.contains("cursor") {
            return true;
        }
    }
    if std::env::var("VSCODE_GIT_IPC_HANDLE").is_ok() || std::env::var("VSCODE_PORT").is_ok() {
        return true;
    }
    false
}

/// Parse a boolean-ish environment value, e.g. for `L0_CACHE_GUARD`.
/// Returns `Some(true/false)` for recognized truthy/falsy values, `None` otherwise.
pub(crate) fn parse_bool_env(val: &str) -> Option<bool> {
    match val.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" | "" => Some(false),
        _ => None,
    }
}

/// Single source of truth for whether the safety guard is active.
///
/// Precedence: `--no-guard` → `--guard` → `L0_CACHE_GUARD` (truthy/falsy) →
/// LLM-environment auto-detect. Both the enforcement path (`main`) and the
/// `--doctor` report call this, so they can never disagree (the previous code
/// treated `L0_CACHE_GUARD=true` as "off" in enforcement but "on" in doctor).
pub fn guard_enabled(force_on: bool, force_off: bool) -> bool {
    if force_off {
        return false;
    }
    if force_on {
        return true;
    }
    if let Ok(val) = std::env::var("L0_CACHE_GUARD") {
        if let Some(b) = parse_bool_env(&val) {
            return b;
        }
    }
    is_llm_environment()
}

/// Critical filesystem roots that must never be the target of a recursive force-remove.
const CRITICAL_ROOTS: &[&str] = &[
    "/", "/etc", "/var", "/usr", "/boot", "/dev", "/sys", "/proc", "/lib", "/lib64", "/bin",
    "/sbin", "/root",
];

/// Normalize a path-like argument so guard comparisons survive cosmetic variation:
/// strip surrounding quotes, collapse repeated slashes, resolve trailing `/.`,
/// and drop trailing slashes (keeping the root `/`). Does NOT resolve `..` (we err
/// toward blocking, and a `..` that climbs to a root is caught by the literal root).
pub(crate) fn normalize_guard_path(arg: &str) -> String {
    let s = arg.trim().trim_matches(|c| c == '\'' || c == '"');
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for ch in s.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push('/');
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    while out.ends_with("/.") {
        out.truncate(out.len() - 2);
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// True if a (possibly glob/normalized) argument targets a protected root, e.g.
/// `/etc`, `/etc/`, `/etc//`, `/etc/.`, `/etc/*`, `/`, `/*`.
pub(crate) fn is_critical_target(arg: &str) -> bool {
    let n = normalize_guard_path(arg);
    let base = match n.strip_suffix("/*") {
        Some("") => "/",
        Some(rest) => rest,
        None => n.as_str(),
    };
    CRITICAL_ROOTS.contains(&base)
}

/// Run the dangerous-pattern rules against a single command segment
/// (`cmd_name` = the segment's binary basename, `tokens` = its whitespace argv).
fn scan_segment(cmd_name: &str, tokens: &[String]) -> Result<(), String> {
    let cmd_lower = cmd_name.to_lowercase();
    let full_args: Vec<String> = tokens.iter().map(|s| s.to_lowercase()).collect();
    let full_command_str = full_args.join(" ");

    // 1. Destructive rm check
    if cmd_lower == "rm" || cmd_lower.ends_with("/rm") {
        let has_recursive = full_args
            .iter()
            .any(|arg| arg.starts_with('-') && (arg.contains('r') || arg.contains('R')));
        let has_force = full_args
            .iter()
            .any(|arg| arg.starts_with('-') && arg.contains('f'))
            || full_args.contains(&"--force".to_string());

        if has_recursive && has_force {
            if let Some(target) = full_args.iter().find(|arg| is_critical_target(arg)) {
                return Err(format!(
                    "Destructive system-level removal detected: 'rm' recursively targeted critical path '{}'",
                    target
                ));
            }
        }
    }

    // 2. Reverse shells & TCP redirections
    if full_command_str.contains("/dev/tcp/") || full_command_str.contains("/dev/udp/") {
        return Err(
            "Unauthorized network socket redirection detected (/dev/tcp or /dev/udp)".to_string(),
        );
    }

    // 3. Exfiltration check (curl/wget/nc combined with sensitive files)
    let is_network_utility = ["curl", "wget", "nc", "netcat", "telnet", "ssh"]
        .iter()
        .any(|&u| cmd_lower == u || cmd_lower.ends_with(&format!("/{}", u)));

    if is_network_utility {
        // Exfiltration targets
        let sensitive_patterns = [
            "id_rsa",
            "id_ed25519",
            ".env",
            "master.key",
            "passwd",
            "shadow",
            "credentials",
        ];

        // Data payload flags
        let has_payload_flag = full_args.iter().any(|arg| {
            arg == "-d"
                || arg.starts_with("--data")
                || arg == "-f"
                || arg.starts_with("--form")
                || arg == "-t"
                || arg == "--upload-file"
                || arg.starts_with("--post-file")
                || arg.starts_with("--post-data")
        });

        // Or input redirection in case of nc
        let has_redirection = full_command_str.contains('<') || full_command_str.contains('|');

        if has_payload_flag || has_redirection || cmd_lower == "ssh" {
            for pattern in &sensitive_patterns {
                if full_command_str.contains(pattern) {
                    return Err(format!(
                        "Potential credentials exfiltration: network utility '{}' invoked with sensitive target '{}'",
                        cmd_name, pattern
                    ));
                }
            }
        }
    }

    // 4. Obvious destructive SQL drops palesi
    if (cmd_lower == "sqlite3"
        || cmd_lower == "mysql"
        || cmd_lower == "psql"
        || cmd_lower == "sqlcmd")
        && full_command_str.contains("drop database")
    {
        return Err(format!(
            "Destructive SQL command blocked: database drop query detected in '{}'",
            cmd_name
        ));
    }

    Ok(())
}

/// If `command` is a shell wrapper (`sh`/`bash`/… `-c <payload>`), return the payload string.
fn shell_c_payload(command: &[String]) -> Option<&str> {
    let bin = command.first()?;
    let base = bin.rsplit('/').next().unwrap_or(bin);
    const SHELLS: &[&str] = &["sh", "bash", "zsh", "ksh", "csh", "tcsh", "fish", "dash"];
    if !SHELLS.contains(&base) {
        return None;
    }
    let idx = command.iter().position(|a| a == "-c")?;
    command.get(idx + 1).map(|s| s.as_str())
}

/// Evaluates a command name and arguments against dangerous pattern rules
/// (system destruction, reverse shell, exfiltration, SQL drops).
///
/// Beyond the literal argv, this also unwraps shell wrappers: for
/// `bash -c "echo hi && rm -rf /etc/"` the `-c` payload is split into command
/// segments and each is re-scanned, so destructive commands hidden inside the
/// dominant LLM-agent invocation pattern (`sh -c "…"`) are not bypassed.
///
/// This is a best-effort lint, not a sandbox: it does not parse the shell grammar
/// (quoting, command substitution, variable expansion) and can be bypassed by a
/// determined caller. Bypass intentionally with `--no-guard` / `L0_CACHE_GUARD=0`.
pub fn check_dangerous_command(cmd_name: &str, command: &[String]) -> Result<(), String> {
    // Literal argv scan (preserves prior behavior).
    scan_segment(cmd_name, command)?;

    // Shell-wrapper scan: re-scan each segment of the `-c` payload.
    if let Some(payload) = shell_c_payload(command) {
        for seg in payload.split([';', '&', '|', '\n']) {
            let toks: Vec<String> = seg.split_whitespace().map(|s| s.to_string()).collect();
            if let Some(first) = toks.first() {
                let seg_cmd = first.rsplit('/').next().unwrap_or(first).to_string();
                scan_segment(&seg_cmd, &toks)?;
            }
        }
    }

    Ok(())
}
