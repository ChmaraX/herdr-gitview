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
    /// The diff command produced no output.
    Empty,
    /// The diff command failed; holds the first stderr line.
    Error(String),
}

pub struct PreviewApp {
    pub cfg: Config,
    pub repo: Repo,
    pub keys: Keymap,

    /// Last Show request (what the header/stale-guard describe).
    pub current: Option<ShowReq>,
    /// Raw ANSI bytes of the current diff (kept for phase-4 line lookup).
    pub raw: Vec<u8>,
    /// Parsed, capped diff text (plus a truncation notice line when capped).
    pub doc: Text<'static>,
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
            raw: Vec::new(),
            doc: Text::default(),
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
    pub fn apply_diff(&mut self, req: &ShowReq, result: Result<Vec<u8>, String>) {
        if self.current.as_ref() != Some(req) {
            return; // stale — a newer Show already superseded this one
        }
        match result {
            Ok(bytes) => self.set_diff(bytes),
            Err(msg) => self.state = State::Error(msg),
        }
    }

    fn set_diff(&mut self, bytes: Vec<u8>) {
        self.raw = bytes;
        if self.raw.is_empty() {
            self.doc = Text::default();
            self.truncated = 0;
            self.state = State::Empty;
            self.clamp_scroll();
            return;
        }

        let (capped, hidden) = cap_lines(&self.raw, MAX_LINES);
        let mut doc = build_diff_doc(&capped);
        if hidden > 0 {
            doc.lines.push(Line::from(Span::styled(
                format!("… diff truncated ({hidden} more lines)"),
                Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
        }
        self.truncated = hidden;
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

// ---- diff styling ---------------------------------------------------------
//
// git's own ANSI colors render inconsistently across terminal themes, so the
// diff is re-styled here: strip the escapes, parse the unified-diff structure,
// and paint each line kind deliberately — with an old/new line-number gutter
// (the look borrows from persiyanov/herdr-reviewr).

fn build_diff_doc(raw: &[u8]) -> Text<'static> {
    let plain = super::editor::strip_ansi(raw);
    let text = String::from_utf8_lossy(&plain);

    let dim = Style::new().add_modifier(Modifier::DIM);
    let green = Style::new().fg(Color::Green);
    let red = Style::new().fg(Color::Red);
    let cyan = Style::new().fg(Color::Cyan);

    let mut lines: Vec<Line> = Vec::new();
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;
    let mut first_hunk_seen = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            (old_no, new_no) = hunk_start(line).unwrap_or((0, 0));
            if first_hunk_seen {
                lines.push(Line::from(Span::styled("─".repeat(60), dim)));
            }
            first_hunk_seen = true;
            // Keep the context hint after the second `@@`, drop the numbers.
            let hint = rest.split_once("@@").map(|x| x.1).unwrap_or("").trim();
            if !hint.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("… {hint}"),
                    cyan.add_modifier(Modifier::DIM),
                )));
            }
            continue;
        }
        if !first_hunk_seen {
            // File header block: diff --git / index / --- / +++ / rename …
            lines.push(Line::from(Span::styled(line.to_string(), dim)));
            continue;
        }
        let (gutter, style, content) = match line.as_bytes().first() {
            Some(b'+') => {
                let g = format!("{:>4} {:>4} ", "", new_no);
                new_no += 1;
                (g, green, line.to_string())
            }
            Some(b'-') => {
                let g = format!("{:>4} {:>4} ", old_no, "");
                old_no += 1;
                (g, red, line.to_string())
            }
            Some(b'\\') => ("          ".to_string(), dim, line.to_string()),
            _ => {
                let g = format!("{:>4} {:>4} ", old_no, new_no);
                old_no += 1;
                new_no += 1;
                (g, Style::new(), line.to_string())
            }
        };
        lines.push(Line::from(vec![
            Span::styled(gutter, dim),
            Span::styled(content, style),
        ]));
    }
    Text::from(lines)
}

/// Parse `@@ -a[,b] +c[,d] @@` into the starting (old, new) line numbers.
fn hunk_start(line: &str) -> Option<(u32, u32)> {
    let mut old = None;
    let mut new = None;
    for tok in line.split(' ') {
        if let Some(n) = tok.strip_prefix('-') {
            old = n.split(',').next()?.parse().ok();
        } else if let Some(n) = tok.strip_prefix('+') {
            new = n.split(',').next()?.parse().ok();
        }
    }
    Some((old?, new?))
}

/// Return the diff bytes truncated to at most `cap` lines, plus how many lines
/// were dropped. Cheap: counts `\n` bytes, no UTF-8 work.
fn cap_lines(raw: &[u8], cap: usize) -> (Vec<u8>, usize) {
    let newlines = raw.iter().filter(|b| **b == b'\n').count();
    let total = newlines + usize::from(raw.last().is_some_and(|b| *b != b'\n'));
    if total <= cap {
        return (raw.to_vec(), 0);
    }
    let mut count = 0;
    let mut end = raw.len();
    for (i, b) in raw.iter().enumerate() {
        if *b == b'\n' {
            count += 1;
            if count == cap {
                end = i; // keep up to (not including) the cap-th newline
                break;
            }
        }
    }
    (raw[..end].to_vec(), total - cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_lines_leaves_small_diffs_untouched() {
        let raw = b"a\nb\nc\n";
        let (out, hidden) = cap_lines(raw, 20_000);
        assert_eq!(out, raw);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn cap_lines_truncates_and_counts_remainder() {
        // 10 lines, cap at 4 → keep 4, hide 6.
        let raw: Vec<u8> = (0..10)
            .map(|i| format!("line{i}\n"))
            .collect::<String>()
            .into_bytes();
        let (out, hidden) = cap_lines(&raw, 4);
        assert_eq!(hidden, 6);
        assert_eq!(out.iter().filter(|b| **b == b'\n').count(), 3); // 4th newline dropped
    }
}
