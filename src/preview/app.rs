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
use crate::git::{ChangeKind, Repo, Scope};
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

/// A batched review note, anchored to a file (and optionally a line range).
#[derive(Debug, Clone)]
pub struct Note {
    pub id: u64,
    pub file: PathBuf,
    /// New-file line range; 0-0 = whole file (list-side note).
    pub start: u32,
    pub end: u32,
    pub text: String,
    /// The selected diff lines (`-`/`+`/space prefixed), possibly empty.
    pub snippet: String,
    /// Whether this note was written against the staged (cached) diff, so
    /// re-showing it later (notes view) picks the same side.
    pub cached: bool,
}

/// A popup the run loop should open on our behalf.
pub enum PopupReq {
    Annotate,
    PickAgent,
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
    /// The built diff (kept for click-to-unfold rebuilds).
    built: Option<super::render::DiffDoc>,
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

    // ---- review notes / selection ----
    /// Cursor line in the rendered doc (drives selection).
    pub cursor_line: usize,
    /// Selection anchor (`v`); selection = anchor..=cursor.
    pub select_anchor: Option<usize>,
    /// Batched review notes across files.
    pub notes: Vec<Note>,
    /// Set when `a` was pressed: what the annotate popup will describe.
    pub pending_note: Option<Note>,
    /// Popup for the run loop to open.
    pub popup_request: Option<PopupReq>,
    /// `n` pressed here: ask the list pane to open the notes view.
    pub notes_view_request: bool,
    /// Enter pressed here: ask the list pane to open the editor on the
    /// current selection (same flow as Enter in the list).
    pub edit_request: bool,
    /// Pristine copies of the lines currently tinted, so a cursor move can
    /// restore them instead of re-cloning the whole (up to 20k-line) doc.
    saved_tint: Vec<(usize, ratatui::text::Line<'static>)>,
    /// The file the current doc was built for (cursor-preservation check).
    shown_file: Option<PathBuf>,
    /// Rendered indices of injected note-card lines (excluded from ranges).
    card_lines: Vec<usize>,
    /// Note id to scroll to once its file's diff arrives (notes view hover).
    pending_focus: Option<u64>,
    /// Bumped on every note mutation; the run loop broadcasts on change.
    pub notes_rev: u64,
    /// Monotonic id source for notes.
    next_note_id: u64,
    /// Transient footer flash message.
    pub flash: Option<(String, std::time::Instant)>,

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
            built: None,
            doc: Text::default(),
            first_change: None,
            truncated: 0,
            scroll: 0,
            viewport_h: 0,
            base: None,
            cursor_line: 0,
            select_anchor: None,
            notes: Vec::new(),
            pending_note: None,
            popup_request: None,
            notes_view_request: false,
            edit_request: false,
            saved_tint: Vec::new(),
            shown_file: None,
            card_lines: Vec::new(),
            pending_focus: None,
            notes_rev: 0,
            next_note_id: 1,
            flash: None,
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

    /// The list has nothing selected any more — drop the shown diff.
    pub fn clear(&mut self) {
        self.current = None;
        self.built = None;
        self.shown_file = None;
        self.saved_tint.clear();
        self.doc = Text::default();
        self.first_change = None;
        self.truncated = 0;
        self.scroll = 0;
        self.state = State::Splash("no file selected");
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
            self.built = None;
            self.saved_tint.clear();
            self.doc = Text::default();
            self.state = State::Binary;
            self.clamp_scroll();
            return;
        }
        if built.is_empty {
            self.built = None;
            self.saved_tint.clear();
            self.doc = Text::default();
            self.truncated = 0;
            self.state = State::Empty;
            self.clamp_scroll();
            return;
        }
        // A same-file refresh (auto-poll, stage toggle) keeps the cursor and
        // any live selection; only a *different* file resets them.
        let same_file = self
            .current
            .as_ref()
            .map(|req| self.shown_file.as_ref() == Some(&req.file))
            .unwrap_or(false);
        self.shown_file = self.current.as_ref().map(|req| req.file.clone());
        self.built = Some(built);
        if !same_file {
            self.cursor_line = 0;
            self.select_anchor = None;
        }
        self.state = State::Diff;
        self.clamp_scroll();
        self.rebuild();
        if let Some(id) = self.pending_focus.take() {
            self.scroll_to_note(id);
        }
    }

