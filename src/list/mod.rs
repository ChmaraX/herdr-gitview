//! The changed-file list pane: state (`app`), rendering (`ui`), the
//! event-loop logic (`session`), and the thin `run` shell that wires
//! threads and the terminal around a `Session`.

pub mod app;
pub mod session;
pub mod ui;

pub use app::App;
pub use session::{Event, Session};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEventKind};

use crate::config::Config;
use crate::git::{Repo, Scope};
use crate::hostenv::HostEnv;
use crate::keymap::Keymap;
use session::Shared;

pub fn run() -> Result<()> {
    crate::logx::init_panic_hook();

    let start = Instant::now();
    let cfg = Config::load();
    let (keys, keymap_err) = build_keymap(&cfg);
    let repo = resolve_repo()?;
    let poll_ms = cfg.poll_ms;
    let root = repo.root.clone();
    let env = HostEnv::from_process();

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

    let popup_supported = crate::popup::supported(&env);
    let in_herdr = env.in_herdr();
    let socket = env.socket.clone();
    let mut session = Session::new(app, env, tx.clone(), popup_supported);

    if poll_ms > 0 {
        spawn_poll_thread(tx.clone(), session.shared_handle(), Repo { root }, poll_ms);
    }
    // Connect to the preview in the background so a slow/absent preview
    // never blanks or blocks the list UI; the Conn arrives as an event.
    if session::spawn_connector(&tx, socket, Duration::from_secs(10)) {
        session.app.set_status("connecting…");
    }

    let mut terminal = ratatui::init();
    crate::term::enable_mouse();
    let result = event_loop(&mut terminal, &mut session, &rx);
    crate::term::disable_mouse();
    ratatui::restore();

    if session.app.should_quit && in_herdr {
        crate::orchestrate::spawn_close();
    }
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    session: &mut Session,
    rx: &mpsc::Receiver<Event>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, &mut session.app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => session.on_event(event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        session.tick();

        if session.should_quit() {
            return Ok(());
        }
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

/// Every `poll_ms`, hash the status; on change reload the current scope and
/// push a `Refresh`. Cheap fingerprint avoids redundant reloads.
fn spawn_poll_thread(tx: Sender<Event>, shared: Arc<Mutex<Shared>>, repo: Repo, poll_ms: u64) {
    thread::spawn(move || {
        let show_untracked = shared.lock().map(|s| s.show_untracked).unwrap_or(true);
        let mut last = repo.fingerprint(show_untracked);
        loop {
            thread::sleep(Duration::from_millis(poll_ms));
            let fp = repo.fingerprint(show_untracked);
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
