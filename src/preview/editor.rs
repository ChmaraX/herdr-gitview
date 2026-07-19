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
