use std::collections::HashMap;

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Down,
    Up,
    Top,
    Bottom,
    Edit, // Enter
    ScrollDown,
    ScrollUp,
    HalfPageDown,
    HalfPageUp,
    DiffTop,
    DiffBottom,
    ToggleScope,  // worktree <-> branch
    ToggleCached, // unstaged <-> staged diff view (worktree scope)
    Stage,
    Discard,
    Commit,
    Refresh,
    Help,
    Quit,
}

/// (action, config key name, default key strings)
pub const DEFAULT_KEYS: &[(Action, &str, &[&str])] = &[
    (Action::Down, "down", &["j", "down"]),
    (Action::Up, "up", &["k", "up"]),
    (Action::Top, "top", &["g"]),
    (Action::Bottom, "bottom", &["shift+g"]),
    (Action::Edit, "edit", &["enter"]),
    (Action::ScrollDown, "scroll_down", &["shift+j"]),
    (Action::ScrollUp, "scroll_up", &["shift+k"]),
    (Action::HalfPageDown, "half_page_down", &["ctrl+d"]),
    (Action::HalfPageUp, "half_page_up", &["ctrl+u"]),
    (Action::DiffTop, "diff_top", &["home"]),
    (Action::DiffBottom, "diff_bottom", &["end"]),
    (Action::ToggleScope, "toggle_scope", &["w"]),
    (Action::ToggleCached, "toggle_cached", &["tab"]),
    (Action::Stage, "stage", &["s"]),
    (Action::Discard, "discard", &["x"]),
    (Action::Commit, "commit", &["c"]),
    (Action::Refresh, "refresh", &["r"]),
    (Action::Help, "help", &["?"]),
    (Action::Quit, "quit", &["q", "esc"]),
];

/// Normalized key: `(code, modifiers)`. For char keys SHIFT is folded into
/// the char itself (`shift+g` is stored and looked up as `Char('G')` with no
/// SHIFT bit) because that is how crossterm reports shifted characters.
type Key = (KeyCode, KeyModifiers);

#[derive(Debug)]
pub struct Keymap {
    map: HashMap<Key, Action>,
    /// action -> first bound key string, for footer hints.
    hints: HashMap<Action, String>,
}

impl Keymap {
    /// Build from defaults + config `[keybindings]` overrides
    /// (`action name -> key string`). An override *replaces* all default keys
    /// for that action. Two actions on one key is an error naming both.
    pub fn build(overrides: &HashMap<String, String>) -> Result<Keymap> {
        let known: Vec<&str> = DEFAULT_KEYS.iter().map(|(_, name, _)| *name).collect();
        for name in overrides.keys() {
            if !known.contains(&name.as_str()) {
                bail!("unknown action in [keybindings]: {name}");
            }
        }

        let mut map: HashMap<Key, Action> = HashMap::new();
        let mut hints: HashMap<Action, String> = HashMap::new();
        let mut owner_name: HashMap<Key, &str> = HashMap::new();

        for (action, name, defaults) in DEFAULT_KEYS {
            let keys: Vec<String> = match overrides.get(*name) {
                Some(over) => vec![over.clone()],
                None => defaults.iter().map(|s| s.to_string()).collect(),
            };
            for (i, spec) in keys.iter().enumerate() {
                let key = parse_key(spec)?;
                if let Some(other) = owner_name.get(&key) {
                    bail!("key '{spec}' bound to both '{other}' and '{name}'");
                }
                owner_name.insert(key, name);
                map.insert(key, *action);
                if i == 0 {
                    hints.insert(*action, spec.clone());
                }
            }
        }
        Ok(Keymap { map, hints })
    }

    pub fn action(&self, ev: &KeyEvent) -> Option<Action> {
        self.map.get(&normalize(ev.code, ev.modifiers)).copied()
    }

    /// First bound key for the footer, e.g. "enter", "shift+g".
    pub fn hint(&self, action: Action) -> String {
        self.hints.get(&action).cloned().unwrap_or_default()
    }
}

