//! The JSON engine the Herdr app's gitview panel draws from
//! (`herdr-gitview json …`), run as the panel runs it: the real binary
//! against a real repo. These pin the contract the mod relies on.
//!
//! Failure modes this catches:
//! - status grouped differently from the TUI's list (a partly staged file
//!   in one section only, conflicts mixed into changes), or carrying fields
//!   the mods don't read;
//! - a folder path reaching past its own path component (`src` taking
//!   `src-old/`), or acting on another section than the one asked for (a
//!   conflict folder staging plain changes under it);
//! - a discard from "changes" throwing away the staged part too, or a path
//!   without changes discarded silently;
//! - a message or path starting with `--` read as a flag;
//! - a commit that takes conflict markers as resolved, sweeps in untracked
//!   files the list hides, or takes unstaged changes while something is
//!   staged;
//! - a branch base passed back by the caller kept after HEAD moved;
//! - a huge diff's last sent rows left without syntax colors (only the
//!   rows sent are highlighted, so a cap counted wrong shows as plain
//!   text);
//! - HEAD's commit or branch misread: before the first commit, on a branch,
//!   detached;
//! - diff rows without their markers, numbers or tint roles; history, a
//!   commit's files or a commit's own diff (the root commit's too) wrong.

mod common;

use std::path::Path;

use common::{fixture, write};
use serde_json::Value;

/// `json <op> --repo REPO …`: whether it succeeded, and its one JSON object.
/// The config is `REPO/.git/gitview-config/config.toml` when a test wrote
/// one (see [`config`]), else the defaults.
fn json(repo: &Path, args: &[&str]) -> (bool, Value) {
    let out = common::gitview()
        .arg("json")
        .arg(args[0])
        .args(["--repo", &repo.display().to_string()])
        .args(&args[1..])
        .env("GITVIEW_CONFIG_DIR", repo.join(".git/gitview-config"))
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn herdr-gitview");
    let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    (out.status.success(), v)
}

fn gv(repo: &Path, args: &[&str]) -> Value {
    let (ok, v) = json(repo, args);
    assert!(ok, "json {args:?}: {v}");
    v
}

fn gv_err(repo: &Path, args: &[&str]) -> String {
    let (ok, v) = json(repo, args);
    assert!(!ok, "json {args:?} should fail: {v}");
    v["error"].as_str().unwrap().to_string()
}

/// The person's gitview config for this repo (inside `.git`, so it never
/// shows as an untracked file).
fn config(repo: &Path, toml: &str) {
    write(repo, ".git/gitview-config/config.toml", toml);
}

fn section<'a>(status: &'a Value, id: &str) -> Vec<&'a str> {
    status["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .map(|s| {
            s["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["path"].as_str().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn status_groups_like_the_tui_list_and_stage_moves_a_file() {
    let repo = fixture("json-status");
    write(&repo.dir, "base.txt", "one\nTWO\n");
    write(&repo.dir, "new.txt", "fresh\n");
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "changes"), ["base.txt", "new.txt"]);
    assert!(section(&status, "staged").is_empty());

    gv(&repo.dir, &["stage", "base.txt"]);
    write(&repo.dir, "base.txt", "one\nTWO\nthree\n"); // partly staged now
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "staged"), ["base.txt"]);
    // A partly staged file is in both sections, as in the TUI.
    assert_eq!(section(&status, "changes"), ["base.txt", "new.txt"]);
}

#[test]
fn diff_rows_carry_markers_numbers_and_the_tui_colors() {
    let repo = fixture("json-diff");
    write(&repo.dir, "base.txt", "one\n2\n");
    let diff = gv(
        &repo.dir,
        &[
            "diff", "--path", "base.txt", "--kind", "modified", "--theme", "dark",
        ],
    );
    let rows = diff["rows"].as_array().unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r["k"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["ctx", "del", "add"]);
    assert_eq!(rows[1]["m"], "-two");
    assert_eq!(rows[1]["o"], 2);
    assert_eq!(rows[2]["m"], "+2");
    assert_eq!(rows[2]["n"], 2);
    // The TUI's tints come as roles, for the panel to paint in the app's theme.
    assert_eq!(rows[1]["bg"], "@del");
    assert_eq!(rows[2]["bg"], "@ins");
    assert_eq!(diff["first_change"], 2);
}

