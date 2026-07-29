//! Fixture tests: build real git repos in tempdirs and assert FileEntry
//! vectors. Each test gets its own repo.

use std::path::{Path, PathBuf};
use std::process::Command;

use herdr_gitview::git::{ChangeKind, Repo, StageState};

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

/// Run git with a deterministic identity + config, returning the raw output.
/// Every git invocation must share this identity so operations that record a
/// committer (merge, commit) behave the same on every git version and CI host.
fn git_raw(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn git")
}

fn git(dir: &Path, args: &[&str]) {
    let out = git_raw(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Like `git`, but tolerates a non-zero exit (e.g. a conflicting merge, which
/// git reports with exit code 1 after leaving the conflict markers in place).
fn git_lenient(dir: &Path, args: &[&str]) {
    let _ = git_raw(dir, args);
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn fixture(name: &str) -> TempRepo {
    let dir = std::env::temp_dir().join(format!("gitview-test-{name}-{}", std::process::id()));
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
    let by_path = |p: &str| entries.iter().find(|e| e.path == *p).unwrap();

    let staged = by_path("staged.txt");
    assert_eq!(
        (staged.kind, staged.stage),
        (ChangeKind::Added, StageState::Staged)
    );
    assert_eq!((staged.adds, staged.dels), (Some(1), Some(0)));

    let unstaged = by_path("base.txt");
    assert_eq!(
        (unstaged.kind, unstaged.stage),
        (ChangeKind::Modified, StageState::Unstaged)
    );
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
    let by_path = |p: &str| entries.iter().find(|e| e.path == *p).unwrap();

    let renamed = by_path("renamed.txt");
    assert_eq!(renamed.kind, ChangeKind::Renamed);
    assert_eq!(renamed.orig_path, Some(PathBuf::from("base.txt")));
    assert_eq!(renamed.stage, StageState::Staged);

    let untracked = by_path("new file.txt");
    assert_eq!(
        (untracked.kind, untracked.stage),
        (ChangeKind::Untracked, StageState::Untracked)
    );
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
    let by_path = |p: &str| entries.iter().find(|e| e.path == *p).unwrap();
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
    git_lenient(&t.dir, &["merge", "feature"]);
    // add a second, boring change that would sort before by path
    write(&t.dir, "aaa.txt", "x\n");

    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entries[0].path, PathBuf::from("base.txt"));
    assert_eq!(entries[0].kind, ChangeKind::Conflicted);
    assert_eq!(entries[0].stage, StageState::NA);
    assert_eq!(entries[1].path, PathBuf::from("aaa.txt"));
}

// ---- file content fetching (feeds the structured diff renderer) -----------

#[test]
fn file_at_reads_index_head_and_commits() {
    let t = fixture("file-at");
    // modify + stage, then modify again: three distinct versions
    write(&t.dir, "base.txt", "staged version\n");
    git(&t.dir, &["add", "base.txt"]);
    write(&t.dir, "base.txt", "worktree version\n");

    assert_eq!(
        t.repo
            .file_at("HEAD", Path::new("base.txt"))
            .unwrap()
            .as_deref(),
        Some("one\ntwo\n")
    );
    assert_eq!(
        t.repo
            .file_at(":0", Path::new("base.txt"))
            .unwrap()
            .as_deref(),
        Some("staged version\n")
    );
    assert_eq!(
        t.repo.file_in_worktree(Path::new("base.txt")).as_deref(),
        Some("worktree version\n")
    );
    // missing path at a rev → None, not an error
    assert_eq!(t.repo.file_at("HEAD", Path::new("nope.txt")).unwrap(), None);
}

#[test]
fn file_at_commit_sha_and_parent() {
    let t = fixture("file-at-sha");
    write(&t.dir, "base.txt", "second\n");
    git(&t.dir, &["commit", "-q", "-am", "second"]);
    let sha = t.repo.log_commits(1).unwrap()[0].sha.clone();

    assert_eq!(
        t.repo
            .file_at(&sha, Path::new("base.txt"))
            .unwrap()
            .as_deref(),
        Some("second\n")
    );
    assert_eq!(
        t.repo
            .file_at(&format!("{sha}^"), Path::new("base.txt"))
            .unwrap()
            .as_deref(),
        Some("one\ntwo\n")
    );
}

// ---- phase 5: write side ---------------------------------------------------

fn entry_for<'a>(
    entries: &'a [herdr_gitview::git::FileEntry],
    p: &str,
) -> &'a herdr_gitview::git::FileEntry {
    entries
        .iter()
        .find(|e| e.path == *p)
        .unwrap_or_else(|| panic!("no entry for {p}"))
}

fn status_len(t: &TempRepo) -> usize {
    t.repo.worktree_status(true).unwrap().len()
}

#[test]
fn stage_unstage_round_trip() {
    let t = fixture("stage-round-trip");
    write(&t.dir, "base.txt", "one\ntwo\nmore\n");

    t.repo.stage(Path::new("base.txt")).unwrap();
    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entry_for(&entries, "base.txt").stage, StageState::Staged);
    assert_eq!(t.repo.staged_count().unwrap(), 1);

    t.repo.unstage(Path::new("base.txt")).unwrap();
    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entry_for(&entries, "base.txt").stage, StageState::Unstaged);
    assert_eq!(t.repo.staged_count().unwrap(), 0);
}

