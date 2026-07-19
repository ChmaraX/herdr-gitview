//! Fixture tests: build real git repos in tempdirs and assert FileEntry
//! vectors (reviewr's pattern). Each test gets its own repo.

use std::path::{Path, PathBuf};
use std::process::Command;

use herdr_gitview::git::{ChangeKind, Repo, Scope, StageState};

/// Minimal tempdir: unique path under the OS temp dir, removed on drop.
struct TempRepo {
    dir: PathBuf,
    repo: Repo,
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn fixture(name: &str) -> TempRepo {
    let dir = std::env::temp_dir().join(format!(
        "gitview-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "base.txt", "one\ntwo\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    // canonicalize: macOS /tmp is a symlink to /private/tmp
    let dir = dir.canonicalize().unwrap();
    let repo = Repo::discover(&dir).unwrap();
    TempRepo { dir, repo }
}

#[test]
fn staged_unstaged_partial() {
    let t = fixture("stage-states");
    // staged-only
    write(&t.dir, "staged.txt", "s\n");
    git(&t.dir, &["add", "staged.txt"]);
    // unstaged-only (tracked, then modified)
    write(&t.dir, "base.txt", "one\ntwo\nthree\n");
    // partial: add, then modify again
    write(&t.dir, "partial.txt", "p1\n");
    git(&t.dir, &["add", "partial.txt"]);
    write(&t.dir, "partial.txt", "p1\np2\n");

    let entries = t.repo.worktree_status(true).unwrap();
    let by_path = |p: &str| entries.iter().find(|e| e.path == PathBuf::from(p)).unwrap();

    let staged = by_path("staged.txt");
    assert_eq!((staged.kind, staged.stage), (ChangeKind::Added, StageState::Staged));
    assert_eq!((staged.adds, staged.dels), (Some(1), Some(0)));

    let unstaged = by_path("base.txt");
    assert_eq!((unstaged.kind, unstaged.stage), (ChangeKind::Modified, StageState::Unstaged));
    assert_eq!((unstaged.adds, unstaged.dels), (Some(1), Some(0)));

    let partial = by_path("partial.txt");
    assert_eq!(partial.stage, StageState::Partial);
    assert_eq!((partial.adds, partial.dels), (Some(2), Some(0))); // 1 staged + 1 unstaged
}

#[test]
fn rename_untracked_spaces_binary() {
    let t = fixture("mixed");
    // rename, staged
    git(&t.dir, &["mv", "base.txt", "renamed.txt"]);
    // untracked with spaces in the name
    write(&t.dir, "new file.txt", "a\nb\nc\n");
    // binary, staged
    std::fs::write(t.dir.join("blob.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    git(&t.dir, &["add", "blob.bin"]);

    let entries = t.repo.worktree_status(true).unwrap();
    let by_path = |p: &str| entries.iter().find(|e| e.path == PathBuf::from(p)).unwrap();

    let renamed = by_path("renamed.txt");
    assert_eq!(renamed.kind, ChangeKind::Renamed);
    assert_eq!(renamed.orig_path, Some(PathBuf::from("base.txt")));
    assert_eq!(renamed.stage, StageState::Staged);

    let untracked = by_path("new file.txt");
    assert_eq!((untracked.kind, untracked.stage), (ChangeKind::Untracked, StageState::Untracked));
    assert_eq!((untracked.adds, untracked.dels), (Some(3), Some(0)));

    let binary = by_path("blob.bin");
    assert_eq!(binary.kind, ChangeKind::Added);
    assert_eq!((binary.adds, binary.dels), (None, None));
}

#[test]
fn untracked_hidden_when_disabled() {
    let t = fixture("no-untracked");
    write(&t.dir, "loose.txt", "x\n");
    assert_eq!(t.repo.worktree_status(true).unwrap().len(), 1);
    assert_eq!(t.repo.worktree_status(false).unwrap().len(), 0);
}

#[test]
fn branch_scope_changes_vs_main() {
    let t = fixture("branch");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "feat.txt", "f1\nf2\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feat 1"]);
    write(&t.dir, "base.txt", "one\ntwo\nchanged\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feat 2"]);

    assert_eq!(t.repo.head_branch().as_deref(), Some("feature"));
    assert_eq!(t.repo.detect_base(), "main");
    let mb = t.repo.merge_base("main").unwrap();
    let entries = t.repo.branch_changes(&mb).unwrap();

    assert_eq!(entries.len(), 2);
    let by_path = |p: &str| entries.iter().find(|e| e.path == PathBuf::from(p)).unwrap();
    let feat = by_path("feat.txt");
    assert_eq!((feat.kind, feat.stage), (ChangeKind::Added, StageState::NA));
    assert_eq!((feat.adds, feat.dels), (Some(2), Some(0)));
    let base = by_path("base.txt");
    assert_eq!(base.kind, ChangeKind::Modified);
    assert_eq!((base.adds, base.dels), (Some(1), Some(0))); // appended one line
}

#[test]
fn conflicted_file_sorts_first() {
    let t = fixture("conflict");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "base.txt", "feature version\n");
    git(&t.dir, &["commit", "-q", "-am", "feature edit"]);
    git(&t.dir, &["checkout", "-q", "main"]);
    write(&t.dir, "base.txt", "main version\n");
    git(&t.dir, &["commit", "-q", "-am", "main edit"]);
    // merge conflicts: git merge exits non-zero, so run it leniently
    let _ = Command::new("git")
        .arg("-C")
        .arg(&t.dir)
        .args(["merge", "feature"])
        .output()
        .unwrap();
    // add a second, boring change that would sort before by path
    write(&t.dir, "aaa.txt", "x\n");

    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entries[0].path, PathBuf::from("base.txt"));
    assert_eq!(entries[0].kind, ChangeKind::Conflicted);
    assert_eq!(entries[0].stage, StageState::NA);
    assert_eq!(entries[1].path, PathBuf::from("aaa.txt"));
}

