//! The diff preview pane: state (`app`), rendering (`ui`), the event-loop
//! logic (`session`), and the thin `run` shell that wires threads, the
//! terminal, and the real PTY editor host around a `Session`.

pub mod app;
pub mod card;
pub mod compose;
pub mod editor;
pub mod highlight;
pub mod render;
pub mod session;
pub mod ui;

pub use app::{PreviewApp, ShowReq};
pub use session::{EditorHost, Event, Session};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEventKind};

use crate::config::Config;
use crate::git::Repo;
use crate::hostenv::HostEnv;
use crate::ipc::Conn;
use crate::keymap::Keymap;

pub fn run() -> Result<()> {
    crate::logx::init_panic_hook();

    let cfg = Config::load();
    // Bad [keybindings] must not stop startup; fall back to defaults.
    let keys = Keymap::build(&cfg.keybindings).unwrap_or_else(|err| {
        crate::logx::log(format!("preview: keybindings ignored: {err}"));
        Keymap::build(&Default::default()).expect("default keymap is valid")
    });
    let repo = resolve_repo()?;
    let env = HostEnv::from_process();
    let in_herdr = env.in_herdr();
    let mut app = PreviewApp::new(cfg, repo, keys);

    let (tx, rx) = mpsc::channel::<Event>();
    // While an editor owns the PTY, the input thread must stop reading stdin
    // or it would steal nvim's keystrokes.
    let input_paused = Arc::new(AtomicBool::new(false));
    spawn_input_thread(tx.clone(), Arc::clone(&input_paused));

    // Preview is the IPC listener. `accept` blocks, so do it on a thread and
    // render the "waiting…" splash meanwhile; the connected `Conn` arrives as
    // an event. Standalone (no socket) skips IPC entirely.
    if let Some(sock) = env.socket.clone() {
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

    let mut session = Session::new(app, env, tx);

    let mut terminal = ratatui::init();
    // `key_disambiguation` is what lets the note composer see shift+enter as
    // a newline rather than a save. The guard leaves both modes on every exit
    // path, panic included.
    let term = crate::term::TermGuard::enter(crate::term::Modes {
        mouse: true,
        key_disambiguation: true,
    });
    let result = event_loop(&mut terminal, &mut session, &rx, &input_paused, &term);
    drop(term);
    ratatui::restore();

    if session.app.close_view && in_herdr {
        crate::orchestrate::spawn_close();
    }
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    session: &mut Session,
    rx: &mpsc::Receiver<Event>,
    input_paused: &Arc<AtomicBool>,
    term: &crate::term::TermGuard,
) -> Result<()> {
    loop {
        session.tick();
        terminal.draw(|frame| ui::render(frame, &mut session.app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                let mut host = TerminalHost {
                    term,
                    terminal,
                    input_paused,
                };
                session.on_event(event, &mut host);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        if session.should_quit() {
            session.drop_conn(); // list sees EOF promptly
            return Ok(());
        }
    }
}

/// The real editor host: suspends the ratatui terminal, pauses the input
/// thread, releases the mouse, and runs the child on this pane's PTY.
struct TerminalHost<'a> {
    terminal: &'a mut ratatui::DefaultTerminal,
    input_paused: &'a Arc<AtomicBool>,
    term: &'a crate::term::TermGuard,
}

impl EditorHost for TerminalHost<'_> {
    fn run(&mut self, cwd: &Path, argv: &[String], envs: &[(String, String)]) -> Result<bool> {
        self.input_paused.store(true, Ordering::SeqCst);
        // The child owns the terminal's modes until this guard drops.
        let suspended = self.term.suspend();
        let result = editor::run_on_pty(self.terminal, cwd, argv, envs);
        drop(suspended);
        self.input_paused.store(false, Ordering::SeqCst);
        result
    }
}

/// Input reader on its own thread; forwards key presses only. Uses
/// `poll` + `read` so it can stop touching stdin while `paused` is set
/// (an editor owns the PTY then).
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
                    Ok(CtEvent::Mouse(m)) => {
                        if tx.send(Event::Mouse(m)).is_err() {
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

/// Repo root: `GITVIEW_REPO` under herdr, else discover from cwd (dev mode).
fn resolve_repo() -> Result<Repo> {
    match std::env::var_os("GITVIEW_REPO") {
        Some(dir) => Repo::discover(&PathBuf::from(dir)),
        None => Repo::discover(Path::new(&std::env::current_dir()?)),
    }
}
