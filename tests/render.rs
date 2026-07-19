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
        entry("src/main.rs", ChangeKind::Modified, StageState::Unstaged, 3, 1),
        entry("new.rs", ChangeKind::Added, StageState::Staged, 10, 0),
        entry("notes.txt", ChangeKind::Untracked, StageState::Untracked, 5, 0),
    ]);
    let buf = draw(&mut app);

    // Body rows start at y=1 (y=0 is the header).
    let r0 = row_text(&buf, 1);
    let r1 = row_text(&buf, 2);
    let r2 = row_text(&buf, 3);

    // Markers sit in column 1 (column 0 is the stage dot).
    assert_eq!(buf[(1, 1)].symbol(), "M");
    assert_eq!(buf[(1, 2)].symbol(), "A");
    assert_eq!(buf[(1, 3)].symbol(), "?");

    // Staged file shows a filled dot in column 0.
    assert_eq!(buf[(0, 2)].symbol(), "●");

    // Stats are present and right-aligned (row ends with them).
    assert!(r0.contains("+3 -1"), "row0: {r0:?}");
    assert!(r1.contains("+10 -0"), "row1: {r1:?}");
    assert!(r0.trim_end().ends_with("-1"), "row0: {r0:?}");
    assert!(r1.trim_end().ends_with("-0"), "row1: {r1:?}");
    assert!(r2.contains("+5 -0"), "row2: {r2:?}");

    // Basename is shown; directory prefix is dimmed but still present.
    assert!(r0.contains("main.rs"), "row0: {r0:?}");
}

#[test]
fn selected_row_is_reversed() {
    let mut app = app_with(vec![
        entry("a.rs", ChangeKind::Modified, StageState::Unstaged, 1, 0),
        entry("b.rs", ChangeKind::Modified, StageState::Unstaged, 1, 0),
    ]);
    app.cursor = 1;
    let buf = draw(&mut app);

    // Selected row is index 1 → body y=2.
    let selected = buf[(1, 2)].style().add_modifier.contains(Modifier::REVERSED);
    let other = buf[(1, 1)].style().add_modifier.contains(Modifier::REVERSED);
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