#[test]
fn discard_restores_the_file_and_notes_render_the_agent_message() {
    let repo = fixture("json-discard");
    write(&repo.dir, "base.txt", "changed\n");
    gv(&repo.dir, &["discard", "base.txt"]);
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("base.txt")).unwrap(),
        "one\ntwo\n"
    );

    let notes = r#"[{"file":"base.txt","start":2,"end":3,"text":"why?\nreally","snippet":"+x\n"}]"#;
    let msg = gv(&repo.dir, &["notes", notes]);
    assert_eq!(
        msg["message"],
        "base.txt:2-3 — why?\n  really\n```diff\n+x\n```\n"
    );
}

#[test]
fn folders_stage_unstage_and_discard_what_their_section_holds() {
    let repo = fixture("json-folders");
    write(&repo.dir, "src/a.txt", "a\n");
    write(&repo.dir, "src/deep/b.txt", "b\n");
    write(&repo.dir, "src-old/c.txt", "c\n");
    write(&repo.dir, "base.txt", "changed\n");

    // A folder (with or without the slash) is everything under it, per
    // path component: `src` leaves `src-old/` alone.
    let staged = gv(&repo.dir, &["stage", "src"]);
    assert_eq!(staged["count"], 2);
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "staged"), ["src/a.txt", "src/deep/b.txt"]);
    assert_eq!(section(&status, "changes"), ["base.txt", "src-old/c.txt"]);

    gv(&repo.dir, &["unstage", "src/deep/"]);
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "staged"), ["src/a.txt"]);

    // `.` is the whole repo; `--section` keeps a discard to one side.
    gv(&repo.dir, &["discard", "--section", "changes", "."]);
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "staged"), ["src/a.txt"]);
    assert!(section(&status, "changes").is_empty());
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("base.txt")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn discard_takes_many_paths_and_refuses_one_without_changes() {
    let repo = fixture("json-discard-many");
    write(&repo.dir, "base.txt", "changed\n");
    write(&repo.dir, "new.txt", "fresh\n");
    write(&repo.dir, "dir/x.txt", "x\n");
    let done = gv(&repo.dir, &["discard", "base.txt", "new.txt", "dir/"]);
    assert_eq!(done["count"], 3);
    assert!(
        gv(&repo.dir, &["status"])["sections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!repo.dir.join("new.txt").exists());
    assert!(gv_err(&repo.dir, &["discard", "base.txt"]).contains("has no changes"));
}

#[test]
fn log_commit_files_and_a_commits_diff() {
    let repo = fixture("json-log");
    common::git(&repo.dir, &["checkout", "-q", "-b", "feature"]);
    write(&repo.dir, "base.txt", "one\n2\n");
    write(&repo.dir, "added.txt", "new\n");
    common::git(&repo.dir, &["add", "."]);
    common::git(&repo.dir, &["commit", "-q", "-m", "feature work"]);

    let log = gv(&repo.dir, &["log"]);
    let commits = log["commits"].as_array().unwrap();
    let subjects: Vec<&str> = commits
        .iter()
        .map(|c| c["subject"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["feature work", "base"]);
    let top = &commits[0];
    assert_eq!(top["author"], "t");
    assert!(
        top["sha"]
            .as_str()
            .unwrap()
            .starts_with(top["short"].as_str().unwrap())
    );
    assert!(top["when"].as_str().unwrap().ends_with("ago"), "{top}");
    assert_eq!(top["refs"], serde_json::json!(["HEAD -> feature"]));
    assert_eq!(commits[1]["refs"], serde_json::json!(["main"]));

    let base_sha = commits[1]["sha"].as_str().unwrap().to_string();
    let only = gv(&repo.dir, &["log", "--merge-base", &base_sha]);
    assert_eq!(only["commits"].as_array().unwrap().len(), 1);
    assert_eq!(
        gv(&repo.dir, &["log", "--limit", "1"])["commits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let sha = top["sha"].as_str().unwrap();
    let files = gv(&repo.dir, &["commit-files", "--sha", sha]);
    let files = files["files"].as_array().unwrap();
    let got: Vec<(&str, &str, u64)> = files
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["letter"].as_str().unwrap(),
                f["add"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(got, [("added.txt", "A", 1), ("base.txt", "M", 1)]);

    // The commit's change, even after the worktree moved on.
    write(&repo.dir, "base.txt", "something else\n");
    let diff = gv(
        &repo.dir,
        &[
            "diff", "--path", "base.txt", "--kind", "modified", "--commit", sha,
        ],
    );
    let marks: Vec<&str> = diff["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["m"].as_str().unwrap())
        .collect();
    assert_eq!(marks, [" one", "-two", "+2"]);
    // The root commit diffs against nothing.
    let root = gv(
        &repo.dir,
        &[
            "diff", "--path", "base.txt", "--kind", "added", "--commit", &base_sha,
        ],
    );
    assert_eq!(root["rows"].as_array().unwrap().len(), 2);
}

/// `git -C dir <args>`'s stdout.
fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The paths a commit changed.
fn committed(dir: &Path, rev: &str) -> Vec<String> {
    git_out(dir, &["show", "--format=", "--name-only", rev])
        .lines()
        .map(str::to_string)
        .collect()
}

/// `main` in the middle of a merge whose one file, `dir/c.txt`, conflicted.
fn conflicted(repo: &Path) {
    let git = |args: &[&str]| common::git(repo, args);
    write(repo, "dir/c.txt", "base\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "c"]);
    git(&["checkout", "-q", "-b", "other"]);
    write(repo, "dir/c.txt", "theirs\n");
    git(&["commit", "-q", "-am", "theirs"]);
    git(&["checkout", "-q", "main"]);
    write(repo, "dir/c.txt", "ours\n");
    git(&["commit", "-q", "-am", "ours"]);
    common::git_lenient(repo, &["merge", "-q", "other"]);
}

#[test]
fn a_folder_in_merge_conflicts_stages_the_conflicts_not_the_changes_under_it() {
    let repo = fixture("json-conflicts");
    conflicted(&repo.dir);
    write(&repo.dir, "dir/plain.txt", "unrelated change\n");
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "conflicts"), ["dir/c.txt"]);
    assert_eq!(section(&status, "changes"), ["dir/plain.txt"]);

    let staged = gv(
        &repo.dir,
        &["stage", "--section", "conflicts", "--", "dir/"],
    );
    assert_eq!(staged["count"], 1);
    let status = gv(&repo.dir, &["status"]);
    assert!(section(&status, "conflicts").is_empty());
    assert_eq!(section(&status, "staged"), ["dir/c.txt"]);
    assert_eq!(section(&status, "changes"), ["dir/plain.txt"], "left alone");
}

#[test]
fn discard_from_changes_keeps_the_staged_part_of_a_partly_staged_file() {
    let repo = fixture("json-partial");
    write(&repo.dir, "base.txt", "one\nTWO\n");
    gv(&repo.dir, &["stage", "--", "base.txt"]);
    write(&repo.dir, "base.txt", "one\nTWO\nthree\n");
    gv(
        &repo.dir,
        &["discard", "--section", "changes", "--", "base.txt"],
    );
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("base.txt")).unwrap(),
        "one\nTWO\n",
        "the worktree is back to the staged version"
    );
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "staged"), ["base.txt"]);
    assert!(section(&status, "changes").is_empty());

    // Without a section: everything goes, as before.
    write(&repo.dir, "base.txt", "one\nTWO\nthree\n");
    gv(&repo.dir, &["discard", "--", "base.txt"]);
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("base.txt")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn after_a_double_dash_everything_is_positional() {
    let repo = fixture("json-dashes");
    write(&repo.dir, "--odd.txt", "x\n");
    gv(&repo.dir, &["stage", "--", "--odd.txt"]);
    let done = gv(&repo.dir, &["commit", "--", "--wip: a message"]);
    assert!(
        done["summary"]
            .as_str()
            .unwrap()
            .ends_with(" --wip: a message"),
        "{done}"
    );
}

#[test]
fn status_carries_only_what_the_mods_read_and_takes_a_base_from_the_caller() {
    let repo = fixture("json-trim");
    common::git(&repo.dir, &["checkout", "-q", "-b", "feature"]);
    write(&repo.dir, "base.txt", "one\n2\n");
    let status = gv(&repo.dir, &["status"]);
    let keys: Vec<&str> = status
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "root",
            "branch",
            "head",
            "scope",
            "sections",
            "editor",
            "ignored_dirs",
            "ignore_submodules"
        ]
    );
    let file = &status["sections"][0]["files"][0];
    let keys: Vec<&str> = file
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["path", "orig", "kind", "letter", "add", "del", "rev"]
    );

    let branch = gv(&repo.dir, &["status", "--scope", "branch"]);
    assert_eq!(branch["base"]["label"], "main");
    // A base passed in (with the HEAD it was for) is used as given, not
    // detected again.
    let sha = branch["base"]["merge_base"].as_str().unwrap().to_string();
    let head = branch["head"].as_str().unwrap().to_string();
    let given = gv(
        &repo.dir,
        &[
            "status",
            "--scope",
            "branch",
            "--base",
            "elsewhere",
            "--merge-base",
            &sha,
            "--head",
            &head,
        ],
    );
    assert_eq!(given["base"]["label"], "elsewhere");
    assert_eq!(given["sections"], branch["sections"]);

    let log = gv(&repo.dir, &["log"]);
    let keys: Vec<&str> = log["commits"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["sha", "short", "subject", "author", "when", "refs"]);
}

