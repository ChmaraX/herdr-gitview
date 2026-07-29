//! The inline note composer: the text area spliced into the diff under the
//! lines being commented on.
//!
//! Keeping the key table here means it can be exercised directly, and that
//! `PreviewApp` only has to deal with the three things that can come out of a
//! keystroke rather than with twenty key bindings.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::textarea::TextArea;

/// An open composer. `editing` distinguishes rewriting an existing note from
/// writing a new one; the note it targets lives in `PreviewApp::pending_note`.
pub struct Composer {
    pub input: TextArea,
    /// The note being rewritten, or `None` when this is a new one.
    pub editing: Option<u64>,
}

impl Composer {
    pub fn new(text: String, editing: Option<u64>) -> Composer {
        Composer {
            input: TextArea::new(text),
            editing,
        }
    }

    /// What a keystroke asks of the pane around the composer.
    pub fn key(&mut self, ev: KeyEvent, width: usize) -> Outcome {
        if ev.kind != KeyEventKind::Press && ev.kind != KeyEventKind::Repeat {
            return Outcome::Ignored;
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        let alt = ev.modifiers.contains(KeyModifiers::ALT);
        let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
        match ev.code {
            // Enter saves; every modifier combination a terminal can actually
            // deliver for "newline" inserts one instead. `ctrl+j` is the
            // fallback for terminals that report shift+enter as a plain enter.
            KeyCode::Enter if shift || alt || ctrl => self.input.insert_newline(),
            KeyCode::Char('j') if ctrl => self.input.insert_newline(),
            KeyCode::Enter => return Outcome::Save,
            KeyCode::Esc => return Outcome::Cancel,

            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char('w') if ctrl => self.input.delete_word(),
            KeyCode::Backspace if ctrl || alt => self.input.delete_word(),
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),

            KeyCode::Left => self.input.move_left(),
            KeyCode::Right => self.input.move_right(),
            KeyCode::Up => self.input.move_up(width),
            KeyCode::Down => self.input.move_down(width),
            KeyCode::Home => self.input.move_home(width),
            KeyCode::End => self.input.move_end(width),
            KeyCode::Char('a') if ctrl => self.input.move_home(width),
            KeyCode::Char('e') if ctrl => self.input.move_end(width),

            KeyCode::Char(c) if !ctrl && !alt => self.input.insert(c),
            _ => return Outcome::Ignored,
        }
        Outcome::Edited
    }

    /// The note text, or `None` when it is empty — an empty note is a cancel,
    /// since there is nothing to send an agent.
    pub fn finish(self) -> Option<String> {
        let text = self.input.text().trim_end().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// What the pane should do after handing a key to the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The text changed: re-splice the box and keep it in view.
    Edited,
    Save,
    Cancel,
    /// Not a key the composer wants; nothing to redraw.
    Ignored,
}
