use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::logx::{log, state_dir};

const PLUGIN_ID: &str = "adamchmara.gitview";

/// Everything needed to find and tear down an open view later, keyed on disk
/// by a hash of the repo root (see `views_dir`).
#[derive(Serialize, Deserialize)]
struct ViewState {
    repo: PathBuf,
    tab_id: String,
    preview_pane: String,
    list_pane: String,
    socket: PathBuf,
}

pub fn toggle() -> Result<()> {
    let repo = resolve_repo()?;
    match read_state(&repo) {
        Some(state) if tab_alive(&state.tab_id) => close_view(&repo, &state),
        Some(state) => {
            log("stale state file (tab gone) — cleaning up and reopening");
            cleanup(&repo, &state);
            open_view(&repo)
        }
        None => open_view(&repo),
    }
}

pub fn open() -> Result<()> {
    let repo = resolve_repo()?;
    match read_state(&repo) {
        Some(state) if tab_alive(&state.tab_id) => Ok(()), // already open
        Some(state) => {
            cleanup(&repo, &state);
            open_view(&repo)
        }
        None => open_view(&repo),
    }
}

pub fn close() -> Result<()> {
    let repo = resolve_repo()?;
    match read_state(&repo) {
        Some(state) => close_view(&repo, &state),
        None => Ok(()),
    }
}

/// Fire-and-forget `herdr-gitview close`, used by a pane to tear down the
/// whole view from inside (a pane can't close its own tab and keep running).
pub fn spawn_close() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("close").spawn();
    }
}

// ---- open/close ----------------------------------------------------------

fn open_view(repo: &Path) -> Result<()> {
    log(format!("opening view for {}", repo.display()));
    let views = views_dir();
    std::fs::create_dir_all(&views)?;
    let socket = views.join(format!("{}.sock", repo_hash(repo)));
    let _ = std::fs::remove_file(&socket);

    let preview_pane = open_pane(
        repo,
        &socket,
        "preview",
        &["--placement", "tab", "--no-focus"],
        &[],
    )?;
    let tab_id = pane_tab_id(&preview_pane)?;
    let list_pane = open_pane(
        repo,
        &socket,
        "list",
        &[
            "--placement",
            "split",
            "--direction",
            "right",
            "--target-pane",
            &preview_pane,
            "--focus",
        ],
        &[("GITVIEW_PREVIEW_PANE", &preview_pane)],
    )?;

    // `list_side = "left"`: panes are opened preview-left/list-right (the
    // preview must exist first so the list knows its pane id), then swapped.
    if crate::config::Config::load().list_side == crate::config::ListSide::Left {
        let _ = herdr_json(&[
            "pane",
            "swap",
            "--source-pane",
            &preview_pane,
            "--target-pane",
            &list_pane,
        ]);
    }

    let state = ViewState {
        repo: repo.to_path_buf(),
        tab_id,
        preview_pane,
        list_pane,
        socket,
    };
    std::fs::write(state_path(repo), serde_json::to_vec_pretty(&state)?)?;
    log("view opened");
    Ok(())
}

fn close_view(repo: &Path, state: &ViewState) -> Result<()> {
    log(format!("closing view (tab {})", state.tab_id));
    if herdr_json(&["tab", "close", &state.tab_id]).is_err() {
        // Tab-level close failed (already gone?) — try pane by pane.
        let _ = herdr_json(&["pane", "close", &state.preview_pane]);
        let _ = herdr_json(&["pane", "close", &state.list_pane]);
    }
    cleanup(repo, state);
    Ok(())
}

fn cleanup(repo: &Path, state: &ViewState) {
    let _ = std::fs::remove_file(&state.socket);
    let _ = std::fs::remove_file(state_path(repo));
}

