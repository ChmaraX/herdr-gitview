//! `herdr-gitview json <op> --on <machine>`: a repo on another machine is
//! only ever reached through `hc exec --on <machine> --cwd <dir> -- …`.
//!
//! The "machine" is a fake `hc` (a bash shim, via `GITVIEW_HC`) that strips
//! `exec --on box --cwd D [--timeout N] [-i] --`, maps the remote path prefix
//! `/gitview-box-<pid>` (which doesn't exist here) onto a scratch folder, runs
//! the command there and logs every call. Two identical repos (fixed dates,
//! so even commit ids match) get the same ops, one here and one "on box",
//! and must give the same JSON. The remote repo's path doesn't exist on this
//! machine, the `git` on `PATH` is a trap that records any call, and the
//! child runs in an empty folder: anything that reached the repo except
//! through the shim fails or shows.
//!
//! Failure modes this catches:
//! - any local git call or local file read of the remote repo (the trap
//!   `git`, the path that doesn't exist here, the empty working folder);
//! - a reply that differs between here and there (untracked counts, diffs,
//!   commit ids), or an hc call outside `exec --on box --cwd <remote>`;
//! - stdin not fed (`-i`) to `cat-file --batch`, or a commit cut at hc's
//!   60 s default;
//! - untracked files measured with a process per file instead of one call
//!   per batch;
//! - a failed remote read shown as a deleted file, an unknown machine
//!   falling back to this one;
//! - a branch-scope base passed back after HEAD moved on the machine (a
//!   rebase, a merge of the base) kept as is, or base detection run on every
//!   poll while HEAD stays put;
//! - `json file` there: an image that doesn't arrive byte for byte (from
//!   the worktree or from git), or text, binary, too big and missing told
//!   apart differently from here, or a failed read there taken as missing;
//! - a diff whose git reads fail there (the machine dropped) shown as a
//!   whole file added or as no change, instead of the error;
//! - `--root` (the top level status gave) looked up there again anyway, or
//!   a reply other than without it;
//! - the reload hints (`rev`, `ignored_dirs`, a diff's `hash` and its
//!   `--unless`) told from this machine's files, or other there than here,
//!   or the revs costing more than one call per status;
//! - `json editor` there answered from this machine (its `PATH`, `$VISUAL`,
//!   its system opener) instead of that machine's login shell.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const SHIM: &str = r#"#!/bin/bash
all="$*"; printf '%s\n' "${all//$'\n'/ }" >> "$SHIM_LOG"
[ "$1" = exec ] || { echo "shim: not exec: $*" >&2; exit 90; }
shift
cwd=; stdin=
while [ $# -gt 0 ]; do
  case "$1" in
    --on) [ "$2" = "$SHIM_MACHINE" ] || { echo "shim: unknown machine $2" >&2; exit 91; }; shift 2 ;;
    --cwd) cwd=$2; shift 2 ;;
    --timeout) shift 2 ;;
    -i) stdin=1; shift ;;
    --) shift; break ;;
    *) echo "shim: option $1" >&2; exit 92 ;;
  esac
done
case "$cwd" in
  "$SHIM_FAKE"*) cwd="$SHIM_REAL${cwd#"$SHIM_FAKE"}" ;;
  *) echo "shim: cwd $cwd is not on the machine" >&2; exit 93 ;;
esac
[ "$1" = git ] && { shift; set -- "$SHIM_GIT" "$@"; }
# The machine's own git is first on its PATH (for a script's `git`).
PATH="${SHIM_GIT%/*}:$PATH"
cd "$cwd" || exit 94
[ -n "$stdin" ] || exec </dev/null
if [ -n "$SHIM_FAIL_GIT_READS" ] && [[ " $* " == *" cat-file "* || " $* " == *" show "* ]]; then
  echo '{"error":{"kind":"machine_unreachable","message":"machine \"box\" is unreachable","hint":"check ssh"}}' >&2; exit 3
fi
if [ -n "$SHIM_FAIL_READS" ] && [ "$1" = sh ] && [[ "$3" == *"exit 44"* ]]; then
  echo "shim: the connection dropped" >&2; exit 255
