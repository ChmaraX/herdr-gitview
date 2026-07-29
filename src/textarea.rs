//! A small wrapping text area: the editing model behind the note popup.
//!
//! Pure state (`text` + a byte caret) plus a soft-wrap layout, so the widget
//! stays a rendering detail and every editing rule is unit-testable.
//!
//! Rows produced by [`TextArea::layout`] partition the text: they cover it
//! contiguously except for the `\n` bytes, which belong to no row. That makes
//! the caret unambiguous at a hard line end (it stays on that line) while a
//! caret at a soft wrap moves to the start of the next row, which is what
//! every editor does.

use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// An editable, soft-wrapping multi-line buffer.
#[derive(Debug, Clone)]
pub struct TextArea {
    text: String,
    /// Caret position as a byte index; always on a char boundary.
    caret: usize,
}

impl TextArea {
    /// Start with `text`, caret at the end (so editing an existing note
    /// continues where it left off).
    pub fn new(text: String) -> TextArea {
        let caret = text.len();
        TextArea { text, caret }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Characters (not bytes) — what a "N chars" counter should show.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    // ---- editing ----------------------------------------------------------

    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.caret, ch);
        self.caret += ch.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        self.insert('\n');
    }

    /// Delete the character before the caret.
    pub fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary(self.caret) {
            self.text.replace_range(prev..self.caret, "");
            self.caret = prev;
        }
    }

    /// Delete the character after the caret.
    pub fn delete(&mut self) {
        if let Some(next) = self.next_boundary(self.caret) {
            self.text.replace_range(self.caret..next, "");
        }
    }

    /// Delete the word before the caret, plus the whitespace in front of it.
    pub fn delete_word(&mut self) {
        let mut at = self.caret;
        while let Some(prev) = self.prev_boundary(at) {
            if !self.text[prev..at].starts_with(char::is_whitespace) {
                break;
            }
            at = prev;
        }
        while let Some(prev) = self.prev_boundary(at) {
            if self.text[prev..at].starts_with(char::is_whitespace) {
                break;
            }
            at = prev;
        }
        self.text.replace_range(at..self.caret, "");
        self.caret = at;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }

    // ---- caret movement ---------------------------------------------------

    pub fn move_left(&mut self) {
        if let Some(prev) = self.prev_boundary(self.caret) {
            self.caret = prev;
        }
    }

    pub fn move_right(&mut self) {
        if let Some(next) = self.next_boundary(self.caret) {
            self.caret = next;
        }
    }

    /// Move to the start / end of the caret's *visual* row.
    pub fn move_home(&mut self, width: usize) {
        let rows = self.layout(width);
        self.caret = rows[self.caret_row(&rows)].start;
    }

    pub fn move_end(&mut self, width: usize) {
        let rows = self.layout(width);
        self.caret = rows[self.caret_row(&rows)].end;
    }

    /// Move one visual row up / down, keeping the column where possible.
    pub fn move_up(&mut self, width: usize) {
        self.move_vertical(width, -1);
    }

    pub fn move_down(&mut self, width: usize) {
        self.move_vertical(width, 1);
    }

    fn move_vertical(&mut self, width: usize, delta: isize) {
        let rows = self.layout(width);
        let row = self.caret_row(&rows);
        let target = row as isize + delta;
        if target < 0 || target as usize >= rows.len() {
            return; // at an edge: stay put
        }
        let col = self.text[rows[row].start..self.caret].width();
        self.caret = self.byte_at_col(&rows[target as usize], col);
    }

    /// The byte index in `row` closest to display column `col`.
    fn byte_at_col(&self, row: &Range<usize>, col: usize) -> usize {
        let mut at = row.start;
        let mut acc = 0usize;
        for ch in self.text[row.clone()].chars() {
            let cw = ch.width().unwrap_or(0);
            if acc + cw > col {
                break;
            }
            acc += cw;
            at += ch.len_utf8();
        }
        at
    }

    // ---- layout -----------------------------------------------------------

    /// Soft-wrap to `width` columns. Always returns at least one row.
    pub fn layout(&self, width: usize) -> Vec<Range<usize>> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut line_start = 0usize;
        // Split on '\n' by hand so the byte offsets stay exact.
        loop {
            let rel_end = self.text[line_start..]
                .find('\n')
                .map(|i| line_start + i)
                .unwrap_or(self.text.len());
            wrap_line(&self.text, line_start..rel_end, width, &mut rows);
            if rel_end == self.text.len() {
                break;
            }
            line_start = rel_end + 1; // skip the '\n' itself
        }
        rows
    }

    /// Which row the caret sits on: the *last* row that starts at or before
    /// it, so a caret at a soft wrap shows at the start of the next row.
    pub fn caret_row(&self, rows: &[Range<usize>]) -> usize {
        rows.iter()
            .rposition(|r| r.start <= self.caret && self.caret <= r.end)
            .unwrap_or(0)
    }

    /// The caret as `(row, column)` in display cells.
    pub fn caret_pos(&self, width: usize) -> (usize, usize) {
        let rows = self.layout(width);
        let row = self.caret_row(&rows);
        (row, self.text[rows[row].start..self.caret].width())
    }

    // ---- char boundary helpers --------------------------------------------

    fn prev_boundary(&self, at: usize) -> Option<usize> {
        self.text[..at]
            .chars()
            .next_back()
            .map(|c| at - c.len_utf8())
    }

    fn next_boundary(&self, at: usize) -> Option<usize> {
        self.text[at..].chars().next().map(|c| at + c.len_utf8())
    }
}

