//! Rendering for the file-list pane: header / body / footer, plus the help
//! and confirm overlays. Pure view code — it reads `App` and draws, mutating
//! only `list_offset` (so the selected row stays visible across redraws).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::App;
use super::app::{ListRow, Modal, Mode};
use crate::git::{ChangeKind, CommitInfo, FileEntry, Scope};
use crate::keymap::Action;

/// Actions shown in the help overlay, in a sensible reading order.
const HELP_ACTIONS: &[(Action, &str)] = &[
    (Action::Down, "move down"),
    (Action::Up, "move up"),
    (Action::Top, "jump to top"),
    (Action::Bottom, "jump to bottom"),
    (Action::Edit, "open in editor"),
    (Action::ToggleScope, "worktree / branch"),
    (Action::Stage, "stage / unstage file"),
    (Action::Discard, "discard changes"),
    (Action::Commit, "commit"),
    (Action::Log, "commit history"),
    (Action::ScrollDown, "scroll diff down"),
    (Action::ScrollUp, "scroll diff up"),
    (Action::HalfPageDown, "half page down"),
    (Action::HalfPageUp, "half page up"),
    (Action::DiffTop, "diff top"),
    (Action::DiffBottom, "diff bottom"),
    (Action::Refresh, "refresh"),
    (Action::Help, "help"),
    (Action::Quit, "back / quit"),
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
        Some(Modal::EditorClose { .. }) => render_confirm(
            frame,
            area,
            "editor has unsaved changes — y save · n discard · esc cancel",
        ),
        None => {}
    }
}

// ---- header ---------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let branch = app.branch.clone().unwrap_or_else(|| "DETACHED".to_string());
    let left = match (&app.busy, app.mode) {
        (Some(what), _) => format!(" {what}"),
        (None, Mode::Log) => format!(" log — {branch}"),
        (None, Mode::CommitFiles) => match &app.commit {
            Some(c) => format!(" {} {}", c.short, c.subject),
            None => " commit".to_string(),
        },
        (None, Mode::Files) => {
            let scope_label = match app.scope {
                Scope::Worktree => "working tree".to_string(),
                Scope::Branch => format!("vs {}", base_label(app)),
            };
            format!(" {branch}  {scope_label}")
        }
    };
    let right = match app.mode {
        Mode::Log => format!("{} commits ", app.commits.len()),
        _ => format!("{} files ", app.entries.len()),
    };

    let width = area.width as usize;
    let left = elide_tail(&left, width.saturating_sub(right.width() + 1));
    let pad = width.saturating_sub(left.width() + right.width());
    let line = Line::from(vec![
        Span::styled(left, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, dim()),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(bar_bg(&app.cfg.theme))),
        area,
    );
}

/// The surface color behind the header/footer bars (reviewr-style panels),
/// picked to fit the configured diff theme.
fn bar_bg(flavor: &str) -> Color {
    if flavor == "light" {
        Color::Rgb(0xe6, 0xe9, 0xef)
    } else {
        Color::Rgb(0x31, 0x32, 0x44)
    }
}

// ---- body -----------------------------------------------------------------

fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.rows.is_empty() {
        let msg = match (app.mode, app.scope) {
            (Mode::Log, _) => "no commits".to_string(),
            (Mode::CommitFiles, _) => "empty commit".to_string(),
            (_, Scope::Worktree) => "working tree clean".to_string(),
            (_, Scope::Branch) => format!("no changes vs {}", base_label(app)),
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

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match row {
            ListRow::Header { title, count } => header_row(title, *count),
            ListRow::Entry { idx, .. } => match app.entries.get(*idx) {
                Some(e) => entry_row(
                    e,
                    area.width,
                    app.mode == Mode::Files && app.scope == Scope::Worktree,
                ),
                None => ListItem::new(""),
            },
            ListRow::Commit(idx) => match app.commits.get(*idx) {
                Some(c) => commit_row(c, area.width),
                None => ListItem::new(""),
            },
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default().with_offset(app.list_offset);
    state.select(Some(app.cursor));
    frame.render_stateful_widget(list, area, &mut state);
    app.list_offset = state.offset();
}

fn header_row(title: &str, count: usize) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(
            format!(" ▾ {}", title.to_uppercase()),
            Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {count}"), dim()),
    ]))
}

/// One file row: `  <marker> <path>  <+a −d>` — marker colored by kind, dirs
/// dimmed, stats colored green/red flush right (reviewr's look). `indent`
/// only in the grouped worktree view.
fn entry_row(entry: &FileEntry, width: u16, grouped: bool) -> ListItem<'static> {
    let width = width as usize;
    let indent = if grouped { "  " } else { " " };
    let (stats_text, stats_spans) = stats(entry);
    let gap = if stats_text.is_empty() { 0 } else { 2 };
    let marker_w = 2; // letter + space
    let fixed = indent.len() + marker_w + stats_text.width() + gap;

    let path_text = path_text(entry);
    let shown = elide_head(&path_text, width.saturating_sub(fixed).max(1));
    let (dir, base) = split_dir(&shown);

    let (letter, color) = marker(entry.kind);
    let mut spans = vec![
        Span::raw(indent.to_string()),
        Span::styled(format!("{letter} "), Style::new().fg(color)),
    ];
    if !dir.is_empty() {
        spans.push(Span::styled(dir.to_string(), dim()));
    }
    spans.push(Span::raw(base.to_string()));
    if !stats_text.is_empty() {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = width.saturating_sub(used + stats_text.width());
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(stats_spans);
    }
    ListItem::new(Line::from(spans))
}

