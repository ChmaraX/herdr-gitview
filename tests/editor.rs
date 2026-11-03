//! `herdr-gitview json editor`: which editor Enter starts for a new file
//! view, and `json nvim` asking herdr-nvim only where nvim is. Run as the
//! mod runs them, with a private `PATH` of fake programs and nothing of the
//! person's environment (no `VISUAL`/`EDITOR` of theirs). The same through
//! `hc exec` is in `tests/remote.rs`.
//!
//! Failure modes this catches:
//! - a program counted as there when it isn't (a file that isn't
//!   executable, a name not on `PATH`), or a later choice winning over an
//!   earlier one (`$EDITOR` over `$VISUAL`, the default `nvim` over
//!   `$EDITOR`, `$VISUAL` over the config's own `editor`);
//! - `$VISUAL`/`$EDITOR` split on every blank (a path with a space, quotes);
//! - `+LINE` offered to an editor that doesn't take it (kind wrong);
//! - nothing there and no answer the mod can show: the system opener here,
//!   the error naming config.toml otherwise;
//! - herdr-nvim asked to open a file where there's no nvim to run.

mod common;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

struct Box_ {
    dir: PathBuf,
    bin: PathBuf,
}

impl Drop for Box_ {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn scratch(name: &str) -> Box_ {
    let dir = std::env::temp_dir().join(format!("gitview-editor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::create_dir_all(dir.join("cfg")).unwrap();
    let dir = dir.canonicalize().unwrap();
    Box_ {
        bin: dir.join("bin"),
        dir,
    }
}

fn program(path: &Path, executable: bool) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// `json editor` with only `env` (and `PATH` = the box's bin).
fn pick(b: &Box_, env: &[(&str, &str)]) -> Value {
    let out = common::gitview()
        .args(["json", "editor", "--repo"])
        .arg(&b.dir)
        .env_clear()
        .env("PATH", &b.bin)
        .env("HOME", &b.dir)
        .env("GITVIEW_CONFIG_DIR", b.dir.join("cfg"))
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    assert!(out.status.success(), "{v}");
    v
}

fn at(b: &Box_, name: &str) -> String {
    b.bin.join(name).display().to_string()
}

#[test]
fn vim_is_taken_where_there_is_no_nvim() {
    let b = scratch("vim");
    program(&b.bin.join("vim"), true);
    program(&b.bin.join("nvim"), false); // there, but can't run
    assert_eq!(
        pick(&b, &[]),
        json!({"editor": {"argv": [at(&b, "vim")], "kind": "vim", "name": "vim"}})
    );
    program(&b.bin.join("nvim"), true);
    assert_eq!(pick(&b, &[])["editor"]["kind"], "nvim");
}

#[test]
fn the_config_then_visual_then_editor_then_the_vi_family() {
    let b = scratch("order");
    for p in ["nvim", "vim", "code", "hx"] {
        program(&b.bin.join(p), true);
    }
    let name = |v: Value| v["editor"]["name"].as_str().unwrap().to_string();
    // No `editor` in config.toml: $EDITOR comes before the default nvim.
    assert_eq!(name(pick(&b, &[("EDITOR", "hx")])), "hx");
    assert_eq!(
        name(pick(&b, &[("VISUAL", "code"), ("EDITOR", "hx")])),
        "code"
    );
    // A $VISUAL that isn't there is skipped.
    assert_eq!(name(pick(&b, &[("VISUAL", "zed"), ("EDITOR", "hx")])), "hx");
    // The config's own choice comes first, with its arguments.
    std::fs::write(
        b.dir.join("cfg/config.toml"),
        "editor = [\"code\", \"--wait\"]\n",
    )
    .unwrap();
    assert_eq!(
        pick(&b, &[("VISUAL", "hx")])["editor"],
        json!({"argv": [at(&b, "code"), "--wait"], "kind": "other", "name": "code"})
    );
    // Unless it isn't there.
    std::fs::write(b.dir.join("cfg/config.toml"), "editor = [\"subl\"]\n").unwrap();
    assert_eq!(name(pick(&b, &[("VISUAL", "hx")])), "hx");
    assert_eq!(name(pick(&b, &[])), "nvim");
}

#[test]
fn nothing_but_editor_split_like_a_shell_word_list() {
    let b = scratch("only-editor");
    let ed = b.dir.join("My Apps/ed it");
    program(&ed, true);
    let v = pick(&b, &[("EDITOR", &format!("'{}' -w --new", ed.display()))]);
    assert_eq!(
        v["editor"],
        json!({"argv": [ed.display().to_string(), "-w", "--new"], "kind": "other", "name": "ed it"})
    );
    // A quote left open names nothing.
    let v = pick(&b, &[("EDITOR", &format!("'{} -w", ed.display()))]);
    assert_ne!(v["editor"]["name"], "ed it", "{v}");
}

#[test]
fn nothing_at_all_is_the_system_opener_here_or_an_error_naming_the_config() {
    let b = scratch("nothing");
    let v = pick(&b, &[]);
    if cfg!(target_os = "macos") {
        assert_eq!(
            v,
            json!({"editor": {"argv": ["/usr/bin/open", "-t"], "kind": "system", "name": "open"}})
        );
    } else {
        assert_eq!(v["editor"], Value::Null);
        let err = v["error"].as_str().unwrap();
        assert!(
            err.starts_with("No editor found: install nvim or set editor"),
            "{err}"
        );
        assert!(
            err.contains(&b.dir.join("cfg/config.toml").display().to_string()),
            "{err}"
        );
    }
}

#[test]
fn herdr_nvim_is_asked_only_where_nvim_is() {
    let b = scratch("herdr-nvim");
    let repo = common::fixture("editor-herdr-nvim");
    let git = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::os::unix::fs::symlink(git.trim(), b.bin.join("git")).unwrap();
    // A herdr-nvim that records what it's asked and takes it.
    let log = b.dir.join("herdr-nvim.log");
    let hnv = b.dir.join("app/mods/herdr-nvim/bin/herdr-nvim");
    std::fs::create_dir_all(hnv.parent().unwrap()).unwrap();
    std::fs::write(
        &hnv,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\necho '{{\"terminal\":\"t9\"}}'\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hnv, std::fs::Permissions::from_mode(0o755)).unwrap();
    let open = || -> Value {
        let out = common::gitview()
            .args(["json", "nvim", "--repo"])
            .arg(&repo.dir)
            .args([
                "--path", "base.txt", "--line", "2", "--tab", "tb1", "--space", "sp1",
            ])
            .env_clear()
            .env("PATH", &b.bin)
            .env("HOME", &b.dir)
            .env("HERDR_APP_HOME", b.dir.join("app"))
            .env("GITVIEW_HC", b.dir.join("no-hc"))
            .env("GITVIEW_CONFIG_DIR", b.dir.join("cfg"))
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    assert_eq!(
        open(),
        json!({"terminal": null}),
        "no nvim: herdr-nvim not asked"
    );
    assert!(!log.exists(), "herdr-nvim ran");
    program(&b.bin.join("nvim"), true);
    assert_eq!(open(), json!({"terminal": "herdr-nvim"}));
    let asked = std::fs::read_to_string(&log).unwrap();
    assert!(asked.starts_with("open --tab tb1 --space sp1 "), "{asked}");
    assert!(asked.trim_end().ends_with("/base.txt:2"), "{asked}");
}
