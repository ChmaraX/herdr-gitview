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
    Header {
        title: &'static str,
        count: usize,
    },
    /// A directory row in the file tree — selectable; Enter or a click
    /// collapses/expands the subtree below it. `path` (full path from the
    /// tree root, trailing slash) plus `staged` (which section, in the
    /// grouped worktree view) is its stable collapse identity.
    Dir {
        depth: usize,
        name: String,
        path: String,
        staged: bool,
        collapsed: bool,
    },
    Entry {
        idx: usize,
        staged: bool,
        depth: usize,
    },
    Commit(usize),
    /// A file heading in the notes view (not selectable).
    NoteFile {
        name: String,
        count: usize,
    },
    Note(usize),
}

impl ListRow {
    pub fn selectable(&self) -> bool {
        !matches!(self, ListRow::Header { .. } | ListRow::NoteFile { .. })
    }

    /// How many terminal rows this entry draws as. Notes are two lines (an
    /// anchor line plus their text), everything else is one — the list's
    /// scroll offset and click hit-testing both need this.
    pub fn height(&self) -> usize {
        match self {
            ListRow::Note(_) => 2,
            _ => 1,
        }
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

/// What a confirmed modal should do. Paths are resolved back to entries when
/// the action actually runs, so a refresh between ask and answer is harmless.
pub enum PendingAction {
    Discard { paths: Vec<std::path::PathBuf> },
}

/// What a mutating action (stage / unstage / discard) operates on: the file
/// under the cursor, or every file under the selected directory row that
/// belongs to that row's section.
enum Target {
    File {
        idx: usize,
        staged: bool,
    },
    Dir {
        path: String,
        staged: bool,
        idxs: Vec<usize>,
    },
}

impl Target {
    fn staged(&self) -> bool {
        match self {
            Target::File { staged, .. } | Target::Dir { staged, .. } => *staged,
        }
    }

    fn indices(&self) -> Vec<usize> {
        match self {
            Target::File { idx, .. } => vec![*idx],
            Target::Dir { idxs, .. } => idxs.clone(),
        }
    }
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
    /// Log view filter: true = only the commits this branch added on top of
    /// its base (`<merge-base>..HEAD`), false = all of HEAD's history.
    pub log_branch_only: bool,
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
    /// Collapsed tree directories, keyed by (staged section, full path).
    /// Survives refreshes/rebuilds; stale keys are harmless.
    collapsed: std::collections::HashSet<(bool, String)>,
    /// True while the current modal is being shown as a native herdr popup
    /// pane instead of the in-pane overlay (the answer arrives via file).
    pub modal_external: bool,
    /// Review notes (owned by the preview; mirrored for the notes view).
    pub notes: Vec<crate::ipc::NoteMeta>,
    /// Where `n` was pressed, so the notes view can return there.
    notes_return: Option<Mode>,
    /// Path plus whether it was selected in the staged section.
    pub annotate_request: Option<(std::path::PathBuf, bool)>,
    pub send_notes_request: bool,
    /// Edit/delete requests for the run loop (they need popups / the conn).
    pub edit_note_request: Option<(u64, String)>,
    pub delete_note_request: Option<u64>,
    /// The nvim remote socket (injected by the session; None standalone).
    pub nvim_server: Option<std::path::PathBuf>,
}

impl App {
    /// Load the initial status and build the app. Honors `default_scope`:
    /// a branch default resolves the merge base and loads branch changes up
    /// front (falling back to worktree if no base exists).
    pub fn new(repo: Repo, cfg: Config, keys: Keymap) -> Result<App> {
        let entries = repo.worktree_status(cfg.show_untracked)?;
        let mut app = App::from_entries(repo, cfg, keys, entries);
        if app.scope == Scope::Branch {
            match app.repo.resolve_base(&app.cfg.base) {
                Ok((base, mb)) => {
                    app.base = base;
                    app.merge_base = Some(mb);
                    if let Ok(entries) = app.load_entries() {
                        app.entries = entries;
                        app.rebuild_rows();
                    }
                }
                Err(_) => app.scope = Scope::Worktree, // no base — stay in worktree
            }
        }
        Ok(app)
    }

    /// Build an app around an already-loaded entry vector (no git status call).
    /// Used by render tests and by `new`.
    pub fn from_entries(repo: Repo, cfg: Config, keys: Keymap, entries: Vec<FileEntry>) -> App {
        let branch = repo.head_branch();
        let scope = match cfg.default_scope {
            crate::config::ScopePref::Branch => Scope::Branch,
            crate::config::ScopePref::Worktree => Scope::Worktree,
        };
        let mut app = App {
            repo,
            cfg,
            keys,
            mode: Mode::Files,
            scope,
            entries,
            commits: Vec::new(),
            log_branch_only: false,
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
            collapsed: std::collections::HashSet::new(),
            modal_external: false,
            notes: Vec::new(),
            notes_return: None,
            annotate_request: None,
            send_notes_request: false,
            edit_note_request: None,
            delete_note_request: None,
            nvim_server: None,
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
            Mode::Notes => self.note_rows(),
            Mode::CommitFiles => self.flat_tree_rows(),
            Mode::Files if self.scope == Scope::Branch => self.flat_tree_rows(),
            Mode::Files => self.grouped_rows(),
        };
        self.snap_cursor();
    }

    /// Notes grouped under a header per file, in first-seen order, so a
    /// review of several files reads as a review rather than a flat list.
    fn note_rows(&self) -> Vec<ListRow> {
        let mut rows = Vec::new();
        let mut seen: Vec<&std::path::Path> = Vec::new();
        for note in &self.notes {
            if !seen.contains(&note.file.as_path()) {
                seen.push(note.file.as_path());
            }
        }
        for file in seen {
            let idxs: Vec<usize> = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.file == file)
                .map(|(i, _)| i)
                .collect();
            rows.push(ListRow::NoteFile {
                name: file.display().to_string(),
                count: idxs.len(),
            });
            rows.extend(idxs.into_iter().map(ListRow::Note));
        }
        rows
    }

    /// One tree spanning every entry, unsectioned (Branch scope, CommitFiles).
    fn flat_tree_rows(&self) -> Vec<ListRow> {
        let pairs: Vec<(usize, &std::path::Path)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.path.as_path()))
            .collect();
        tree_rows(&pairs, false, &self.collapsed_for(false))
    }

    /// The collapsed paths for one section, in the shape `tree::build_tree`
    /// wants.
    fn collapsed_for(&self, staged: bool) -> std::collections::HashSet<String> {
        self.collapsed
            .iter()
            .filter(|(s, _)| *s == staged)
            .map(|(_, p)| p.clone())
            .collect()
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
                    let pairs: Vec<(usize, &std::path::Path)> = idxs
                        .iter()
                        .map(|&idx| (idx, self.entries[idx].path.as_path()))
                        .collect();
                    rows.extend(tree_rows(&pairs, staged, &self.collapsed_for(staged)));
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
            ListRow::Entry { idx, staged, .. } => Some((self.entries.get(*idx)?, *staged)),
            _ => None,
        }
    }

    /// The selected directory row's (path, staged) identity, if the cursor
    /// sits on one.
    pub fn selected_dir(&self) -> Option<(&str, bool)> {
        match self.rows.get(self.cursor)? {
            ListRow::Dir { path, staged, .. } => Some((path.as_str(), *staged)),
            _ => None,
        }
    }

    /// Collapse/expand the directory under the cursor. Returns true when the
    /// cursor was on a directory row (the toggle happened).
    pub fn toggle_selected_dir(&mut self) -> bool {
        let Some((path, staged)) = self.selected_dir().map(|(p, s)| (p.to_string(), s)) else {
            return false;
        };
        let key = (staged, path.clone());
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        self.rebuild_rows();
        // Keep the cursor on the toggled directory.
        if let Some(i) = self.rows.iter().position(|row| {
            matches!(row, ListRow::Dir { path: p, staged: s, .. } if *p == path && *s == staged)
        }) {
            self.cursor = i;
        }
        true
    }

    pub fn selected_commit(&self) -> Option<&CommitInfo> {
        match self.rows.get(self.cursor)? {
            ListRow::Commit(idx) => self.commits.get(*idx),
            _ => None,
        }
    }

    pub fn selected_note(&self) -> Option<&crate::ipc::NoteMeta> {
        match self.rows.get(self.cursor)? {
            ListRow::Note(idx) => self.notes.get(*idx),
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
                    self.delete_note_request = self.selected_note().map(|n| n.id);
                }
            }
            Action::Annotate => {
                if self.mode != Mode::Files || self.scope != Scope::Worktree {
                    self.set_status("notes work on staged/unstaged changes only");
                } else {
                    match self.selected_entry() {
                        Some((entry, staged)) => {
                            self.annotate_request = Some((entry.path.clone(), staged))
                        }
                        None => self.set_status("select a file to annotate"),
                    }
                }
            }
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
            // Enter on a directory row still collapses/expands it — that
            // needs no link.
            Action::Edit if self.toggle_selected_dir() => {}
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
                let Some(idx) = self.row_at(y - 1) else {
                    return false;
                };
                if self.rows.get(idx).map(ListRow::selectable).unwrap_or(false) {
                    self.cursor = idx;
                    // A single click on a directory row toggles its collapse
                    // (VSCode-style); no double-click needed.
                    if matches!(self.rows.get(idx), Some(ListRow::Dir { .. })) {
                        self.toggle_selected_dir();
                        self.last_click = None;
                        return false;
                    }
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

    /// The row index drawn at body line `offset_y` (0 = the first visible
    /// line). Walks the visible rows' heights, since a note draws as two
    /// lines — a click on either of them selects that note.
    pub fn row_at(&self, offset_y: u16) -> Option<usize> {
        let mut y = 0usize;
        for idx in self.list_offset..self.rows.len() {
            y += self.rows[idx].height();
            if (offset_y as usize) < y {
                return Some(idx);
            }
        }
        None
    }

    // ---- history view -----------------------------------------------------

    /// `l`: enter the log view; `l` again (or q/esc) returns to files.
    /// Entering from branch scope keeps that framing: the log opens filtered
    /// to the commits this branch added on top of its base.
    fn toggle_log(&mut self) {
        match self.mode {
            Mode::Notes => self.set_status("leave the notes view first (n)"),
            Mode::Files => {
                self.log_branch_only = self.scope == Scope::Branch && self.merge_base.is_some();
                match self.load_commits() {
                    Ok(commits) if commits.is_empty() && !self.log_branch_only => {
                        self.set_status("no commits yet")
                    }
                    Ok(commits) => {
                        let empty = commits.is_empty();
                        self.commits = commits;
                        self.mode = Mode::Log;
                        self.cursor = 0;
                        self.rebuild_rows();
                        if empty {
                            self.set_status(format!(
                                "no commits on this branch vs {} — w shows all",
                                self.base_or_default()
                            ));
                        }
                    }
                    Err(err) => self.set_status(format!("log failed: {err}")),
                }
            }
            Mode::Log | Mode::CommitFiles => self.leave_history(),
        }
    }

    /// Load the commit vector the current log filter asks for.
    fn load_commits(&self) -> Result<Vec<CommitInfo>> {
        match (self.log_branch_only, self.merge_base.as_deref()) {
            (true, Some(mb)) => self.repo.log_branch_commits(mb, LOG_LIMIT),
            _ => self.repo.log_commits(LOG_LIMIT),
        }
    }

    /// `w` in the log view: flip between "this branch's commits" and the
    /// full history. Resolves the base lazily, exactly like scope toggling.
    fn toggle_log_filter(&mut self) {
        if !self.log_branch_only && self.merge_base.is_none() {
            match self.repo.resolve_base(&self.cfg.base) {
                Ok((base, mb)) => {
                    self.base = base;
                    self.merge_base = Some(mb);
                }
                Err(err) => {
                    self.set_status(format!("no base found: {err}"));
                    return;
                }
            }
        }
        self.log_branch_only = !self.log_branch_only;
        match self.load_commits() {
            Ok(commits) => {
                self.commits = commits;
                self.cursor = 0;
                self.list_offset = 0;
                self.rebuild_rows();
                let msg = if self.log_branch_only {
                    format!(
                        "{} commits on this branch vs {}",
                        self.commits.len(),
                        self.base_or_default()
                    )
                } else {
                    "showing all commits".to_string()
                };
                self.set_status(msg);
            }
            Err(err) => {
                self.log_branch_only = !self.log_branch_only; // roll back
                self.set_status(format!("log failed: {err}"));
            }
        }
    }

    /// The base ref label, with a placeholder when it hasn't resolved.
    pub fn base_or_default(&self) -> String {
        if self.base.is_empty() {
            "base".to_string()
        } else {
            self.base.clone()
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
    pub fn toggle_notes_view(&mut self) {
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
        let editor = self.cfg.editor.first().cloned().unwrap_or_default();
        if crate::nvim::request_close(&editor, self.nvim_server.as_deref(), save) {
            self.after_edit = Some(then);
        } else {
            self.set_status("could not close the editor");
        }
    }

    fn run_pending(&mut self, pending: PendingAction) {
        match pending {
            PendingAction::Discard { paths } => self.discard_paths(&paths),
        }
    }

    // ---- stage / discard (worktree scope) ---------------------------------

    /// `s`: section-aware — in "staged changes" it unstages, in "changes" it
    /// stages. The file then moves to the other section (VSCode-style).
    /// On a directory row it applies to every file under it in that section.
    fn stage_toggle(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        let paths = self.target_paths(&target);
        let count = paths.len();
        let result = if target.staged() {
            self.repo.unstage_many(&paths)
        } else {
            self.repo.stage_many(&paths)
        };
        if let Err(err) = result {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        if let Target::Dir { path, .. } = &target {
            let verb = if target.staged() {
                "unstaged"
            } else {
                "staged"
            };
            let msg = format!("{verb} {count} file(s) under {path}");
            self.set_status(msg);
        }
        self.force_refresh();
        self.needs_reshow = true;
        let all_staged =
            !self.entries.is_empty() && self.entries.iter().all(|e| e.stage == StageState::Staged);
        if all_staged {
            self.set_status("all changes staged — c to commit");
        }
    }

    /// `u`: explicitly unstage the selected file (or folder), whichever
    /// section it's in.
    fn unstage_selected(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        let paths: Vec<std::path::PathBuf> = target
            .indices()
            .into_iter()
            .filter_map(|i| self.entries.get(i))
            .filter(|e| matches!(e.stage, StageState::Staged | StageState::Partial))
            .map(|e| e.path.clone())
            .collect();
        if paths.is_empty() {
            self.set_status(match target {
                Target::File { .. } => "nothing staged for this file",
                Target::Dir { .. } => "nothing staged in this folder",
            });
            return;
        }
        if let Err(err) = self.repo.unstage_many(&paths) {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        self.force_refresh();
        self.needs_reshow = true;
    }

    /// `x`: ask before throwing changes away (refused for conflicts).
    /// On a directory row it discards every file under it in that section.
    fn open_discard_confirm(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        let paths = self.target_paths(&target);
        if paths.is_empty() {
            return;
        }
        let text = match &target {
            Target::File { .. } => format!(
                "Discard changes to {}? This cannot be undone. (y/n)",
                paths[0].display()
            ),
            Target::Dir { path, .. } => format!(
                "Discard changes to {} file(s) under {path}? This cannot be undone. (y/n)",
                paths.len()
            ),
        };
        self.modal = Some(Modal::Confirm {
            text,
            pending: PendingAction::Discard { paths },
        });
    }

    /// Resolve what the cursor points at into a mutation target, applying the
    /// shared guards (working-tree scope only, no conflicts). Sets a status
    /// message and returns None when the action can't run.
    fn mutation_target(&mut self) -> Option<Target> {
        if self.mode != Mode::Files {
            self.set_status("read-only in history view");
            return None;
        }
        if self.scope != Scope::Worktree {
            self.set_status("works in working-tree scope (w)");
            return None;
        }
        match self.rows.get(self.cursor).cloned()? {
            ListRow::Entry { idx, staged, .. } => match self.entries.get(idx) {
                Some(e) if e.kind == ChangeKind::Conflicted => {
                    self.set_status("resolve the conflict in the editor first");
                    None
                }
                Some(_) => Some(Target::File { idx, staged }),
                None => None,
            },
            ListRow::Dir { path, staged, .. } => {
                let idxs = self.entries_in_dir(&path, staged);
                if idxs.is_empty() {
                    self.set_status("nothing to do in this folder");
                    return None;
                }
                Some(Target::Dir { path, staged, idxs })
            }
            _ => None,
        }
    }

    /// Entry indices under a directory row, restricted to that row's section
    /// so `s` on "staged changes / src/" only touches the staged side.
    /// Conflicted entries are never included — they need the editor first.
    fn entries_in_dir(&self, dir: &str, staged: bool) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.path.to_string_lossy().starts_with(dir))
            .filter(|(_, e)| e.kind != ChangeKind::Conflicted && in_section(e, staged))
            .map(|(i, _)| i)
            .collect()
    }

    fn target_paths(&self, target: &Target) -> Vec<std::path::PathBuf> {
        target
            .indices()
            .into_iter()
            .filter_map(|i| self.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Discard by path (the confirm modal stores paths, not indices, so a
    /// refresh between question and answer can't discard the wrong file).
    /// Stops at the first failure and reports it.
    fn discard_paths(&mut self, paths: &[std::path::PathBuf]) {
        let mut failure = None;
        for path in paths {
            let Some(entry) = self.entries.iter().find(|e| e.path == *path).cloned() else {
                continue; // already gone (staged+discarded elsewhere)
            };
            if let Err(err) = self.repo.discard(&entry) {
                failure = Some(first_line(&err.to_string()));
                break;
            }
        }
        if let Some(msg) = failure {
            self.set_status(msg);
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
    fn cursor_identity(&self) -> Option<CursorId> {
        match self.rows.get(self.cursor)? {
            ListRow::Entry { .. } => self
                .selected_entry()
                .map(|(e, staged)| CursorId::Entry(e.path.clone(), staged)),
            ListRow::Dir { path, staged, .. } => Some(CursorId::Dir(path.clone(), *staged)),
            _ => None,
        }
    }

    fn restore_cursor(&mut self, keep: Option<CursorId>) {
        match keep {
            Some(CursorId::Entry(path, staged)) => {
                // Same path + same section first; then same path anywhere.
                let find = |want_staged: Option<bool>| {
                    self.rows.iter().position(|row| match row {
                        ListRow::Entry { idx, staged: s, .. } => {
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
            Some(CursorId::Dir(path, staged)) => {
                if let Some(i) = self.rows.iter().position(|row| {
                    matches!(row, ListRow::Dir { path: p, staged: s, .. } if *p == path && *s == staged)
                }) {
                    self.cursor = i;
                    return;
                }
            }
            None => {}
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
        // In the log view `w` filters commits instead of switching scope.
        if self.mode == Mode::Log {
            self.toggle_log_filter();
            return;
        }
        if self.mode != Mode::Files {
            self.set_status("read-only in history view");
            return;
        }
        match self.scope {
            Scope::Worktree => {
                if self.merge_base.is_none() {
                    match self.repo.resolve_base(&self.cfg.base) {
                        Ok((base, mb)) => {
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

/// Does this entry belong to the grouped worktree view's staged section
/// (`staged = true`) or its unstaged "changes" section? A partially staged
/// file is in both, exactly like [`App::grouped_rows`] renders it.
fn in_section(e: &FileEntry, staged: bool) -> bool {
    if staged {
        matches!(e.stage, StageState::Staged | StageState::Partial)
    } else {
        matches!(
            e.stage,
            StageState::Unstaged | StageState::Partial | StageState::Untracked
        )
    }
}

/// First line of an error (git errors embed stderr; the top line is the news).
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// What the cursor pointed at before a refresh, in a rebuild-stable form.
enum CursorId {
    Entry(std::path::PathBuf, bool),
    Dir(String, bool),
}

/// Build tree rows (dirs + files) for one section's `(idx, path)` pairs,
/// converting the pure `tree::TreeRow`s into `ListRow`s.
fn tree_rows(
    pairs: &[(usize, &std::path::Path)],
    staged: bool,
    collapsed: &std::collections::HashSet<String>,
) -> Vec<ListRow> {
    super::tree::build_tree(pairs, collapsed)
        .into_iter()
        .map(|row| match row {
            super::tree::TreeRow::Dir {
                depth,
                name,
                path,
                collapsed,
            } => ListRow::Dir {
                depth,
                name,
                path,
                staged,
                collapsed,
            },
            super::tree::TreeRow::File { depth, idx } => ListRow::Entry { idx, staged, depth },
        })
        .collect()
}
