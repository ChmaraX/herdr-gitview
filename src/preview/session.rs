//! The preview pane's event-loop logic, extracted into a pumpable state
//! machine (same shape as `list::session`). The only piece that cannot run
//! headless — suspending the TUI to hand the PTY to an editor — is abstracted
//! behind [`EditorHost`], implemented over the real terminal in `run()` and
//! by a recorder in the scenario tests.

use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};

use super::app::{self, PreviewApp, ShowReq};
use super::{highlight, render};
use crate::config::Config;
use crate::git::{Repo, Scope};
use crate::hostenv::HostEnv;
use crate::ipc::{Conn, ToList, ToPreview};
use crate::popup::{Answer, Popups};

/// Runs an argv on the pane's PTY with the TUI suspended. The real
/// implementation wraps the terminal (see `preview::run`); tests record.
pub trait EditorHost {
    fn run(&mut self, cwd: &Path, argv: &[String], envs: &[(String, String)]) -> Result<bool>;
}

/// Which popup interaction an answer belongs to.
enum PreviewPopup {
    Annotate,
    PickAgent,
}

/// Everything the session can be woken by.
pub enum Event {
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

pub struct Session {
    pub app: PreviewApp,
    pub env: HostEnv,
    tx: Sender<Event>,
    work_tx: Sender<ShowReq>,
    conn: Option<Conn>,
    popups: Popups<PreviewPopup>,
    last_notes_rev: u64,
}

impl Session {
    /// Builds the session and spawns the latest-wins diff worker (real git,
    /// real highlighting — also under test).
    pub fn new(app: PreviewApp, env: HostEnv, tx: Sender<Event>) -> Session {
        let work_tx = spawn_diff_worker(
            tx.clone(),
            Repo {
                root: app.repo.root.clone(),
            },
            app.cfg.clone(),
        );
        Session {
            app,
            env,
            tx,
            work_tx,
            conn: None,
            popups: Popups::default(),
            last_notes_rev: 0,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.app.should_quit
    }

    /// Drop the IPC link (so the peer sees EOF promptly on quit).
    pub fn drop_conn(&mut self) {
        self.conn = None;
    }

    /// Shrink the popup liveness probe interval (scenario tests).
    pub fn set_popup_liveness(&mut self, interval: std::time::Duration) {
        self.popups.liveness_interval = interval;
    }

    /// Adopt a raw connection: reader thread + Ready handshake.
    fn adopt_conn(&mut self, conn: Conn) {
        let (ipc_tx, ipc_rx) = mpsc::channel::<ToPreview>();
        let mut conn = conn.spawn_reader(ipc_tx);
        let tx = self.tx.clone();
        thread::spawn(move || {
            while let Ok(msg) = ipc_rx.recv() {
                if tx.send(Event::Ipc(msg)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Event::IpcClosed);
        });
        let _ = conn.send(&ToList::Ready);
        self.conn = Some(conn);
        self.app.on_connected();
    }

    // ---- event handling ---------------------------------------------------

    pub fn on_event(&mut self, event: Event, host: &mut dyn EditorHost) {
        match event {
            Event::Key(key) => self.app.on_key(key),
            Event::Mouse(m) => self.app.on_mouse(m.kind, m.row),
            Event::Connected(conn) => self.adopt_conn(conn),
            // Edit / GitInPane suspend the TUI, so they go through the host.
            Event::Ipc(ToPreview::Edit { file }) => {
                self.run_editor(&file, host);
                if let Some(req) = self.app.current.clone() {
                    let _ = self.work_tx.send(req); // file changed — re-diff
                }
                self.send(&ToList::EditDone { file });
            }
            Event::Ipc(ToPreview::GitInPane { argv }) => {
                let ok = self.run_git_in_pane(&argv, host);
                if let Some(req) = self.app.current.clone() {
                    let _ = self.work_tx.send(req);
                }
                self.send(&ToList::GitDone { ok });
            }
            Event::Ipc(msg) => self.on_ipc(msg),
            Event::IpcClosed => {
                // The list (and thus the whole view) is gone — exit cleanly.
                self.app.should_quit = true;
            }
            Event::Diff { req, result } => self.app.apply_diff(&req, result),
        }
    }

    fn on_ipc(&mut self, msg: ToPreview) {
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
                self.app.begin_show(req.clone());
                let _ = self.work_tx.send(req);
            }
            ToPreview::Scroll { delta } => self.app.scroll_by(delta),
            ToPreview::Page { down, full } => self.app.page(down, full),
            ToPreview::Clear => self.app.clear(),
            ToPreview::AddNote { file, text, cached } => self.app.add_file_note(file, text, cached),
            ToPreview::FocusNote { id } => self.app.focus_note(id),
            ToPreview::EditNote { id, text } => self.app.edit_note(id, text),
            ToPreview::DeleteNote { id } => self.app.delete_note(id),
            ToPreview::SendNotes => {
                if self.app.notes.is_empty() {
                    self.app.flash("no notes yet");
                } else {
                    self.app.popup_request = Some(app::PopupReq::PickAgent);
                }
            }
            ToPreview::Quit => self.app.should_quit = true, // list-initiated
            // Handled in `on_event` (they need the editor host).
            ToPreview::Edit { .. } | ToPreview::GitInPane { .. } => {}
        }
    }

