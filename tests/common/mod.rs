//! Shared fixtures for the integration tests: throwaway git repos and a fake
//! `herdr` binary that records its invocations and answers with canned JSON.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A throwaway git repo, removed on drop.
pub struct TempRepo {
    pub dir: PathBuf,
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// A repo with one committed file (`base.txt` = "one\ntwo\n") on `main`.
pub fn fixture(name: &str) -> TempRepo {
    let dir = std::env::temp_dir().join(format!(
        "gitview-scenario-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "base.txt", "one\ntwo\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    let dir = dir.canonicalize().unwrap(); // macOS /tmp symlink
    TempRepo { dir }
}

/// A fake `herdr` executable: logs every invocation (one line of argv per
/// call) to `<dir>/herdr.log`, answers `--version` with 0.7.4, `plugin pane
/// open` with a canned pane object, and `pane get` with success unless
/// `<dir>/popup-dead` exists.
pub struct FakeHerdr {
    pub bin: PathBuf,
    pub log: PathBuf,
    pub dead_marker: PathBuf,
}

impl FakeHerdr {
    pub fn install(dir: &Path) -> FakeHerdr {
        let bin = dir.join("herdr");
        let log = dir.join("herdr.log");
        let dead_marker = dir.join("popup-dead");
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> "{log}"
case "$1" in
  --version)
    echo "herdr 0.7.4" ;;
  plugin)
    # plugin pane open / focus
    echo '{{"id":"cli:plugin","result":{{"plugin_pane":{{"pane":{{"pane_id":"wT:pPOP","tab_id":"wT:t1"}}}}}}}}' ;;
  pane)
    if [ "$2" = "get" ] && [ -f "{dead}" ]; then
      echo '{{"error":{{"code":"pane_not_found","message":"gone"}}}}'
      exit 1
    fi
    echo '{{"id":"cli:pane","result":{{}}}}' ;;
  *)
    echo '{{"id":"cli","result":{{}}}}' ;;
esac
"#,
            log = log.display(),
            dead = dead_marker.display(),
        );
        std::fs::write(&bin, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeHerdr {
            bin,
            log,
            dead_marker,
        }
    }

    /// Every recorded invocation, one argv-line per call.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Mark the popup pane as dead (`pane get` starts failing).
    pub fn kill_popup(&self) {
        std::fs::write(&self.dead_marker, "").unwrap();
    }
}
