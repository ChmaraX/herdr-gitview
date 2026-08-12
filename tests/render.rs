//! Render tests: draw the list pane into a ratatui `TestBackend` at 40×12 and
//! assert on the resulting buffer (marker chars, stats alignment, selected-row
//! styling, empty state). No git or terminal needed — `App::from_entries`
//! takes a preloaded entry vector.

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use herdr_gitview::config::Config;
use herdr_gitview::git::{ChangeKind, FileEntry, Repo, StageState};
use herdr_gitview::keymap::Keymap;
use herdr_gitview::list::App;
use herdr_gitview::list::ui;

const W: u16 = 40;
const H: u16 = 12;

fn entry(path: &str, kind: ChangeKind, stage: StageState, adds: u32, dels: u32) -> FileEntry {
    FileEntry {
        path: PathBuf::from(path),
        orig_path: None,
        kind,
        stage,
        adds: Some(adds),
        dels: Some(dels),
    }
}

fn app_with(entries: Vec<FileEntry>) -> App {
    let cfg = Config::default();
    let keys = Keymap::build(&HashMap::new()).unwrap();
    let repo = Repo {
        root: PathBuf::from("."),
    };
    App::from_entries(repo, cfg, keys, entries)
}

/// Render into a fresh 40×12 backend and return the buffer.
fn draw(app: &mut App) -> Buffer {
    let backend = TestBackend::new(W, H);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// The text of buffer row `y`.
fn row_text(buf: &Buffer, y: u16) -> String {
    (0..W).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn markers_and_stats_render() {
    let mut app = app_with(vec![
        entry(
            "src/main.rs",
            ChangeKind::Modified,
            StageState::Unstaged,
            3,
            1,
        ),
        entry("new.rs", ChangeKind::Added, StageState::Staged, 10, 0),
        entry(
            "notes.txt",
            ChangeKind::Untracked,
            StageState::Untracked,
            5,
            0,
        ),
    ]);
    let buf = draw(&mut app);

    // Grouped VSCode-style layout: staged section first, then changes.
    // y=0 header · y=1 "STAGED CHANGES" · y=2 new.rs · y=3 "CHANGES"
    // · y=4 "src/" dir row (main.rs nests under it) · y=5 main.rs · y=6 notes.txt
    assert!(row_text(&buf, 1).contains("STAGED CHANGES"));
    let staged = row_text(&buf, 2);
    assert!(row_text(&buf, 3).contains("CHANGES"));
    assert!(
        row_text(&buf, 4).contains("src/"),
        "dir row: {}",
        row_text(&buf, 4)
    );
    let modified = row_text(&buf, 5);
    let untracked = row_text(&buf, 6);

    // Markers sit after the 2-column section indent (dirs push files one
    // tree-depth deeper, so main.rs's marker shifts one column right).
    assert_eq!(buf[(2, 2)].symbol(), "A");
    assert_eq!(buf[(4, 5)].symbol(), "M");
    assert_eq!(buf[(2, 6)].symbol(), "U");

    // Stats colored + right-aligned, zero sides dropped (− is unicode minus).
    assert!(modified.contains("+3 −1"), "modified: {modified:?}");
    assert!(staged.trim_end().ends_with("+10"), "staged: {staged:?}");
    assert!(
        untracked.trim_end().ends_with("+5"),
        "untracked: {untracked:?}"
    );

    // Basename is shown; directory prefix is dimmed but still present.
    assert!(modified.contains("main.rs"), "modified: {modified:?}");
}

#[test]
fn partial_file_appears_in_both_sections() {
    let mut app = app_with(vec![entry(
        "both.rs",
        ChangeKind::Modified,
        StageState::Partial,
        2,
        1,
    )]);
    let buf = draw(&mut app);
    let body: String = (1..H - 1).map(|y| row_text(&buf, y) + "\n").collect();
    let hits = body.matches("both.rs").count();
    assert_eq!(
        hits, 2,
        "partial file should be in staged AND changes:\n{body}"
    );
}

#[test]
fn selected_row_is_reversed() {
    let mut app = app_with(vec![
        entry("a.rs", ChangeKind::Modified, StageState::Unstaged, 1, 0),
        entry("b.rs", ChangeKind::Modified, StageState::Unstaged, 1, 0),
    ]);
    // Rows: y=1 "CHANGES" header, y=2 a.rs (cursor row 1), y=3 b.rs (row 2).
    app.cursor = 2;
    let buf = draw(&mut app);

    let selected_bg = buf[(1, 3)].style().bg;
    let other_bg = buf[(1, 2)].style().bg;
    assert!(selected_bg.is_some(), "selected row should have a bg tint");
    assert_ne!(
        selected_bg, other_bg,
        "non-selected row should not share the highlight tint"
    );
}

#[test]
fn header_shows_file_count() {
    let mut app = app_with(vec![
        entry("a.rs", ChangeKind::Modified, StageState::Unstaged, 1, 0),
        entry("b.rs", ChangeKind::Added, StageState::Staged, 2, 0),
    ]);
    let buf = draw(&mut app);
    let header = row_text(&buf, 0);
    assert!(header.contains("2 files"), "header: {header:?}");
}

#[test]
fn empty_state_message() {
    let mut app = app_with(vec![]);
    let buf = draw(&mut app);
    let body: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(body.contains("working tree clean"), "body: {body:?}");
}

#[test]
fn narrow_footer_drops_tail_hints_but_keeps_help() {
    let mut app = app_with(vec![entry(
        "a.rs",
        ChangeKind::Modified,
        StageState::Unstaged,
        1,
        0,
    )]);
    // Render at 30 columns — far too narrow for the full files-mode footer.
    let backend = TestBackend::new(30, H);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let footer: String = (0..30).map(|x| buf[(x, H - 1)].symbol()).collect();

    assert!(footer.contains("help"), "help must survive: {footer:?}");
    assert!(!footer.contains("quit"), "tail hints dropped: {footer:?}");
    // Nothing rendered beyond the width (no wrap artifacts on the row above).
    let above: String = (0..30).map(|x| buf[(x, H - 2)].symbol()).collect();
    assert!(!above.contains("help"), "no wrapping: {above:?}");
}

// ---- the notes panel -------------------------------------------------------

fn note(id: u64, file: &str, start: u32, end: u32, text: &str) -> herdr_gitview::ipc::NoteMeta {
    herdr_gitview::ipc::NoteMeta {
        id,
        file: PathBuf::from(file),
        start,
        end,
        text: text.to_string(),
        cached: false,
    }
}

fn notes_app(notes: Vec<herdr_gitview::ipc::NoteMeta>) -> App {
    let mut app = app_with(vec![]);
    app.notes = notes;
    app.toggle_notes_view();
    app
}

#[test]
fn notes_are_grouped_under_a_heading_per_file() {
    let mut app = notes_app(vec![
        note(1, "src/a.rs", 12, 20, "first note"),
        note(2, "src/b.rs", 3, 3, "other file"),
        note(3, "src/a.rs", 44, 44, "second note on a"),
    ]);
    let buf = draw(&mut app);
    let body: Vec<String> = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    let joined = body.join("\n");

    // One heading per file, with its own count, notes gathered under it.
    assert!(joined.contains("SRC/A.RS"), "{joined}");
    assert!(joined.contains("SRC/B.RS"), "{joined}");
    let a_head = body.iter().position(|l| l.contains("SRC/A.RS")).unwrap();
    let b_head = body.iter().position(|l| l.contains("SRC/B.RS")).unwrap();
    assert!(body[a_head].contains('2'), "a.rs count: {:?}", body[a_head]);
    assert!(body[b_head].contains('1'), "b.rs count: {:?}", body[b_head]);
    // Both of a.rs's notes precede b.rs's heading.
    assert!(
        body[a_head..b_head]
            .iter()
            .any(|l| l.contains("first note"))
    );
    assert!(
        body[a_head..b_head]
            .iter()
            .any(|l| l.contains("second note on a"))
    );
}

#[test]
fn a_note_draws_its_anchor_and_text_on_separate_lines() {
    let mut app = notes_app(vec![note(1, "src/a.rs", 12, 20, "prefer a named constant")]);
    let buf = draw(&mut app);
    let body: Vec<String> = (1..H - 1).map(|y| row_text(&buf, y)).collect();

    let anchor = body.iter().position(|l| l.contains("lines 12-20")).unwrap();
    assert!(
        body[anchor + 1].contains("prefer a named constant"),
        "text should be on its own line: {:?}",
        body[anchor + 1]
    );
}

#[test]
fn a_single_line_and_whole_file_note_read_naturally() {
    let mut app = notes_app(vec![
        note(1, "a.rs", 7, 7, "one line"),
        note(2, "b.rs", 0, 0, "the whole thing"),
    ]);
    let buf = draw(&mut app);
    let joined: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(joined.contains("line 7"), "{joined}");
    assert!(!joined.contains("lines 7-7"), "{joined}");
    assert!(joined.contains("whole file"), "{joined}");
}

#[test]
fn the_headings_are_skipped_when_moving_and_clicking() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = notes_app(vec![
        note(1, "src/a.rs", 1, 1, "one"),
        note(2, "src/b.rs", 2, 2, "two"),
    ]);
    draw(&mut app);

    // The cursor lands on the first *note*, not the heading above it.
    assert_eq!(app.selected_note().map(|n| n.id), Some(1));
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(
        app.selected_note().map(|n| n.id),
        Some(2),
        "skips the heading"
    );

    // A click hit-tests through the two-line note rows: body line 0 is the
    // first heading, 1-2 the first note, 3 the second heading, 4-5 the second.
    assert_eq!(app.row_at(1), app.row_at(2), "both lines select one note");
    app.on_mouse(
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        3, // +1 for the header band
    );
    assert_eq!(app.selected_note().map(|n| n.id), Some(1));
    app.on_mouse(
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        6,
    );
    assert_eq!(app.selected_note().map(|n| n.id), Some(2));
}

#[test]
fn a_long_note_is_elided_rather_than_overflowing() {
    let mut app = notes_app(vec![note(1, "a.rs", 1, 1, &"very long note ".repeat(20))]);
    let buf = draw(&mut app);
    for y in 1..H - 1 {
        assert!(row_text(&buf, y).chars().count() as u16 <= W);
    }
    let joined: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(joined.contains('…'), "expected an elision marker");
}

#[test]
fn a_multi_line_note_collapses_to_one_line_in_the_panel() {
    let mut app = notes_app(vec![note(1, "a.rs", 1, 1, "first\nsecond")]);
    let buf = draw(&mut app);
    let joined: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(joined.contains("first ⏎ second"), "{joined}");
}

#[test]
fn a_repo_git_cannot_report_on_says_so_instead_of_clean() {
    let mut app = app_with(vec![]);
    // Empty because git is clean.
    let buf = draw(&mut app);
    let body: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(body.contains("working tree clean"), "{body}");

    // Empty because git failed: that must not read as clean, and it must not
    // expire the way the footer message does.
    app.load_error = Some("git status failed: bad things".to_string());
    let buf = draw(&mut app);
    let body: String = (1..H - 1).map(|y| row_text(&buf, y)).collect();
    assert!(body.contains("bad things"), "{body}");
    assert!(!body.contains("working tree clean"), "{body}");
    assert!(body.contains("retries"), "no way out offered: {body}");
}

// ---- "which diff am I looking at" ------------------------------------------

/// Render at an explicit width, for the wide-pane header extras.
fn draw_wide(app: &mut App, width: u16) -> Buffer {
    let backend = TestBackend::new(width, H);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn text_at(buf: &Buffer, y: u16, width: u16) -> String {
    (0..width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn the_header_names_the_comparison_not_just_the_branch() {
    let mut app = app_with(vec![entry(
        "src/main.rs",
        ChangeKind::Modified,
        StageState::Unstaged,
        3,
        1,
    )]);
    let header = row_text(&draw(&mut app), 0);
    assert!(header.contains("uncommitted"), "{header}");

    app.scope = herdr_gitview::git::Scope::Branch;
    app.base = "origin/next".to_string();
    let header = row_text(&draw(&mut app), 0);
    assert!(header.contains("vs origin/next"), "{header}");
}

#[test]
fn the_comparison_survives_a_narrow_pane_and_a_long_branch_name() {
    let mut app = app_with(vec![entry(
        "src/main.rs",
        ChangeKind::Modified,
        StageState::Unstaged,
        3,
        1,
    )]);
    app.scope = herdr_gitview::git::Scope::Branch;
    app.base = "origin/next".to_string();
    app.branch = Some("feature/a-very-long-branch-name-indeed".to_string());
    // The branch name is elided away; the comparison stays.
    let header = text_at(&draw_wide(&mut app, 28), 0, 28);
    assert!(header.contains("vs origin/next"), "{header}");
}

#[test]
fn a_wide_pane_also_says_how_many_commits_the_branch_scope_covers() {
    let mut app = app_with(vec![entry(
        "src/main.rs",
        ChangeKind::Modified,
        StageState::Unstaged,
        3,
        1,
    )]);
    app.scope = herdr_gitview::git::Scope::Branch;
    app.base = "origin/next".to_string();
    app.branch = Some("feat/x".to_string());
    app.branch_commits = Some(3);
    let header = text_at(&draw_wide(&mut app, 80), 0, 80);
    assert!(header.contains("3 commits"), "{header}");

    app.branch_commits = Some(1);
    let header = text_at(&draw_wide(&mut app, 80), 0, 80);
    assert!(header.contains("1 commit "), "{header}");
}

#[test]
fn the_footer_says_which_comparison_w_switches_to() {
    let mut app = app_with(vec![entry(
        "src/main.rs",
        ChangeKind::Modified,
        StageState::Unstaged,
        3,
        1,
    )]);
    app.base = "origin/next".to_string();
    let footer = text_at(&draw_wide(&mut app, 80), H - 1, 80);
    assert!(footer.contains("vs origin/next"), "{footer}");

    app.scope = herdr_gitview::git::Scope::Branch;
    let footer = text_at(&draw_wide(&mut app, 80), H - 1, 80);
    assert!(footer.contains("uncommitted"), "{footer}");
}
