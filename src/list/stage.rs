//! Staging actions: what `s`, `u` and `x` operate on, and the guards that
//! decide whether they may run at all.
//!
//! Split out of `app` because "which entries does the cursor mean" is a model
//! in its own right — a file, or every file under a folder row *in that row's
//! section*. Getting that wrong let a folder in "merge conflicts" stage a
//! file from "changes".

use super::App;
use super::app::{Modal, Mode, PendingAction, first_line};
use super::rows::{ListRow, Section};
use crate::git::{ChangeKind, FileEntry, Scope, StageState};

/// What a mutating action (stage / unstage / discard) operates on: the entries
/// under the cursor, plus the section that decides which way `s` goes.
/// `folder` is the directory row's path when the cursor is on one, which only
/// the status and confirm messages care about.
struct Target {
    section: Section,
    folder: Option<String>,
    idxs: Vec<usize>,
}

impl Target {
    fn paths(&self, entries: &[FileEntry]) -> Vec<std::path::PathBuf> {
        self.idxs
            .iter()
            .filter_map(|i| entries.get(*i))
            .map(|e| e.path.clone())
            .collect()
    }
}

/// Which mutating actions the current selection allows, for the footer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ops {
    pub stage: bool,
    pub unstage: bool,
    pub discard: bool,
    /// True when `s` would unstage rather than stage (staged section).
    pub stage_unstages: bool,
}

impl App {
    /// `s`: section-aware — in "staged changes" it unstages, in "changes" it
    /// stages. The file then moves to the other section (VSCode-style).
    /// On a directory row it applies to every file under it in that section.
    pub(super) fn stage_toggle(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        let paths = target.paths(&self.entries);
        let count = paths.len();
        let unstaging = target.section.cached();
        let result = if unstaging {
            self.repo.unstage_many(&paths)
        } else {
            self.repo.stage_many(&paths)
        };
        if let Err(err) = result {
            self.set_status(first_line(&err.to_string()));
            return;
        }
        self.force_refresh();
        self.needs_reshow = true;
        // One message, decided once: "everything is staged" is the more
        // useful news when a folder action got us there.
        let all_staged =
            !self.entries.is_empty() && self.entries.iter().all(|e| e.stage == StageState::Staged);
        if all_staged {
            self.set_status("all changes staged — c to commit");
        } else if let Some(folder) = &target.folder {
            let verb = if unstaging { "unstaged" } else { "staged" };
            self.set_status(format!("{verb} {count} file(s) under {folder}"));
        }
    }

    /// `u`: explicitly unstage the selected file (or folder), whichever
    /// section it's in.
    pub(super) fn unstage_selected(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        // `u` means "unstage", so it looks at the staged side of these entries
        // whichever section the cursor was in.
        let paths: Vec<std::path::PathBuf> = target
            .idxs
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter(|e| matches!(e.stage, StageState::Staged | StageState::Partial))
            .map(|e| e.path.clone())
            .collect();
        if paths.is_empty() {
            self.set_status(match target.folder {
                None => "nothing staged for this file",
                Some(_) => "nothing staged in this folder",
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
    pub(super) fn open_discard_confirm(&mut self) {
        let Some(target) = self.mutation_target() else {
            return;
        };
        let paths = target.paths(&self.entries);
        if paths.is_empty() {
            return;
        }
        let text = match &target.folder {
            None => format!(
                "Discard changes to {}? This cannot be undone. (y/n)",
                paths[0].display()
            ),
            Some(folder) => format!(
                "Discard changes to {} file(s) under {folder}? This cannot be undone. (y/n)",
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
    ///
    /// [`Self::selection_ops`] answers the same question without the status
    /// side effects, so the footer and the actions can't disagree.
    fn mutation_target(&mut self) -> Option<Target> {
        if self.mode != Mode::Files {
            self.set_status("read-only in history view");
            return None;
        }
        if self.scope != Scope::Worktree {
            self.set_status("works in working-tree scope (w)");
            return None;
        }
        let (section, folder, idxs) = match self.rows.get(self.cursor).cloned()? {
            ListRow::Entry { idx, section, .. } => (section, None, vec![idx]),
            ListRow::Dir { path, section, .. } => {
                let idxs = self.entries_in_dir(&path, section);
                (section, Some(path), idxs)
            }
            _ => return None,
        };
        // A conflicted file needs the editor, whether it was selected directly
        // or swept up by a folder row in the conflicts section.
        if idxs
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .any(|e| e.kind == ChangeKind::Conflicted)
        {
            self.set_status("resolve the conflict in the editor first");
            return None;
        }
        if idxs.is_empty() {
            self.set_status("nothing to do in this folder");
            return None;
        }
        Some(Target {
            section,
            folder,
            idxs,
        })
    }

    /// Which mutating actions the cursor's selection allows. Pure: the footer
    /// asks this rather than re-deriving eligibility with its own predicate.
    pub fn selection_ops(&self) -> Ops {
        if self.mode != Mode::Files || self.scope != Scope::Worktree {
            return Ops::default();
        }
        let (section, idxs) = match self.rows.get(self.cursor) {
            Some(ListRow::Entry { idx, section, .. }) => (*section, vec![*idx]),
            Some(ListRow::Dir { path, section, .. }) => {
                (*section, self.entries_in_dir(path, *section))
            }
            _ => return Ops::default(),
        };
        let entries: Vec<&FileEntry> = idxs.iter().filter_map(|i| self.entries.get(*i)).collect();
        if entries.is_empty() || entries.iter().any(|e| e.kind == ChangeKind::Conflicted) {
            return Ops::default();
        }
        Ops {
            stage: true,
            discard: true,
            // `u` only does something if there is a staged side to restore.
            unstage: entries
                .iter()
                .any(|e| matches!(e.stage, StageState::Staged | StageState::Partial)),
            // `s` on the staged section means "unstage".
            stage_unstages: section.cached(),
        }
    }

    /// Entry indices under a directory row, restricted to that row's section
    /// so `s` on "staged changes / src/" only touches the staged side — and
    /// so a folder row in "merge conflicts" can never reach into "changes".
    ///
    /// `dir` always ends in `/` (see `tree::build_tree`), which is what makes
    /// the prefix match component-safe: `src/` cannot match `src-old/…`.
    fn entries_in_dir(&self, dir: &str, section: Section) -> Vec<usize> {
        debug_assert!(dir.ends_with('/'), "tree dir paths carry a trailing slash");
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.path.to_string_lossy().starts_with(dir))
            .filter(|(_, e)| section.holds(e))
            .map(|(i, _)| i)
            .collect()
    }

    /// Discard by path (the confirm modal stores paths, not indices, so a
    /// refresh between question and answer can't discard the wrong file).
    /// Stops at the first failure and reports it.
    pub(super) fn discard_paths(&mut self, paths: &[std::path::PathBuf]) {
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
}
