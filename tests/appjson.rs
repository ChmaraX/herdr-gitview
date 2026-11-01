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
    // The dark palette's deletion tint, as the TUI paints it.
    assert_eq!(rows[1]["bg"], "#45232f");
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
