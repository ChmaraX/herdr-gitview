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
    remote_command(editor, server, args)?.output().ok()
}

/// The [`remote`] command, `None` when there's no nvim to ask.
fn remote_command(editor: &str, server: Option<&Path>, args: &[&str]) -> Option<Command> {
    if !editor.contains("nvim") {
        return None;
    }
    let server = server?;
    if !server.exists() {
        return None;
    }
    let mut cmd = Command::new(editor);
    cmd.arg("--server").arg(server).args(args);
    Some(cmd)
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

/// The expression [`edit_at`] sends: `:edit +<line> <file>`, with the file
/// name built from its code points (`list2str`) and escaped by nvim itself
/// (`fnameescape`). No character of the name is ever read as a key, a
/// command separator (`|`), a special file name (`%`, `#`) or a quote.
pub fn edit_expr(file: &Path, line: u32) -> String {
    let points: Vec<String> = file
        .to_string_lossy()
        .chars()
        .map(|c| u32::from(c).to_string())
        .collect();
    format!(
        "execute('edit +{line} ' .. fnameescape(list2str([{}], 1)))",
        points.join(",")
    )
}

/// Open `file` at `line` in the running nvim. Returns whether it succeeded.
pub fn edit_at(editor: &str, server: Option<&Path>, file: &Path, line: u32) -> bool {
    let Some(mut cmd) = remote_command(editor, server, &["--remote-expr", &edit_expr(file, line)])
    else {
        return false;
    };
    // An nvim busy with a prompt doesn't answer: give up after 5 s.
    crate::deadline::output_within(&mut cmd, None, std::time::Duration::from_secs(5))
        .is_ok_and(|out| out.status.success())
}
