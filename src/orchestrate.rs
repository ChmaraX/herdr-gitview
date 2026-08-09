use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::{Dir, MoveStep};
use crate::logx::{log, state_dir};

const PLUGIN_ID: &str = "chmarax.gitview";

/// How the view is laid out.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// Full-height sidebar in the invoking tab (existing layout squeezed
    /// left).
    #[default]
    Sidebar,
    /// The view's own dedicated tab (the original behavior).
    Tab,
}

/// Everything needed to find and tear down an open view later.
///
/// Disk layout under `views_dir`:
/// - Tab mode (singleton per repo): `{repo_hash}.tab.json` + `.tab.sock`
/// - Sidebar mode (one per host tab): `{repo_hash}.{tab_key}.json` + matching sock
///   (`tab_key` is 8 hex chars — 16 would push `.sock` past macOS `sun_path`)
/// - Legacy single-file `{repo_hash}.json` is still read for migration.
///
/// For `Sidebar` mode `tab_id` is the *host* tab the sidebar lives in; for
/// `Tab` mode it is the view's own tab.
#[derive(Serialize, Deserialize, Clone)]
struct ViewState {
    repo: PathBuf,
    tab_id: String,
    preview_pane: String,
    list_pane: String,
    socket: PathBuf,
    #[serde(default)]
    mode: ViewMode,
    /// Tab mode: the tab the toggle was invoked from, refocused on close
    /// (herdr's own fallback is "previous tab in order", not "where you
    /// came from").
    #[serde(default)]
    origin_tab: Option<String>,
}

/// Written just before evacuating panes to a parking tab, so an interrupted
/// open can move them back on the next toggle instead of stranding them.
#[derive(Serialize, Deserialize)]
struct RecoverState {
    tab: String,
    parking_tab: String,
    parked: Vec<String>,
    steps: Vec<MoveStep>,
}

/// Sidebar toggle (`toggle` action): open/close a sidebar in the *current*
/// tab only. Never jumps to another tab's gitview — each tab can have its
/// own instance.
pub fn toggle() -> Result<()> {
    let repo = resolve_repo()?;
    let tab_id = current_tab_id()?;
    match read_sidebar_state(&repo, &tab_id) {
        Some(state) if view_alive(&state) => close_view(&repo, &state),
        Some(state) => {
            log("stale sidebar state (panes gone) — cleaning up and reopening");
            cleanup(&repo, &state);
            open_sidebar_view(&repo)
        }
        None => open_sidebar_view(&repo),
    }
}

/// Dedicated-tab toggle (`toggle-tab` action): one view tab per repo.
/// Pressing again from another tab focuses that view; from inside it closes.
pub fn toggle_tab() -> Result<()> {
    let repo = resolve_repo()?;
    match read_tab_state(&repo) {
        Some(state) if view_alive(&state) && !invoked_from(&state.tab_id) => {
            log(format!("focusing existing view tab {}", state.tab_id));
            herdr_json(&["tab", "focus", &state.tab_id])?;
            Ok(())
        }
        Some(state) if view_alive(&state) => close_view(&repo, &state),
        Some(state) => {
            log("stale tab state (panes gone) — cleaning up and reopening");
            cleanup(&repo, &state);
            open_tab_view(&repo)
        }
        None => open_tab_view(&repo),
    }
}

pub fn open() -> Result<()> {
    let repo = resolve_repo()?;
    let tab_id = current_tab_id().unwrap_or_default();
    match read_sidebar_state(&repo, &tab_id) {
        Some(state) if view_alive(&state) => Ok(()), // already open here
        Some(state) => {
            cleanup(&repo, &state);
            open_sidebar_view(&repo)
        }
        None => open_sidebar_view(&repo),
    }
}

