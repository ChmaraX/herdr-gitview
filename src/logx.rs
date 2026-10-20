use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// debug.log is rotated once it crosses this size, so a long-lived herdr
/// session (list + preview + repeated toggles, all appending) doesn't grow
/// without bound.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Append a line to the debug log. Never fails: a TUI has no stdout to
/// complain on, so logging is strictly best-effort.
pub fn log(msg: impl AsRef<str>) {
    let path = state_dir().join("debug.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    append_with_rotation(&path, MAX_LOG_BYTES, msg.as_ref());
}

/// Rotate `path` to `path.1` (clobbering any previous one) if it has grown
/// past `cap`, then append `line`. Several processes append to the same
/// file concurrently; a rename racing another process's rotation can lose
/// or duplicate a line, which is fine, but it must never panic or bubble an
/// error up to the user.
fn append_with_rotation(path: &Path, cap: u64, line: &str) {
    rotate_if_needed(path, cap);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let mode = std::env::args().nth(1).unwrap_or_else(|| "?".into());
        let _ = writeln!(file, "[{mode} {}] {}", std::process::id(), line);
    }
}

fn rotate_if_needed(path: &Path, cap: u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return; // doesn't exist yet — nothing to rotate
    };
    if meta.len() <= cap {
        return;
    }
    let mut rotated = path.as_os_str().to_os_string();
    rotated.push(".1");
    // Best-effort: if another process already rotated between our size
    // check and here, this rename simply fails (source gone) — fine.
    let _ = std::fs::rename(path, PathBuf::from(rotated));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gitview-logx-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn rotates_once_the_cap_is_crossed() {
        let path = temp_path("debug.log");
        let rotated = {
            let mut p = path.as_os_str().to_os_string();
            p.push(".1");
            PathBuf::from(p)
        };
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&rotated);

        let cap = 50; // tiny cap so a handful of lines trips it
        for i in 0..20 {
            append_with_rotation(&path, cap, &format!("line {i} padding padding padding"));
        }

        assert!(rotated.exists(), "debug.log.1 should exist after rotation");
        let current_size = std::fs::metadata(&path).unwrap().len();
        assert!(
            current_size < cap * 3,
            "current log should be small after rotating, got {current_size} bytes"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&rotated);
    }

    #[test]
    fn leaves_a_small_file_alone() {
        let path = temp_path("small.log");
        let _ = std::fs::remove_file(&path);
        append_with_rotation(&path, MAX_LOG_BYTES, "just one line");
        let rotated = {
            let mut p = path.as_os_str().to_os_string();
            p.push(".1");
            PathBuf::from(p)
        };
        assert!(!rotated.exists());
        let _ = std::fs::remove_file(&path);
    }
}
