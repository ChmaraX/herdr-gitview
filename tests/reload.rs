//! What the mods use to load less: `json diff`/`json file` with `--unless
//! <hash>`, the per-file `rev` in `json status`, and its `ignored_dirs`.
//! Real binary, real repos; the same through `hc exec` is in
//! `tests/remote.rs`.
//!
//! Failure modes this catches:
//! - `{same}` while the rows would differ: a same-size edit, a staged edit
//!   asked for with `--cached`, the same text under another name (another
//!   syntax), another theme, `--unfold`, another `context_lines`; or the
//!   staged view moved by a worktree edit;
//! - never `{same}` for what didn't change (a hash with a time or a
//!   process id in it), or rows sent along with `same`;
//! - `json file`'s `{same}` across a change of kind (a file deleted, an
//!   empty file made), or an image written again when it's the same;
//! - a `rev` that stays put when the index, HEAD or the worktree file
//!   changed (a same-size edit, staging, a commit, an untracked file
//!   edited), or moves for a file nobody touched (another file's edit);
//! - `ignored_dirs` missing an ignored folder (nested, or with untracked
//!   files hidden, or in branch scope), or listing what isn't one (an
//!   ignored file, a folder inside an ignored one, an untracked folder).

mod common;

use std::path::Path;

use common::{fixture, git, write};
use serde_json::Value;

fn gv(repo: &Path, args: &[&str]) -> Value {
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
    assert!(out.status.success(), "json {args:?}: {v}");
    v
}

fn hash(v: &Value) -> String {
    let h = v["hash"]
        .as_str()
        .unwrap_or_else(|| panic!("no hash in {v}"));
    assert_eq!(h.len(), 32, "{v}");
    h.to_string()
}

/// `json <args> --unless <h>`: whether it answered `{same}` (and nothing
/// else), checking a full reply's hash is new.
fn same(repo: &Path, args: &[&str], h: &str) -> bool {
    let mut with = args.to_vec();
    with.extend(["--unless", h]);
    let v = gv(repo, &with);
    if v.get("same") == Some(&Value::Bool(true)) {
        assert_eq!(v.as_object().unwrap().len(), 2, "only same and hash: {v}");
        assert_eq!(v["hash"], h);
        true
    } else {
        assert_ne!(hash(&v), h, "a full reply with the hash asked about: {v}");
        false
    }
}

#[test]
fn a_diff_is_same_only_while_its_rows_would_be() {
    let repo = fixture("unless-diff");
    let d = &repo.dir;
    write(d, "a.rs", "fn main() {}\nlet x = 1;\n");
    write(d, "a.md", "fn main() {}\nlet x = 1;\n");
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "two"]);
    write(d, "a.rs", "fn main() {}\nlet x = 2;\n");
    write(d, "a.md", "fn main() {}\nlet x = 2;\n");
    let diff = ["diff", "--path", "a.rs", "--kind", "modified"];
    let first = gv(d, &diff);
    let h = hash(&first);
    assert!(first["rows"].as_array().unwrap().len() > 1, "{first}");
    assert!(same(d, &diff, &h), "nothing changed");
    assert_eq!(hash(&gv(d, &diff)), h, "the same hash every time");

    // The same text under another name is another syntax.
    assert!(!same(
        d,
        &["diff", "--path", "a.md", "--kind", "modified"],
        &h
    ));
    // Another theme, unfolded, other context lines.
    assert!(!same(d, &[&diff[..], &["--theme", "light"]].concat(), &h));
    write(
        d,
        "a.rs",
        "fn main() {}\nlet x = 2;\n// 3\n// 4\n// 5\n// 6\n// 7\nend\n",
    );
    git(d, &["commit", "-q", "-am", "long"]);
    write(
        d,
        "a.rs",
        "fn main() {}\nlet x = 3;\n// 3\n// 4\n// 5\n// 6\n// 7\nend\n",
    );
    let h = hash(&gv(d, &diff));
    assert!(!same(d, &[&diff[..], &["--unfold"]].concat(), &h));
    write(d, ".git/gitview-config/config.toml", "context_lines = 0\n");
    assert!(!same(d, &diff, &h));
    std::fs::remove_file(d.join(".git/gitview-config/config.toml")).unwrap();
    assert!(same(d, &diff, &h));

    // A same-size edit.
    write(
        d,
        "a.rs",
        "fn main() {}\nlet x = 4;\n// 3\n// 4\n// 5\n// 6\n// 7\nend\n",
    );
    assert!(!same(d, &diff, &h));
    // The staged view: the worktree doesn't count, a staged edit does.
    let cached = ["diff", "--path", "a.rs", "--kind", "modified", "--cached"];
    git(d, &["add", "a.rs"]);
    let hc = hash(&gv(d, &cached));
    assert!(same(d, &cached, &hc));
    write(
        d,
        "a.rs",
        "fn main() {}\nlet x = 5;\n// 3\n// 4\n// 5\n// 6\n// 7\nend\n",
    );
    assert!(
        same(d, &cached, &hc),
        "the worktree isn't in the staged view"
    );
    git(d, &["add", "a.rs"]);
    assert!(!same(d, &cached, &hc));
}