    // ---- per-iteration work -----------------------------------------------

    pub fn tick(&mut self) {
        self.open_requested_popups();
        self.poll_popups();

        // `n` in the preview opens the notes view over in the list pane.
        if self.app.notes_view_request {
            self.app.notes_view_request = false;
            self.send(&ToList::ShowNotesView);
        }

        // Keep the list's notes view in sync whenever the store changes
        // (add/edit/delete/clear all funnel through here).
        if self.app.notes_rev != self.last_notes_rev {
            self.last_notes_rev = self.app.notes_rev;
            let snapshot: Vec<_> = self
                .app
                .notes
                .iter()
                .map(|n| crate::ipc::NoteMeta {
                    id: n.id,
                    file: n.file.clone(),
                    start: n.start,
                    end: n.end,
                    text: n.text.clone(),
                    cached: n.cached,
                })
                .collect();
            self.send(&ToList::Notes { notes: snapshot });
        }

        if self.app.should_quit {
            // Drop the link so the list sees EOF promptly.
            self.conn = None;
        }
    }

    fn open_requested_popups(&mut self) {
        // A request while another popup is open is put back for later.
        if let Some(req) = self.app.popup_request.take() {
            match req {
                app::PopupReq::Annotate if self.popups.is_open() => {
                    self.app.popup_request = Some(app::PopupReq::Annotate);
                }
                app::PopupReq::Annotate => {
                    let title = self
                        .app
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
                    if !self.popups.open(
                        &self.env,
                        "annotate",
                        &envs,
                        (64, 8),
                        PreviewPopup::Annotate,
                    ) {
                        self.app.pending_note = None;
                        self.app
                            .flash("couldn't open the note popup (see debug log)");
                    }
                }
                app::PopupReq::PickAgent if self.popups.is_open() => {
                    self.app.popup_request = Some(app::PopupReq::PickAgent);
                }
                app::PopupReq::PickAgent => {
                    let agents = crate::popup::workspace_agents();
                    if agents.is_empty() {
                        self.app.flash("no agent panes in this workspace");
                    } else {
                        let json = serde_json::to_string(&agents).unwrap_or_default();
                        let envs = [
                            ("GITVIEW_AGENTS".to_string(), json),
                            (
                                "GITVIEW_ASK_TEXT".to_string(),
                                format!("send {} note(s) to…", self.app.notes.len()),
                            ),
                        ];
                        let size = (74, (agents.len() as u16 + 6).min(14));
                        if !self.popups.open(
                            &self.env,
                            "pick-agent",
                            &envs,
                            size,
                            PreviewPopup::PickAgent,
                        ) {
                            self.app
                                .flash("couldn't open the agent picker (see debug log)");
                        }
                    }
                }
            }
        }
    }

    fn poll_popups(&mut self) {
        // Popup outcomes (a dead popup pane just cancels the interaction).
        match self.popups.poll() {
            Some((PreviewPopup::Annotate, Answer::Text(text))) => {
                if text.is_empty() {
                    self.app.pending_note = None; // cancelled
                } else {
                    self.app.finish_annotate(text);
                }
            }
            Some((PreviewPopup::Annotate, Answer::Dead)) => {
                self.app.pending_note = None;
            }
            Some((PreviewPopup::PickAgent, Answer::Text(answer))) => {
                if answer != "cancel"
                    && let Some((pane, mode)) = answer.split_once('\t')
                {
                    match self.deliver_notes(pane, mode == "submit") {
                        Ok(agent) => {
                            self.app.clear_notes();
                            self.app.flash(format!("notes sent to {agent}"));
                        }
                        Err(err) => self.app.flash(format!("send failed: {err}")),
                    }
                }
            }
            Some((PreviewPopup::PickAgent, Answer::Dead)) | None => {}
        }
    }

    // ---- editor -----------------------------------------------------------

    /// Open the configured editor on `file`, jumping to its first changed
    /// line (derived from the shown diff when it is for the same file).
    fn run_editor(&mut self, file: &Path, host: &mut dyn EditorHost) {
        let mut argv = self.app.cfg.editor.clone();
        // nvim gets a remote-control socket so the list pane can switch the
        // open file mid-session.
        let server = self.env.nvim_server();
        if let Some(server) = &server
            && argv.first().map(|e| e.contains("nvim")).unwrap_or(false)
        {
            let _ = std::fs::remove_file(server);
            argv.push("--listen".into());
            argv.push(server.display().to_string());
        }
        let same_file = self
            .app
            .current
            .as_ref()
            .map(|c| c.file == *file)
            .unwrap_or(false);
        if same_file && let Some(line) = self.app.first_change {
            argv.push(format!("+{line}"));
        }
        argv.push(self.app.repo.root.join(file).display().to_string());
        self.run_suspended(&argv, &[], host);
        if let Some(server) = &server {
            let _ = std::fs::remove_file(server);
        }
    }