pub fn close() -> Result<()> {
    let repo = resolve_repo()?;
    // Pane-initiated close inherits GITVIEW_SOCKET — close that instance only.
    if let Ok(socket) = std::env::var("GITVIEW_SOCKET")
        && let Some(state) = find_state_by_socket(&repo, Path::new(&socket))
    {
        return close_view(&repo, &state);
    }
    // CLI / no socket: tear down every alive view for this repo.
    for state in all_states(&repo) {
        if view_alive(&state) {
            close_view(&repo, &state)?;
        } else {
            cleanup(&repo, &state);
        }
    }
    Ok(())
}

/// Fire-and-forget `herdr-gitview close`, used by a pane to tear down the
/// whole view from inside (a pane can't close its own tab and keep running).
pub fn spawn_close() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("close").spawn();
    }
}

// ---- open/close ----------------------------------------------------------

/// Open the view in its own dedicated tab (the original behavior):
/// preview + list panes, list at `list_width_percent` of the tab.
fn open_tab_view(repo: &Path) -> Result<()> {
    log(format!("opening tab view for {}", repo.display()));
    let origin_tab = view_context().ok().map(|ctx| ctx.tab);
    let cfg = crate::config::Config::load();
    let views = views_dir();
    std::fs::create_dir_all(&views)?;
    let socket = tab_socket_path(repo);
    let _ = std::fs::remove_file(&socket);

    let preview_pane = open_pane(
        repo,
        &socket,
        "preview",
        &["--placement", "tab", "--no-focus"],
        &[],
    )?;
    let reply = herdr_json(&["pane", "get", &preview_pane])?;
    let tab_id = find_str(&reply, "tab_id")
        .map(str::to_string)
        .context("no tab_id in `herdr pane get` reply")?;
    // Label the tab so the tab bar reads as ours, not as a generic pane title.
    let _ = herdr_json(&["tab", "rename", &tab_id, "gitview"]);

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

    // Cut the tab into preview/list at `list_width_percent` (the split
    // lands at 50/50; `pane resize` sets the real size).
    let list_frac = f64::from(cfg.list_width_percent) / 100.0;
    if cfg.list_side == crate::config::ListSide::Left {
        let _ = herdr_json(&[
            "pane",
            "swap",
            "--source-pane",
            &preview_pane,
            "--target-pane",
            &list_pane,
        ]);
        if list_frac < 0.5 {
            resize_pane(&preview_pane, "left", 0.5 - list_frac);
        } else {
            resize_pane(&list_pane, "right", list_frac - 0.5);
        }
    } else if list_frac < 0.5 {
        resize_pane(&preview_pane, "right", 0.5 - list_frac);
    } else {
        resize_pane(&list_pane, "left", list_frac - 0.5);
    }

    let state = ViewState {
        repo: repo.to_path_buf(),
        tab_id,
        preview_pane,
        list_pane,
        socket,
        mode: ViewMode::Tab,
        origin_tab,
    };
    write_state(repo, &state)?;
    log("tab view opened");
    Ok(())
}

