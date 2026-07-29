//! Preview-pane behaviour + render tests. `PreviewApp` owns no terminal or
//! sockets, so we can drive it directly and render into a `TestBackend`.

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use herdr_gitview::config::Config;
use herdr_gitview::git::{ChangeKind, Repo, Scope};
use herdr_gitview::keymap::Keymap;
use herdr_gitview::preview::app::State;
use herdr_gitview::preview::highlight::Highlighter;
use herdr_gitview::preview::{PreviewApp, ShowReq, render, ui};

const W: u16 = 60;
const H: u16 = 12;

fn app() -> PreviewApp {
    let cfg = Config::default();
    let keys = Keymap::build(&HashMap::new()).unwrap();
    let repo = Repo {
        root: PathBuf::from("."),
    };
    PreviewApp::new(cfg, repo, keys)
}

fn req(file: &str) -> ShowReq {
    ShowReq {
        file: PathBuf::from(file),
        orig_path: None,
        scope: Scope::Worktree,
        cached: false,
        kind: ChangeKind::Modified,
        commit: None,
    }
}

/// A built diff doc with `n` inserted lines (plain-text path → no syntax
/// highlighting cost, even at 20k lines).
fn fake_diff(n: usize) -> render::DiffDoc {
    let hl = Highlighter::new(herdr_gitview::config::Theme::Dark);
    let new: String = (0..n).map(|i| format!("line {i}\n")).collect();
    render::build(
        &PathBuf::from("f.txt"),
        "",
        &new,
        &hl,
        herdr_gitview::config::Theme::Dark,
        3,
    )
}

/// A built doc for identical content (the "no changes" case).
fn empty_diff() -> render::DiffDoc {
    let hl = Highlighter::new(herdr_gitview::config::Theme::Dark);
    render::build(
        &PathBuf::from("f.txt"),
        "same\n",
        "same\n",
        &hl,
        herdr_gitview::config::Theme::Dark,
        3,
    )
}

fn draw(app: &mut PreviewApp) -> Buffer {
    let backend = TestBackend::new(W, H);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..W).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn splash_before_any_diff() {
    let mut a = app();
    let buf = draw(&mut a);
    let body: String = (1..H - 1).map(|y| row(&buf, y)).collect();
    assert!(body.contains("waiting for file list"), "body: {body:?}");
}

#[test]
fn shows_diff_and_header_path() {
    let mut a = app();
    let r = req("src/main.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(5)));

    let buf = draw(&mut a);
    let header = row(&buf, 0);
    assert!(header.contains("src/main.rs"), "header: {header:?}");
    assert!(header.contains("worktree"), "header: {header:?}");

    let body: String = (1..H - 1).map(|y| row(&buf, y)).collect();
    assert!(body.contains("line 0"), "body: {body:?}");
}

#[test]
fn stale_diff_result_is_dropped() {
    let mut a = app();
    let first = req("a.rs");
    a.begin_show(first.clone());
    // Selection moves on before the first diff returns.
    let second = req("b.rs");
    a.begin_show(second.clone());
    // Late result for the old request must be ignored.
    a.apply_diff(&first, Ok(fake_diff(3)));
    assert!(matches!(a.state, State::Splash(_) | State::Empty));

    // The current request's result is accepted.
    a.apply_diff(&second, Ok(fake_diff(3)));
    assert!(matches!(a.state, State::Diff));
}

#[test]
fn empty_diff_shows_no_changes() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(empty_diff()));
    assert!(matches!(a.state, State::Empty));

    let buf = draw(&mut a);
    let body: String = (1..H - 1).map(|y| row(&buf, y)).collect();
    assert!(body.contains("no changes"), "body: {body:?}");
}

#[test]
fn diff_error_shows_message() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Err("fatal: bad revision".to_string()));

    let buf = draw(&mut a);
    let body: String = (0..H).map(|y| row(&buf, y)).collect();
    assert!(body.contains("bad revision"), "body: {body:?}");
}

