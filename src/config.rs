//! Optional per-command configuration.
//!
//! A small file lets users tune l0-compressor per command without recompiling and
//! without per-tool parsers — staying "universal-first". It overrides the built-in
//! defaults; an explicit CLI flag always wins over the config.
//!
//! **Transparent multi-format** (zero extra dependencies): the config dir
//! (`$XDG_CONFIG_HOME/l0-compressor/`, else `$HOME/.config/l0-compressor/`) is searched for
//! `config.{json,toml,yaml,yml,conf,ini}`. JSON is parsed strictly via serde; the
//! rest share a forgiving flat parser (the schema is flat, so TOML/YAML/INI styles
//! reduce to the same shape). Absent/unreadable/malformed config is non-fatal.
//!
//! ```toml
//! # config.toml  (or config.json / config.yaml — same schema)
//! [defaults]
//! recover = true
//!
//! [cargo]
//! tail_error = 300
//! head = 50
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
    /// Clean-success squelch (default on). `squelch = false` disables it.
    pub squelch: Option<bool>,
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
            squelch: other.squelch.or(self.squelch),
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
    /// A malformed JSON file prints a single stderr note unless `quiet`; the flat
    /// formats never hard-fail (bad lines are skipped).
    pub fn load(quiet: bool) -> Config {
        migrate_legacy_config_dir();
        let path = match find_config() {
            Some(p) => p,
            None => return Config::default(),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Config::default(), // absent/unreadable → no config
        };
        if path.extension().is_some_and(|e| e == "json") {
            match serde_json::from_str::<Config>(&text) {
                Ok(c) => c,
                Err(e) => {
                    if !quiet {
                        eprintln!(
                            "l0-compressor: ignoring malformed config at {} ({})",
                            path.display(),
                            e
                        );
                    }
                    Config::default()
                }
            }
        } else {
            parse_flat(&text)
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

/// Config file names searched in priority order (transparent multi-format).
const CONFIG_NAMES: &[&str] = &[
    "config.json",
    "config.toml",
    "config.yaml",
    "config.yml",
    "config.conf",
    "config.ini",
];

/// `$XDG_CONFIG_HOME/l0-compressor/` then `$HOME/.config/l0-compressor/`.
fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("l0-compressor"));
        }
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".config").join("l0-compressor"))
}

/// One-time, best-effort migration of the pre-rename config directory
/// (`…/l0-cache/`) to `…/l0-compressor/`, so a user's `config.*` survives the
/// rebrand. Renames ONLY when the new dir does not exist and the legacy one
/// does; never deletes, and any I/O error is ignored.
fn migrate_legacy_config_dir() {
    if let Some(new_dir) = config_dir() {
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

/// First existing config file among [`CONFIG_NAMES`].
fn find_config() -> Option<PathBuf> {
    let dir = config_dir()?;
    CONFIG_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
}

/// Parse a flat TOML/YAML/INI-style config (zero-dependency). The schema is flat:
/// a section header names a command (or `defaults` / `*`), and `key = value` /
/// `key: value` lines set its tunables — so all three styles reduce to the same
/// shape. `#` starts a comment; unknown keys and unparseable lines are skipped,
/// mirroring the lenient JSON behavior.
fn parse_flat(text: &str) -> Config {
    let mut cfg = Config::default();
    let mut section = String::from("defaults"); // keys before any header → defaults
    for raw in text.lines() {
        let line = match raw.find('#') {
            Some(i) => &raw[..i],
            None => raw,
        }
        .trim();
        if line.is_empty() {
            continue;
        }
        // TOML/INI section header: `[name]`.
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim().to_string();
            continue;
        }
        // key/value split on the first `=` or `:`.
        let Some(i) = line.find(['=', ':']) else {
            continue; // bare token, not a header or kv → ignore
        };
        let key = line[..i].trim();
        let val = line[i + 1..].trim().trim_matches(['"', '\'']);
        if val.is_empty() {
            // `name:` with nothing after → YAML-style section header.
            if line.ends_with(':') && !key.is_empty() {
                section = key.to_string();
            }
            continue;
        }
        apply_kv(&mut cfg, &section, key, val);
    }
    cfg
}

/// Assign one `key = value` into the right [`Overrides`] block.
fn apply_kv(cfg: &mut Config, section: &str, key: &str, val: &str) {
    let ov = if section.eq_ignore_ascii_case("defaults") || section == "*" {
        &mut cfg.defaults
    } else {
        cfg.commands.entry(section.to_string()).or_default()
    };
    let as_num = val.parse::<usize>().ok();
    let as_bool = parse_bool_value(val);
    match key {
        "head" => ov.head = as_num.or(ov.head),
        "tail" => ov.tail = as_num.or(ov.tail),
        "tail_error" => ov.tail_error = as_num.or(ov.tail_error),
        "threshold" => ov.threshold = as_num.or(ov.threshold),
        "only_errors" => ov.only_errors = as_bool.or(ov.only_errors),
        "recover" => ov.recover = as_bool.or(ov.recover),
        "squelch" => ov.squelch = as_bool.or(ov.squelch),
        _ => {} // unknown key ignored (forward-compatible)
    }
}

/// Lenient boolean: accepts TOML `true`/`false`, YAML `yes`/`no`/`on`/`off`, `1`/`0`.
fn parse_bool_value(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
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
        // from a newer l0-compressor must not break an older binary.
        let c = parse(r#"{ "commands": { "cargo": { "head": 5, "future_key": 1 } } }"#);
        assert_eq!(c.for_command("cargo").head, Some(5));
    }

    // ── flat (TOML / YAML / INI) parser ─────────────────────────────────

    #[test]
    fn flat_parser_toml_style() {
        let c = parse_flat("[defaults]\nrecover = true\n\n[cargo]\nhead = 50\ntail_error = 300\n");
        let o = c.for_command("cargo");
        assert_eq!(o.head, Some(50));
        assert_eq!(o.tail_error, Some(300));
        assert_eq!(o.recover, Some(true)); // inherited from [defaults]
        assert_eq!(c.for_command("git").recover, Some(true));
    }

    #[test]
    fn flat_parser_yaml_style_with_comments() {
        let c = parse_flat(
            "# my config\ndefaults:\n  recover: yes\ncargo:\n  head: 7   # inline note\n  only_errors: false\n",
        );
        let o = c.for_command("cargo");
        assert_eq!(o.head, Some(7));
        assert_eq!(o.recover, Some(true)); // yes → true, inherited
        assert_eq!(o.only_errors, Some(false));
    }

    #[test]
    fn flat_parser_skips_garbage_and_unknown() {
        let c = parse_flat(
            "[cargo]\nhead = 9\nfuture_key = 1\nthis is garbage\n= no key\nhead = notanumber\n",
        );
        // Valid `head=9` kept; `future_key` unknown, bare/garbage lines, an empty
        // key, and `head=notanumber` are all skipped without clobbering head.
        assert_eq!(c.for_command("cargo").head, Some(9));
        assert_eq!(c.for_command("cargo").tail, None);
    }

    #[test]
    fn flat_parser_star_section_is_defaults() {
        let c = parse_flat("[*]\nthreshold = 200\n");
        assert_eq!(c.for_command("anything").threshold, Some(200));
    }
}
