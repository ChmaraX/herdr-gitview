//! `herdr-gitview json file`: one side of a file for the diff pane's rich
//! preview, run as the mod runs it (the real binary, a real repo). The same
//! op through `hc exec` is in `tests/remote.rs`.
//!
//! Failure modes this catches:
//! - the wrong side read (the worktree for a staged change, HEAD for a
//!   commit, the commit itself for its "before");
//! - an image not byte-identical in the cache (a lossy UTF-8 read, CRLF or
//!   NUL bytes touched), named other than by its content, or written again
//!   when it's already there (churn while the app reads it);
//! - a file classified wrongly: an image by its upper-case extension, an
//!   SVG taken as text, invalid UTF-8 or NUL bytes taken as text;
//! - a big file read whole instead of reported `too_big`, or cached anyway;
//! - a file absent on that side (deleted, untracked at HEAD, a folder, a
//!   root commit's parent) reported as an error or empty text, not
//!   `missing`;
//! - a path or side that reaches outside the file (`..`, a rev expression).

mod common;

use std::path::{Path, PathBuf};

use common::{fixture, write};
use serde_json::{Value, json};

/// Bytes a lossy or text-mode read would change: a PNG signature (with its
/// CRLF and ^Z), NULs, invalid UTF-8, a lone CR.
fn png_bytes(seed: u8) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend((0..=255u8).map(|i| i.wrapping_mul(seed)));
    b.extend(b"\xff\xfe\r\xc3(\0");
    b
}

fn json_file(repo: &Path, args: &[&str]) -> (bool, Value) {
    let out = common::gitview()
        .args(["json", "file", "--repo"])
        .arg(repo)
        .args(args)
        .env("GITVIEW_CONFIG_DIR", repo.join(".git/no-config"))
        .output()
        .expect("spawn herdr-gitview");
    let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    (out.status.success(), v)
}