#[test]
fn discard_untracked_deletes_file() {
    let t = fixture("discard-untracked");
    write(&t.dir, "loose.txt", "x\n");
    let entries = t.repo.worktree_status(true).unwrap();
    t.repo.discard(entry_for(&entries, "loose.txt")).unwrap();
    assert!(!t.dir.join("loose.txt").exists());
    assert_eq!(status_len(&t), 0);
}

#[test]
fn discard_unstaged_restores_head() {
    let t = fixture("discard-unstaged");
    write(&t.dir, "base.txt", "changed\n");
    let entries = t.repo.worktree_status(true).unwrap();
    t.repo.discard(entry_for(&entries, "base.txt")).unwrap();
    assert_eq!(
        std::fs::read_to_string(t.dir.join("base.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(status_len(&t), 0);
}

#[test]
fn discard_staged_and_partial_full_revert() {
    let t = fixture("discard-staged");
    // staged modification
    write(&t.dir, "base.txt", "staged edit\n");
    t.repo.stage(Path::new("base.txt")).unwrap();
    // …then another unstaged edit on top → partial
    write(&t.dir, "base.txt", "staged edit\nplus unstaged\n");

    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entry_for(&entries, "base.txt").stage, StageState::Partial);
    t.repo.discard(entry_for(&entries, "base.txt")).unwrap();
    assert_eq!(
        std::fs::read_to_string(t.dir.join("base.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(status_len(&t), 0);
}

#[test]
fn discard_staged_new_file_removes_it() {
    let t = fixture("discard-added");
    write(&t.dir, "brand-new.txt", "n\n");
    t.repo.stage(Path::new("brand-new.txt")).unwrap();
    let entries = t.repo.worktree_status(true).unwrap();
    t.repo
        .discard(entry_for(&entries, "brand-new.txt"))
        .unwrap();
    assert!(!t.dir.join("brand-new.txt").exists());
    assert_eq!(status_len(&t), 0);
}

#[test]
fn discard_rename_restores_old_path() {
    let t = fixture("discard-rename");
    git(&t.dir, &["mv", "base.txt", "moved.txt"]);
    let entries = t.repo.worktree_status(true).unwrap();
    t.repo.discard(entry_for(&entries, "moved.txt")).unwrap();
    assert!(t.dir.join("base.txt").exists());
    assert!(!t.dir.join("moved.txt").exists());
    assert_eq!(status_len(&t), 0);
}

#[test]
fn discard_conflicted_is_refused() {
    let t = fixture("discard-conflict");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "base.txt", "feature\n");
    git(&t.dir, &["commit", "-q", "-am", "f"]);
    git(&t.dir, &["checkout", "-q", "main"]);
    write(&t.dir, "base.txt", "main\n");
    git(&t.dir, &["commit", "-q", "-am", "m"]);
    git_lenient(&t.dir, &["merge", "feature"]);

    let entries = t.repo.worktree_status(true).unwrap();
    let err = t.repo.discard(entry_for(&entries, "base.txt")).unwrap_err();
    assert!(err.to_string().contains("conflict"), "got: {err}");
    // file untouched by the refusal
    assert!(
        std::fs::read_to_string(t.dir.join("base.txt"))
            .unwrap()
            .contains("<<<<<<<")
    );
}

// ---- branch-scoped log -----------------------------------------------------

#[test]
fn log_branch_commits_only_lists_this_branch() {
    let t = fixture("log-branch");
    // two more commits on main, then a feature branch with two of its own
    write(&t.dir, "base.txt", "main2\n");
    git(&t.dir, &["commit", "-q", "-am", "main second"]);
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f1.txt", "a\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature one"]);
    write(&t.dir, "f2.txt", "b\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature two"]);

    let all = t.repo.log_commits(50).unwrap();
    assert_eq!(all.len(), 4, "full history: base + main second + 2 feature");

    let mb = t.repo.merge_base("main").unwrap();
    let branch = t.repo.log_branch_commits(&mb, 50).unwrap();
    let subjects: Vec<&str> = branch.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["feature two", "feature one"]);
}

#[test]
fn log_branch_commits_is_empty_on_the_base_branch() {
    let t = fixture("log-branch-empty");
    let mb = t.repo.merge_base("main").unwrap();
    assert!(t.repo.log_branch_commits(&mb, 50).unwrap().is_empty());
}

// ---- folder-wide staging ---------------------------------------------------

#[test]
fn stage_and_unstage_many_move_a_whole_folder() {
    let t = fixture("stage-many");
    write(&t.dir, "src/a.rs", "a\n");
    write(&t.dir, "src/deep/b.rs", "b\n");
    write(&t.dir, "other.rs", "o\n");

    let paths: Vec<PathBuf> = vec!["src/a.rs".into(), "src/deep/b.rs".into()];
    t.repo.stage_many(&paths).unwrap();

    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entry_for(&entries, "src/a.rs").stage, StageState::Staged);
    assert_eq!(
        entry_for(&entries, "src/deep/b.rs").stage,
        StageState::Staged
    );
    // the sibling outside the folder is untouched
    assert_eq!(entry_for(&entries, "other.rs").stage, StageState::Untracked);

    t.repo.unstage_many(&paths).unwrap();
    let entries = t.repo.worktree_status(true).unwrap();
    assert_eq!(entry_for(&entries, "src/a.rs").stage, StageState::Untracked);
    assert_eq!(
        entry_for(&entries, "src/deep/b.rs").stage,
        StageState::Untracked
    );
}

#[test]
fn stage_many_with_no_paths_is_a_no_op() {
    let t = fixture("stage-many-empty");
    t.repo.stage_many(&[]).unwrap();
    t.repo.unstage_many(&[]).unwrap();
    assert_eq!(status_len(&t), 0);
}

// ---- fork-point base detection ---------------------------------------------

/// `main` → `develop` → `feature`: the base must be `develop`, the branch
/// `feature` was actually cut from, not `main`.
#[test]
fn base_is_the_branch_this_one_was_cut_from() {
    let t = fixture("base-fork-chain");
    git(&t.dir, &["checkout", "-q", "-b", "develop"]);
    write(&t.dir, "d.txt", "d\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "develop work"]);

    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);

    assert_eq!(t.repo.detect_base(), "develop");
    // ...and the diff is scoped to the feature's own work.
    let (_, mb) = t.repo.resolve_base("").unwrap();
    let paths: Vec<String> = t
        .repo
        .branch_changes(&mb)
        .unwrap()
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert_eq!(paths, vec!["f.txt"], "should not include develop's work");
}

/// The parent moving on after the branch was cut must not change the answer.
#[test]
fn base_survives_the_parent_advancing() {
    let t = fixture("base-parent-advanced");
    git(&t.dir, &["checkout", "-q", "-b", "develop"]);
    write(&t.dir, "d.txt", "d\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "develop work"]);

    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);

    // develop gains a commit after feature branched off it.
    git(&t.dir, &["checkout", "-q", "develop"]);
    write(&t.dir, "d2.txt", "d2\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "more develop"]);
    git(&t.dir, &["checkout", "-q", "feature"]);

    assert_eq!(t.repo.detect_base(), "develop");
}

/// Branched straight off the trunk: still the trunk.
#[test]
fn base_of_a_branch_cut_from_main_is_main() {
    let t = fixture("base-from-main");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);
    assert_eq!(t.repo.detect_base(), "main");
}

/// Two branches cut from the same commit are tied on recency; the trunk wins
/// rather than a sibling feature branch.
#[test]
fn a_sibling_branch_does_not_beat_the_trunk_on_a_tie() {
    let t = fixture("base-sibling-tie");
    git(&t.dir, &["checkout", "-q", "-b", "sibling"]);
    write(&t.dir, "s.txt", "s\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "sibling work"]);

    git(&t.dir, &["checkout", "-q", "main"]);
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);

    assert_eq!(t.repo.detect_base(), "main");
}

/// A branch cut *from* this one contains all of HEAD, so it is not a parent.
#[test]
fn a_branch_cut_from_this_one_is_not_its_base() {
    let t = fixture("base-child-branch");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);
    // A branch created from feature, left exactly where feature is.
    git(&t.dir, &["branch", "child"]);

    assert_eq!(t.repo.detect_base(), "main");
}

