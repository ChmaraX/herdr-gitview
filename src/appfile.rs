//! `herdr-gitview json file`: one side of a file, for the diff pane's rich
//! preview (rendered markdown, images). The op's contract is in
//! [`crate::appjson`]'s module doc; this is how it's read and classified.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::git::Repo;
use crate::remote::Contents;

/// Read at most this much of a file that isn't an image (the preview
/// renders it whole).
pub const TEXT_MAX: u64 = 1024 * 1024;

/// Read at most this much of an image.
pub const IMAGE_MAX: u64 = 16 * 1024 * 1024;

/// Extensions shown as images (lowercased); svg included.
const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"];

/// Which version of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Worktree,
    Index,
    Head,
    /// That commit's version.
    Commit(String),
    /// The version before that commit (its first parent's).
    Parent(String),
    /// The branch's merge-base's version.
    Base(String),
}

impl Side {
    /// `worktree|index|head|commit:<sha>|parent:<sha>|base:<sha>`.
    pub fn parse(raw: &str) -> Result<Side> {
        let sha = |s: &str| -> Result<String> {
            if (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(s.to_string())
            } else {
                bail!("--side {raw}: not a commit id")
            }
        };
        Ok(match raw.split_once(':') {
            None if raw == "worktree" => Side::Worktree,
            None if raw == "index" => Side::Index,
            None if raw == "head" => Side::Head,
            Some(("commit", s)) => Side::Commit(sha(s)?),
            Some(("parent", s)) => Side::Parent(sha(s)?),
            Some(("base", s)) => Side::Base(sha(s)?),
            _ => bail!(
                "--side {raw}: worktree, index, head, commit:<sha>, parent:<sha> or base:<sha>"
            ),
        })
    }

    /// The git object spec for `path` on this side; `None` for the worktree.
    fn spec(&self, path: &str) -> Option<String> {
        Some(match self {
            Side::Worktree => return None,
            Side::Index => format!(":{path}"),
            Side::Head => format!("HEAD:{path}"),
            Side::Commit(sha) | Side::Base(sha) => format!("{sha}:{path}"),
            Side::Parent(sha) => format!("{sha}^:{path}"),
        })
    }
}

/// The reply.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileOut {
    Text {
        text: String,
        bytes: u64,
    },
    Image {
        path: PathBuf,
        bytes: u64,
        ext: String,
    },
    Binary {
        bytes: u64,
    },
    TooBig {
        bytes: u64,
    },
    Missing,
}

/// The reply with its `hash` (of the kind and the bytes read, so the same
/// hash is the same reply), or, when that's the `--unless` hash, just
/// `{same: true, hash}`.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Reply {
    Same {
        same: bool,
        hash: String,
    },
    File {
        #[serde(flatten)]
        file: FileOut,
        hash: String,
    },
}

/// An image's extension (lowercased), when `path` names one.
fn image_ext(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    IMAGES.contains(&ext.as_str()).then_some(ext)
}

/// `path` as given, when it stays inside the repo: relative, no `..`.
fn inside(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw.strip_prefix("./").unwrap_or(raw));
    if raw.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        bail!("--path {raw}: a path inside the repo");
    }
    Ok(path)
}

/// Read `raw` on `side` of `repo` and say what it is. An image is written
/// to `out` (required then) as `<sha256>.<ext>`, unless that file is there
/// already. `max`: the most bytes read (default [`TEXT_MAX`], or
/// [`IMAGE_MAX`] for an image). `unless`: the hash of a reply the caller
/// has; when this one's is the same, [`Reply::Same`] (nothing written).
pub fn file(
    repo: &Repo,
    raw: &str,
    side: &Side,
    out: Option<&Path>,
    max: Option<u64>,
    unless: Option<&str>,
) -> Result<Reply> {
    let path = inside(raw)?;
    let ext = image_ext(&path);
    let max = max.unwrap_or(if ext.is_some() { IMAGE_MAX } else { TEXT_MAX });
    let contents = match side.spec(&path.to_string_lossy()) {
        None => repo.machine.read_at_most(&repo.root, &path, max)?,
        Some(spec) => repo.read_object(&spec, max)?,
    };
    let bytes = match contents {
        Contents::Missing => return Ok(reply(FileOut::Missing, &[b"missing"], unless)),
        Contents::TooBig(n) => {
            return Ok(reply(
                FileOut::TooBig { bytes: n },
                &[b"too_big", &n.to_le_bytes()],
                unless,
            ));
        }
        Contents::Bytes(bytes) => bytes,
    };
    let hash = digest(&[b"bytes", ext.as_deref().unwrap_or("").as_bytes(), &bytes]);
    if unless == Some(hash.as_str()) {
        return Ok(Reply::Same { same: true, hash });
    }
    let size = bytes.len() as u64;
    let file = if let Some(ext) = ext {
        let out = out.context("file: --out DIR, where an image is written")?;
        let path = cache(out, &bytes, &ext)?;
        FileOut::Image {
            path,
            bytes: size,
            ext,
        }
    } else if bytes.contains(&0) {
        FileOut::Binary { bytes: size }
    } else {
        match String::from_utf8(bytes) {
            Ok(text) => FileOut::Text { text, bytes: size },
            Err(_) => FileOut::Binary { bytes: size },
        }
    };
    Ok(Reply::File { file, hash })
}

/// `file` with the hash of `parts`, or [`Reply::Same`] when that's `unless`.
fn reply(file: FileOut, parts: &[&[u8]], unless: Option<&str>) -> Reply {
    let hash = digest(parts);
    if unless == Some(hash.as_str()) {
        Reply::Same { same: true, hash }
    } else {
        Reply::File { file, hash }
    }
}

/// A hash of `parts` (each length-prefixed): 32 hex digits.
pub fn digest(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `<out>/<sha256 of bytes>.<ext>`, written (through a temp file and a
/// rename, so a reader never sees half of it) only when it isn't there.
fn cache(out: &Path, bytes: &[u8], ext: &str) -> Result<PathBuf> {
    let name = format!("{:x}.{ext}", Sha256::digest(bytes));
    let path = out.join(&name);
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
    let tmp = out.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_side_is_one_of_the_six_and_a_sha_is_only_hex() {
        for good in [
            "worktree",
            "index",
            "head",
            "commit:abc123",
            "parent:abc123",
            "base:abc123",
        ] {
            assert!(Side::parse(good).is_ok(), "{good}");
        }
        // A rev expression could read any object: only a commit id passes.
        for bad in [
            "commit:HEAD~1",
            "commit:",
            "base:ab",
            "tree",
            "commit:a;b",
            "parent:main",
        ] {
            assert!(Side::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_path_must_stay_inside_the_repo() {
        assert_eq!(inside("./docs/a.md").unwrap(), PathBuf::from("docs/a.md"));
        for bad in ["", "../x", "a/../../x", "/etc/passwd"] {
            assert!(inside(bad).is_err(), "{bad}");
        }
    }
}
