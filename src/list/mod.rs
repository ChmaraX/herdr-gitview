//! The changed-file list pane: state (`app`), rendering (`ui`), and the
//! `run` event loop that wires threads, git, and IPC to the preview together.

pub mod app;
pub mod ui;

pub use app::App;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind, MouseEvent};

use crate::config::Config;
use crate::git::{FileEntry, Repo, Scope};
use crate::ipc::{Conn, ToList, ToPreview};
use crate::keymap::{Action, Keymap};

/// Debounce window for re-showing the diff while the cursor moves quickly.
const SHOW_DEBOUNCE: Duration = Duration::from_millis(40);

/// Everything the main loop can be woken by.
enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Poll thread noticed a change and reloaded the entry vector.
    Refresh(Vec<FileEntry>),
    /// A message from the preview pane.
    Ipc(ToList),
    /// The preview pane went away (socket EOF).
    IpcClosed,
    /// Background nvim probe finished. `unsaved: Some(false)` means the
    /// editor was clean and has already been told to quit; `Some(true)` means
    /// it holds unsaved buffers; `None` means it couldn't be asked.
    EditorProbe {
        then: Option<app::EditorThen>,
        unsaved: Option<bool>,
    },
}

/// Scope info the poll thread needs to reload with the right git command.
struct Shared {
    scope: Scope,
    merge_base: Option<String>,
    show_untracked: bool,
}

