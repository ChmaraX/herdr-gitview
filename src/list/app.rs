//! List-pane application state and the input→action logic.
//!
//! `App` is deliberately thread-agnostic: it owns no channels or terminals.
//! The event loop in `list::run` feeds it key events and refreshed entry
//! vectors, then renders it via `list::ui`. Keeping git/threading out of here
//! makes the state easy to unit- and render-test.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::KeyEvent;

use crate::config::Config;
use crate::git::{ChangeKind, CommitInfo, FileEntry, Repo, Scope, StageState};
use crate::keymap::{Action, Keymap};

/// How long a transient footer message stays visible on screen.
const STATUS_TTL: Duration = Duration::from_secs(3);

/// How many commits the history view loads.
const LOG_LIMIT: usize = 200;

/// What the list is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Working-tree / branch changes (the normal view).
    Files,
    /// `git log` — pick a commit to inspect.
    Log,
    /// The files changed by the selected commit.
    CommitFiles,
    /// The batched review notes.
    Notes,
}

/// One visual row of the list. Sections group entries VSCode-style: a file
/// with both staged and unstaged changes appears in *both* sections, and the
/// section decides which diff the preview shows (`staged` → `--cached`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRow {
    Header { title: &'static str, count: usize },
    Entry { idx: usize, staged: bool },
    Commit(usize),
    Note(usize),
}

impl ListRow {
    pub fn selectable(&self) -> bool {
        !matches!(self, ListRow::Header { .. })
    }
}

/// A centered overlay that captures all keys while open.
pub enum Modal {
    Help,
    /// Yes/no question; `y`/enter runs the pending action, `n`/esc cancels.
    Confirm {
        text: String,
        pending: PendingAction,
    },
    /// The editor has unsaved changes and something needs it closed:
    /// `y` saves & closes, `n` discards & closes, esc cancels.
    EditorClose {
        then: EditorThen,
    },
}

/// What a confirmed modal should do.
pub enum PendingAction {
    Discard,
}

/// What to do once the editor has been closed (delivered via EditDone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorThen {
    QuitView,
    Commit,
}

pub struct App {
    pub repo: Repo,
    pub cfg: Config,
    pub keys: Keymap,

    pub mode: Mode,
    pub scope: Scope,

    /// Entries backing the current Files/CommitFiles view.
    pub entries: Vec<FileEntry>,
    /// Commits backing the Log view.
    pub commits: Vec<CommitInfo>,
    /// The commit whose files the CommitFiles view shows.
    pub commit: Option<CommitInfo>,

    /// Visual rows derived from the vectors above; `cursor` indexes this and
    /// always sits on a selectable row.
    pub rows: Vec<ListRow>,
    pub cursor: usize,
    pub list_offset: usize,

    /// Branch-scope base ref and its merge-base with HEAD, resolved lazily on
    /// the first scope toggle.
    pub base: String,
    pub merge_base: Option<String>,

    /// Current branch name for the header (None = detached HEAD).
    pub branch: Option<String>,

    /// Transient footer message + when it was set.
    pub status_msg: Option<(String, Instant)>,
    pub should_quit: bool,
    pub modal: Option<Modal>,
    /// Header text while the preview PTY is busy (editor or commit): actions
    /// that need that PTY are refused until EditDone/GitDone.
    pub busy: Option<String>,
    /// Set when the shown diff's *content* changed without the selection
    /// changing (stage toggle, discard, edit) — the run loop re-Shows and
    /// clears it.
    pub needs_reshow: bool,
    /// Action to resume after the editor finishes closing (EditDone).
    pub after_edit: Option<EditorThen>,
    /// Set by `on_key` when an action needs the editor closed; the run loop
    /// picks it up and probes nvim on a background thread (no UI stall).
    pub editor_close_request: Option<EditorThen>,
    /// Last left-click (row, time) for double-click detection.
    last_click: Option<(usize, Instant)>,
    /// True while the current modal is being shown as a native herdr popup
    /// pane instead of the in-pane overlay (the answer arrives via file).
    pub modal_external: bool,
    /// Review notes (owned by the preview; mirrored for the notes view):
    /// `(file, start, end, text, commit-context)`.
    pub notes: Vec<(std::path::PathBuf, u32, u32, String, Option<String>)>,
    /// Where `n` was pressed, so the notes view can return there.
    notes_return: Option<Mode>,
    pub annotate_request: Option<std::path::PathBuf>,
    pub send_notes_request: bool,
    /// Edit/delete requests for the run loop (they need popups / the conn).
    pub edit_note_request: Option<(usize, String)>,
    pub delete_note_request: Option<usize>,
}

