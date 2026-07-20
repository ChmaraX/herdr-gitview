//! Shared helper for herdr floating popup panes (≥0.7.4): spawn a plugin
//! popup entrypoint and poll for its answer file. Used for confirms
//! (`ask`), note input (`annotate`), and the agent picker (`pick_agent`).

use std::path::PathBuf;
use std::process::Command;

/// Does this herdr support popup plugin panes (≥0.7.4)?
pub fn supported() -> bool {
    if std::env::var_os("HERDR_PANE_ID").is_none() {
        return false; // standalone
    }
    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let Ok(out) = Command::new(bin).arg("--version").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(ver) = text.split_whitespace().last() else {
        return false;
    };
    let mut parts = ver.split('.').filter_map(|p| p.parse::<u32>().ok());
    let (maj, min, pat) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    (maj, min, pat) >= (0, 7, 4)
}

/// Open a popup pane running `entrypoint` with extra env vars, sized
/// `width`×`height` cells. Returns the answer-file path to poll, or None
/// when the popup could not be opened (caller decides the fallback).
/// The `GITVIEW_ANSWER_FILE` env var is added automatically.
pub fn spawn(
    entrypoint: &str,
    envs: &[(String, String)],
    width: u16,
    height: u16,
) -> Option<PathBuf> {
    let answer = PathBuf::from(std::env::var_os("GITVIEW_SOCKET")?)
        .with_extension(format!("{entrypoint}.answer"));
    let _ = std::fs::remove_file(&answer);

    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let mut cmd = Command::new(bin);
    cmd.args(["plugin", "pane", "open", "--plugin", "adamchmara.gitview"])
        .args([
            "--entrypoint",
            entrypoint,
            "--placement",
            "popup",
            "--focus",
        ])
        .args([
            "--width",
            &width.to_string(),
            "--height",
            &height.to_string(),
        ])
        .arg("--env")
        .arg(format!("GITVIEW_ANSWER_FILE={}", answer.display()));
    for (key, value) in envs {
        cmd.arg("--env").arg(format!("{key}={value}"));
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        crate::logx::log(format!(
            "popup {entrypoint} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
        return None;
    }
    Some(answer)
}

/// Non-blocking: if the answer file appeared, consume it and return its
/// content (trimmed). The popup writes via tmp+rename, so a read never sees
/// a partial answer.
pub fn poll(pending: &mut Option<PathBuf>) -> Option<String> {
    let path = pending.as_ref()?;
    let answer = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    *pending = None;
    Some(answer.trim_end_matches('\n').to_string())
}

/// Write an answer atomically (tmp + rename), from inside a popup pane.
pub fn write_answer(answer: &str) -> anyhow::Result<()> {
    let file = std::env::var("GITVIEW_ANSWER_FILE")?;
    let tmp = format!("{file}.tmp");
    std::fs::write(&tmp, answer)?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

/// The agent panes in the current workspace:
/// `(pane_id, agent, status, tab_label, cwd_basename)`.
pub fn workspace_agents() -> Vec<(String, String, String, String, String)> {
    let Some(workspace) = std::env::var_os("HERDR_WORKSPACE_ID") else {
        return Vec::new();
    };
    let workspace = workspace.to_string_lossy().to_string();
    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());

    let json_of = |args: &[&str]| -> Option<serde_json::Value> {
        let out = Command::new(&bin).args(args).output().ok()?;
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).ok()
    };

    // tab_id -> label, for "which tab is this agent in".
    let mut tab_labels = std::collections::HashMap::new();
    if let Some(value) = json_of(&["tab", "list"])
        && let Some(tabs) = value
            .get("result")
            .and_then(|r| r.get("tabs"))
            .and_then(|t| t.as_array())
    {
        for tab in tabs {
            let get = |k: &str| tab.get(k).and_then(|v| v.as_str()).unwrap_or("");
            tab_labels.insert(get("tab_id").to_string(), get("label").to_string());
        }
    }

    let Some(value) = json_of(&["pane", "list"]) else {
        return Vec::new();
    };
    let mut agents = Vec::new();
    if let Some(panes) = value
        .get("result")
        .and_then(|r| r.get("panes"))
        .and_then(|p| p.as_array())
    {
        for pane in panes {
            let get = |k: &str| pane.get(k).and_then(|v| v.as_str()).unwrap_or("");
            if get("workspace_id") == workspace && !get("agent").is_empty() {
                let cwd_base = get("cwd").rsplit('/').next().unwrap_or("").to_string();
                agents.push((
                    get("pane_id").to_string(),
                    get("agent").to_string(),
                    get("agent_status").to_string(),
                    tab_labels.get(get("tab_id")).cloned().unwrap_or_default(),
                    cwd_base,
                ));
            }
        }
    }
    agents
}