/// The branch's own remote-tracking ref is not a parent either — otherwise
/// unpushed commits would be the only thing branch scope ever showed.
#[test]
fn the_branchs_own_remote_ref_is_not_its_base() {
    let t = fixture("base-own-remote");
    // A bare "remote" with main plus the feature branch pushed to it.
    let remote = t.dir.parent().unwrap().join("base-own-remote-origin.git");
    let _ = std::fs::remove_dir_all(&remote);
    git(&t.dir, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(
        &t.dir,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&t.dir, &["push", "-q", "origin", "main"]);

    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "pushed work"]);
    git(&t.dir, &["push", "-q", "-u", "origin", "feature"]);
    // ...plus a local commit that is not pushed.
    write(&t.dir, "g.txt", "g\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "local work"]);

    let base = t.repo.detect_base();
    assert!(
        base == "main" || base == "origin/main",
        "picked its own remote ref: {base}"
    );
    let (_, mb) = t.repo.resolve_base("").unwrap();
    let paths: Vec<String> = t
        .repo
        .branch_changes(&mb)
        .unwrap()
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert_eq!(
        paths,
        vec!["f.txt", "g.txt"],
        "both commits are this branch's"
    );
    let _ = std::fs::remove_dir_all(&remote);
}

/// A configured base still wins over the guess.
#[test]
fn a_configured_base_overrides_detection() {
    let t = fixture("base-configured");
    git(&t.dir, &["checkout", "-q", "-b", "develop"]);
    write(&t.dir, "d.txt", "d\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "develop work"]);
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);

    let (base, _) = t.repo.resolve_base("main").unwrap();
    assert_eq!(base, "main");
}

