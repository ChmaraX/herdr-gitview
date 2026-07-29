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
use crate::git::{ChangeKind, CommitInfo, FileEntry, Scope, StageState};
use crate::keymap::Action;
use crate::textarea::elide_tail;

/// Actions shown in the help overlay, in a sensible reading order.
const HELP_ACTIONS: &[(Action, &str)] = &[
    (Action::Down, "move down"),
    (Action::Up, "move up"),
    (Action::Top, "jump to top"),
    (Action::Bottom, "jump to bottom"),
    (Action::Edit, "open in editor"),
    (
        Action::ToggleScope,
        "worktree / branch (log: branch commits)",
    ),
    (Action::Stage, "stage / unstage file or folder"),
    (Action::Unstage, "unstage file or folder"),
    (Action::Discard, "discard changes (file or folder)"),
    (Action::Commit, "commit"),
    (Action::Log, "commit history"),
    (Action::Annotate, "add review note"),
    (Action::SendNotes, "send notes to an agent"),
    (Action::Select, "select lines (preview)"),
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
        Some(_) if app.modal_external => {} // shown as a herdr popup pane
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
        (None, Mode::Log) if app.log_branch_only => {
            format!(" log — {branch}  vs {}", base_label(app))
        }
        (None, Mode::Log) => format!(" log — {branch}  all"),
        (None, Mode::CommitFiles) => match &app.commit {
            Some(c) => format!(" {} {}", c.short, c.subject),
            None => " commit".to_string(),
        },
        (None, Mode::Notes) => " review notes".to_string(),
        (None, Mode::Files) => {
            let scope_label = match app.scope {
                Scope::Worktree => "working tree".to_string(),
                Scope::Branch => format!("vs {}", base_label(app)),
            };
            format!(" {branch}  {scope_label}")
        }
    };
    let right_spans = header_right_spans(app);
    let right_width: usize = right_spans.iter().map(Span::width).sum();

    let width = area.width as usize;
    let left = elide_tail(&left, width.saturating_sub(right_width + 1));
    let pad = width.saturating_sub(left.width() + right_width);
    let mut line_spans = vec![
        Span::styled(left, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
    ];
    line_spans.extend(right_spans);
    let line = Line::from(line_spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(bar_bg(app.cfg.theme))),
        area,
    );
}

/// The header's right-aligned summary: commit/note counts for history
/// views, or "N files +A −D" (zero sides dropped, like the per-file stats)
/// for Files/CommitFiles.
fn header_right_spans(app: &App) -> Vec<Span<'static>> {
    match app.mode {
        Mode::Log => vec![Span::styled(
            format!("{} commits ", app.commits.len()),
            dim(),
        )],
        Mode::Notes => vec![Span::styled(format!("{} notes ", app.notes.len()), dim())],
        _ => {
            let adds: u32 = app.entries.iter().filter_map(|e| e.adds).sum();
            let dels: u32 = app.entries.iter().filter_map(|e| e.dels).sum();
            let mut spans = vec![Span::styled(format!("{} files", app.entries.len()), dim())];
            if adds > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("+{adds}"),
                    Style::new().fg(Color::Green),
                ));
            }
            if dels > 0 {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("−{dels}"),
                    Style::new().fg(Color::Red),
                ));
            }
            spans.push(Span::raw(" "));
            spans
        }
    }
}

/// The surface color behind the header/footer bars, picked to fit the
/// configured diff theme.
fn bar_bg(theme: crate::config::Theme) -> Color {
    if theme.is_light() {
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
            (Mode::Notes, _) => "no notes".to_string(),
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
            ListRow::Dir {
                depth,
                name,
                collapsed,
                ..
            } => dir_row(
                name,
                *depth,
                app.mode == Mode::Files && app.scope == Scope::Worktree,
                *collapsed,
            ),
            ListRow::Entry { idx, depth, .. } => match app.entries.get(*idx) {
                Some(e) => entry_row(
                    e,
                    area.width,
                    app.mode == Mode::Files && app.scope == Scope::Worktree,
                    *depth,
                ),
                None => ListItem::new(""),
            },
            ListRow::Commit(idx) => match app.commits.get(*idx) {
                Some(c) => commit_row(c, area.width),
                None => ListItem::new(""),
            },
            ListRow::NoteFile { name, count } => note_file_row(name, *count, area.width),
            ListRow::Note(idx) => match app.notes.get(*idx) {
                Some(n) => note_row(n, area.width),
                None => ListItem::new(""),
            },
        })
        .collect();
    let highlight_bg = if app.cfg.theme.is_light() {
        Color::Rgb(0xd9, 0xde, 0xe8)
    } else {
        Color::Rgb(0x3d, 0x3f, 0x54)
    };
    let list = List::new(items).highlight_style(Style::new().bg(highlight_bg));

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

