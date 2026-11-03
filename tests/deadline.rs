//! Calls that could hang end: `deadline::output_within` itself, and the app
//! ops that talk to herdr-core (`json sidebar`, `json send`) against an
//! `hc` whose `connect` never answers.
//!
//! Failure modes this catches:
//! - an op the mods wait on that never returns (the app's exec has no
//!   deadline of its own the mods see);
//! - the hung program, or what it started, left running afterwards;
//! - an error that doesn't say what hung;
//! - a call that finishes in time losing its output, input or exit code.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use herdr_gitview::deadline::output_within;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gitview-deadline-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Whether the pid in `file` is a live process.
fn alive(file: &Path) -> bool {
    let pid = std::fs::read_to_string(file).unwrap();
    Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .unwrap()
        .success()
}

/// Wait up to 2 s for `file` to have a pid in it.
fn pid_in(file: &Path) {
    let until = Instant::now() + Duration::from_secs(2);
    while std::fs::read_to_string(file).map_or(true, |s| s.trim().is_empty()) {
        assert!(
            Instant::now() < until,
            "no pid written to {}",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_program_past_its_deadline_is_stopped_with_what_it_started() {
    let dir = scratch("unit");
    let pid = dir.join("child.pid");
    let t = Instant::now();
    let err = output_within(
        Command::new("sh").args([
            "-c",
            &format!("sleep 30 & echo $! > '{}'; wait", pid.display()),
        ]),
        None,
        Duration::from_millis(500),
    )
    .unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert!(format!("{err:#}").contains("sh took more than"), "{err:#}");
    pid_in(&pid);
    assert!(!alive(&pid), "what it started is still running");

    // In time: input in, output and exit code out.
    let out = output_within(
        Command::new("sh").args(["-c", "tr a-z A-Z; echo err >&2; exit 3"]),
        Some(b"hello\n"),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(out.stdout, b"HELLO\n");
    assert_eq!(out.stderr, b"err\n");
    assert_eq!(out.status.code(), Some(3));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `json <args>` with an `hc` whose `connect` hangs (writing the pid of the
/// sleep it started): (seconds taken, reply, the sleep's pid file).
fn against_hanging_hc(name: &str, args: &[&str]) -> (Duration, serde_json::Value, PathBuf) {
    let dir = scratch(name);
    let pid = dir.join("sleep.pid");
    let hc = dir.join("hc");
    std::fs::write(
        &hc,
        format!(
            "#!/bin/sh\ncase \"$1\" in connect) sleep 60 & echo $! > '{}'; wait;; esac\nexit 1\n",
            pid.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hc, std::fs::Permissions::from_mode(0o755)).unwrap();
    let t = Instant::now();
    let out = common::gitview()
        .arg("json")
        .args(args)
        .env("GITVIEW_HC", &hc)
        .env("GITVIEW_HERDR_APP", dir.join("no-app"))
        .env("GITVIEW_CONFIG_DIR", dir.join("no-config"))
        .output()
        .unwrap();
    let took = t.elapsed();
    let reply = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!out.status.success(), "{reply}");
    (took, reply, pid)
}

#[test]
fn ops_that_ask_herdr_core_end_when_it_doesnt_answer() {
    let ops: [(&str, &[&str]); 2] = [
        (
            "sidebar",
            &["sidebar", "--tab", "t1", "--mod", "gitview-diff"],
        ),
        ("send", &["send", "--terminal", "t1", "--", "hi"]),
    ];
    let runs: Vec<_> = ops
        .into_iter()
        .map(|(name, args)| std::thread::spawn(move || (name, against_hanging_hc(name, args))))
        .collect();
    for run in runs {
        let (name, (took, reply, pid)) = run.join().unwrap();
        assert!(took < Duration::from_secs(15), "{name} took {took:?}");
        let err = reply["error"].as_str().unwrap_or_default();
        assert!(err.contains("took more than 10 s"), "{name}: {reply}");
        pid_in(&pid);
        assert!(!alive(&pid), "{name}: hc's sleep is still running");
    }
}