/// On the trunk itself there is no parent to find; detection must not pick a
/// child branch and must fall back cleanly.
#[test]
fn detection_falls_back_on_the_trunk_itself() {
    let t = fixture("base-on-trunk");
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f.txt", "f\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature work"]);
    git(&t.dir, &["checkout", "-q", "main"]);

    // `feature` contains all of main, so it is skipped; nothing else exists.
    assert_eq!(t.repo.detect_base(), "main");
}

/// On a trunk, every branch ever merged into it is an ancestor. Picking the
/// most recent of those would make branch scope mean "the last few commits on
/// main", so a trunk must fall back to the conventional base instead.
#[test]
fn on_a_trunk_a_merged_branch_is_not_the_base() {
    let t = fixture("base-trunk-with-merges");
    git(&t.dir, &["checkout", "-q", "-b", "shipped"]);
    write(&t.dir, "s.txt", "s\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "shipped work"]);
    git(&t.dir, &["checkout", "-q", "main"]);
    git(
        &t.dir,
        &["merge", "-q", "--no-ff", "-m", "merge shipped", "shipped"],
    );

    assert_ne!(
        t.repo.detect_base(),
        "shipped",
        "a merged branch is not what main is diffed against"
    );
    assert_eq!(t.repo.detect_base(), "main");
}

/// The parent does not have to be conventionally named: a branch stacked on
/// another feature branch diffs against that feature branch.
#[test]
fn a_stacked_branch_diffs_against_the_branch_below_it() {
    let t = fixture("base-stacked");
    git(&t.dir, &["checkout", "-q", "-b", "nv-1-first-part"]);
    write(&t.dir, "one.txt", "1\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "first part"]);

    git(&t.dir, &["checkout", "-q", "-b", "nv-2-second-part"]);
    write(&t.dir, "two.txt", "2\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "second part"]);

    assert_eq!(t.repo.detect_base(), "nv-1-first-part");
    let (_, mb) = t.repo.resolve_base("").unwrap();
    let paths: Vec<String> = t
        .repo
        .branch_changes(&mb)
        .unwrap()
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert_eq!(paths, vec!["two.txt"], "only this branch's own work");
    // The log filter follows the same base.
    let subjects: Vec<String> = t
        .repo
        .log_branch_commits(&mb, 10)
        .unwrap()
        .iter()
        .map(|c| c.subject.clone())
        .collect();
    assert_eq!(subjects, vec!["second part"]);
}

/// A detached HEAD has no branch to reason about; detection must not panic
/// and the fallback still applies.
#[test]
fn detached_head_falls_back_without_panicking() {
    let t = fixture("base-detached");
    write(&t.dir, "x.txt", "x\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "second"]);
    let sha = t.repo.log_commits(1).unwrap()[0].sha.clone();
    git(&t.dir, &["checkout", "-q", &sha]);
    assert!(t.repo.head_branch().is_none());
    let base = t.repo.detect_base();
    assert!(!base.is_empty());
}

// ---- repositories git refuses to report on ---------------------------------

/// A path recorded as a submodule but present on disk as a symlink — how some
/// worktree setups link back to their main checkout. `git status` exits 128
/// with *no output* ("expected submodule path 'x' not to be a symbolic
/// link"), which used to take the whole file list down with it.
fn submodule_symlink_repo(name: &str) -> TempRepo {
    let t = fixture(name);
    let sha = t.repo.head_sha().unwrap();
    git(
        &t.dir,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sha},.source"),
        ],
    );
    std::os::unix::fs::symlink("/tmp/nowhere-in-particular", t.dir.join(".source")).unwrap();
    t
}

#[test]
fn status_survives_a_submodule_path_that_is_a_symlink() {
    let t = submodule_symlink_repo("status-submodule-symlink");
    write(&t.dir, "base.txt", "one\ntwo\nchanged\n");
    write(&t.dir, "fresh.txt", "new\n");

    // Plain status really does fail here — the fallback is load-bearing.
    let plain = std::process::Command::new("git")
        .arg("-C")
        .arg(&t.dir)
        .args(["status", "--porcelain=v2", "-z"])
        .output()
        .unwrap();
    assert!(!plain.status.success(), "fixture no longer reproduces");

    let entries = t.repo.worktree_status(true).unwrap();
    let paths: Vec<String> = entries
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert!(paths.contains(&"base.txt".to_string()), "got {paths:?}");
    assert!(paths.contains(&"fresh.txt".to_string()), "got {paths:?}");
    // The submodule path itself is still reported as changed; only its
    // internal dirtiness is hidden.
    assert!(paths.contains(&".source".to_string()), "got {paths:?}");
}

#[test]
fn auto_refresh_still_sees_changes_when_status_needs_the_fallback() {
    let t = submodule_symlink_repo("status-fallback-fingerprint");
    let before = t.repo.fingerprint(true);
    write(&t.dir, "base.txt", "one\ntwo\nchanged\n");
    let after = t.repo.fingerprint(true);
    assert_ne!(
        before, after,
        "fingerprint went blind, so polling would too"
    );
}

#[test]
fn the_file_list_starts_even_when_git_status_cannot_run() {
    // No fallback can save a directory that is not a repository at all; the
    // pane must still come up and say so, rather than exiting and leaving the
    // diff pane waiting for a file list that never arrives.
    let dir = std::env::temp_dir().join(format!("gitview-not-a-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = Repo { root: dir.clone() };
    let app = herdr_gitview::list::App::new(
        repo,
        herdr_gitview::config::Config::default(),
        herdr_gitview::keymap::Keymap::build(&std::collections::HashMap::new()).unwrap(),
    )
    .expect("the pane must start anyway");
    assert!(app.entries.is_empty());
    assert!(
        app.active_status().unwrap_or_default().contains("status"),
        "no explanation shown: {:?}",
        app.active_status()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
