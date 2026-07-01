//! Safety guard: best-effort detection of obviously destructive commands, plus
//! the LLM-environment auto-detection that turns the guard on by default.
//!
//! This is a guard rail, not a sandbox. It pattern-matches argv and shell `-c`
//! payloads and can be bypassed by a determined caller (`--no-guard` /
//! `L0_CACHE_GUARD=0`).
//!
//! ## Recursive-removal coverage
//!
//! A recursive force-remove (`rm -r -f` and friends) is blocked when it targets
//! a *protected* path. Protected paths are:
//!
//! * **System roots** — `/`, `/etc`, `/usr`, `/var`, `/root`, … (see
//!   [`CRITICAL_ROOTS`]).
//! * **The user's HOME**, resolved at runtime from the environment (`$HOME`,
//!   `%USERPROFILE%`, `HOMEDRIVE`+`HOMEPATH`) — both the home directory itself
//!   and its first-level data folders (Documents, Desktop, Downloads, …; see
//!   [`HOME_DATA_SUBDIRS`]). This is derived from the environment, not merely
//!   hardcoded, so it follows the account the process actually runs as.
//! * **Literal, un-expanded home references** (`~`, `$HOME`, `${HOME}`,
//!   `%USERPROFILE%`) and their `…/*` globs, in case they reach us verbatim
//!   inside a `bash -c` payload (where no shell expanded them first).
//!
//! Coverage is cross-OS: Linux (`/home/<user>`, XDG dirs), macOS
//! (`/Users/<user>`), and Windows (`C:\Users\<user>`, backslash separators,
//! drive letters, and the `/c/Users` form used by POSIX-like shells such as Git
//! Bash) all normalize to a common comparison form.
//!
//! ## Residual limits (still best-effort, NOT a sandbox)
//!
//! The `-c` payload re-scan applies a *minimal* quote-removal de-obfuscation
//! (`r"m" -"r"f` → `rm -rf`) so the trivial quote-insertion bypass no longer
//! works, but it does NOT perform variable expansion, command substitution,
//! `$()`/backtick evaluation, path canonicalization, or symlink resolution.
//! `..` is deliberately left unresolved (we err toward blocking on the literal
//! root instead). A determined caller can still evade it; that is what
//! `--no-guard` / `L0_CACHE_GUARD=0` are for.

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

/// First-level data folders directly under `$HOME` that hold user files and must
/// not be recursively removed. Covers the macOS/Windows defaults and the Linux
/// XDG user-dirs set. Compared case-insensitively (targets are lowercased before
/// matching), which also covers the case-insensitive macOS/Windows filesystems.
const HOME_DATA_SUBDIRS: &[&str] = &[
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "movies",
    "music",
    "videos",
    "public",
    "templates",
];

/// Literal, un-expanded home references. A shell would normally expand these
/// before we ever see them, but inside a `bash -c "…"` payload they can reach
/// the re-scan verbatim, so we treat them as protected targets in their own
/// right (lowercased to match the lowercased argv the scanner compares).
const LITERAL_HOME_TOKENS: &[&str] = &["~", "$home", "${home}", "%userprofile%"];

/// Home directories resolved from the environment. Derived at runtime (not only
/// hardcoded) so the guard follows the account the process actually runs as,
/// across shells and OSes: `$HOME` (Linux/macOS/POSIX shells), `%USERPROFILE%`
/// (Windows), and `HOMEDRIVE`+`HOMEPATH` (Windows fallback).
fn resolved_home_dirs() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.trim().is_empty() && !v.contains(&s) {
            v.push(s);
        }
    };
    if let Ok(h) = std::env::var("HOME") {
        push(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        push(h);
    }
    if let (Ok(d), Ok(p)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
        if !d.is_empty() && !p.is_empty() {
            push(format!("{d}{p}"));
        }
    }
    v
}

