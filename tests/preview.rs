//! Preview-pane behaviour + render tests. `PreviewApp` owns no terminal or
//! sockets, so we can drive it directly and render into a `TestBackend`.

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use unicode_width::UnicodeWidthStr;

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

// ---- note cards in the diff ------------------------------------------------

fn note(id: u64, start: u32, end: u32, text: &str) -> herdr_gitview::preview::app::Note {
    herdr_gitview::preview::app::Note {
        id,
        file: PathBuf::from("f.txt"),
        start,
        end,
        text: text.to_string(),
        snippet: String::new(),
        cached: false,
    }
}

/// A diff with `n` inserted lines plus the given notes, rendered once.
fn app_with_notes(notes: Vec<herdr_gitview::preview::app::Note>) -> PreviewApp {
    let mut a = app();
    let r = req("f.txt");
    a.begin_show(r.clone());
    a.apply_diff(&r, Ok(fake_diff(10)));
    a.notes = notes;
    a.apply_diff(&r, Ok(fake_diff(10))); // re-sync the doc with the notes
    draw(&mut a);
    a
}

#[test]
fn a_note_renders_as_a_boxed_card_under_its_line() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "use a constant")]);
    let buf = draw(&mut a);
    let body: Vec<String> = (1..H - 1).map(|y| row(&buf, y)).collect();

    let top = body
        .iter()
        .position(|l| l.contains('╭'))
        .expect("no card top");
    assert!(
        body[top].contains("note · line 3"),
        "title: {:?}",
        body[top]
    );
    assert!(
        body[top + 1].contains("use a constant"),
        "body: {:?}",
        body[top + 1]
    );
    assert!(
        body[top + 2].contains('╰'),
        "no card bottom: {:?}",
        body[top + 2]
    );
    // The card sits *under* the line it comments on.
    assert!(
        body[top - 1].contains("line 2"),
        "anchor: {:?}",
        body[top - 1]
    );
}

#[test]
fn a_multi_line_note_keeps_its_lines_inside_one_card() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "first thought\nsecond thought")]);
    let buf = draw(&mut a);
    let body: Vec<String> = (1..H - 1).map(|y| row(&buf, y)).collect();

    let top = body.iter().position(|l| l.contains('╭')).expect("no card");
    assert!(body[top + 1].contains("first thought"));
    assert!(body[top + 2].contains("second thought"));
    assert!(body[top + 3].contains('╰'), "one box around both lines");
    // ...and exactly one box.
    assert_eq!(body.iter().filter(|l| l.contains('╭')).count(), 1);
}

#[test]
fn an_empty_note_still_draws_a_box() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "")]);
    let buf = draw(&mut a);
    let body: Vec<String> = (1..H - 1).map(|y| row(&buf, y)).collect();
    let top = body.iter().position(|l| l.contains('╭')).expect("no card");
    assert!(body[top + 1].contains('│'));
    assert!(body[top + 2].contains('╰'));
}

#[test]
fn the_commented_line_gets_an_accented_gutter() {
    let mut a = app_with_notes(vec![note(1, 4, 4, "look here")]);
    let buf = draw(&mut a);
    // Find the row for new-line 4 and check its line-number cell is accented.
    let y = (1..H - 1)
        .find(|y| row(&buf, *y).contains("line 3")) // "line 3" is new-file line 4
        .expect("anchor row not visible");
    let accented = (0..8).any(|x| buf[(x, y)].style().fg == Some(ratatui::style::Color::Yellow));
    assert!(accented, "gutter of the commented line is not accented");
    // An uncommented row is not accented.
    let other = (1..H - 1)
        .find(|y| row(&buf, *y).contains("line 6"))
        .expect("other row");
    assert!(!(0..8).any(|x| buf[(x, other)].style().fg == Some(ratatui::style::Color::Yellow)));
}