/// Parse `[ctrl+][alt+][shift+]<key>` where `<key>` is a single char or a
/// named key (enter, esc, tab, space, up/down/left/right, pgup/pgdn,
/// home/end).
pub fn parse_key(spec: &str) -> Result<Key> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = spec.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(r) = lower.strip_prefix("ctrl+") {
            mods |= KeyModifiers::CONTROL;
            rest = &rest[rest.len() - r.len()..];
        } else if let Some(r) = lower.strip_prefix("alt+") {
            mods |= KeyModifiers::ALT;
            rest = &rest[rest.len() - r.len()..];
        } else if let Some(r) = lower.strip_prefix("shift+") {
            mods |= KeyModifiers::SHIFT;
            rest = &rest[rest.len() - r.len()..];
        } else {
            break;
        }
    }
    if rest.is_empty() {
        bail!("empty key in '{spec}'");
    }

    let code = match rest.to_ascii_lowercase().as_str() {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pgup" => KeyCode::PageUp,
        "pgdn" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        single if single.chars().count() == 1 => {
            // preserve the original char (case matters for e.g. '?')
            KeyCode::Char(rest.chars().next().unwrap())
        }
        other => bail!("unknown key '{other}' in '{spec}'"),
    };

    // Fold SHIFT into char keys: shift+g == G.
    if let KeyCode::Char(c) = code
        && mods.contains(KeyModifiers::SHIFT)
    {
        return Ok((
            KeyCode::Char(c.to_ascii_uppercase()),
            mods - KeyModifiers::SHIFT,
        ));
    }
    Ok((code, mods))
}

/// Normalize an incoming crossterm event the same way `parse_key` stores
/// keys: char keys drop the SHIFT bit (the char already carries the case).
fn normalize(code: KeyCode, mods: KeyModifiers) -> Key {
    match code {
        KeyCode::Char(_) => (code, mods - KeyModifiers::SHIFT),
        _ => (code, mods),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn parse_key_round_trips() {
        assert_eq!(parse_key("j").unwrap(), (KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(parse_key("enter").unwrap(), (KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            parse_key("ctrl+d").unwrap(),
            (KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse_key("alt+shift+left").unwrap(),
            (KeyCode::Left, KeyModifiers::ALT | KeyModifiers::SHIFT)
        );
        assert_eq!(parse_key("pgdn").unwrap(), (KeyCode::PageDown, KeyModifiers::NONE));
    }

    #[test]
    fn shift_char_folds_into_uppercase() {
        // shift+g ≡ G: stored as Char('G') with no SHIFT bit
        assert_eq!(parse_key("shift+g").unwrap(), (KeyCode::Char('G'), KeyModifiers::NONE));
        assert_eq!(parse_key("G").unwrap(), (KeyCode::Char('G'), KeyModifiers::NONE));
    }

    #[test]
    fn parse_key_rejects_garbage() {
        assert!(parse_key("").is_err());
        assert!(parse_key("ctrl+").is_err());
        assert!(parse_key("meta+x").is_err());
        assert!(parse_key("f13").is_err());
    }

    #[test]
    fn default_map_resolves_events() {
        let km = Keymap::build(&HashMap::new()).unwrap();
        assert_eq!(km.action(&ev(KeyCode::Char('j'), KeyModifiers::NONE)), Some(Action::Down));
        // terminal reports shift+g as Char('G') with SHIFT set
        assert_eq!(
            km.action(&ev(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Some(Action::Bottom)
        );
        assert_eq!(
            km.action(&ev(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::HalfPageDown)
        );
        assert_eq!(km.action(&ev(KeyCode::Esc, KeyModifiers::NONE)), Some(Action::Quit));
        assert_eq!(km.action(&ev(KeyCode::Char('z'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn override_replaces_default() {
        let overrides = HashMap::from([("stage".to_string(), "a".to_string())]);
        let km = Keymap::build(&overrides).unwrap();
        assert_eq!(km.action(&ev(KeyCode::Char('a'), KeyModifiers::NONE)), Some(Action::Stage));
        // old default 's' is gone
        assert_eq!(km.action(&ev(KeyCode::Char('s'), KeyModifiers::NONE)), None);
        assert_eq!(km.hint(Action::Stage), "a");
    }

    #[test]
    fn collision_error_names_both_actions() {
        // rebind stage onto 'q', which quit already owns
        let overrides = HashMap::from([("stage".to_string(), "q".to_string())]);
        let err = Keymap::build(&overrides).unwrap_err().to_string();
        assert!(err.contains("quit") && err.contains("stage"), "got: {err}");
    }

    #[test]
    fn unknown_override_action_errors() {
        let overrides = HashMap::from([("staeg".to_string(), "a".to_string())]);
        assert!(Keymap::build(&overrides).is_err());
    }

    #[test]
    fn hint_returns_first_default() {
        let km = Keymap::build(&HashMap::new()).unwrap();
        assert_eq!(km.hint(Action::Down), "j");
        assert_eq!(km.hint(Action::Quit), "q");
        assert_eq!(km.hint(Action::Bottom), "shift+g");
    }
}