impl App {
    /// Load the initial worktree status and build the app.
    pub fn new(repo: Repo, cfg: Config, keys: Keymap) -> Result<App> {
        let entries = repo.worktree_status(cfg.show_untracked)?;
        Ok(App::from_entries(repo, cfg, keys, entries))
    }

    /// Build an app around an already-loaded entry vector (no git status call).
    /// Used by render tests and by `new`.
    pub fn from_entries(repo: Repo, cfg: Config, keys: Keymap, entries: Vec<FileEntry>) -> App {
        let branch = repo.head_branch();
        let mut app = App {
            repo,
            cfg,
            keys,
            mode: Mode::Files,
            scope: Scope::Worktree,
            entries,
            commits: Vec::new(),
            commit: None,
            rows: Vec::new(),
            cursor: 0,
            list_offset: 0,
            base: String::new(),
            merge_base: None,
            branch,
            status_msg: None,
            should_quit: false,
            modal: None,
            busy: None,
            needs_reshow: false,
            after_edit: None,
            editor_close_request: None,
            last_click: None,
            modal_external: false,
            notes: Vec::new(),
            notes_return: None,
            annotate_request: None,
            send_notes_request: false,
            edit_note_request: None,
            delete_note_request: None,
        };
        app.rebuild_rows();
        app
    }

    // ---- row derivation ---------------------------------------------------

    /// Rebuild `rows` from the current mode's backing vector, keeping the
    /// cursor on a selectable row.
    pub fn rebuild_rows(&mut self) {
        self.rows = match self.mode {
            Mode::Log => (0..self.commits.len()).map(ListRow::Commit).collect(),
            Mode::Notes => (0..self.notes.len()).map(ListRow::Note).collect(),
            Mode::CommitFiles => (0..self.entries.len())
                .map(|idx| ListRow::Entry { idx, staged: false })
                .collect(),
            Mode::Files if self.scope == Scope::Branch => (0..self.entries.len())
                .map(|idx| ListRow::Entry { idx, staged: false })
                .collect(),
            Mode::Files => self.grouped_rows(),
        };
        self.snap_cursor();
    }