#[test]
fn multi_line_cards_do_not_break_the_line_number_mapping() {
    // Every card line must be excluded from the doc→source mapping, or the
    // header and note anchors drift once a card is more than one line tall.
    let mut a = app_with_notes(vec![
        note(1, 2, 2, "one\ntwo\nthree"),
        note(2, 5, 5, "another\nmulti-line note"),
    ]);
    draw(&mut a);

    // Walk the whole doc: every line is either a card line (no source line)
    // or maps to a source line, and the source lines only ever increase.
    let mut last = 0;
    for i in 0..a.doc.lines.len() {
        a.cursor_line = i;
        if let Some(no) = a.cursor_file_line() {
            assert!(no >= last, "line numbers went backwards at doc line {i}");
            last = no;
        }
    }
    assert_eq!(last, 10, "should reach the last line of the file");
}

#[test]
fn cards_are_re_boxed_when_the_pane_width_changes() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "a note that is long enough to wrap")]);
    let wide = draw(&mut a);
    let wide_top = (1..H - 1)
        .map(|y| row(&wide, y))
        .find(|l| l.contains('╭'))
        .expect("no card");

    // Redraw into a narrower terminal.
    let mut term = Terminal::new(TestBackend::new(34, H)).unwrap();
    term.draw(|f| ui::render(f, &mut a)).unwrap();
    let narrow = term.backend().buffer().clone();
    let narrow_top = (1..H - 1)
        .map(|y| (0..34).map(|x| narrow[(x, y)].symbol()).collect::<String>())
        .find(|l| l.contains('╭'))
        .expect("no card when narrow");

    assert!(narrow_top.trim_end().width() < wide_top.trim_end().width());
    assert!(
        narrow_top.contains('╮'),
        "the box must still close: {narrow_top:?}"
    );
}

// ---- cards are decoration, not content -------------------------------------

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

fn press(a: &mut PreviewApp, c: char) {
    a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

/// Is the cursor sitting on a line that belongs to a note card?
fn on_card(a: &PreviewApp) -> bool {
    let line = &a.doc.lines[a.cursor_line];
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    text.contains('╭') || text.contains('│') || text.contains('╰')
}

#[test]
fn the_cursor_steps_over_a_card_instead_of_into_it() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "a\nmulti\nline\nnote")]);
    // Walk the whole document downwards, then back up.
    for _ in 0..a.doc.lines.len() + 2 {
        press(&mut a, 'j');
        assert!(
            !on_card(&a),
            "cursor landed inside a card at {}",
            a.cursor_line
        );
    }
    for _ in 0..a.doc.lines.len() + 2 {
        press(&mut a, 'k');
        assert!(
            !on_card(&a),
            "cursor landed inside a card at {}",
            a.cursor_line
        );
    }
}

#[test]
fn home_and_end_skip_cards_at_the_edges() {
    // A whole-file note puts a card at the very top of the document.
    let mut a = app_with_notes(vec![
        note(1, 0, 0, "whole file"),
        note(2, 10, 10, "last line"),
    ]);
    a.on_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    assert!(!on_card(&a), "home landed on the leading card");
    a.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert!(!on_card(&a), "end landed on the trailing card");
}

#[test]
fn a_card_appearing_under_the_cursor_pushes_it_off() {
    let mut a = app_with_notes(vec![]);
    for _ in 0..3 {
        press(&mut a, 'j');
    }
    let before = a.cursor_line;
    // A note lands right where the cursor is sitting.
    a.notes = vec![note(1, 3, 3, "new note\nwith two lines")];
    let r = req("f.txt");
    a.apply_diff(&r, Ok(fake_diff(10)));
    draw(&mut a);
    assert!(!on_card(&a), "cursor was left inside the new card");
    assert!(a.cursor_line >= before);
}

#[test]
fn dragging_across_a_card_does_not_land_on_it() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "one\ntwo\nthree")]);
    draw(&mut a);
    a.on_mouse(MouseEventKind::Down(MouseButton::Left), 1);
    for y in 2..9u16 {
        a.on_mouse(MouseEventKind::Drag(MouseButton::Left), y);
        assert!(!on_card(&a), "drag rested on a card at row {y}");
    }
}

