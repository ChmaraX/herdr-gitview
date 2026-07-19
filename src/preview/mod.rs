//! The diff preview pane: state (`app`), rendering (`ui`), and the `run` event
//! loop that owns the IPC socket, the latest-wins diff worker, and input.

pub mod app;
pub mod ui;

pub use app::{PreviewApp, ShowReq};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind};

use crate::config::Config;
use crate::git::Repo;
use crate::ipc::{Conn, ToList, ToPreview};
use crate::keymap::Keymap;

/// Everything the main loop can be woken by.
enum Event {
    Key(KeyEvent),
    /// The list finished connecting; `Conn` is the live link.
    Connected(Conn),
    /// A message arrived from the list pane.
    Ipc(ToPreview),
    /// The list pane went away (socket EOF).
    IpcClosed,
    /// The diff worker produced a result for `req`.
    Diff {
        req: ShowReq,
        result: Result<Vec<u8>, String>,
    },
}

pub fn run() -> Result<()> {
    crate::logx::init_panic_hook();

    let cfg = Config::load();
    let keys = Keymap::build(&cfg.keybindings)?;
    let repo = resolve_repo()?;
    let root = repo.root.clone();
    let mut app = PreviewApp::new(cfg, repo, keys);

    let (tx, rx) = mpsc::channel::<Event>();
    spawn_input_thread(tx.clone());

    // Preview is the IPC listener. `accept` blocks, so do it on a thread and
    // render the "waiting…" splash meanwhile; the connected `Conn` arrives as
    // an event. Standalone (no socket) skips IPC entirely.
    if let Some(sock) = std::env::var_os("GITVIEW_SOCKET").map(PathBuf::from) {
        let tx = tx.clone();
        thread::spawn(move || match Conn::listen(&sock) {
            Ok(conn) => {
                let _ = tx.send(Event::Connected(conn));
            }
            Err(err) => crate::logx::log(format!("preview: listen failed: {err}")),
        });
    } else {
        app.on_connected(); // dev mode: nothing will connect
    }

    // Latest-wins diff worker. Each Show is pushed here; the worker drains to
    // the newest request before running git, so holding `j` never queues runs.
    let work_tx = spawn_diff_worker(tx.clone(), Repo { root });

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &rx, &tx, &work_tx);
    ratatui::restore();

    if app.close_view && std::env::var_os("HERDR_PANE_ID").is_some() {
        crate::orchestrate::spawn_close();
    }
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut PreviewApp,
    rx: &mpsc::Receiver<Event>,
    tx: &Sender<Event>,
    work_tx: &Sender<ShowReq>,
) -> Result<()> {
    // Held once connected so the UI thread can send `ToList` (e.g. `Ready`).
    let mut conn: Option<Conn> = None;

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Key(key)) => app.on_key(key),

            Ok(Event::Connected(new_conn)) => {
                // Split the reader onto a thread that forwards `ToList`-shaped
                // frames? No — the list sends `ToPreview`; decode those and
                // fan them into our event channel, with EOF → IpcClosed.
                let (ipc_tx, ipc_rx) = mpsc::channel::<ToPreview>();
                let mut c = new_conn.spawn_reader(ipc_tx);
                spawn_ipc_forwarder(ipc_rx, tx.clone());
                let _ = c.send(&ToList::Ready);
                conn = Some(c);
                app.on_connected();
            }

            Ok(Event::Ipc(msg)) => handle_ipc(app, msg, work_tx),

            Ok(Event::IpcClosed) => {
                // The list (and thus the whole view) is gone — exit cleanly.
                app.should_quit = true;
            }

            Ok(Event::Diff { req, result }) => app.apply_diff(&req, result),

            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        if app.should_quit {
            // Drop the link so the list sees EOF promptly.
            drop(conn.take());
            return Ok(());
        }
    }
}

fn handle_ipc(app: &mut PreviewApp, msg: ToPreview, work_tx: &Sender<ShowReq>) {
    match msg {
        ToPreview::Show {
            file,
            orig_path,
            scope,
            cached,
            kind,
        } => {
            let req = ShowReq {
                file,
                orig_path,
                scope,
                cached,
                kind,
            };
            app.begin_show(req.clone());
            let _ = work_tx.send(req);
        }
        ToPreview::Scroll { delta } => app.scroll_by(delta),
        ToPreview::Page { down, full } => app.page(down, full),
        ToPreview::Quit => app.should_quit = true, // list initiated teardown
        // Editor / git-in-pane land in later phases.
        ToPreview::Edit { .. } | ToPreview::GitInPane { .. } => {}
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

/// Bridge the typed IPC reader channel into the main event channel, turning a
/// closed channel (socket EOF) into `IpcClosed`.
fn spawn_ipc_forwarder(ipc_rx: mpsc::Receiver<ToPreview>, tx: Sender<Event>) {
    thread::spawn(move || {
        while let Ok(msg) = ipc_rx.recv() {
            if tx.send(Event::Ipc(msg)).is_err() {
                return;
            }
        }
        let _ = tx.send(Event::IpcClosed);
    });
}

/// Latest-wins diff runner. Returns the sender used to enqueue `ShowReq`s.
fn spawn_diff_worker(tx: Sender<Event>, repo: Repo) -> Sender<ShowReq> {
    let (work_tx, work_rx) = mpsc::channel::<ShowReq>();
    thread::spawn(move || {
        while let Ok(mut req) = work_rx.recv() {
            // Collapse a backlog to the newest request.
            while let Ok(newer) = work_rx.try_recv() {
                req = newer;
            }
            let entry = req.to_entry();
            let result = repo
                .diff_ansi(&entry, req.scope, req.cached)
                .map_err(|err| first_line(&err.to_string()));
            if tx.send(Event::Diff { req, result }).is_err() {
                break;
            }
        }
    });
    work_tx
}

/// First line of an error message (git failures embed stderr; we want the top).
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// Repo root: `GITVIEW_REPO` under herdr, else discover from cwd (dev mode).
fn resolve_repo() -> Result<Repo> {
    match std::env::var_os("GITVIEW_REPO") {
        Some(dir) => Repo::discover(&PathBuf::from(dir)),
        None => Repo::discover(Path::new(&std::env::current_dir()?)),
    }
}
