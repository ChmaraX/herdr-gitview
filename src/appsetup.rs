//! `herdr-gitview app-setup install|uninstall --keys <keys.json> [--home DIR]
//! [--app BIN]`: the two edits the Herdr app install makes outside the mod
//! folders, and undoes them. The same model as herdr-nvim's
//! `src/app/setup.rs`, so the two install and uninstall in any order.
//!
//! * **Layout** ([`crate::applayout`]): the person's one personal layout, the
//!   one on screen, gets gitview in its `changes` slot. Herdr's own layouts
//!   (`installed.json` lists them as `layout:<name>`) can't be edited: on one,
//!   the person's existing copy of it is used (`<name>-copy`, else the only
//!   layout cloned from it, e.g. an older install's `<name>-mods`) and
//!   switched to, else it is cloned once (`layout.clone {name}` makes
//!   `<name>-copy` and shows it). Several copies: stop and ask the person to
//!   switch to the one they want. A personal layout is edited in place.
//! * **Uninstall** takes gitview's own edits (`props.replaces`) out of every
//!   personal layout. It never deletes or switches a layout, and needs no
//!   running app.
//! * **Write rule**: every write re-reads the file and renames a temp file
//!   in only if the file is unchanged since the read (else reads again, a
//!   few times).
//! * **Keys**: our `keys.json` entries are added where the key is free; a key
//!   the person already uses for something else is kept and reported.
//!   Uninstall removes only bindings that still have our action.
//! * **Retired**: what older installs added and nothing uses any more goes on
//!   install and uninstall alike: the TUI-in-the-app keys (only while they
//!   still carry exactly our old actions) and the mod folders
//!   `gitview-popup`, `gitview-tui-sidebar`, `gitview-tui-tab` (only when
//!   their `mod.json` says they're gitview's).
//! * **`installed.json`** (which layouts are Herdr's own) must be readable:
//!   without it a shipped layout would look personal and get edited, so both
//!   stop before changing anything.
//! * **Order**: install reads the app's status and chooses the layout before
//!   any write, so a refused install (several copies, a foreign
//!   `<name>-copy`) leaves `keys.json`, the mod folders and every layout as
//!   they were. Key removals keep the order of the rest of `keys.json`.
//!
//! `--home`: the app home (default `$HERDR_APP_HOME`, else
//! `$HOME/.config/herdr-app`). `--app`: the `herdr-app` binary (default
//! `$GITVIEW_HERDR_APP`, else `herdr-app` on `PATH`); install only. Both as
//! [`App::from_env`] reads them for `json`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::apphost::App;
use crate::applayout::{self, Patched};

/// Keys older installs bound for the TUI inside the app, now gone.
const RETIRED_KEYS: &str = r#"{"global": {
    "cmd-alt-g": "mod.open:gitview-tui-tab",
    "cmd-alt-shift-g": "mod.open:gitview-tui-sidebar"
}}"#;

/// Mods older installs copied, now gone.
const RETIRED_MODS: [&str; 3] = ["gitview-popup", "gitview-tui-sidebar", "gitview-tui-tab"];

const USAGE: &str =
    "usage: herdr-gitview app-setup install|uninstall --keys <keys.json> [--home DIR] [--app BIN]";

pub fn run(args: &[String]) -> Result<()> {
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let mut app = App::from_env();
    if let Some(home) = value("--home") {
        app.home = PathBuf::from(home);
    }
    if let Some(bin) = value("--app") {
        app.bin = bin.into();
    }
    let keys_path = value("--keys").context(USAGE)?;
    let ours: Value = serde_json::from_str(
        &fs::read_to_string(&keys_path).with_context(|| format!("cannot read {keys_path}"))?,
    )
    .with_context(|| format!("{keys_path}: invalid JSON"))?;
    match args.first().map(String::as_str) {
        Some("install") => install(&app, &ours),
        Some("uninstall") => uninstall(&app.home, &ours),
        _ => bail!(USAGE),
    }
}

fn say(line: impl AsRef<str>) {
    println!("gitview: {}", line.as_ref());
}

