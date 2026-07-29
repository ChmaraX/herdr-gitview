//! In-process scenario tests: both panes' real `Session`s wired over a real
//! unix socket pair, against a real git repo, pumped deterministically.
//! Only the terminal is absent — keys are injected as events, herdr is a
//! fake recording binary, and the PTY editor is a recording host.
//!
//! This is the layer where the wiring bugs live (stale probe results, popup
//! lifecycle, Show/Clear ordering), so several tests here are regression
//! tests for previously shipped bugs.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyEvent, KeyModifiers};

use common::{FakeHerdr, TempRepo, fixture, write};
use herdr_gitview::config::Config;
use herdr_gitview::git::Repo;
use herdr_gitview::hostenv::HostEnv;
use herdr_gitview::ipc::Conn;
use herdr_gitview::keymap::{Keymap, parse_key};
use herdr_gitview::list;
use herdr_gitview::list::app::Mode;
use herdr_gitview::preview;
use herdr_gitview::preview::app::State;

/// Records editor invocations instead of touching a PTY.
#[derive(Default)]
struct RecordingEditor {
    runs: Vec<Vec<String>>,
}

impl preview::EditorHost for RecordingEditor {
    fn run(
        &mut self,
        _cwd: &std::path::Path,
        argv: &[String],
        _envs: &[(String, String)],
    ) -> anyhow::Result<bool> {
        self.runs.push(argv.to_vec());
        Ok(true)
    }
}

/// Both panes, wired and pumpable.
struct World {
    repo: TempRepo,
    /// Host artifacts (fake herdr, socket, answer files) — kept *outside*
    /// the repo so they never show up as untracked entries.
    host_dir: PathBuf,
    herdr: FakeHerdr,
    list: list::Session,
    list_rx: Receiver<list::Event>,
    preview: preview::Session,
    preview_rx: Receiver<preview::Event>,
    editor: RecordingEditor,
    socket_base: PathBuf,
}

impl World {
    fn new(repo: TempRepo) -> World {
        let host_dir = repo.dir.parent().unwrap().join(format!(
            "{}-host",
            repo.dir.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&host_dir).unwrap();
        let herdr = FakeHerdr::install(&host_dir);
        let socket_base = host_dir.join("view.sock");
        let env = |own: &str, preview_pane: Option<&str>| HostEnv {
            herdr_bin: herdr.bin.clone().into_os_string(),
            own_pane: Some(own.to_string()),
            preview_pane: preview_pane.map(str::to_string),
            socket: Some(socket_base.clone()),
        };

        let cfg = Config::default();
        let keys = Keymap::build(&HashMap::new()).unwrap();
        let list_app = list::App::new(
            Repo {
                root: repo.dir.clone(),
            },
            cfg.clone(),
            keys,
        )
        .unwrap();
        let (list_tx, list_rx) = mpsc::channel();
        let mut list = list::Session::new(list_app, env("w:pLIST", Some("w:pPREV")), list_tx, true);
        list.show_debounce = Duration::ZERO;
        list.set_popup_liveness(Duration::ZERO);

        let keys = Keymap::build(&HashMap::new()).unwrap();
        let preview_app = herdr_gitview::preview::PreviewApp::new(
            cfg,
            Repo {
                root: repo.dir.clone(),
            },
            keys,
        );
        let (preview_tx, preview_rx) = mpsc::channel();
        let mut preview = preview::Session::new(preview_app, env("w:pPREV", None), preview_tx);
        preview.set_popup_liveness(Duration::ZERO);

        // Wire the two panes with a real socket pair.
        let (a, b) = Conn::pair().unwrap();
        list.on_event(list::Event::Connected(a));
        let mut world = World {
            repo,
            host_dir,
            herdr,
            list,
            list_rx,
            preview,
            preview_rx,
            editor: RecordingEditor::default(),
            socket_base,
        };
        world
            .preview
            .on_event(preview::Event::Connected(b), &mut world.editor);
        world.pump();
        world
    }

    /// Drain both event channels + run both ticks until nothing moves for a
    /// quiet window (worker threads deliver asynchronously), with a deadline.
    fn pump(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut quiet_since = Instant::now();
        loop {
            let mut moved = false;
            while let Ok(ev) = self.list_rx.try_recv() {
                self.list.on_event(ev);
                moved = true;
            }
            while let Ok(ev) = self.preview_rx.try_recv() {
                self.preview.on_event(ev, &mut self.editor);
                moved = true;
            }
            self.list.tick();
            self.preview.tick();

            if moved {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() > Duration::from_millis(150) {
                return; // quiescent
            }
            assert!(Instant::now() < deadline, "pump did not quiesce");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn press(&mut self, key: &str) {
        let (code, mods) = parse_key(key).unwrap();
        self.list
            .on_event(list::Event::Key(KeyEvent::new(code, mods)));
        self.pump();
    }

    /// The path of the file whose diff the preview currently shows.
    fn shown_file(&self) -> Option<String> {
        self.preview
            .app
            .current
            .as_ref()
            .map(|req| req.file.display().to_string())
    }

    /// Plain text of the preview's rendered diff doc.
    fn diff_text(&self) -> String {
        self.preview
            .app
            .doc
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect()
    }

    /// Answer file path for a popup entrypoint (mirrors `Popups::open`).
    /// Type into the diff pane's inline note composer and save with enter.
    /// Mirrors what the user does once the composer has taken the focus.
    fn compose(&mut self, text: &str) {
        // Opening the composer is a round trip: the list sends over the link
        // and the diff pane acts on it, so wait for it rather than assuming
        // one pump delivered it.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.preview.app.composer.is_none() && Instant::now() < deadline {
            self.pump();
        }
        assert!(
            self.preview.app.composer.is_some(),
            "no composer opened in the diff pane (flash: {:?})",
            self.preview.app.active_flash(),
        );
        for ch in text.chars() {
            self.preview.on_event(
                preview::Event::Key(KeyEvent::new(
                    crossterm::event::KeyCode::Char(ch),
                    KeyModifiers::NONE,
                )),
                &mut self.editor,
            );
        }
        self.preview.on_event(
            preview::Event::Key(KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                KeyModifiers::NONE,
            )),
            &mut self.editor,
        );
        self.pump();
    }

    fn answer_popup(&mut self, entrypoint: &str, answer: &str) {
        let path = self
            .socket_base
            .with_extension(format!("{entrypoint}.answer"));
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, answer).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        self.pump();
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.host_dir);
    }
}

