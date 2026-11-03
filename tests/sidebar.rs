//! `herdr-gitview json sidebar`: where the diff/list pane goes, against a
//! fake `hc` (the layout documents, `doc.get {name, on?}`) and a fake
//! `herdr-app` (`status`, `do pane.split`, `do layout.resize`), both Python.
//!
//! Failure modes this catches:
//! - a local tab not split at its root (the pane wouldn't span the tab);
//! - a remote tab (`machine/…`) looked up only in this machine's document,
//!   or split by a bare id from that machine's document (which could name a
//!   pane here);
//! - no fallback to the tab's focused pane, or a split of the focused pane
//!   when it belongs to another tab;
//! - the width not applied to the new pane;
//! - the app asked for more than the split and the resize when the layout
//!   document has the tab (its status costs a process per open).

mod common;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FAKE_HC: &str = r#"#!/usr/bin/env python3
import json, os, sys
if sys.argv[1:] != ["connect"]:
    sys.exit(2)
docs = json.loads(os.environ["FAKE_DOCS"])
for line in sys.stdin:
    r = json.loads(line)
    result = {}
    if r["method"] == "doc.get":
        result = {"value": docs.get(r["params"].get("on") or "", None), "version": 1}
    print(json.dumps({"jsonrpc": "2.0", "id": r["id"], "result": result}), flush=True)
"#;

const FAKE_APP: &str = r#"#!/usr/bin/env python3
import json, os, sys
with open(os.environ["FAKE_APP_LOG"], "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")
a = sys.argv[1:]
if a == ["status"]:
    print(os.environ["FAKE_STATUS"])
elif a[:2] == ["do", "pane.split"]:
    p = json.loads(a[2])
    if p["pane"] in json.loads(os.environ["FAKE_SPLITTABLE"]):
        print(json.dumps({"pane": "new-" + p["pane"]}))
    else:
        print(json.dumps({"error": "no pane " + p["pane"]}))
        sys.exit(1)
else:
    print("{}")
"#;

fn doc(tab: &str, root: &str) -> Value {
    json!({"spaces": [{"id": "s1", "tabs": [{"id": tab, "root": {"id": root, "type": "row"}}]}]})
}

struct Fakes {
    dir: PathBuf,
}

impl Drop for Fakes {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fakes(name: &str) -> Fakes {
    let dir = std::env::temp_dir().join(format!("gitview-sidebar-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    for (file, body) in [("hc", FAKE_HC), ("herdr-app", FAKE_APP)] {
        std::fs::write(dir.join(file), body).unwrap();
        std::fs::set_permissions(dir.join(file), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Fakes { dir }
}

impl Fakes {
    /// `json sidebar --tab <tab>`: (ok, reply, the app calls made).
    fn sidebar(
        &self,
        tab: &str,
        docs: Value,
        status: Value,
        splittable: &[&str],
    ) -> (bool, Value, Vec<Value>) {
        let log = self.dir.join("app.log");
        let _ = std::fs::remove_file(&log);
        let out = common::gitview()
            .args(["json", "sidebar", "--tab", tab, "--mod", "gitview-diff"])
            .args(["--percent", "25", "--props", r#"{"tab":"x"}"#])
            .env("GITVIEW_HC", self.dir.join("hc"))
            .env("GITVIEW_HERDR_APP", self.dir.join("herdr-app"))
            .env("FAKE_DOCS", docs.to_string())
            .env("FAKE_STATUS", status.to_string())
            .env("FAKE_SPLITTABLE", json!(splittable).to_string())
            .env("FAKE_APP_LOG", &log)
            .env("GITVIEW_CONFIG_DIR", self.dir.join("no-config"))
            .output()
            .unwrap();
        let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.success(), reply, calls(&log))
    }
}

fn calls(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn splits(calls: &[Value]) -> Vec<String> {
    calls
        .iter()
        .filter(|c| c[1] == "pane.split")
        .map(|c| {
            let p: Value = serde_json::from_str(c[2].as_str().unwrap()).unwrap();
            p["pane"].as_str().unwrap().to_string()
        })
        .collect()
}

#[test]
fn a_local_tab_is_split_at_its_root_and_the_pane_gets_its_width() {
    let f = fakes("local");
    let (ok, reply, calls) = f.sidebar(
        "tb1",
        json!({"": doc("tb1", "root1")}),
        json!({"tab": "tb1", "focused_pane": "p9"}),
        &["root1", "p9"],
    );
    assert!(ok, "{reply}");
    assert_eq!(reply, json!({"pane": "new-root1"}));
    assert_eq!(splits(&calls), ["root1"]);
    let asked: Vec<&str> = calls
        .iter()
        .map(|c| c[1].as_str().unwrap_or("status"))
        .collect();
    assert_eq!(
        asked,
        ["pane.split", "layout.resize"],
        "nothing else asked of the app"
    );
    let split: Value = serde_json::from_str(calls[0][2].as_str().unwrap()).unwrap();
    assert_eq!(split["mod"], "gitview-diff");
    assert_eq!(split["props"], json!({"tab": "x"}));
    let resize = calls.iter().find(|c| c[1] == "layout.resize").unwrap();
    let resize: Value = serde_json::from_str(resize[2].as_str().unwrap()).unwrap();
    assert_eq!(resize["id"], "new-root1");
    assert!((resize["weight"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn a_remote_tab_is_split_at_its_root_in_that_machines_document() {
    let f = fakes("remote-doc");
    let (ok, reply, calls) = f.sidebar(
        "box/tb2",
        json!({"": doc("tb1", "root1"), "box": doc("tb2", "r2")}),
        json!({"tab": "box/tb2", "focused_pane": "box/p3"}),
        &["box/r2", "box/p3"],
    );
    assert!(ok, "{reply}");
    assert_eq!(reply, json!({"pane": "new-box/r2"}));
    assert_eq!(splits(&calls), ["box/r2"], "never the bare id r2");
}

#[test]
fn a_remote_tab_nobody_can_split_at_the_root_gets_its_focused_pane_split() {
    let f = fakes("remote-focused");
    let (ok, reply, calls) = f.sidebar(
        "box/tb2",
        json!({"": doc("tb1", "root1"), "box": doc("tb2", "r2")}),
        json!({"tab": "box/tb2", "focused_pane": "box/p3"}),
        &["box/p3"],
    );
    assert!(ok, "{reply}");
    assert_eq!(reply, json!({"pane": "new-box/p3"}));
    assert_eq!(splits(&calls), ["box/r2", "box/p3"]);
}

#[test]
fn a_tab_in_no_document_and_not_focused_is_an_error_and_splits_nothing() {
    let f = fakes("unknown");
    let (ok, reply, calls) = f.sidebar(
        "box/tb2",
        json!({"": doc("tb1", "root1")}),
        json!({"tab": "tb1", "focused_pane": "p9"}),
        &["root1", "p9"],
    );
    assert!(!ok);
    assert!(
        reply["error"].as_str().unwrap().contains("box/tb2"),
        "{reply}"
    );
    assert!(splits(&calls).is_empty(), "{calls:?}");
}