#[test]
fn commit_refuses_while_files_are_in_conflict_and_stages_nothing() {
    let repo = fixture("json-commit-conflicts");
    conflicted(&repo.dir);
    let head = git_out(&repo.dir, &["rev-parse", "HEAD"]);
    let markers = std::fs::read_to_string(repo.dir.join("dir/c.txt")).unwrap();
    assert!(markers.contains("<<<<<<<"), "{markers}");

    let err = gv_err(&repo.dir, &["commit", "--", "merge other"]);
    assert_eq!(err, "resolve the merge conflicts first");
    // Nothing moved: no commit, the conflict still a conflict (an `add`
    // would have marked the markers resolved), the merge still going.
    assert_eq!(git_out(&repo.dir, &["rev-parse", "HEAD"]), head);
    let status = gv(&repo.dir, &["status"]);
    assert_eq!(section(&status, "conflicts"), ["dir/c.txt"]);
    assert!(section(&status, "staged").is_empty());
    assert!(repo.dir.join(".git/MERGE_HEAD").exists());
}

#[test]
fn commit_with_nothing_staged_takes_every_change_but_hidden_untracked_files() {
    let repo = fixture("json-commit-all");
    config(&repo.dir, "show_untracked = false\n");
    write(&repo.dir, "base.txt", "one\n2\n");
    write(&repo.dir, "hidden.txt", "not on screen\n");
    let done = gv(&repo.dir, &["commit", "--", "tracked only"]);
    assert_eq!(done["all"], true, "{done}");
    assert!(done["summary"].as_str().unwrap().ends_with(" tracked only"));
    assert_eq!(committed(&repo.dir, "HEAD"), ["base.txt"]);
    assert_eq!(
        git_out(&repo.dir, &["status", "--porcelain"]),
        "?? hidden.txt\n",
        "an untracked file the list didn't show stays out"
    );

    // With untracked files shown, they're part of "everything".
    config(&repo.dir, "show_untracked = true\n");
    write(&repo.dir, "base.txt", "one\n3\n");
    let done = gv(&repo.dir, &["commit", "--", "everything"]);
    assert_eq!(done["all"], true);
    assert_eq!(committed(&repo.dir, "HEAD"), ["base.txt", "hidden.txt"]);
    assert_eq!(git_out(&repo.dir, &["status", "--porcelain"]), "");
}