#[test]
fn a_file_is_same_only_while_its_reply_would_be() {
    let repo = fixture("unless-file");
    let d = &repo.dir;
    let out = d.join(".git/preview-cache");
    let o = out.to_str().unwrap();
    write(d, "doc.md", "# one\n");
    let text = ["file", "--path", "doc.md"];
    let h = hash(&gv(d, &text));
    assert!(same(d, &text, &h));
    write(d, "doc.md", "# two\n");
    assert!(!same(d, &text, &h));
    // Gone, then empty: two other replies, each the same as itself.
    std::fs::remove_file(d.join("doc.md")).unwrap();
    let gone = gv(d, &text);
    assert_eq!(gone["kind"], "missing");
    assert!(same(d, &text, &hash(&gone)));
    write(d, "doc.md", "");
    assert!(!same(d, &text, &hash(&gone)));
    // An image the same is not written again; the same bytes under another
    // kind of name are another reply.
    write(d, "pic.png", "\u{89}PNG fake");
    let png = ["file", "--path", "pic.png", "--out", o];
    let first = gv(d, &png);
    let cached = Path::new(first["path"].as_str().unwrap()).to_path_buf();
    std::fs::remove_file(&cached).unwrap();
    assert!(same(d, &png, &hash(&first)));
    assert!(!cached.exists(), "written again for a same reply");
    write(d, "pic.txt", "\u{89}PNG fake");
    assert!(!same(d, &["file", "--path", "pic.txt"], &hash(&first)));
}

/// `path → rev` over every section of a status reply.
fn revs(status: &Value) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for s in status["sections"].as_array().unwrap() {
        for f in s["files"].as_array().unwrap() {
            let rev = f["rev"].as_str().unwrap_or_else(|| panic!("no rev: {f}"));
            let path = f["path"].as_str().unwrap().to_string();
            if let Some(was) = out.insert(path, rev.to_string()) {
                assert_eq!(was, rev, "one rev per file across sections: {f}");
            }
        }
    }
    out
}

#[test]
fn a_rev_moves_when_its_file_does_and_only_then() {
    let repo = fixture("revs");
    let d = &repo.dir;
    write(d, "other.txt", "x\n");
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "other"]);
    write(d, "base.txt", "one\nTWO\n");
    write(d, "other.txt", "y\n");
    write(d, "new.txt", "new\n");
    let rev = |f: &str| revs(&gv(d, &["status"]))[f].clone();
    let r0 = revs(&gv(d, &["status"]));
    assert_eq!(revs(&gv(d, &["status"])), r0, "nothing changed");

    // A same-size edit, at once (the same second).
    write(d, "base.txt", "one\nTwO\n");
    let r1 = revs(&gv(d, &["status"]));
    assert_ne!(r1["base.txt"], r0["base.txt"]);
    assert_eq!(r1["other.txt"], r0["other.txt"], "another file's edit");
    assert_eq!(r1["new.txt"], r0["new.txt"]);
    // An untracked file edited.
    write(d, "new.txt", "neW\n");
    assert_ne!(rev("new.txt"), r1["new.txt"]);

    // Staged (worktree then the same as the index), then a commit of the
    // file staged with a worktree edit on top.
    let before = rev("base.txt");
    git(d, &["add", "base.txt"]);
    let staged = rev("base.txt");
    assert_ne!(staged, before);
    write(d, "base.txt", "one\nTwO\nthree\n");
    let partly = rev("base.txt");
    assert_ne!(partly, staged);
    git(d, &["commit", "-q", "-m", "two"]);
    assert_ne!(rev("base.txt"), partly, "HEAD moved under it");

    // Branch scope: the worktree file's edits.
    let branch = |f: &str| revs(&gv(d, &["status", "--scope", "branch"]))[f].clone();
    let b0 = branch("base.txt");
    assert_eq!(branch("base.txt"), b0);
    write(d, "base.txt", "one\nTwO\nthreE\n");
    assert_ne!(branch("base.txt"), b0);
}

#[test]
fn ignored_dirs_are_the_outermost_ignored_folders() {
    let repo = fixture("ignored");
    let d = &repo.dir;
    write(d, ".gitignore", "target/\nnode_modules/\n*.log\n");
    write(d, "web/.gitignore", "dist/\n");
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "ignores"]);
    for f in [
        "target/debug/a.o",
        "target/node_modules/x.js",
        "web/node_modules/y.js",
        "web/dist/app.js",
        "web/src/app.ts",
        "build.log",
        "fresh/untracked.txt",
    ] {
        write(d, f, "x\n");
    }
    let dirs = |args: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = gv(d, args)["ignored_dirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    let want = ["target/", "web/dist/", "web/node_modules/"];
    assert_eq!(dirs(&["status"]), want);
    write(
        d,
        ".git/gitview-config/config.toml",
        "show_untracked = false\n",
    );
    assert_eq!(dirs(&["status"]), want, "untracked files hidden");
    assert_eq!(dirs(&["status", "--scope", "branch"]), want);
}
