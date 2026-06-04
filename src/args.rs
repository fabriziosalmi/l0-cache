//! CLI argument parsing.
//!
//! Separates proxy flags (`--stats`, `--raw`, `--head N`, `--tail N`, `-i`)
//! from the wrapped command and its arguments.
//! Everything after the first non-flag token is the child command.

use clap::Parser;
use clap_complete::Shell;

/// Lightweight CLI proxy that reduces LLM token consumption.
#[derive(Parser, Debug)]
#[command(
    name = "l0-cache",
    version,
    long_version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("L0_CACHE_GIT_HASH"), ")"),
    about = "CLI proxy: filters & truncates command output to save LLM tokens",
    // Allow unknown args so we can pass them to the child command
    trailing_var_arg = true,
)]
pub struct Args {
    /// Show aggregated token savings statistics, then exit.
    #[arg(long)]
    pub stats: bool,

    /// Filter stats to entries within this time window (e.g. "7d", "24h").
    #[arg(long, requires = "stats")]
    pub since: Option<String>,

    /// Reset (delete) all telemetry statistics.
    #[arg(long)]
    pub reset_stats: bool,

    /// Run command but print full output without truncation (still logs metrics).
    #[arg(long)]
    pub raw: bool,

    /// Force interactive/passthrough mode (no capture, no metrics).
    #[arg(short = 'i', long = "interactive")]
    pub interactive: bool,

    /// Number of head lines to keep.
    #[arg(long, default_value_t = crate::filter::DEFAULT_HEAD)]
    pub head: usize,

    /// Number of tail lines to keep (120 on error exit).
    #[arg(long, default_value_t = crate::filter::DEFAULT_TAIL)]
    pub tail: usize,

    /// Number of tail lines on error exit.
    #[arg(long, default_value_t = crate::filter::DEFAULT_TAIL_ERROR)]
    pub tail_error: usize,

    /// Line count threshold below which output is never truncated.
    #[arg(long, default_value_t = crate::filter::DEFAULT_THRESHOLD)]
    pub threshold: usize,

    /// Enable adaptive auto-tuning (now enabled by default).
    #[arg(long)]
    pub auto: bool,

    /// Disable adaptive auto-tuning of parameters.
    #[arg(long)]
    pub no_auto: bool,

    /// Aggressively filter out lines that don't look like errors (e.g. ERROR, WARN, FAIL).
    #[arg(long)]
    pub only_errors: bool,

    /// Kill the command if it produces no output for this many seconds (prevents interactive deadlocks).
    #[arg(long, default_value_t = 0)]
    pub idle_timeout: u64,

    /// Suppress proxy's own stderr warnings (e.g., auto-tuning notifications).
    #[arg(short, long)]
    pub quiet: bool,

    /// Enable safety guard to block dangerous commands (defaults to auto-detect).
    #[arg(long)]
    pub guard: bool,

    /// Disable safety guard to block dangerous commands.
    #[arg(long)]
    pub no_guard: bool,

    /// Diagnose system installation, shell environment, and active LLM editors.
    #[arg(long)]
    pub doctor: bool,

    /// Floor for success optimization decay under --auto.
    #[arg(long, default_value_t = 10)]
    pub auto_floor: usize,

    /// Ceiling for failure backoff tail expansion under --auto.
    #[arg(long, default_value_t = 1000)]
    pub auto_ceiling: usize,

    /// Divisor for token estimation (default: 4).
    #[arg(long, default_value_t = 4)]
    pub token_factor: usize,

    /// Generate shell completions and print to stdout (bash, zsh, fish, elvish, powershell).
    #[arg(long, value_name = "SHELL")]
    pub completions: Option<Shell>,

    /// The command and arguments to execute.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

/// Commands that are inherently interactive and should passthrough
/// when stdin is a TTY.
const INTERACTIVE_ALLOWLIST: &[&str] = &[
    "vim", "vi", "nvim", "nano", "emacs", "less", "more", "man", "htop", "top", "ssh", "fzf",
];

impl Args {
    /// Returns true if the command should run in passthrough mode.
    pub fn should_passthrough(&self) -> bool {
        if self.interactive {
            return true;
        }

        // Only passthrough if stdin is a TTY (i.e., human at keyboard, not Claude Code)
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return false;
        }