pub fn run() -> Result<()> {
    crate::logx::init_panic_hook();

    let start = Instant::now();
    let cfg = Config::load();
    let (keys, keymap_err) = build_keymap(&cfg);
    let repo = resolve_repo()?;
    let poll_ms = cfg.poll_ms;
    let show_untracked = cfg.show_untracked;
    let root = repo.root.clone();

    let mut app = App::new(repo, cfg, keys)?;
    if let Some(err) = keymap_err {
        app.set_status(err);
    }
    crate::logx::log(format!(
        "list: first status loaded in {:?}",
        start.elapsed()
    ));

    let (tx, rx) = mpsc::channel::<Event>();
    spawn_input_thread(tx.clone());

    let shared = Arc::new(Mutex::new(Shared {
        scope: app.scope,
        merge_base: app.merge_base.clone(),
        show_untracked,
    }));
    if poll_ms > 0 {
        spawn_poll_thread(tx.clone(), Arc::clone(&shared), Repo { root }, poll_ms);
    }

    // Connect to the preview pane's socket, if we are running under herdr.
    // Standalone (`GITVIEW_SOCKET` unset) renders the list only — no IPC.
    let conn = connect_preview(&mut app, &tx, Duration::from_secs(10));

    let mut terminal = ratatui::init();
    crate::preview::enable_mouse();
    let result = event_loop(&mut terminal, &mut app, &rx, &tx, &shared, conn);
    crate::preview::disable_mouse();
    ratatui::restore();

    if app.should_quit && std::env::var_os("HERDR_PANE_ID").is_some() {
        crate::orchestrate::spawn_close();
    }
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &mpsc::Receiver<Event>,
    tx: &Sender<Event>,
    shared: &Arc<Mutex<Shared>>,
    mut conn: Option<Conn>,
) -> Result<()> {
    // Debounced diff refresh: mark dirty on selection/scope/view change, flush
    // after `SHOW_DEBOUNCE` in the timeout arm. Start dirty so the first frame
    // sends the initial Show.
    let mut show_dirty = true;
    let mut dirty_since = Instant::now();
    // At most one background nvim probe at a time.
    let mut probe_pending = false;
    // Native popup confirms (herdr ≥0.7.4): answer file we are waiting on.
    let popup_supported = crate::popup::supported();
    let mut popup_answer: Option<PathBuf> = None;
    // Whole-file annotate popup: (answer file, annotated path).
    let mut annotate_answer: Option<(PathBuf, PathBuf)> = None;
    // Note-edit popup: (answer file, note index).
    let mut edit_note_answer: Option<(PathBuf, usize)> = None;

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Key(key)) => {
                // Enter/commit need the IPC link + focus handoff, so they are
                // handled here — but only when a preview is connected and no
                // overlay/lockout is active (App::on_key covers those cases).
                let free = conn.is_some() && app.modal.is_none() && app.busy.is_none();
                let action = app.keys.action(&key);
                let before = show_key(app);
                // Enter activates the selected row (open commit / edit file).
                if app.modal.is_none() && action == Some(Action::Edit) {
                    activate_selection(app, &mut conn);
                } else if free && action == Some(Action::Commit) && app.mode == app::Mode::Files {
                    start_commit(app, &mut conn);
                // `r` with a dead preview link retries the connection too.
                } else if action == Some(Action::Refresh) && conn.is_none() && app.modal.is_none() {
                    conn = connect_preview(app, tx, Duration::from_secs(2));
                    if conn.is_some() {
                        app.set_status("preview reconnected");
                        show_dirty = true;
                        dirty_since = Instant::now();
                    }
                    app.on_key(key); // still do the refresh itself
                // Diff scroll/page keys are forwarded straight to the preview
                // (skipped while nvim owns that PTY).
                } else if app.busy.is_none()
                    && let Some(msg) = scroll_message(app, &key)
                {
                    send(&mut conn, &msg);
                } else {
                    app.on_key(key);
                    sync_shared(shared, app);
                }
                // Any path that changed what should be shown (moving the
                // cursor, but also entering a commit's files or the notes
                // view) re-Shows after the debounce.
                if show_key(app) != before {
                    show_dirty = true;
                    dirty_since = Instant::now();
                }
                // Content changed under the same selection (stage/discard).
                if app.needs_reshow {
                    app.needs_reshow = false;
                    show_dirty = true;
                    dirty_since = Instant::now();
                }
                // Editor-close triggers, probed off-thread so the UI never
                // stalls on nvim's remote API:
                //  - an action explicitly requested it (q / c), or
                //  - the cursor moved while editing — a clean nvim quits so
                //    the diff preview comes back as you browse.
                if app.busy.is_some() && !probe_pending {
                    let moved = matches!(
                        action,
                        Some(Action::Down | Action::Up | Action::Top | Action::Bottom)
                    ) && app.modal.is_none();
                    if let Some(then) = app.editor_close_request.take() {
                        probe_pending = true;
                        spawn_editor_probe(tx.clone(), app.cfg.editor.first().cloned(), Some(then));
                    } else if moved {
                        probe_pending = true;
                        spawn_editor_probe(tx.clone(), app.cfg.editor.first().cloned(), None);
                    }
                } else {
                    app.editor_close_request = None; // probe already in flight
                }
            }
            Ok(Event::Mouse(m)) => {
                let before = show_key(app);
                let activate = app.on_mouse(m.kind, m.row);
                if activate {
                    activate_selection(app, &mut conn);
                }
                if show_key(app) != before {
                    show_dirty = true;
                    dirty_since = Instant::now();
                }
            }
            Ok(Event::EditorProbe { then, unsaved }) => {
                probe_pending = false;
                match (unsaved, then) {
                    // Clean — already told to quit; EditDone resumes `then`.
                    (Some(false), then) => app.after_edit = then,
                    // Dirty + an action waiting → ask.
                    (Some(true), Some(then)) => app.modal = Some(app::Modal::EditorClose { then }),
                    // Dirty + just browsing → leave nvim alone, hint once.
                    (Some(true), None) => app.set_status("editor has unsaved changes — stays open"),
                    (None, Some(_)) => app.set_status("editor is open — close it first"),
                    (None, None) => {}
                }
            }
            Ok(Event::Refresh(entries)) => {
                app.apply_refresh(entries);
                // Content may have changed under the cursor — re-show the diff.
                show_dirty = true;
                dirty_since = Instant::now();
            }
            Ok(Event::Ipc(ToList::Ready)) => {
                if matches!(app.active_status(), Some("connecting…")) {
                    app.status_msg = None;
                }
            }
            Ok(Event::Ipc(ToList::EditDone { .. })) => {
                app.on_edit_done();
                show_dirty = true;
                dirty_since = Instant::now();
                focus_self();
                // Resume whatever asked the editor to close.
                match app.after_edit.take() {
                    Some(app::EditorThen::QuitView) => app.should_quit = true,
                    Some(app::EditorThen::Commit) => start_commit(app, &mut conn),
                    None => {}
                }
            }
            Ok(Event::Ipc(ToList::ShowNotesView)) => {
                if app.mode != app::Mode::Notes && !app.notes.is_empty() {
                    app.on_key(KeyEvent::new(
                        crossterm::event::KeyCode::Char('n'),
                        crossterm::event::KeyModifiers::NONE,
                    ));
                }
                focus_self();
                show_dirty = true;
                dirty_since = Instant::now();
            }
            Ok(Event::Ipc(ToList::Notes { notes })) => {
                app.notes = notes;
                if app.mode == app::Mode::Notes {
                    app.rebuild_rows();
                    if app.notes.is_empty() {
                        app.set_status("notes sent");
                        app.on_key(KeyEvent::new(
                            crossterm::event::KeyCode::Char('n'),
                            crossterm::event::KeyModifiers::NONE,
                        ));
                    }
                }
            }
            Ok(Event::Ipc(ToList::GitDone { ok })) => {
                app.on_edit_done();
                show_dirty = true;
                dirty_since = Instant::now();
                focus_self();
                if ok {
                    match app.repo.last_commit_summary() {
                        Some(s) => app.set_status(format!("committed {s}")),
                        None => app.set_status("committed"),
                    }
                } else {
                    app.set_status("commit aborted");
                }
            }
            Ok(Event::IpcClosed) => conn = None, // preview gone; keep list usable
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        // Flush a debounced Show once the cursor has settled.
        // Whole-file annotate (`a` on a list row) → popup → AddNote to the
        // preview, which owns the note store.
        if let Some(file) = app.annotate_request.take() {
            let envs = [(
                "GITVIEW_ASK_TEXT".to_string(),
                format!("note for {}", file.display()),
            )];
            match crate::popup::spawn("annotate", &envs, 64, 8) {
                Some(path) => annotate_answer = Some((path, file)),
                None => app.set_status(
                    "popup failed — needs herdr ≥0.7.4 + re-linked plugin (see debug log)",
                ),
            }
        }
        if let Some((path, file)) = &annotate_answer {
            let mut pending = Some(path.clone());
            if let Some(text) = crate::popup::poll(&mut pending) {
                if !text.is_empty() {
                    send(
                        &mut conn,
                        &ToPreview::AddNote {
                            file: file.clone(),
                            text,
                        },
                    );
                }
                annotate_answer = None;
            }
        }
        // `p`: hand off to the preview (it owns the notes + picker flow).
        if app.send_notes_request {
            app.send_notes_request = false;
            send(&mut conn, &ToPreview::SendNotes);
        }
        // Notes view: enter = edit via popup, d = delete.
        if let Some((idx, current)) = app.edit_note_request.take() {
            let envs = [
                ("GITVIEW_ASK_TEXT".to_string(), "edit note".to_string()),
                ("GITVIEW_PREFILL".to_string(), current),
            ];
            match crate::popup::spawn("annotate", &envs, 64, 8) {
                Some(path) => edit_note_answer = Some((path, idx)),
                None => app.set_status("popup failed (see debug log)"),
            }
        }
        if let Some((path, idx)) = &edit_note_answer {
            let mut pending = Some(path.clone());
            if let Some(text) = crate::popup::poll(&mut pending) {
                if !text.is_empty() {
                    send(&mut conn, &ToPreview::EditNote { idx: *idx, text });
                }
                edit_note_answer = None;
            }
        }
        if let Some(idx) = app.delete_note_request.take() {
            send(&mut conn, &ToPreview::DeleteNote { idx });
        }

        // Confirm modals become native floating popup panes when herdr
        // supports them; the in-pane overlay is the fallback.
        if popup_supported
            && !app.modal_external
            && popup_answer.is_none()
            && matches!(
                app.modal,
                Some(app::Modal::Confirm { .. } | app::Modal::EditorClose { .. })
            )
            && let Some(path) = spawn_popup_confirm(app)
        {
            app.modal_external = true;
            popup_answer = Some(path);
        }
        // Poll for the popup's answer and feed it through the same modal
        // key-handling as the in-pane overlay.
        if let Some(answer) = crate::popup::poll(&mut popup_answer) {
            app.modal_external = false;
            let code = match answer.trim() {
                "y" => crossterm::event::KeyCode::Char('y'),
                "n" => crossterm::event::KeyCode::Char('n'),
                _ => crossterm::event::KeyCode::Esc,
            };
            app.on_key(KeyEvent::new(code, crossterm::event::KeyModifiers::NONE));
            if app.needs_reshow {
                app.needs_reshow = false;
                show_dirty = true;
                dirty_since = Instant::now();
            }
        }

        if show_dirty && dirty_since.elapsed() >= SHOW_DEBOUNCE {
            if app.mode == app::Mode::Notes {
                // Hovering a note: show its file's diff + scroll to the card.
                if let Some(idx) = app.selected_note() {
                    if let Some(msg) = note_show(app, idx) {
                        send(&mut conn, &msg);
                    }
                    send(&mut conn, &ToPreview::FocusNote { idx });
                }
            } else {
                match current_show(app) {
                    Some(msg) => send(&mut conn, &msg),
                    // Nothing selectable left (e.g. the last change was
                    // discarded/committed) — clear the stale diff.
                    None => send(&mut conn, &ToPreview::Clear),
                }
            }
            show_dirty = false;
        }

        if app.should_quit {
            send(&mut conn, &ToPreview::Quit);
            return Ok(());
        }
    }
}

