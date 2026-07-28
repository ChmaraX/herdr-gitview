//! Popup entrypoints for the review-notes flow:
//! - `annotate`: a note input (title from `GITVIEW_ASK_TEXT`); writes the
//!   note text (empty = cancelled) to the answer file. The text wraps and
//!   scrolls, so a note longer than the popup keeps its caret in view.
//! - `pick-agent`: choose an agent pane from `GITVIEW_AGENTS` (JSON
//!   `[[pane_id, agent, status], …]`); writes `pane\tplace`, `pane\tsubmit`,
//!   or `cancel`. Enter places the notes in the agent's prompt; shift+enter
//!   or ctrl+enter also submits them.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::popup::write_answer;

/// The caret drawn at the end of the input.
const CARET: char = '\u{2588}';

/// Popup size for the note input, shared by every caller so the wrapping
/// width is predictable. Tall enough for a few sentences; longer notes
/// scroll rather than disappearing.
pub const NOTE_POPUP_SIZE: (u16, u16) = (72, 14);

pub fn run_annotate() -> Result<()> {
    let title = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "note".into());
    // Editing an existing note pre-fills the input.
    let mut input = std::env::var("GITVIEW_PREFILL").unwrap_or_default();

    let mut terminal = ratatui::init();
    let answer = loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1), // title
                Constraint::Min(1),    // input
                Constraint::Length(1), // hint
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(format!(" {title}"))
                    .style(Style::new().add_modifier(Modifier::BOLD)),
                chunks[0],
            );

            // Wrap by hand so the view can follow the caret: a Paragraph
            // just clips, which silently hides everything past the last
            // visible row once the note outgrows the box.
            let inner_w = chunks[1].width.saturating_sub(3).max(1) as usize; // borders + lead space
            let inner_h = chunks[1].height.saturating_sub(2).max(1) as usize;
            let lines = caret_lines(&input, inner_w);
            let scrolled = lines.len().saturating_sub(inner_h);
            let visible: Vec<Line> = lines[scrolled..]
                .iter()
                .map(|l| Line::from(format!(" {l}")))
                .collect();
            let block = Block::bordered();
            frame.render_widget(Paragraph::new(visible).block(block), chunks[1]);

            frame.render_widget(
                Paragraph::new(hint_text(&input, scrolled > 0))
                    .alignment(Alignment::Center)
                    .style(Style::new().add_modifier(Modifier::DIM)),
                chunks[2],
            );
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Enter => break input.clone(),
                KeyCode::Esc => break String::new(),
                // Long notes need more than one-char-at-a-time deletion.
                KeyCode::Char('u') if ctrl => input.clear(),
                KeyCode::Char('w') if ctrl => delete_word(&mut input),
                KeyCode::Backspace if ctrl => delete_word(&mut input),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) if !ctrl => input.push(c),
                _ => {}
            }
        }
    };
    ratatui::restore();
    write_answer(&answer)
}

/// The footer hint, with a character count once there's text and a marker
/// when earlier lines have scrolled out of view.
fn hint_text(input: &str, scrolled: bool) -> String {
    let mut hint = String::from("enter save · esc cancel");
    if !input.is_empty() {
        hint.push_str(&format!(" · {} chars", input.chars().count()));
    }
    if scrolled {
        hint.push_str(" · ↑ more");
    }
    hint
}

/// Drop the trailing word (and any whitespace before it) — ctrl+w / ctrl+bs.
fn delete_word(input: &mut String) {
    while input.ends_with(char::is_whitespace) {
        input.pop();
    }
    while !input.is_empty() && !input.ends_with(char::is_whitespace) {
        input.pop();
    }
}

/// The wrapped input with the caret appended, always as the last line — so
/// showing the last `height` lines keeps the caret visible at any length.
fn caret_lines(input: &str, width: usize) -> Vec<String> {
    let mut lines = wrap_text(input, width);
    let caret_w = CARET.width().unwrap_or(1);
    let last = lines.last().map(|l| l.width()).unwrap_or(0);
    if last + caret_w > width {
        lines.push(String::new()); // caret rolls onto the next line
    }
    lines.last_mut().expect("never empty").push(CARET);
    lines
}