/// A tree directory row: dim, indented 2 spaces per depth (plus the
/// section's own indent), with a collapse arrow and a trailing slash on the
/// (possibly folded) name. Selectable — Enter or a click toggles collapse.
fn dir_row(name: &str, depth: usize, grouped: bool, collapsed: bool) -> ListItem<'static> {
    let section_indent = if grouped { "  " } else { " " };
    let indent = format!("{section_indent}{}", "  ".repeat(depth));
    let arrow = if collapsed { "▸" } else { "▾" };
    ListItem::new(Line::from(Span::styled(
        format!("{indent}{arrow} {name}"),
        dim(),
    )))
}

/// One file row: `  <marker> <name>  <+a −d>` — marker colored by kind, name
/// is the basename only (the tree already shows the directory nesting),
/// stats colored green/red flush right. `indent` combines the section's own
/// indent (grouped worktree view vs. flat) with 2 spaces per tree depth.
fn entry_row(entry: &FileEntry, width: u16, grouped: bool, depth: usize) -> ListItem<'static> {
    let width = width as usize;
    let section_indent = if grouped { "  " } else { " " };
    let indent = format!("{section_indent}{}", "  ".repeat(depth));
    let (stats_text, stats_spans) = stats(entry);
    let gap = if stats_text.is_empty() { 0 } else { 2 };
    let marker_w = 2; // letter + space
    let fixed = indent.width() + marker_w + stats_text.width() + gap;

    let name_text = basename_text(entry);
    let shown = elide_head(&name_text, width.saturating_sub(fixed).max(1));

    let (letter, color) = marker(entry.kind);
    let mut spans = vec![
        Span::raw(indent),
        Span::styled(format!("{letter} "), Style::new().fg(color)),
        Span::raw(shown),
    ];
    if !stats_text.is_empty() {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = width.saturating_sub(used + stats_text.width());
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(stats_spans);
    }
    ListItem::new(Line::from(spans))
}

/// A file heading in the notes view: `▾ SRC/LIST/APP.RS  2`, matching the
/// section headers in the files view.
fn note_file_row(name: &str, count: usize, width: u16) -> ListItem<'static> {
    let name = elide_tail(&name.to_uppercase(), (width as usize).saturating_sub(7));
    ListItem::new(Line::from(vec![
        Span::styled(
            format!(" ▾ {name}"),
            Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {count}"), dim()),
    ]))
}

/// One note, drawn as two lines so the text gets a line of its own:
///
/// ```text
///   ▎ lines 12-20
///     the note text, elided if it runs long
/// ```
fn note_row(note: &crate::ipc::NoteMeta, width: u16) -> ListItem<'static> {
    let anchor = if note.end == 0 {
        "whole file".to_string()
    } else if note.start == note.end {
        format!("line {}", note.start)
    } else {
        format!("lines {}-{}", note.start, note.end)
    };
    let text = crate::ipc::one_line(&note.text);
    let text = elide_tail(&text, (width as usize).saturating_sub(6));
    ListItem::new(vec![
        Line::from(Span::styled(
            format!("   ▎ {anchor}"),
            Style::new().fg(Color::Yellow),
        )),
        Line::from(vec![
            Span::styled("   ▎ ".to_string(), Style::new().fg(Color::Yellow)),
            Span::raw(text),
        ]),
    ])
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

/// The name shown on a file row: just the basename (the tree already shows
/// the directory nesting), or "old-basename → new-basename" for renames.
fn basename_text(entry: &FileEntry) -> String {
    fn base(p: &std::path::Path) -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    }
    match &entry.orig_path {
        Some(orig) => format!("{} → {}", base(orig), base(&entry.path)),
        None => base(&entry.path),
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

// ---- footer ---------------------------------------------------------------

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.active_status() {
        Some(msg) => Line::from(Span::styled(
            format!(" {msg}"),
            Style::new().fg(Color::Yellow),
        )),
        None => footer_hints(app, area.width as usize),
    };
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(bar_bg(app.cfg.theme))),
        area,
    );
}

