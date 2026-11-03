//! `herdr-gitview json send`: placing notes in an agent's input goes through
//! a framed `terminal.attach` over `hc connect` (no clipboard), and
//! `--submit` goes through `hc send`. A fake `hc` (Python) plays herdr-core:
//! it answers `terminal.get`, the framed attach and its screen replay, and
//! records what was typed.
//!
//! Failure modes this catches:
//! - placing reaches for anything but the framed attach (`hc send`, a
//!   clipboard tool): the attach log is empty or an `argv` call shows up;
//! - the attach resizes the terminal (cols/rows other than its own);
//! - bracketed paste isn't detected from the replay, or newlines get typed
//!   as Enter into a program without it (it must refuse instead);
//! - `--submit` doesn't go through `hc send`, or sends trailing newlines.

mod common;

use std::path::{Path, PathBuf};

const FAKE_HC: &str = r#"#!/usr/bin/env python3
import json, os, struct, sys
LOG = os.environ["FAKE_HC_LOG"]
REPLAY = os.environ.get("FAKE_HC_REPLAY", "").encode().decode("unicode_escape").encode("latin-1")
def log(kind, data):
    with open(LOG, "a") as f:
        f.write(json.dumps([kind, data]) + "\n")
args = sys.argv[1:]
if args[:1] != ["connect"]:
    log("argv", args)
    print("{}")
    sys.exit(0)
inp, out = sys.stdin.buffer, sys.stdout.buffer
first = json.loads(inp.readline())
if not first.get("params", {}).get("framed"):
    # Line mode: answer each request (hello, terminal.get) until EOF.
    reqs = [first] + [json.loads(l) for l in inp if l.strip()]
    for r in reqs:
        result = {"cols": 101, "rows": 33, "state": "live"} if r["method"] == "terminal.get" else {}
        log("rpc", r["method"])
        out.write((json.dumps({"jsonrpc": "2.0", "id": r["id"], "result": result}) + "\n").encode())
    sys.exit(0)
out.write(b'{"jsonrpc":"2.0","id":0,"result":{}}\n'); out.flush()
def frame(ch, kind, payload):
    out.write(struct.pack("<IBI", ch, kind, len(payload)) + payload); out.flush()
while True:
    head = inp.read(9)
    if len(head) < 9:
        break
    ch, kind, n = struct.unpack("<IBI", head)
    payload = inp.read(n)
    if ch == 0:
        msg = json.loads(payload)
        log("attach", msg["params"])
        frame(0, 0, json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": {"channel": 5}}).encode())
    elif kind == 1:
        frame(5, 0, REPLAY)
    elif kind == 0:
        log("typed", payload.decode())
    elif kind == 2:
        log("closed", ch)
        break
"#;

fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("gitview-send-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("hc");
    std::fs::write(&bin, FAKE_HC).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, bin)
}

fn send_raw(
    dir: &Path,
    hc: &Path,
    replay: &str,
    args: &[&str],
) -> (bool, String, Vec<serde_json::Value>) {
    let log = dir.join("hc.log");
    let _ = std::fs::remove_file(&log);
    let out = common::gitview()
        .args(["json", "send", "--terminal", "t9"])
        .args(args)
        .env("GITVIEW_HC", hc)
        .env("FAKE_HC_LOG", &log)
        .env("FAKE_HC_REPLAY", replay)
        .output()
        .unwrap();
    let log = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        log,
    )
}

fn send(dir: &Path, hc: &Path, replay: &str, args: &[&str]) -> Vec<serde_json::Value> {
    let (ok, stdout, log) = send_raw(dir, hc, replay, args);
    assert!(ok, "send failed: {stdout}");
    log
}

fn entries<'a>(log: &'a [serde_json::Value], kind: &str) -> Vec<&'a serde_json::Value> {
    log.iter().filter(|e| e[0] == kind).map(|e| &e[1]).collect()
}

#[test]
fn placing_pastes_through_a_framed_attach_at_the_terminals_size() {
    let (dir, hc) = setup("bracketed");
    let log = send(&dir, &hc, "\\x1b[?2004h> ", &["--", "note one\nnote two\n"]);
    let attach = entries(&log, "attach");
    assert_eq!(attach.len(), 1, "{log:?}");
    assert_eq!(attach[0]["terminal"], "t9");
    assert_eq!(
        (attach[0]["cols"].clone(), attach[0]["rows"].clone()),
        (101.into(), 33.into())
    );
    // The program asked for bracketed paste: newlines stay, wrapped.
    assert_eq!(
        entries(&log, "typed"),
        [&serde_json::json!(
            "\u{1b}[200~note one\nnote two\u{1b}[201~"
        )]
    );
    assert_eq!(entries(&log, "closed").len(), 1);
    assert!(entries(&log, "argv").is_empty(), "no hc send: {log:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn placing_multi_line_text_into_a_plain_program_is_refused_and_types_nothing() {
    let (dir, hc) = setup("plain");
    let (ok, stdout, log) = send_raw(&dir, &hc, "$ ", &["--", "a\nb"]);
    assert!(!ok);
    assert!(stdout.contains("use shift+enter to send"), "{stdout}");
    assert!(entries(&log, "typed").is_empty(), "nothing typed: {log:?}");
    assert_eq!(entries(&log, "closed").len(), 1, "detached cleanly");
    // One line is fine there.
    let log = send(&dir, &hc, "$ ", &["--", "one line\n"]);
    assert_eq!(entries(&log, "typed"), [&serde_json::json!("one line")]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn submitting_uses_hc_send() {
    let (dir, hc) = setup("submit");
    let log = send(&dir, &hc, "", &["--submit", "hello\n"]);
    assert_eq!(
        entries(&log, "argv"),
        [&serde_json::json!(["send", "t9", "hello", "--json"])]
    );
    assert!(entries(&log, "attach").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
