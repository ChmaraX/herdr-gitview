//! Which git the engine runs: `GITVIEW_GIT` when set, for every git call of
//! an op, the content fingerprints' script included (on macOS the default
//! skips Apple's `/usr/bin/git` shim; that choice is measured, not tested).
//!
//! Failure modes this catches:
//! - the override ignored by some calls (a `Command::new("git")` left
//!   behind, the fingerprint script's own `git`);
//! - an override that breaks the op (the reply no longer what git says);
//! - the repo's top level looked up again for an op on it (`.git` is
//!   there), or taken as given for a folder inside it (no `.git`: the
//!   paths would be read from the wrong folder).

mod common;

use std::path::{Path, PathBuf};

use common::{fixture, write};

/// A git that logs its arguments to `<d>/.git/calls.log`, then runs git.
fn logging_git(d: &Path) -> (PathBuf, PathBuf) {
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
    (wrapper, log)
}

#[test]
fn every_git_call_of_an_op_runs_the_git_asked_for() {
    let repo = fixture("git-bin");
    let d = &repo.dir;
    write(d, "base.txt", "one\n2\n");
    write(d, "new.txt", "new\n");
    let (wrapper, log) = logging_git(d);
    use std::os::unix::fs::PermissionsExt;
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

#[test]
fn an_op_on_the_top_level_doesnt_look_it_up_again() {
    let repo = fixture("git-root");
    let d = &repo.dir;
    write(d, "sub/f.txt", "x\n");
    common::git(d, &["add", "."]);
    common::git(d, &["commit", "-q", "-m", "sub"]);
    write(d, "sub/f.txt", "y\n");
    let (wrapper, log) = logging_git(d);
    let diff = |dir: &Path| {
        let _ = std::fs::remove_file(&log);
        let out = common::gitview()
            .args([
                "json",
                "diff",
                "--path",
                "sub/f.txt",
                "--kind",
                "modified",
                "--repo",
            ])
            .arg(dir)
            .env("GITVIEW_GIT", &wrapper)
            .env("GITVIEW_CONFIG_DIR", d.join(".git/no-config"))
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(out.status.success(), "{v}");
        (
            v["hash"].clone(),
            std::fs::read_to_string(&log).unwrap_or_default(),
        )
    };
    let (at_root, calls) = diff(d);
    assert!(!calls.contains("rev-parse"), "{calls}");
    // A folder inside: looked up, and the same diff.
    let (inside, calls) = diff(&d.join("sub"));
    assert!(calls.contains("rev-parse --show-toplevel"), "{calls}");
    assert_eq!(inside, at_root);
}
