//! `herdr-gitview app-setup install|uninstall` end to end, against a fake
//! `herdr-app` (Python) that answers `status`, does `layout.clone` and
//! `layout.switch` the way the app does, and logs every call. Install then
//! uninstall leaves every layout and `keys.json` byte for byte as before, and
//! uninstall never switches or deletes a layout.
//!
//! Failure modes this catches:
//! - a shipped layout edited in place (also when `installed.json` is missing:
//!   then both stop before writing anything);
//! - a second clone, a switch or clone on uninstall, a deleted layout;
//! - files not byte-identical after install + uninstall (keys and layouts);
//! - a person's key replaced, or a retired key/mod removed that isn't ours;
//! - an ambiguous copy guessed instead of refused.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

const FAKE_APP: &str = r#"#!/usr/bin/env python3
import json, os, sys
home = os.environ["HERDR_APP_HOME"]
with open(os.path.join(home, "app.log"), "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")
shown_file = os.path.join(home, "shown")
args = sys.argv[1:]
if args == ["status"]:
    print(json.dumps({"layout": open(shown_file).read().strip()}))
elif args[:1] == ["do"]:
    cmd, params = args[1], json.loads(args[2])
    if cmd == "layout.clone":
        src = os.path.join(home, "layouts", params["name"] + ".json")
        name = params.get("as") or params["name"] + "-copy"
        doc = json.load(open(src))
        doc["cloned_from"] = {"name": params["name"], "version": "0.1.12"}
        with open(os.path.join(home, "layouts", name + ".json"), "w") as f:
            f.write(json.dumps(doc, indent=2, sort_keys=True) + "\n")
        open(shown_file, "w").write(name)
    elif cmd == "layout.switch":
        open(shown_file, "w").write(params["name"])
    print("{}")
else:
    sys.exit(2)
"#;

/// The keys gitview ships, as the installer passes them.
const OURS: &str =
    r#"{"global": {"cmd-g": "mod.open:gitview-tab", "cmd-shift-g": "mod.open:gitview-sidebar"}}"#;

struct Home {
    dir: PathBuf,
    app: PathBuf,
    keys: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.dir.parent().unwrap());
    }
}

fn pretty(v: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(v).unwrap())
}

/// Herdr's shipped `main` (abridged).
fn main_layout() -> Value {
    json!({"children": [
        {"content": {"tab": "active"}, "id": "main", "type": "block"},
        {"children": [{"active": "files", "children": [
            {"content": {"mod": "files"}, "id": "files", "type": "block"},
            {"content": {"mod": "changes"}, "id": "changes", "type": "block"}],
          "id": "panel-body", "type": "stack"}],
         "hidden": true, "id": "panel", "size": {"px": 368}, "type": "column"}],
      "id": "root", "type": "row"})
}