fn key(code: char) -> KeyEvent {
    KeyEvent::new(crossterm::event::KeyCode::Char(code), KeyModifiers::NONE)
}

// ---------------------------------------------------------------------------

#[test]
fn browsing_shows_each_files_diff() {
    let repo = fixture("browse");
    write(&repo.dir, "alpha.txt", "added alpha\n");
    write(&repo.dir, "base.txt", "one\ntwo\nthree added\n");
    let mut w = World::new(repo);

    // Two entries under CHANGES; the first selection previews automatically.
    assert_eq!(w.shown_file().as_deref(), Some("alpha.txt"));
    assert!(matches!(w.preview.app.state, State::Diff));
    assert!(w.diff_text().contains("added alpha"), "{}", w.diff_text());

    w.press("j");
    assert_eq!(w.shown_file().as_deref(), Some("base.txt"));
    assert!(w.diff_text().contains("three added"));
}

#[test]
fn staging_moves_the_file_and_previews_the_staged_diff() {
    let repo = fixture("stage");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    assert_eq!(w.shown_file().as_deref(), Some("base.txt"));
    let staged_rows = |w: &World| {
        w.list
            .app
            .rows
            .iter()
            .filter(|r| matches!(r, list::app::ListRow::Entry { staged: true, .. }))
            .count()
    };
    assert_eq!(staged_rows(&w), 0);

    w.press("s");
    // The file moved to the staged section, and the re-Show carries cached.
    assert_eq!(staged_rows(&w), 1);
    let req = w.preview.app.current.as_ref().unwrap();
    assert!(req.cached, "staged section selection previews --cached");

    w.press("u"); // explicit unstage moves it back
    assert_eq!(staged_rows(&w), 0);
}

#[test]
fn discarding_the_last_change_clears_the_preview() {
    let repo = fixture("discard");
    write(&repo.dir, "loose.txt", "scratch\n");
    let mut w = World::new(repo);
    // Answer the confirm through the popup flow (fake herdr opened it).
    w.press("x");
    w.answer_popup("ask", "y");

    assert!(w.list.app.entries.is_empty(), "entry discarded");
    assert!(
        matches!(w.preview.app.state, State::Splash(_)),
        "preview cleared instead of showing a stale diff"
    );
    assert!(!w.repo.dir.join("loose.txt").exists());
}

