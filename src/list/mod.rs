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

    let cfg = Config::load();
    let keys = Keymap::build(&cfg.keybindings)?;
    let repo = resolve_repo()?;
    let poll_ms = cfg.poll_ms;
    let show_untracked = cfg.show_untracked;
    let root = repo.root.clone();

    let mut app = App::new(repo, cfg, keys)?;

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
                // Diff scroll/page keys are forwarded straight to the preview.
                if let Some(msg) = scroll_message(app, &key) {
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
            Ok(Event::Ipc(_)) => {} // EditDone / GitDone handled in later phases
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
    thread::spawn(move || loop {
        match event::read() {
            Ok(CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                if tx.send(Event::Key(key)).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
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
