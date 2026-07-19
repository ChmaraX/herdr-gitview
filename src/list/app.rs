//! List-pane application state and the input→action logic.
//!
//! `App` is deliberately thread-agnostic: it owns no channels or terminals.
//! The event loop in `list::run` feeds it key events and refreshed entry
//! vectors, then renders it via `list::ui`. Keeping git/threading out of here
//! makes the state easy to unit- and render-test.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::KeyEvent;

use crate::config::Config;
use crate::git::{FileEntry, Repo, Scope, StageState};
use crate::keymap::{Action, Keymap};

/// How long a transient footer message stays on screen.
const STATUS_TTL: Duration = Duration::from_secs(3);

pub struct App {
    pub repo: Repo,
    pub cfg: Config,
    pub keys: Keymap,

    pub scope: Scope,
    /// unstaged↔staged diff view (worktree scope); wired in phase 3.
    pub cached_view: bool,

    pub entries: Vec<FileEntry>,
    pub cursor: usize,
    pub list_offset: usize,

    /// Branch-scope base ref and its merge-base with HEAD, resolved lazily on
    /// the first scope toggle.
    pub base: String,
    pub merge_base: Option<String>,

    /// Current branch name for the header (None = detached HEAD).
    pub branch: Option<String>,

    /// Transient footer message + when it was set.
    pub status_msg: Option<(String, Instant)>,
    pub should_quit: bool,
    pub show_help: bool,
}

impl App {
    /// Load the initial worktree status and build the app.
    pub fn new(repo: Repo, cfg: Config, keys: Keymap) -> Result<App> {
        let entries = repo.worktree_status(cfg.show_untracked)?;
        Ok(App::from_entries(repo, cfg, keys, entries))
    }

    /// Build an app around an already-loaded entry vector (no git status call).
    /// Used by render tests and by `new`.
    pub fn from_entries(repo: Repo, cfg: Config, keys: Keymap, entries: Vec<FileEntry>) -> App {
        let branch = repo.head_branch();
        App {
            repo,
            cfg,
            keys,
            scope: Scope::Worktree,
            cached_view: false,
            entries,
            cursor: 0,
            list_offset: 0,
            base: String::new(),
            merge_base: None,
            branch,
            status_msg: None,
            should_quit: false,
            show_help: false,
        }
    }

    // ---- event handling ---------------------------------------------------

    pub fn on_key(&mut self, ev: KeyEvent) {
        let Some(action) = self.keys.action(&ev) else {
            return;
        };

        // While the help overlay is up, any recognized key dismisses it.
        if self.show_help {
            self.show_help = false;
            return;
        }

        match action {
            Action::Down => self.move_cursor(1),
            Action::Up => self.move_cursor(-1),
            Action::Top => self.cursor = 0,
            Action::Bottom => {
                if !self.entries.is_empty() {
                    self.cursor = self.entries.len() - 1;
                }
            }
            Action::ToggleScope => self.toggle_scope(),
            Action::ToggleCached => self.toggle_cached(),
            Action::Refresh => self.force_refresh(),
            Action::Help => self.show_help = true,
            Action::Quit => self.should_quit = true,

            // Diff scroll keys are intercepted by the run loop and forwarded
            // to the preview pane over IPC, so they are no-ops here.
            Action::ScrollDown
            | Action::ScrollUp
            | Action::HalfPageDown
            | Action::HalfPageUp
            | Action::DiffTop
            | Action::DiffBottom => {}

            // Not implemented until later phases; announce, don't crash.
            Action::Edit | Action::Stage | Action::Discard | Action::Commit => {
                self.set_status("(soon)")
            }
        }
    }

    /// Toggle the staged-diff view (worktree scope only). The preview re-Shows
    /// with `cached = true`; if the entry has nothing staged, hint as much.
    fn toggle_cached(&mut self) {
        if self.scope != Scope::Worktree {
            return; // staged view is meaningless in branch scope
        }
        self.cached_view = !self.cached_view;
        if self.cached_view
            && self
                .entries
                .get(self.cursor)
                .map(|e| e.stage == StageState::Unstaged)
                .unwrap_or(false)
        {
            self.set_status("no staged changes");
        }
    }

    /// Replace the entry vector (from an auto-refresh or a forced reload),
    /// keeping the cursor on the same path when possible, else clamped.
    pub fn apply_refresh(&mut self, entries: Vec<FileEntry>) {
        let current = self.entries.get(self.cursor).map(|e| e.path.clone());
        self.entries = entries;
        if let Some(path) = current
            && let Some(i) = self.entries.iter().position(|e| e.path == path)
        {
            self.cursor = i;
        }
        self.clamp_cursor();
    }

    /// The footer message if one is set and still fresh.
    pub fn active_status(&self) -> Option<&str> {
        match &self.status_msg {
            Some((msg, at)) if at.elapsed() < STATUS_TTL => Some(msg.as_str()),
            _ => None,
        }
    }

    // ---- internals --------------------------------------------------------

    fn move_cursor(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
    }

    fn clamp_cursor(&mut self) {
        if self.entries.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len() - 1;
        }
    }

    fn toggle_scope(&mut self) {
        match self.scope {
            Scope::Worktree => {
                if self.merge_base.is_none() {
                    let base = if self.cfg.base.is_empty() {
                        self.repo.detect_base()
                    } else {
                        self.cfg.base.clone()
                    };
                    match self.repo.merge_base(&base) {
                        Ok(mb) => {
                            self.base = base;
                            self.merge_base = Some(mb);
                        }
                        Err(err) => {
                            self.set_status(format!("no base found: {err}"));
                            return; // stay in worktree scope
                        }
                    }
                }
                self.scope = Scope::Branch;
            }
            Scope::Branch => self.scope = Scope::Worktree,
        }
        self.force_refresh();
    }

    fn force_refresh(&mut self) {
        match self.load_entries() {
            Ok(entries) => self.apply_refresh(entries),
            Err(err) => self.set_status(format!("refresh failed: {err}")),
        }
    }

    fn load_entries(&self) -> Result<Vec<FileEntry>> {
        match self.scope {
            Scope::Worktree => self.repo.worktree_status(self.cfg.show_untracked),
            Scope::Branch => {
                let mb = self.merge_base.as_deref().unwrap_or("HEAD");
                self.repo.branch_changes(mb)
            }
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), Instant::now()));
    }
}