fn home(name: &str, shown: &str) -> Home {
    let root =
        std::env::temp_dir().join(format!("gitview-app-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("herdr-app");
    std::fs::create_dir_all(dir.join("layouts")).unwrap();
    std::fs::write(
        dir.join("installed.json"),
        r#"{"layout:main": {"version": "0.1.12"}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("layouts/main.json"), pretty(&main_layout())).unwrap();
    std::fs::write(
        dir.join("keys.json"),
        "{\n  \"global\": {\n    \"cmd-g\": \"mod.open:mine\"\n  }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("shown"), shown).unwrap();
    let app = root.join("herdr-app-bin");
    std::fs::write(&app, FAKE_APP).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    let keys = root.join("keys.json");
    std::fs::write(&keys, OURS).unwrap();
    Home { dir, app, keys }
}

impl Home {
    fn setup(&self, op: &str) -> String {
        let (ok, text) = self.try_setup(op);
        assert!(ok, "app-setup {op}: {text}");
        text
    }

    fn try_setup(&self, op: &str) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_herdr-gitview"))
            .args(["app-setup", op, "--home"])
            .arg(&self.dir)
            .arg("--app")
            .arg(&self.app)
            .arg("--keys")
            .arg(&self.keys)
            .env_remove("HERDR_APP_HOME")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    fn file(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.join(rel)).unwrap()
    }

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("app.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn snapshot(&self) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(self.dir.join("layouts"))
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();
        files.push(("keys.json".into(), self.file("keys.json")));
        files.sort();
        files
    }
}

fn changes_slot(layout: &str) -> Value {
    let v: Value = serde_json::from_str(layout).unwrap();
    v.pointer("/children/1/children/0/children/1/content")
        .unwrap()
        .clone()
}

#[test]
fn on_shipped_main_it_clones_once_then_uninstall_leaves_the_copy_as_cloned() {
    let h = home("clone", "main");
    let out = h.setup("install");
    assert!(out.contains("cloned main as main-copy"), "{out}");
    assert!(out.contains("kept yours: global cmd-g"), "{out}");
    assert_eq!(
        changes_slot(&h.file("layouts/main-copy.json")),
        json!({"mod": "gitview", "props": {"replaces": {"mod": "changes"}}})
    );
    assert_eq!(h.file("layouts/main.json"), pretty(&main_layout()));
    let keys: Value = serde_json::from_str(&h.file("keys.json")).unwrap();
    assert_eq!(
        keys,
        json!({"global": {"cmd-g": "mod.open:mine", "cmd-shift-g": "mod.open:gitview-sidebar"}})
    );

    // A second install: the copy is used, nothing is cloned again.
    std::fs::write(h.dir.join("shown"), "main").unwrap();
    let out = h.setup("install");
    assert!(out.contains("switched to main-copy"), "{out}");
    assert!(out.contains("already shows gitview"), "{out}");
    let clones = h.calls().iter().filter(|c| c[1] == "layout.clone").count();
    assert_eq!(clones, 1);

    let calls_before = h.calls().len();
    h.setup("uninstall");
    assert_eq!(h.calls().len(), calls_before, "uninstall needs no app");
    assert!(
        h.dir.join("layouts/main-copy.json").exists(),
        "never deleted"
    );
    let mut cloned = main_layout();
    cloned["cloned_from"] = json!({"name": "main", "version": "0.1.12"});
    let restored: Value = serde_json::from_str(&h.file("layouts/main-copy.json")).unwrap();
    assert_eq!(restored, cloned);
    assert_eq!(
        h.file("keys.json"),
        "{\n  \"global\": {\n    \"cmd-g\": \"mod.open:mine\"\n  }\n}\n"
    );
}

#[test]
fn install_then_uninstall_is_byte_identical_on_an_old_main_mods_herdr_nvim_patched() {
    let h = home("mods", "main-mods");
    let mut mods = main_layout();
    mods["cloned_from"] = json!({"name": "main", "version": "0.1.11"});
    mods["children"][1]["size"] = json!({"px": 300});
    mods["children"][1]["children"][0]["children"][0]["content"] =
        json!({"mod": "herdr-nvim", "props": {"panel_width": 300, "replaces": {"mod": "files"}}});
    mods["floating"] = json!([{"content": {"mod": "herdr-nvim", "props": {"role": "sync"}},
        "id": "herdr-nvim-sync"}]);
    std::fs::write(h.dir.join("layouts/main-mods.json"), pretty(&mods)).unwrap();
    let before = h.snapshot();

    let out = h.setup("install");
    assert!(
        out.contains("main-mods: the Changes panel shows gitview now"),
        "{out}"
    );
    let patched: Value = serde_json::from_str(&h.file("layouts/main-mods.json")).unwrap();
    assert_eq!(patched["children"][1]["size"], json!({"px": 300}));
    assert_eq!(
        patched["children"][1]["children"][0]["children"][0]["content"]["mod"],
        "herdr-nvim"
    );
    assert_eq!(patched["floating"], mods["floating"]);
    assert!(
        h.calls().iter().all(|c| c[0] == "status"),
        "a personal layout on screen: no switch, no clone"
    );

    h.setup("uninstall");
    assert_eq!(h.snapshot(), before);
}

#[test]
fn several_copies_stop_the_install_without_touching_a_layout() {
    let h = home("ambiguous", "main");
    let mut copy = main_layout();
    copy["cloned_from"] = json!({"name": "main"});
    std::fs::write(h.dir.join("layouts/main-mods.json"), pretty(&copy)).unwrap();
    std::fs::write(h.dir.join("layouts/main-2.json"), pretty(&copy)).unwrap();
    let layouts_before: Vec<_> = h
        .snapshot()
        .into_iter()
        .filter(|(n, _)| n != "keys.json")
        .collect();
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-gitview"))
        .args(["app-setup", "install", "--home"])
        .arg(&h.dir)
        .arg("--app")
        .arg(&h.app)
        .arg("--keys")
        .arg(&h.keys)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("several personal copies of main"));
    let layouts_after: Vec<_> = h
        .snapshot()
        .into_iter()
        .filter(|(n, _)| n != "keys.json")
        .collect();
    assert_eq!(layouts_after, layouts_before);
}

#[test]
fn without_installed_json_neither_install_nor_uninstall_touches_anything() {
    let h = home("no-installed", "main");
    std::fs::remove_file(h.dir.join("installed.json")).unwrap();
    let before = h.snapshot();
    for op in ["install", "uninstall"] {
        let (ok, out) = h.try_setup(op);
        assert!(!ok, "{op} went ahead: {out}");
        assert!(out.contains("installed.json"), "{out}");
        assert_eq!(h.snapshot(), before, "{op} changed files");
    }
    assert!(h.calls().is_empty(), "no app call either");
    // Unreadable is the same as missing.
    std::fs::write(h.dir.join("installed.json"), "not json").unwrap();
    assert!(!h.try_setup("install").0);
    assert_eq!(h.snapshot(), before);
}

#[test]
fn retired_keys_and_mods_of_older_installs_go_but_only_ours() {
    let h = home("retired", "main");
    std::fs::write(
        h.dir.join("keys.json"),
        pretty(&json!({"global": {
            "cmd-alt-g": "mod.open:gitview-tui-tab",
            "cmd-alt-shift-g": "mod.open:my-own-thing",
            "cmd-t": "tab.new"}})),
    )
    .unwrap();
    for (name, title) in [
        ("gitview-popup", "gitview: popup"),
        ("gitview-tui-tab", "gitview TUI: toggle in own tab"),
        ("gitview-tui-sidebar", "someone else's"),
    ] {
        std::fs::create_dir_all(h.dir.join("mods").join(name)).unwrap();
        std::fs::write(
            h.dir.join("mods").join(name).join("mod.json"),
            json!({"title": title}).to_string(),
        )
        .unwrap();
    }
    let out = h.setup("install");
    assert!(
        out.contains("removed the retired global cmd-alt-g"),
        "{out}"
    );
    let keys: Value = serde_json::from_str(&h.file("keys.json")).unwrap();
    assert!(keys["global"].get("cmd-alt-g").is_none());
    assert_eq!(keys["global"]["cmd-alt-shift-g"], "mod.open:my-own-thing");
    assert!(!h.dir.join("mods/gitview-popup").exists());
    assert!(!h.dir.join("mods/gitview-tui-tab").exists());
    assert!(h.dir.join("mods/gitview-tui-sidebar").exists(), "not ours");

    // An uninstall also takes a retired key a newer install never saw.
    let mut keys: Value = serde_json::from_str(&h.file("keys.json")).unwrap();
    keys["global"]["cmd-alt-g"] = json!("mod.open:gitview-tui-tab");
    std::fs::write(h.dir.join("keys.json"), pretty(&keys)).unwrap();
    std::fs::create_dir_all(h.dir.join("mods/gitview-popup")).unwrap();
    std::fs::write(
        h.dir.join("mods/gitview-popup/mod.json"),
        r#"{"title":"gitview: popup"}"#,
    )
    .unwrap();
    h.setup("uninstall");
    let keys: Value = serde_json::from_str(&h.file("keys.json")).unwrap();
    assert_eq!(
        keys,
        json!({"global": {"cmd-alt-shift-g": "mod.open:my-own-thing", "cmd-t": "tab.new"}})
    );
    assert!(!h.dir.join("mods/gitview-popup").exists());
}