/// Wrap `text` to `width` display columns, breaking at spaces where that
/// works and hard-breaking anything longer than a line (long paths, urls).
/// Always returns at least one (possibly empty) line.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;
    // The pending run of characters that may still move to the next line
    // together (a word), plus the spaces that preceded it on this line.
    let mut word = String::new();
    let mut word_w = 0usize;

    let flush_word =
        |line: &mut String, line_w: &mut usize, word: &mut String, word_w: &mut usize| {
            line.push_str(word);
            *line_w += *word_w;
            word.clear();
            *word_w = 0;
        };

    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if ch.is_whitespace() {
            flush_word(&mut line, &mut line_w, &mut word, &mut word_w);
            if line_w + cw > width {
                lines.push(std::mem::take(&mut line));
                line_w = 0;
                continue; // the wrap swallows the space
            }
            line.push(ch);
            line_w += cw;
            continue;
        }
        // A word longer than a whole line can't be moved — hard-break it.
        if word_w + cw > width {
            flush_word(&mut line, &mut line_w, &mut word, &mut word_w);
            lines.push(std::mem::take(&mut line));
            line_w = 0;
        }
        if line_w + word_w + cw > width {
            lines.push(std::mem::take(&mut line));
            line_w = 0;
        }
        word.push(ch);
        word_w += cw;
    }
    flush_word(&mut line, &mut line_w, &mut word, &mut word_w);
    lines.push(line);
    lines
}

pub fn run_pick_agent() -> Result<()> {
    let title = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "send notes to…".into());
    let agents: Vec<(String, String, String, String, String)> =
        serde_json::from_str(&std::env::var("GITVIEW_AGENTS").unwrap_or_default())
            .unwrap_or_default();
    if agents.is_empty() {
        write_answer("cancel")?;
        return Ok(());
    }
    let mut cursor = 0usize;

    let mut terminal = ratatui::init();
    crate::term::enable_mouse();
    let answer = loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1), // title
                Constraint::Min(1),    // agents
                Constraint::Length(2), // hint
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(format!(" {title}"))
                    .style(Style::new().add_modifier(Modifier::BOLD)),
                chunks[0],
            );
            let items: Vec<ListItem> = agents
                .iter()
                .map(|(pane, agent, status, tab, cwd)| {
                    let mut spans = vec![
                        Span::styled(
                            format!("  {agent}"),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" · {status}"),
                            Style::new().fg(color_for_status(status)),
                        ),
                    ];
                    if !tab.is_empty() {
                        spans.push(Span::raw(format!("  tab: {tab}")));
                    }
                    if !cwd.is_empty() {
                        spans.push(Span::styled(
                            format!("  {cwd}"),
                            Style::new().add_modifier(Modifier::DIM),
                        ));
                    }
                    spans.push(Span::styled(
                        format!("  {pane}"),
                        Style::new().add_modifier(Modifier::DIM),
                    ));
                    ListItem::new(Line::from(spans))
                })
                .collect();
            let list =
                List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            let mut state = ListState::default();
            state.select(Some(cursor));
            frame.render_stateful_widget(list, chunks[1], &mut state);
            frame.render_widget(
                Paragraph::new("enter place in prompt · shift/ctrl+enter send now\nesc cancel")
                    .alignment(Alignment::Center)
                    .style(Style::new().add_modifier(Modifier::DIM)),
                chunks[2],
            );
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Enter
                    if key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
                {
                    break format!("{}\tsubmit", agents[cursor].0);
                }
                KeyCode::Enter => break format!("{}\tplace", agents[cursor].0),
                KeyCode::Down | KeyCode::Char('j') => {
                    cursor = (cursor + 1).min(agents.len() - 1);
                }
                KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                KeyCode::Esc | KeyCode::Char('q') => break "cancel".to_string(),
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                // Rows start under the title line.
                let idx = m.row.saturating_sub(1) as usize;
                if idx < agents.len() {
                    if cursor == idx {
                        break format!("{}\tplace", agents[idx].0); // click-click = choose
                    }
                    cursor = idx;
                }
            }
            _ => {}
        }
    };
    crate::term::disable_mouse();
    ratatui::restore();
    write_answer(&answer)
}

