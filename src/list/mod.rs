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
use crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind};

use crate::config::Config;
use crate::git::{FileEntry, Repo, Scope};
use crate::ipc::{Conn, ToList, ToPreview};
use crate::keymap::{Action, Keymap};

/// Debounce window for re-showing the diff while the cursor moves quickly.
const SHOW_DEBOUNCE: Duration = Duration::from_millis(40);

/// Everything the main loop can be woken by.
enum Event {
    Key(KeyEvent),
    /// Poll thread noticed a change and reloaded the entry vector.
    Refresh(Vec<FileEntry>),
    /// A message from the preview pane.
    Ipc(ToList),
    /// The preview pane went away (socket EOF).
    IpcClosed,
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
    let conn = connect_preview(&mut app, &tx);

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &rx, &shared, conn);
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
    shared: &Arc<Mutex<Shared>>,
    mut conn: Option<Conn>,
) -> Result<()> {
    // Debounced diff refresh: mark dirty on selection/scope/view change, flush
    // after `SHOW_DEBOUNCE` in the timeout arm. Start dirty so the first frame
    // sends the initial Show.
    let mut show_dirty = true;
    let mut dirty_since = Instant::now();

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Key(key)) => {
                // Enter/commit need the IPC link + focus handoff, so they are
                // handled here — but only when a preview is connected and no
                // overlay/lockout is active (App::on_key covers those cases).
                let free = conn.is_some() && app.modal.is_none() && app.busy.is_none();
                let action = app.keys.action(&key);
                if free && action == Some(Action::Edit) {
                    start_edit(app, &mut conn);
                } else if free && action == Some(Action::Commit) {
                    start_commit(app, &mut conn);
                // Diff scroll/page keys are forwarded straight to the preview.
                } else if let Some(msg) = scroll_message(app, &key) {
                    send(&mut conn, &msg);
                } else {
                    let before = show_key(app);
                    app.on_key(key);
                    sync_shared(shared, app);
                    if show_key(app) != before {
                        show_dirty = true;
                        dirty_since = Instant::now();
                    }
                }
                // Content changed under the same selection (stage/discard).
                if app.needs_reshow {
                    app.needs_reshow = false;
                    show_dirty = true;
                    dirty_since = Instant::now();
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
        if show_dirty && dirty_since.elapsed() >= SHOW_DEBOUNCE {
            if let Some(msg) = current_show(app) {
                send(&mut conn, &msg);
            }
            show_dirty = false;
        }

        if app.should_quit {
            send(&mut conn, &ToPreview::Quit);
            return Ok(());
        }
    }
}

/// Connect to the preview socket (10 s budget) and start reading `ToList`.
/// Returns `None` in standalone mode or on failure (list still runs).
fn connect_preview(app: &mut App, tx: &Sender<Event>) -> Option<Conn> {
    let sock = std::env::var_os("GITVIEW_SOCKET").map(PathBuf::from)?;
    match Conn::connect_retry(&sock, Duration::from_secs(10)) {
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
    let Some(entry) = app.entries.get(app.cursor) else {
        return; // empty list
    };
    if entry.kind == crate::git::ChangeKind::Deleted {
        app.set_status("file is deleted — nothing to edit");
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

/// The Show message for the current selection, or `None` when the list is empty.
fn current_show(app: &App) -> Option<ToPreview> {
    let e = app.entries.get(app.cursor)?;
    Some(ToPreview::Show {
        file: e.path.clone(),
        orig_path: e.orig_path.clone(),
        scope: app.scope,
        cached: app.cached_view && app.scope == Scope::Worktree,
        kind: e.kind,
    })
}

/// Identity of what the preview is showing; a change here means "re-Show".
fn show_key(app: &App) -> Option<(PathBuf, Scope, bool)> {
    let e = app.entries.get(app.cursor)?;
    Some((
        e.path.clone(),
        app.scope,
        app.cached_view && app.scope == Scope::Worktree,
    ))
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
