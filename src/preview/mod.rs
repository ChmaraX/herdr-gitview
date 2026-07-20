//! The diff preview pane: state (`app`), rendering (`ui`), and the `run` event
//! loop that owns the IPC socket, the latest-wins diff worker, and input.

pub mod app;
pub mod editor;
pub mod highlight;
pub mod render;
pub mod ui;

pub use app::{PreviewApp, ShowReq};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind, MouseEvent};

use crate::config::Config;
use crate::git::{Repo, Scope};
use crate::ipc::{Conn, ToList, ToPreview};
use crate::keymap::Keymap;

/// Everything the main loop can be woken by.
enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// The list finished connecting; `Conn` is the live link.
    Connected(Conn),
    /// A message arrived from the list pane.
    Ipc(ToPreview),
    /// The list pane went away (socket EOF).
    IpcClosed,
    /// The diff worker produced a built document for `req`.
    Diff {
        req: ShowReq,
        result: Result<render::DiffDoc, String>,
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
    let work_tx = spawn_diff_worker(tx.clone(), Repo { root }, app.cfg.clone());

    let mut terminal = ratatui::init();
    enable_mouse();
    let result = event_loop(&mut terminal, &mut app, &rx, &tx, &work_tx, &input_paused);
    disable_mouse();
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
    // Popup answers we are waiting on.
    let mut annotate_answer: Option<std::path::PathBuf> = None;
    let mut pick_answer: Option<std::path::PathBuf> = None;
    let mut last_notes_rev = 0u64;

    loop {
        // Open popups the app requested (annotate input / agent picker).
        match app.popup_request.take() {
            Some(app::PopupReq::Annotate) if annotate_answer.is_none() => {
                let title = app
                    .pending_note
                    .as_ref()
                    .map(|n| {
                        if n.end == 0 {
                            format!("note for {}", n.file.display())
                        } else {
                            format!("note for {}:{}-{}", n.file.display(), n.start, n.end)
                        }
                    })
                    .unwrap_or_default();
                let envs = [("GITVIEW_ASK_TEXT".to_string(), title)];
                match crate::popup::spawn("annotate", &envs, 64, 8) {
                    Some(path) => annotate_answer = Some(path),
                    None => {
                        app.pending_note = None;
                        app.flash(
                            "popup failed — needs herdr ≥0.7.4 + re-linked plugin (see debug log)",
                        );
                    }
                }
            }
            Some(app::PopupReq::PickAgent) if pick_answer.is_none() => {
                let agents = crate::popup::workspace_agents();
                if agents.is_empty() {
                    app.flash("no agent panes in this workspace");
                } else {
                    let json = serde_json::to_string(&agents).unwrap_or_default();
                    let envs = [
                        ("GITVIEW_AGENTS".to_string(), json),
                        (
                            "GITVIEW_ASK_TEXT".to_string(),
                            format!("send {} note(s) to…", app.notes.len()),
                        ),
                    ];
                    match crate::popup::spawn(
                        "pick-agent",
                        &envs,
                        74,
                        (agents.len() as u16 + 6).min(14),
                    ) {
                        Some(path) => pick_answer = Some(path),
                        None => app.flash(
                            "popup failed — needs herdr ≥0.7.4 + re-linked plugin (see debug log)",
                        ),
                    }
                }
            }
            Some(_) => {} // a popup is already open
            None => {}
        }
        // Annotate answer: non-empty text commits the note.
        if let Some(text) = crate::popup::poll(&mut annotate_answer) {
            if text.is_empty() {
                app.pending_note = None; // cancelled
            } else {
                app.finish_annotate(text);
            }
        }
        // Agent-picker answer: "pane\tplace|submit" or "cancel".
        if let Some(answer) = crate::popup::poll(&mut pick_answer)
            && answer != "cancel"
            && let Some((pane, mode)) = answer.split_once('\t')
        {
            match deliver_notes(app, pane, mode == "submit") {
                Ok(agent) => {
                    app.clear_notes();
                    app.flash(format!("notes sent to {agent}"));
                }
                Err(err) => app.flash(format!("send failed: {err}")),
            }
        }

        // `n` in the preview opens the notes view over in the list pane.
        if app.notes_view_request {
            app.notes_view_request = false;
            if let Some(c) = conn.as_mut() {
                let _ = c.send(&ToList::ShowNotesView);
            }
        }

        // Keep the list's notes view in sync whenever the store changes
        // (add/edit/delete/clear all funnel through here).
        if app.notes_rev != last_notes_rev {
            last_notes_rev = app.notes_rev;
            if let Some(c) = conn.as_mut() {
                let snapshot: Vec<_> = app
                    .notes
                    .iter()
                    .map(|n| (n.file.clone(), n.start, n.end, n.text.clone()))
                    .collect();
                let _ = c.send(&ToList::Notes { notes: snapshot });
            }
        }

        terminal.draw(|frame| ui::render(frame, app))?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Key(key)) => app.on_key(key),
            Ok(Event::Mouse(m)) => app.on_mouse(m.kind, m.row),

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
            commit,
        } => {
            let req = ShowReq {
                file,
                orig_path,
                scope,
                cached,
                kind,
                commit,
            };
            app.begin_show(req.clone());
            let _ = work_tx.send(req);
        }
        ToPreview::Scroll { delta } => app.scroll_by(delta),
        ToPreview::Page { down, full } => app.page(down, full),
        ToPreview::Clear => app.clear(),
        ToPreview::AddNote { file, text } => app.add_file_note(file, text),
        ToPreview::FocusNote { idx } => app.focus_note(idx),
        ToPreview::EditNote { idx, text } => app.edit_note(idx, text),
        ToPreview::DeleteNote { idx } => app.delete_note(idx),
        ToPreview::SendNotes => {
            if app.notes.is_empty() {
                app.flash("no notes yet");
            } else {
                app.popup_request = Some(app::PopupReq::PickAgent);
            }
        }
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
    // nvim gets a remote-control socket so the list pane can switch the open
    // file mid-session (Enter on another row while the editor runs).
    let server = editor_server_path();
    if let Some(server) = &server
        && argv.first().map(|e| e.contains("nvim")).unwrap_or(false)
    {
        let _ = std::fs::remove_file(server);
        argv.push("--listen".into());
        argv.push(server.display().to_string());
    }
    let same_file = app
        .current
        .as_ref()
        .map(|c| c.file == *file)
        .unwrap_or(false);
    if same_file && let Some(line) = app.first_change {
        argv.push(format!("+{line}"));
    }
    argv.push(app.repo.root.join(file).display().to_string());
    run_suspended(terminal, app, &argv, &[], input_paused);
    if let Some(server) = &server {
        let _ = std::fs::remove_file(server);
    }
}