fn install(app: &App, ours: &Value) -> Result<()> {
    let home = &app.home;
    if !home.is_dir() {
        bail!(
            "no Herdr app home at {} (start the app once, or pass --home)",
            home.display()
        );
    }
    let shipped = shipped_layouts(home)?;
    // Where the layout part goes is decided first: it may refuse, and then
    // nothing has changed yet.
    let (shown, target) = plan_layout(app, &shipped)?;
    let keys = home.join("keys.json");
    if !keys.exists() {
        fs::write(&keys, "{}\n")?;
    }
    let retired: Value = serde_json::from_str(RETIRED_KEYS).expect("valid JSON");
    // A keys file that still lists a retired binding never brings it back.
    let mut ours = ours.clone();
    unmerge_keys(&mut ours, &retired);
    let (mut report, mut gone) = ((Vec::new(), Vec::new()), Vec::new());
    edit_json(&keys, &mut |value| {
        gone = unmerge_keys(value, &retired);
        report = merge_keys(value, &ours);
        Ok(!report.0.is_empty() || !gone.is_empty())
    })?;
    gone.iter()
        .for_each(|l| say(format!("keys: removed the retired {l}")));
    report
        .0
        .iter()
        .for_each(|l| say(format!("keys: added {l}")));
    report
        .1
        .iter()
        .for_each(|l| say(format!("keys: kept yours: {l}")));
    remove_retired_mods(home)?;
    install_layout(app, &shown, target)
}

fn uninstall(home: &Path, ours: &Value) -> Result<()> {
    let shipped = shipped_layouts(home)?;
    let keys = home.join("keys.json");
    if keys.exists() {
        let retired: Value = serde_json::from_str(RETIRED_KEYS).expect("valid JSON");
        let mut removed = Vec::new();
        edit_json(&keys, &mut |value| {
            removed = unmerge_keys(value, ours);
            removed.extend(unmerge_keys(value, &retired));
            Ok(!removed.is_empty())
        })?;
        removed
            .iter()
            .for_each(|l| say(format!("keys: removed {l}")));
    }
    remove_retired_mods(home)?;
    uninstall_layouts(home, &shipped)
}

/// Delete the retired mod folders that are gitview's.
fn remove_retired_mods(home: &Path) -> Result<()> {
    for name in RETIRED_MODS {
        let dir = home.join("mods").join(name);
        if !dir.exists() {
            continue;
        }
        let title = fs::read_to_string(dir.join("mod.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|m| m.get("title")?.as_str().map(str::to_string))
            .unwrap_or_default();
        if title.starts_with("gitview") {
            fs::remove_dir_all(&dir).with_context(|| format!("cannot remove {}", dir.display()))?;
            say(format!("mods: removed the retired {name}"));
        } else {
            say(format!("mods: left {} (not gitview's)", dir.display()));
        }
    }
    Ok(())
}

// ---- layouts --------------------------------------------------------------------

fn layout_path(home: &Path, name: &str) -> PathBuf {
    home.join("layouts").join(format!("{name}.json"))
}

/// Herdr's own layouts (`installed.json` lists them as `layout:<name>`).
/// An error when the file can't be read: then nothing tells them apart.
fn shipped_layouts(home: &Path) -> Result<HashSet<String>> {
    let path = home.join("installed.json");
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "cannot read {} (it tells Herdr's own layouts from yours; start the app once)",
            path.display()
        )
    })?;
    let installed: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let entries = installed
        .as_object()
        .with_context(|| format!("{} is not an object", path.display()))?;
    Ok(entries
        .keys()
        .filter_map(|k| k.strip_prefix("layout:"))
        .map(str::to_string)
        .collect())
}

/// The person's own layouts: every `layouts/*.json` Herdr doesn't ship.
fn personal_layouts(home: &Path, shipped: &HashSet<String>) -> Vec<(String, Value)> {
    let mut found: Vec<(String, Value)> = fs::read_dir(home.join("layouts"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let layout = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            Some((name, layout))
        })
        .filter(|(name, _)| !shipped.contains(name))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Where the install goes, given the layout on screen.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// A personal layout: edit it in place.
    Edit(String),
    /// Herdr's own on screen; switch to the person's copy of it and edit that.
    Switch(String),
    /// Herdr's own on screen and no copy yet: clone it (`<name>-copy`).
    Clone(String),
}