/// Open one of our plugin panes and return its pane id. Prefers the id from
/// the command's own JSON reply; falls back to diffing `pane list`.
fn open_pane(
    repo: &Path,
    socket: &Path,
    entrypoint: &str,
    placement_args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<String> {
    let before = pane_ids().unwrap_or_default();

    let mut args: Vec<String> = vec![
        "plugin".into(),
        "pane".into(),
        "open".into(),
        "--plugin".into(),
        PLUGIN_ID.into(),
        "--entrypoint".into(),
        entrypoint.into(),
        "--cwd".into(),
        repo.display().to_string(),
        "--env".into(),
        format!("GITVIEW_REPO={}", repo.display()),
        "--env".into(),
        format!("GITVIEW_SOCKET={}", socket.display()),
    ];
    for (key, value) in extra_env {
        args.push("--env".into());
        args.push(format!("{key}={value}"));
    }
    for arg in placement_args {
        args.push((*arg).to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let reply = herdr_json(&arg_refs)?;
    if let Some(id) = find_str(&reply, "pane_id") {
        return Ok(id.to_string());
    }

    log("pane open reply had no pane_id — falling back to pane list diff");
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        let after = pane_ids().unwrap_or_default();
        if let Some(new_id) = after.iter().find(|id| !before.contains(*id)) {
            return Ok(new_id.clone());
        }
    }
    bail!("could not determine pane id for new {entrypoint} pane")
}

// ---- repo / state resolution ---------------------------------------------

/// Where was the shortcut pressed? Priority: explicit GITVIEW_REPO (panes,
/// tests) → cwd-ish paths inside HERDR_PLUGIN_CONTEXT_JSON (actions) → our
/// own cwd. First candidate that is inside a git repo wins.
fn resolve_repo() -> Result<PathBuf> {
    if let Some(repo) = std::env::var_os("GITVIEW_REPO") {
        return Ok(PathBuf::from(repo));
    }
    let mut candidates = Vec::new();
    if let Ok(raw) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        log(format!("context json: {raw}"));
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            collect_cwds(&value, &mut candidates);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    for dir in &candidates {
        if let Some(root) = git_toplevel(dir) {
            return Ok(root);
        }
    }
    bail!(
        "not inside a git repository (checked {} candidate dirs)",
        candidates.len()
    )
}

fn collect_cwds(value: &Value, out: &mut Vec<PathBuf>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if (key.ends_with("cwd") || key == "repo_root")
                    && let Some(s) = val.as_str()
                {
                    out.push(PathBuf::from(s));
                }
                collect_cwds(val, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_cwds(v, out)),
        _ => {}
    }
}

fn git_toplevel(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn views_dir() -> PathBuf {
    state_dir().join("views")
}

fn state_path(repo: &Path) -> PathBuf {
    views_dir().join(format!("{}.json", repo_hash(repo)))
}

fn read_state(repo: &Path) -> Option<ViewState> {
    let bytes = std::fs::read(state_path(repo)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// FNV-1a, hand-rolled: deterministic across runs and Rust versions, which
/// std's DefaultHasher does not guarantee.
fn repo_hash(repo: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in repo.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

// ---- herdr CLI ------------------------------------------------------------

fn tab_alive(tab_id: &str) -> bool {
    herdr_json(&["tab", "get", tab_id]).is_ok()
}

fn pane_tab_id(pane_id: &str) -> Result<String> {
    let reply = herdr_json(&["pane", "get", pane_id])?;
    find_str(&reply, "tab_id")
        .map(str::to_string)
        .context("no tab_id in `herdr pane get` reply")
}

fn pane_ids() -> Result<Vec<String>> {
    let reply = herdr_json(&["pane", "list"])?;
    let mut ids = Vec::new();
    collect_strs(&reply, "pane_id", &mut ids);
    Ok(ids)
}

/// Run a herdr CLI command and parse its JSON-envelope reply
/// (`{"id":"cli:…","result":{…}}`). Logs every invocation.
fn herdr_json(args: &[&str]) -> Result<Value> {
    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let out = Command::new(&bin)
        .args(args)
        .output()
        .with_context(|| format!("spawning {} {args:?}", bin.to_string_lossy()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log(format!(
        "herdr {args:?} -> {} | out: {} | err: {}",
        out.status,
        stdout.trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    ));
    if !out.status.success() {
        bail!(
            "herdr {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(serde_json::from_str(stdout.trim()).unwrap_or(Value::Null))
}

/// Depth-first search for the first string value under `key`.
fn find_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(map) => {
            if let Some(s) = map.get(key).and_then(Value::as_str) {
                return Some(s);
            }
            map.values().find_map(|v| find_str(v, key))
        }
        Value::Array(items) => items.iter().find_map(|v| find_str(v, key)),
        _ => None,
    }
}

/// Collect every string value under `key`, at any depth.
fn collect_strs(value: &Value, key: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key
                    && let Some(s) = v.as_str()
                {
                    out.push(s.to_string());
                }
                collect_strs(v, key, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_strs(v, key, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_cwds_matches_real_action_context_keys() {
        // Real shape captured from herdr 0.7.3 (see debug.log / contracts doc).
        let ctx: Value = serde_json::from_str(
            r#"{
                "workspace_id": "wF",
                "workspace_cwd": "/repo/from-workspace",
                "tab_id": "wF:t2",
                "focused_pane_id": "wF:p3",
                "focused_pane_cwd": "/repo/from-pane",
                "invocation_source": "cli"
            }"#,
        )
        .unwrap();
        let mut out = Vec::new();
        collect_cwds(&ctx, &mut out);
        out.sort();
        assert_eq!(
            out,
            vec![
                PathBuf::from("/repo/from-pane"),
                PathBuf::from("/repo/from-workspace"),
            ]
        );
    }

    #[test]
    fn collect_cwds_ignores_non_cwd_keys_and_recurses() {
        let ctx: Value =
            serde_json::from_str(r#"{"nested": [{"cwd": "/a", "label": "x"}], "repo_root": "/b"}"#)
                .unwrap();
        let mut out = Vec::new();
        collect_cwds(&ctx, &mut out);
        out.sort();
        assert_eq!(out, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }
}
