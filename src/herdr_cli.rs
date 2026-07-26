//! Tiny helpers for panes talking to the herdr CLI at runtime (the
//! orchestrator has its own richer wrapper; panes need focus and, in
//! sidebar mode, discovery of an already-running nvim in the host tab).

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Focus a pane (best-effort, fire-and-forget). Used for the editor handoff:
/// list → preview when an edit starts, preview → list when it ends.
pub fn focus_pane(bin: &OsStr, pane_id: &str) {
    match Command::new(bin)
        .args(["plugin", "pane", "focus", pane_id])
        .output()
    {
        Ok(out) if !out.status.success() => crate::logx::log(format!(
            "focus {pane_id} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(err) => crate::logx::log(format!("focus {pane_id} spawn failed: {err}")),
        _ => {}
    }
}

// ---- existing-nvim discovery (sidebar mode) -------------------------------

/// An nvim already running in another pane of our tab, remote-controllable
/// over `socket`.
pub struct TabNvim {
    pub pane_id: String,
    pub socket: PathBuf,
}

/// Find an nvim running in another pane of the tab containing `own_pane`.
/// Covers both `nvim --remote-ui` clients (e.g. the herdr-nvim sidebar —
/// the daemon socket is right in argv) and plain `nvim` runs (nvim ≥0.9
/// auto-listens on `<rundir>/nvim.<user>/<rand>/nvim.<pid>.0`).
/// In tab mode the gitview tab only holds our own panes, so this naturally
/// finds nothing there.
pub fn find_nvim_in_tab(bin: &OsStr, own_pane: &str, exclude: &[&str]) -> Option<TabNvim> {
    let reply = run_json(bin, &["pane", "get", own_pane])?;
    let tab = reply.pointer("/result/pane/tab_id")?.as_str()?.to_owned();

    let reply = run_json(bin, &["pane", "list"])?;
    let panes = reply.pointer("/result/panes")?.as_array()?;
    for pane in panes {
        let id = pane.get("pane_id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty()
            || id == own_pane
            || exclude.contains(&id)
            || pane.get("tab_id").and_then(Value::as_str) != Some(tab.as_str())
        {
            continue;
        }
        if let Some(socket) = pane_nvim_socket(bin, id) {
            crate::logx::log(format!(
                "found nvim in tab pane {id} (socket {})",
                socket.display()
            ));
            return Some(TabNvim {
                pane_id: id.to_owned(),
                socket,
            });
        }
    }
    None
}

/// The remote socket of the nvim running in `pane`, if its foreground
/// process is nvim and a socket can be located.
fn pane_nvim_socket(bin: &OsStr, pane: &str) -> Option<PathBuf> {
    let reply = run_json(bin, &["pane", "process-info", "--pane", pane])?;
    let procs = reply
        .pointer("/result/process_info/foreground_processes")?
        .as_array()?;
    for proc in procs {
        if proc.get("name").and_then(Value::as_str) != Some("nvim") {
            continue;
        }
        let argv: Vec<&str> = proc
            .get("argv")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // `nvim --server <sock> --remote-ui` (herdr-nvim sidebar and co.).
        if let Some(pos) = argv.iter().position(|a| *a == "--server")
            && let Some(sock) = argv.get(pos + 1)
        {
            let sock = PathBuf::from(sock);
            if sock.exists() {
                return Some(sock);
            }
        }
        // Plain nvim: derive the auto-listen socket from the pid.
        if let Some(pid) = proc.get("pid").and_then(Value::as_u64)
            && let Some(sock) = socket_for_pid(pid)
        {
            return Some(sock);
        }
    }
    None
}

/// nvim ≥0.9's default server socket: `<rundir>/nvim.<user>/<rand>/nvim.<pid>.0`
/// where rundir is `$XDG_RUNTIME_DIR` or the temp dir.
fn socket_for_pid(pid: u64) -> Option<PathBuf> {
    let user = std::env::var("USER").ok()?;
    let bases = [
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        Some(std::env::temp_dir()),
    ];
    for base in bases.into_iter().flatten() {
        let dir = base.join(format!("nvim.{user}"));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let cand = entry.path().join(format!("nvim.{pid}.0"));
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    None
}

fn run_json(bin: &OsStr, args: &[&str]) -> Option<Value> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}
