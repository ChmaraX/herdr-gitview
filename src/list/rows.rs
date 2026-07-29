//! The list's visual rows: which section an entry belongs to, what a row is,
//! and how the current mode's backing vector becomes a row vector.
//!
//! Split out of `app` so the row model can be read (and reasoned about) on
//! its own — it is the thing every other part of the pane indexes into.

use super::App;
use super::app::Mode;
use super::tree;
use crate::git::{ChangeKind, FileEntry, Scope, StageState};

/// Which section of the grouped worktree view a row belongs to. This is the
/// row's identity, not a `staged` flag: "merge conflicts" and "changes" are
/// both unstaged, so a boolean cannot tell them apart — and a folder row that
/// can't tell them apart mutates files from the section you didn't select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Conflicts,
    Staged,
    Changes,
    /// Branch scope and commit files: one unsectioned tree.
    Flat,
}

impl Section {
    /// Does this section show `entry`? A partially staged file is in both
    /// "staged changes" and "changes", exactly as the view renders it.
    pub fn holds(self, entry: &FileEntry) -> bool {
        let conflicted = entry.kind == ChangeKind::Conflicted;
        match self {
            Section::Conflicts => conflicted,
            Section::Staged => {
                !conflicted && matches!(entry.stage, StageState::Staged | StageState::Partial)
            }
            Section::Changes => {
                !conflicted
                    && matches!(
                        entry.stage,
                        StageState::Unstaged | StageState::Partial | StageState::Untracked
                    )
            }
            Section::Flat => true,
        }
    }

    /// Whether the preview should show the staged (`--cached`) diff.
    pub fn cached(self) -> bool {
        self == Section::Staged
    }

    fn title(self) -> &'static str {
        match self {
            Section::Conflicts => "merge conflicts",
            Section::Staged => "staged changes",
            Section::Changes => "changes",
            Section::Flat => "",
        }
    }
}

/// One visual row of the list. Sections group entries VSCode-style: a file
/// with both staged and unstaged changes appears in *both* sections, and the
/// section decides which diff the preview shows (staged → `--cached`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRow {
    Header {
        title: &'static str,
        count: usize,
    },
    /// A directory row in the file tree — selectable; Enter or a click
    /// collapses/expands the subtree below it. `path` (full path from the
    /// tree root, trailing slash) plus `section` is its stable identity, for
    /// collapse state and for deciding what a folder action applies to.
    Dir {
        depth: usize,
        name: String,
        path: String,
        section: Section,
        collapsed: bool,
    },
    Entry {
        idx: usize,
        section: Section,
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

impl App {
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
        tree_rows(&pairs, Section::Flat, &self.collapsed_for(Section::Flat))
    }

    /// The collapsed paths for one section, in the shape `tree::build_tree`
    /// wants.
    fn collapsed_for(&self, section: Section) -> std::collections::HashSet<String> {
        self.collapsed
            .iter()
            .filter(|(s, _)| *s == section)
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// Worktree sections: conflicts, staged, changes. A partially staged file
    /// appears under both "staged" and "changes".
    fn grouped_rows(&self) -> Vec<ListRow> {
        let mut rows = Vec::new();
        for section in [Section::Conflicts, Section::Staged, Section::Changes] {
            let idxs = self.entries_in_section(section);
            if idxs.is_empty() {
                continue;
            }
            rows.push(ListRow::Header {
                title: section.title(),
                count: idxs.len(),
            });
            let pairs: Vec<(usize, &std::path::Path)> = idxs
                .iter()
                .map(|&idx| (idx, self.entries[idx].path.as_path()))
                .collect();
            rows.extend(tree_rows(&pairs, section, &self.collapsed_for(section)));
        }
        rows
    }

    /// Entry indices this section shows, in entry order.
    fn entries_in_section(&self, section: Section) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| section.holds(e))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Build tree rows (dirs + files) for one section's `(idx, path)` pairs,
/// converting the pure `tree::TreeRow`s into `ListRow`s.
fn tree_rows(
    pairs: &[(usize, &std::path::Path)],
    section: Section,
    collapsed: &std::collections::HashSet<String>,
) -> Vec<ListRow> {
    tree::build_tree(pairs, collapsed)
        .into_iter()
        .map(|row| match row {
            tree::TreeRow::Dir {
                depth,
                name,
                path,
                collapsed,
            } => ListRow::Dir {
                depth,
                name,
                path,
                section,
                collapsed,
            },
            tree::TreeRow::File { depth, idx } => ListRow::Entry {
                idx,
                section,
                depth,
            },
        })
        .collect()
}
