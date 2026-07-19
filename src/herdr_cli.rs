//! Tiny helpers for panes talking to the herdr CLI at runtime (the
//! orchestrator has its own richer wrapper; panes only need focus).

use std::process::Command;

/// Focus a pane (best-effort, fire-and-forget). Used for the editor handoff:
/// list → preview when an edit starts, preview → list when it ends.
pub fn focus_pane(pane_id: &str) {
    let bin = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    match Command::new(&bin)
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
