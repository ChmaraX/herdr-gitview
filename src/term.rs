//! Terminal mode ownership for the TUI entrypoints.
//!
//! Every mode we turn on (mouse reporting, the kitty keyboard protocol) has
//! to be turned off again on *every* exit path, including a panic. Hand-paired
//! enable/disable calls got that wrong: `ratatui::restore()` does not pop
//! keyboard-enhancement flags, so a panic in the diff pane left the user's
//! shell with disambiguated escape codes — visibly broken arrow keys and
//! enter. [`TermGuard`] owns the modes instead, so `Drop` runs on the panic
//! path too.

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;

/// Which optional terminal modes a pane wants.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modes {
    pub mouse: bool,
    /// The kitty keyboard protocol, which is what lets a terminal report
    /// `shift+enter` as something other than a plain `enter`. Best-effort: a
    /// terminal without it ignores the request and callers fall back to the
    /// combinations that always come through (`ctrl+j`, `alt+enter`).
    pub key_disambiguation: bool,
}

/// Owns the terminal modes for as long as it lives. Enters them on
/// construction, leaves them on `Drop` — panic included.
#[derive(Debug)]
pub struct TermGuard {
    modes: Modes,
}

impl TermGuard {
    pub fn enter(modes: Modes) -> TermGuard {
        let guard = TermGuard { modes };
        guard.apply(true);
        guard
    }

    /// Hand the terminal to a child process (an editor on our PTY) for as
    /// long as the returned guard lives: our modes are dropped while it is
    /// alive and restored when it goes out of scope, on every path.
    pub fn suspend(&self) -> Suspended<'_> {
        self.apply(false);
        Suspended { guard: self }
    }

    fn apply(&self, on: bool) {
        let mut out = std::io::stdout();
        if self.modes.mouse {
            let _ = if on {
                execute!(out, EnableMouseCapture)
            } else {
                execute!(out, DisableMouseCapture)
            };
        }
        if self.modes.key_disambiguation {
            let _ = if on {
                execute!(
                    out,
                    PushKeyboardEnhancementFlags(
                        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    )
                )
            } else {
                execute!(out, PopKeyboardEnhancementFlags)
            };
        }
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        self.apply(false);
    }
}

/// The terminal is on loan to a child process; our modes resume on drop.
pub struct Suspended<'a> {
    guard: &'a TermGuard,
}

impl Drop for Suspended<'_> {
    fn drop(&mut self) {
        self.guard.apply(true);
    }
}