/// Open the view as a full-height sidebar in the *current* tab: the whole
/// existing layout is squeezed to the left (via a parking tab + rebuild,
/// herdr-nvim style) and preview+list take the right `view_width_percent`.
fn open_sidebar_view(repo: &Path) -> Result<()> {
    log(format!("opening view for {}", repo.display()));
    let ctx = view_context()?;
    replay_recovery(repo, &ctx.tab); // best-effort: undo an interrupted earlier open

    let cfg = crate::config::Config::load();
    let views = views_dir();
    std::fs::create_dir_all(&views)?;
    let socket = sidebar_socket_path(repo, &ctx.tab);
    let _ = std::fs::remove_file(&socket);

    let rects = pane_rects(&ctx.focused_pane)?;
    let plan = crate::layout::plan_rebuild(&rects)?;

    // Evacuate every non-anchor pane to a hidden parking tab so the sidebar
    // split spans the full tab height, recording how to put them back first.
    let mut parking: Option<(String, String)> = None; // (tab, placeholder pane)
    if rects.len() > 1 {
        let (parking_tab, placeholder) = create_parking_tab(&ctx.workspace)?;
        let parked: Vec<String> = rects
            .iter()
            .filter(|r| r.pane_id != plan.anchor)
            .map(|r| r.pane_id.clone())
            .collect();
        let recover = RecoverState {
            tab: ctx.tab.clone(),
            parking_tab: parking_tab.clone(),
            parked: parked.clone(),
            steps: plan.steps.clone(),
        };
        std::fs::write(
            recover_path(repo, &ctx.tab),
            serde_json::to_vec_pretty(&recover)?,
        )?;
        for pane in &parked {
            move_pane(pane, &parking_tab, Dir::Right, None, None)?;
        }
        parking = Some((parking_tab, placeholder));
    }

    // Sidebar region: split the anchor, then re-cut to the configured width.
    // `plugin pane open` has no --ratio and herdr refuses same-tab `pane
    // move` re-splits (reason: same_tab), so the split lands at 50/50 and a
    // `pane resize` (amount = ratio delta of that split) sets the real size.
    let sidebar_frac = f64::from(cfg.view_width_percent) / 100.0;
    let preview_pane = open_pane(
        repo,
        &socket,
        "preview",
        &[
            "--placement",
            "split",
            "--direction",
            "right",
            "--target-pane",
            &plan.anchor,
            "--no-focus",
        ],
        &[],
    )?;
    // Sidebar currently holds 0.5 of the tab; bring it to `sidebar_frac`.
    if sidebar_frac < 0.5 {
        resize_pane(&plan.anchor, "right", 0.5 - sidebar_frac);
    } else {
        resize_pane(&preview_pane, "left", sidebar_frac - 0.5);
    }

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

    // Cut the sidebar region into preview/list at `list_width_percent`.
    let list_frac = f64::from(cfg.list_width_percent) / 100.0;
    if cfg.list_side == crate::config::ListSide::Left {
        // Swap contents so the list sits left, then size the split.
        let _ = herdr_json(&[
            "pane",
            "swap",
            "--source-pane",
            &preview_pane,
            "--target-pane",
            &list_pane,
        ]);
        if list_frac < 0.5 {
            resize_pane(&preview_pane, "left", 0.5 - list_frac);
        } else {
            resize_pane(&list_pane, "right", list_frac - 0.5);
        }
    } else if list_frac < 0.5 {
        resize_pane(&preview_pane, "right", 0.5 - list_frac);
    } else {
        resize_pane(&list_pane, "left", list_frac - 0.5);
    }

    // Rebuild the original layout inside the anchor's (now squeezed) slot.
    for step in &plan.steps {
        move_pane(
            &step.pane,
            &ctx.tab,
            step.dir,
            Some(&step.target),
            Some(step.ratio),
        )?;
    }
    if let Some((_, placeholder)) = parking {
        let _ = herdr_json(&["pane", "close", &placeholder]);
    }
    let _ = std::fs::remove_file(recover_path(repo, &ctx.tab));
    let _ = herdr_json(&["plugin", "pane", "focus", &list_pane]);

    let state = ViewState {
        repo: repo.to_path_buf(),
        tab_id: ctx.tab,
        preview_pane,
        list_pane,
        socket,
        mode: ViewMode::Sidebar,
        origin_tab: None,
    };
    write_state(repo, &state)?;
    log("view opened");
    Ok(())
}