/// One commit row: `<short> <subject>  <date>`.
fn commit_row(c: &CommitInfo, width: u16) -> ListItem<'static> {
    let width = width as usize;
    let short = format!(" {} ", c.short);
    let date = format!("{} ", c.date);
    let avail = width.saturating_sub(short.width() + date.width() + 1);
    let subject = elide_tail(&c.subject, avail);
    let pad = width.saturating_sub(short.width() + subject.width() + date.width());
    ListItem::new(Line::from(vec![
        Span::styled(short, Style::new().fg(Color::Yellow)),
        Span::raw(subject),
        Span::raw(" ".repeat(pad)),
        Span::styled(date, dim()),
    ]))
}

fn marker(kind: ChangeKind) -> (char, Color) {
    match kind {
        ChangeKind::Modified => ('M', Color::Yellow),
        ChangeKind::Added => ('A', Color::Green),
        ChangeKind::Deleted => ('D', Color::Red),
        ChangeKind::Renamed => ('R', Color::Cyan),
        ChangeKind::Untracked => ('U', Color::Green),
        ChangeKind::Conflicted => ('!', Color::Red),
    }
}

/// The `+a −d` stats: text (for width) and colored spans, zero sides dropped.
fn stats(entry: &FileEntry) -> (String, Vec<Span<'static>>) {
    let (adds, dels) = match (entry.adds, entry.dels) {
        (Some(a), Some(d)) => (a, d),
        _ => return ("bin".into(), vec![Span::styled("bin", dim())]),
    };
    let mut text = String::new();
    let mut spans = Vec::new();
    if adds > 0 || dels == 0 {
        text.push_str(&format!("+{adds}"));
        spans.push(Span::styled(
            format!("+{adds}"),
            Style::new().fg(Color::Green),
        ));
    }
    if dels > 0 {
        if !text.is_empty() {
            text.push(' ');
            spans.push(Span::raw(" "));
        }
        text.push_str(&format!("−{dels}"));
        spans.push(Span::styled(
            format!("−{dels}"),
            Style::new().fg(Color::Red),
        ));
    }
    (text, spans)
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
fn elide_head(s: &str, max: usize) -> String {
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

/// Right-truncate to `max` display columns, suffixing `…` when cut.
fn elide_tail(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1;
    let mut acc = 0;
    let mut kept = String::new();
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        kept.push(ch);
    }
    format!("{kept}…")
}

// ---- footer ---------------------------------------------------------------

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.active_status() {
        Some(msg) => Line::from(Span::styled(
            format!(" {msg}"),
            Style::new().fg(Color::Yellow),
        )),
        None => footer_hints(app),
    };
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(bar_bg(&app.cfg.theme))),
        area,
    );
}

/// ` key label · key label … ` — keys accented, labels dim (reviewr's look).
fn footer_hints(app: &App) -> Line<'static> {
    let pairs: Vec<(String, &str)> = match app.mode {
        Mode::Log => vec![
            (sym(app.keys.hint(Action::Edit)), "show commit"),
            (sym(app.keys.hint(Action::Quit)), "back"),
            (sym(app.keys.hint(Action::Help)), "help"),
        ],
        Mode::CommitFiles => vec![
            (sym(app.keys.hint(Action::Edit)), "edit"),
            (sym(app.keys.hint(Action::HalfPageDown)), "scroll"),
            (sym(app.keys.hint(Action::Quit)), "back"),
            (sym(app.keys.hint(Action::Help)), "help"),
        ],
        Mode::Files => vec![
            (sym(app.keys.hint(Action::Edit)), "edit"),
            (sym(app.keys.hint(Action::Stage)), "stage"),
            (sym(app.keys.hint(Action::Discard)), "discard"),
            (sym(app.keys.hint(Action::Commit)), "commit"),
            (sym(app.keys.hint(Action::Log)), "log"),
            (sym(app.keys.hint(Action::Help)), "help"),
            (sym(app.keys.hint(Action::Quit)), "quit"),
        ],
    };
    let mut spans = Vec::new();
    for (i, (key, label)) in pairs.into_iter().enumerate() {
        if key.is_empty() {
            continue;
        }
        if i > 0 {
            spans.push(Span::styled(" ·".to_string(), dim()));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().fg(Color::Cyan),
        ));
        spans.push(Span::styled(label.to_string(), dim()));
    }
    Line::from(spans)
}

/// Prettify a hint for the footer (`enter` → `↵`).
fn sym(hint: String) -> String {
    if hint == "enter" {
        "↵".to_string()
    } else {
        hint
    }
}

// ---- overlays -------------------------------------------------------------

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = HELP_ACTIONS
        .iter()
        .filter(|(action, _)| !app.keys.hint(*action).is_empty())
        .map(|(action, label)| {
            Line::from(vec![
                Span::styled(
                    format!(" {:>8}  ", app.keys.hint(*action)),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw((*label).to_string()),
            ])
        })
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