/// Right-truncate `s` to `max` display columns, marking a cut with a
/// trailing `…`. Shared by every pane that has to fit text into a fixed
/// width.
pub fn elide_tail(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut kept = String::new();
    let mut acc = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > max - 1 {
            break;
        }
        acc += cw;
        kept.push(ch);
    }
    format!("{kept}…")
}

/// Word-wrap one logical line to `width` columns, hard-breaking anything
/// that cannot fit. Always yields at least one (possibly empty) row.
pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let area = TextArea::new(text.to_string());
    area.layout(width)
        .into_iter()
        .map(|r| area.text()[r].trim_end().to_string())
        .collect()
}

/// Wrap one logical line (`range`) into `rows`, breaking before a word that
/// doesn't fit and hard-breaking words longer than the whole width. Emits at
/// least one row, so empty lines survive.
fn wrap_line(text: &str, range: Range<usize>, width: usize, rows: &mut Vec<Range<usize>>) {
    let line = &text[range.clone()];
    let base = range.start;
    let mut row_start = 0usize; // relative to `line`
    let mut row_w = 0usize;
    // The current word, which may still move to the next row as a unit.
    let mut word_start = 0usize;
    let mut word_w = 0usize;

    for (i, ch) in line.char_indices() {
        let cw = ch.width().unwrap_or(0);
        if ch.is_whitespace() {
            // Whitespace ends the word. A space that no longer fits is
            // swallowed by the wrap: it stays at the end of this row rather
            // than starting the next one with a stray indent.
            row_w += word_w;
            word_w = 0;
            word_start = i + ch.len_utf8();
            if row_w + cw > width {
                rows.push(base + row_start..base + word_start);
                row_start = word_start;
                row_w = 0;
                continue;
            }
            row_w += cw;
            continue;
        }
        if word_w + cw > width {
            // The word alone exceeds a row — hard-break it here.
            rows.push(base + row_start..base + i);
            row_start = i;
            row_w = 0;
            word_start = i;
            word_w = 0;
        } else if row_w + word_w + cw > width {
            // The word doesn't fit after what's already on the row: move the
            // whole word down.
            rows.push(base + row_start..base + word_start);
            row_start = word_start;
            row_w = 0;
        }
        word_w += cw;
    }
    rows.push(base + row_start..range.end);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(text: &str) -> TextArea {
        TextArea::new(text.to_string())
    }

    /// The rows as strings, for readable assertions.
    fn rows_of(a: &TextArea, width: usize) -> Vec<String> {
        a.layout(width)
            .into_iter()
            .map(|r| a.text()[r].to_string())
            .collect()
    }

    // ---- layout -----------------------------------------------------------

    #[test]
    fn elide_tail_marks_a_cut_and_fits_the_width() {
        assert_eq!(elide_tail("short", 10), "short");
        assert_eq!(elide_tail("truncate me", 6), "trunc…");
        assert_eq!(elide_tail("x", 0), "");
        for max in 1..12 {
            assert!(elide_tail("a longer string here", max).width() <= max);
        }
    }

    #[test]
    fn wrap_plain_splits_and_trims() {
        assert_eq!(
            wrap_plain("the quick brown fox", 10),
            vec!["the quick", "brown fox"]
        );
        assert_eq!(wrap_plain("", 10), vec![""]);
    }

    #[test]
    fn empty_text_is_one_empty_row() {
        assert_eq!(rows_of(&area(""), 10), vec![""]);
    }

    #[test]
    fn hard_newlines_split_rows_and_keep_empty_lines() {
        assert_eq!(rows_of(&area("a\n\nb"), 10), vec!["a", "", "b"]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(
            rows_of(&area("the quick brown fox"), 10),
            vec!["the quick ", "brown fox"]
        );
    }

    #[test]
    fn hard_breaks_a_word_longer_than_the_width() {
        assert_eq!(
            rows_of(&area("src/preview/render.rs"), 8),
            vec!["src/prev", "iew/rend", "er.rs"]
        );
    }

    #[test]
    fn no_row_content_exceeds_the_width() {
        let text = "note about src/list/session.rs and a-very-long-hyphenated-name\n\
                    plus a second paragraph that also needs wrapping";
        for width in [3, 5, 9, 16, 24, 40] {
            let a = area(text);
            for row in rows_of(&a, width) {
                // Trailing spaces may sit at the edge; the content does not.
                assert!(
                    row.trim_end().width() <= width,
                    "width {width}: {row:?} is {} cols",
                    row.trim_end().width()
                );
            }
        }
    }

    #[test]
    fn rows_cover_the_text_apart_from_newlines() {
        let a = area("hello there\nsecond line here");
        let rows = a.layout(7);
        let mut rebuilt = String::new();
        for (i, r) in rows.iter().enumerate() {
            if i > 0 && r.start > rows[i - 1].end {
                rebuilt.push('\n'); // the gap is exactly the newline byte
            }
            rebuilt.push_str(&a.text()[r.clone()]);
        }
        assert_eq!(rebuilt, a.text());
    }

    // ---- caret ------------------------------------------------------------

    #[test]
    fn a_new_area_puts_the_caret_at_the_end() {
        let a = area("hello");
        assert_eq!(a.caret(), 5);
        assert_eq!(a.caret_pos(10), (0, 5));
    }

    #[test]
    fn the_caret_is_visible_however_long_the_text_gets() {
        // The regression: the input used to be clipped, so past the last
        // visible row the caret was drawn off the bottom edge.
        let (width, height) = (20, 4);
        let mut a = area("");
        for _ in 0..400 {
            a.insert('x');
            let rows = a.layout(width);
            let (row, col) = a.caret_pos(width);
            let scroll = rows.len().saturating_sub(height);
            assert!(row >= scroll, "caret row {row} scrolled off the top");
            assert!(row < scroll + height, "caret row {row} below the window");
            assert!(col <= width, "caret column {col} past the edge");
        }
    }

    #[test]
    fn the_caret_moves_to_the_next_row_at_a_soft_wrap() {
        let mut a = area("abcde fgh");
        a.move_home(5); // row 1 ("fgh") starts at byte 6
        assert_eq!(a.caret(), 6);
        a.move_left(); // onto the space, which lives at the end of row 0
        assert_eq!(a.caret_pos(5), (0, 5));
    }

    #[test]
    fn the_caret_stays_on_the_line_at_a_hard_newline() {
        let mut a = area("ab\ncd");
        a.move_up(10);
        a.move_end(10);
        assert_eq!(a.caret(), 2, "end of the first line, before the newline");
        assert_eq!(a.caret_pos(10), (0, 2));
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut a = area("hello\nworld");
        a.move_home(10);
        a.move_right();
        a.move_right(); // column 2 of "world"
        assert_eq!(a.caret_pos(10), (1, 2));
        a.move_up(10);
        assert_eq!(a.caret_pos(10), (0, 2));
        a.move_down(10);
        assert_eq!(a.caret_pos(10), (1, 2));
    }

    #[test]
    fn up_and_down_clamp_to_a_shorter_line() {
        let mut a = area("ab\nlonger line");
        assert_eq!(a.caret_pos(20).0, 1);
        a.move_up(20);
        assert_eq!(
            a.caret_pos(20),
            (0, 2),
            "clamped to the end of a short line"
        );
    }

    #[test]
    fn moving_past_the_edges_is_a_no_op() {
        let mut a = area("only");
        a.move_up(10);
        assert_eq!(a.caret(), 4);
        a.move_down(10);
        assert_eq!(a.caret(), 4);
        a.move_home(10);
        a.move_left();
        assert_eq!(a.caret(), 0);
        a.move_end(10);
        a.move_right();
        assert_eq!(a.caret(), 4);
    }

    // ---- editing ----------------------------------------------------------

    #[test]
    fn insert_and_backspace_at_the_caret() {
        let mut a = area("ac");
        a.move_left();
        a.insert('b');
        assert_eq!(a.text(), "abc");
        assert_eq!(a.caret(), 2);
        a.backspace();
        assert_eq!(a.text(), "ac");
    }

    #[test]
    fn delete_removes_the_character_after_the_caret() {
        let mut a = area("abc");
        a.move_home(10);
        a.delete();
        assert_eq!(a.text(), "bc");
        assert_eq!(a.caret(), 0);
    }

    #[test]
    fn newlines_are_inserted_at_the_caret() {
        let mut a = area("ab");
        a.move_left();
        a.insert_newline();
        assert_eq!(a.text(), "a\nb");
        assert_eq!(a.caret_pos(10), (1, 0));
    }

    #[test]
    fn backspace_joins_two_lines() {
        let mut a = area("a\nb");
        a.move_home(10);
        a.backspace();
        assert_eq!(a.text(), "ab");
    }

    #[test]
    fn delete_word_takes_the_word_and_its_leading_space() {
        let mut a = area("fix the parser bug");
        a.delete_word();
        assert_eq!(a.text(), "fix the parser ");
        a.delete_word();
        assert_eq!(a.text(), "fix the ");
    }

    #[test]
    fn editing_an_empty_or_blank_buffer_is_safe() {
        let mut a = area("");
        a.backspace();
        a.delete();
        a.delete_word();
        a.move_left();
        a.move_right();
        assert_eq!(a.text(), "");
        assert_eq!(a.caret(), 0);
        let mut a = area("   ");
        a.delete_word();
        assert_eq!(a.text(), "");
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        let mut a = area("héllo → wörld");
        for _ in 0..20 {
            a.move_left(); // would panic on a non-boundary
        }
        a.insert('é');
        a.delete();
        a.backspace();
        assert!(a.text().starts_with('h') || a.text().starts_with('é'));
        // and the layout keeps working at any width
        for width in [1, 2, 5, 13] {
            assert!(!a.layout(width).is_empty());
        }
    }

    #[test]
    fn wide_characters_count_as_two_columns() {
        let a = area("日本語");
        assert_eq!(a.caret_pos(10), (0, 6));
        assert_eq!(rows_of(&a, 4), vec!["日本", "語"]);
    }

    #[test]
    fn clear_empties_everything() {
        let mut a = area("some note");
        a.clear();
        assert!(a.is_empty());
        assert_eq!(a.caret(), 0);
        assert_eq!(a.char_count(), 0);
    }

    #[test]
    fn char_count_counts_characters_not_bytes() {
        assert_eq!(area("héllo").char_count(), 5);
    }
}