/// Connect to the preview socket and start reading `ToList`.
/// Returns `None` in standalone mode or on failure (list still runs).
fn connect_preview(app: &mut App, tx: &Sender<Event>, budget: Duration) -> Option<Conn> {
    let sock = std::env::var_os("GITVIEW_SOCKET").map(PathBuf::from)?;
    match Conn::connect_retry(&sock, budget) {
        Ok(conn) => {
            let (ipc_tx, ipc_rx) = mpsc::channel::<ToList>();
            let conn = conn.spawn_reader(ipc_tx);
            spawn_ipc_forwarder(ipc_rx, tx.clone());
            app.status_msg = Some(("connecting…".to_string(), Instant::now()));
            Some(conn)
        }
        Err(err) => {
            crate::logx::log(format!("list: preview connect failed: {err}"));
            None
        }
    }
}

/// Enter on the selected entry: guard deleted files, then hand the preview
/// pane the Edit and the focus. The lockout ends when `EditDone` arrives.
fn start_edit(app: &mut App, conn: &mut Option<Conn>) {
    let Some((entry, _)) = app.selected_entry() else {
        return; // empty list or header row
    };
    // The editor always opens the *current* file — also from the history
    // view — so guard on what exists on disk, not on the change kind.
    if !app.repo.root.join(&entry.path).exists() {
        app.set_status("file no longer exists — nothing to edit");
        return;
    }
    let file = entry.path.clone();
    send(conn, &ToPreview::Edit { file: file.clone() });
    if conn.is_none() {
        return; // send failed — preview link just broke
    }
    app.busy = Some(format!("editing {}…", file.display()));
    focus_preview();
}

