//! gitview's whole layout footprint in the Herdr app, put into a personal
//! layout and taken out again (the same model as herdr-nvim's
//! `src/app/layout_patch.rs`):
//!
//! * the `changes` slot (the side panel's Changes view) shows the `gitview`
//!   mod; what it showed before is kept in the block's `props.replaces`.
//!
//! Nothing else is touched: not the `files` slot (herdr-nvim's), not the
//! panel's width, not floating blocks, and every key stays in its place.
//! A layout without a `changes` slot is left as it is (gitview is then on
//! its keys only).

use anyhow::{Result, bail};
use serde_json::{Value, json};

pub const MOD: &str = "gitview";
pub const SLOT: &str = "changes";

/// What [`patch`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patched {
    /// The slot shows gitview now.
    Changed,
    /// It already did.
    Already,
    /// The layout has no `changes` slot; nothing changed.
    NoSlot,
}

fn find_mut<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if node.get("id").and_then(Value::as_str) == Some(id) {
        return Some(node);
    }
    node.get_mut("children")?
        .as_array_mut()?
        .iter_mut()
        .find_map(|child| find_mut(child, id))
}

/// Pure: point the `changes` slot at gitview, keeping what it showed.
pub fn patch(layout: &mut Value) -> Result<Patched> {
    if !layout.is_object() {
        bail!("the layout is not an object");
    }
    let Some(slot) = find_mut(layout, SLOT) else {
        return Ok(Patched::NoSlot);
    };
    if slot.pointer("/content/mod").and_then(Value::as_str) == Some(MOD) {
        return Ok(Patched::Already);
    }
    let old = slot.get("content").cloned().unwrap_or(Value::Null);
    slot["content"] = json!({ "mod": MOD, "props": { "replaces": old } });
    Ok(Patched::Changed)
}

/// Pure: put back what every gitview block of ours replaced (blocks showing
/// gitview with a `props.replaces`, which only [`patch`] writes). Returns
/// whether anything changed.
pub fn restore(layout: &mut Value) -> bool {
    fn walk(node: &mut Value, changed: &mut bool) {
        let ours = node.pointer("/content/mod").and_then(Value::as_str) == Some(MOD);
        if let Some(old) = ours
            .then(|| node.pointer("/content/props/replaces").cloned())
            .flatten()
        {
            node["content"] = if old.is_null() {
                json!({ "mod": SLOT })
            } else {
                old
            };
            *changed = true;
        }
        for key in ["children", "floating"] {
            if let Some(list) = node.get_mut(key).and_then(Value::as_array_mut) {
                for child in list {
                    walk(child, changed);
                }
            }
        }
    }
    let mut changed = false;
    walk(layout, &mut changed);
    changed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The side panel of Herdr's shipped `main` (abridged), as the app
    /// writes it: sorted keys.
    pub(crate) fn main() -> Value {
        json!({"children": [
            {"content": {"tab": "active"}, "id": "main", "type": "block"},
            {"children": [{"active": "files", "children": [
                {"content": {"mod": "files"}, "id": "files", "type": "block"},
                {"content": {"mod": "changes"}, "id": "changes", "type": "block"}],
              "id": "panel-body", "type": "stack"}],
             "hidden": true, "id": "panel", "size": {"px": 368}, "type": "column"}],
          "id": "root", "type": "row"})
    }

    /// `main` after herdr-nvim's install: its tree in `files`, the panel
    /// at 300 px, and its floating controller.
    pub(crate) fn herdr_nvim_patched() -> Value {
        let mut v = main();
        v["children"][1]["size"] = json!({"px": 300});
        v["children"][1]["children"][0]["children"][0]["content"] = json!({"mod": "herdr-nvim",
            "props": {"panel_width": 300, "panel_width_was": 368, "replaces": {"mod": "files"}}});
        v["floating"] = json!([{"align": "end", "content": {"mod": "herdr-nvim",
            "props": {"role": "sync"}}, "edge": "bottom", "id": "herdr-nvim-sync"}]);
        v
    }

    fn slot(v: &Value) -> &Value {
        &v["children"][1]["children"][0]["children"][1]["content"]
    }

    #[test]
    fn patch_keeps_what_the_slot_showed_and_is_idempotent() {
        let mut v = main();
        assert_eq!(patch(&mut v).unwrap(), Patched::Changed);
        assert_eq!(
            slot(&v),
            &json!({"mod": "gitview", "props": {"replaces": {"mod": "changes"}}})
        );
        let once = v.clone();
        assert_eq!(patch(&mut v).unwrap(), Patched::Already);
        assert_eq!(v, once);
        assert!(restore(&mut v));
        assert_eq!(v, main());
        assert!(!restore(&mut v));
    }

    #[test]
    fn herdr_nvims_files_slot_panel_width_and_controller_are_left_alone() {
        let mut v = herdr_nvim_patched();
        patch(&mut v).unwrap();
        let mut expect = herdr_nvim_patched();
        expect["children"][1]["children"][0]["children"][1]["content"] =
            json!({"mod": "gitview", "props": {"replaces": {"mod": "changes"}}});
        assert_eq!(v, expect);
        assert!(restore(&mut v));
        assert_eq!(v, herdr_nvim_patched());
    }

    #[test]
    fn a_layout_without_a_changes_slot_is_left_as_it_is() {
        let original = json!({"children": [{"content": {"tab": "active"}, "id": "main",
            "type": "block"}], "id": "root", "type": "row"});
        let mut v = original.clone();
        assert_eq!(patch(&mut v).unwrap(), Patched::NoSlot);
        assert_eq!(v, original);
        assert!(!restore(&mut v));
    }

    #[test]
    fn restore_leaves_gitview_blocks_it_did_not_make() {
        // The person put gitview in a block of their own (no `replaces`).
        let mut v = json!({"children": [{"content": {"mod": "gitview"}, "id": "mine",
            "type": "block"}], "id": "root", "type": "row"});
        let before = v.clone();
        assert!(!restore(&mut v));
        assert_eq!(v, before);
    }

    #[test]
    fn an_old_layout_slot_install_is_taken_out_too() {
        // What herdr-app/layout_slot.py wrote into `main-mods`.
        let mut v = herdr_nvim_patched();
        v["children"][1]["children"][0]["children"][1]["content"] =
            json!({"mod": "gitview", "props": {"replaces": {"mod": "changes"}}});
        assert_eq!(patch(&mut v).unwrap(), Patched::Already);
        assert!(restore(&mut v));
        assert_eq!(v, herdr_nvim_patched());
    }
}