/// ` key label · key label … ` — keys accented, labels dim.
/// On narrow panes, hints are dropped from the tail until the line fits;
/// `? help` always survives so everything stays discoverable.
fn footer_hints(app: &App, width: usize) -> Line<'static> {
    let pairs: Vec<(String, &str)> = match app.mode {
        Mode::Log => {
            let mut pairs = Vec::new();
            if !app.commits.is_empty() {
                pairs.push((sym(app.keys.hint(Action::Edit)), "show commit"));
            }
            pairs.push((
                sym(app.keys.hint(Action::ToggleScope)),
                if app.log_branch_only {
                    "all commits"
                } else {
                    "branch only"
                },
            ));
            pairs.push((sym(app.keys.hint(Action::Quit)), "back"));
            pairs.push((sym(app.keys.hint(Action::Help)), "help"));
            pairs
        }
        Mode::CommitFiles => {
            let mut pairs = Vec::new();
            if !app.entries.is_empty() {
                pairs.push((sym(app.keys.hint(Action::Edit)), "edit"));
                pairs.push((sym(app.keys.hint(Action::HalfPageDown)), "scroll"));
            }
            pairs.push((sym(app.keys.hint(Action::Quit)), "back"));
            pairs.push((sym(app.keys.hint(Action::Help)), "help"));
            pairs
        }
        Mode::Notes => vec![
            (sym(app.keys.hint(Action::Edit)), "edit note"),
            (sym(app.keys.hint(Action::Delete)), "delete"),
            (sym(app.keys.hint(Action::SendNotes)), "send"),
            (sym(app.keys.hint(Action::Quit)), "back"),
            (sym(app.keys.hint(Action::Help)), "help"),
        ],
        Mode::Files => {
            // Only advertise what is currently possible.
            let has_files = !app.entries.is_empty();
            let worktree = app.scope == Scope::Worktree;
            // What the selection actually allows, straight from the model —
            // the footer used to re-derive this with a repo-wide predicate
            // and advertise keys that then refused to run.
            let ops = app.selection_ops();
            let any_staged = app
                .entries
                .iter()
                .any(|e| matches!(e.stage, StageState::Staged | StageState::Partial));

            let mut pairs = Vec::new();
            if has_files {
                pairs.push((sym(app.keys.hint(Action::Edit)), "edit"));
            }
            if ops.stage {
                pairs.push((
                    sym(app.keys.hint(Action::Stage)),
                    if ops.stage_unstages {
                        "unstage"
                    } else {
                        "stage"
                    },
                ));
            }
            if ops.unstage && !ops.stage_unstages {
                pairs.push((sym(app.keys.hint(Action::Unstage)), "unstage"));
            }
            if ops.discard {
                pairs.push((sym(app.keys.hint(Action::Discard)), "discard"));
            }
            if worktree && any_staged {
                pairs.push((sym(app.keys.hint(Action::Commit)), "commit"));
            }
            if worktree && has_files {
                pairs.push((sym(app.keys.hint(Action::Annotate)), "note"));
            }
            if !app.notes.is_empty() {
                pairs.push((sym(app.keys.hint(Action::NotesView)), "notes"));
            }
            pairs.push((sym(app.keys.hint(Action::Log)), "log"));
            pairs.push((sym(app.keys.hint(Action::Help)), "help"));
            pairs.push((sym(app.keys.hint(Action::Quit)), "quit"));
            pairs
        }
    };
    hint_line(pairs, width)
}

/// Build the hint spans, dropping tail hints that would overflow `width`
/// (the `help` pair is retained even when others are cut).
fn hint_line(pairs: Vec<(String, &str)>, width: usize) -> Line<'static> {
    let pair_w = |key: &str, label: &str| key.width() + label.width() + 2; // " k label"
    let sep_w = 2; // " ·"

    let pairs: Vec<(String, &str)> = pairs.into_iter().filter(|(k, _)| !k.is_empty()).collect();
    let help = pairs.iter().position(|(_, label)| *label == "help");
    let help_w = help
        .map(|i| pair_w(&pairs[i].0, pairs[i].1) + sep_w)
        .unwrap_or(0);

    let mut kept: Vec<(String, &str)> = Vec::new();
    let mut used = 0;
    let mut cut = false;
    for (i, (key, label)) in pairs.iter().enumerate() {
        // After the first cut only the help pair may still enter, so the
        // footer never shows a gap-toothed subset.
        if cut && *label != "help" {
            continue;
        }
        let w = pair_w(key, label) + if kept.is_empty() { 0 } else { sep_w };
        // Reserve room for the upcoming help pair while it hasn't landed yet.
        let reserve = match help {
            Some(h) if h > i => help_w,
            _ => 0,
        };
        if used + w + if cut { 0 } else { reserve } > width {
            cut = true;
            continue;
        }
        used += w;
        kept.push((key.clone(), label));
    }

    let mut spans = Vec::new();
    for (i, (key, label)) in kept.into_iter().enumerate() {
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
