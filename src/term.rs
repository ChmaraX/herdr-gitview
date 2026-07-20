//! Small shared terminal helpers used by every TUI entrypoint.

use crossterm::execute;

pub fn enable_mouse() {
    let _ = execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
}

pub fn disable_mouse() {
    let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
}