/// Pure: which layout to install into while `shown` is on screen.
pub fn choose(shown: &str, shown_is_shipped: bool, personal: &[(String, Value)]) -> Result<Target> {
    if !shown_is_shipped {
        return Ok(Target::Edit(shown.to_string()));
    }
    let default = format!("{shown}-copy");
    let copies: Vec<&str> = personal
        .iter()
        .filter(|(_, l)| l.pointer("/cloned_from/name").and_then(Value::as_str) == Some(shown))
        .map(|(name, _)| name.as_str())
        .collect();
    if copies.contains(&default.as_str()) {
        return Ok(Target::Switch(default));
    }
    match copies.as_slice() {
        [one] => Ok(Target::Switch((*one).to_string())),
        [] if personal.iter().any(|(name, _)| *name == default) => bail!(
            "{default} exists but is not a copy of {shown}: switch to the personal layout you \
             want gitview in (ctrl-shift-l) and run the install again"
        ),
        [] => Ok(Target::Clone(shown.to_string())),
        several => bail!(
            "you have several personal copies of {shown} ({}): switch to the one you want \
             gitview in (ctrl-shift-l) and run the install again",
            several.join(", ")
        ),
    }
}

/// The layout on screen and where the install goes, without changing
/// anything (an error when [`choose`] refuses).
fn plan_layout(app: &App, shipped: &HashSet<String>) -> Result<(String, Target)> {
    let status = app.status()?;
    let shown = status
        .get("layout")
        .and_then(Value::as_str)
        .context("herdr-app status names no layout")?
        .to_string();
    let target = choose(
        &shown,
        shipped.contains(&shown),
        &personal_layouts(&app.home, shipped),
    )?;
    Ok((shown, target))
}

fn install_layout(app: &App, shown: &str, target: Target) -> Result<()> {
    let home = &app.home;
    let target = match target {
        Target::Edit(name) => name,
        Target::Switch(copy) => {
            app.command("layout.switch", json!({ "name": copy }))?;
            say(format!(
                "layout: switched to {copy}, your personal copy of {shown} (Herdr's own layouts can't be edited)"
            ));
            copy
        }
        Target::Clone(shown) => {
            app.command("layout.clone", json!({ "name": shown }))?;
            let copy = format!("{shown}-copy");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !layout_path(home, &copy).exists() {
                if std::time::Instant::now() > deadline {
                    bail!("layout.clone {shown} made no {copy}");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            say(format!(
                "layout: cloned {shown} as {copy}, your personal layout from now on (Herdr's own layouts can't be edited)"
            ));
            copy
        }
    };
    install_into(home, &target)
}

/// Patch the personal layout `name` (a file in `home/layouts`).
fn install_into(home: &Path, name: &str) -> Result<()> {
    let mut outcome = Patched::Already;
    edit_json(&layout_path(home, name), &mut |layout| {
        outcome = applayout::patch(layout)?;
        Ok(outcome == Patched::Changed)
    })?;
    say(match outcome {
        Patched::Changed => format!("layout: {name}: the Changes panel shows gitview now"),
        Patched::Already => format!("layout: {name}: the Changes panel already shows gitview"),
        Patched::NoSlot => {
            format!("layout: {name} has no `changes` block; left as it is (gitview is on its keys)")
        }
    });
    Ok(())
}

/// Take gitview's edits out of every personal layout. Layouts themselves
/// stay, and so does the one on screen.
fn uninstall_layouts(home: &Path, shipped: &HashSet<String>) -> Result<()> {
    for (name, _) in personal_layouts(home, shipped) {
        if edit_json(&layout_path(home, &name), &mut |layout| {
            Ok(applayout::restore(layout))
        })? {
            say(format!(
                "layout: {name}: the Changes panel shows what it showed before"
            ));
        }
    }
    Ok(())
}

/// Read, change, and replace `path` whole, unless it changed meanwhile (then
/// read again). `edit` returns whether it changed anything.
fn edit_json(path: &Path, edit: &mut dyn FnMut(&mut Value) -> Result<bool>) -> Result<bool> {
    for _ in 0..5 {
        let before = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        let mut value: Value = serde_json::from_slice(&before)
            .with_context(|| format!("{} is not valid JSON", path.display()))?;
        if !edit(&mut value)? {
            return Ok(false);
        }
        let name = path.file_name().context("no file name")?.to_string_lossy();
        let tmp = path.with_file_name(format!(".{name}.gitview.tmp"));
        fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&value)?))?;
        if fs::read(path)? != before {
            let _ = fs::remove_file(&tmp);
            continue;
        }
        fs::rename(&tmp, path)?;
        return Ok(true);
    }
    bail!("{} keeps changing; try again", path.display())
}