/// Cross-OS spellings of a single home path. A Windows home like `C:\Users\foo`
/// is reachable both as the drive form (`C:/Users/foo`) and, from a POSIX-like
/// shell (Git Bash / MSYS), as `/c/Users/foo`; we protect both.
fn home_path_variants(home: &str) -> Vec<String> {
    let fwd = home.replace('\\', "/");
    let mut v = vec![fwd.clone()];
    let bytes = fwd.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        // `C:/Users/foo` -> `/c/Users/foo`
        v.push(format!("/{}{}", drive, &fwd[2..]));
    }
    v
}

/// Build the full set of protected recursive-removal targets (normalized +
/// lowercased): system roots, literal home tokens (and their data-subdir
/// children), plus every runtime-resolved home directory, its cross-OS
/// spellings, and their data-subdir children. `homes` is injected so tests can
/// simulate each OS's home layout without mutating the shared environment.
fn protected_targets(homes: &[String]) -> Vec<String> {
    let mut set: Vec<String> = CRITICAL_ROOTS.iter().map(|s| s.to_string()).collect();

    let mut add_with_subdirs = |base: String| {
        for sub in HOME_DATA_SUBDIRS {
            set.push(format!("{base}/{sub}"));
        }
        set.push(base);
    };

    for &lit in LITERAL_HOME_TOKENS {
        add_with_subdirs(lit.to_string());
    }

    for home in homes {
        for variant in home_path_variants(home) {
            let base = normalize_guard_path(&variant).to_lowercase();
            // Never let a degenerate/empty home collapse the guard down to "/".
            if base.len() > 1 && !base.is_empty() {
                add_with_subdirs(base);
            }
        }
    }

    set
}