#[test]
fn clicking_a_card_leaves_the_cursor_where_it_was() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "note text")]);
    draw(&mut a);
    a.on_mouse(MouseEventKind::Down(MouseButton::Left), 1);
    let before = a.cursor_line;
    // Row 4 is inside the card (line 1 anchor, then the card's three rows).
    a.on_mouse(MouseEventKind::Down(MouseButton::Left), 4);
    assert_eq!(a.cursor_line, before, "a click on a card must be inert");
}

#[test]
fn annotating_a_range_with_no_source_lines_is_refused() {
    let mut a = app_with_notes(vec![note(1, 2, 2, "existing")]);
    draw(&mut a);
    // Force the selection onto the card itself, which no key or click can do
    // — this is the guard of last resort for the annotate path.
    let card = (0..a.doc.lines.len())
        .find(|i| {
            let t: String = a.doc.lines[*i]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            t.contains('╭')
        })
        .unwrap();
    a.cursor_line = card;
    a.select_anchor = Some(card);
    press(&mut a, 'a');
    assert!(a.popup_request.is_none(), "should not open the note popup");
    assert_eq!(a.active_flash(), Some("select some code to annotate"));
}

#[test]
fn the_header_always_has_a_line_number_to_show() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "x\ny\nz")]);
    for _ in 0..a.doc.lines.len() {
        press(&mut a, 'j');
        assert!(
            a.cursor_file_line().is_some(),
            "no source line at doc line {}",
            a.cursor_line
        );
    }
}

#[test]
fn a_selection_spanning_a_card_leaves_the_card_untinted() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "one\ntwo")]);
    draw(&mut a);
    press(&mut a, 'v'); // start selecting at the top
    for _ in 0..6 {
        press(&mut a, 'j'); // extend down, across the card
    }
    let buf = draw(&mut a);

    let selected_bg = buf[(0, 1)].style().bg;
    let mut saw_card = false;
    for y in 1..H - 1 {
        let text = row(&buf, y);
        if text.contains('╭') || text.contains('│') || text.contains('╰') {
            saw_card = true;
            assert_ne!(
                buf[(0, y)].style().bg,
                selected_bg,
                "card row {y} is tinted as selected: {text:?}"
            );
        }
    }
    assert!(saw_card, "the card should be inside the selected range");
}

// ---- the inline composer ---------------------------------------------------

fn key(a: &mut PreviewApp, code: KeyCode, mods: KeyModifiers) {
    a.on_key(KeyEvent::new(code, mods));
}

fn type_text(a: &mut PreviewApp, text: &str) {
    for ch in text.chars() {
        key(a, KeyCode::Char(ch), KeyModifiers::NONE);
    }
}

fn body_lines(buf: &Buffer) -> Vec<String> {
    (1..H - 1).map(|y| row(buf, y)).collect()
}

/// An app showing a diff, with the cursor moved down `n` lines.
fn app_at(n: usize) -> PreviewApp {
    let mut a = app_with_notes(vec![]);
    for _ in 0..n {
        press(&mut a, 'j');
    }
    a
}

#[test]
fn annotate_opens_the_composer_inline_under_the_line() {
    let mut a = app_at(2);
    press(&mut a, 'a');
    assert!(a.composer.is_some(), "no composer");
    let buf = draw(&mut a);
    let body = body_lines(&buf);
    let top = body.iter().position(|l| l.contains('╭')).expect("no box");
    assert!(
        body[top].contains("new note · line 3"),
        "title: {:?}",
        body[top]
    );
    // ...directly under the line being commented on.
    assert!(
        body[top - 1].contains("line 2"),
        "anchor: {:?}",
        body[top - 1]
    );
}

