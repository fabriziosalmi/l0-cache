//! Optional per-command configuration.
//!
//! A small JSON file lets users tune l0-cache per command without recompiling and
//! without per-tool parsers — staying "universal-first". It overrides the built-in
//! defaults; an explicit CLI flag always wins over the config.
//!
//! Location: `$XDG_CONFIG_HOME/l0-cache/config.json`, else
//! `$HOME/.config/l0-cache/config.json`. Absent or malformed config is non-fatal
//! (an empty config is used); a malformed file prints one stderr note so the user
//! knows it was ignored.
//!
//! ```json
//! {
//!   "defaults": { "recover": true },
//!   "commands": {
//!     "cargo": { "tail_error": 300, "head": 50 },
//!     "git":   { "head": 10, "tail": 40 },
//!     "docker": { "head": 10, "tail": 80 }
//!   }
//! }
//! ```

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Per-command tunables. Every field is optional; `None` means "leave as-is".
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Overrides {
    pub head: Option<usize>,
    pub tail: Option<usize>,
    pub tail_error: Option<usize>,
    pub threshold: Option<usize>,
    pub only_errors: Option<bool>,
    pub recover: Option<bool>,
}

impl Overrides {
    /// Fields set in `other` win over `self` (used to layer command over defaults).
    fn overlay(&self, other: &Overrides) -> Overrides {
        Overrides {
            head: other.head.or(self.head),
            tail: other.tail.or(self.tail),
            tail_error: other.tail_error.or(self.tail_error),
            threshold: other.threshold.or(self.threshold),
            only_errors: other.only_errors.or(self.only_errors),
            recover: other.recover.or(self.recover),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    defaults: Overrides,
    #[serde(default)]
    commands: HashMap<String, Overrides>,
}

impl Config {
    /// Load the config, or an empty config if it is absent/unreadable/malformed.
    /// A malformed (but present) file prints a single stderr note unless `quiet`.
    pub fn load(quiet: bool) -> Config {
        let path = match config_path() {
            Some(p) => p,
            None => return Config::default(),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Config::default(), // absent/unreadable → no config
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(c) => c,
            Err(e) => {
                if !quiet {
                    eprintln!(
                        "l0-cache: ignoring malformed config at {} ({})",
                        path.display(),
                        e
                    );
                }
                Config::default()
            }
        }
    }

    /// Effective overrides for `cmd`: the `defaults` block with the per-command
    /// block layered on top.
    pub fn for_command(&self, cmd: &str) -> Overrides {
        match self.commands.get(cmd) {
            Some(c) => self.defaults.overlay(c),
            None => self.defaults.clone(),
        }
    }
}

/// `$XDG_CONFIG_HOME/l0-cache/config.json` then `$HOME/.config/l0-cache/config.json`.
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("l0-cache").join("config.json"));
        }
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("l0-cache")
            .join("config.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Config {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn empty_config_yields_no_overrides() {
        let c = Config::default();
        assert_eq!(c.for_command("cargo"), Overrides::default());
    }

    #[test]
    fn per_command_overrides_apply() {
        let c = parse(r#"{ "commands": { "cargo": { "tail_error": 300, "head": 50 } } }"#);
        let o = c.for_command("cargo");
        assert_eq!(o.tail_error, Some(300));
        assert_eq!(o.head, Some(50));
        assert_eq!(o.tail, None);
        // Unlisted command → defaults (empty).
        assert_eq!(c.for_command("git"), Overrides::default());
    }

    #[test]
    fn command_block_layers_over_defaults() {
        let c = parse(
            r#"{ "defaults": { "recover": true, "head": 20 },
                 "commands": { "cargo": { "head": 50 } } }"#,
        );
        let o = c.for_command("cargo");
        assert_eq!(o.recover, Some(true)); // inherited from defaults
        assert_eq!(o.head, Some(50)); // command wins over defaults
        let g = c.for_command("git"); // a command with no block still gets the defaults

        assert_eq!(g.recover, Some(true));
        assert_eq!(g.head, Some(20));
    }

    #[test]
    fn unknown_fields_are_ignored_gracefully() {
        // serde ignores unknown keys by default — a forward-compatible config
        // from a newer l0-cache must not break an older binary.
        let c = parse(r#"{ "commands": { "cargo": { "head": 5, "future_key": 1 } } }"#);
        assert_eq!(c.for_command("cargo").head, Some(5));
    }
}
