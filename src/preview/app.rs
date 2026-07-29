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

/// Floor for the note-card width, so a card still has a shape before the
/// first draw has reported the real pane width.
const MIN_CARD_WIDTH: u16 = 24;

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

/// The inline note composer: a text area spliced into the diff under the
/// line(s) being commented on, so writing a note happens where the note will
/// live instead of in a detached popup.
pub struct Composer {
    pub input: crate::textarea::TextArea,
    /// The note being rewritten, or `None` when this is a new one.
    pub editing: Option<u64>,
}

/// A popup the run loop should open on our behalf.
pub enum PopupReq {
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
    /// Body width of the last draw; note cards are boxed to it.
    pub viewport_w: u16,

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
    /// The open inline composer, if any. While set it owns every keystroke.
    pub composer: Option<Composer>,
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
    /// The first doc line of each note's card, in the same rank order the
    /// cards were injected (used to scroll a note into view).
    card_starts: Vec<usize>,
    /// Where the open composer box sits in the doc: `(first line, height)`.
    composer_span: Option<(usize, usize)>,
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
            viewport_w: 0,
            base: None,
            cursor_line: 0,
            select_anchor: None,
            notes: Vec::new(),
            pending_note: None,
            composer: None,
            popup_request: None,
            notes_view_request: false,
            edit_request: false,
            saved_tint: Vec::new(),
            shown_file: None,
            card_lines: Vec::new(),
            card_starts: Vec::new(),
            composer_span: None,
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