#[test]
fn commit_with_something_staged_takes_only_the_staged_changes() {
    let repo = fixture("json-commit-staged");
    write(&repo.dir, "staged.txt", "one\n");
    write(&repo.dir, "base.txt", "one\nTWO\n");
    gv(&repo.dir, &["stage", "--", "staged.txt", "base.txt"]);
    // Changes made after staging: a partly staged file, a new untracked one.
    write(&repo.dir, "base.txt", "one\nTWO\nthree\n");
    write(&repo.dir, "later.txt", "later\n");

    let done = gv(&repo.dir, &["commit", "--", "just the staged"]);
    assert_eq!(done["all"], false, "{done}");
    assert_eq!(committed(&repo.dir, "HEAD"), ["base.txt", "staged.txt"]);
    assert_eq!(
        git_out(&repo.dir, &["show", "HEAD:base.txt"]),
        "one\nTWO\n",
        "the staged version, not the worktree's"
    );
    let status = gv(&repo.dir, &["status"]);
    assert!(section(&status, "staged").is_empty());
    assert_eq!(section(&status, "changes"), ["base.txt", "later.txt"]);
}

#[test]
fn a_branch_base_passed_back_is_resolved_again_once_head_moved() {
    let repo = fixture("json-head-moved");
    let git = |args: &[&str]| common::git(&repo.dir, args);
    git(&["checkout", "-q", "-b", "feature"]);
    write(&repo.dir, "mine.txt", "feature work\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "feature work"]);
    let first = gv(&repo.dir, &["status", "--scope", "branch"]);
    let head = first["head"].as_str().unwrap().to_string();
    let base = first["base"]["merge_base"].as_str().unwrap().to_string();
    assert_eq!(head, git_out(&repo.dir, &["rev-parse", "HEAD"]).trim());
    assert_eq!(section(&first, "branch"), ["mine.txt"]);
    let again = |head: &str| {
        gv(
            &repo.dir,
            &[
                "status",
                "--scope",
                "branch",
                "--base",
                "main",
                "--merge-base",
                &base,
                "--head",
                head,
            ],
        )
    };
    // HEAD where it was: the base is used as given.
    assert_eq!(again(&head)["base"]["merge_base"], base);

    // main moves on and feature merges it: the old merge-base would show
    // main's own file as this branch's change.
    git(&["checkout", "-q", "main"]);
    write(&repo.dir, "theirs.txt", "main moved on\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "main work"]);
    git(&["checkout", "-q", "feature"]);
    git(&["merge", "-q", "--no-edit", "main"]);
    let moved = again(&head);
    let main = git_out(&repo.dir, &["rev-parse", "main"]);
    assert_eq!(moved["base"]["merge_base"], main.trim(), "resolved again");
    assert_eq!(section(&moved, "branch"), ["mine.txt"]);
    assert_ne!(moved["head"], head.as_str());
}

#[test]
fn status_names_heads_commit_and_branch_as_git_has_them() {
    let repo = fixture("json-head");
    let d = &repo.dir;
    let st = gv(d, &["status"]);
    assert_eq!(st["branch"], "main");
    assert_eq!(st["head"], git_out(d, &["rev-parse", "HEAD"]).trim());
    common::git(d, &["checkout", "-q", "--detach"]);
    let st = gv(d, &["status"]);
    assert_eq!(st["branch"], Value::Null, "detached");
    assert_eq!(st["head"], git_out(d, &["rev-parse", "HEAD"]).trim());
    common::git(d, &["checkout", "-q", "--orphan", "fresh"]);
    let st = gv(d, &["status"]);
    assert_eq!(st["branch"], "fresh");
    assert_eq!(st["head"], Value::Null, "before the first commit");
}

#[test]
fn every_row_sent_of_a_huge_diff_has_its_syntax_colors() {
    let repo = fixture("json-huge");
    let d = &repo.dir;
    let line = "fn f() -> u32 { 1 }\n";
    write(d, "big.rs", &line.repeat(6000));
    let v = gv(
        d,
        &[
            "diff",
            "--path",
            "big.rs",
            "--kind",
            "untracked",
            "--unfold",
        ],
    );
    assert_eq!(v["truncated"], true, "more rows than are sent");
    let rows = v["rows"].as_array().unwrap();
    let code = |row: &Value| {
        let spans = row["s"].as_array().unwrap();
        let gutter = spans.iter().position(|s| s[1] == "@gutter").unwrap();
        spans[gutter + 1..].to_vec()
    };
    let first = code(&rows[0]);
    assert!(first.len() > 3, "colored: {first:?}");
    assert_eq!(code(rows.last().unwrap()), first, "the last row sent");
}
