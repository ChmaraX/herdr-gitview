//! Syntax colors for a built `DiffDoc`, added after the fact: which lines on
//! screen still need them (`highlight_job`), computing them off the UI
//! thread (`HighlightJob::run`), and swapping them in (`apply_highlights`).

use std::collections::HashMap;
use std::sync::Arc;

use syntect::util::LinesWithEndings;

use super::{DiffDoc, Row, Side};
use crate::preview::highlight::{Highlighter, Run};

impl DiffDoc {
    /// The work to color every on-screen line that is still plain (folded
    /// lines stay plain until revealed). `None` when nothing is left, or
    /// the language is unknown.
    pub fn highlight_job(&self) -> Option<HighlightJob> {
        if !self.colorable {
            return None;
        }
        let (mut old, mut new) = (Vec::new(), Vec::new());
        // Diff order keeps each side's line numbers ascending.
        for (side, line, code) in self.rows.iter().filter_map(Row::code) {
            if code.runs.is_none() {
                match side {
                    Side::Old => old.push(line),
                    Side::New => new.push(line),
                }
            }
        }
        if old.is_empty() && new.is_empty() {
            return None;
        }
        Some(HighlightJob {
            old_text: Arc::clone(&self.old),
            new_text: Arc::clone(&self.new),
            ext: self.ext.clone(),
            old,
            new,
        })
    }

    /// Lines on screen are still waiting for colors.
    pub fn highlight_pending(&self) -> bool {
        self.colorable
            && self
                .rows
                .iter()
                .filter_map(Row::code)
                .any(|(_, _, code)| code.runs.is_none())
    }

    /// Swap highlighted runs in for the lines they cover (folded rows
    /// included), then re-render. The text of each line is unchanged, so
    /// word emphasis, folds, and line maps all stay valid. Refused (false)
    /// for colors computed from another build of the file.
    pub fn apply_highlights(&mut self, h: &Highlights) -> bool {
        if !Arc::ptr_eq(&self.old, &h.old_text) || !Arc::ptr_eq(&self.new, &h.new_text) {
            return false;
        }
        let mut by_line: HashMap<(Side, usize), &Vec<Run>> = HashMap::new();
        by_line.extend(h.old.iter().map(|(i, r)| ((Side::Old, *i), r)));
        by_line.extend(h.new.iter().map(|(i, r)| ((Side::New, *i), r)));
        fn patch(rows: &mut [Row], by_line: &HashMap<(Side, usize), &Vec<Run>>) {
            for row in rows {
                if let Row::Fold { lines } = row {
                    patch(lines, by_line);
                } else if let Some((side, line, code)) = row.code_mut()
                    && let Some(runs) = by_line.get(&(side, line))
                {
                    code.runs = Some((*runs).clone());
                }
            }
        }
        patch(&mut self.rows, &by_line);
        self.rebuild();
        true
    }
}

/// Lines of a doc still waiting for syntax colors, with the content needed
/// to color them — self-contained, so it can run on another thread.
pub struct HighlightJob {
    old_text: Arc<str>,
    new_text: Arc<str>,
    ext: Option<String>,
    /// 0-based, ascending, per side.
    old: Vec<usize>,
    new: Vec<usize>,
}

impl HighlightJob {
    /// Color the job's lines, calling `keep_going` along the way (see
    /// `Highlighter::highlight_lines`); `None` if it said stop.
    pub fn run(
        &self,
        hl: &Highlighter,
        keep_going: &mut dyn FnMut() -> bool,
    ) -> Option<Highlights> {
        let ext = self.ext.as_deref();
        let mut side = |text: &str, wanted: &[usize]| {
            if wanted.is_empty() {
                return Some(Vec::new());
            }
            let lines: Vec<&str> = LinesWithEndings::from(text).collect();
            hl.highlight_lines(&lines, ext, wanted, keep_going)
        };
        Some(Highlights {
            old_text: Arc::clone(&self.old_text),
            new_text: Arc::clone(&self.new_text),
            old: side(&self.old_text, &self.old)?,
            new: side(&self.new_text, &self.new)?,
        })
    }
}

/// Syntax-colored runs for some lines of each side (0-based line indices).
#[derive(Debug, Clone)]
pub struct Highlights {
    /// The content the runs were computed from: they only fit a doc built
    /// from these very texts (a refresh of the same file may not be).
    old_text: Arc<str>,
    new_text: Arc<str>,
    old: Vec<(usize, Vec<Run>)>,
    new: Vec<(usize, Vec<Run>)>,
}
