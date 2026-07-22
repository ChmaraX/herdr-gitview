//! Pure file-tree builder for the list pane. Given `(idx, path)` pairs for
//! one section (or one whole entries vector), produces a flat sequence of
//! tree rows: directories (always expanded, never selectable) followed by
//! the files under them. Directories sort before files at each level, both
//! alphabetically; a directory that holds nothing but a single nested
//! directory (and no files) folds into its parent's row so chains like
//! `src/preview/foo.rs` show one "src/preview/" row instead of two empty
//! intermediate ones.

use std::collections::BTreeMap;
use std::path::Path;

/// One visual row produced by [`build_tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    /// A (possibly folded) directory chain, e.g. name = "src/preview/".
    Dir { depth: usize, name: String },
    /// A file, referencing back into the caller's original index space.
    File { depth: usize, idx: usize },
}

/// A directory's children, keyed by name for alphabetical, dir-before-file
/// ordering (`BTreeMap` iterates in key order).
#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: BTreeMap<String, usize>,
}

/// Build the tree rows for one section's `(idx, path)` pairs. `idx` is
/// whatever the caller wants echoed back on `TreeRow::File` (typically the
/// index into its backing entries vector).
pub fn build_tree(entries: &[(usize, &Path)]) -> Vec<TreeRow> {
    let mut root = DirNode::default();
    for &(idx, path) in entries {
        insert(&mut root, path, idx);
    }
    let mut rows = Vec::new();
    emit(&root, 0, &mut rows);
    rows
}

fn insert(root: &mut DirNode, path: &Path, idx: usize) {
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if comps.is_empty() {
        return;
    }
    let mut node = root;
    for dir in &comps[..comps.len() - 1] {
        node = node.dirs.entry(dir.clone()).or_default();
    }
    node.files.insert(comps[comps.len() - 1].clone(), idx);
}

fn emit(node: &DirNode, depth: usize, rows: &mut Vec<TreeRow>) {
    for (name, child) in &node.dirs {
        // Fold a chain of directories that each hold nothing but a single
        // nested directory (and no files of their own) into one row.
        let mut names = vec![name.clone()];
        let mut cur = child;
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let (next_name, next_node) = cur.dirs.iter().next().expect("len == 1");
            names.push(next_name.clone());
            cur = next_node;
        }
        rows.push(TreeRow::Dir {
            depth,
            name: format!("{}/", names.join("/")),
        });
        emit(cur, depth + 1, rows);
    }
    for (_, idx) in &node.files {
        rows.push(TreeRow::File { depth, idx: *idx });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(items: &[(usize, &str)]) -> Vec<(usize, PathBuf)> {
        items.iter().map(|(i, p)| (*i, PathBuf::from(p))).collect()
    }

    fn refs(items: &[(usize, PathBuf)]) -> Vec<(usize, &Path)> {
        items.iter().map(|(i, p)| (*i, p.as_path())).collect()
    }

    #[test]
    fn nesting_and_depth() {
        let owned = paths(&[(0, "a/b/c.rs"), (1, "top.rs")]);
        let rows = build_tree(&refs(&owned));
        // "a" and "b" are a single-child dir chain (no files at either
        // level) so they fold into one "a/b/" row at depth 0, with c.rs
        // nested one level deeper.
        assert_eq!(
            rows,
            vec![
                TreeRow::Dir {
                    depth: 0,
                    name: "a/b/".to_string()
                },
                TreeRow::File { depth: 1, idx: 0 },
                TreeRow::File { depth: 0, idx: 1 },
            ]
        );
    }

    #[test]
    fn dirs_sort_before_files_alphabetically() {
        let owned = paths(&[(0, "z.rs"), (1, "a.rs"), (2, "m/one.rs")]);
        let rows = build_tree(&refs(&owned));
        assert_eq!(
            rows,
            vec![
                TreeRow::Dir {
                    depth: 0,
                    name: "m/".to_string()
                },
                TreeRow::File { depth: 1, idx: 2 },
                TreeRow::File { depth: 0, idx: 1 }, // a.rs
                TreeRow::File { depth: 0, idx: 0 }, // z.rs
            ]
        );
    }

    #[test]
    fn chain_does_not_fold_when_dir_has_multiple_children() {
        // "src" holds both "preview" (a dir) and "list.rs" (a file), so it
        // must NOT fold with anything below it — it has more than one child.
        let owned = paths(&[(0, "src/preview/mod.rs"), (1, "src/list.rs")]);
        let rows = build_tree(&refs(&owned));
        assert_eq!(
            rows,
            vec![
                TreeRow::Dir {
                    depth: 0,
                    name: "src/".to_string()
                },
                TreeRow::Dir {
                    depth: 1,
                    name: "preview/".to_string()
                },
                TreeRow::File { depth: 2, idx: 0 },
                TreeRow::File { depth: 1, idx: 1 },
            ]
        );
    }

    #[test]
    fn long_single_child_chain_folds_fully() {
        let owned = paths(&[(0, "src/preview/inner/mod.rs")]);
        let rows = build_tree(&refs(&owned));
        assert_eq!(
            rows,
            vec![
                TreeRow::Dir {
                    depth: 0,
                    name: "src/preview/inner/".to_string()
                },
                TreeRow::File { depth: 1, idx: 0 },
            ]
        );
    }
}