/// Enter (or a double-click): open the selected commit in the log view, or
/// the selected file in the editor — remotely switching a running nvim.
fn activate_selection(app: &mut App, conn: &mut Option<Conn>) {
    match app.mode {
        app::Mode::Log => app.open_commit(),
        app::Mode::Notes => {
            if let Some(idx) = app.selected_note() {
                let text = app.notes.get(idx).map(|n| n.3.clone()).unwrap_or_default();
                app.edit_note_request = Some((idx, text));
            }
        }
        app::Mode::Files | app::Mode::CommitFiles => {
            if app.busy.is_some() {
                remote_open(app);
            } else if conn.is_some() {
                start_edit(app, conn);
            } else if std::env::var_os("HERDR_PANE_ID").is_none() {
                app.set_status("editing needs the preview pane (run inside herdr)");
            } else {
                app.set_status("preview not connected — press r to reconnect");
            }
        }
    }
}

/// Open the current modal as a native floating popup pane; returns the
/// answer-file path to poll, or None when the popup could not be opened
/// (caller falls back to the in-pane overlay).
fn spawn_popup_confirm(app: &App) -> Option<PathBuf> {
    let text = match &app.modal {
        Some(app::Modal::Confirm { text, .. }) => text.clone(),
        Some(app::Modal::EditorClose { .. }) => {
            "The editor has unsaved changes. Save them? (no discards)".to_string()
        }
        _ => return None,
    };
    crate::popup::spawn("ask", &[("GITVIEW_ASK_TEXT".to_string(), text)], 60, 7)
}

