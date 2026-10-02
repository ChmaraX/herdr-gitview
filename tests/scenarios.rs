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
    list_tx: mpsc::Sender<list::Event>,
    preview: preview::Session,
    preview_rx: Receiver<preview::Event>,
    editor: RecordingEditor,
    socket_base: PathBuf,
}

impl World {
    fn new(repo: TempRepo) -> World {
        World::with_config(repo, Config::default())
    }

    fn with_config(repo: TempRepo, cfg: Config) -> World {
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
        let mut list = list::Session::new(
            list_app,
            env("w:pLIST", Some("w:pPREV")),
            list_tx.clone(),
            true,
        );
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
            list_tx,
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

    /// Feed the preview whatever its worker delivers for `dur`, without
    /// waiting for quiet — for measuring latency while keys keep coming.
    fn drain_preview_for(&mut self, dur: Duration) {
        let until = Instant::now() + dur;
        loop {
            while let Ok(ev) = self.preview_rx.try_recv() {
                self.preview.on_event(ev, &mut self.editor);
            }
            self.preview.tick();
            if Instant::now() >= until {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Wait until every line on the preview's screen carries its colors.
    fn wait_colored(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.preview.app.highlight_pending() {
            assert!(Instant::now() < deadline, "highlighting never finished");
            self.drain_preview_for(Duration::from_millis(2));
        }
    }

    /// Wait until the preview has had `shows` Shows and the newest is on
    /// screen with all its colors.
    fn wait_landed(&mut self, shows: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.preview.timings.len() < shows
            || self
                .preview
                .timings
                .last()
                .is_none_or(|t| t.colored.is_none())
        {
            assert!(Instant::now() < deadline, "the last Show never landed");
            self.drain_preview_for(Duration::from_millis(2));
        }
    }

    /// Distinct foreground colors in the preview's rendered doc.
    fn diff_colors(&self) -> usize {
        let colors: std::collections::HashSet<_> = self
            .preview
            .app
            .doc
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().filter_map(|s| s.style.fg))
            .collect();
        colors.len()
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
            .filter(|r| {
                matches!(
                    r,
                    list::app::ListRow::Entry {
                        section: list::app::Section::Staged,
                        ..
                    }
                )
            })
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

/// Regression: `ComposeNote` used to carry only a path and require the diff
/// pane to already be showing it, but the list's `Show` is debounced and was
/// flushed *after* the compose dispatch in the same tick. Moving the cursor
/// and annotating inside that window failed with "open that file's diff
/// first" — and stole the focus on the way.
#[test]
fn annotating_immediately_after_moving_still_opens_the_composer() {
    let repo = fixture("annotate-after-move");
    write(&repo.dir, "base.txt", "one\ntwo\nchanged\n");
    write(&repo.dir, "second.txt", "brand new\n");
    let mut w = World::new(repo);
    // A realistic debounce, not the zero the other scenarios use.
    w.list.show_debounce = Duration::from_millis(40);
    w.pump();

    // Move to the other file and annotate without waiting for the Show.
    w.list.on_event(list::Event::Key(KeyEvent::new(
        crossterm::event::KeyCode::Char('j'),
        KeyModifiers::NONE,
    )));
    w.list.on_event(list::Event::Key(KeyEvent::new(
        crossterm::event::KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    w.compose("a note on the file I just moved to");

    assert_eq!(w.preview.app.notes.len(), 1);
    assert_eq!(
        w.preview.app.notes[0].file,
        PathBuf::from("second.txt"),
        "the note landed on the wrong file"
    );
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
        .filter(|r| {
            matches!(
                r,
                list::app::ListRow::Entry {
                    section: list::app::Section::Staged,
                    ..
                }
            )
        })
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

/// Holding `j` through a stack of large TypeScript files must not queue up
/// a full rebuild per file: the diff for the file the cursor stops on lands
/// promptly, and every Show's latency is written to `target/gitview-perf.txt`
/// so a regression is visible as numbers, not as a vague "feels laggy".
#[test]
fn rapid_browsing_through_large_files_lands_the_final_diff() {
    const FILES: usize = 11;
    let big = include_str!("fixtures/large.ts");
    let repo = fixture("rapid-browse");
    let name = |i: usize| format!("big{i:02}.ts");
    // One more file below the last one browsed to: only a prefetch ever
    // builds it.
    for i in 0..=FILES {
        write(&repo.dir, &name(i), big);
    }
    common::git(&repo.dir, &["add", "."]);
    common::git(&repo.dir, &["commit", "-q", "-m", "big files"]);
    // Three scattered edits per file, each carrying its file's marker.
    for i in 0..=FILES {
        let mut lines: Vec<String> = big.lines().map(str::to_string).collect();
        let n = lines.len();
        for at in [n / 4, n / 2, 3 * n / 4] {
            lines[at].push_str(&format!(" // edit in file {i}"));
        }
        write(&repo.dir, &name(i), &(lines.join("\n") + "\n"));
    }
    let mut w = World::new(repo);
    assert_eq!(w.shown_file().as_deref(), Some("big00.ts"));

    // Ten presses at key-repeat speed, the preview draining as it goes.
    for _ in 0..FILES - 1 {
        w.list.on_event(list::Event::Key(key('j')));
        w.list.tick();
        w.drain_preview_for(Duration::from_millis(30));
    }
    let last_press = Instant::now();
    let marker = format!("edit in file {}", FILES - 1);
    while !(w.shown_file() == Some(name(FILES - 1))
        && matches!(w.preview.app.state, State::Diff)
        && w.diff_text().contains(&marker))
    {
        assert!(
            last_press.elapsed() < Duration::from_secs(60),
            "the final file's diff never landed"
        );
        w.drain_preview_for(Duration::from_millis(2));
    }
    let landed = last_press.elapsed();
    w.wait_colored();
    let colored = last_press.elapsed();
    assert!(
        w.diff_colors() > 3,
        "the landed diff carries syntax colors, not one plain color"
    );

    // Expanding the leading fold reveals uncolored lines; they get colored
    // too, without rebuilding the doc (the cursor stays put).
    let before = w.preview.app.doc.lines.len();
    w.preview.app.on_mouse(
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        1,
    );
    assert!(w.preview.app.doc.lines.len() > before, "the fold expanded");
    assert!(
        w.preview.app.highlight_pending(),
        "revealed lines start plain"
    );
    w.wait_colored();

    // Step back and forth: the file already built comes back colored in
    // one paint, from the worker's cache.
    for k in ['k', 'j'] {
        let shows = w.preview.timings.len();
        w.list.on_event(list::Event::Key(key(k)));
        w.list.tick();
        w.wait_landed(shows + 1);
    }
    let revisit = w.preview.timings.last().unwrap();
    assert_eq!(revisit.file, PathBuf::from(name(FILES - 1)));
    assert!(
        revisit.first_paint.is_some() && revisit.first_paint == revisit.colored,
        "a revisited file paints colored at once: {revisit:?}"
    );

    // Resting on big10 has its never-visited neighbor big11 built in the
    // background, so stepping onto it paints colored at once too.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !w
        .preview
        .prefetches
        .iter()
        .any(|(f, _)| f.as_os_str() == name(FILES).as_str())
    {
        assert!(
            Instant::now() < deadline,
            "the neighbor was never prefetched"
        );
        w.list.tick(); // the settle timer runs in the list's tick
        w.drain_preview_for(Duration::from_millis(5));
    }
    let shows = w.preview.timings.len();
    w.list.on_event(list::Event::Key(key('j')));
    w.list.tick();
    w.wait_landed(shows + 1);
    let stepped = w.preview.timings.last().unwrap();
    assert_eq!(stepped.file, PathBuf::from(name(FILES)));
    assert!(
        stepped.first_paint == stepped.colored,
        "a prefetched neighbor paints colored at once: {stepped:?}"
    );

    let mut report = format!(
        "rapid browse: {} files x {} lines, {} j presses\nfinal diff landed {landed:?} after the last press, colored after {colored:?}\n",
        FILES,
        big.lines().count(),
        FILES - 1
    );
    for t in &w.preview.timings {
        let paint = t
            .first_paint
            .map_or("superseded".to_string(), |d| format!("{d:?}"));
        let colored = t.colored.map_or("-".to_string(), |d| format!("{d:?}"));
        report.push_str(&format!(
            "{}\tfirst paint {paint}\tcolored {colored}\n",
            t.file.display()
        ));
    }
    for (file, took) in &w.preview.prefetches {
        report.push_str(&format!("{}\tprefetched in {took:?}\n", file.display()));
    }
    let artifact = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .unwrap()
        .join("gitview-perf.txt");
    std::fs::write(&artifact, &report).unwrap();
    eprintln!("{report}(written to {})", artifact.display());
}

/// Every kind of change diffs the right pair of sides, and a doc served
/// again from the worker's cache never outlives an edit to its file.
#[test]
fn each_change_kind_previews_its_own_sides_and_edits_bypass_the_cache() {
    use herdr_gitview::git::ChangeKind;
    use herdr_gitview::ipc::ToPreview;

    let repo = fixture("sides");
    let dir = repo.dir.clone();
    let commit = |msg: &str| {
        common::git(&dir, &["add", "-A"]);
        common::git(&dir, &["commit", "-q", "-m", msg]);
    };
    // A conflict: main and feature both rewrite c.txt.
    write(&dir, "c.txt", "base line\n");
    write(&dir, "staged.txt", "staged before\n");
    write(&dir, "r.txt", "rename me\nkeep\n");
    write(&dir, "d.txt", "doomed\n");
    commit("seed");
    common::git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "c.txt", "theirs line\n");
    commit("theirs");
    let theirs_sha = {
        let out = std::process::Command::new("git")
            .args(["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    common::git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "c.txt", "ours line\n");
    commit("ours");
    common::git_lenient(&dir, &["merge", "-q", "feature"]);
    // The rest, on top of the conflicted merge.
    write(&dir, "base.txt", "one\ntwo unstaged\n");
    write(&dir, "staged.txt", "staged after\n");
    common::git(&dir, &["add", "staged.txt"]);
    common::git(&dir, &["mv", "r.txt", "r2.txt"]);
    write(&dir, "r2.txt", "renamed\nkeep\n");
    common::git(&dir, &["add", "r2.txt"]);
    write(&dir, "r2.txt", "renamed\nkeep\nthen edited\n"); // RM: more after staging
    std::fs::remove_file(dir.join("d.txt")).unwrap();
    write(&dir, "u.txt", "brand new\n");
    let mut w = World::new(repo);

    let show = |w: &mut World,
                file: &str,
                orig: Option<&str>,
                cached,
                kind,
                commit: Option<&str>| {
        let msg = ToPreview::Show(herdr_gitview::ipc::ShowReq {
            commit: commit.map(str::to_string),
            ..herdr_gitview::ipc::ShowReq::worktree(file.into(), orig.map(Into::into), cached, kind)
        });
        w.preview.on_event(preview::Event::Ipc(msg), &mut w.editor);
        w.wait_colored();
        w.pump();
        w.diff_text()
    };
    let has = |text: &str, want: &[&str], not: &[&str]| {
        for s in want {
            assert!(text.contains(s), "missing {s:?} in {text:?}");
        }
        for s in not {
            assert!(!text.contains(s), "unexpected {s:?} in {text:?}");
        }
    };

    // Unstaged: index -> worktree.
    let t = show(&mut w, "base.txt", None, false, ChangeKind::Modified, None);
    has(&t, &["two", "two unstaged"], &[]);
    // Staged: HEAD -> index.
    let t = show(&mut w, "staged.txt", None, true, ChangeKind::Modified, None);
    has(&t, &["staged before", "staged after"], &[]);
    // Staged rename: HEAD:old path -> index:new path.
    let t = show(
        &mut w,
        "r2.txt",
        Some("r.txt"),
        true,
        ChangeKind::Renamed,
        None,
    );
    has(&t, &["rename me", "renamed", "keep"], &["then edited"]);
    // Unstaged side of that rename: index (new path) -> worktree, so only
    // the later edit is a change — not the whole file against nothing.
    let t = show(
        &mut w,
        "r2.txt",
        Some("r.txt"),
        false,
        ChangeKind::Renamed,
        None,
    );
    has(&t, &["then edited"], &["rename me"]);
    assert!(
        !w.diff_text().contains("▌   1 renamed"),
        "line 1 is unchanged context, not an insertion: {t:?}"
    );
    // Deleted: everything removed, nothing added.
    let t = show(&mut w, "d.txt", None, false, ChangeKind::Deleted, None);
    has(&t, &["doomed"], &[]);
    assert!(matches!(w.preview.app.state, State::Diff));
    // Untracked: all added against nothing.
    let t = show(&mut w, "u.txt", None, false, ChangeKind::Untracked, None);
    has(&t, &["brand new"], &[]);
    // Conflicted: ours (stage 2) -> the worktree's conflict markers.
    let t = show(&mut w, "c.txt", None, false, ChangeKind::Conflicted, None);
    has(&t, &["ours line", "<<<<<<<", "theirs line"], &["base line"]);
    // A commit: its parent -> it.
    let t = show(
        &mut w,
        "c.txt",
        None,
        false,
        ChangeKind::Modified,
        Some(&theirs_sha),
    );
    has(&t, &["base line", "theirs line"], &["ours line", "<<<<<<<"]);

    // Back to base.txt (a cache hit), then a same-size edit: the edit wins.
    let t = show(&mut w, "base.txt", None, false, ChangeKind::Modified, None);
    has(&t, &["two unstaged"], &[]);
    write(&dir, "base.txt", "one\ntwo UNSTAGED\n");
    let t = show(&mut w, "u.txt", None, false, ChangeKind::Untracked, None);
    has(&t, &["brand new"], &[]);
    let t = show(&mut w, "base.txt", None, false, ChangeKind::Modified, None);
    has(&t, &["two UNSTAGED"], &["two unstaged"]);
}

/// A pinned `base` is what both panes compare against and name — the diff
/// pane used to auto-detect its own base for the header and ignore the pin.
/// Resolving it doesn't block the list: `w` returns at once and the scope
/// switches when the resolution lands.
#[test]
fn a_pinned_base_is_resolved_off_the_ui_thread_and_named_by_both_panes() {
    let repo = fixture("pinned-base");
    let dir = repo.dir.clone();
    // feature is stacked on release, so auto-detection would pick release;
    // the pin says main, which brings release's own commit into the diff.
    common::git(&dir, &["checkout", "-q", "-b", "release"]);
    write(&dir, "r.txt", "from release\n");
    common::git(&dir, &["add", "."]);
    common::git(&dir, &["commit", "-q", "-m", "release work"]);
    common::git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "f.txt", "from feature\n");
    common::git(&dir, &["add", "."]);
    common::git(&dir, &["commit", "-q", "-m", "feature work"]);
    let cfg = Config {
        base: "main".to_string(),
        ..Config::default()
    };
    let mut w = World::with_config(repo, cfg);

    w.list.on_event(list::Event::Key(key('w')));
    w.list.tick();
    assert_eq!(
        w.list.app.scope,
        herdr_gitview::git::Scope::Worktree,
        "the key returns before the base is resolved"
    );
    assert!(
        matches!(w.list.app.base_job, list::app::BaseJob::Running { .. }),
        "resolving on the session's thread"
    );
    w.pump();
    assert_eq!(w.list.app.scope, herdr_gitview::git::Scope::Branch);
    assert_eq!(w.list.app.base, "main");
    let files: Vec<String> = w
        .list
        .app
        .entries
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert_eq!(files, vec!["f.txt", "r.txt"], "diffed against the pin");

    // The diff pane names the same base, and diffs against its merge-base.
    let shown = w.shown_file().unwrap();
    let req = w.preview.app.current.clone().unwrap();
    assert_eq!(req.base.map(|b| b.label).as_deref(), Some("main"));
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 10)).unwrap();
    term.draw(|f| preview::ui::render(f, &mut w.preview.app))
        .unwrap();
    let header: String = (0..80)
        .map(|x| term.backend().buffer()[(x, 0)].symbol().to_string())
        .collect();
    assert!(header.contains("[vs main]"), "header: {header:?}");
    assert_eq!(shown, "f.txt");
    assert!(w.diff_text().contains("from feature"), "{}", w.diff_text());
    // release's file is new relative to main's merge-base.
    w.press("j");
    assert_eq!(w.shown_file().as_deref(), Some("r.txt"));
    assert!(w.diff_text().contains("from release"), "{}", w.diff_text());
}

/// HEAD moving (here: merging the base in) moves the merge-base. Both panes
/// must follow — diffing against the merge-base resolved before the move
/// lists the base's own new commits as this branch's changes.
#[test]
fn branch_scope_follows_the_merge_base_when_head_moves() {
    let repo = fixture("head-moves");
    let dir = repo.dir.clone();
    let rev = |what: &str| {
        let out = std::process::Command::new("git")
            .args(["-C", dir.to_str().unwrap(), "rev-parse", what])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    common::git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "f.txt", "feature work\n");
    common::git(&dir, &["add", "."]);
    common::git(&dir, &["commit", "-q", "-m", "feature work"]);
    common::git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "m.txt", "main moved on\n");
    common::git(&dir, &["add", "."]);
    common::git(&dir, &["commit", "-q", "-m", "main work"]);
    common::git(&dir, &["checkout", "-q", "feature"]);
    let cfg = Config {
        base: "main".to_string(),
        ..Config::default()
    };
    let mut w = World::with_config(repo, cfg);
    list::spawn_poll_thread(
        w.list_tx.clone(),
        w.list.shared_handle(),
        Repo { root: dir.clone() },
        20,
    );
    let files = |w: &World| -> Vec<String> {
        w.list
            .app
            .entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect()
    };
    w.press("w");
    assert_eq!(files(&w), vec!["f.txt"]);
    let before = w.list.app.merge_base.clone();

    common::git(&dir, &["merge", "-q", "--no-edit", "main"]);
    let main = rev("main");
    let deadline = Instant::now() + Duration::from_secs(10);
    while w.list.app.merge_base.as_deref() != Some(main.as_str()) {
        assert!(Instant::now() < deadline, "the merge-base never moved");
        w.pump();
    }
    assert_ne!(w.list.app.merge_base, before);
    assert_eq!(
        files(&w),
        vec!["f.txt"],
        "main's own file is not a branch change"
    );
    let shown = w.preview.app.current.clone().unwrap();
    assert_eq!(
        shown.base.map(|b| b.merge_base),
        Some(main),
        "the diff pane follows"
    );
}
