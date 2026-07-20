use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::logx::log;

/// User config from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`. Every key is
/// optional; a missing file or a parse error never prevents startup — we log
/// and fall back to defaults (per shared contracts).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Branch-scope base; "" = auto-detect via `Repo::detect_base`.
    pub base: String,
    /// Which side the list pane sits on: "right" | "left".
    pub list_side: ListSide,
    /// Editor argv; file (and `+<line>`) appended when launching.
    pub editor: Vec<String>,
    /// Auto-refresh interval in ms; 0 disables polling.
    pub poll_ms: u64,
    pub show_untracked: bool,
    /// Diff color flavor — picks the syntax theme and all UI tints.
    pub theme: Theme,
    /// Which scope the view opens in: "worktree" (default) or "branch".
    pub default_scope: ScopePref,
    /// Unchanged context lines kept around each change before folding (git's
    /// diff.context equivalent). Clamped to 0..=20.
    pub context_lines: usize,
    /// action name -> key string, overrides `keymap::DEFAULT_KEYS`.
    pub keybindings: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListSide {
    Right,
    Left,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn is_light(self) -> bool {
        self == Theme::Light
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopePref {
    #[default]
    Worktree,
    Branch,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: String::new(),
            list_side: ListSide::Right,
            editor: vec!["nvim".into()],
            poll_ms: 2000,
            show_untracked: true,
            theme: Theme::Dark,
            default_scope: ScopePref::Worktree,
            context_lines: 3,
            keybindings: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let Some(path) = config_path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => Config::parse(&raw),
            Err(_) => Config::default(), // missing file is the normal case
        }
    }

    /// Parse TOML; on error log + fall back to defaults, then clamp.
    fn parse(raw: &str) -> Config {
        let mut cfg = match toml::from_str::<Config>(raw) {
            Ok(cfg) => cfg,
            Err(err) => {
                log(format!("config parse error, using defaults: {err}"));
                eprintln!("gitview: config parse error, using defaults: {err}");
                Config::default()
            }
        };
        cfg.context_lines = cfg.context_lines.min(20);
        // A tiny non-zero poll interval would hammer git; keep it sane.
        if cfg.poll_ms > 0 {
            cfg.poll_ms = cfg.poll_ms.max(250);
        }
        cfg
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(|dir| PathBuf::from(dir).join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let cfg = Config::parse("");
        assert_eq!(cfg.list_side, ListSide::Right);
        assert_eq!(cfg.editor, vec!["nvim".to_string()]);
        assert_eq!(cfg.poll_ms, 2000);
        assert!(cfg.show_untracked);
        assert!(cfg.base.is_empty());
        assert!(cfg.keybindings.is_empty());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg = Config::parse(
            r#"
            list_side = "left"

            [keybindings]
            stage = "a"
            "#,
        );
        assert_eq!(cfg.list_side, ListSide::Left);
        assert_eq!(cfg.editor, vec!["nvim".to_string()]); // untouched default
        assert_eq!(cfg.keybindings.get("stage").map(String::as_str), Some("a"));
    }

    #[test]
    fn bad_toml_falls_back_to_defaults() {
        let cfg = Config::parse("list_side = [not toml");
        assert_eq!(cfg.list_side, ListSide::Right);
    }

    #[test]
    fn poll_ms_has_a_sane_floor_but_zero_disables() {
        assert_eq!(Config::parse("poll_ms = 5").poll_ms, 250);
        assert_eq!(Config::parse("poll_ms = 0").poll_ms, 0); // disabled
        assert_eq!(Config::parse("poll_ms = 1000").poll_ms, 1000);
    }

    #[test]
    fn context_lines_clamped_and_scope_parses() {
        assert_eq!(Config::parse("context_lines = 99").context_lines, 20);
        assert_eq!(
            Config::parse(r#"default_scope = "branch""#).default_scope,
            ScopePref::Branch
        );
    }
}