/// The nvim remote socket, derived from the view's IPC socket path (shared
/// convention with the list pane).
pub fn editor_server_path() -> Option<std::path::PathBuf> {
    let sock = std::env::var_os("GITVIEW_SOCKET")?;
    let mut path = std::path::PathBuf::from(sock);
    path.set_extension("nvim");
    Some(path)
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
    disable_mouse(); // the child (nvim) owns mouse reporting while it runs
    let result = editor::run_on_pty(terminal, &app.repo.root.clone(), argv, envs);
    enable_mouse();
    input_paused.store(false, Ordering::SeqCst);
    match result {
        Ok(ok) => ok,
        Err(err) => {
            app.state = app::State::Error(first_line(&err.to_string()));
            false
        }
    }
}

/// Compose the batched notes and type them into the agent pane's input
/// (submit optionally presses enter). Returns the agent name on success.
fn deliver_notes(app: &PreviewApp, pane: &str, submit: bool) -> Result<String> {
    let mut msg = String::new();
    for note in &app.notes {
        if note.end == 0 {
            msg.push_str(&format!("{} — {}\n", note.file.display(), note.text));
        } else {
            msg.push_str(&format!(
                "{}:{}-{} — {}\n",
                note.file.display(),
                note.start,
                note.end,
                note.text
            ));
        }
        if !note.snippet.is_empty() {
            msg.push_str("```diff\n");
            msg.push_str(&note.snippet);
            msg.push_str("```\n");
        }
    }

    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let out = std::process::Command::new(&bin)
        .args(["pane", "send-text", pane, &msg])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    if submit {
        let _ = std::process::Command::new(&bin)
            .args(["pane", "send-keys", pane, "enter"])
            .output();
    }
    let agent = crate::popup::workspace_agents()
        .into_iter()
        .find(|(id, _, _, _, _)| id == pane)
        .map(|(_, name, _, _, _)| name)
        .unwrap_or_else(|| pane.to_string());
    Ok(agent)
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

/// Latest-wins diff runner: fetches the old/new file contents for a request
/// and builds the styled document (syntax highlighting + tints) off the UI
/// thread. Returns the sender used to enqueue `ShowReq`s.
fn spawn_diff_worker(tx: Sender<Event>, repo: Repo, cfg: Config) -> Sender<ShowReq> {
    let (work_tx, work_rx) = mpsc::channel::<ShowReq>();
    thread::spawn(move || {
        // The highlighter is expensive to set up — build it once per process.
        let hl = highlight::Highlighter::new(&cfg.theme);
        while let Ok(mut req) = work_rx.recv() {
            // Collapse a backlog to the newest request.
            while let Ok(newer) = work_rx.try_recv() {
                req = newer;
            }
            let result = fetch_contents(&repo, &cfg, &req)
                .map(|(old, new)| render::build(&req.file, &old, &new, &hl, &cfg.theme));
            if tx.send(Event::Diff { req, result }).is_err() {
                break;
            }
        }
    });
    work_tx
}

/// The (old, new) content pair a request diffs, per scope/staged/commit.
fn fetch_contents(repo: &Repo, cfg: &Config, req: &ShowReq) -> Result<(String, String), String> {
    let path = &req.file;
    let old_path = req.orig_path.as_deref().unwrap_or(path);
    let err = |e: anyhow::Error| first_line(&e.to_string());
    let some =
        |r: Result<Option<String>, anyhow::Error>| r.map_err(err).map(Option::unwrap_or_default);

    if let Some(sha) = &req.commit {
        // One commit's change: parent vs commit (root commit → empty old).
        let old = some(repo.file_at(&format!("{sha}^"), old_path))?;
        let new = some(repo.file_at(sha, path))?;
        return Ok((old, new));
    }
    match req.scope {
        Scope::Branch => {
            let base = if cfg.base.is_empty() {
                repo.detect_base()
            } else {
                cfg.base.clone()
            };
            let mb = repo.merge_base(&base).map_err(err)?;
            let old = some(repo.file_at(&mb, old_path))?;
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((old, new))
        }
        Scope::Worktree if req.cached => {
            // Staged view: HEAD vs index.
            let old = some(repo.file_at("HEAD", old_path))?;
            let new = some(repo.file_at(":0", path))?;
            Ok((old, new))
        }
        Scope::Worktree => {
            // Unstaged view: index vs working tree (untracked → empty old).
            let old = some(repo.file_at(":0", path))?;
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((old, new))
        }
    }
}

pub(crate) fn enable_mouse() {
    use crossterm::execute;
    let _ = execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
}

pub(crate) fn disable_mouse() {
    use crossterm::execute;
    let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
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
