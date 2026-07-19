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
use herdr_gitview::preview::ui;
use herdr_gitview::preview::{PreviewApp, ShowReq};

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
    }
}

/// A colorized diff big enough to scroll (30 numbered lines).
fn fake_diff(n: usize) -> Vec<u8> {
    let mut s = String::from("\x1b[1mdiff --git a/f b/f\x1b[0m\n");
    for i in 0..n {
        s.push_str(&format!("\x1b[32m+line {i}\x1b[0m\n"));
    }
    s.into_bytes()
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
    a.apply_diff(&r, Ok(Vec::new()));
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
