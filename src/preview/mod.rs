//! The diff preview pane: state (`app`), rendering (`ui`), and the `run` event
//! loop that owns the IPC socket, the latest-wins diff worker, and input.

pub mod app;
pub mod editor;
pub mod ui;

pub use app::{PreviewApp, ShowReq};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    // Bad [keybindings] must not stop startup; fall back to defaults.
    let keys = Keymap::build(&cfg.keybindings).unwrap_or_else(|err| {
        crate::logx::log(format!("preview: keybindings ignored: {err}"));
        Keymap::build(&Default::default()).expect("default keymap is valid")
    });
    let repo = resolve_repo()?;
    let root = repo.root.clone();
    let mut app = PreviewApp::new(cfg, repo, keys);

    let (tx, rx) = mpsc::channel::<Event>();
    // While an editor owns the PTY, the input thread must stop reading stdin
    // or it would steal nvim's keystrokes.
    let input_paused = Arc::new(AtomicBool::new(false));
    spawn_input_thread(tx.clone(), Arc::clone(&input_paused));

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
    let result = event_loop(&mut terminal, &mut app, &rx, &tx, &work_tx, &input_paused);
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
    input_paused: &Arc<AtomicBool>,
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

            // Edit / GitInPane suspend the TUI, so they need the terminal —
            // handle them here; everything else goes through `handle_ipc`.
            Ok(Event::Ipc(ToPreview::Edit { file })) => {
                run_editor(terminal, app, &file, input_paused);
                if let Some(req) = app.current.clone() {
                    let _ = work_tx.send(req); // file changed on disk — re-diff
                }
                if let Some(c) = conn.as_mut() {
                    let _ = c.send(&ToList::EditDone { file });
                }
            }
            Ok(Event::Ipc(ToPreview::GitInPane { argv })) => {
                let ok = run_git_in_pane(terminal, app, &argv, input_paused);
                if let Some(req) = app.current.clone() {
                    let _ = work_tx.send(req);
                }
                if let Some(c) = conn.as_mut() {
                    let _ = c.send(&ToList::GitDone { ok });
                }
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
        // Handled directly in the event loop (they need the terminal).
        ToPreview::Edit { .. } | ToPreview::GitInPane { .. } => {}
    }
}

/// Open the configured editor on `file`, jumping to its first changed line
/// (derived from the currently shown diff when it is for the same file).
fn run_editor(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut PreviewApp,
    file: &Path,
    input_paused: &Arc<AtomicBool>,
) {
    let mut argv = app.cfg.editor.clone();
    let same_file = app
        .current
        .as_ref()
        .map(|c| c.file == *file)
        .unwrap_or(false);
    if same_file && let Some(line) = editor::first_new_line(&app.raw) {
        argv.push(format!("+{line}"));
    }
    argv.push(app.repo.root.join(file).display().to_string());
    run_suspended(terminal, app, &argv, &[], input_paused);
}

/// Run `git -C <root> <argv…>` interactively on this PTY (e.g. commit -e).
/// Sets GIT_EDITOR from our config only when the user configured nothing.
fn run_git_in_pane(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut PreviewApp,
    argv: &[String],
    input_paused: &Arc<AtomicBool>,
) -> bool {
    let mut full = vec![
        "git".to_string(),
        "-C".to_string(),
        app.repo.root.display().to_string(),
    ];
    full.extend(argv.iter().cloned());

    let mut envs = Vec::new();
    if std::env::var_os("GIT_EDITOR").is_none() && !has_core_editor(app) {
        envs.push(("GIT_EDITOR".to_string(), app.cfg.editor.join(" ")));
    }
    run_suspended(terminal, app, &full, &envs, input_paused)
}

/// Common suspend→run→restore path; a spawn failure lands in the Error state.
fn run_suspended(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut PreviewApp,
    argv: &[String],
    envs: &[(String, String)],
    input_paused: &Arc<AtomicBool>,
) -> bool {
    input_paused.store(true, Ordering::SeqCst);
    let result = editor::run_on_pty(terminal, &app.repo.root.clone(), argv, envs);
    input_paused.store(false, Ordering::SeqCst);
    match result {
        Ok(ok) => ok,
        Err(err) => {
            app.state = app::State::Error(first_line(&err.to_string()));
            false
        }
    }
}

/// Does the user have an editor configured for git itself?
fn has_core_editor(app: &PreviewApp) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(&app.repo.root)
        .args(["config", "--get", "core.editor"])
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

/// Input reader on its own thread; forwards key presses only. Uses
/// `poll` + `read` so it can stop touching stdin while `paused` is set
/// (an editor owns the PTY then — see `run_suspended`).
fn spawn_input_thread(tx: Sender<Event>, paused: Arc<AtomicBool>) {
    thread::spawn(move || {
        loop {
            if paused.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(30));
                continue;
            }
            match event::poll(Duration::from_millis(100)) {
                Ok(false) => continue,
                Ok(true) => match event::read() {
                    Ok(CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                        if tx.send(Event::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {} // resize etc. — next 100 ms redraw picks it up
                    Err(_) => break,
                },
                Err(_) => break,
            }
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
