//! The changed-file list pane: state (`app`), rendering (`ui`), and the
//! `run` event loop that wires threads + git together.

pub mod app;
pub mod ui;

pub use app::App;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEventKind};

use crate::config::Config;
use crate::git::{FileEntry, Repo, Scope};
use crate::keymap::Keymap;

/// Everything the main loop can be woken by.
enum Event {
    Key(crossterm::event::KeyEvent),
    /// Poll thread noticed a change and reloaded the entry vector.
    Refresh(Vec<FileEntry>),
    /// Reserved periodic wakeup (the 100 ms `recv_timeout` also serves this).
    #[allow(dead_code)]
    Tick,
}

/// Scope info the poll thread needs to reload with the right git command.
/// The main loop keeps this in sync after every key it handles.
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
        spawn_poll_thread(tx, Arc::clone(&shared), Repo { root }, poll_ms);
    }

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &rx, &shared);
    ratatui::restore();

    // Any pane's quit tears down the whole herdr view; standalone there is
    // nothing to close.
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
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Key(key)) => {
                app.on_key(key);
                // Keep the poll thread aware of the current scope so its
                // background reloads match what the user is viewing.
                if let Ok(mut s) = shared.lock() {
                    s.scope = app.scope;
                    s.merge_base = app.merge_base.clone();
                }
            }
            Ok(Event::Refresh(entries)) => app.apply_refresh(entries),
            Ok(Event::Tick) => {}
            Err(RecvTimeoutError::Timeout) => {} // redraw (status messages expire)
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Blocking `event::read` on its own thread; forwards key presses only.
fn spawn_input_thread(tx: mpsc::Sender<Event>) {
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

/// Every `poll_ms`, hash the status; on change reload the current scope and
/// push a `Refresh`. Cheap fingerprint avoids redundant reloads.
fn spawn_poll_thread(
    tx: mpsc::Sender<Event>,
    shared: Arc<Mutex<Shared>>,
    repo: Repo,
    poll_ms: u64,
) {
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