/// Sidebar: closing the panes hands their space back to the squeezed
/// layout. Tab: close the whole dedicated tab (pane-by-pane fallback).
fn close_view(repo: &Path, state: &ViewState) -> Result<()> {
    log(format!("closing view (tab {})", state.tab_id));
    match state.mode {
        ViewMode::Tab => {
            if herdr_json(&["tab", "close", &state.tab_id]).is_err() {
                let _ = herdr_json(&["pane", "close", &state.list_pane]);
                let _ = herdr_json(&["pane", "close", &state.preview_pane]);
            }
            // Return to where the view was opened from, not to herdr's
            // "previous tab in order" fallback.
            if let Some(origin) = &state.origin_tab {
                let _ = herdr_json(&["tab", "focus", origin]);
            }
        }
        ViewMode::Sidebar => {
            let _ = herdr_json(&["pane", "close", &state.list_pane]);
            let _ = herdr_json(&["pane", "close", &state.preview_pane]);
        }
    }
    cleanup(repo, state);
    Ok(())
}

fn cleanup(repo: &Path, state: &ViewState) {
    let _ = std::fs::remove_file(&state.socket);
    let _ = std::fs::remove_file(state_file_for(repo, state));
    // Drop legacy single-file state only when it describes this same view.
    if let Some(legacy) = read_state_file(&legacy_state_path(repo))
        && legacy.tab_id == state.tab_id
        && legacy.preview_pane == state.preview_pane
    {
        let _ = std::fs::remove_file(legacy_state_path(repo));
    }
}

/// The view is alive as long as either of its panes still exists.
fn view_alive(state: &ViewState) -> bool {
    pane_alive(&state.preview_pane) || pane_alive(&state.list_pane)
}

/// If an earlier open crashed mid-evacuation, move the parked panes back to
/// their recorded positions and drop the parking tab. Best-effort: a pane
/// that no longer exists is skipped.
fn replay_recovery(repo: &Path, tab_id: &str) {
    let path = recover_path(repo, tab_id);
    let Ok(bytes) = std::fs::read(&path) else {
        // Legacy per-repo recover file (pre multi-sidebar).
        let legacy = views_dir().join(format!("{}.recover.json", repo_hash(repo)));
        let Ok(bytes) = std::fs::read(&legacy) else {
            return;
        };
        replay_recovery_bytes(&bytes, &legacy);
        return;
    };
    replay_recovery_bytes(&bytes, &path);
}

