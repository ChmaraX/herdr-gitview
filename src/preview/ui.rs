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

    app.set_viewport(chunks[1].height);

    render_header(frame, chunks[0], app);
    render_body(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);
}

// ---- header ---------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, app: &PreviewApp) {
    let left = match &app.current {
        Some(req) => {
            let scope = match req.scope {
                Scope::Worktree => "worktree".to_string(),
                Scope::Branch => format!("branch vs {}", app.base.as_deref().unwrap_or("base")),
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

    let right = match app.state {
        State::Diff => format!("{}/{} ", app.scroll as usize + 1, app.doc.lines.len()),
        _ => String::new(),
    };

    let width = area.width as usize;
    let pad = width.saturating_sub(left.width() + right.width());
    let line = Line::from(vec![
        Span::raw(left),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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
    let hint = |a| app.keys.hint(a);
    let text = format!(
        " {}/{} scroll  {}/{} page  {}/{} ends  {} quit",
        hint(Action::ScrollDown),
        hint(Action::ScrollUp),
        hint(Action::HalfPageDown),
        hint(Action::HalfPageUp),
        hint(Action::DiffTop),
        hint(Action::DiffBottom),
        hint(Action::Quit),
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, dim()))),
        area,
    );
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}
