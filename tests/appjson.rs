//! The JSON engine the Herdr app's gitview panel draws from
//! (`herdr-gitview json …`), run as the panel runs it: the real binary
//! against a real repo. These pin the contract the mod relies on.

mod common;

use std::path::Path;
use std::process::Command;

use common::{fixture, write};
use serde_json::Value;

fn gv(repo: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-gitview"))
        .arg("json")
        .args(args)
        .args(["--repo", &repo.display().to_string()])
        .env("GITVIEW_HOST", "app")
        // No personal config: defaults only.
        .env("GITVIEW_CONFIG_DIR", repo.join(".no-config"))
        .output()
        .expect("spawn herdr-gitview");
    let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    assert!(out.status.success(), "json {args:?}: {v}");
    v
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

fn gv_err(repo: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-gitview"))
        .arg("json")
        .args(args)
        .args(["--repo", &repo.display().to_string()])
        .env("GITVIEW_CONFIG_DIR", repo.join(".no-config"))
        .output()
        .expect("spawn herdr-gitview");
    assert!(!out.status.success(), "json {args:?} should fail");
    let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    v["error"].as_str().unwrap().to_string()
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
    assert_eq!(files[0]["stage"], "na");

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