fn replay_recovery_bytes(bytes: &[u8], path: &Path) {
    let Ok(rec) = serde_json::from_slice::<RecoverState>(bytes) else {
        let _ = std::fs::remove_file(path);
        return;
    };
    log("replaying interrupted-open recovery");
    for step in &rec.steps {
        if !rec.parked.contains(&step.pane) || !pane_alive(&step.pane) {
            continue;
        }
        let _ = move_pane(
            &step.pane,
            &rec.tab,
            step.dir,
            Some(&step.target),
            Some(step.ratio),
        );
    }
    let _ = herdr_json(&["tab", "close", &rec.parking_tab]);
    let _ = std::fs::remove_file(path);
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

/// Was the action invoked from inside the given tab? Falls back to `true`
/// (→ close behavior) when no invocation context is available.
fn invoked_from(tab_id: &str) -> bool {
    let Ok(raw) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") else {
        return true;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return true;
    };
    match find_str(&value, "tab_id") {
        Some(ctx_tab) => ctx_tab == tab_id,
        None => true,
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

fn legacy_state_path(repo: &Path) -> PathBuf {
    views_dir().join(format!("{}.json", repo_hash(repo)))
}

fn tab_state_path(repo: &Path) -> PathBuf {
    views_dir().join(format!("{}.tab.json", repo_hash(repo)))
}

fn sidebar_state_path(repo: &Path, tab_id: &str) -> PathBuf {
    views_dir().join(format!("{}.{}.json", repo_hash(repo), tab_key(tab_id)))
}

fn tab_socket_path(repo: &Path) -> PathBuf {
    views_dir().join(format!("{}.tab.sock", repo_hash(repo)))
}

fn sidebar_socket_path(repo: &Path, tab_id: &str) -> PathBuf {
    views_dir().join(format!("{}.{}.sock", repo_hash(repo), tab_key(tab_id)))
}

fn recover_path(repo: &Path, tab_id: &str) -> PathBuf {
    views_dir().join(format!(
        "{}.{}.recover.json",
        repo_hash(repo),
        tab_key(tab_id)
    ))
}

fn state_file_for(repo: &Path, state: &ViewState) -> PathBuf {
    match state.mode {
        ViewMode::Tab => tab_state_path(repo),
        ViewMode::Sidebar => sidebar_state_path(repo, &state.tab_id),
    }
}

fn write_state(repo: &Path, state: &ViewState) -> Result<()> {
    let path = state_file_for(repo, state);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

fn read_state_file(path: &Path) -> Option<ViewState> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Sidebar state for a specific host tab (new path, then legacy if it matches).
fn read_sidebar_state(repo: &Path, tab_id: &str) -> Option<ViewState> {
    if tab_id.is_empty() {
        return None;
    }
    if let Some(state) = read_state_file(&sidebar_state_path(repo, tab_id)) {
        return Some(state);
    }
    let legacy = read_state_file(&legacy_state_path(repo))?;
    (legacy.mode == ViewMode::Sidebar && legacy.tab_id == tab_id).then_some(legacy)
}

/// Dedicated-tab view state (new path, then legacy Tab-mode file).
fn read_tab_state(repo: &Path) -> Option<ViewState> {
    if let Some(state) = read_state_file(&tab_state_path(repo)) {
        return Some(state);
    }
    let legacy = read_state_file(&legacy_state_path(repo))?;
    (legacy.mode == ViewMode::Tab).then_some(legacy)
}

fn all_states(repo: &Path) -> Vec<ViewState> {
    let mut out = Vec::new();
    let prefix = repo_hash(repo);
    let Ok(entries) = std::fs::read_dir(views_dir()) else {
        return out;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) || !name.ends_with(".json") {
            continue;
        }
        if name.ends_with(".recover.json") {
            continue;
        }
        if let Some(state) = read_state_file(&path) {
            out.push(state);
        }
    }
    out
}

fn find_state_by_socket(repo: &Path, socket: &Path) -> Option<ViewState> {
    all_states(repo).into_iter().find(|s| s.socket == socket)
}

fn current_tab_id() -> Result<String> {
    Ok(view_context()?.tab)
}

/// FNV-1a, hand-rolled: deterministic across runs and Rust versions, which
/// std's DefaultHasher does not guarantee.
fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn repo_hash(repo: &Path) -> String {
    format!("{:016x}", fnv1a(&repo.to_string_lossy()))
}

/// Compact tab id for filenames. Full 16-hex + `{repo}.….sock` exceeds macOS
/// `sockaddr_un.sun_path` (104 incl. NUL) under the default plugin state dir.
fn tab_key(tab_id: &str) -> String {
    format!("{:08x}", fnv1a(tab_id) as u32)
}

// ---- herdr CLI ------------------------------------------------------------

fn pane_alive(pane_id: &str) -> bool {
    herdr_json(&["pane", "get", pane_id]).is_ok()
}

/// Where the sidebar should open: the invoking workspace/tab/focused pane.
/// Prefers the action's context JSON; falls back to the pane env vars (when
/// invoked from inside one of our own panes).
struct ViewCtx {
    workspace: String,
    tab: String,
    focused_pane: String,
}

fn view_context() -> Result<ViewCtx> {
    let ctx_json = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let from_ctx = |key: &str| {
        ctx_json
            .as_ref()
            .and_then(|v| find_str(v, key))
            .map(str::to_string)
    };

    let focused_pane = from_ctx("focused_pane_id")
        .or_else(|| std::env::var("HERDR_PANE_ID").ok())
        .context("no focused pane in context or HERDR_PANE_ID")?;
    let workspace = from_ctx("workspace_id")
        .or_else(|| std::env::var("HERDR_WORKSPACE_ID").ok())
        .context("no workspace in context or HERDR_WORKSPACE_ID")?;
    let tab = match from_ctx("tab_id") {
        Some(tab) => tab,
        None => {
            let reply = herdr_json(&["pane", "get", &focused_pane])?;
            find_str(&reply, "tab_id")
                .map(str::to_string)
                .context("no tab_id in `herdr pane get` reply")?
        }
    };
    Ok(ViewCtx {
        workspace,
        tab,
        focused_pane,
    })
}

/// The pane rectangles of the tab containing `pane`.
fn pane_rects(pane: &str) -> Result<Vec<crate::layout::PaneRect>> {
    let reply = herdr_json(&["pane", "layout", "--pane", pane])?;
    crate::layout::parse_pane_rects(&reply)
}

/// Create a hidden parking tab; returns (tab_id, placeholder root pane id).
fn create_parking_tab(workspace: &str) -> Result<(String, String)> {
    let reply = herdr_json(&["tab", "create", "--workspace", workspace, "--no-focus"])?;
    let tab = find_str(&reply, "tab_id")
        .map(str::to_string)
        .context("no tab_id in `herdr tab create` reply")?;
    let placeholder = find_str(&reply, "pane_id")
        .map(str::to_string)
        .context("no root pane_id in `herdr tab create` reply")?;
    Ok((tab, placeholder))
}

/// Grow `pane` toward `dir` ("left"/"right"/"up"/"down") by `amount`,
/// a ratio delta of the split that owns that edge. Best-effort.
fn resize_pane(pane: &str, dir: &str, amount: f64) {
    if amount.abs() < 0.01 {
        return;
    }
    let _ = herdr_json(&[
        "pane",
        "resize",
        "--pane",
        pane,
        "--direction",
        dir,
        "--amount",
        &amount.to_string(),
    ]);
}

/// `herdr pane move`: re-split `pane` into `tab`, optionally as a split of
/// `target` where the target keeps `ratio` of the region.
fn move_pane(
    pane: &str,
    tab: &str,
    dir: Dir,
    target: Option<&str>,
    ratio: Option<f64>,
) -> Result<()> {
    let ratio_s = ratio.map(|r| r.to_string());
    let mut args = vec![
        "pane",
        "move",
        pane,
        "--tab",
        tab,
        "--split",
        dir.as_cli_arg(),
    ];
    if let Some(target) = target {
        args.extend(["--target-pane", target]);
    }
    if let Some(r) = ratio_s.as_deref() {
        args.extend(["--ratio", r]);
    }
    args.push("--no-focus");
    herdr_json(&args)?;
    Ok(())
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
    use std::sync::Mutex;

    // state_dir() reads HERDR_PLUGIN_STATE_DIR; serialize tests that set it.
    static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_state_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        let _guard = STATE_DIR_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "gitview-orch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("views")).unwrap();
        // SAFETY: guarded by STATE_DIR_LOCK; restored before unlock.
        unsafe {
            std::env::set_var("HERDR_PLUGIN_STATE_DIR", &dir);
        }
        let result = f(&dir);
        unsafe {
            std::env::remove_var("HERDR_PLUGIN_STATE_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn sample_state(repo: &Path, tab_id: &str, mode: ViewMode, socket: PathBuf) -> ViewState {
        ViewState {
            repo: repo.to_path_buf(),
            tab_id: tab_id.into(),
            preview_pane: "p1".into(),
            list_pane: "p2".into(),
            socket,
            mode,
            origin_tab: None,
        }
    }

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

    #[test]
    fn sidebar_state_is_per_tab_not_shared_across_tabs() {
        with_temp_state_dir(|_| {
            let repo = Path::new("/repo");
            let a = sample_state(
                repo,
                "tab-a",
                ViewMode::Sidebar,
                sidebar_socket_path(repo, "tab-a"),
            );
            write_state(repo, &a).unwrap();

            assert!(read_sidebar_state(repo, "tab-a").is_some());
            // Another tab must not see tab-a's sidebar — that's the bug fix.
            assert!(read_sidebar_state(repo, "tab-b").is_none());
        });
    }

    #[test]
    fn tab_state_is_singleton_and_ignores_sidebars() {
        with_temp_state_dir(|_| {
            let repo = Path::new("/repo");
            let sidebar = sample_state(
                repo,
                "host-tab",
                ViewMode::Sidebar,
                sidebar_socket_path(repo, "host-tab"),
            );
            write_state(repo, &sidebar).unwrap();
            assert!(read_tab_state(repo).is_none());

            let tab = sample_state(repo, "view-tab", ViewMode::Tab, tab_socket_path(repo));
            write_state(repo, &tab).unwrap();
            let got = read_tab_state(repo).unwrap();
            assert_eq!(got.tab_id, "view-tab");
            assert_eq!(got.mode, ViewMode::Tab);
        });
    }

    #[test]
    fn legacy_sidebar_state_only_matches_its_host_tab() {
        with_temp_state_dir(|_| {
            let repo = Path::new("/repo");
            let state = sample_state(
                repo,
                "tab-a",
                ViewMode::Sidebar,
                views_dir().join(format!("{}.sock", repo_hash(repo))),
            );
            std::fs::write(
                legacy_state_path(repo),
                serde_json::to_vec_pretty(&state).unwrap(),
            )
            .unwrap();

            assert!(read_sidebar_state(repo, "tab-a").is_some());
            assert!(read_sidebar_state(repo, "tab-b").is_none());
            assert!(read_tab_state(repo).is_none());
        });
    }

    #[test]
    fn find_state_by_socket_picks_the_right_instance() {
        with_temp_state_dir(|_| {
            let repo = Path::new("/repo");
            let a = sample_state(
                repo,
                "tab-a",
                ViewMode::Sidebar,
                sidebar_socket_path(repo, "tab-a"),
            );
            let b = sample_state(
                repo,
                "tab-b",
                ViewMode::Sidebar,
                sidebar_socket_path(repo, "tab-b"),
            );
            write_state(repo, &a).unwrap();
            write_state(repo, &b).unwrap();

            let found = find_state_by_socket(repo, &b.socket).unwrap();
            assert_eq!(found.tab_id, "tab-b");
        });
    }

    #[test]
    fn sidebar_socket_fits_macos_sun_path_and_binds() {
        // macOS sockaddr_un.sun_path is 104 bytes including the NUL.
        const MAX_SOCK_PATH: usize = 103;
        let repo = Path::new("/Users/adamchmara/projects/herdr-gitview");
        let tab = "w22:t8";
        let prefix = "/Users/adamchmara/.local/state/herdr/plugins/chmarax.gitview/views";
        let new_path = format!("{prefix}/{}.{}.sock", repo_hash(repo), tab_key(tab));
        let old_path = format!("{prefix}/{}.{:016x}.sock", repo_hash(repo), fnv1a(tab));
        assert!(
            new_path.len() <= MAX_SOCK_PATH,
            "sidebar socket too long: {new_path} ({} bytes)",
            new_path.len()
        );
        // Sanity: the pre-fix 16-hex tab key is what broke preview IPC.
        assert!(
            old_path.len() > MAX_SOCK_PATH,
            "expected old naming to exceed limit, got {} bytes",
            old_path.len()
        );

        // Bind under a short dir — macOS temp paths are themselves long enough
        // to blow the limit even with an 8-hex tab key.
        let dir = std::env::temp_dir().join("gv");
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join(format!("{}.{}.sock", repo_hash(repo), tab_key(tab)));
        assert!(
            sock.to_string_lossy().len() <= MAX_SOCK_PATH,
            "short-dir bind path still too long: {} ({} bytes)",
            sock.display(),
            sock.to_string_lossy().len()
        );
        let _ = std::fs::remove_file(&sock);
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        drop(listener);
        let _ = std::fs::remove_file(&sock);
    }
}
