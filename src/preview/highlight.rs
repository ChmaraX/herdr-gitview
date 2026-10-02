//! Syntax highlighting via syntect + two-face (bat's broad syntax set),
//! after persiyanov/herdr-reviewr's `highlight.rs` (MIT).
//!
//! Produces per-line runs of `(text, fg-rgb)`; the renderer adds diff
//! backgrounds on top, so only token colors come from the syntax theme.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

/// Lines parsed above a hunk to prime the grammar state.
const LEAD_IN: usize = 20;

/// An 8-bit RGB color.
pub type Rgb = (u8, u8, u8);

/// A run of one line's text in a single color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub color: Rgb,
}

/// The broad syntax set, built once per process (expensive to deserialize).
fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// The embedded theme set, deserialized once and shared.
fn themes() -> &'static two_face::theme::EmbeddedLazyThemeSet {
    static THEMES: OnceLock<two_face::theme::EmbeddedLazyThemeSet> = OnceLock::new();
    THEMES.get_or_init(two_face::theme::extra)
}

pub struct Highlighter {
    theme: Theme,
    pub default_fg: Rgb,
}

impl Highlighter {
    pub fn new(theme: crate::config::Theme) -> Highlighter {
        let name = if theme.is_light() {
            EmbeddedThemeName::InspiredGithub
        } else {
            EmbeddedThemeName::OneHalfDark
        };
        let theme = themes().get(name).clone();
        let default_fg = theme
            .settings
            .foreground
            .map_or((0xc8, 0xc8, 0xc8), |c| (c.r, c.g, c.b));
        Highlighter { theme, default_fg }
    }

    /// Do we know the language of files with this extension?
    pub fn knows(extension: Option<&str>) -> bool {
        Self::syntax_for(extension).is_some()
    }

    /// The grammar for a file extension, if we know the language.
    fn syntax_for(extension: Option<&str>) -> Option<&'static SyntaxReference> {
        let syntaxes = syntaxes();
        extension.and_then(|ext| {
            syntaxes
                .find_syntax_by_extension(ext)
                .or_else(|| syntaxes.find_syntax_by_token(ext))
        })
    }

    /// One line, uncolored: the fallback for unknown languages and errors.
    pub fn plain(&self, line: &str) -> Vec<Run> {
        vec![Run {
            text: line.trim_end_matches('\n').to_string(),
            color: self.default_fg,
        }]
    }

    /// Highlight only the `wanted` lines (0-based, ascending) of `lines`
    /// (each with its `\n`). Each run of wanted lines is parsed from up to
    /// `LEAD_IN` lines above it, so the cost follows the diff, not the file;
    /// a string or comment opened further up than that may mis-color, the
    /// same trade-off delta makes. `keep_going` is the caller's checkpoint
    /// between runs and every 32 lines: returning false abandons the work
    /// (`None`).
    pub fn highlight_lines(
        &self,
        lines: &[&str],
        extension: Option<&str>,
        wanted: &[usize],
        keep_going: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<(usize, Vec<Run>)>> {
        let mut out = Vec::with_capacity(wanted.len());
        let Some(syntax) = Self::syntax_for(extension) else {
            for &i in wanted {
                out.push((i, self.plain(lines.get(i).copied().unwrap_or(""))));
            }
            return Some(out);
        };
        let mut rest = wanted
            .iter()
            .copied()
            .filter(|&i| i < lines.len())
            .peekable();
        while let Some(first) = rest.next() {
            if !keep_going() {
                return None;
            }
            // Extend the run while the next wanted line is close enough that
            // parsing through the gap is no dearer than a fresh lead-in.
            let mut want = vec![first];
            while let Some(&next) = rest.peek() {
                if next - want[want.len() - 1] > LEAD_IN {
                    break;
                }
                want.push(next);
                rest.next();
            }
            let last = want[want.len() - 1];
            let mut h = HighlightLines::new(syntax, &self.theme);
            let mut want = want.into_iter().peekable();
            let start = first.saturating_sub(LEAD_IN);
            for (i, line) in lines.iter().enumerate().take(last + 1).skip(start) {
                if i % 32 == 0 && !keep_going() {
                    return None;
                }
                let runs = self.highlight_one(&mut h, line);
                if want.peek() == Some(&i) {
                    want.next();
                    out.push((i, runs));
                }
            }
        }
        Some(out)
    }

    fn highlight_one(&self, h: &mut HighlightLines, line: &str) -> Vec<Run> {
        match h.highlight_line(line, syntaxes()) {
            Ok(regions) => regions
                .into_iter()
                .map(|(style, text)| Run {
                    text: text.trim_end_matches('\n').to_string(),
                    color: (style.foreground.r, style.foreground.g, style.foreground.b),
                })
                .collect(),
            // A grammar error degrades to plain text, never blocks the diff.
            Err(_) => self.plain(line),
        }
    }

    /// Highlight `content` line by line; each inner Vec is one line's runs.
    /// Unknown language → one plain run per line.
    pub fn highlight(&self, content: &str, extension: Option<&str>) -> Vec<Vec<Run>> {
        let Some(syntax) = Self::syntax_for(extension) else {
            return content.lines().map(|l| self.plain(l)).collect();
        };
        let mut h = HighlightLines::new(syntax, &self.theme);
        LinesWithEndings::from(content)
            .map(|line| self.highlight_one(&mut h, line))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_tokenizes_into_multiple_colored_runs() {
        let h = Highlighter::new(crate::config::Theme::Dark);
        let lines = h.highlight("let x = 1;\n", Some("rs"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() > 1, "rust should split into several runs");
        let joined: String = lines[0].iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, "let x = 1;");
    }

    #[test]
    fn unknown_language_is_plain() {
        let h = Highlighter::new(crate::config::Theme::Dark);
        let lines = h.highlight("alpha\nbeta\n", None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].color, h.default_fg);
    }
}
