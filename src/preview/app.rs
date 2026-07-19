//! Preview-pane state: the currently shown diff, scroll position, and the
//! small state machine (splash / diff / empty / error).
//!
//! Like the list's `App`, this owns no channels or terminals. `preview::run`
//! feeds it IPC messages, key events, and diff-worker results, then renders it
//! via `preview::ui`. That keeps the diff parsing/scroll math easy to test.

use std::path::PathBuf;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::config::Config;
use crate::git::{ChangeKind, FileEntry, Repo, Scope, StageState};
use crate::keymap::{Action, Keymap};

/// Hard cap on rendered diff lines; beyond this we show a truncation notice so
/// a 100k-line diff can't stall the render loop.
const MAX_LINES: usize = 20_000;

/// The fields of a `ToPreview::Show`, kept together so we can compare the
/// request that produced a diff against the one currently selected (stale
/// results from the worker are dropped when they differ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowReq {
    pub file: PathBuf,
    pub orig_path: Option<PathBuf>,
    pub scope: Scope,
    pub cached: bool,
    pub kind: ChangeKind,
    /// History view: show this commit's change instead of a live diff.
    pub commit: Option<String>,
}

impl ShowReq {
    /// A minimal `FileEntry` for `diff_ansi` (only path/orig/kind are read).
    pub fn to_entry(&self) -> FileEntry {
        FileEntry {
            path: self.file.clone(),
            orig_path: self.orig_path.clone(),
            kind: self.kind,
            stage: StageState::NA,
            adds: None,
            dels: None,
        }
    }
}

pub enum State {
    /// Nothing to show yet: dim centered message.
    Splash(&'static str),
    /// A diff is loaded in `doc`.
    Diff,
    /// The file has no changes in the requested view.
    Empty,
    /// The file is binary; no diff to render.
    Binary,
    /// The diff build failed; holds the first stderr line.
    Error(String),
}

pub struct PreviewApp {
    pub cfg: Config,
    pub repo: Repo,
    pub keys: Keymap,

    /// Last Show request (what the header/stale-guard describe).
    pub current: Option<ShowReq>,
    /// Styled, capped diff text (plus a truncation notice line when capped).
    pub doc: Text<'static>,
    /// First inserted line's new-file number (editor jump target).
    pub first_change: Option<u32>,
    /// Extra lines hidden by the cap (0 = not truncated).
    pub truncated: usize,

    pub scroll: u16,
    /// Last body height, remembered so Page math is viewport-aware.
    pub viewport_h: u16,

    /// Branch-scope base ref, resolved once for the header.
    pub base: Option<String>,

    pub state: State,
    pub should_quit: bool,
    /// True only when *this* pane initiated the quit (via `q`), so it should
    /// tear down the whole herdr view. A `Quit` message or EOF just exits.
    pub close_view: bool,
}

impl PreviewApp {
    pub fn new(cfg: Config, repo: Repo, keys: Keymap) -> PreviewApp {
        PreviewApp {
            cfg,
            repo,
            keys,
            current: None,
            doc: Text::default(),
            first_change: None,
            truncated: 0,
            scroll: 0,
            viewport_h: 0,
            base: None,
            state: State::Splash("waiting for file list…"),
            should_quit: false,
            close_view: false,
        }
    }

    /// The list connected; if we haven't shown anything yet, switch the splash
    /// text from "waiting" to "no file selected".
    pub fn on_connected(&mut self) {
        if self.current.is_none() {
            self.state = State::Splash("no file selected");
        }
    }

    // ---- Show / diff results ---------------------------------------------

    /// Record a new Show request. Scroll resets on a file change, is preserved
    /// on a same-file refresh (e.g. Tab staged toggle or auto-refresh). The
    /// old `doc` is kept until the worker returns, so there is no flicker.
    pub fn begin_show(&mut self, req: ShowReq) {
        let same_file = self
            .current
            .as_ref()
            .map(|c| c.file == req.file)
            .unwrap_or(false);
        if !same_file {
            self.scroll = 0;
        }
        if req.scope == Scope::Branch && self.base.is_none() {
            self.base = Some(self.repo.detect_base());
        }
        self.current = Some(req);
    }

    /// Apply a worker result, dropping it if it is stale (the selection moved
    /// on while the diff was computing).
    pub fn apply_diff(&mut self, req: &ShowReq, result: Result<super::render::DiffDoc, String>) {
        if self.current.as_ref() != Some(req) {
            return; // stale — a newer Show already superseded this one
        }
        match result {
            Ok(doc) => self.set_diff(doc),
            Err(msg) => self.state = State::Error(msg),
        }
    }

    fn set_diff(&mut self, built: super::render::DiffDoc) {
        self.first_change = built.first_change;
        if built.binary {
            self.doc = Text::default();
            self.state = State::Binary;
            self.clamp_scroll();
            return;
        }
        if built.is_empty {
            self.doc = Text::default();
            self.truncated = 0;
            self.state = State::Empty;
            self.clamp_scroll();
            return;
        }

        let mut doc = built.text;
        let total = doc.lines.len();
        if total > MAX_LINES {
            doc.lines.truncate(MAX_LINES);
            doc.lines.push(Line::from(Span::styled(
                format!("… diff truncated ({} more lines)", total - MAX_LINES),
                Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
            self.truncated = total - MAX_LINES;
        } else {
            self.truncated = 0;
        }
        self.doc = doc;
        self.state = State::Diff;
        self.clamp_scroll();
    }

    // ---- scrolling --------------------------------------------------------

    pub fn set_viewport(&mut self, h: u16) {
        self.viewport_h = h;
        self.clamp_scroll();
    }

    /// Number of content lines currently in the document (incl. any notice).
    fn content_lines(&self) -> usize {
        self.doc.lines.len()
    }

    fn max_scroll(&self) -> u16 {
        self.content_lines()
            .saturating_sub(self.viewport_h as usize)
            .min(u16::MAX as usize) as u16
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.scroll > max {
            self.scroll = max;
        }
    }

    /// Scroll by `delta` lines. `i32::MIN` jumps home, `i32::MAX` to bottom.
    pub fn scroll_by(&mut self, delta: i32) {
        let max = self.max_scroll();
        self.scroll = match delta {
            i32::MIN => 0,
            i32::MAX => max,
            d => {
                let next = i64::from(self.scroll) + i64::from(d);
                next.clamp(0, i64::from(max)) as u16
            }
        };
    }

    /// Page relative to the viewport height (`full` = whole page, else half).
    pub fn page(&mut self, down: bool, full: bool) {
        let vh = self.viewport_h.max(1) as i32;
        let amount = if full { vh } else { (vh / 2).max(1) };
        self.scroll_by(if down { amount } else { -amount });
    }

    // ---- direct keys (preview pane focused) ------------------------------

    pub fn on_key(&mut self, ev: crossterm::event::KeyEvent) {
        let Some(action) = self.keys.action(&ev) else {
            return;
        };
        match action {
            Action::Down | Action::ScrollDown => self.scroll_by(1),
            Action::Up | Action::ScrollUp => self.scroll_by(-1),
            Action::HalfPageDown => self.page(true, false),
            Action::HalfPageUp => self.page(false, false),
            Action::Top | Action::DiffTop => self.scroll_by(i32::MIN),
            Action::Bottom | Action::DiffBottom => self.scroll_by(i32::MAX),
            Action::Quit => {
                self.should_quit = true;
                self.close_view = true;
            }
            _ => {}
        }
    }
}
