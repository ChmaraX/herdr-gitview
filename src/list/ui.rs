//! Rendering for the file-list pane: header / body / footer, plus the help
//! overlay. Pure view code — it reads `App` and draws, mutating only
//! `list_offset` (so the selected row stays visible across redraws).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::App;
use super::app::Modal;
use crate::git::{ChangeKind, FileEntry, Scope, StageState};
use crate::keymap::Action;

/// Actions shown in the help overlay, in a sensible reading order.
const HELP_ACTIONS: &[(Action, &str)] = &[
    (Action::Down, "move down"),
    (Action::Up, "move up"),
    (Action::Top, "jump to top"),
    (Action::Bottom, "jump to bottom"),
    (Action::Edit, "open in editor"),
    (Action::ToggleScope, "worktree / branch"),
    (Action::ToggleCached, "staged / unstaged view"),
    (Action::Stage, "stage file"),
    (Action::Discard, "discard changes"),
    (Action::Commit, "commit"),
    (Action::ScrollDown, "scroll diff down"),
    (Action::ScrollUp, "scroll diff up"),
    (Action::HalfPageDown, "half page down"),
    (Action::HalfPageUp, "half page up"),
    (Action::DiffTop, "diff top"),
    (Action::DiffBottom, "diff bottom"),
    (Action::Refresh, "refresh"),
    (Action::Help, "help"),
    (Action::Quit, "quit"),
];

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(1),    // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    render_header(frame, chunks[0], app);
    render_body(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);

    match &app.modal {
        Some(Modal::Help) => render_help(frame, area, app),
        Some(Modal::Confirm { text, .. }) => render_confirm(frame, area, text),
        None => {}
    }
}

// ---- header ---------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let branch = app.branch.clone().unwrap_or_else(|| "DETACHED".to_string());
    let scope_label = match app.scope {
        Scope::Worktree => "working tree".to_string(),
        Scope::Branch => format!("vs {}", base_label(app)),
    };
    let left = match &app.busy {
        Some(what) => format!(" {what}"),
        None => format!(" {branch}  {scope_label}"),
    };
    let right = format!("{} files ", app.entries.len());

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

fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.entries.is_empty() {
        let msg = match app.scope {
            Scope::Worktree => "working tree clean".to_string(),
            Scope::Branch => format!("no changes vs {}", base_label(app)),
        };
        let mid = Rect {
            x: area.x,
            y: area.y + area.height / 2,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(dim()),
            mid,
        );
        return;
    }

    let items: Vec<ListItem> = app.entries.iter().map(|e| row(e, area.width)).collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default().with_offset(app.list_offset);
    state.select(Some(app.cursor));
    frame.render_stateful_widget(list, area, &mut state);
    app.list_offset = state.offset();
}

/// One row: `<stage-dot><marker> <path>  <+a -d>` (stats flush right).
fn row(entry: &FileEntry, width: u16) -> ListItem<'static> {
    let width = width as usize;
    let stats = stats_str(entry);
    let stats_w = stats.width();
    let prefix_w = 3; // dot + marker + space
    let min_gap = 1;

    let path_text = path_text(entry);
    let avail = width.saturating_sub(prefix_w + stats_w + min_gap);
    let shown = truncate_left(&path_text, avail);
    let shown_w = shown.width();
    let pad = width.saturating_sub(prefix_w + shown_w + stats_w);

    let (marker_char, marker_style) = marker(entry.kind);
    let (dir, base) = split_dir(&shown);

    let mut spans = vec![
        dot_span(entry.stage),
        Span::styled(marker_char.to_string(), marker_style),
        Span::raw(" "),
    ];
    if !dir.is_empty() {
        spans.push(Span::styled(dir.to_string(), dim()));
    }
    spans.push(Span::raw(base.to_string()));
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(stats, dim()));

    ListItem::new(Line::from(spans))
}

fn dot_span(stage: StageState) -> Span<'static> {
    match stage {
        StageState::Staged => Span::styled("●", Style::new().fg(Color::Green)),
        StageState::Partial => Span::styled("◐", Style::new().fg(Color::Yellow)),
        _ => Span::raw(" "),
    }
}

fn marker(kind: ChangeKind) -> (char, Style) {
    match kind {
        ChangeKind::Modified => ('M', Style::new().fg(Color::Yellow)),
        ChangeKind::Added => ('A', Style::new().fg(Color::Green)),
        ChangeKind::Deleted => ('D', Style::new().fg(Color::Red)),
        ChangeKind::Renamed => ('R', Style::new().fg(Color::Cyan)),
        ChangeKind::Untracked => ('?', Style::new().fg(Color::Magenta)),
        ChangeKind::Conflicted => (
            'U',
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    }
}

fn stats_str(entry: &FileEntry) -> String {
    match (entry.adds, entry.dels) {
        (Some(a), Some(d)) => format!("+{a} -{d}"),
        _ => "bin".to_string(),
    }
}

fn path_text(entry: &FileEntry) -> String {
    match &entry.orig_path {
        Some(orig) => format!("{} → {}", orig.display(), entry.path.display()),
        None => entry.path.display().to_string(),
    }
}

/// Split a path into (dir-prefix-including-slash, basename).
fn split_dir(s: &str) -> (&str, &str) {
    match s.rfind('/') {
        Some(i) => (&s[..=i], &s[i + 1..]),
        None => ("", s),
    }
}

/// Left-truncate to `max` display columns, prefixing `…` when cut.
fn truncate_left(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // reserve a column for the ellipsis
    let mut acc = 0;
    let mut kept: Vec<char> = Vec::new();
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        kept.push(ch);
    }
    kept.reverse();
    let tail: String = kept.into_iter().collect();
    format!("…{tail}")
}

// ---- footer ---------------------------------------------------------------

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.active_status() {
        Some(msg) => Line::from(Span::styled(
            format!(" {msg}"),
            Style::new().fg(Color::Yellow),
        )),
        None => Line::from(Span::styled(footer_hints(app), dim())),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn footer_hints(app: &App) -> String {
    let k = |a| sym(app.keys.hint(a));
    format!(
        " {} edit  {} stage  {} discard  {} commit  {} scope  {} help  {} quit",
        k(Action::Edit),
        k(Action::Stage),
        k(Action::Discard),
        k(Action::Commit),
        k(Action::ToggleScope),
        k(Action::Help),
        k(Action::Quit),
    )
}

/// Prettify a hint for the footer (`enter` → `↵`).
fn sym(hint: String) -> String {
    if hint == "enter" {
        "↵".to_string()
    } else {
        hint
    }
}

// ---- help overlay ---------------------------------------------------------

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = HELP_ACTIONS
        .iter()
        .map(|(action, label)| Line::from(format!(" {:>8}  {label}", app.keys.hint(*action))))
        .collect();

    let content_w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let w = (content_w + 4).min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let popup = centered_rect(area, w, h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys ")),
        popup,
    );
}

/// Yes/no confirmation: centered bordered box, wrapped text, max 60×5.
fn render_confirm(frame: &mut Frame, area: Rect, text: &str) {
    let w = 60.min(area.width);
    let h = 5.min(area.height);
    let popup = centered_rect(area, w, h);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text.to_string())
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(Block::bordered().title(" confirm ")),
        popup,
    );
}

fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

// ---- small helpers --------------------------------------------------------

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn base_label(app: &App) -> &str {
    if app.base.is_empty() {
        "base"
    } else {
        &app.base
    }
}