fi
# The machine's login shell: its own PATH, $VISUAL and $EDITOR, nothing of
# the caller's environment.
if [ -n "${SHIM_LOGIN_PATH+x}" ] && [ "$1" = sh ] && [ "$2" = -lc ]; then
  shift 2
  exec env -i HOME="$cwd" PATH="$SHIM_LOGIN_PATH" VISUAL="${SHIM_VISUAL-}" EDITOR="${SHIM_EDITOR-}" /bin/sh -c "$@"
fi
if [[ " $* " == *" rev-parse --show-toplevel "* ]]; then
  out=$("$@") || exit $?
  printf '%s\n' "$SHIM_FAKE${out#"$SHIM_REAL"}"
  exit 0
fi
exec "$@"
"#;

const TRAP_GIT: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TRAP_LOG"
exit 1
"#;

/// Fixed identity and dates: both repos get identical commit ids.
const GIT_ENV: [(&str, &str); 8] = [
    ("GIT_AUTHOR_NAME", "t"),
    ("GIT_AUTHOR_EMAIL", "t@t"),
    ("GIT_COMMITTER_NAME", "t"),
    ("GIT_COMMITTER_EMAIL", "t@t"),
    ("GIT_AUTHOR_DATE", "2024-01-02T03:04:05Z"),
    ("GIT_COMMITTER_DATE", "2024-01-02T03:04:05Z"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

struct World {
    tmp: PathBuf,
    /// The repo here.
    here: PathBuf,
    /// The same repo "on box": its path there (doesn't exist here).
    remote: PathBuf,
    /// Where the remote repo really is (for the test's own checks).
    remote_real: PathBuf,
    git: String,
    hc: PathBuf,
    trap_dir: PathBuf,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(GIT_ENV)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn write(dir: &Path, rel: &str, content: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A repo with history on `main` and a `feature` branch, plus every kind of
/// change: modified, staged, untracked (text without a final newline, a
/// binary, one in a folder) and a deletion.
fn build_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    write(dir, "base.txt", b"one\ntwo\nthree\n");
    write(dir, "gone.txt", b"bye\n");
    write(dir, "src/lib.rs", b"fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "base"]);
    git(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "src/lib.rs", b"fn a() {}\nfn b() {}\n");
    git(dir, &["commit", "-q", "-am", "feature one"]);
    write(dir, "base.txt", b"one\nTWO\nthree\n");
    write(dir, "staged.txt", b"staged\n");
    git(dir, &["add", "staged.txt"]);
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    write(dir, "new.txt", b"a\nb");
    write(dir, "blob.bin", b"\0\x01\x02");
    write(dir, "dir/deep.txt", b"deep\n");
}

fn world(name: &str) -> World {
    let tmp = std::env::temp_dir().join(format!("gitview-remote-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("box")).unwrap();
    let tmp = tmp.canonicalize().unwrap();
    let here = tmp.join("here/repo");
    build_repo(&here);
    let remote_real = tmp.join("box/repo");
    build_repo(&remote_real);

    let git = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let hc = tmp.join("hc");
    exe(&hc, SHIM);
    let trap_dir = tmp.join("trap");
    std::fs::create_dir_all(&trap_dir).unwrap();
    exe(&trap_dir.join("git"), TRAP_GIT);
    std::fs::create_dir_all(tmp.join("empty")).unwrap();
    World {
        here,
        remote: PathBuf::from(format!("/gitview-box-{name}-{}/repo", std::process::id())),
        remote_real,
        git,
        hc,
        trap_dir,
        tmp,
    }
}

impl World {
    fn fake_prefix(&self) -> PathBuf {
        self.remote.parent().unwrap().to_path_buf()
    }

    fn shim_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.tmp.join("shim.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn trap_log(&self) -> String {
        std::fs::read_to_string(self.tmp.join("trap.log")).unwrap_or_default()
    }

    /// `json <op> <args> --repo …` here, and the same `--on box`, with the
    /// repo root in the replies replaced by `ROOT`. Both must succeed.
    fn both(&self, args: &[&str]) -> Value {
        let local = self.run(args, false);
        let remote = self.run(args, true);
        assert_eq!(local, remote, "json {args:?}: here vs --on box");
        local
    }

    /// `cmd` (a `json` op) against the remote repo, with `extra` env for
    /// the shim; (succeeded, reply).
    fn run_with(&self, mut cmd: Command, extra: &[(&str, &str)]) -> (bool, Value) {
        cmd.args(["--on", "box", "--repo", &self.remote.display().to_string()])
            .envs(GIT_ENV)
            .envs(extra.iter().copied())
            .env("GITVIEW_CONFIG_DIR", self.tmp.join("no-config"))
            .env("HERDR_APP_HOME", self.tmp.join("no-app"))
            .env("GITVIEW_HC", &self.hc)
            .env("SHIM_LOG", self.tmp.join("shim.log"))
            .env("SHIM_MACHINE", "box")
            .env("SHIM_FAKE", self.fake_prefix())
            .env("SHIM_REAL", self.remote_real.parent().unwrap())
            .env("SHIM_GIT", &self.git)
            .current_dir(self.tmp.join("empty"));
        let out = cmd.output().unwrap();
        (
            out.status.success(),
            serde_json::from_slice(&out.stdout).unwrap(),
        )
    }

    fn run(&self, args: &[&str], on_box: bool) -> Value {
        let (repo, root) = if on_box {
            (&self.remote, self.remote.display().to_string())
        } else {
            (&self.here, self.here.display().to_string())
        };
        let mut cmd = common::gitview();
        cmd.arg("json")
            .args(args)
            .args(["--repo", &repo.display().to_string()])
            .envs(GIT_ENV)
            .env("GITVIEW_CONFIG_DIR", self.tmp.join("no-config"))
            .env("HERDR_APP_HOME", self.tmp.join("no-app"))
            .current_dir(self.tmp.join("empty"));
        if on_box {
            cmd.args(["--on", "box"])
                .env("GITVIEW_HC", &self.hc)
                .env("SHIM_LOG", self.tmp.join("shim.log"))
                .env("SHIM_MACHINE", "box")
                .env("SHIM_FAKE", self.fake_prefix())
                .env("SHIM_REAL", self.remote_real.parent().unwrap())
                .env("SHIM_GIT", &self.git)
                .env("TRAP_LOG", self.tmp.join("trap.log"))
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        self.trap_dir.display(),
                        std::env::var("PATH").unwrap()
                    ),
                );
        } else {
            // The local run must not need hc at all.
            cmd.env("GITVIEW_HC", self.tmp.join("no-hc"));
        }
        let out = cmd.output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).replace(&root, "ROOT");
        let v: Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| panic!("json {args:?} (on box: {on_box}): {text}"));
        assert!(
            out.status.success(),
            "json {args:?} (on box: {on_box}): {v}"
        );
        v
    }
}

fn paths(status: &Value, section: &str) -> Vec<String> {
    status["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["id"] == section)
        .flat_map(|s| s["files"].as_array().unwrap().iter())
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn status_diff_stage_discard_and_commit_match_with_and_without_on() {
    let w = world("ops");

    let status = w.both(&["status"]);
    assert_eq!(status["root"], "ROOT");
    assert_eq!(paths(&status, "staged"), ["staged.txt"]);
    assert_eq!(
        paths(&status, "changes"),
        [
            "base.txt",
            "blob.bin",
            "dir/deep.txt",
            "gone.txt",
            "new.txt"
        ]
    );
    // Untracked stats were counted on the machine: a final line without a
    // newline counts, a binary has none.
    let file = |p: &str| {
        status["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["files"].as_array().unwrap().iter())
            .find(|f| f["path"] == p)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            file("new.txt")["add"].clone(),
            file("new.txt")["del"].clone()
        ),
        (2.into(), 0.into())
    );
    assert_eq!(file("blob.bin")["add"], Value::Null);

    let branch = w.both(&["status", "--scope", "branch"]);
    assert_eq!(branch["base"]["label"], "main");
    let merge_base = branch["base"]["merge_base"].as_str().unwrap().to_string();

    let diff = w.both(&["diff", "--path", "base.txt", "--kind", "modified"]);
    assert!(diff["rows"].as_array().unwrap().len() >= 2);
    w.both(&["diff", "--path", "new.txt", "--kind", "untracked"]);
    w.both(&[
        "diff",
        "--path",
        "staged.txt",
        "--kind",
        "added",
        "--cached",
    ]);
    w.both(&["diff", "--path", "gone.txt", "--kind", "deleted"]);
    w.both(&[
        "diff",
        "--path",
        "src/lib.rs",
        "--kind",
        "modified",
        "--merge-base",
        &merge_base,
        "--base",
        "main",
    ]);

    w.both(&["stage", "base.txt", "new.txt"]);
    let status = w.both(&["status"]);
    assert_eq!(
        paths(&status, "staged"),
        ["base.txt", "new.txt", "staged.txt"]
    );
    w.both(&["unstage", "new.txt"]);
    w.both(&["discard", "gone.txt"]);
    let status = w.both(&["status"]);
    assert!(!paths(&status, "changes").contains(&"gone.txt".to_string()));

    let commit = w.both(&["commit", "remote commit"]);
    assert_eq!(commit["all"], false, "staged changes only");
    assert!(
        commit["summary"]
            .as_str()
            .unwrap()
            .ends_with(" remote commit"),
        "{commit}"
    );
    let status = w.both(&["status"]);
    assert!(paths(&status, "staged").is_empty());
    let commit = w.both(&["commit", "everything"]);
    assert_eq!(commit["all"], true, "nothing staged: everything");
    assert!(commit["summary"].as_str().unwrap().ends_with(" everything"));
    assert!(
        w.both(&["status"])["sections"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // History, a commit's files and its diff.
    let log = w.both(&["log"]);
    let commits = log["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 4);
    let sha = commits[0]["sha"].as_str().unwrap().to_string();
    let mine = w.both(&["log", "--merge-base", &merge_base]);
    assert_eq!(mine["commits"].as_array().unwrap().len(), 3);
    let files = w.both(&["commit-files", "--sha", &sha]);
    assert!(!files["files"].as_array().unwrap().is_empty());
    w.both(&[
        "diff",
        "--path",
        "dir/deep.txt",
        "--kind",
        "added",
        "--commit",
        &sha,
    ]);

    // Folders and many paths.
    for (path, body) in [
        ("dir/deep.txt", "changed\n"),
        ("dir/more.txt", "more\n"),
        ("base.txt", "x\n"),
    ] {
        write(&w.here, path, body.as_bytes());
        write(&w.remote_real, path, body.as_bytes());
    }
    assert_eq!(w.both(&["stage", "dir/"])["count"], 2);
    assert_eq!(w.both(&["discard", "dir", "base.txt"])["count"], 3);
    assert!(
        w.both(&["status"])["sections"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The remote repo really changed, and only through the shim.
    assert!(!w.remote.exists(), "the remote path must not exist here");
    assert_eq!(w.trap_log(), "", "a local git ran");
    let log = w.shim_log();
    assert!(log.len() > 20, "{log:?}");
    let prefix = format!("exec --on box --cwd {}", w.remote.display());
    for call in &log {
        assert!(call.starts_with(&prefix), "unexpected hc call: {call}");
    }
    assert!(
        log.iter()
            .any(|c| c.contains("-i -- git") && c.contains("cat-file --batch"))
    );
    assert!(
        log.iter()
            .any(|c| c.contains("--timeout 600 -- git") && c.contains("commit -m"))
    );
    let head = |dir: &Path| {
        String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["log", "--format=%H %s"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };
    assert_eq!(head(&w.here), head(&w.remote_real));
    assert!(
        head(&w.remote_real)
            .lines()
            .next()
            .unwrap()
            .ends_with(" everything")
    );
}

#[test]
fn nvim_on_a_machine_never_reaches_for_a_local_nvim() {
    let w = world("nvim");
    let reply = w.run(
        &["nvim", "--path", "base.txt", "--line", "2", "--tab", "tb1"],
        true,
    );
    assert_eq!(reply, serde_json::json!({ "terminal": null }));
    assert_eq!(w.trap_log(), "");
}

#[test]
fn an_unknown_machine_is_an_error_not_a_local_fallback() {
    let w = world("unknown");
    let out = common::gitview()
        .args(["json", "status", "--on", "elsewhere", "--repo"])
        .arg(&w.remote)
        .env("GITVIEW_HC", &w.hc)
        .env("SHIM_LOG", w.tmp.join("shim.log"))
        .env("SHIM_MACHINE", "box")
        .env("SHIM_FAKE", w.fake_prefix())
        .env("SHIM_REAL", w.remote_real.parent().unwrap())
        .env("SHIM_GIT", &w.git)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["error"].as_str().unwrap().contains("on elsewhere"), "{v}");
}

#[test]
fn untracked_files_are_counted_alike_in_one_call_per_status() {
    let w = world("untracked");
    let big: String = "x\n".repeat(600 * 1024);
    for dir in [&w.here, &w.remote_real] {
        write(dir, "empty.txt", b"");
        write(dir, "big.txt", big.as_bytes());
        write(dir, "a name: with spaces.txt", b"1\n2\n3");
        write(dir, "-dash.txt", b"d\n");
    }
    let calls_before = w.shim_log().len();
    let status = w.both(&["status"]);
    let measuring: Vec<String> = w.shim_log()[calls_before..]
        .iter()
        .filter(|c| c.contains("wc -c"))
        .cloned()
        .collect();
    assert_eq!(measuring.len(), 1, "one call measures all: {measuring:?}");
    let add = |p: &str| {
        status["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["files"].as_array().unwrap().iter())
            .find(|f| f["path"] == p)
            .unwrap_or_else(|| panic!("{p} not listed"))["add"]
            .clone()
    };
    assert_eq!(add("empty.txt"), 0);
    assert_eq!(add("big.txt"), Value::Null, "over 1 MiB: not counted");
    assert_eq!(add("a name: with spaces.txt"), 3);
    assert_eq!(add("-dash.txt"), 1);
    assert_eq!(add("blob.bin"), Value::Null);
    assert_eq!(w.trap_log(), "");
}

#[test]
fn a_failed_remote_read_is_an_error_not_a_deleted_file() {
    let w = world("read-fails");
    let mut cmd = common::gitview();
    cmd.args(["json", "diff", "--path", "base.txt", "--kind", "modified"]);
    let (ok, reply) = w.run_with(cmd, &[("SHIM_FAIL_READS", "1")]);
    assert!(!ok, "a diff against nothing: {reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("the connection dropped"),
        "{reply}"
    );
}

#[test]
fn branch_scope_on_a_machine_resolves_the_base_again_once_head_moved_there() {
    let w = world("head-moved");
    let first = w.run(&["status", "--scope", "branch"], true);
    let head = first["head"].as_str().unwrap().to_string();
    let merge_base = first["base"]["merge_base"].as_str().unwrap().to_string();
    let poll = |head: &str| {
        w.run(
            &[
                "status",
                "--scope",
                "branch",
                "--base",
                "main",
                "--merge-base",
                &merge_base,
                "--head",
                head,
            ],
            true,
        )
    };

    // HEAD where it was: the base passed back is used, nothing detected.
    let calls_before = w.shim_log().len();
    assert_eq!(poll(&head)["base"]["merge_base"], merge_base.as_str());
    let detecting: Vec<String> = w.shim_log()[calls_before..]
        .iter()
        .filter(|c| c.contains(" merge-base "))
        .cloned()
        .collect();
    assert!(detecting.is_empty(), "{detecting:?}");

    // On the machine, main moves on and feature merges it.
    let box_repo = &w.remote_real;
    git(box_repo, &["add", "-A"]);
    git(box_repo, &["commit", "-q", "-m", "wip"]);
    git(box_repo, &["checkout", "-q", "main"]);
    write(box_repo, "theirs.txt", b"main moved on\n");
    git(box_repo, &["add", "."]);
    git(box_repo, &["commit", "-q", "-m", "main work"]);
    git(box_repo, &["checkout", "-q", "feature"]);
    git(box_repo, &["merge", "-q", "--no-edit", "main"]);
    let main_tip = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(box_repo)
            .args(["rev-parse", "main"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();

    let moved = poll(&head);
    assert_eq!(
        moved["base"]["merge_base"],
        main_tip.trim(),
        "resolved again"
    );
    assert!(
        !paths(&moved, "branch").contains(&"theirs.txt".to_string()),
        "main's own file isn't this branch's change: {moved}"
    );
    assert_ne!(moved["head"], head.as_str());
    assert_eq!(w.trap_log(), "");
}

#[test]
fn a_file_for_the_preview_arrives_from_the_machine_byte_for_byte() {
    let w = world("file");
    // Every byte value, plus what a text-mode or lossy read would change.
    let mut before: Vec<u8> = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    before.extend(0..=255u8);
    before.extend(b"\r\n\xff\xfe\xc3(");
    let after: Vec<u8> = before.iter().rev().copied().collect();
    for dir in [&w.here, &w.remote_real] {
        write(dir, "logo.png", &before);
        write(dir, "notes.md", "# notes ✓\r\n".as_bytes());
        git(dir, &["add", "logo.png", "notes.md"]);
        git(dir, &["commit", "-q", "-m", "logo"]);
        write(dir, "logo.png", &after);
    }
    for (side, bytes) in [("worktree", &after), ("head", &before)] {
        let mut replies = Vec::new();
        for on_box in [false, true] {
            let out = w.tmp.join(format!("cache-{side}-{on_box}"));
            let reply = w.run(
                &[
                    "file",
                    "--path",
                    "logo.png",
                    "--side",
                    side,
                    "--out",
                    out.to_str().unwrap(),
                ],
                on_box,
            );
            assert_eq!(reply["kind"], "image", "{reply}");
            let path = PathBuf::from(reply["path"].as_str().unwrap());
            assert_eq!(path.parent().unwrap(), out);
            assert_eq!(
                &std::fs::read(&path).unwrap(),
                bytes,
                "{side} (on box: {on_box}) byte for byte"
            );
            replies.push((path.file_name().unwrap().to_owned(), reply["bytes"].clone()));
        }
        assert_eq!(replies[0], replies[1], "{side}: here vs on box");
    }

    // The other kinds come out the same as here.
    let text = w.both(&["file", "--path", "notes.md", "--side", "head"]);
    assert_eq!(text["text"], "# notes ✓\r\n");
    assert_eq!(w.both(&["file", "--path", "blob.bin"])["kind"], "binary");
    let big = w.both(&["file", "--path", "base.txt", "--max-bytes", "4"]);
    assert_eq!(
        (&big["kind"], &big["bytes"]),
        (&"too_big".into(), &14.into())
    );
    assert_eq!(
        w.both(&[
            "file",
            "--path",
            "base.txt",
            "--side",
            "index",
            "--max-bytes",
            "4"
        ])["kind"],
        "too_big"
    );
    assert_eq!(w.both(&["file", "--path", "gone.txt"])["kind"], "missing");
    assert_eq!(
        w.both(&["file", "--path", "new.txt", "--side", "head"])["kind"],
        "missing"
    );
    assert_eq!(w.both(&["file", "--path", "dir"])["kind"], "missing");

    // A read that fails there is an error, not a missing file.
    let mut cmd = common::gitview();
    cmd.args(["json", "file", "--path", "notes.md"]);
    let (ok, reply) = w.run_with(cmd, &[("SHIM_FAIL_READS", "1")]);
    assert!(!ok, "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("the connection dropped"),
        "{reply}"
    );

    assert!(!w.remote.exists());
    assert_eq!(w.trap_log(), "", "a local git ran");
    let prefix = format!("exec --on box --cwd {}", w.remote.display());
    for call in w.shim_log() {
        assert!(call.starts_with(&prefix), "unexpected hc call: {call}");
    }
}

#[test]
fn the_editor_is_picked_from_the_machines_login_shell() {
    let w = world("editor");
    // This machine has a vim and a $VISUAL of its own; neither counts there.
    let here_bin = w.tmp.join("here-bin");
    let there_bin = w.remote_real.parent().unwrap().join("bin");
    for (dir, name) in [
        (&here_bin, "vim"),
        (&here_bin, "here-ed"),
        (&there_bin, "vi"),
        (&there_bin, "hx"),
    ] {
        std::fs::create_dir_all(dir).unwrap();
        write(dir, name, b"#!/bin/sh\nexit 0\n");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let pick = |login: &[(&str, &str)]| {
        let mut cmd = common::gitview();
        cmd.args(["json", "editor"])
            .env(
                "PATH",
                format!("{}:{}", here_bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("VISUAL", "here-ed");
        let mut extra = vec![("SHIM_LOGIN_PATH", there_bin.to_str().unwrap())];
        extra.extend(login.iter().copied());
        let (ok, reply) = w.run_with(cmd, &extra);
        assert!(ok, "{reply}");
        reply
    };
    let there = |name: &str| there_bin.join(name).display().to_string();
    assert_eq!(
        pick(&[]),
        serde_json::json!({"editor": {"argv": [there("vi")], "kind": "vim", "name": "vi"}})
    );
    assert_eq!(
        pick(&[("SHIM_EDITOR", "hx --vsplit")])["editor"],
        serde_json::json!({"argv": [there("hx"), "--vsplit"], "kind": "other", "name": "hx"})
    );
    // Nothing there: an error, never this machine's opener.
    std::fs::remove_file(there_bin.join("vi")).unwrap();
    let none = pick(&[("SHIM_VISUAL", "zed")]);
    assert_eq!(none["editor"], Value::Null);
    assert!(
        none["error"]
            .as_str()
            .unwrap()
            .starts_with("No editor found"),
        "{none}"
    );
    let prefix = format!("exec --on box --cwd {}", w.remote.display());
    for call in w.shim_log() {
        assert!(call.starts_with(&prefix), "unexpected hc call: {call}");
    }
}

#[test]
fn the_reload_hints_come_from_the_machine_alike() {
    let w = world("hints");
    for repo in [&w.here, &w.remote_real] {
        write(repo, ".gitignore", b"target/\n");
        write(repo, "target/debug/x.o", b"o\n");
    }
    let calls_before = w.shim_log().len();
    let status = w.both(&["status"]);
    let hashing = w.shim_log()[calls_before..]
        .iter()
        .filter(|c| c.contains("hash-object"))
        .count();
    assert_eq!(hashing, 1, "one call for every file's rev");
    assert_eq!(status["ignored_dirs"], serde_json::json!(["target/"]));
    let rev = |s: &Value| {
        s["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["files"].as_array().unwrap())
            .find(|f| f["path"] == "base.txt")
            .map(|f| f["rev"].clone())
            .unwrap()
    };
    let diff = ["diff", "--path", "base.txt", "--kind", "modified"];
    let hash = w.both(&diff)["hash"].as_str().unwrap().to_string();
    let unless = [&diff[..], &["--unless", &hash]].concat();
    assert_eq!(
        w.run(&unless, true),
        serde_json::json!({"same": true, "hash": hash})
    );

    // A same-size edit there (not here) moves its rev and its diff there.
    let there = std::fs::read(w.remote_real.join("base.txt")).unwrap();
    let mut edited = there.clone();
    let last = edited
        .iter()
        .rposition(|b| b.is_ascii_alphabetic())
        .unwrap();
    edited[last] ^= 0x20; // the case of one letter
    std::fs::write(w.remote_real.join("base.txt"), &edited).unwrap();
    assert_ne!(rev(&w.run(&["status"], true)), rev(&status));
    assert_eq!(rev(&w.run(&["status"], false)), rev(&status));
    assert_ne!(w.run(&unless, true).get("same"), Some(&Value::Bool(true)));
}

#[test]
fn a_diff_whose_reads_fail_there_is_an_error_not_a_wrong_diff() {
    let w = world("readfail");
    let fail = [("SHIM_FAIL_GIT_READS", "1")];
    for args in [
        &["diff", "--path", "base.txt", "--kind", "modified"][..],
        &[
            "diff",
            "--path",
            "staged.txt",
            "--kind",
            "added",
            "--cached",
        ],
        &[
            "diff", "--path", "base.txt", "--kind", "modified", "--commit", "HEAD",
        ],
    ] {
        let mut cmd = common::gitview();
        cmd.arg("json").args(args);
        let (ok, reply) = w.run_with(cmd, &fail);
        assert!(!ok, "{args:?}: {reply}");
        let err = reply["error"].as_str().unwrap_or_default();
        assert!(err.contains("unreachable"), "{args:?}: {reply}");
    }
}

#[test]
fn a_root_status_gave_is_not_looked_up_again_there() {
    let w = world("root");
    let diff = ["diff", "--path", "base.txt", "--kind", "modified"];
    let calls = |args: &[&str]| {
        let before = w.shim_log().len();
        let reply = w.run(args, true);
        let lookups = w.shim_log()[before..]
            .iter()
            .filter(|c| c.contains("rev-parse --show-toplevel"))
            .count();
        (reply, lookups)
    };
    let (looked_up, n) = calls(&diff);
    assert_eq!(n, 1);
    let (trusted, n) = calls(&[&diff[..], &["--root"]].concat());
    assert_eq!(n, 0, "--root: no lookup");
    assert_eq!(trusted, looked_up);
}
