//! Shared helper for herdr floating popup panes (≥0.7.4): spawn a plugin
//! popup entrypoint and poll for its answer file. Used for confirms
//! (`ask`), note input (`annotate`), and the agent picker (`pick_agent`).

use std::path::PathBuf;
use std::process::Command;

/// Does this herdr support popup plugin panes (≥0.7.4)?
pub fn supported(env: &crate::hostenv::HostEnv) -> bool {
    if !env.in_herdr() {
        return false; // standalone
    }
    let Ok(out) = Command::new(&env.herdr_bin).arg("--version").output() else {
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

/// The outcome of a popup interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The popup wrote its answer (may be empty = cancelled, per popup).
    Text(String),
    /// The popup pane died without answering (closed/killed/crashed).
    Dead,
}

struct Pending<K> {
    key: K,
    path: PathBuf,
    pane_id: Option<String>,
    herdr_bin: std::ffi::OsString,
    last_liveness_check: std::time::Instant,
}

/// At most one popup at a time per pane, with liveness tracking so a popup
/// that dies without writing its answer can never wedge the caller.
pub struct Popups<K> {
    pending: Option<Pending<K>>,
    /// How often `poll` probes a silent popup for liveness (tests shrink it).
    pub liveness_interval: std::time::Duration,
}

impl<K> Default for Popups<K> {
    fn default() -> Self {
        Popups {
            pending: None,
            liveness_interval: std::time::Duration::from_secs(2),
        }
    }
}

impl<K> Popups<K> {
    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    /// Open a popup pane running `entrypoint` with extra env vars, sized
    /// `width`×`height` cells. `key` identifies the interaction when the
    /// answer arrives. Returns false when the popup could not be opened (the
    /// caller decides the fallback) or when one is already open.
    /// `GITVIEW_ANSWER_FILE` is added automatically; env values are
    /// newline-sanitized (newlines would break herdr's --env framing).
    pub fn open(
        &mut self,
        env: &crate::hostenv::HostEnv,
        entrypoint: &str,
        envs: &[(String, String)],
        (width, height): (u16, u16),
        key: K,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(sock) = env.socket.clone() else {
            return false;
        };
        let answer = sock.with_extension(format!("{entrypoint}.answer"));
        let _ = std::fs::remove_file(&answer);

        let mut cmd = Command::new(&env.herdr_bin);
        cmd.args(["plugin", "pane", "open", "--plugin", "chmarax.gitview"])
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
        for (k, v) in envs {
            cmd.arg("--env")
                .arg(format!("{k}={}", v.replace(['\n', '\r'], " ")));
        }
        let Ok(out) = cmd.output() else {
            return false;
        };
        if !out.status.success() {
            crate::logx::log(format!(
                "popup {entrypoint} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return false;
        }
        // The reply carries the new pane object; remember its id so `poll`
        // can detect the popup dying without an answer.
        let pane_id =
            serde_json::from_str::<serde_json::Value>(String::from_utf8_lossy(&out.stdout).trim())
                .ok()
                .as_ref()
                .and_then(find_pane_id);
        self.pending = Some(Pending {
            key,
            path: answer,
            pane_id,
            herdr_bin: env.herdr_bin.clone(),
            last_liveness_check: std::time::Instant::now(),
        });
        true
    }

    /// Non-blocking: the answer if it arrived, or `Dead` if the popup pane
    /// vanished without writing one. The popup writes via tmp+rename, so a
    /// read never sees a partial answer. Liveness is probed at most every 2 s.
    pub fn poll(&mut self) -> Option<(K, Answer)> {
        let pending = self.pending.as_mut()?;
        if let Ok(answer) = std::fs::read_to_string(&pending.path) {
            let _ = std::fs::remove_file(&pending.path);
            let taken = self.pending.take().unwrap();
            return Some((
                taken.key,
                Answer::Text(answer.trim_end_matches('\n').to_string()),
            ));
        }
        if pending.last_liveness_check.elapsed() > self.liveness_interval {
            pending.last_liveness_check = std::time::Instant::now();
            let alive = match &pending.pane_id {
                Some(id) => pane_alive(&pending.herdr_bin, id),
                None => true, // can't check — rely on the answer file
            };
            if !alive {
                let taken = self.pending.take().unwrap();
                return Some((taken.key, Answer::Dead));
            }
        }
        None
    }
}

fn find_pane_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(s) = map.get("pane_id").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
            map.values().find_map(find_pane_id)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_pane_id),
        _ => None,
    }
}

fn pane_alive(bin: &std::ffi::OsStr, pane_id: &str) -> bool {
    Command::new(bin)
        .args(["pane", "get", pane_id])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
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
