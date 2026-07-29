//! The list pane's event-loop logic, extracted into a pumpable state machine.
//!
//! `Session` owns everything the loop needs besides threads and the terminal:
//! the `App`, the IPC link, popups, the show debounce, and editor-probe
//! state. `run()` feeds it events and calls `tick()`; the scenario tests do
//! exactly the same thing in-process — which is the whole point of the shape.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyEvent, MouseEvent};

use super::app::{self, App};
use crate::git::{FileEntry, Scope};
use crate::hostenv::HostEnv;
use crate::ipc::{Conn, ToList, ToPreview};
use crate::keymap::Action;
use crate::popup::{Answer, Popups};

/// Everything the session can be woken by.
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// The background connector reached the preview socket.
    Connected(Conn),
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
pub struct Shared {
    pub scope: Scope,
    pub merge_base: Option<String>,
    pub show_untracked: bool,
}

/// Which popup interaction an answer belongs to.
enum ListPopup {
    Confirm,
}

pub struct Session {
    pub app: App,
    pub env: HostEnv,
    tx: Sender<Event>,
    shared: Arc<Mutex<Shared>>,
    conn: Option<Conn>,
    popups: Popups<ListPopup>,
    popup_supported: bool,
    probe_pending: bool,
    show_dirty: bool,
    dirty_since: Instant,
    /// Debounce window for re-showing the diff while the cursor moves
    /// quickly. Tests set this to zero for determinism.
    pub show_debounce: Duration,
    /// Budget for `r`-triggered reconnect attempts.
    pub reconnect_budget: Duration,
    quit_sent: bool,
}

impl Session {
    pub fn new(mut app: App, env: HostEnv, tx: Sender<Event>, popup_supported: bool) -> Session {
        app.nvim_server = env.nvim_server();
        let shared = Arc::new(Mutex::new(Shared {
            scope: app.scope,
            merge_base: app.merge_base.clone(),
            show_untracked: app.cfg.show_untracked,
        }));
        Session {
            app,
            env,
            tx,
            shared,
            conn: None,
            popups: Popups::default(),
            popup_supported,
            probe_pending: false,
            show_dirty: true, // first frame sends the initial Show
            dirty_since: Instant::now(),
            show_debounce: Duration::from_millis(40),
            reconnect_budget: Duration::from_secs(2),
            quit_sent: false,
        }
    }

    /// Handle for the poll thread (scope/merge-base it should reload with).
    pub fn shared_handle(&self) -> Arc<Mutex<Shared>> {
        Arc::clone(&self.shared)
    }

    /// Shrink the popup liveness probe interval (scenario tests).
    pub fn set_popup_liveness(&mut self, interval: Duration) {
        self.popups.liveness_interval = interval;
    }

    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    pub fn should_quit(&self) -> bool {
        self.app.should_quit && self.quit_sent
    }

