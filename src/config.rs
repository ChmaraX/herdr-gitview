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
    /// Number of columns a tab expands to in the diff pane. Tabs are expanded
    /// to spaces at render time because ratatui does not interpret `\t`.
    /// Clamped to 1..=16.
    pub tab_width: usize,
    /// action name -> key string, overrides `keymap::DEFAULT_KEYS`.
    pub keybindings: HashMap<String, String>,
    /// How much of the tab the whole gitview sidebar takes (percent of the
    /// tab width). The existing layout is squeezed into the rest, so the
    /// default is an even split with whatever you already had open. Clamped
    /// to 20..=80.
    pub view_width_percent: u8,
    /// How much of the gitview area the file list takes (percent). Clamped
    /// to 10..=80.
    pub list_width_percent: u8,
    /// Sidebar mode: when another pane of the tab already runs nvim (plain
    /// or the herdr-nvim sidebar), Enter opens the file there instead of
    /// starting an editor on the gitview diff pane.
    pub reuse_tab_nvim: bool,
    /// Whether the file set `editor` (the default is `nvim`); the app tries
    /// `$VISUAL`/`$EDITOR` before the default, never before a chosen one.
    #[serde(skip)]
    pub editor_set: bool,
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
            tab_width: 4,
            keybindings: HashMap::new(),
            view_width_percent: 50,
            list_width_percent: 29,
            reuse_tab_nvim: true,
            editor_set: false,
        }
    }
}

impl Config {
    pub fn load() -> Config {
        Config::load_from(config_path().as_deref())
    }

    /// The config in `path` (`None` or a missing file: the defaults).
    pub fn load_from(path: Option<&std::path::Path>) -> Config {
        let Some(path) = path else {
            return Config::default();
        };
        match std::fs::read_to_string(path) {
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
        cfg.editor_set = toml::from_str::<toml::Table>(raw).is_ok_and(|t| t.contains_key("editor"));
        cfg.context_lines = cfg.context_lines.min(20);
        cfg.tab_width = cfg.tab_width.clamp(1, 16);
        cfg.view_width_percent = cfg.view_width_percent.clamp(20, 80);
        cfg.list_width_percent = cfg.list_width_percent.clamp(10, 80);
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
    fn numbers_take_their_defaults_and_are_clamped_into_range() {
        type Field = fn(&Config) -> u64;
        let poll: Field = |c| c.poll_ms;
        let view: Field = |c| c.view_width_percent.into();
        let list: Field = |c| c.list_width_percent.into();
        let tab: Field = |c| c.tab_width as u64;
        let context: Field = |c| c.context_lines as u64;
        for (toml, field, want) in [
            ("poll_ms = 5", poll, 250),
            ("poll_ms = 0", poll, 0), // disabled, not clamped
            ("poll_ms = 1000", poll, 1000),
            ("", view, 50),
            ("view_width_percent = 5", view, 20),
            ("", list, 29),
            ("list_width_percent = 99", list, 80),
            ("", tab, 4),
            ("tab_width = 0", tab, 1),
            ("tab_width = 99", tab, 16),
            ("tab_width = 8", tab, 8),
            ("context_lines = 99", context, 20),
        ] {
            assert_eq!(field(&Config::parse(toml)), want, "{toml:?}");
        }
    }

    #[test]
    fn default_scope_parses() {
        assert_eq!(
            Config::parse(r#"default_scope = "branch""#).default_scope,
            ScopePref::Branch
        );
    }
}
