//! Note cards: the boxed blocks spliced into the diff under the lines they
//! comment on, and the composer box that stands in their place while a note
//! is being written.
//!
//! Pure presentation — nothing here touches `PreviewApp`. It sits beside
//! `render.rs` rather than in `ui.rs` because these lines are spliced into
//! the *document* before any frame is drawn.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::textarea::TextArea;

/// Floor for the card width, so a card still has a shape before the first
/// draw has reported the real pane width.
pub const MIN_WIDTH: u16 = 24;

/// One block spliced into the diff: a saved note's card, or the composer.
pub struct Card {
    /// Doc line (pre-splice) the block is inserted at.
    pub anchor: usize,
    pub lines: Vec<Line<'static>>,
    /// The note this card shows; `None` for the composer.
    pub note: Option<u64>,
}

/// Where a note anchors in the built diff, and whether its line is gone.
/// `end == 0` means a whole-file note, which legitimately sits at the top;
/// a line that cannot be found is a *different* thing and says so.
pub fn anchor_of(built: &super::render::DiffDoc, end: u32) -> (usize, bool) {
    if end == 0 {
        return (0, false);
    }
    match built.line_for_new(end) {
        Some(line) => (line + 1, false),
        None => (0, true),
    }
}

/// `<prefix> · line 12` / `· lines 12-20` / `· whole file`.
pub fn range_label(prefix: &str, start: u32, end: u32) -> String {
    match (start, end) {
        (_, 0) => format!("{prefix} · whole file"),
        (s, e) if s == e => format!("{prefix} · line {s}"),
        (s, e) => format!("{prefix} · lines {s}-{e}"),
    }
}

/// Recolor the line-number cell of a commented row so it reads as annotated
/// even when its card is off-screen. The number is the first span on a
/// context row and the second on a `+`/`-` row (whose first span is the
/// change bar), which the row's old/new numbers identify.
pub fn accent_gutter(lines: &mut [Line<'static>], idx: usize, built: &super::render::DiffDoc) {
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
pub fn note_card(
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
pub fn card_text_width(width: usize) -> usize {
    card_box_width(width).saturating_sub(4).max(1)
}

fn card_box_width(width: usize) -> usize {
    // Never wider than the pane allows, never narrower than a usable box.
    let outer = width.saturating_sub(CARD_INDENT).max(MIN_WIDTH as usize);
    width
        .saturating_sub(CARD_INDENT + CARD_RIGHT_MARGIN)
        .min(MAX_CARD_WIDTH)
        .max(MIN_WIDTH as usize)
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
pub fn composer_card(
    label: &str,
    input: &TextArea,
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
