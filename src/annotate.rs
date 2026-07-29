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
use crate::textarea::TextArea;

/// Popup size for the note input, shared by every caller so the wrapping
/// width is predictable. Tall enough for a few sentences; longer notes
/// scroll rather than disappearing.
pub const NOTE_POPUP_SIZE: (u16, u16) = (72, 14);

pub fn run_annotate() -> Result<()> {
    let title = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "note".into());
    // Editing an existing note pre-fills the input. Newlines survive the
    // env-var trip as an escape (herdr's --env framing is line-based).
    let mut input = TextArea::new(decode_prefill(
        &std::env::var("GITVIEW_PREFILL").unwrap_or_default(),
    ));
    // First visible row; kept between draws so the view follows the caret
    // instead of snapping.
    let mut scroll = 0usize;
    let (mut scroll_above, mut scroll_below) = (false, false);

    let mut terminal = ratatui::init();
    crate::term::enable_key_disambiguation();
    let answer = loop {
        // The wrap width the last draw used, so key handling that needs it
        // (up/down/home/end) agrees with what is on screen.
        let mut width = 1usize;
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1), // title
                Constraint::Min(1),    // input
                Constraint::Length(1), // hint
            ])
            .split(area);
            frame.render_widget(
                header_line(&title, &input, scroll_above, scroll_below, chunks[0].width),
                chunks[0],
            );

            let inner_w = chunks[1].width.saturating_sub(3).max(1) as usize; // borders + lead space
            let inner_h = chunks[1].height.saturating_sub(2).max(1) as usize;
            width = inner_w;

            // Scroll the window the smallest amount that keeps the caret in
            // it — a Paragraph would just clip, which is what used to hide
            // everything past the last visible row.
            let rows = input.layout(inner_w);
            let caret_row = input.caret_row(&rows);
            scroll = scroll.min(caret_row);
            if caret_row >= scroll + inner_h {
                scroll = caret_row + 1 - inner_h;
            }
            scroll = scroll.min(rows.len().saturating_sub(1));

            let end = (scroll + inner_h).min(rows.len());
            scroll_above = scroll > 0;
            scroll_below = end < rows.len();
            let lines: Vec<Line> = (scroll..end)
                .map(|i| render_row(&input, &rows[i], i == caret_row))
                .collect();
            frame.render_widget(Paragraph::new(lines).block(Block::bordered()), chunks[1]);

            frame.render_widget(
                Paragraph::new(HINT)
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
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            match key.code {
                // Enter saves; every modifier combination a terminal can
                // actually deliver for "newline" inserts one instead.
                KeyCode::Enter if shift || alt || ctrl => input.insert_newline(),
                KeyCode::Enter => break input.text().to_string(),
                // ctrl+j is a literal LF: the fallback for terminals that
                // report shift+enter as a plain enter.
                KeyCode::Char('j') if ctrl => input.insert_newline(),
                KeyCode::Esc => break String::new(),

                KeyCode::Char('u') if ctrl => input.clear(),
                KeyCode::Char('w') if ctrl => input.delete_word(),
                KeyCode::Backspace if ctrl || alt => input.delete_word(),
                KeyCode::Backspace => input.backspace(),
                KeyCode::Delete => input.delete(),

                KeyCode::Left => input.move_left(),
                KeyCode::Right => input.move_right(),
                KeyCode::Up => input.move_up(width),
                KeyCode::Down => input.move_down(width),
                KeyCode::Home => input.move_home(width),
                KeyCode::End => input.move_end(width),
                KeyCode::Char('a') if ctrl => input.move_home(width),
                KeyCode::Char('e') if ctrl => input.move_end(width),

                KeyCode::Char(c) if !ctrl && !alt => input.insert(c),
                _ => {}
            }
        }
    };
    crate::term::disable_key_disambiguation();
    ratatui::restore();
    write_answer(&answer)
}

/// One visual row, with the caret drawn as a reversed cell when it sits on
/// this row (so it shows mid-text, not just at the end).
fn render_row<'a>(input: &'a TextArea, row: &std::ops::Range<usize>, has_caret: bool) -> Line<'a> {
    let text = &input.text()[row.clone()];
    if !has_caret {
        return Line::from(format!(" {text}"));
    }
    let at = input.caret() - row.start;
    let (before, rest) = text.split_at(at);
    let mut chars = rest.chars();
    let under = chars.next();
    let after: String = chars.collect();
    vec![
        Span::raw(format!(" {before}")),
        Span::styled(
            under.map(String::from).unwrap_or_else(|| " ".into()),
            Style::new().add_modifier(Modifier::REVERSED),
        ),
        Span::raw(after),
    ]
    .into()
}

