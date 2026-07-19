use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Append a line to the debug log. Never fails: a TUI has no stdout to
/// complain on, so logging is strictly best-effort.
pub fn log(msg: impl AsRef<str>) {
    let path = state_dir().join("debug.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let mode = std::env::args().nth(1).unwrap_or_else(|| "?".into());
        let _ = writeln!(file, "[{mode} {}] {}", std::process::id(), msg.as_ref());
    }
}

/// Plugin state directory: herdr provides it when we run as a plugin;
/// standalone runs fall back to ~/.local/state/herdr-gitview.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("HERDR_PLUGIN_STATE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".local/state/herdr-gitview"),
    }
}

/// Restore the terminal before printing a panic, otherwise the message is
/// invisible garbage inside the alternate screen.
pub fn init_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        original(info);
    }));
}