/// Normalize a path-like argument so guard comparisons survive cosmetic variation:
/// strip surrounding quotes, fold `\` separators to `/` (Windows), collapse
/// repeated slashes, resolve trailing `/.`, and drop trailing slashes (keeping the
/// root `/`). Does NOT resolve `..` (we err toward blocking, and a `..` that climbs
/// to a root is caught by the literal root).
pub(crate) fn normalize_guard_path(arg: &str) -> String {
    let trimmed = arg.trim().trim_matches(|c| c == '\'' || c == '"');
    let s = trimmed.replace('\\', "/");
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

/// Normalize an argument to its comparison base: strip a trailing `/*` glob
/// (`/etc/*` → `/etc`, `/*` → `/`) so a glob targeting a protected dir is caught
/// like the bare dir.
fn target_base(arg: &str) -> String {
    let n = normalize_guard_path(arg);
    match n.strip_suffix("/*") {
        Some("") => "/".to_string(),
        Some(rest) => rest.to_string(),
        None => n,
    }
}

/// True if a (possibly glob/normalized) argument targets a critical *system*
/// root, e.g. `/etc`, `/etc/`, `/etc//`, `/etc/.`, `/etc/*`, `/`, `/*`.
pub(crate) fn is_critical_target(arg: &str) -> bool {
    CRITICAL_ROOTS.contains(&target_base(arg).as_str())
}

/// True if a (possibly glob/normalized) argument targets any protected path in
/// `protected` (system roots ∪ home dirs ∪ home data folders ∪ literal home
/// tokens). Comparison is case-insensitive to match the lowercased `protected`
/// set and the case-insensitive macOS/Windows filesystems.
fn is_protected_target(arg: &str, protected: &[String]) -> bool {
    let base = target_base(arg).to_lowercase();
    protected.iter().any(|p| p == &base)
}

/// Run the dangerous-pattern rules against a single command segment
/// (`cmd_name` = the segment's binary basename, `tokens` = its whitespace argv,
/// `protected` = the precomputed set of protected recursive-removal targets).
fn scan_segment(cmd_name: &str, tokens: &[String], protected: &[String]) -> Result<(), String> {
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
            if let Some(target) = full_args
                .iter()
                .find(|arg| is_protected_target(arg, protected))
            {
                // Keep the system-level wording for system roots; call out home
                // /user-data separately so the message is actionable.
                let scope = if is_critical_target(target) {
                    "critical system path"
                } else {
                    "home / user-data path"
                };
                return Err(format!(
                    "Destructive recursive removal detected: 'rm' recursively targeted {scope} '{target}'"
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

/// Minimal, quote-aware tokenizer used ONLY to de-obfuscate a `-c` payload
/// before re-scanning. It splits the payload into command segments on UNQUOTED
/// `;`, `&`, `|`, and newlines, and within each segment into tokens on unquoted
/// whitespace — while removing `'…'`/`"…"` quotes and honoring a single
/// backslash escape, so adjacent fragments recompose into one token
/// (`r"m"` → `rm`, `-"r"f` → `-rf`). This defeats the trivial quote-insertion
/// bypass that plain `split_whitespace` fell for.
///
/// It is deliberately NOT a shell parser: no variable expansion, no command
/// substitution, no globbing. When in doubt it errs toward emitting MORE tokens
/// to match on, never fewer. (`$HOME` is left literal on purpose — the literal
/// home tokens are themselves protected.)
fn split_segments_dequoted(payload: &str) -> Vec<Vec<String>> {
    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut seg: Vec<String> = Vec::new();
    let mut tok = String::new();
    let mut in_tok = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for c in payload.chars() {
        if escaped {
            tok.push(c);
            in_tok = true;
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                tok.push(c);
                in_tok = true;
            }
            continue;
        }
        match c {
            '\\' => {
                // Escape the next char; the token stays "active" so a bare
                // `\` still contributes to the current token.
                escaped = true;
                in_tok = true;
            }
            '\'' | '"' => {
                // Quotes are removed but still open a token (so `""` → empty tok).
                quote = Some(c);
                in_tok = true;
            }
            ';' | '&' | '|' | '\n' => {
                if in_tok {
                    seg.push(std::mem::take(&mut tok));
                    in_tok = false;
                }
                if !seg.is_empty() {
                    segments.push(std::mem::take(&mut seg));
                }
            }
            c if c.is_whitespace() => {
                if in_tok {
                    seg.push(std::mem::take(&mut tok));
                    in_tok = false;
                }
            }
            _ => {
                tok.push(c);
                in_tok = true;
            }
        }
    }
    if in_tok {
        seg.push(tok);
    }
    if !seg.is_empty() {
        segments.push(seg);
    }
    segments
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
/// (command substitution, variable expansion) and can be bypassed by a determined
/// caller. Bypass intentionally with `--no-guard` / `L0_CACHE_GUARD=0`.
pub fn check_dangerous_command(cmd_name: &str, command: &[String]) -> Result<(), String> {
    check_dangerous_command_with_homes(cmd_name, command, &resolved_home_dirs())
}

/// Core of [`check_dangerous_command`] with the set of home directories injected
/// rather than read from the environment, so tests can simulate each OS's home
/// layout (`/home/<user>`, `/Users/<user>`, `C:\Users\<user>`) deterministically
/// without mutating the shared, race-prone process environment.
pub(crate) fn check_dangerous_command_with_homes(
    cmd_name: &str,
    command: &[String],
    homes: &[String],
) -> Result<(), String> {
    let protected = protected_targets(homes);

    // Literal argv scan (preserves prior behavior).
    scan_segment(cmd_name, command, &protected)?;

    // Shell-wrapper scan: re-scan each quote-normalized segment of the `-c`
    // payload, so `bash -c 'r"m" -"r"f /etc'` and `bash -c 'rm -rf $HOME'` are
    // caught despite quote-insertion / literal-token obfuscation.
    if let Some(payload) = shell_c_payload(command) {
        for toks in split_segments_dequoted(payload) {
            if let Some(first) = toks.first() {
                let seg_cmd = first.rsplit('/').next().unwrap_or(first).to_string();
                scan_segment(&seg_cmd, &toks, &protected)?;
            }
        }
    }

    Ok(())
}