        // Check if command binary is in the interactive allowlist
        if let Some(cmd) = self.command.first() {
            let binary = cmd.rsplit('/').next().unwrap_or(cmd);
            INTERACTIVE_ALLOWLIST.contains(&binary)
        } else {
            false
        }
    }

    /// Returns the command binary name (for metrics).
    pub fn cmd_name(&self) -> String {
        if let Some(real) = self.extract_real_cmd() {
            return real;
        }
        self.command
            .first()
            .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
            .unwrap_or_else(|| "(none)".to_string())
    }

    /// Try to extract the real command from shell wrappers like sh -c "cmd"
    fn extract_real_cmd(&self) -> Option<String> {
        let raw_bin = self.command.first()?;
        let bin = raw_bin.rsplit('/').next().unwrap_or(raw_bin);
        let shells = ["sh", "bash", "zsh", "ksh", "csh", "tcsh", "fish", "dash"];
        if shells.contains(&bin) {
            // Find the position of "-c"
            if let Some(c_idx) = self.command.iter().position(|a| a == "-c") {
                if let Some(cmd_str) = self.command.get(c_idx + 1) {
                    let trimmed = cmd_str.trim_start();
                    if !trimmed.is_empty() {
                        // Find the first word before spaces or operators
                        let first_word = trimmed
                            .split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|')
                            .next()?;
                        let parsed = first_word.trim_matches(|c| c == '\'' || c == '"');
                        let real_bin = parsed.rsplit('/').next().unwrap_or(parsed);
                        if !real_bin.is_empty() {
                            return Some(real_bin.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns the command arguments as a single string (for metrics).
    pub fn cmd_args_string(&self) -> String {
        if self.command.len() > 1 {
            self.command[1..].join(" ")
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_command() {
        let args = Args::parse_from(["t", "cargo", "test", "--all"]);
        assert_eq!(args.command, vec!["cargo", "test", "--all"]);
        assert!(!args.stats);
        assert!(!args.raw);
        assert!(!args.auto);
    }

    #[test]
    fn parse_stats_flag() {
        let args = Args::parse_from(["t", "--stats"]);
        assert!(args.stats);
        assert!(args.command.is_empty());
    }

    #[test]
    fn parse_raw_with_command() {
        let args = Args::parse_from(["t", "--raw", "git", "log"]);
        assert!(args.raw);
        assert_eq!(args.command, vec!["git", "log"]);
    }

    #[test]
    fn parse_custom_head_tail() {
        let args = Args::parse_from(["t", "--head", "50", "--tail", "50", "ls", "-la"]);
        assert_eq!(args.head, 50);
        assert_eq!(args.tail, 50);
        assert_eq!(args.command, vec!["ls", "-la"]);
    }

    #[test]
    fn cmd_name_extracts_binary() {
        let args = Args::parse_from(["t", "/usr/bin/git", "status"]);
        assert_eq!(args.cmd_name(), "git");
    }

    #[test]
    fn cmd_args_string_joins() {
        let args = Args::parse_from(["t", "cargo", "test", "--all", "--release"]);
        assert_eq!(args.cmd_args_string(), "test --all --release");
    }

    #[test]
    fn parse_interactive_flag() {
        let args = Args::parse_from(["t", "-i", "vim", "file.txt"]);
        assert!(args.interactive);
        assert_eq!(args.command, vec!["vim", "file.txt"]);
    }

    #[test]
    fn parse_interactive_long() {
        let args = Args::parse_from(["t", "--interactive", "htop"]);
        assert!(args.interactive);
        assert_eq!(args.command, vec!["htop"]);
    }

    #[test]
    fn parse_no_command() {
        let args = Args::parse_from(["t"]);
        assert!(args.command.is_empty());
        assert!(!args.stats);
        assert!(!args.raw);
        assert!(!args.interactive);
        assert!(!args.auto);
    }

    #[test]
    fn parse_stats_with_since() {
        let args = Args::parse_from(["t", "--stats", "--since", "7d"]);
        assert!(args.stats);
        assert_eq!(args.since, Some("7d".to_string()));
    }

    #[test]
    fn parse_threshold_override() {
        let args = Args::parse_from(["t", "--threshold", "200", "ls"]);
        assert_eq!(args.threshold, 200);
        assert_eq!(args.command, vec!["ls"]);
    }

    #[test]
    fn parse_tail_error_override() {
        let args = Args::parse_from(["t", "--tail-error", "200", "cargo", "build"]);
        assert_eq!(args.tail_error, 200);
        assert_eq!(args.command, vec!["cargo", "build"]);
    }

    #[test]
    fn parse_defaults() {
        let args = Args::parse_from(["t", "echo"]);
        assert_eq!(args.head, 30);
        assert_eq!(args.tail, 30);
        assert_eq!(args.tail_error, 120);
        assert_eq!(args.threshold, 100);
        assert!(!args.raw);
        assert!(!args.stats);
        assert!(!args.interactive);
        assert!(!args.auto);
        assert_eq!(args.command, vec!["echo"]);
    }

    #[test]
    fn parse_all_flags_combined() {
        let args = Args::parse_from([
            "t",
            "--raw",
            "--head",
            "10",
            "--tail",
            "20",
            "--threshold",
            "50",
            "cargo",
            "test",
        ]);
        assert!(args.raw);
        assert_eq!(args.head, 10);
        assert_eq!(args.tail, 20);
        assert_eq!(args.threshold, 50);
        assert_eq!(args.command, vec!["cargo", "test"]);
    }

    #[test]
    fn cmd_name_no_command() {
        let args = Args::parse_from(["t"]);
        assert_eq!(args.cmd_name(), "(none)");
    }

    #[test]
    fn cmd_name_simple() {
        let args = Args::parse_from(["t", "ls"]);
        assert_eq!(args.cmd_name(), "ls");
    }

    #[test]
    fn cmd_args_string_no_args() {
        let args = Args::parse_from(["t", "ls"]);
        assert_eq!(args.cmd_args_string(), "");
    }

    #[test]
    fn cmd_args_string_single_arg() {
        let args = Args::parse_from(["t", "git", "status"]);
        assert_eq!(args.cmd_args_string(), "status");
    }

    #[test]
    fn should_passthrough_interactive_flag() {
        // When -i is set, should_passthrough returns true regardless of TTY state
        let args = Args::parse_from(["t", "-i", "echo", "hello"]);
        assert!(args.should_passthrough());
    }

    #[test]
    fn should_passthrough_no_command() {
        // Empty command, not interactive → false
        let args = Args::parse_from(["t"]);
        assert!(!args.should_passthrough());
    }

    #[test]
    fn parse_command_with_dashes() {
        let args = Args::parse_from(["t", "cargo", "test", "--", "--nocapture"]);
        assert_eq!(args.command, vec!["cargo", "test", "--", "--nocapture"]);
    }

    #[test]
    fn parse_raw_flag_alone_no_cmd() {
        let args = Args::parse_from(["t", "--raw"]);
        assert!(args.raw);
        assert!(args.command.is_empty());
    }

    #[test]
    fn parse_auto_flag() {
        let args = Args::parse_from(["t", "--auto", "cargo", "test"]);
        assert!(args.auto);
        assert_eq!(args.command, vec!["cargo", "test"]);
    }

    #[test]
    fn cmd_name_extracts_from_shell_wrapper() {
        let args = Args::parse_from(["t", "sh", "-c", "cargo test --all"]);
        assert_eq!(args.cmd_name(), "cargo");

        let args2 = Args::parse_from(["t", "/bin/bash", "-c", "  git diff && echo 1"]);
        assert_eq!(args2.cmd_name(), "git");
    }

    #[test]
    fn parse_custom_auto_limits_and_token_factor() {
        let args = Args::parse_from([
            "t",
            "--auto",
            "--auto-floor",
            "5",
            "--auto-ceiling",
            "500",
            "--token-factor",
            "8",
            "cargo",
            "test",
        ]);
        assert!(args.auto);
        assert_eq!(args.auto_floor, 5);
        assert_eq!(args.auto_ceiling, 500);
        assert_eq!(args.token_factor, 8);
        assert_eq!(args.command, vec!["cargo", "test"]);
    }

    #[test]
    fn parse_no_auto_and_doctor() {
        let args = Args::parse_from(["t", "--no-auto", "--doctor"]);
        assert!(args.no_auto);
        assert!(args.doctor);
        assert!(args.command.is_empty());
    }
}