/// Probe the remote nvim on a background thread: a clean editor is told to
/// quit immediately; the result comes back as `Event::EditorProbe`.
fn spawn_editor_probe(tx: Sender<Event>, editor: Option<String>, then: Option<app::EditorThen>) {
    thread::spawn(move || {
        let editor = editor.unwrap_or_default();
        let unsaved = app::editor_has_unsaved(&editor);
        if unsaved == Some(false) {
            let _ = app::editor_remote(&editor, &["--remote-send", "<C-\\><C-n>:qa!<CR>"]);
        }
        let _ = tx.send(Event::EditorProbe { then, unsaved });
    });
}

/// Enter while the editor is running: nvim was started with `--listen`, so
/// tell it to open the newly selected file instead of refusing the key.
fn remote_open(app: &mut App) {
    let Some((entry, _)) = app.selected_entry() else {
        return;
    };
    let abs = app.repo.root.join(&entry.path);
    if !abs.exists() {
        app.set_status("file no longer exists — nothing to edit");
        return;
    }
    let editor = app.cfg.editor.first().cloned().unwrap_or_default();
    let server = crate::preview::editor_server_path();
    let Some(server) = server.filter(|s| s.exists() && editor.contains("nvim")) else {
        app.set_status("editor is open — close it first");
        return;
    };
    let result = std::process::Command::new(&editor)
        .arg("--server")
        .arg(&server)
        .arg("--remote")
        .arg(&abs)
        .output();
    match result {
        Ok(out) if out.status.success() => {
            app.busy = Some(format!("editing {}…", entry.path.display()));
            focus_preview();
        }
        _ => app.set_status("could not switch the editor's file"),
    }
}

/// `c`: preflight the staged set, then run `git commit -e` on the preview PTY
/// (nvim opens the commit template there). Same lockout as editing.
fn start_commit(app: &mut App, conn: &mut Option<Conn>) {
    match app.repo.staged_count() {
        Ok(0) => {
            app.set_status("nothing staged — s to stage files");
            return;
        }
        Err(err) => {
            app.set_status(format!("commit preflight failed: {err}"));
            return;
        }
        Ok(_) => {}
    }
    send(
        conn,
        &ToPreview::GitInPane {
            argv: vec!["commit".to_string(), "-e".to_string()],
        },
    );
    if conn.is_none() {
        return;
    }
    app.busy = Some("committing…".to_string());
    focus_preview();
}

fn focus_preview() {
    if let Some(preview) = std::env::var_os("GITVIEW_PREVIEW_PANE") {
        crate::herdr_cli::focus_pane(&preview.to_string_lossy());
    }
}

fn focus_self() {
    if let Some(own) = std::env::var_os("HERDR_PANE_ID") {
        crate::herdr_cli::focus_pane(&own.to_string_lossy());
    }
}

/// The Show message for the current selection, or `None` when nothing
/// diffable is selected (headers, commit rows, empty list).
fn current_show(app: &App) -> Option<ToPreview> {
    let (e, staged) = app.selected_entry()?;
    let commit = match app.mode {
        app::Mode::CommitFiles => Some(app.commit.as_ref()?.sha.clone()),
        _ => None,
    };
    Some(ToPreview::Show {
        file: e.path.clone(),
        orig_path: e.orig_path.clone(),
        scope: app.scope,
        cached: staged && app.scope == Scope::Worktree,
        kind: e.kind,
        commit,
    })
}

/// Identity of what the preview is showing; a change here means "re-Show".
fn show_key(app: &App) -> Option<(PathBuf, Scope, bool, Option<String>)> {
    if app.mode == app::Mode::Notes {
        let idx = app.selected_note()?;
        let file = app.notes.get(idx)?.0.clone();
        return Some((file, app.scope, false, Some(format!("note-{idx}"))));
    }
    let (e, staged) = app.selected_entry()?;
    let commit = match app.mode {
        app::Mode::CommitFiles => app.commit.as_ref().map(|c| c.sha.clone()),
        _ => None,
    };
    Some((e.path.clone(), app.scope, staged, commit))
}

/// The Show for a hovered note: its file in the note's own context — the
/// commit it was made against (history notes) or the live worktree diff.
fn note_show(app: &App, idx: usize) -> Option<ToPreview> {
    let (file, _, _, _) = app.notes.get(idx)?;
    // Prefer real entry metadata when the file is among the current entries;
    // otherwise synthesize (kind only affects rename pathspecs).
    let entry = app.entries.iter().find(|e| &e.path == file);
    Some(ToPreview::Show {
        file: file.clone(),
        orig_path: entry.and_then(|e| e.orig_path.clone()),
        scope: Scope::Worktree,
        cached: false,
        kind: entry
            .map(|e| e.kind)
            .unwrap_or(crate::git::ChangeKind::Modified),
        commit: None,
    })
}