    /// Adopt a raw connection: spawn its reader, forward decoded frames into
    /// our event channel, mark the link live.
    fn adopt_conn(&mut self, conn: Conn) {
        let (ipc_tx, ipc_rx) = mpsc::channel::<ToList>();
        let conn = conn.spawn_reader(ipc_tx);
        let tx = self.tx.clone();
        thread::spawn(move || {
            while let Ok(msg) = ipc_rx.recv() {
                if tx.send(Event::Ipc(msg)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Event::IpcClosed);
        });
        self.conn = Some(conn);
        self.mark_dirty();
    }

    fn mark_dirty(&mut self) {
        self.show_dirty = true;
        self.dirty_since = Instant::now();
    }

    // ---- event handling ---------------------------------------------------

    pub fn on_event(&mut self, event: Event) {
        match event {
            Event::Connected(conn) => self.adopt_conn(conn),
            Event::Key(key) => self.on_key(key),
            Event::Mouse(m) => self.on_mouse(m),
            Event::EditorProbe { then, unsaved } => self.on_probe(then, unsaved),
            Event::Refresh(entries) => {
                self.app.apply_refresh(entries);
                // Content may have changed under the cursor — re-show.
                self.mark_dirty();
            }
            Event::Ipc(msg) => self.on_ipc(msg),
            Event::IpcClosed => self.conn = None, // preview gone; keep list usable
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        // Enter/commit need the IPC link + focus handoff, so they are
        // handled here; App::on_key covers the plain cases.
        let free = self.conn.is_some() && self.app.busy.is_none();
        let action = self.app.keys.action(&key);
        let before = show_key(&self.app);
        // An open modal captures every key — checked first so no
        // special-cased arm below can bypass it.
        if self.app.modal.is_some() {
            self.app.on_key(key);
        // Enter activates the selected row (open commit / edit file).
        } else if action == Some(Action::Edit) {
            self.activate_selection();
        } else if free && action == Some(Action::Commit) && self.app.mode == app::Mode::Files {
            self.start_commit();
        // `r` with a dead preview link retries the connection too.
        } else if action == Some(Action::Refresh) && self.conn.is_none() {
            if spawn_connector(&self.tx, self.env.socket.clone(), self.reconnect_budget) {
                self.app.set_status("reconnecting…");
            }
            self.app.on_key(key); // still do the refresh itself
        // Diff scroll/page keys are forwarded straight to the preview
        // (skipped while nvim owns that PTY).
        } else if self.app.busy.is_none()
            && let Some(msg) = scroll_message(&self.app, &key)
        {
            self.send(&msg);
        } else {
            self.app.on_key(key);
            self.sync_shared();
        }
        // Any path that changed what should be shown (moving the cursor, but
        // also entering a commit's files or the notes view) re-Shows.
        if show_key(&self.app) != before {
            self.mark_dirty();
        }
        // Content changed under the same selection (stage/discard).
        if self.app.needs_reshow {
            self.app.needs_reshow = false;
            self.mark_dirty();
        }
        // Browsing while a clean nvim is open closes it so the diff preview
        // returns (explicit q/c requests are picked up in `tick`).
        if self.app.busy.is_some() && !self.probe_pending {
            let moved = matches!(
                action,
                Some(Action::Down | Action::Up | Action::Top | Action::Bottom)
            ) && self.app.modal.is_none();
            if moved && self.app.editor_close_request.is_none() {
                self.spawn_probe(None);
            }
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let before = show_key(&self.app);
        let activate = self.app.on_mouse(m.kind, m.row);
        if activate {
            self.activate_selection();
        }
        if show_key(&self.app) != before {
            self.mark_dirty();
        }
    }

    fn on_probe(&mut self, then: Option<app::EditorThen>, unsaved: Option<bool>) {
        self.probe_pending = false;
        if self.app.busy.is_none() {
            // The editor already exited (EditDone won the race) — a stale
            // probe result must not schedule anything.
            return;
        }
        match (unsaved, then) {
            // Clean — already told to quit; EditDone resumes `then`.
            (Some(false), then) => self.app.after_edit = then,
            // Dirty + an action waiting → ask.
            (Some(true), Some(then)) => self.app.modal = Some(app::Modal::EditorClose { then }),
            // Dirty + just browsing → leave nvim alone, hint once.
            (Some(true), None) => self
                .app
                .set_status("editor has unsaved changes — stays open"),
            (None, Some(_)) => self.app.set_status("editor is open — close it first"),
            (None, None) => {}
        }
    }

    fn on_ipc(&mut self, msg: ToList) {
        match msg {
            ToList::Ready => {
                if matches!(self.app.active_status(), Some("connecting…")) {
                    self.app.status_msg = None;
                }
            }
            // Editor or commit finished on the preview PTY: same unlock /
            // refresh / refocus dance, then the message-or-resume tail.
            msg @ (ToList::EditDone { .. } | ToList::GitDone { .. }) => {
                self.app.on_edit_done();
                self.mark_dirty();
                self.focus_self();
                match msg {
                    ToList::EditDone { .. } => match self.app.after_edit.take() {
                        Some(app::EditorThen::QuitView) => self.app.should_quit = true,
                        Some(app::EditorThen::Commit) => self.start_commit(),
                        None => {}
                    },
                    ToList::GitDone { ok } => {
                        let summary = ok.then(|| self.app.repo.last_commit_summary()).flatten();
                        match summary {
                            Some(s) => self.app.set_status(format!("committed {s}")),
                            None => self.app.set_status("commit aborted"),
                        }
                    }
                    _ => unreachable!(),
                }
            }
            ToList::EditRequest => self.activate_selection(),
            ToList::ShowNotesView => {
                if self.app.mode != app::Mode::Notes && !self.app.notes.is_empty() {
                    self.app.toggle_notes_view();
                }
                self.focus_self();
                self.mark_dirty();
            }
            ToList::Notes { notes } => {
                self.app.notes = notes;
                if self.app.mode == app::Mode::Notes {
                    self.app.rebuild_rows();
                    if self.app.notes.is_empty() {
                        // Sent or last one deleted — nothing left to look at.
                        self.app.toggle_notes_view();
                    }
                }
            }
        }
    }

    // ---- per-iteration work -----------------------------------------------

    /// The loop's between-events work: editor-close requests, popup
    /// open/poll, request draining, the debounced Show flush, and the quit
    /// handshake. Call once per loop iteration (after any event).
    pub fn tick(&mut self) {
        // Explicit editor-close requests (q / c while nvim is open): probed
        // off-thread; a request arriving mid-probe waits here until the
        // probe settles instead of being dropped.
        if self.app.busy.is_some()
            && !self.probe_pending
            && let Some(then) = self.app.editor_close_request.take()
        {
            self.spawn_probe(Some(then));
        }
        if self.app.busy.is_none() {
            self.app.editor_close_request = None; // editor already gone
        }

        self.open_requested_popups();
        self.poll_popups();

        // `p`: hand off to the preview (it owns the notes + picker flow).
        if self.app.send_notes_request {
            self.app.send_notes_request = false;
            self.send(&ToPreview::SendNotes);
        }
        if let Some(id) = self.app.delete_note_request.take() {
            self.send(&ToPreview::DeleteNote { id });
        }

        // Debounced Show flush once the cursor has settled.
        if self.show_dirty && self.dirty_since.elapsed() >= self.show_debounce {
            if self.app.mode == app::Mode::Notes {
                // Hovering a note: show its file's diff + scroll to the card.
                if let Some(note) = self.app.selected_note() {
                    let id = note.id;
                    if let Some(msg) = note_show(&self.app, id) {
                        self.send(&msg);
                    }
                    self.send(&ToPreview::FocusNote { id });
                }
            } else {
                match current_show(&self.app) {
                    Some(msg) => self.send(&msg),
                    // Cursor resting on a directory row: keep showing the
                    // last diff instead of blanking the preview.
                    None if self.app.selected_dir().is_some() => {}
                    // Nothing selectable left (e.g. the last change was
                    // discarded/committed) — clear the stale diff.
                    None => self.send(&ToPreview::Clear),
                }
            }
            self.show_dirty = false;
        }

        if self.app.should_quit && !self.quit_sent {
            self.send(&ToPreview::Quit);
            self.quit_sent = true;
        }
    }

    fn open_requested_popups(&mut self) {
        // Popups (confirm dialogs) run through one manager with liveness
        // tracking: a popup pane dying without an answer cancels the
        // interaction instead of wedging it. Notes are written in the diff
        // pane's inline composer, so those requests travel over the link and
        // take the focus with them.
        if self.app.annotate_request.take().is_some() {
            // The composer needs the diff pane to be showing this file. Carry
            // the Show along instead of relying on the debounced one having
            // been flushed first — pressing `j` then `a` inside the debounce
            // window used to fail with "open that file's diff first".
            match current_show(&self.app) {
                Some(show) => {
                    self.hand_off(ToPreview::ComposeNote {
                        show: Box::new(show),
                    });
                    self.show_dirty = false; // the composer's Show is the current one
                }
                None => self.app.set_status("select a file to annotate"),
            }
        }
        if let Some(id) = self.app.edit_note_request.take() {
            // The preview owns the note, so it can re-show the right file
            // itself; nothing here needs to know which file that is.
            self.hand_off(ToPreview::ComposeEditNote { id });
        }
        // Confirm modals become native floating popup panes when herdr
        // supports them; the in-pane overlay is the fallback.
        if self.popup_supported
            && !self.app.modal_external
            && !self.popups.is_open()
            && matches!(
                self.app.modal,
                Some(app::Modal::Confirm { .. } | app::Modal::EditorClose { .. })
            )
            && self.open_popup_confirm()
        {
            self.app.modal_external = true;
        }
    }

    fn open_popup_confirm(&mut self) -> bool {
        let text = match &self.app.modal {
            Some(app::Modal::Confirm { text, .. }) => text.clone(),
            Some(app::Modal::EditorClose { .. }) => {
                "The editor has unsaved changes. Save them? (no discards)".to_string()
            }
            _ => return false,
        };
        self.popups.open(
            &self.env,
            "ask",
            &[("GITVIEW_ASK_TEXT".to_string(), text)],
            (60, 7),
            ListPopup::Confirm,
        )
    }

    fn poll_popups(&mut self) {
        // Popup outcomes, fed through the same handling as in-pane input.
        match self.popups.poll() {
            Some((ListPopup::Confirm, answer)) => {
                self.app.modal_external = false;
                let code = match answer {
                    Answer::Text(text) => match text.trim() {
                        "y" => crossterm::event::KeyCode::Char('y'),
                        "n" => crossterm::event::KeyCode::Char('n'),
                        _ => crossterm::event::KeyCode::Esc,
                    },
                    Answer::Dead => crossterm::event::KeyCode::Esc,
                };
                self.app
                    .on_key(KeyEvent::new(code, crossterm::event::KeyModifiers::NONE));
                if self.app.needs_reshow {
                    self.app.needs_reshow = false;
                    self.mark_dirty();
                }
            }
            None => {}
        }
    }

    // ---- actions needing the link / host ----------------------------------

    /// Enter (or a double-click): open the selected commit in the log view,
    /// or the selected file in the editor — remotely switching a running nvim.
    fn activate_selection(&mut self) {
        match self.app.mode {
            app::Mode::Log => self.app.open_commit(),
            app::Mode::Notes => {
                if let Some(note) = self.app.selected_note() {
                    self.app.edit_note_request = Some(note.id);
                }
            }
            app::Mode::Files | app::Mode::CommitFiles => {
                // Enter on a directory row collapses/expands it.
                if self.app.toggle_selected_dir() {
                    return;
                }
                if self.app.busy.is_some() {
                    self.remote_open();
                } else if self.conn.is_some() {
                    self.start_edit();
                } else if !self.env.in_herdr() {
                    self.app
                        .set_status("editing needs the preview pane (run inside herdr)");
                } else {
                    self.app
                        .set_status("preview not connected — press r to reconnect");
                }
            }
        }
    }

    /// Enter on the selected entry: guard deleted files, then hand the
    /// preview pane the Edit and the focus. Lockout ends on `EditDone`.
    /// If another pane of this tab already runs an nvim (sidebar mode),
    /// the file opens there instead and the diff preview stays up.
    fn start_edit(&mut self) {
        let Some((entry, _)) = self.app.selected_entry() else {
            return; // empty list or header row
        };
        // The editor always opens the *current* file — also from the history
        // view — so guard on what exists on disk, not on the change kind.
        if !self.app.repo.root.join(&entry.path).exists() {
            self.app
                .set_status("file no longer exists — nothing to edit");
            return;
        }
        let file = entry.path.clone();
        if self.open_in_tab_nvim(&file) {
            return;
        }
        self.send(&ToPreview::Edit { file: file.clone() });
        if self.conn.is_none() {
            return; // send failed — preview link just broke
        }
        self.app.busy = Some(format!("editing {}…", file.display()));
        self.focus_preview();
    }

    /// Try to open `path` in an nvim already running in another pane of
    /// this tab (e.g. the herdr-nvim sidebar, or a plain `nvim` pane).
    /// Returns true when the file was handed off (and that pane focused).
    fn open_in_tab_nvim(&mut self, path: &std::path::Path) -> bool {
        if !self.app.cfg.reuse_tab_nvim {
            return false;
        }
        let Some(own) = self.env.own_pane.clone() else {
            return false; // standalone — no herdr to ask
        };
        let mut exclude: Vec<&str> = Vec::new();
        if let Some(preview) = &self.env.preview_pane {
            exclude.push(preview);
        }
        let Some(target) = crate::herdr_cli::find_nvim_in_tab(&self.env.herdr_bin, &own, &exclude)
        else {
            return false;
        };
        let abs = self.app.repo.root.join(path);
        let editor = self.app.cfg.editor.first().cloned().unwrap_or_default();
        if crate::nvim::open_file(&editor, Some(&target.socket), &abs) {
            crate::herdr_cli::focus_pane(&self.env.herdr_bin, &target.pane_id);
            true
        } else {
            false // dead socket etc. — fall through to the preview editor
        }
    }

    /// Enter while the editor is running: nvim was started with `--listen`,
    /// so tell it to open the newly selected file instead of refusing.
    fn remote_open(&mut self) {
        let Some((entry, _)) = self.app.selected_entry() else {
            return;
        };
        let abs = self.app.repo.root.join(&entry.path);
        if !abs.exists() {
            self.app
                .set_status("file no longer exists — nothing to edit");
            return;
        }
        let editor = self.app.cfg.editor.first().cloned().unwrap_or_default();
        let server = self.env.nvim_server();
        if crate::nvim::open_file(&editor, server.as_deref(), &abs) {
            self.app.busy = Some(format!("editing {}…", entry.path.display()));
            self.focus_preview();
        } else if server.map(|s| s.exists()).unwrap_or(false) {
            self.app.set_status("could not switch the editor's file");
        } else {
            self.app.set_status("editor is open — close it first");
        }
    }

    /// `c`: preflight the staged set, then run `git commit -e` on the
    /// preview PTY (nvim opens the commit template there). Same lockout as
    /// editing.
    fn start_commit(&mut self) {
        match self.app.repo.staged_count() {
            Ok(0) => {
                self.app.set_status("nothing staged — s to stage files");
                return;
            }
            Err(err) => {
                self.app
                    .set_status(format!("commit preflight failed: {err}"));
                return;
            }
            Ok(_) => {}
        }
        self.send(&ToPreview::GitInPane {
            argv: vec!["commit".to_string(), "-e".to_string()],
        });
        if self.conn.is_none() {
            return;
        }
        self.app.busy = Some("committing…".to_string());
        self.focus_preview();
    }

    /// Probe the remote nvim on a background thread: a clean editor is told
    /// to quit immediately; the result comes back as `Event::EditorProbe`.
    fn spawn_probe(&mut self, then: Option<app::EditorThen>) {
        self.probe_pending = true;
        let tx = self.tx.clone();
        let editor = self.app.cfg.editor.first().cloned().unwrap_or_default();
        let server = self.env.nvim_server();
        thread::spawn(move || {
            let unsaved = crate::nvim::has_unsaved(&editor, server.as_deref());
            if unsaved == Some(false) {
                crate::nvim::request_close(&editor, server.as_deref(), false);
            }
            let _ = tx.send(Event::EditorProbe { then, unsaved });
        });
    }

    /// Send a composer request to the diff pane and give it the focus, which
    /// is where the keystrokes need to land.
    fn hand_off(&mut self, msg: ToPreview) {
        if self.conn.is_none() {
            self.app
                .set_status("preview not connected — press r to reconnect");
            return;
        }
        self.send(&msg);
        self.focus_preview();
    }

    fn focus_preview(&self) {
        if let Some(preview) = &self.env.preview_pane {
            crate::herdr_cli::focus_pane(&self.env.herdr_bin, preview);
        }
    }

    fn focus_self(&self) {
        if let Some(own) = &self.env.own_pane {
            crate::herdr_cli::focus_pane(&self.env.herdr_bin, own);
        }
    }

    /// Best-effort send; a broken pipe just drops the preview link.
    fn send(&mut self, msg: &ToPreview) {
        if let Some(c) = &mut self.conn
            && c.send(msg).is_err()
        {
            self.conn = None;
        }
    }

    fn sync_shared(&self) {
        if let Ok(mut s) = self.shared.lock() {
            s.scope = self.app.scope;
            s.merge_base = self.app.merge_base.clone();
        }
    }
}

/// Connect to the preview socket on a background thread; the raw `Conn`
/// arrives as `Event::Connected`. Returns false in standalone mode.
pub fn spawn_connector(tx: &Sender<Event>, socket: Option<PathBuf>, budget: Duration) -> bool {
    let Some(sock) = socket else {
        return false; // standalone — no preview to talk to
    };
    let tx = tx.clone();
    thread::spawn(move || match Conn::connect_retry(&sock, budget) {
        Ok(conn) => {
            let _ = tx.send(Event::Connected(conn));
        }
        Err(err) => crate::logx::log(format!("list: preview connect failed: {err}")),
    });
    true
}

// ---- pure message builders (unit-testable) --------------------------------

/// The Show message for the current selection, or `None` when nothing
/// diffable is selected (headers, commit rows, empty list).
fn current_show(app: &App) -> Option<ToPreview> {
    let (e, section) = app.selected_entry()?;
    let commit = match app.mode {
        app::Mode::CommitFiles => Some(app.commit.as_ref()?.sha.clone()),
        _ => None,
    };
    Some(ToPreview::Show {
        file: e.path.clone(),
        orig_path: e.orig_path.clone(),
        scope: app.scope,
        // Only the staged section shows the --cached diff; Flat (branch
        // scope / commit files) never does.
        cached: section.cached(),
        kind: e.kind,
        commit,
    })
}

/// Identity of what the preview is showing; a change here means "re-Show".
fn show_key(app: &App) -> Option<(PathBuf, Scope, bool, Option<String>)> {
    if app.mode == app::Mode::Notes {
        let note = app.selected_note()?;
        return Some((
            note.file.clone(),
            app.scope,
            false,
            Some(format!("note-{}", note.id)),
        ));
    }
    let (e, section) = app.selected_entry()?;
    let commit = match app.mode {
        app::Mode::CommitFiles => app.commit.as_ref().map(|c| c.sha.clone()),
        _ => None,
    };
    Some((e.path.clone(), app.scope, section.cached(), commit))
}

/// The Show for a hovered note: its file's live worktree diff.
fn note_show(app: &App, id: u64) -> Option<ToPreview> {
    let note = app.notes.iter().find(|n| n.id == id)?;
    let file = &note.file;
    // Prefer real entry metadata when the file is among the current entries;
    // otherwise synthesize (kind only affects rename pathspecs).
    let entry = app.entries.iter().find(|e| &e.path == file);
    Some(ToPreview::Show {
        file: file.clone(),
        orig_path: entry.and_then(|e| e.orig_path.clone()),
        scope: Scope::Worktree,
        cached: note.cached,
        kind: entry
            .map(|e| e.kind)
            .unwrap_or(crate::git::ChangeKind::Modified),
        commit: None,
    })
}

/// Translate a diff scroll/page key into the message the preview
/// understands, or `None` if this key is not a scroll key.
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
