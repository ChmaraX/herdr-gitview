//! Pure file-tree builder for the list pane. Given `(idx, path)` pairs for
//! one section (or one whole entries vector), produces a flat sequence of
//! tree rows: directories (collapsible) followed by the files under them.
//! Directories sort before files at each level, both alphabetically; a
//! directory that holds nothing but a single nested directory (and no
//! files) folds into its parent's row so chains like `src/preview/foo.rs`
//! show one "src/preview/" row instead of two empty intermediate ones.
//! A directory whose full path is in the caller's `collapsed` set is
//! emitted as a single collapsed row with everything under it hidden.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// One visual row produced by [`build_tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    /// A (possibly folded) directory chain, e.g. name = "src/preview/".
    /// `path` is the full path from the tree root (also with a trailing
    /// slash) — the stable identity used for collapse tracking.
    Dir {
        depth: usize,
        name: String,
        path: String,
        collapsed: bool,
    },
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
pub fn build_tree(entries: &[(usize, &Path)], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = DirNode::default();
    for &(idx, path) in entries {
        insert(&mut root, path, idx);
    }
    let mut rows = Vec::new();
    emit(&root, 0, "", collapsed, &mut rows);
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

fn emit(
    node: &DirNode,
    depth: usize,
    prefix: &str,
    collapsed: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
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
        let path = format!("{prefix}{}/", names.join("/"));
        let is_collapsed = collapsed.contains(&path);
        rows.push(TreeRow::Dir {
            depth,
            name: format!("{}/", names.join("/")),
            path: path.clone(),
            collapsed: is_collapsed,
        });
        if !is_collapsed {
            emit(cur, depth + 1, &path, collapsed, rows);
        }
    }
    for idx in node.files.values() {
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

    fn dir(depth: usize, name: &str, path: &str, collapsed: bool) -> TreeRow {
        TreeRow::Dir {
            depth,
            name: name.to_string(),
            path: path.to_string(),
            collapsed,
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn nesting_and_depth() {
        let owned = paths(&[(0, "a/b/c.rs"), (1, "top.rs")]);
        let rows = build_tree(&refs(&owned), &none());
        // "a" and "b" are a single-child dir chain (no files at either
        // level) so they fold into one "a/b/" row at depth 0, with c.rs
        // nested one level deeper.
        assert_eq!(
            rows,
            vec![
                dir(0, "a/b/", "a/b/", false),
                TreeRow::File { depth: 1, idx: 0 },
                TreeRow::File { depth: 0, idx: 1 },
            ]
        );
    }

    #[test]
    fn dirs_sort_before_files_alphabetically() {
        let owned = paths(&[(0, "z.rs"), (1, "a.rs"), (2, "m/one.rs")]);
        let rows = build_tree(&refs(&owned), &none());
        assert_eq!(
            rows,
            vec![
                dir(0, "m/", "m/", false),
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
        let rows = build_tree(&refs(&owned), &none());
        assert_eq!(
            rows,
            vec![
                dir(0, "src/", "src/", false),
                dir(1, "preview/", "src/preview/", false),
                TreeRow::File { depth: 2, idx: 0 },
                TreeRow::File { depth: 1, idx: 1 },
            ]
        );
    }

    #[test]
    fn long_single_child_chain_folds_fully() {
        let owned = paths(&[(0, "src/preview/inner/mod.rs")]);
        let rows = build_tree(&refs(&owned), &none());
        assert_eq!(
            rows,
            vec![
                dir(0, "src/preview/inner/", "src/preview/inner/", false),
                TreeRow::File { depth: 1, idx: 0 },
            ]
        );
    }

    #[test]
    fn collapsed_dir_hides_everything_under_it() {
        let owned = paths(&[(0, "src/preview/mod.rs"), (1, "src/list.rs"), (2, "top.rs")]);
        let collapsed = HashSet::from(["src/".to_string()]);
        let rows = build_tree(&refs(&owned), &collapsed);
        // The whole src/ subtree (including the nested preview/ dir) is
        // hidden; the dir row itself is marked collapsed.
        assert_eq!(
            rows,
            vec![
                dir(0, "src/", "src/", true),
                TreeRow::File { depth: 0, idx: 2 },
            ]
        );
    }

    #[test]
    fn collapsing_a_nested_dir_keeps_siblings_visible() {
        let owned = paths(&[(0, "src/preview/mod.rs"), (1, "src/list.rs")]);
        let collapsed = HashSet::from(["src/preview/".to_string()]);
        let rows = build_tree(&refs(&owned), &collapsed);
        assert_eq!(
            rows,
            vec![
                dir(0, "src/", "src/", false),
                dir(1, "preview/", "src/preview/", true),
                TreeRow::File { depth: 1, idx: 1 }, // src/list.rs stays
            ]
        );
    }
}