    /// Worktree sections: conflicts, staged, changes. A partially staged file
    /// appears under both "staged" and "changes".
    fn grouped_rows(&self) -> Vec<ListRow> {
        let mut rows = Vec::new();
        let section =
            |title, rows: &mut Vec<ListRow>, filter: &dyn Fn(&FileEntry) -> bool, staged| {
                let idxs: Vec<usize> = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| filter(e))
                    .map(|(i, _)| i)
                    .collect();
                if !idxs.is_empty() {
                    rows.push(ListRow::Header {
                        title,
                        count: idxs.len(),
                    });
                    rows.extend(idxs.into_iter().map(|idx| ListRow::Entry { idx, staged }));
                }
            };
        section(
            "merge conflicts",
            &mut rows,
            &|e| e.kind == ChangeKind::Conflicted,
            false,
        );
        section(
            "staged changes",
            &mut rows,
            &|e| {
                e.kind != ChangeKind::Conflicted
                    && matches!(e.stage, StageState::Staged | StageState::Partial)
            },
            true,
        );
        section(
            "changes",
            &mut rows,
            &|e| {
                e.kind != ChangeKind::Conflicted
                    && matches!(
                        e.stage,
                        StageState::Unstaged | StageState::Partial | StageState::Untracked
                    )
            },
            false,
        );
        rows
    }

    /// The selected entry and whether it sits in the staged section.
    pub fn selected_entry(&self) -> Option<(&FileEntry, bool)> {
        match self.rows.get(self.cursor)? {
            ListRow::Entry { idx, staged } => Some((self.entries.get(*idx)?, *staged)),
            _ => None,
        }
    }

    pub fn selected_commit(&self) -> Option<&CommitInfo> {
        match self.rows.get(self.cursor)? {
            ListRow::Commit(idx) => self.commits.get(*idx),
            _ => None,
        }
    }

    pub fn selected_note(&self) -> Option<usize> {
        match self.rows.get(self.cursor)? {
            ListRow::Note(idx) => Some(*idx),
            _ => None,
        }
    }

    // ---- event handling ---------------------------------------------------

    pub fn on_key(&mut self, ev: KeyEvent) {
        // Modals capture all keys, recognized or not.
        if self.modal.is_some() {
            self.on_modal_key(ev);
            return;
        }

        let Some(action) = self.keys.action(&ev) else {
            return;
        };

        // While the editor owns the preview PTY, actions that need it gone
        // close it gracefully (auto when clean, confirm modal when dirty).
        // Everything else — including history navigation — keeps working.
        if self.busy.is_some() {
            match action {
                Action::Quit if self.mode == Mode::Files => {
                    self.editor_close_request = Some(EditorThen::QuitView);
                    return;
                }
                Action::Commit => {
                    self.editor_close_request = Some(EditorThen::Commit);
                    return;
                }
                Action::Edit => return, // remote file switch happens in the run loop
                _ => {}
            }
        }

        match action {
            Action::Down => self.move_cursor(1),
            Action::Up => self.move_cursor(-1),
            Action::Top => self.cursor_to_edge(true),
            Action::Bottom => self.cursor_to_edge(false),
            Action::ToggleScope => self.toggle_scope(),
            Action::ToggleCached => self.set_status("staged/unstaged now follow the list sections"),
            Action::Refresh => self.force_refresh(),
            Action::Help => self.modal = Some(Modal::Help),
            Action::Log => self.toggle_log(),
            Action::NotesView => self.toggle_notes_view(),
            Action::Select => {}
            Action::Delete => {
                if self.mode == Mode::Notes {
                    self.delete_note_request = self.selected_note();
                }
            }
            Action::Annotate => match self.selected_entry() {
                Some((entry, _)) => self.annotate_request = Some(entry.path.clone()),
                None if self.mode == Mode::Notes => {
                    self.set_status("annotate from the files view (n to go back)")
                }
                None => self.set_status("select a file to annotate"),
            },
            Action::SendNotes => {
                if self.notes.is_empty() {
                    self.set_status("no notes yet — a annotates a file, v+a lines in the diff");
                } else {
                    self.send_notes_request = true;
                }
            }
            Action::Quit => self.back_or_quit(),
            Action::Stage => self.stage_toggle(),
            Action::Unstage => self.unstage_selected(),
            Action::Discard => self.open_discard_confirm(),

            // Diff scroll keys are intercepted by the run loop and forwarded
            // to the preview pane over IPC, so they are no-ops here.
            Action::ScrollDown
            | Action::ScrollUp
            | Action::HalfPageDown
            | Action::HalfPageUp
            | Action::DiffTop
            | Action::DiffBottom => {}

            // Edit/Commit are intercepted by the run loop (they need the IPC
            // link); reaching here means there is no preview connection.
            Action::Edit | Action::Commit => {
                if std::env::var_os("HERDR_PANE_ID").is_none() {
                    self.set_status("editing needs the preview pane (run inside herdr)")
                } else {
                    self.set_status("preview not connected — press r to reconnect")
                }
            }
        }
    }

    // ---- mouse ------------------------------------------------------------

    /// Wheel moves the cursor; a click selects the row under the pointer.
    /// Returns true when a double-click should *activate* the row (like
    /// Enter) — the run loop owns activation because it needs the IPC link.
    pub fn on_mouse(&mut self, kind: crossterm::event::MouseEventKind, y: u16) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};
        if self.modal.is_some() {
            return false; // modals are keyboard-only
        }
        match kind {
            MouseEventKind::ScrollDown => self.move_cursor(1),
            MouseEventKind::ScrollUp => self.move_cursor(-1),
            MouseEventKind::Down(MouseButton::Left) if y >= 1 => {
                let idx = self.list_offset + (y - 1) as usize;
                if self.rows.get(idx).map(ListRow::selectable).unwrap_or(false) {
                    self.cursor = idx;
                    // Double-click on the same row = activate.
                    let now = Instant::now();
                    let double = matches!(
                        self.last_click,
                        Some((i, at)) if i == idx && now.duration_since(at) < Duration::from_millis(400)
                    );
                    self.last_click = Some((idx, now));
                    return double;
                }
            }
            _ => {}
        }
        false
    }

    // ---- history view -----------------------------------------------------

    /// `l`: enter the log view; `l` again (or q/esc) returns to files.
    fn toggle_log(&mut self) {
        match self.mode {
            Mode::Notes => self.set_status("leave the notes view first (n)"),
            Mode::Files => match self.repo.log_commits(LOG_LIMIT) {
                Ok(commits) if commits.is_empty() => self.set_status("no commits yet"),
                Ok(commits) => {
                    self.commits = commits;
                    self.mode = Mode::Log;
                    self.cursor = 0;
                    self.rebuild_rows();
                }
                Err(err) => self.set_status(format!("log failed: {err}")),
            },
            Mode::Log | Mode::CommitFiles => self.leave_history(),
        }
    }

    /// Enter on a commit: show its files.
    pub fn open_commit(&mut self) {
        let Some(info) = self.selected_commit().cloned() else {
            return;
        };
        match self.repo.commit_files(&info.sha) {
            Ok(entries) => {
                self.entries = entries;
                self.commit = Some(info);
                self.mode = Mode::CommitFiles;
                self.cursor = 0;
                self.rebuild_rows();
            }
            Err(err) => self.set_status(format!("commit files failed: {err}")),
        }
    }

    /// `n`: open the notes view from anywhere; `n`/q returns to where you
    /// came from (files, log, or a commit's files).
    fn toggle_notes_view(&mut self) {
        match self.mode {
            Mode::Notes => {
                self.mode = self.notes_return.take().unwrap_or(Mode::Files);
                self.cursor = 0;
                if self.mode == Mode::Files {
                    self.force_refresh();
                }
                self.rebuild_rows();
            }
            mode => {
                if self.notes.is_empty() {
                    self.set_status("no notes yet");
                    return;
                }
                self.notes_return = Some(mode);
                self.mode = Mode::Notes;
                self.cursor = 0;
                self.rebuild_rows();
            }
        }
    }

    /// q/esc: CommitFiles → Log → Files → actually quit.
    fn back_or_quit(&mut self) {
        match self.mode {
            Mode::CommitFiles => {
                self.commit = None;
                self.mode = Mode::Log;
                self.entries = Vec::new();
                self.cursor = 0;
                self.rebuild_rows();
                // Restore the cursor onto the commit we came from? The log
                // vector is unchanged, so position 0 is fine and predictable.
            }
            Mode::Log => self.leave_history(),
            Mode::Notes => self.toggle_notes_view(),
            Mode::Files => self.should_quit = true,
        }
    }

    fn leave_history(&mut self) {
        self.mode = Mode::Files;
        self.commit = None;
        self.cursor = 0;
        self.force_refresh();
        self.rebuild_rows();
    }

    // ---- modals -----------------------------------------------------------

    /// Keys while a modal is open. Help: any key closes. Confirm: y/enter
    /// runs the pending action, n/esc cancels, anything else is ignored.
    fn on_modal_key(&mut self, ev: KeyEvent) {
        use crossterm::event::KeyCode;
        match self.modal.take() {
            Some(Modal::Help) | None => {}
            Some(Modal::Confirm { text, pending }) => match ev.code {
                KeyCode::Char('y') | KeyCode::Enter => self.run_pending(pending),
                KeyCode::Char('n') | KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Confirm { text, pending }), // keep open
            },
            Some(Modal::EditorClose { then }) => match ev.code {
                KeyCode::Char('y') | KeyCode::Enter => self.close_editor(true, then),
                KeyCode::Char('n') => self.close_editor(false, then),
                KeyCode::Esc => {}
                _ => self.modal = Some(Modal::EditorClose { then }), // keep open
            },
        }
    }

    // ---- graceful editor close --------------------------------------------

    /// Quit the remote nvim (`:wqa` / `:qa!`); the resumed action runs when
    /// its exit comes back as EditDone. One remote-send is fast enough to do
    /// inline (the *probing* is what happens on a background thread).
    pub fn close_editor(&mut self, save: bool, then: EditorThen) {
        let keys = if save {
            "<C-\\><C-n>:wqa<CR>"
        } else {
            "<C-\\><C-n>:qa!<CR>"
        };
        let editor = self.cfg.editor.first().cloned().unwrap_or_default();
        match editor_remote(&editor, &["--remote-send", keys]) {
            Some(out) if out.status.success() => self.after_edit = Some(then),
            _ => self.set_status("could not close the editor"),
        }
    }

    fn run_pending(&mut self, pending: PendingAction) {
        match pending {
            PendingAction::Discard => self.discard_selected(),
        }
    }

    // ---- stage / discard (worktree scope) ---------------------------------

    /// `s`: section-aware — in "staged changes" it unstages, in "changes" it
    /// stages. The file then moves to the other section (VSCode-style).
    fn stage_toggle(&mut self) {
        if !self.can_stage() {
            return;
        }
        let Some((entry, staged_section)) = self.selected_entry() else {
            return;
        };
        let result = if staged_section {
            self.repo.unstage(&entry.path)
        } else {
            self.repo.stage(&entry.path)
        };
        if let Err(err) = result {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        self.force_refresh();
        self.needs_reshow = true;
        let all_staged =
            !self.entries.is_empty() && self.entries.iter().all(|e| e.stage == StageState::Staged);
        if all_staged {
            self.set_status("all changes staged — c to commit");
        }
    }

    /// `u`: explicitly unstage the selected file, whichever section it's in.
    fn unstage_selected(&mut self) {
        if !self.can_stage() {
            return;
        }
        let Some((entry, _)) = self.selected_entry() else {
            return;
        };
        if matches!(entry.stage, StageState::Unstaged | StageState::Untracked) {
            self.set_status("nothing staged for this file");
            return;
        }
        if let Err(err) = self.repo.unstage(&entry.path) {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        self.force_refresh();
        self.needs_reshow = true;
    }

    /// `x`: ask before throwing changes away (refused for conflicts).
    fn open_discard_confirm(&mut self) {
        if !self.can_stage() {
            return;
        }
        let Some((entry, _)) = self.selected_entry() else {
            return;
        };
        self.modal = Some(Modal::Confirm {
            text: format!(
                "Discard changes to {}? This cannot be undone. (y/n)",
                entry.path.display()
            ),
            pending: PendingAction::Discard,
        });
    }

    /// Shared guards for the mutating actions.
    fn can_stage(&mut self) -> bool {
        if self.mode != Mode::Files {
            self.set_status("read-only in history view");
            return false;
        }
        if self.scope != Scope::Worktree {
            self.set_status("works in working-tree scope (w)");
            return false;
        }
        match self.selected_entry() {
            Some((e, _)) if e.kind == ChangeKind::Conflicted => {
                self.set_status("resolve the conflict in the editor first");
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    fn discard_selected(&mut self) {
        let Some((entry, _)) = self.selected_entry() else {
            return;
        };
        if let Err(err) = self.repo.discard(entry) {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        self.force_refresh();
        self.needs_reshow = true;
    }

    // ---- refresh ----------------------------------------------------------

    /// Replace the entry vector (from an auto-refresh or a forced reload),
    /// keeping the cursor on the same path+section when possible.
    pub fn apply_refresh(&mut self, entries: Vec<FileEntry>) {
        if self.mode != Mode::Files {
            return; // history views are snapshots; ignore live refreshes
        }
        let keep = self.cursor_identity();
        self.entries = entries;
        self.rebuild_rows();
        self.restore_cursor(keep);
    }

    /// Editor/commit finished: unlock, reload the status (files changed on
    /// disk); cursor preservation handles moved entries.
    pub fn on_edit_done(&mut self) {
        self.busy = None;
        self.force_refresh();
        self.needs_reshow = true;
    }

    pub fn force_refresh(&mut self) {
        if self.mode != Mode::Files {
            return;
        }
        match self.load_entries() {
            Ok(entries) => {
                let keep = self.cursor_identity();
                self.entries = entries;
                self.rebuild_rows();
                self.restore_cursor(keep);
            }
            Err(err) => self.set_status(format!("refresh failed: {err}")),
        }
    }

    /// The footer message if one is set and still fresh.
    pub fn active_status(&self) -> Option<&str> {
        match &self.status_msg {
            Some((msg, at)) if at.elapsed() < STATUS_TTL => Some(msg.as_str()),
            _ => None,
        }
    }

    // ---- internals --------------------------------------------------------

    /// What the cursor points at, in a refresh-stable form.
    fn cursor_identity(&self) -> Option<(std::path::PathBuf, bool)> {
        self.selected_entry()
            .map(|(e, staged)| (e.path.clone(), staged))
    }

    fn restore_cursor(&mut self, keep: Option<(std::path::PathBuf, bool)>) {
        if let Some((path, staged)) = keep {
            // Same path + same section first; then same path anywhere.
            let find = |want_staged: Option<bool>| {
                self.rows.iter().position(|row| match row {
                    ListRow::Entry { idx, staged: s } => {
                        self.entries
                            .get(*idx)
                            .map(|e| e.path == path)
                            .unwrap_or(false)
                            && want_staged.map(|w| *s == w).unwrap_or(true)
                    }
                    _ => false,
                })
            };
            if let Some(i) = find(Some(staged)).or_else(|| find(None)) {
                self.cursor = i;
                return;
            }
        }
        self.snap_cursor();
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let mut i = self.cursor as i32;
        loop {
            i += delta;
            if i < 0 || i >= self.rows.len() as i32 {
                return; // hit an edge — stay where we were
            }
            if self.rows[i as usize].selectable() {
                self.cursor = i as usize;
                return;
            }
        }
    }

    fn cursor_to_edge(&mut self, top: bool) {
        let found = if top {
            self.rows.iter().position(ListRow::selectable)
        } else {
            self.rows.iter().rposition(ListRow::selectable)
        };
        if let Some(i) = found {
            self.cursor = i;
        }
    }

    /// Clamp the cursor into range and off header rows.
    fn snap_cursor(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let start = self.cursor.min(self.rows.len() - 1);
        // Nearest selectable at or after `start`, else before it.
        let after = (start..self.rows.len()).find(|i| self.rows[*i].selectable());
        let before = (0..start).rev().find(|i| self.rows[*i].selectable());
        self.cursor = after.or(before).unwrap_or(0);
    }

    fn toggle_scope(&mut self) {
        if self.mode != Mode::Files {
            self.set_status("read-only in history view");
            return;
        }
        match self.scope {
            Scope::Worktree => {
                if self.merge_base.is_none() {
                    let base = if self.cfg.base.is_empty() {
                        self.repo.detect_base()
                    } else {
                        self.cfg.base.clone()
                    };
                    match self.repo.merge_base(&base) {
                        Ok(mb) => {
                            self.base = base;
                            self.merge_base = Some(mb);
                        }
                        Err(err) => {
                            self.set_status(format!("no base found: {err}"));
                            return; // stay in worktree scope
                        }
                    }
                }
                self.scope = Scope::Branch;
            }
            Scope::Branch => self.scope = Scope::Worktree,
        }
        self.force_refresh();
    }

    fn load_entries(&self) -> Result<Vec<FileEntry>> {
        match self.scope {
            Scope::Worktree => self.repo.worktree_status(self.cfg.show_untracked),
            Scope::Branch => {
                let mb = self.merge_base.as_deref().unwrap_or("HEAD");
                self.repo.branch_changes(mb)
            }
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), Instant::now()));
    }
}

/// First line of an error (git errors embed stderr; the top line is the news).
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// Run an nvim remote command against the editor's `--listen` socket.
/// `None` = not remote-controllable (not nvim, or no live socket).
pub(crate) fn editor_remote(editor: &str, args: &[&str]) -> Option<std::process::Output> {
    if !editor.contains("nvim") {
        return None;
    }
    let server = crate::preview::editor_server_path()?;
    if !server.exists() {
        return None;
    }
    std::process::Command::new(editor)
        .arg("--server")
        .arg(&server)
        .args(args)
        .output()
        .ok()
}

/// Does the remote nvim hold modified buffers? Runs two child processes —
/// call from a background thread, never the UI loop.
pub(crate) fn editor_has_unsaved(editor: &str) -> Option<bool> {
    let out = editor_remote(
        editor,
        &[
            "--remote-expr",
            r#"len(filter(getbufinfo(), "v:val.changed"))"#,
        ],
    )?;
    if !out.status.success() {
        return None;
    }
    let count = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(!count.is_empty() && count != "0")
}
