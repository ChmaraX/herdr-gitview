//! Preview-pane rendering: header (path + scope + line counter), the scrolled
//! diff body, and a footer of scroll hints. Reads `PreviewApp`, and updates
//! `viewport_h` so Page keys know the body height.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::app::{PreviewApp, State};
use crate::git::{ChangeKind, Scope};
use crate::keymap::Action;

pub fn render(frame: &mut Frame, app: &mut PreviewApp) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(1),    // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    app.set_viewport(chunks[1].width, chunks[1].height);

    render_header(frame, chunks[0], app);
    render_body(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);
}

// ---- header ---------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, app: &PreviewApp) {
    let left = match &app.current {
        Some(req) => {
            let scope = match (&req.commit, req.scope) {
                (Some(sha), _) => format!("commit {}", &sha[..sha.len().min(7)]),
                // Same words as the list pane's header: both panes must
                // name the same comparison, or the split is confusing.
                (None, Scope::Worktree) => "uncommitted".to_string(),
                (None, Scope::Branch) => {
                    format!("vs {}", app.base.as_deref().unwrap_or("base"))
                }
            };
            let staged = if req.cached && req.scope == Scope::Worktree {
                " [staged]"
            } else {
                ""
            };
            let deleted = if req.kind == ChangeKind::Deleted {
                " (deleted)"
            } else {
                ""
            };
            format!(" {}  [{scope}]{staged}{deleted}", req.file.display())
        }
        None => " gitview: diff".to_string(),
    };

    // Where the *cursor* is, not where the viewport starts: "which line am
    // I on" is the question this answers. `ln` is the file's own numbering
    // (absent on folds and note cards, which belong to no source line).
    let right = match app.state {
        State::Diff => match app.cursor_file_line() {
            Some(no) => format!("ln {no}  {}/{} ", app.cursor_line + 1, app.doc.lines.len()),
            None => format!("{}/{} ", app.cursor_line + 1, app.doc.lines.len()),
        },
        _ => String::new(),
    };

    let width = area.width as usize;
    let pad = width.saturating_sub(left.width() + right.width());
    let line = Line::from(vec![
        Span::styled(left, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, dim()),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(bar_bg(app.cfg.theme))),
        area,
    );
}

/// The surface color behind the header/footer bars (matches the list pane).
fn bar_bg(theme: crate::config::Theme) -> Color {
    if theme.is_light() {
        Color::Rgb(0xe6, 0xe9, 0xef)
    } else {
        Color::Rgb(0x31, 0x32, 0x44)
    }
}

// ---- body -----------------------------------------------------------------

fn render_body(frame: &mut Frame, area: Rect, app: &PreviewApp) {
    match &app.state {
        State::Splash(msg) => centered(frame, area, msg, dim()),
        State::Empty => {
            let msg = if app.current.as_ref().map(|c| c.cached).unwrap_or(false) {
                "(no staged changes)"
            } else {
                "(no changes in this view)"
            };
            centered(frame, area, msg, dim());
        }
        State::Binary => centered(frame, area, "(binary file)", dim()),
        State::Error(msg) => {
            let line = Line::from(Span::styled(
                format!(" diff error: {msg}"),
                Style::new().fg(Color::Red),
            ));
            frame.render_widget(Paragraph::new(line), area);
        }
        State::Diff => {
            let para = Paragraph::new(app.doc.clone()).scroll((app.scroll, 0));
            frame.render_widget(para, area);
        }
    }
}

/// Render a dim message centered vertically in `area`.
fn centered(frame: &mut Frame, area: Rect, msg: &str, style: Style) {
    let mid = Rect {
        x: area.x,
        y: area.y + area.height / 2,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(msg.to_string())
            .alignment(Alignment::Center)
            .style(style),
        mid,
    );
}

// ---- footer ---------------------------------------------------------------

fn render_footer(frame: &mut Frame, area: Rect, app: &PreviewApp) {
    // The composer owns the keyboard, so it owns the footer too — nothing
    // else advertised there works while you are typing a note.
    if app.composer.is_some() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " enter save · shift+enter / ctrl+j newline · esc cancel",
                Style::new().fg(Color::Yellow),
            )))
            .style(Style::new().bg(bar_bg(app.cfg.theme))),
            area,
        );
        return;
    }
    if let Some(msg) = app.active_flash() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {msg}"),
                Style::new().fg(Color::Yellow),
            )))
            .style(Style::new().bg(bar_bg(app.cfg.theme))),
            area,
        );
        return;
    }
    let hint = |a| app.keys.hint(a);
    // Only advertise what applies to the current diff.
    let has_diff = matches!(app.state, State::Diff);
    let annotatable = has_diff
        && app
            .current
            .as_ref()
            .map(|r| r.commit.is_none() && r.scope == Scope::Worktree)
            .unwrap_or(false);
    let mut pairs = Vec::new();
    if has_diff {
        pairs.push(("j/k".to_string(), "move".to_string()));
    }
    if annotatable {
        pairs.push((hint(Action::Select), "select".to_string()));
        pairs.push((hint(Action::Annotate), "note".to_string()));
    }
    if !app.notes.is_empty() {
        pairs.push((
            hint(Action::SendNotes),
            format!("send ({})", app.notes.len()),
        ));
    }
    if has_diff {
        pairs.push((
            format!("{}/{}", hint(Action::DiffTop), hint(Action::DiffBottom)),
            "ends".to_string(),
        ));
    }
    pairs.push((hint(Action::Quit), "quit".to_string()));
    // Drop tail hints that would overflow a narrow pane.
    let width = area.width as usize;
    let mut spans = Vec::new();
    let mut used = 0;
    for (i, (key, label)) in pairs.into_iter().enumerate() {
        let w = key.width() + label.width() + 2 + if i > 0 { 2 } else { 0 };
        if used + w > width {
            break;
        }
        used += w;
        if i > 0 {
            spans.push(Span::styled(" ·".to_string(), dim()));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().fg(Color::Cyan),
        ));
        spans.push(Span::styled(label.to_string(), dim()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(bar_bg(app.cfg.theme))),
        area,
    );
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}
