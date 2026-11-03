//! Which git the engine runs: `GITVIEW_GIT` when set, for every git call of
//! an op, the content fingerprints' script included (on macOS the default
//! skips Apple's `/usr/bin/git` shim; that choice is measured, not tested).
//!
//! Failure modes this catches:
//! - the override ignored by some calls (a `Command::new("git")` left
//!   behind, the fingerprint script's own `git`);
//! - an override that breaks the op (the reply no longer what git says).

mod common;

use common::{fixture, write};

#[test]
fn every_git_call_of_an_op_runs_the_git_asked_for() {
    let repo = fixture("git-bin");
    let d = &repo.dir;
    write(d, "base.txt", "one\n2\n");
    write(d, "new.txt", "new\n");
    let real = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let log = d.join(".git/calls.log");
    let wrapper = d.join(".git/my-git");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real.trim()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    // `git` on PATH would fail: only the one asked for can answer.
    let trap = d.join(".git/trap");
    std::fs::create_dir_all(&trap).unwrap();
    std::fs::write(trap.join("git"), "#!/bin/sh\necho trapped >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(trap.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = common::gitview()
        .args(["json", "status", "--repo"])
        .arg(d)
        .env("GITVIEW_GIT", &wrapper)
        .env("GITVIEW_CONFIG_DIR", d.join(".git/no-config"))
        .env(
            "PATH",
            format!("{}:{}", trap.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(out.status.success(), "{v}");
    let files: Vec<&str> = v["sections"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["base.txt", "new.txt"]);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains("status --porcelain=v2"), "{calls}");
    assert!(calls.contains("hash-object --no-filters"), "{calls}");
}