#[test]
fn note_flow_annotate_edit_delete_syncs_both_panes() {
    let repo = fixture("notes");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    // Whole-file note from the list: `a` hands off to the diff pane's
    // inline composer, which is where every note is written.
    w.press("a");
    w.compose("please refactor this");
    assert_eq!(w.preview.app.notes.len(), 1);
    assert_eq!(w.list.app.notes.len(), 1, "snapshot synced to the list");
    assert!(
        w.diff_text().contains("please refactor this"),
        "note card rendered"
    );

    // Notes view, edit the note: the composer reopens prefilled.
    w.press("n");
    assert_eq!(w.list.app.mode, Mode::Notes);
    w.list.on_event(list::Event::Key(KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    w.pump();
    assert_eq!(
        w.preview
            .app
            .composer
            .as_ref()
            .map(|c| c.input.text().to_string()),
        Some("please refactor this".to_string()),
        "the composer opens prefilled with the note being edited"
    );
    // Clear it and type the replacement.
    w.preview.on_event(
        preview::Event::Key(KeyEvent::new(
            crossterm::event::KeyCode::Char('u'),
            crossterm::event::KeyModifiers::CONTROL,
        )),
        &mut w.editor,
    );
    w.compose("actually delete it");
    assert_eq!(w.list.app.notes[0].text, "actually delete it");

    // Delete it; the empty notes view returns to files automatically.
    w.press("d");
    assert!(w.preview.app.notes.is_empty());
    assert_eq!(w.list.app.mode, Mode::Files);
}

#[test]
fn staged_note_notes_view_previews_the_staged_diff() {
    // Regression: a whole-file note added on a *staged* file used to force
    // the notes-view preview to `cached: false` (worktree side), which is
    // empty for a fully staged file — "no changes in this view" even though
    // the note and its diff both exist.
    let repo = fixture("staged-note");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    w.press("s"); // stage the only change
    let staged_rows = w
        .list
        .app
        .rows
        .iter()
        .filter(|r| matches!(r, list::app::ListRow::Entry { staged: true, .. }))
        .count();
    assert_eq!(staged_rows, 1);

    // Whole-file note from the list, on the now-staged file.
    w.press("a");
    w.compose("please refactor this");
    assert_eq!(w.preview.app.notes.len(), 1);
    assert!(w.preview.app.notes[0].cached, "note remembers staged side");

    // Open the notes view and hover the note — the preview must re-show the
    // staged diff, not an empty worktree one.
    w.press("n");
    assert_eq!(w.list.app.mode, Mode::Notes);
    w.pump();
    assert!(
        !matches!(w.preview.app.state, State::Empty),
        "staged diff should render, not 'no changes in this view'"
    );
    assert!(w.diff_text().contains("changed"));
}

#[test]
fn dead_popup_cancels_instead_of_wedging() {
    // Regression: a popup pane dying without an answer used to permanently
    // wedge the popup subsystem (invisible modal eating keys).
    let repo = fixture("dead-popup");
    write(&repo.dir, "loose.txt", "scratch\n");
    let mut w = World::new(repo);

    w.press("x"); // confirm popup opens (external modal)
    assert!(w.list.app.modal.is_some());
    w.herdr.kill_popup(); // popup pane dies without answering
    std::thread::sleep(Duration::from_millis(20));
    w.pump();

    assert!(w.list.app.modal.is_none(), "modal cancelled, not wedged");
    assert!(!w.list.app.modal_external);
    assert!(
        w.repo.dir.join("loose.txt").exists(),
        "nothing was discarded"
    );

    // And the subsystem still works: a new confirm opens + answers fine.
    std::fs::remove_file(&w.herdr.dead_marker).unwrap();
    w.press("x");
    w.answer_popup("ask", "n");
    assert!(w.list.app.modal.is_none());
    assert!(w.repo.dir.join("loose.txt").exists());
}

#[test]
fn stale_probe_result_cannot_fire_after_editdone() {
    // Regression: EditDone winning the race against the editor probe used to
    // leave a stale `after_edit` that quit the view much later.
    let repo = fixture("probe-race");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    w.list.app.busy = Some("editing…".to_string());
    // EditDone arrives first (nvim exited on its own)…
    w.list
        .on_event(list::Event::Ipc(herdr_gitview::ipc::ToList::EditDone {
            file: PathBuf::from("base.txt"),
        }));
    // …then the probe result lands, carrying a quit intention.
    w.list.on_event(list::Event::EditorProbe {
        then: Some(list::app::EditorThen::QuitView),
        unsaved: Some(false),
    });
    w.pump();

    assert!(w.list.app.after_edit.is_none(), "stale probe discarded");
    assert!(!w.list.app.should_quit, "no spontaneous quit");
}

#[test]
fn enter_runs_the_editor_and_editdone_unlocks() {
    let repo = fixture("editor");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    w.list.on_event(list::Event::Key(KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    w.pump();

    // The recording editor ran with the file (and a +line jump).
    assert_eq!(w.editor.runs.len(), 1);
    let argv = &w.editor.runs[0];
    assert!(argv.iter().any(|a| a.ends_with("base.txt")), "{argv:?}");
    assert!(
        argv.iter().any(|a| a.starts_with('+')),
        "jumps to the change"
    );
    // EditDone round-tripped: the lockout is released again.
    assert!(w.list.app.busy.is_none(), "unlocked after EditDone");

    // Focus handoffs went through the fake herdr.
    let calls = w.herdr.calls().join("\n");
    assert!(calls.contains("plugin pane focus w:pPREV"), "{calls}");
    assert!(calls.contains("plugin pane focus w:pLIST"), "{calls}");
}

#[test]
fn quit_hands_shakes_both_panes_down() {
    let repo = fixture("quit");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    let mut w = World::new(repo);

    w.list.on_event(list::Event::Key(key('q')));
    w.pump();

    assert!(w.list.should_quit());
    assert!(w.preview.should_quit(), "preview received Quit over IPC");
}
