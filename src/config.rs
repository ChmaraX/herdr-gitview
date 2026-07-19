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
    /// List pane fraction of total width, clamped to 0.15..=0.6.
    pub split_ratio: f64,
    /// Which side the list pane sits on: "right" | "left".
    pub list_side: ListSide,
    /// Editor argv; file (and `+<line>`) appended when launching.
    pub editor: Vec<String>,
    /// Auto-refresh interval in ms; 0 disables polling.
    pub poll_ms: u64,
    pub show_untracked: bool,
    /// action name -> key string, overrides `keymap::DEFAULT_KEYS`.
    pub keybindings: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListSide {
    Right,
    Left,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: String::new(),
            split_ratio: 0.35,
            list_side: ListSide::Right,
            editor: vec!["nvim".into()],
            poll_ms: 2000,
            show_untracked: true,
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
        cfg.split_ratio = cfg.split_ratio.clamp(0.15, 0.6);
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
        assert_eq!(cfg.split_ratio, 0.35);
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
            split_ratio = 0.5
            list_side = "left"

            [keybindings]
            stage = "a"
            "#,
        );
        assert_eq!(cfg.split_ratio, 0.5);
        assert_eq!(cfg.list_side, ListSide::Left);
        assert_eq!(cfg.editor, vec!["nvim".to_string()]); // untouched default
        assert_eq!(cfg.keybindings.get("stage").map(String::as_str), Some("a"));
    }

    #[test]
    fn bad_toml_falls_back_to_defaults() {
        let cfg = Config::parse("split_ratio = [not toml");
        assert_eq!(cfg.split_ratio, 0.35);
    }

    #[test]
    fn split_ratio_is_clamped() {
        assert_eq!(Config::parse("split_ratio = 0.05").split_ratio, 0.15);
        assert_eq!(Config::parse("split_ratio = 0.9").split_ratio, 0.6);
    }
}