/// Translate a diff scroll/page key into the message the preview understands,
/// or `None` if this key is not a scroll key.
fn scroll_message(app: &App, key: &KeyEvent) -> Option<ToPreview> {
    match app.keys.action(key)? {
        Action::ScrollDown => Some(ToPreview::Scroll { delta: 1 }),
        Action::ScrollUp => Some(ToPreview::Scroll { delta: -1 }),
        Action::HalfPageDown => Some(ToPreview::Page {
            down: true,
            full: false,
        }),
        Action::HalfPageUp => Some(ToPreview::Page {
            down: false,
            full: false,
        }),
        Action::DiffTop => Some(ToPreview::Scroll { delta: i32::MIN }),
        Action::DiffBottom => Some(ToPreview::Scroll { delta: i32::MAX }),
        _ => None,
    }
}

/// Keymap from config overrides; a bad `[keybindings]` table must not stop
/// startup — fall back to defaults and surface the error as a status message.
fn build_keymap(cfg: &Config) -> (Keymap, Option<String>) {
    match Keymap::build(&cfg.keybindings) {
        Ok(keys) => (keys, None),
        Err(err) => (
            Keymap::build(&Default::default()).expect("default keymap is valid"),
            Some(format!("keybindings ignored: {err}")),
        ),
    }
}

/// Best-effort send; a broken pipe just drops the preview link.
fn send(conn: &mut Option<Conn>, msg: &ToPreview) {
    if let Some(c) = conn
        && c.send(msg).is_err()
    {
        *conn = None;
    }
}

fn sync_shared(shared: &Arc<Mutex<Shared>>, app: &App) {
    if let Ok(mut s) = shared.lock() {
        s.scope = app.scope;
        s.merge_base = app.merge_base.clone();
    }
}

/// Blocking `event::read` on its own thread; forwards key presses only.
fn spawn_input_thread(tx: Sender<Event>) {
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                    if tx.send(Event::Key(key)).is_err() {
                        break;
                    }
                }
                Ok(CtEvent::Mouse(m)) => {
                    if tx.send(Event::Mouse(m)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

/// Bridge the typed IPC reader channel into the main event channel; a closed
/// channel (socket EOF) becomes `IpcClosed`.
fn spawn_ipc_forwarder(ipc_rx: mpsc::Receiver<ToList>, tx: Sender<Event>) {
    thread::spawn(move || {
        while let Ok(msg) = ipc_rx.recv() {
            if tx.send(Event::Ipc(msg)).is_err() {
                return;
            }
        }
        let _ = tx.send(Event::IpcClosed);
    });
}

/// Every `poll_ms`, hash the status; on change reload the current scope and
/// push a `Refresh`. Cheap fingerprint avoids redundant reloads.
fn spawn_poll_thread(tx: Sender<Event>, shared: Arc<Mutex<Shared>>, repo: Repo, poll_ms: u64) {
    thread::spawn(move || {
        let mut last = repo.fingerprint();
        loop {
            thread::sleep(Duration::from_millis(poll_ms));
            let fp = repo.fingerprint();
            if fp == last {
                continue;
            }
            last = fp;

            let (scope, merge_base, show_untracked) = match shared.lock() {
                Ok(s) => (s.scope, s.merge_base.clone(), s.show_untracked),
                Err(_) => return,
            };
            let entries = match scope {
                Scope::Worktree => repo.worktree_status(show_untracked),
                Scope::Branch => repo.branch_changes(merge_base.as_deref().unwrap_or("HEAD")),
            };
            if let Ok(entries) = entries
                && tx.send(Event::Refresh(entries)).is_err()
            {
                break;
            }
        }
    });
}

/// Repo root: `GITVIEW_REPO` when running under herdr, else discover from cwd
/// (standalone dev mode — `cargo run -- list` in any repo).
fn resolve_repo() -> Result<Repo> {
    match std::env::var_os("GITVIEW_REPO") {
        Some(dir) => Repo::discover(&PathBuf::from(dir)),
        None => Repo::discover(Path::new(&std::env::current_dir()?)),
    }
}
