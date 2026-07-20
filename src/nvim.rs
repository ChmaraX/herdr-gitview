//! Remote control of the nvim instance running on the preview pane's PTY.
//!
//! The preview starts nvim with `--listen` on the per-view server socket
//! (see [`crate::hostenv::HostEnv::nvim_server`]); both panes use these
//! helpers to talk to it. Everything degrades to `None`/failure when the
//! editor is not nvim or no server socket is live.

use std::path::Path;
use std::process::{Command, Output};

/// Run an nvim remote command against the editor's `--listen` socket.
/// `None` = not remote-controllable (not nvim, or no live socket).
pub fn remote(editor: &str, server: Option<&Path>, args: &[&str]) -> Option<Output> {
    if !editor.contains("nvim") {
        return None;
    }
    let server = server?;
    if !server.exists() {
        return None;
    }
    Command::new(editor)
        .arg("--server")
        .arg(server)
        .args(args)
        .output()
        .ok()
}

/// Does the remote nvim hold modified buffers? Runs child processes —
/// call from a background thread, never a UI loop.
pub fn has_unsaved(editor: &str, server: Option<&Path>) -> Option<bool> {
    let out = remote(
        editor,
        server,
        &[
            "--remote-expr",
            r#"len(filter(getbufinfo(), "v:val.changed"))"#,
        ],
    )?;
    if !out.status.success() {
        return None;
    }
    let count = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(!count.is_empty() && count != "0")
}

/// Ask the remote nvim to quit (`:wqa` when saving, `:qa!` otherwise).
/// Returns whether the request was delivered.
pub fn request_close(editor: &str, server: Option<&Path>, save: bool) -> bool {
    let keys = if save {
        "<C-\\><C-n>:wqa<CR>"
    } else {
        "<C-\\><C-n>:qa!<CR>"
    };
    remote(editor, server, &["--remote-send", keys])
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Switch the running nvim to another file. Returns whether it succeeded.
pub fn open_file(editor: &str, server: Option<&Path>, file: &Path) -> bool {
    remote(editor, server, &["--remote", &file.display().to_string()])
        .map(|out| out.status.success())
        .unwrap_or(false)
}
