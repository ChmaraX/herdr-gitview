//! `GITVIEW_TRACE`: the line each `json <op>` appends for the lead to read,
//! run as the mods run the binary. (The same op on a remote, its `hc exec`
//! count against the fake hc's own log, is in `tests/remote.rs`.)
//!
//! Failure modes this catches:
//! - an op without its line, or with two; a line written with the trace
//!   unset (into some default file);
//! - a line that isn't one JSON object (ops at once tearing each other's
//!   lines);
//! - the person's words in the trace: a commit message, notes, text sent;
//! - a failed op traced as ok, or without its error; a `--unless` hit not
//!   told apart;
//! - git calls not counted (the line can't say an op got slow from more
//!   git), or counted for a process that made none;
//! - the file growing without bound, or its old part thrown away instead
//!   of kept as `<name>.1.jsonl`.

mod common;

use std::path::{Path, PathBuf};

use common::{fixture, write};
use serde_json::Value;

fn trace_file(repo: &Path) -> PathBuf {
    repo.join(".git/gitview-trace/engine.jsonl")
}

/// `json <op> --repo REPO …` with the trace on; whether it succeeded.
fn traced(repo: &Path, args: &[&str]) -> bool {
    run(repo, args, true).0
}

/// `json <op> --repo REPO …`, traced or not: (succeeded, reply).
fn run(repo: &Path, args: &[&str], trace: bool) -> (bool, Value) {
    let mut cmd = common::gitview();
    cmd.arg("json")
        .arg(args[0])
        .args(["--repo", &repo.display().to_string()])
        .args(&args[1..])
        .env("GITVIEW_CONFIG_DIR", repo.join(".git/no-config"))
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env_remove("GITVIEW_TRACE");
    if trace {
        cmd.env("GITVIEW_TRACE", trace_file(repo));
    }
    let out = cmd.output().unwrap();
    (
        out.status.success(),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

fn lines(repo: &Path) -> Vec<Value> {
    std::fs::read_to_string(trace_file(repo))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|_| panic!("not one object: {l}")))
        .collect()
}

#[test]
fn each_op_appends_one_line_saying_what_it_did() {
    let repo = fixture("trace-ops");
    let d = &repo.dir;
    write(d, "base.txt", "one\nTWO\n");
    write(d, "new.txt", "fresh\n");

    // Off: nothing anywhere.
    assert!(run(d, &["status"], false).0);
    assert!(
        !trace_file(d).parent().unwrap().exists(),
        "written while off"
    );

    assert!(traced(d, &["status"]));
    assert!(traced(d, &["stage", "--", "base.txt"]));
    assert!(traced(d, &["commit", "--", "secret commit words"]));
    assert!(traced(
        d,
        &[
            "notes",
            "--",
            r#"[{"file":"a","start":1,"end":1,"text":"secret note words","snippet":""}]"#
        ]
    ));
    assert!(!traced(
        d,
        &["diff", "--path", "/etc/hosts", "--kind", "untracked"]
    ));
    let diff = ["diff", "--path", "new.txt", "--kind", "untracked"];
    let (ok, reply) = run(d, &diff, true);
    assert!(ok, "{reply}");
    let h = reply["hash"].as_str().unwrap();
    assert!(traced(
        d,
        &[
            "diff",
            "--path",
            "new.txt",
            "--kind",
            "untracked",
            "--unless",
            h
        ]
    ));

    let all = lines(d);
    let ops: Vec<&str> = all.iter().map(|l| l["op"].as_str().unwrap()).collect();
    assert_eq!(
        ops,
        ["status", "stage", "commit", "notes", "diff", "diff", "diff"]
    );
    let text = std::fs::read_to_string(trace_file(d)).unwrap();
    assert!(
        !text.contains("secret"),
        "the person's words in the trace: {text}"
    );

    let [status, stage, commit, notes, refused, shown, same] = &all[..] else {
        unreachable!()
    };
    for l in &all {
        assert_eq!(l["mod"], "engine");
        assert_eq!(l["kind"], "exec");
        assert_eq!(l["remote"], false);
        assert_eq!(l["execs"], 0, "{l}");
        assert!(l["t"].as_u64().unwrap() > 1_600_000_000_000, "{l}");
        assert!(l["ms"].as_f64().unwrap() >= 0.0, "{l}"); // a fast op rounds to 0.0
        assert!(l["pid"].as_u64().is_some(), "{l}");
    }
    assert_eq!(status["ok"], true);
    assert!(
        status["git"].as_u64().unwrap() >= 2,
        "status runs git: {status}"
    );
    assert_eq!(stage["paths"], serde_json::json!(["base.txt"]));
    assert_eq!(commit["text_bytes"], "secret commit words".len());
    assert!(commit["paths"].is_null());
    assert_eq!(notes["git"], 0, "notes runs no git: {notes}");
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("a path inside the repo"),
        "{refused}"
    );
    assert_eq!(refused["args"]["path"], "/etc/hosts");
    assert_eq!(
        (shown["same"].clone(), same["same"].clone()),
        (false.into(), true.into())
    );
    assert_eq!(same["args"]["unless"], h);
}

#[test]
fn ops_at_once_each_get_one_whole_line() {
    let repo = fixture("trace-parallel");
    let d = repo.dir.clone();
    let runs: Vec<_> = (0..24)
        .map(|_| {
            let d = d.clone();
            std::thread::spawn(move || assert!(traced(&d, &["status"])))
        })
        .collect();
    for r in runs {
        r.join().unwrap();
    }
    assert_eq!(lines(&d).len(), 24);
}

#[test]
fn a_full_trace_moves_aside_and_starts_again() {
    let repo = fixture("trace-rotate");
    let d = &repo.dir;
    let file = trace_file(d);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let old = "{\"old\":true}\n".repeat(700_000); // past 8 MB
    std::fs::write(&file, &old).unwrap();
    assert!(traced(d, &["status"]));
    assert_eq!(lines(d).len(), 1, "a fresh file with this op's line");
    let kept = std::fs::read_to_string(file.with_file_name("engine.1.jsonl")).unwrap();
    assert_eq!(kept, old);
}