    /// Copy the built text into `doc`, applying the render cap and injecting
    /// the current file's note cards under their anchor lines.
    fn sync_doc(&mut self) {
        let Some(built) = &self.built else {
            return;
        };
        let mut doc = built.text.clone();

        // Note cards: `▎ 12-20 · note text`, inserted bottom-up so earlier
        // insertions don't shift later anchors.
        self.card_lines.clear();
        if let Some(req) = &self.current {
            let card_style = Style::new().fg(Color::Yellow);
            let mut anchored: Vec<(usize, String)> = self
                .notes
                .iter()
                .filter(|n| n.file == req.file)
                .map(|n| {
                    let line = if n.end == 0 {
                        0
                    } else {
                        built.line_for_new(n.end).map(|l| l + 1).unwrap_or(0)
                    };
                    let label = if n.end == 0 {
                        format!("          ▎ note · {}", n.text)
                    } else {
                        format!("          ▎ {}-{} · {}", n.start, n.end, n.text)
                    };
                    (line, label)
                })
                .collect();
            anchored.sort_by_key(|(line, _)| std::cmp::Reverse(*line));
            for (line, label) in anchored {
                let at = line.min(doc.lines.len());
                doc.lines
                    .insert(at, Line::from(Span::styled(label, card_style)));
            }
            // Recompute card indices top-down for range math.
            let mut cards: Vec<(usize, String)> = self
                .notes
                .iter()
                .filter(|n| n.file == req.file)
                .map(|n| {
                    let line = if n.end == 0 {
                        0
                    } else {
                        built.line_for_new(n.end).map(|l| l + 1).unwrap_or(0)
                    };
                    (line, String::new())
                })
                .collect();
            cards.sort_by_key(|(l, _)| *l);
            for (shift, (line, _)) in cards.into_iter().enumerate() {
                self.card_lines.push(line + shift);
            }
        }

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
    }

    // ---- mouse ------------------------------------------------------------

    /// Wheel scrolls; a left click moves the cursor (and expands folds);
    /// dragging extends a selection. `y` is the terminal row (body starts at
    /// 1, under the header).
    pub fn on_mouse(&mut self, kind: crossterm::event::MouseEventKind, y: u16) {
        use crossterm::event::{MouseButton, MouseEventKind};
        match kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) if y >= 1 => {
                let line = self.scroll as usize + (y - 1) as usize;
                // Cards count as their neighbors; clicks on them do nothing.
                let card_free = !self.card_lines.contains(&line);
                let to_built = self.doc_to_built(line);
                if let (Some(bl), Some(built)) = (to_built, self.built.as_mut())
                    && built.unfold_at(bl)
                {
                    self.clamp_scroll();
                    self.rebuild();
                    return;
                }
                if card_free && line < self.content_lines() {
                    self.select_anchor = None;
                    self.cursor_line = line;
                    self.restyle();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if y >= 1 => {
                let line = (self.scroll as usize + (y - 1) as usize)
                    .min(self.content_lines().saturating_sub(1));
                if self.select_anchor.is_none() {
                    self.select_anchor = Some(self.cursor_line);
                }
                self.cursor_line = line;
                self.restyle();
            }
            _ => {}
        }
    }