#[test]
fn scroll_is_clamped_and_bottom_jumps() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(30)));
    // Render once so the app learns its viewport height.
    draw(&mut a);

    a.scroll_by(-5); // can't go above the top
    assert_eq!(a.scroll, 0);

    a.scroll_by(i32::MAX); // jump to bottom
    let bottom = a.scroll;
    assert!(bottom > 0, "expected a scrollable diff");

    a.scroll_by(1000); // clamped at bottom
    assert_eq!(a.scroll, bottom);

    a.scroll_by(i32::MIN); // home
    assert_eq!(a.scroll, 0);
}

#[test]
fn truncation_notice_when_capped() {
    let mut a = app();
    let r = req("big.rs");
    a.begin_show(r.clone());
    // 20_001 content lines → over the 20k cap.
    a.apply_diff(&r, Ok(fake_diff(20_001)));
    a.scroll_by(i32::MAX);

    let buf = draw(&mut a);
    let body: String = (0..H).map(|y| row(&buf, y)).collect();
    assert!(body.contains("diff truncated"), "body: {body:?}");
}

// ---- "where am I": the cursor line ----------------------------------------

/// The background color of the first cell of body row `y`.
fn bg_at(buf: &Buffer, y: u16) -> ratatui::style::Color {
    buf[(0, y)]
        .style()
        .bg
        .unwrap_or(ratatui::style::Color::Reset)
}

/// The body row the cursor is tinted on, found by the odd-one-out background.
fn cursor_row(buf: &Buffer) -> Option<u16> {
    let rows: Vec<(u16, ratatui::style::Color)> = (1..H - 1).map(|y| (y, bg_at(buf, y))).collect();
    // The cursor tint is whichever background appears exactly once.
    rows.iter()
        .find(|(_, c)| rows.iter().filter(|(_, o)| o == c).count() == 1)
        .map(|(y, _)| *y)
}

#[test]
fn the_cursor_line_is_visibly_tinted() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(30)));
    let buf = draw(&mut a);

    assert_eq!(cursor_row(&buf), Some(1), "cursor starts on the first line");
    // and it is a real tint, not the default background
    assert_ne!(bg_at(&buf, 1), ratatui::style::Color::Reset);
    assert_ne!(bg_at(&buf, 1), bg_at(&buf, 2), "cursor row must stand out");
}

#[test]
fn the_cursor_stays_on_screen_when_the_diff_is_scrolled() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(60)));
    draw(&mut a); // learn the viewport height

    // Scrolling from the list pane (ctrl+d / wheel) used to leave the cursor
    // behind, so the diff pane showed no cursor at all.
    a.scroll_by(20);
    assert!(
        a.cursor_line >= a.scroll as usize,
        "cursor above the viewport"
    );
    assert!(
        a.cursor_line < a.scroll as usize + a.viewport_h as usize,
        "cursor below the viewport"
    );
    let buf = draw(&mut a);
    assert!(
        cursor_row(&buf).is_some(),
        "no cursor visible after scrolling"
    );

    // ...including a jump to the very bottom, and back to the top.
    a.scroll_by(i32::MAX);
    assert!(a.cursor_line >= a.scroll as usize);
    assert!(draw(&mut a).area.height > 0 && cursor_row(&draw(&mut a)).is_some());

    a.scroll_by(i32::MIN);
    assert!(a.cursor_line < a.viewport_h as usize);
    assert!(cursor_row(&draw(&mut a)).is_some());
}

#[test]
fn scrolling_does_not_drag_the_cursor_through_a_live_selection() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(60)));
    draw(&mut a);

    a.on_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)); // start selecting
    let anchored = a.cursor_line;
    a.scroll_by(30);
    assert_eq!(
        a.cursor_line, anchored,
        "scrolling must not silently extend the selection"
    );
}

#[test]
fn the_header_reports_the_cursor_line_not_the_scroll_offset() {
    let mut a = app();
    let r = req("x.rs");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(40)));
    draw(&mut a);

    let header = row(&draw(&mut a), 0);
    assert!(header.contains("ln 1"), "header: {header:?}");

    // Move the cursor down a few lines; the header follows it.
    for _ in 0..5 {
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
    }
    let header = row(&draw(&mut a), 0);
    assert!(header.contains("ln 6"), "header: {header:?}");
    assert_eq!(a.cursor_file_line(), Some(6));
}
