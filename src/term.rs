//! Small shared terminal helpers used by every TUI entrypoint.

use crossterm::execute;

pub fn enable_mouse() {
    let _ = execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
}

pub fn disable_mouse() {
    let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
}

/// Ask for the kitty keyboard protocol, which is what lets a terminal report
/// `shift+enter` as something other than a plain `enter`. Best-effort: on
/// terminals without it the request is ignored and callers fall back to the
/// combinations that always come through (`ctrl+j`, `alt+enter`).
pub fn enable_key_disambiguation() {
    use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    let _ = execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
}

pub fn disable_key_disambiguation() {
    let _ = execute!(
        std::io::stdout(),
        crossterm::event::PopKeyboardEnhancementFlags
    );
}