    /// Map a doc line index (with cards injected) back to the built index.
    fn doc_to_built(&self, line: usize) -> Option<usize> {
        if self.card_lines.contains(&line) {
            return None;
        }
        let cards_before = self.card_lines.iter().filter(|c| **c < line).count();
        Some(line - cards_before)
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
            // nvim-style: j/k always move the cursor; the view follows.
            Action::Down | Action::ScrollDown => self.move_cursor(1),
            Action::Up | Action::ScrollUp => self.move_cursor(-1),
            Action::HalfPageDown => self.move_cursor(self.viewport_h.max(2) as i32 / 2),
            Action::HalfPageUp => self.move_cursor(-(self.viewport_h.max(2) as i32) / 2),
            Action::Top | Action::DiffTop => self.move_cursor(i32::MIN),
            Action::Bottom | Action::DiffBottom => self.move_cursor(i32::MAX),
            Action::Select => {
                if matches!(self.state, State::Diff) {
                    self.select_anchor = match self.select_anchor {
                        Some(_) => None,
                        None => {
                            self.flash("visual: j/k extend · a note · esc cancel");
                            Some(self.cursor_line)
                        }
                    };
                    self.restyle();
                }
            }
            Action::Annotate => self.begin_annotate(),
            Action::NotesView => self.notes_view_request = true,
            // Enter: open the editor, same as Enter over in the list. The
            // list owns that flow (busy lockout, tab-nvim reuse), so just
            // ask it.
            Action::Edit => self.edit_request = true,
            Action::SendNotes => {
                if self.notes.is_empty() {
                    self.flash("no notes yet — select lines and press a");
                } else {
                    self.popup_request = Some(PopupReq::PickAgent);
                }
            }
            Action::Quit => {
                // Esc/q first backs out of an active selection.
                if self.select_anchor.is_some() {
                    self.select_anchor = None;
                    self.restyle();
                } else {
                    self.should_quit = true;
                    self.close_view = true;
                }
            }
            _ => {}
        }
    }

    // ---- cursor / selection ----------------------------------------------

    /// Move the cursor line (i32::MIN/MAX = home/end), keep it visible, and
    /// re-apply the selection styling. Only meaningful while selecting (the
    /// cursor is invisible otherwise).
    fn move_cursor(&mut self, delta: i32) {
        let last = self.content_lines().saturating_sub(1);
        self.cursor_line = match delta {
            i32::MIN => 0,
            i32::MAX => last,
            d => (self.cursor_line as i64 + i64::from(d)).clamp(0, last as i64) as usize,
        };
        // Keep the cursor inside the viewport.
        let vh = self.viewport_h.max(1) as usize;
        if self.cursor_line < self.scroll as usize {
            self.scroll = self.cursor_line as u16;
        } else if self.cursor_line >= self.scroll as usize + vh {
            self.scroll = (self.cursor_line + 1 - vh) as u16;
        }
        self.restyle();
    }

    /// The selected rendered-line range (anchor..=cursor), or the cursor line.
    pub fn selection(&self) -> (usize, usize) {
        match self.select_anchor {
            Some(a) => (a.min(self.cursor_line), a.max(self.cursor_line)),
            None => (self.cursor_line, self.cursor_line),
        }
    }

    /// Full re-render from the built diff (cards re-injected), then tint.
    /// Use after anything that changes the doc's *content*; plain cursor or
    /// selection moves go through `restyle` alone.
    fn rebuild(&mut self) {
        self.sync_doc();
        self.saved_tint.clear();
        self.restyle();
    }

    /// Tint the cursor line / selection with subtle background colors,
    /// restoring the previously tinted lines first (text colors are never
    /// touched; the tint overrides the red/green line tints while selected,
    /// like an editor would).
    fn restyle(&mut self) {
        // Restore whatever was tinted before.
        for (idx, line) in self.saved_tint.drain(..) {
            if let Some(slot) = self.doc.lines.get_mut(idx) {
                *slot = line;
            }
        }
        if !matches!(self.state, State::Diff) {
            return;
        }
        let cursor_bg = if !self.cfg.theme.is_light() {
            Color::Rgb(0x2a, 0x2b, 0x38)
        } else {
            Color::Rgb(0xee, 0xf0, 0xf5)
        };
        let select_bg = if !self.cfg.theme.is_light() {
            Color::Rgb(0x45, 0x47, 0x5a)
        } else {
            Color::Rgb(0xd8, 0xdd, 0xe6)
        };
        let last = self.doc.lines.len().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(last);
        let tint = |idx: usize,
                    bg: Color,
                    saved: &mut Vec<(usize, ratatui::text::Line<'static>)>,
                    lines: &mut Vec<ratatui::text::Line<'static>>| {
            if let Some(line) = lines.get_mut(idx) {
                saved.push((idx, line.clone()));
                line.style = line.style.bg(bg);
                for span in &mut line.spans {
                    span.style = span.style.bg(bg);
                }
            }
        };
        let mut saved = std::mem::take(&mut self.saved_tint);
        match self.select_anchor {
            Some(_) => {
                let (a, b) = self.selection();
                for idx in a..=b {
                    tint(idx, select_bg, &mut saved, &mut self.doc.lines);
                }
            }
            None => tint(self.cursor_line, cursor_bg, &mut saved, &mut self.doc.lines),
        }
        self.saved_tint = saved;
    }

    // ---- notes ------------------------------------------------------------

    /// `a`: capture the selection as a pending note and ask the run loop to
    /// open the annotate popup. With no active selection, the note covers the
    /// whole file.
    fn begin_annotate(&mut self) {
        let Some(req) = self.current.clone() else {
            return;
        };
        if req.commit.is_some() || req.scope != Scope::Worktree {
            self.flash("notes work on staged/unstaged changes only");
            return;
        }

        let Some(built) = &self.built else {
            return;
        };
        let (a, b) = self.selection();
        // Map doc lines back to built lines (skip injected card lines).
        let to_built = |line: usize| -> Option<usize> {
            if self.card_lines.contains(&line) {
                return None;
            }
            let cards_before = self.card_lines.iter().filter(|c| **c < line).count();
            Some(line - cards_before)
        };
        let mut numbers = Vec::new();
        let mut snippet = String::new();
        for line in a..=b {
            let Some(bl) = to_built(line) else { continue };
            if let Some((old, new)) = built.numbers_of_line(bl) {
                numbers.push(new.or(old).unwrap_or(0));
            }
            if let Some(text) = built.marker_text_of_line(bl)
                && !text.is_empty()
                && snippet.lines().count() < 40
            {
                snippet.push_str(&text);
                snippet.push('\n');
            }
        }
        let (start, end) = match (numbers.iter().min(), numbers.iter().max()) {
            (Some(&s), Some(&e)) if s > 0 => (s, e),
            _ => (0, 0),
        };
        self.pending_note = Some(Note {
            id: 0, // allocated when committed
            file: req.file,
            start,
            end,
            text: String::new(),
            snippet,
            cached: req.cached,
        });
        self.popup_request = Some(PopupReq::Annotate);
    }

    /// The annotate popup returned text: commit the pending note.
    pub fn finish_annotate(&mut self, text: String) {
        if let Some(mut note) = self.pending_note.take() {
            note.text = text;
            note.id = self.next_note_id;
            self.next_note_id += 1;
            self.notes.push(note);
            self.notes_rev += 1;
            self.select_anchor = None;
            self.rebuild();
        }
    }

    /// A whole-file note from the list pane.
    pub fn add_file_note(&mut self, file: PathBuf, text: String, cached: bool) {
        self.notes.push(Note {
            id: self.next_note_id,
            file,
            start: 0,
            end: 0,
            text,
            snippet: String::new(),
            cached,
        });
        self.next_note_id += 1;
        self.notes_rev += 1;
        self.rebuild();
    }

    pub fn clear_notes(&mut self) {
        self.notes.clear();
        self.notes_rev += 1;
        self.rebuild();
    }

    pub fn edit_note(&mut self, id: u64, text: String) {
        if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
            note.text = text;
            self.notes_rev += 1;
            self.rebuild();
        }
    }

    pub fn delete_note(&mut self, id: u64) {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.notes.len() != before {
            self.notes_rev += 1;
            self.rebuild();
        }
    }

    /// The notes view hovered note `idx`: scroll its card into view when its
    /// file is already shown, else remember it until that diff arrives.
    pub fn focus_note(&mut self, id: u64) {
        let note = self.notes.iter().find(|n| n.id == id);
        let same_file = match (note, &self.current) {
            (Some(note), Some(req)) => note.file == req.file,
            _ => false,
        };
        if same_file && matches!(self.state, State::Diff) {
            self.scroll_to_note(id);
        } else {
            self.pending_focus = Some(id);
        }
    }

    fn scroll_to_note(&mut self, id: u64) {
        let Some(req) = &self.current else { return };
        let Some(idx) = self.notes.iter().position(|n| n.id == id) else {
            return;
        };
        // Rank of this note's card among the current file's cards (they were
        // injected sorted by anchor line).
        let Some(built) = &self.built else { return };
        let anchor = |n: &Note| {
            if n.end == 0 {
                0
            } else {
                built.line_for_new(n.end).map(|l| l + 1).unwrap_or(0)
            }
        };
        let mut same: Vec<(usize, usize)> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.file == req.file)
            .map(|(i, n)| (anchor(n), i))
            .collect();
        same.sort_by_key(|(a, _)| *a);
        if let Some(rank) = same.iter().position(|(_, i)| *i == idx)
            && let Some(&line) = self.card_lines.get(rank)
        {
            self.scroll = (line.saturating_sub(3)) as u16;
            self.clamp_scroll();
        }
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), std::time::Instant::now()));
    }

    /// The flash message if still fresh (3 s TTL).
    pub fn active_flash(&self) -> Option<&str> {
        match &self.flash {
            Some((msg, at)) if at.elapsed() < std::time::Duration::from_secs(3) => {
                Some(msg.as_str())
            }
            _ => None,
        }
    }
}