    /// Run `git -C <root> <argv…>` interactively on this PTY (e.g. commit
    /// -e). Sets GIT_EDITOR from our config only when the user configured
    /// nothing.
    fn run_git_in_pane(&mut self, argv: &[String], host: &mut dyn EditorHost) -> bool {
        let mut full = vec![
            "git".to_string(),
            "-C".to_string(),
            self.app.repo.root.display().to_string(),
        ];
        full.extend(argv.iter().cloned());

        let mut envs = Vec::new();
        if std::env::var_os("GIT_EDITOR").is_none() && !self.has_core_editor() {
            envs.push(("GIT_EDITOR".to_string(), self.app.cfg.editor.join(" ")));
        }
        self.run_suspended(&full, &envs, host)
    }

    /// Common suspend→run→restore path; a failure lands in the Error state.
    fn run_suspended(
        &mut self,
        argv: &[String],
        envs: &[(String, String)],
        host: &mut dyn EditorHost,
    ) -> bool {
        let cwd = self.app.repo.root.clone();
        match host.run(&cwd, argv, envs) {
            Ok(ok) => ok,
            Err(err) => {
                self.app.state = app::State::Error(first_line(&err.to_string()));
                false
            }
        }
    }

    /// Does the user have an editor configured for git itself?
    fn has_core_editor(&self) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&self.app.repo.root)
            .args(["config", "--get", "core.editor"])
            .output()
            .map(|out| out.status.success() && !out.stdout.is_empty())
            .unwrap_or(false)
    }

    /// Compose the batched notes and type them into the agent pane's input
    /// (submit optionally presses enter). Returns the agent name on success.
    fn deliver_notes(&self, pane: &str, submit: bool) -> Result<String> {
        let mut msg = String::new();
        for note in &self.app.notes {
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

        let out = std::process::Command::new(&self.env.herdr_bin)
            .args(["pane", "send-text", pane, &msg])
            .output()?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        if submit {
            let _ = std::process::Command::new(&self.env.herdr_bin)
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

    /// Best-effort send to the list; a broken pipe just drops the link.
    fn send(&mut self, msg: &ToList) {
        if let Some(c) = &mut self.conn
            && c.send(msg).is_err()
        {
            self.conn = None;
        }
    }
}

/// Latest-wins diff runner: fetches the old/new file contents for a request
/// and builds the styled document off the UI thread.
fn spawn_diff_worker(tx: Sender<Event>, repo: Repo, cfg: Config) -> Sender<ShowReq> {
    let (work_tx, work_rx) = mpsc::channel::<ShowReq>();
    thread::spawn(move || {
        // The highlighter is expensive to set up — build it once per worker.
        let hl = highlight::Highlighter::new(cfg.theme);
        // Branch-scope base resolution cached per HEAD (it only moves when
        // HEAD does) so holding j doesn't spawn a git storm.
        let mut base_cache: Option<(String, String)> = None; // (head, merge_base)
        while let Ok(mut req) = work_rx.recv() {
            // Collapse a backlog to the newest request.
            while let Ok(newer) = work_rx.try_recv() {
                req = newer;
            }
            let result = fetch_contents(&repo, &cfg, &req, &mut base_cache).map(|(old, new)| {
                render::build(&req.file, &old, &new, &hl, cfg.theme, cfg.context_lines)
            });
            if tx.send(Event::Diff { req, result }).is_err() {
                break;
            }
        }
    });
    work_tx
}

/// The (old, new) content pair a request diffs, per scope/staged/commit.
/// `base_cache` holds `(head_sha, merge_base)` across calls.
fn fetch_contents(
    repo: &Repo,
    cfg: &Config,
    req: &ShowReq,
    base_cache: &mut Option<(String, String)>,
) -> Result<(String, String), String> {
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
            let head = repo.head_sha().unwrap_or_default();
            let mb = match base_cache {
                Some((cached_head, mb)) if *cached_head == head => mb.clone(),
                _ => {
                    let (_, mb) = repo.resolve_base(&cfg.base).map_err(err)?;
                    *base_cache = Some((head, mb.clone()));
                    mb
                }
            };
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
        Scope::Worktree if req.kind == crate::git::ChangeKind::Conflicted => {
            // Unmerged paths have no stage-0 entry; diff "ours" (stage 2,
            // falling back to HEAD) against the conflicted worktree file.
            let ours = match repo.file_at(":2", path).map_err(err)? {
                Some(content) => content,
                None => some(repo.file_at("HEAD", path))?,
            };
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((ours, new))
        }
        Scope::Worktree => {
            // Unstaged view: index vs working tree (untracked → empty old).
            let old = some(repo.file_at(":0", path))?;
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((old, new))
        }
    }
}

/// First line of an error message (git failures embed stderr).
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}
