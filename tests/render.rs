//! Render tests: draw the list pane into a ratatui `TestBackend` at 40×12 and
//! assert on the resulting buffer (marker chars, stats alignment, selected-row
//! styling, empty state). No git or terminal needed — `App::from_entries`
//! takes a preloaded entry vector.

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

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
    // · y=4 main.rs · y=5 notes.txt
    assert!(row_text(&buf, 1).contains("STAGED CHANGES"));
    let staged = row_text(&buf, 2);
    assert!(row_text(&buf, 3).contains("CHANGES"));
    let modified = row_text(&buf, 4);
    let untracked = row_text(&buf, 5);

    // Markers sit after the 2-column section indent.
    assert_eq!(buf[(2, 2)].symbol(), "A");
    assert_eq!(buf[(2, 4)].symbol(), "M");
    assert_eq!(buf[(2, 5)].symbol(), "U");

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

    let selected = buf[(1, 3)]
        .style()
        .add_modifier
        .contains(Modifier::REVERSED);
    let other = buf[(1, 2)]
        .style()
        .add_modifier
        .contains(Modifier::REVERSED);
    assert!(selected, "selected row should be REVERSED");
    assert!(!other, "non-selected row should not be REVERSED");
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