#[test]
fn typing_in_the_composer_grows_the_box_and_saves_on_enter() {
    let mut a = app_at(1);
    press(&mut a, 'a');
    type_text(&mut a, "first");
    key(&mut a, KeyCode::Char('j'), KeyModifiers::CONTROL); // newline
    type_text(&mut a, "second");

    let body = body_lines(&draw(&mut a));
    assert!(body.iter().any(|l| l.contains("first")));
    assert!(body.iter().any(|l| l.contains("second")));

    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.composer.is_none(), "composer should close");
    assert_eq!(a.notes.len(), 1);
    assert_eq!(a.notes[0].text, "first\nsecond");
    // The saved note's card stands where the composer was.
    let body = body_lines(&draw(&mut a));
    assert!(body.iter().any(|l| l.contains("note · line")));
}

#[test]
fn every_key_reaches_the_composer_instead_of_the_keymap() {
    let mut a = app_at(1);
    press(&mut a, 'a');
    // j/k/q/v/d would all be actions outside the composer.
    type_text(&mut a, "jkqvd note");
    assert_eq!(
        a.composer.as_ref().map(|c| c.input.text().to_string()),
        Some("jkqvd note".to_string())
    );
    assert!(!a.should_quit, "q must not quit while composing");
}

#[test]
fn esc_cancels_without_leaving_a_note() {
    let mut a = app_at(1);
    press(&mut a, 'a');
    type_text(&mut a, "never mind");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.composer.is_none());
    assert!(a.notes.is_empty());
    assert!(a.pending_note.is_none());
    let body = body_lines(&draw(&mut a));
    assert!(!body.iter().any(|l| l.contains('╭')), "box left behind");
}

#[test]
fn saving_an_empty_composer_is_a_cancel() {
    let mut a = app_at(1);
    press(&mut a, 'a');
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.notes.is_empty(), "an empty note is not worth sending");
    assert!(a.composer.is_none());
}

#[test]
fn the_composer_edits_an_existing_note_in_place() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "original")]);
    assert!(a.begin_edit_note(1), "edit should open");
    assert_eq!(
        a.composer.as_ref().map(|c| c.input.text().to_string()),
        Some("original".to_string()),
        "prefilled with the note"
    );
    // The note's own card is hidden while its composer stands in for it.
    let body = body_lines(&draw(&mut a));
    assert_eq!(body.iter().filter(|l| l.contains('╭')).count(), 1);
    assert!(body.iter().any(|l| l.contains("edit note · line 3")));

    key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
    type_text(&mut a, "rewritten");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.notes.len(), 1, "edited, not duplicated");
    assert_eq!(a.notes[0].text, "rewritten");
}

#[test]
fn editing_a_note_from_another_file_is_declined() {
    let mut a = app_with_notes(vec![note(1, 3, 3, "elsewhere")]);
    a.notes[0].file = PathBuf::from("other.rs");
    assert!(!a.begin_edit_note(1), "should decline, not compose blindly");
    assert!(a.composer.is_none());
    assert!(!a.begin_edit_note(999), "unknown id");
}

#[test]
fn a_whole_file_note_composes_at_the_top() {
    let mut a = app_with_notes(vec![]);
    assert!(a.begin_file_note(PathBuf::from("f.txt"), false));
    let body = body_lines(&draw(&mut a));
    assert!(
        body[0].contains('╭'),
        "box should lead the diff: {:?}",
        body[0]
    );
    assert!(body[0].contains("new note · whole file"));

    type_text(&mut a, "about the file");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.notes[0].start, 0);
    assert_eq!(a.notes[0].end, 0);
    assert!(
        !a.begin_file_note(PathBuf::from("nope.rs"), false),
        "wrong file"
    );
}

#[test]
fn the_cursor_cannot_walk_into_the_composer_either() {
    let mut a = app_at(2);
    press(&mut a, 'a');
    type_text(&mut a, "a\nb\nc");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // After cancelling, walking the doc must still never touch a card.
    a.notes = vec![note(1, 3, 3, "x\ny")];
    let r = req("f.txt");
    a.apply_diff(&r, Ok(fake_diff(10)));
    draw(&mut a);
    for _ in 0..a.doc.lines.len() {
        press(&mut a, 'j');
        assert!(!on_card(&a));
    }
}