/// The footer keys. `ctrl+j` is spelled out next to `shift+enter` because
/// only terminals speaking the kitty keyboard protocol can report the
/// latter — everywhere else it arrives as a plain enter (i.e. save).
const HINT: &str = "enter save · shift+enter / ctrl+j newline · esc cancel";

/// The title row: the note's anchor on the left, and on the right a
/// character count plus markers for rows scrolled out of view. Lives here
/// rather than in the footer so neither can crowd the other out.
fn header_line(
    title: &str,
    input: &TextArea,
    above: bool,
    below: bool,
    width: u16,
) -> Line<'static> {
    let mut meta = String::new();
    if !input.is_empty() {
        meta.push_str(&format!("{} chars ", input.char_count()));
    }
    match (above, below) {
        (true, true) => meta.push_str("↑↓ "),
        (true, false) => meta.push_str("↑ "),
        (false, true) => meta.push_str("↓ "),
        (false, false) => {}
    }
    let width = width as usize;
    // A very narrow popup keeps the counter and loses the anchor, not both.
    if meta.width() > width {
        meta = elide_tail(&meta, width);
    }
    let title = elide_tail(&format!(" {title}"), width.saturating_sub(meta.width() + 1));
    let pad = width.saturating_sub(title.width() + meta.width());
    Line::from(vec![
        Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(meta, Style::new().add_modifier(Modifier::DIM)),
    ])
}

/// Right-truncate to `max` display columns, suffixing `…` when cut.
fn elide_tail(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut acc = 0;
    let mut kept = String::new();
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > max - 1 {
            break;
        }
        acc += cw;
        kept.push(ch);
    }
    format!("{kept}…")
}

/// Newlines can't ride in a herdr `--env` value, so they travel as `\n`.
pub fn encode_prefill(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Inverse of [`encode_prefill`].
pub fn decode_prefill(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
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

    /// The plain text of a rendered line, for assertions.
    fn flat(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_hint_advertises_both_newline_keys() {
        assert!(HINT.contains("shift+enter"), "{HINT}");
        assert!(HINT.contains("ctrl+j"), "{HINT}");
        assert!(HINT.contains("enter save"), "{HINT}");
    }

    #[test]
    fn the_header_reports_the_length_and_scroll_state() {
        let empty = TextArea::new(String::new());
        assert_eq!(
            flat(&header_line("note", &empty, false, false, 40)).trim(),
            "note"
        );
        let a = TextArea::new("abc".into());
        assert!(flat(&header_line("note", &a, false, false, 40)).contains("3 chars"));
        assert!(flat(&header_line("note", &a, true, false, 40)).contains("↑"));
        assert!(flat(&header_line("note", &a, false, true, 40)).contains("↓"));
        assert!(flat(&header_line("note", &a, true, true, 40)).contains("↑↓"));
    }

    #[test]
    fn the_header_fits_its_width_even_with_a_long_anchor() {
        let a = TextArea::new("x".repeat(500));
        for width in [12u16, 20, 40, 72] {
            let line = header_line(
                "note for src/preview/render/very/deep/path/module.rs",
                &a,
                true,
                true,
                width,
            );
            assert!(
                flat(&line).width() <= width as usize,
                "width {width}: {:?}",
                flat(&line)
            );
        }
    }

    #[test]
    fn prefill_survives_newlines_and_backslashes() {
        for original in [
            "one line",
            "two\nlines",
            "trailing\n",
            "\n\nblank lines\n\n",
            r"a windows\path and a \n literal",
            "",
        ] {
            let encoded = encode_prefill(original);
            assert!(
                !encoded.contains('\n'),
                "{encoded:?} still has a raw newline"
            );
            assert_eq!(
                decode_prefill(&encoded),
                original,
                "round trip {original:?}"
            );
        }
    }

    #[test]
    fn decoding_a_stray_trailing_backslash_does_not_panic() {
        assert_eq!(decode_prefill(r"ends with \"), r"ends with \");
        assert_eq!(decode_prefill(r"\q unknown escape"), r"\q unknown escape");
    }

    #[test]
    fn the_caret_row_renders_the_character_under_the_caret() {
        let mut input = TextArea::new("abc".into());
        input.move_left(); // caret on 'c'
        let rows = input.layout(10);
        let line = render_row(&input, &rows[0], true);
        // " ab" + reversed "c" + ""
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, " ab");
        assert_eq!(line.spans[1].content, "c");
        assert!(
            line.spans[1]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn the_caret_at_the_end_renders_as_a_reversed_space() {
        let input = TextArea::new("ab".into());
        let rows = input.layout(10);
        let line = render_row(&input, &rows[0], true);
        assert_eq!(line.spans[1].content, " ");
    }

    #[test]
    fn a_row_without_the_caret_renders_plainly() {
        let input = TextArea::new("ab\ncd".into());
        let rows = input.layout(10);
        let line = render_row(&input, &rows[0], false);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, " ab");
    }
}