        // Note cards: a boxed block spliced in under the anchor line, so a
        // note reads as a comment on the code rather than another diff row.
        // Each card is 1+ lines, so the index bookkeeping below tracks every
        // line it occupies (`card_lines`) plus where each card starts
        // (`card_starts`, indexed by note rank for `scroll_to_note`).
        self.card_lines.clear();
        self.card_starts.clear();
        self.composer_span = None;
        if let Some(req) = &self.current {
            let width = self.viewport_w.max(MIN_CARD_WIDTH) as usize;
            // While a note is being rewritten its own card is hidden — the
            // composer box standing in its place *is* that note.
            let editing = self.composer.as_ref().and_then(|c| c.editing);
            let mut cards: Vec<(usize, Vec<Line<'static>>, bool)> = self
                .notes
                .iter()
                .filter(|n| n.file == req.file && Some(n.id) != editing)
                .map(|n| {
                    let anchor = if n.end == 0 {
                        0
                    } else {
                        built.line_for_new(n.end).map(|l| l + 1).unwrap_or(0)
                    };
                    let label = if n.end == 0 {
                        "note · whole file".to_string()
                    } else if n.start == n.end {
                        format!("note · line {}", n.start)
                    } else {
                        format!("note · lines {}-{}", n.start, n.end)
                    };
                    (
                        anchor,
                        note_card(&label, &n.text, width, self.cfg.theme),
                        false,
                    )
                })
                .collect();
            // The composer goes in last, so among cards anchored to the same
            // line the box you are typing in sits closest to the code.
            let composing = self.composer.as_ref().map(|c| {
                let note = self.pending_note.as_ref();
                let anchor = match note.map(|n| n.end) {
                    Some(0) | None => 0,
                    Some(end) => built.line_for_new(end).map(|l| l + 1).unwrap_or(0),
                };
                let label = match (c.editing.is_some(), note.map(|n| (n.start, n.end))) {
                    (true, Some((s, e))) if e > 0 && s == e => format!("edit note · line {s}"),
                    (true, Some((s, e))) if e > 0 => format!("edit note · lines {s}-{e}"),
                    (true, _) => "edit note · whole file".to_string(),
                    (false, Some((s, e))) if e > 0 && s == e => format!("new note · line {s}"),
                    (false, Some((s, e))) if e > 0 => format!("new note · lines {s}-{e}"),
                    (false, _) => "new note · whole file".to_string(),
                };
                (
                    anchor,
                    composer_card(&label, &c.input, width, self.cfg.theme),
                    true,
                )
            });
            if let Some(card) = composing {
                cards.push(card);
            }

            // Accent the gutter of every commented line *before* anything is
            // spliced in, while the anchors still index the unshifted doc, so
            // a line with a note is recognizable once its card scrolls away.
            for (anchor, _, _) in &cards {
                if *anchor > 0 {
                    accent_gutter(&mut doc.lines, anchor - 1, built);
                }
            }
            // Ascending by anchor (stable, so notes on one line keep their
            // order), then spliced in from the bottom up so an insertion
            // never shifts an anchor that hasn't been used yet.
            cards.sort_by_key(|(anchor, _, _)| *anchor);
            for (anchor, lines, _) in cards.iter().rev() {
                let at = (*anchor).min(doc.lines.len());
                doc.lines.splice(at..at, lines.iter().cloned());
            }
            // Then top-down for the index math: each card sits after every
            // card already spliced in above it.
            let mut shift = 0usize;
            for (anchor, lines, is_composer) in &cards {
                let start = anchor + shift;
                if *is_composer {
                    self.composer_span = Some((start, lines.len()));
                } else {
                    self.card_starts.push(start);
                }
                self.card_lines.extend(start..start + lines.len());
                shift += lines.len();
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
                // Dragging across a card extends past it, never onto it.
                let dir = if line >= self.cursor_line { 1 } else { -1 };
                self.cursor_line = self.snap_off_card(line, dir);
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

    /// Record the body size of the current draw. A width change re-boxes the
    /// note cards, so they always match the pane they are drawn in.
    pub fn set_viewport(&mut self, w: u16, h: u16) {
        self.viewport_h = h;
        if self.viewport_w != w {
            self.viewport_w = w;
            if matches!(self.state, State::Diff) {
                self.rebuild();
            }
        }
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
        self.keep_cursor_visible();
    }

    /// Drag the cursor along with the viewport so it is never off-screen:
    /// scrolling (wheel, or ctrl+d/u forwarded from the list) used to leave
    /// it behind, so focusing the diff pane showed no cursor at all and the
    /// first `j` jumped somewhere unrelated.
    ///
    /// A live selection is left alone — moving the cursor there would silently
    /// extend the selection.
    fn keep_cursor_visible(&mut self) {
        if self.select_anchor.is_some() {
            return;
        }
        let last = self.content_lines().saturating_sub(1);
        let top = self.scroll as usize;
        let bottom = (top + self.viewport_h.max(1) as usize - 1).min(last);
        let clamped = self.cursor_line.clamp(top.min(bottom), bottom);
        // Snapping away from the clamp edge keeps the cursor on screen.
        let dir = if clamped < self.cursor_line { -1 } else { 1 };
        let clamped = self.snap_off_card(clamped, dir);
        if clamped != self.cursor_line {
            self.cursor_line = clamped;
            self.restyle();
        }
    }

    /// The file line number under the cursor for the header: the new-file
    /// number, or the old one on a deleted line. `None` on a fold or a note
    /// card, which belong to no source line.
    pub fn cursor_file_line(&self) -> Option<u32> {
        let built = self.built.as_ref()?;
        let line = self.doc_to_built(self.cursor_line)?;
        let (old, new) = built.numbers_of_line(line)?;
        new.or(old)
    }

    /// Page relative to the viewport height (`full` = whole page, else half).
    pub fn page(&mut self, down: bool, full: bool) {
        let vh = self.viewport_h.max(1) as i32;
        let amount = if full { vh } else { (vh / 2).max(1) };
        self.scroll_by(if down { amount } else { -amount });
    }

    // ---- direct keys (preview pane focused) ------------------------------

    pub fn on_key(&mut self, ev: crossterm::event::KeyEvent) {
        // The composer owns every keystroke while it is open, so a note can
        // contain any character the keymap would otherwise claim.
        if self.composer.is_some() {
            self.compose_key(ev);
            return;
        }
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
        let (target, dir) = match delta {
            i32::MIN => (0, 1),
            i32::MAX => (last, -1),
            d => (
                (self.cursor_line as i64 + i64::from(d)).clamp(0, last as i64) as usize,
                if d < 0 { -1 } else { 1 },
            ),
        };
        // Step over a whole card rather than into it.
        self.cursor_line = self.snap_off_card(target, dir);
        // Keep the cursor inside the viewport.
        let vh = self.viewport_h.max(1) as usize;
        if self.cursor_line < self.scroll as usize {
            self.scroll = self.cursor_line as u16;
        } else if self.cursor_line >= self.scroll as usize + vh {
            self.scroll = (self.cursor_line + 1 - vh) as u16;
        }
        self.restyle();
    }

    /// The nearest line that is not part of a note card, searching in `dir`
    /// (-1 back, +1 forward) first and the other way if that runs out.
    ///
    /// Cards are read-only decoration. Letting the cursor land on one means
    /// you can select and annotate your own annotation, which is nonsense —
    /// and the doc↔source mapping has no line to give it either.
    fn snap_off_card(&self, line: usize, dir: i32) -> usize {
        let last = self.content_lines().saturating_sub(1);
        let line = line.min(last);
        if !self.card_lines.contains(&line) {
            return line;
        }
        let forward = (line..=last).find(|i| !self.card_lines.contains(i));
        let backward = (0..=line).rev().find(|i| !self.card_lines.contains(i));
        let (first, second) = if dir < 0 {
            (backward, forward)
        } else {
            (forward, backward)
        };
        first.or(second).unwrap_or(line)
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
        // Cards have just moved (a note was added, edited, deleted, or the
        // pane was resized) and may now sit under the cursor.
        self.cursor_line = self.snap_off_card(self.cursor_line, 1);
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
        // The cursor line has to read as "you are here" against the diff's
        // own red/green tints, so it is a real cursorline, not a whisper.
        let cursor_bg = if !self.cfg.theme.is_light() {
            Color::Rgb(0x39, 0x3b, 0x4f)
        } else {
            Color::Rgb(0xdf, 0xe4, 0xee)
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
                // Cards inside the range stay untinted — they are not part of
                // the selection and contribute nothing to the note.
                for idx in (a..=b).filter(|i| !self.card_lines.contains(i)) {
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
        // A range that mapped to no source line at all (only cards, or only
        // folds) would silently become a whole-file note — refuse instead.
        let (start, end) = match (numbers.iter().min(), numbers.iter().max()) {
            (Some(&s), Some(&e)) if s > 0 => (s, e),
            _ => {
                self.flash("select some code to annotate");
                return;
            }
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
        self.composer = Some(Composer {
            input: crate::textarea::TextArea::new(String::new()),
            editing: None,
        });
        self.rebuild();
        self.scroll_to_composer();
    }

    /// Keys while the inline composer is open. Mirrors any decent text
    /// input: enter saves, esc cancels, and the newline needs a modifier —
    /// `ctrl+j` alongside `shift+enter` because only terminals speaking the
    /// kitty keyboard protocol can report the latter.
    fn compose_key(&mut self, ev: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
        if ev.kind != KeyEventKind::Press && ev.kind != KeyEventKind::Repeat {
            return;
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let alt = ev.modifiers.contains(KeyModifiers::ALT);
        let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
        let width = self.composer_width();
        let Some(composer) = self.composer.as_mut() else {
            return;
        };
        match ev.code {
            KeyCode::Enter if shift || alt || ctrl => composer.input.insert_newline(),
            KeyCode::Char('j') if ctrl => composer.input.insert_newline(),
            KeyCode::Enter => {
                self.commit_composer();
                return;
            }
            KeyCode::Esc => {
                self.cancel_composer();
                return;
            }

            KeyCode::Char('u') if ctrl => composer.input.clear(),
            KeyCode::Char('w') if ctrl => composer.input.delete_word(),
            KeyCode::Backspace if ctrl || alt => composer.input.delete_word(),
            KeyCode::Backspace => composer.input.backspace(),
            KeyCode::Delete => composer.input.delete(),

            KeyCode::Left => composer.input.move_left(),
            KeyCode::Right => composer.input.move_right(),
            KeyCode::Up => composer.input.move_up(width),
            KeyCode::Down => composer.input.move_down(width),
            KeyCode::Home => composer.input.move_home(width),
            KeyCode::End => composer.input.move_end(width),
            KeyCode::Char('a') if ctrl => composer.input.move_home(width),
            KeyCode::Char('e') if ctrl => composer.input.move_end(width),

            KeyCode::Char(c) if !ctrl && !alt => composer.input.insert(c),
            _ => return,
        }
        self.rebuild();
        self.scroll_to_composer();
    }

    /// Text width inside the composer box (the card's own inner width).
    fn composer_width(&self) -> usize {
        card_text_width(self.viewport_w.max(MIN_CARD_WIDTH) as usize)
    }

    /// Enter in the composer: save a new note or the edit of an existing one.
    /// An empty note is a cancel — there is nothing to send an agent.
    fn commit_composer(&mut self) {
        let Some(composer) = self.composer.take() else {
            return;
        };
        let text = composer.input.text().trim_end().to_string();
        if text.is_empty() {
            self.pending_note = None;
            self.select_anchor = None;
            self.rebuild();
            return;
        }
        match composer.editing {
            Some(id) => {
                self.pending_note = None;
                self.edit_note(id, text);
            }
            None => self.finish_annotate(text),
        }
    }

    fn cancel_composer(&mut self) {
        self.composer = None;
        self.pending_note = None;
        self.select_anchor = None;
        self.rebuild();
    }

    /// Open the composer on an existing note (asked for by the notes view).
    /// Returns false when the note isn't in the shown file, so the caller can
    /// fall back rather than silently doing nothing.
    pub fn begin_edit_note(&mut self, id: u64) -> bool {
        let Some(note) = self.notes.iter().find(|n| n.id == id).cloned() else {
            return false;
        };
        if self.current.as_ref().map(|r| &r.file) != Some(&note.file) {
            return false;
        }
        self.select_anchor = None;
        self.composer = Some(Composer {
            input: crate::textarea::TextArea::new(note.text.clone()),
            editing: Some(id),
        });
        self.pending_note = Some(note);
        self.rebuild();
        self.scroll_to_composer();
        true
    }

    /// Open the composer for a whole-file note (asked for by the file list).
    pub fn begin_file_note(&mut self, file: PathBuf, cached: bool) -> bool {
        if self.current.as_ref().map(|r| &r.file) != Some(&file) {
            return false;
        }
        self.select_anchor = None;
        self.pending_note = Some(Note {
            id: 0,
            file,
            start: 0,
            end: 0,
            text: String::new(),
            snippet: String::new(),
            cached,
        });
        self.composer = Some(Composer {
            input: crate::textarea::TextArea::new(String::new()),
            editing: None,
        });
        self.rebuild();
        self.scroll_to_composer();
        true
    }

    /// Scroll so the whole composer box is on screen, preferring to keep its
    /// top visible when it is taller than the viewport.
    fn scroll_to_composer(&mut self) {
        let Some((start, len)) = self.composer_span else {
            return;
        };
        let vh = self.viewport_h.max(1) as usize;
        let end = start + len;
        if end > self.scroll as usize + vh {
            self.scroll = (end - vh) as u16;
        }
        if (self.scroll as usize) > start {
            self.scroll = start as u16;
        }
        self.clamp_scroll();
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
            && let Some(&line) = self.card_starts.get(rank)
        {
            self.scroll = (line.saturating_sub(3)) as u16;
            self.clamp_scroll();
            self.keep_cursor_visible();
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

/// Recolor the line-number cell of a commented row so it reads as annotated
/// even when its card is off-screen. The number is the first span on a
/// context row and the second on a `+`/`-` row (whose first span is the
/// change bar), which the row's old/new numbers identify.
fn accent_gutter(lines: &mut [Line<'static>], idx: usize, built: &crate::preview::render::DiffDoc) {
    let Some((old, new)) = built.numbers_of_line(idx) else {
        return; // a fold row has no line number to accent
    };
    let span_idx = match (old, new) {
        (Some(_), Some(_)) => 0, // context: " 1234 "
        _ => 1,                  // insertion/deletion: "▌" then "1234 "
    };
    if let Some(span) = lines.get_mut(idx).and_then(|l| l.spans.get_mut(span_idx)) {
        span.style = span.style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    }
}

/// One review note as a boxed block of display lines, spliced into the diff
/// under the line it comments on:
///
/// ```text
///   ╭─ note · lines 12-20 ────────────╮
///   │ the note text, wrapped to fit   │
///   ╰─────────────────────────────────╯
/// ```
///
/// Indented so it reads as a comment *on* the code rather than another diff
/// row, and boxed so a multi-line note stays visually one note.
fn note_card(
    label: &str,
    text: &str,
    width: usize,
    theme: crate::config::Theme,
) -> Vec<Line<'static>> {
    let rows: Vec<Vec<Span<'static>>> = text
        .split('\n')
        .flat_map(|logical| crate::textarea::wrap_plain(logical, card_text_width(width)))
        .map(|piece| vec![Span::raw(piece)])
        .collect();
    card_box(label, rows, width, theme, false)
}

/// The text width inside a card box at pane `width`: the indent, the two
/// borders, and the space either side of the text.
fn card_text_width(width: usize) -> usize {
    card_box_width(width).saturating_sub(4).max(1)
}

fn card_box_width(width: usize) -> usize {
    // Never wider than the pane allows, never narrower than a usable box.
    let outer = width
        .saturating_sub(CARD_INDENT)
        .max(MIN_CARD_WIDTH as usize);
    width
        .saturating_sub(CARD_INDENT + CARD_RIGHT_MARGIN)
        .min(MAX_CARD_WIDTH)
        .max(MIN_CARD_WIDTH as usize)
        .min(outer)
}

/// Indent of every card from the left edge, so a card reads as a comment
/// *on* the code rather than another diff row.
const CARD_INDENT: usize = 4;

/// Air left to the right of a card, so it doesn't run into the pane edge.
const CARD_RIGHT_MARGIN: usize = 6;

/// Cards stop growing past this: a comment is prose, and prose set across a
/// very wide pane is hard to read (and hard to tell apart from the diff).
const MAX_CARD_WIDTH: usize = 60;

/// The open composer as a card, with the caret drawn in place and an accented
/// border so it is obviously the thing taking your keystrokes.
fn composer_card(
    label: &str,
    input: &crate::textarea::TextArea,
    width: usize,
    theme: crate::config::Theme,
) -> Vec<Line<'static>> {
    let text_w = card_text_width(width);
    let caret = Style::new().add_modifier(Modifier::REVERSED);
    // An empty box says what it wants, with the caret waiting in front of it.
    if input.is_empty() {
        let hint = crate::textarea::elide_tail("write a note…", text_w.saturating_sub(1));
        return card_box(
            label,
            vec![vec![
                Span::styled(" ", caret),
                Span::styled(hint, Style::new().add_modifier(Modifier::DIM)),
            ]],
            width,
            theme,
            true,
        );
    }
    let rows = input.layout(text_w);
    let caret_row = input.caret_row(&rows);
    let body: Vec<Vec<Span<'static>>> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let text = &input.text()[r.clone()];
            if i != caret_row {
                return vec![Span::raw(text.to_string())];
            }
            let at = input.caret() - r.start;
            let (before, rest) = text.split_at(at);
            let mut chars = rest.chars();
            let under = chars.next();
            vec![
                Span::raw(before.to_string()),
                Span::styled(under.map(String::from).unwrap_or_else(|| " ".into()), caret),
                Span::raw(chars.collect::<String>()),
            ]
        })
        .collect();
    card_box(label, body, width, theme, true)
}

/// Draw a titled box around pre-wrapped rows of spans.
fn card_box(
    label: &str,
    rows: Vec<Vec<Span<'static>>>,
    width: usize,
    theme: crate::config::Theme,
    accent: bool,
) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;

    const INDENT: usize = CARD_INDENT;
    let box_w = card_box_width(width);
    let text_w = card_text_width(width);
    let border = Style::new().fg(if accent {
        Color::Yellow
    } else if theme.is_light() {
        Color::Rgb(0x9a, 0xa0, 0xa6)
    } else {
        Color::Rgb(0x6c, 0x70, 0x86)
    });
    let title = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let body = Style::new().fg(if theme.is_light() {
        Color::Rgb(0x4c, 0x4f, 0x69)
    } else {
        Color::Rgb(0xcd, 0xd6, 0xf4)
    });
    let pad = || Span::raw(" ".repeat(INDENT));

    let label = format!(" {label} ");
    let label = crate::textarea::elide_tail(&label, box_w.saturating_sub(3));
    let fill = box_w.saturating_sub(3 + label.width());
    let mut lines = vec![Line::from(vec![
        pad(),
        Span::styled("╭─", border),
        Span::styled(label, title),
        Span::styled(format!("{}╮", "─".repeat(fill)), border),
    ])];

    // An empty note still gets one body row, so the box never collapses.
    let rows = if rows.is_empty() {
        vec![vec![Span::raw(String::new())]]
    } else {
        rows
    };
    for spans in rows {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let gap = " ".repeat(text_w.saturating_sub(used));
        let mut line = vec![pad(), Span::styled("│ ", border)];
        line.extend(spans.into_iter().map(|s| {
            if s.style == Style::default() {
                Span::styled(s.content, body)
            } else {
                s
            }
        }));
        line.push(Span::styled(format!("{gap} │"), border));
        lines.push(Line::from(line));
    }

    lines.push(Line::from(vec![
        pad(),
        Span::styled(format!("╰{}╯", "─".repeat(box_w.saturating_sub(2))), border),
    ]));
    lines
}