// ---- diff_ansi ------------------------------------------------------------

fn find<'a>(entries: &'a [herdr_gitview::git::FileEntry], p: &str) -> &'a herdr_gitview::git::FileEntry {
    entries
        .iter()
        .find(|e| e.path == PathBuf::from(p))
        .expect("entry for path")
}

#[test]
fn diff_ansi_unstaged_has_color_and_line() {
    let t = fixture("diff-unstaged");
    write(&t.dir, "base.txt", "one\ntwo\nthree\n");
    let entries = t.repo.worktree_status(true).unwrap();
    let e = find(&entries, "base.txt");
    let out = t.repo.diff_ansi(e, Scope::Worktree, false).unwrap();
    // ANSI escape byte present + the newly added line shows up.
    assert!(out.contains(&0x1b), "expected ANSI escape bytes in diff");
    assert!(String::from_utf8_lossy(&out).contains("three"), "expected added line");
}

#[test]
fn diff_ansi_cached_vs_unstaged() {
    let t = fixture("diff-cached");
    write(&t.dir, "base.txt", "one\ntwo\nstaged\n");
    git(&t.dir, &["add", "base.txt"]);
    let entries = t.repo.worktree_status(true).unwrap();
    let e = find(&entries, "base.txt");
    // Staged view shows the change; unstaged view is empty (nothing left).
    let cached = t.repo.diff_ansi(e, Scope::Worktree, true).unwrap();
    assert!(String::from_utf8_lossy(&cached).contains("staged"));
    let unstaged = t.repo.diff_ansi(e, Scope::Worktree, false).unwrap();
    assert!(unstaged.is_empty(), "no unstaged changes expected");
}

#[test]
fn diff_ansi_untracked_full_add() {
    let t = fixture("diff-untracked");
    write(&t.dir, "fresh.txt", "alpha\nbeta\n");
    let entries = t.repo.worktree_status(true).unwrap();
    let e = find(&entries, "fresh.txt");
    let out = t.repo.diff_ansi(e, Scope::Worktree, false).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("alpha") && text.contains("beta"), "full-file add diff");
    assert!(out.contains(&0x1b), "expected ANSI escape bytes");
}

#[test]
fn diff_ansi_branch_scope() {
    let t = fixture("diff-branch");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "base.txt", "one\ntwo\nbranch\n");
    git(&t.dir, &["commit", "-q", "-am", "edit"]);
    let mb = t.repo.merge_base("main").unwrap();
    let entries = t.repo.branch_changes(&mb).unwrap();
    let e = find(&entries, "base.txt");
    let out = t.repo.diff_ansi(e, Scope::Branch, false).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("branch"), "branch-scope change line");
    assert!(out.contains(&0x1b), "expected ANSI escape bytes");
}

#[test]
fn diff_ansi_branch_rename() {
    let t = fixture("diff-rename");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    git(&t.dir, &["mv", "base.txt", "moved.txt"]);
    git(&t.dir, &["commit", "-q", "-m", "rename"]);
    let mb = t.repo.merge_base("main").unwrap();
    let entries = t.repo.branch_changes(&mb).unwrap();
    let e = entries.iter().find(|e| e.kind == ChangeKind::Renamed).expect("renamed entry");
    assert_eq!(e.orig_path, Some(PathBuf::from("base.txt")));
    let out = t.repo.diff_ansi(e, Scope::Branch, false).unwrap();
    let text = String::from_utf8_lossy(&out);
    // Rename diff references the paths (pathspec included both orig + new).
    assert!(text.contains("moved.txt") || text.contains("base.txt"), "rename diff mentions paths");
}

#[test]
fn fingerprint_changes_with_worktree() {
    let t = fixture("fingerprint");
    let clean = t.repo.fingerprint();
    write(&t.dir, "new.txt", "x\n");
    let dirty = t.repo.fingerprint();
    assert_ne!(clean, dirty);
    std::fs::remove_file(t.dir.join("new.txt")).unwrap();
    assert_eq!(t.repo.fingerprint(), clean);
}