fn color_for_status(status: &str) -> Color {
    match status {
        "idle" => Color::Green,
        "working" => Color::Yellow,
        _ => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last `height` lines are what the popup actually draws.
    fn visible(input: &str, width: usize, height: usize) -> Vec<String> {
        let lines = caret_lines(input, width);
        let start = lines.len().saturating_sub(height);
        lines[start..].to_vec()
    }

    #[test]
    fn short_text_stays_on_one_line_with_the_caret() {
        assert_eq!(caret_lines("hi", 10), vec![format!("hi{CARET}")]);
    }

    #[test]
    fn empty_input_is_just_a_caret() {
        assert_eq!(caret_lines("", 10), vec![CARET.to_string()]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        // A space that still fits stays on the line it was typed on — this is
        // a live input, so the caret must sit after the space you just typed.
        assert_eq!(
            wrap_text("the quick brown fox", 10),
            vec!["the quick ", "brown fox"]
        );
        // A space landing exactly on the edge is swallowed by the wrap.
        assert_eq!(wrap_text("abcde fgh", 5), vec!["abcde", "fgh"]);
    }

    #[test]
    fn hard_breaks_words_longer_than_a_line() {
        // A long path can't be moved to the next line whole.
        assert_eq!(
            wrap_text("src/preview/render.rs", 8),
            vec!["src/prev", "iew/rend", "er.rs"]
        );
    }

    #[test]
    fn no_wrapped_line_exceeds_the_width() {
        let text = "this note mentions src/list/session.rs and \
                    a-very-long-hyphenated-identifier plus prose";
        for width in [4, 7, 12, 20, 33] {
            for line in wrap_text(text, width) {
                assert!(
                    line.width() <= width,
                    "width {width}: {line:?} is {} cols",
                    line.width()
                );
            }
        }
    }

    #[test]
    fn wrapping_preserves_every_word() {
        let text = "the quick brown fox jumps over the lazy dog";
        let joined = wrap_text(text, 11).join(" ");
        let words: Vec<&str> = joined.split_whitespace().collect();
        assert_eq!(words, text.split_whitespace().collect::<Vec<_>>());
    }

    /// The regression: a note longer than the box used to be drawn past the
    /// bottom edge, hiding the caret and everything the user was typing.
    #[test]
    fn the_caret_stays_visible_however_long_the_note_gets() {
        let (width, height) = (20, 4);
        let mut input = String::new();
        for _ in 0..200 {
            input.push('x');
            let shown = visible(&input, width, height);
            assert!(shown.len() <= height, "drew {} lines", shown.len());
            assert!(
                shown.last().unwrap().contains(CARET),
                "caret fell out of view at {} chars",
                input.chars().count()
            );
        }
    }

    #[test]
    fn the_caret_rolls_onto_the_next_line_when_the_last_one_is_full() {
        // "abcd" exactly fills width 4, so the caret needs a line of its own.
        let lines = caret_lines("abcd", 4);
        assert_eq!(lines, vec!["abcd".to_string(), CARET.to_string()]);
    }

    #[test]
    fn a_degenerate_width_does_not_panic() {
        assert_eq!(wrap_text("abc", 0), vec![String::new()]);
        assert!(!caret_lines("abc", 1).is_empty());
    }

    #[test]
    fn ctrl_w_deletes_the_trailing_word() {
        let mut input = String::from("fix the parser bug");
        delete_word(&mut input);
        assert_eq!(input, "fix the parser ");
        delete_word(&mut input);
        assert_eq!(input, "fix the ");
    }

    #[test]
    fn deleting_a_word_from_an_empty_or_blank_input_is_safe() {
        let mut input = String::new();
        delete_word(&mut input);
        assert_eq!(input, "");
        let mut input = String::from("   ");
        delete_word(&mut input);
        assert_eq!(input, "");
    }

    #[test]
    fn the_hint_reports_length_and_scrolling() {
        assert_eq!(hint_text("", false), "enter save · esc cancel");
        assert!(hint_text("abc", false).contains("3 chars"));
        assert!(hint_text("abc", true).contains("↑ more"));
    }
}