fn file(repo: &Path, args: &[&str]) -> Value {
    let (ok, v) = json_file(repo, args);
    assert!(ok, "json file {args:?}: {v}");
    v
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn cache_dir(repo: &Path) -> PathBuf {
    let dir = repo.join(".git/preview-cache");
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn text_comes_from_the_side_asked_for() {
    let repo = fixture("file-sides");
    let dir = &repo.dir;
    write(dir, "doc.md", "# v1\n");
    common::git(dir, &["add", "."]);
    common::git(dir, &["commit", "-q", "-m", "v1"]);
    let v1 = git_out(dir, &["rev-parse", "HEAD"]);
    write(dir, "doc.md", "# v2 ✓\n");
    common::git(dir, &["commit", "-q", "-am", "v2"]);
    let v2 = git_out(dir, &["rev-parse", "HEAD"]);
    write(dir, "doc.md", "# staged\n");
    common::git(dir, &["add", "doc.md"]);
    write(dir, "doc.md", "# worktree\n");

    let text = |side: &str| file(dir, &["--path", "doc.md", "--side", side]);
    assert_eq!(
        file(dir, &["--path", "doc.md"]),
        json!({"kind": "text", "text": "# worktree\n", "bytes": 11})
    );
    assert_eq!(text("index")["text"], "# staged\n");
    assert_eq!(text("head")["text"], "# v2 ✓\n");
    assert_eq!(text("head")["bytes"], "# v2 ✓\n".len());
    assert_eq!(text(&format!("commit:{v1}"))["text"], "# v1\n");
    assert_eq!(text(&format!("parent:{v2}"))["text"], "# v1\n");
    assert_eq!(text(&format!("base:{v1}"))["text"], "# v1\n");
}

#[test]
fn an_image_is_cached_byte_for_byte_under_its_content_hash_once() {
    let repo = fixture("file-image");
    let dir = &repo.dir;
    let out = cache_dir(dir);
    let (before, after) = (png_bytes(7), png_bytes(13));
    std::fs::write(dir.join("Logo.PNG"), &before).unwrap();
    common::git(dir, &["add", "."]);
    common::git(dir, &["commit", "-q", "-m", "logo"]);
    std::fs::write(dir.join("Logo.PNG"), &after).unwrap();

    let o = out.to_str().unwrap();
    let new = file(dir, &["--path", "Logo.PNG", "--out", o]);
    let old = file(dir, &["--path", "Logo.PNG", "--side", "head", "--out", o]);
    for (reply, bytes) in [(&new, &after), (&old, &before)] {
        assert_eq!(reply["kind"], "image", "{reply}");
        assert_eq!(reply["ext"], "png");
        assert_eq!(reply["bytes"], bytes.len());
        let path = PathBuf::from(reply["path"].as_str().unwrap());
        assert_eq!(path.parent().unwrap(), out);
        assert_eq!(&std::fs::read(&path).unwrap(), bytes, "byte for byte");
        let name = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 64, "a sha256: {name}");
    }
    assert_ne!(new["path"], old["path"], "named by content");

    // Asked again: the same file, not written again.
    let path = PathBuf::from(new["path"].as_str().unwrap());
    std::fs::write(&path, b"left alone").unwrap();
    assert_eq!(
        file(dir, &["--path", "Logo.PNG", "--out", o])["path"],
        new["path"]
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"left alone");
    let left: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(left.is_empty(), "no temp files: {left:?}");

    // An SVG is an image too, though it's text; without --out it can't be.
    write(
        dir,
        "icon.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n",
    );
    let svg = file(dir, &["--path", "icon.svg", "--out", o]);
    assert_eq!(
        (svg["kind"].clone(), svg["ext"].clone()),
        (json!("image"), json!("svg"))
    );
    let (ok, err) = json_file(dir, &["--path", "icon.svg"]);
    assert!(
        !ok && err["error"].as_str().unwrap().contains("--out"),
        "{err}"
    );
}

#[test]
fn binary_too_big_and_missing_are_told_apart() {
    let repo = fixture("file-kinds");
    let dir = &repo.dir;
    let out = cache_dir(dir);
    let o = out.to_str().unwrap();
    std::fs::write(dir.join("blob.bin"), b"\x7fELF\0\x01").unwrap();
    std::fs::write(dir.join("latin1.txt"), b"caf\xe9\n").unwrap();
    assert_eq!(
        file(dir, &["--path", "blob.bin"]),
        json!({"kind": "binary", "bytes": 6})
    );
    assert_eq!(file(dir, &["--path", "latin1.txt"])["kind"], "binary");

    // Too big: on disk and in git, the size told, nothing cached.
    let big = "x".repeat(2000);
    write(dir, "big.md", &big);
    write(dir, "big.svg", &format!("<svg>{big}</svg>"));
    common::git(dir, &["add", "big.md"]);
    for side in ["worktree", "index"] {
        assert_eq!(
            file(
                dir,
                &["--path", "big.md", "--side", side, "--max-bytes", "100"]
            ),
            json!({"kind": "too_big", "bytes": 2000}),
            "{side}"
        );
    }
    assert_eq!(
        file(
            dir,
            &["--path", "big.svg", "--max-bytes", "100", "--out", o]
        )["kind"],
        "too_big"
    );
    assert!(!out.exists(), "nothing cached");
    // The default limit is 1 MiB for text.
    write(dir, "huge.md", &"y".repeat(1024 * 1024 + 1));
    assert_eq!(file(dir, &["--path", "huge.md"])["kind"], "too_big");

    // Missing: deleted from the worktree (its old version is at HEAD), not
    // at HEAD yet, a folder, a root commit's parent.
    std::fs::remove_file(dir.join("base.txt")).unwrap();
    assert_eq!(
        file(dir, &["--path", "base.txt"]),
        json!({"kind": "missing"})
    );
    assert_eq!(
        file(dir, &["--path", "base.txt", "--side", "head"])["text"],
        "one\ntwo\n"
    );
    assert_eq!(
        file(dir, &["--path", "blob.bin", "--side", "head"])["kind"],
        "missing"
    );
    assert_eq!(
        file(dir, &["--path", "blob.bin", "--side", "index"])["kind"],
        "missing"
    );
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    write(dir, "folder/x.md", "x\n");
    common::git(dir, &["add", "folder"]);
    assert_eq!(file(dir, &["--path", "folder"])["kind"], "missing");
    assert_eq!(
        file(dir, &["--path", "folder", "--side", "index"])["kind"],
        "missing"
    );
    let root = git_out(dir, &["rev-parse", "HEAD"]);
    assert_eq!(
        file(
            dir,
            &["--path", "base.txt", "--side", &format!("parent:{root}")]
        )["kind"],
        "missing"
    );
}

#[test]
fn a_path_or_side_outside_the_file_is_refused() {
    let repo = fixture("file-refused");
    let dir = &repo.dir;
    let outside = dir
        .parent()
        .unwrap()
        .join(format!("gitview-file-outside-{}.md", std::process::id()));
    std::fs::write(&outside, "secret\n").unwrap();
    let rel = format!("../{}", outside.file_name().unwrap().to_str().unwrap());
    for args in [
        vec!["--path", rel.as_str()],
        vec!["--path", outside.to_str().unwrap()],
        vec!["--path", "base.txt", "--side", "commit:HEAD~1"],
        vec!["--path", "base.txt", "--side", "tree"],
    ] {
        let (ok, v) = json_file(dir, &args);
        assert!(!ok, "{args:?} went ahead: {v}");
        assert!(!v.to_string().contains("secret"), "{v}");
    }
    let _ = std::fs::remove_file(&outside);
}