// ---- keys ---------------------------------------------------------------------------

/// Pure: add `ours` to `keys` where free. Returns (added, conflicts).
pub fn merge_keys(keys: &mut Value, ours: &Value) -> (Vec<String>, Vec<String>) {
    let (mut added, mut conflicts) = (Vec::new(), Vec::new());
    for (section, bindings) in ours.as_object().into_iter().flatten() {
        let Some(bindings) = bindings.as_object() else {
            continue;
        };
        if !keys.get(section).is_some_and(Value::is_object) {
            keys[section] = Value::Object(Map::new());
        }
        let target = keys[section].as_object_mut().expect("just made an object");
        for (key, action) in bindings {
            match target.get(key) {
                None => {
                    target.insert(key.clone(), action.clone());
                    added.push(format!("{section} {key} → {action}"));
                }
                Some(existing) if existing == action => {}
                Some(existing) => conflicts.push(format!(
                    "{section} {key} is {existing}; gitview's {action} is not bound (bind it to a free key yourself if you want it)"
                )),
            }
        }
    }
    (added, conflicts)
}

/// Pure: remove the bindings of `ours` that still have our action.
pub fn unmerge_keys(keys: &mut Value, ours: &Value) -> Vec<String> {
    let mut removed = Vec::new();
    for (section, bindings) in ours.as_object().into_iter().flatten() {
        let Some(target) = keys.get_mut(section).and_then(Value::as_object_mut) else {
            continue;
        };
        let had = removed.len();
        for (key, action) in bindings.as_object().into_iter().flatten() {
            if target.get(key) == Some(action) {
                target.shift_remove(key);
                removed.push(format!("{section} {key}"));
            }
        }
        // A section only our keys were in (install made it) goes too.
        if removed.len() > had && target.is_empty() {
            keys.as_object_mut().map(|k| k.shift_remove(section));
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applayout::tests::{herdr_nvim_patched, main};

    /// A throwaway app home: `installed.json` ships `main` and `terminals`.
    struct Home(PathBuf);

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn home(name: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("gitview-appsetup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("layouts")).unwrap();
        fs::write(
            dir.join("installed.json"),
            r#"{"layout:main": {"version": "0.1.12"}, "layout:terminals": {}}"#,
        )
        .unwrap();
        Home(dir)
    }

    /// Write a layout as the app does (sorted keys, two spaces, newline).
    fn put(home: &Home, name: &str, layout: &Value) -> Vec<u8> {
        let text = format!("{}\n", serde_json::to_string_pretty(layout).unwrap());
        fs::write(layout_path(&home.0, name), &text).unwrap();
        text.into_bytes()
    }

    fn read(home: &Home, name: &str) -> Vec<u8> {
        fs::read(layout_path(&home.0, name)).unwrap()
    }

    fn json_of(home: &Home, name: &str) -> Value {
        serde_json::from_slice(&read(home, name)).unwrap()
    }

    fn cloned(mut v: Value, from: &str) -> Value {
        v["cloned_from"] = json!({"name": from, "version": "0.1.12"});
        v
    }

    fn slot(v: &Value) -> &Value {
        &v["children"][1]["children"][0]["children"][1]["content"]
    }

    /// Install into `name`, check gitview is there, uninstall: the file is
    /// byte for byte what it was, and the shipped `main` was never touched.
    fn round_trip(home: &Home, name: &str) {
        let main_before = read(home, "main");
        let before = read(home, name);
        install_into(&home.0, name).unwrap();
        assert_eq!(
            slot(&json_of(home, name)),
            &json!({"mod": "gitview", "props": {"replaces": {"mod": "changes"}}})
        );
        install_into(&home.0, name).unwrap();
        uninstall_layouts(&home.0, &shipped_layouts(&home.0).unwrap()).unwrap();
        assert_eq!(
            String::from_utf8(read(home, name)).unwrap(),
            String::from_utf8(before).unwrap()
        );
        assert_eq!(read(home, "main"), main_before);
        // No temp files left behind.
        let left: Vec<_> = fs::read_dir(home.0.join("layouts"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn shipped_main_without_a_copy_is_cloned() {
        let h = home("shipped");
        put(&h, "main", &main());
        assert_eq!(
            choose(
                "main",
                shipped_layouts(&h.0).unwrap().contains("main"),
                &personal_layouts(&h.0, &shipped_layouts(&h.0).unwrap())
            )
            .unwrap(),
            Target::Clone("main".into())
        );
        // What layout.clone writes, then the round trip on it.
        put(&h, "main-copy", &cloned(main(), "main"));
        round_trip(&h, "main-copy");
    }

    #[test]
    fn an_existing_main_copy_is_used() {
        let h = home("copy");
        put(&h, "main", &main());
        put(&h, "main-copy", &cloned(main(), "main"));
        put(&h, "main-mods", &cloned(main(), "main"));
        assert_eq!(
            choose(
                "main",
                true,
                &personal_layouts(&h.0, &shipped_layouts(&h.0).unwrap())
            )
            .unwrap(),
            Target::Switch("main-copy".into()),
            "the app's default name wins"
        );
        round_trip(&h, "main-copy");
    }

    #[test]
    fn an_old_main_mods_counts_as_the_persons_copy() {
        let h = home("mods");
        put(&h, "main", &main());
        put(&h, "main-mods", &cloned(main(), "main"));
        put(&h, "mine", &json!({"id": "root", "type": "row"}));
        assert_eq!(
            choose(
                "main",
                true,
                &personal_layouts(&h.0, &shipped_layouts(&h.0).unwrap())
            )
            .unwrap(),
            Target::Switch("main-mods".into())
        );
        round_trip(&h, "main-mods");
        assert!(layout_path(&h.0, "main-mods").exists(), "never deleted");
    }

    #[test]
    fn a_personal_layout_is_edited_in_place() {
        let h = home("personal");
        put(&h, "main", &main());
        put(&h, "mine", &main());
        assert_eq!(
            choose(
                "mine",
                shipped_layouts(&h.0).unwrap().contains("mine"),
                &personal_layouts(&h.0, &shipped_layouts(&h.0).unwrap())
            )
            .unwrap(),
            Target::Edit("mine".into())
        );
        round_trip(&h, "mine");
    }

    #[test]
    fn a_layout_herdr_nvim_patched_keeps_its_files_slot_width_and_controller() {
        let h = home("herdr-nvim");
        put(&h, "main", &main());
        let theirs = cloned(herdr_nvim_patched(), "main");
        put(&h, "main-mods", &theirs);
        install_into(&h.0, "main-mods").unwrap();
        let mut ours = json_of(&h, "main-mods");
        let at = "/children/1/children/0/children/1/content";
        assert_eq!(ours.pointer(at).unwrap()["mod"], "gitview");
        *ours.pointer_mut(at).unwrap() = json!({"mod": "changes"});
        assert_eq!(ours, theirs, "nothing but the changes slot changed");
        uninstall_layouts(&h.0, &shipped_layouts(&h.0).unwrap()).unwrap();
        round_trip(&h, "main-mods");
    }

    #[test]
    fn a_layout_without_a_changes_slot_is_not_written() {
        let h = home("no-slot");
        put(&h, "main", &main());
        let bytes = put(
            &h,
            "bare",
            &json!({"children": [{"content": {"tab": "active"}, "id": "main", "type": "block"}],
                "id": "root", "type": "row"}),
        );
        install_into(&h.0, "bare").unwrap();
        assert_eq!(read(&h, "bare"), bytes);
        uninstall_layouts(&h.0, &shipped_layouts(&h.0).unwrap()).unwrap();
        assert_eq!(read(&h, "bare"), bytes);
    }

    #[test]
    fn hand_ordered_keys_keep_their_order() {
        let h = home("order");
        put(&h, "main", &main());
        let text = "{\n  \"type\": \"row\",\n  \"id\": \"root\",\n  \"children\": [\n    {\n      \"type\": \"block\",\n      \"id\": \"changes\",\n      \"content\": {\n        \"mod\": \"changes\"\n      }\n    }\n  ]\n}\n";
        fs::write(layout_path(&h.0, "mine"), text).unwrap();
        install_into(&h.0, "mine").unwrap();
        uninstall_layouts(&h.0, &shipped_layouts(&h.0).unwrap()).unwrap();
        assert_eq!(String::from_utf8(read(&h, "mine")).unwrap(), text);
    }

    #[test]
    fn several_copies_or_a_foreign_default_name_stop_with_a_message() {
        let copy = |n: &str| (n.to_string(), cloned(json!({}), "main"));
        assert!(choose("main", true, &[copy("main-mods"), copy("main-2")]).is_err());
        assert!(choose("main", true, &[("main-copy".into(), json!({}))]).is_err());
        assert_eq!(
            choose("terminals", true, &[copy("main-mods")]).unwrap(),
            Target::Clone("terminals".into())
        );
    }

    #[test]
    fn the_write_rule_rereads_and_gives_up_on_a_file_that_keeps_changing() {
        let h = home("race");
        let path = layout_path(&h.0, "mine");
        fs::write(&path, "{\"n\": 0}\n").unwrap();
        let mut tries = 0;
        let err = edit_json(&path, &mut |v| {
            tries += 1;
            // Someone writes the file between our read and our rename.
            fs::write(&path, format!("{{\"n\": {tries}}}\n")).unwrap();
            v["ours"] = json!(true);
            Ok(true)
        });
        assert!(err.is_err());
        assert_eq!(tries, 5);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"n\": 5}\n",
            "theirs kept"
        );
    }

    #[test]
    fn keys_merge_only_into_free_keys_and_unmerge_only_ours() {
        let mut keys = json!({"global": {"cmd-g": "mod.open:something", "cmd-t": "tab.new"}});
        let ours = json!({"global": {"cmd-shift-g": "mod.open:gitview-sidebar", "cmd-g": "mod.open:gitview-tab"}});
        let (added, conflicts) = merge_keys(&mut keys, &ours);
        assert_eq!(added.len(), 1);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(keys["global"]["cmd-g"], "mod.open:something");
        assert_eq!(merge_keys(&mut keys, &ours).0.len(), 0, "idempotent");
        assert_eq!(unmerge_keys(&mut keys, &ours), ["global cmd-shift-g"]);
        assert_eq!(
            keys,
            json!({"global": {"cmd-g": "mod.open:something", "cmd-t": "tab.new"}})
        );
    }

    #[test]
    fn unmerging_a_key_in_the_middle_keeps_the_order_of_the_rest() {
        let text = r#"{"global": {"cmd-a": "a", "cmd-alt-g": "mod.open:gitview-tui-tab", "cmd-b": "b", "cmd-c": "c"}}"#;
        let mut keys: Value = serde_json::from_str(text).unwrap();
        let retired: Value = serde_json::from_str(RETIRED_KEYS).unwrap();
        assert_eq!(unmerge_keys(&mut keys, &retired), ["global cmd-alt-g"]);
        assert_eq!(
            keys.to_string(),
            r#"{"global":{"cmd-a":"a","cmd-b":"b","cmd-c":"c"}}"#
        );
    }

    #[test]
    fn a_section_install_made_goes_again() {
        let ours = json!({"global": {"cmd-g": "mod.open:gitview-tab"}});
        let mut keys = json!({});
        merge_keys(&mut keys, &ours);
        assert_eq!(keys, json!({"global": {"cmd-g": "mod.open:gitview-tab"}}));
        unmerge_keys(&mut keys, &ours);
        assert_eq!(keys, json!({}));
    }
}
