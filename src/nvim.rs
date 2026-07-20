//! Remote control of the nvim instance running on the preview pane's PTY.
//!
//! The preview starts nvim with `--listen` on a per-view socket; both panes
//! use these helpers to talk to it (switch files, probe for unsaved buffers,
//! ask it to quit). Everything degrades to `None`/failure when the editor is
//! not nvim or no server socket is live.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The nvim remote socket, derived from the view's IPC socket path.
pub fn server_path() -> Option<PathBuf> {
    let sock = std::env::var_os("GITVIEW_SOCKET")?;
    let mut path = PathBuf::from(sock);
    path.set_extension("nvim");
    Some(path)
}

/// Run an nvim remote command against the editor's `--listen` socket.
/// `None` = not remote-controllable (not nvim, or no live socket).
pub fn remote(editor: &str, args: &[&str]) -> Option<Output> {
    if !editor.contains("nvim") {
        return None;
    }
    let server = server_path()?;
    if !server.exists() {
        return None;
    }
    Command::new(editor)
        .arg("--server")
        .arg(&server)
        .args(args)
        .output()
        .ok()
}

/// Does the remote nvim hold modified buffers? Runs child processes —
/// call from a background thread, never a UI loop.
pub fn has_unsaved(editor: &str) -> Option<bool> {
    let out = remote(
        editor,
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
pub fn request_close(editor: &str, save: bool) -> bool {
    let keys = if save {
        "<C-\\><C-n>:wqa<CR>"
    } else {
        "<C-\\><C-n>:qa!<CR>"
    };
    remote(editor, &["--remote-send", keys])
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Switch the running nvim to another file. Returns whether it succeeded.
pub fn open_file(editor: &str, file: &Path) -> bool {
    let Some(server) = server_path().filter(|s| s.exists() && editor.contains("nvim")) else {
        return false;
    };
    Command::new(editor)
        .arg("--server")
        .arg(&server)
        .arg("--remote")
        .arg(file)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}
