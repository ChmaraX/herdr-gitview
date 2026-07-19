//! Running a real editor (or interactive git) on this pane's PTY: suspend the
//! ratatui TUI, hand the terminal to the child process, restore afterwards.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Run `argv` on this PTY with the TUI suspended. Returns the child's success.
///
/// Order matters: restore the terminal *before* spawning (the child expects a
/// sane cooked-mode terminal), and re-init *after* it exits — even when the
/// spawn fails, so the caller always gets its TUI back.
pub fn run_on_pty(
    terminal: &mut ratatui::DefaultTerminal,
    cwd: &Path,
    argv: &[String],
    envs: &[(String, String)],
) -> Result<bool> {
    if argv.is_empty() {
        bail!("empty editor argv");
    }

    // 1. Leave alt-screen, disable raw mode (symmetric with ratatui::init —
    //    we enable nothing beyond ratatui's defaults, so nothing else to pop).
    ratatui::restore();
    // 2. Clear so the child starts on a clean screen.
    print!("\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();

    // 3. Run the child with inherited stdio; the PTY belongs to it now.
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(cwd);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let status = cmd
        .status()
        .with_context(|| format!("spawning {}", argv[0]));

    // 4. Re-init and force a full redraw *before* propagating any error.
    *terminal = ratatui::init();
    let _ = terminal.clear();

    Ok(status?.success())
}

/// First line number (new-file side) of the first hunk in a colored diff:
/// strip ANSI escapes, find the first `@@ -a[,b] +c[,d] @@`, return `c`.
pub fn first_new_line(raw: &[u8]) -> Option<u32> {
    let stripped = strip_ansi(raw);
    for line in stripped.split(|b| *b == b'\n') {
        if line.starts_with(b"@@") {
            let text = String::from_utf8_lossy(line);
            let plus = text.find('+')?;
            let digits: String = text[plus + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            return digits.parse().ok();
        }
    }
    None
}

/// Remove ANSI escape sequences (CSI `ESC [ … final-byte` and the rare
/// two-byte `ESC x` forms) with a plain byte scan — no regex dependency.
fn strip_ansi(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == 0x1b {
            i += 1;
            if raw.get(i) == Some(&b'[') {
                // CSI: skip until a byte in 0x40..=0x7e (the final byte).
                i += 1;
                while i < raw.len() && !(0x40..=0x7e).contains(&raw[i]) {
                    i += 1;
                }
                i += 1; // skip the final byte itself
            } else {
                i += 1; // ESC + one byte
            }
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_new_line_plain_hunk() {
        let raw = b"diff --git a/f b/f\nindex 123..456 100644\n--- a/f\n+++ b/f\n@@ -10,3 +12,4 @@ fn ctx()\n context\n+added\n";
        assert_eq!(first_new_line(raw), Some(12));
    }

    #[test]
    fn first_new_line_colored_hunk() {
        // git --color=always wraps hunk headers in cyan (36m).
        let raw = b"\x1b[1mdiff --git a/f b/f\x1b[m\n\x1b[36m@@ -1,2 +3 @@\x1b[m\n\x1b[32m+x\x1b[m\n";
        assert_eq!(first_new_line(raw), Some(3));
    }

    #[test]
    fn first_new_line_none_without_hunks() {
        assert_eq!(first_new_line(b""), None);
        assert_eq!(first_new_line(b"Binary files a/x and b/x differ\n"), None);
    }

    #[test]
    fn first_new_line_single_line_count_form() {
        // `@@ -0,0 +1 @@` (new file, one line) — no comma on the + side.
        let raw = b"@@ -0,0 +1 @@\n+hello\n";
        assert_eq!(first_new_line(raw), Some(1));
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi(b"\x1b[32mgreen\x1b[m plain"), b"green plain");
    }
}
