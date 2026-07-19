//! Syntax highlighting via syntect + two-face (bat's broad syntax set),
//! after persiyanov/herdr-reviewr's `highlight.rs` (MIT).
//!
//! Produces per-line runs of `(text, fg-rgb)`; the renderer adds diff
//! backgrounds on top, so only token colors come from the syntax theme.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

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
    /// `flavor` comes from config: "light" | "dark" (default) | ignored else.
    pub fn new(flavor: &str) -> Highlighter {
        let name = match flavor {
            "light" => EmbeddedThemeName::InspiredGithub,
            _ => EmbeddedThemeName::OneHalfDark,
        };
        let theme = themes().get(name).clone();
        let default_fg = theme
            .settings
            .foreground
            .map_or((0xc8, 0xc8, 0xc8), |c| (c.r, c.g, c.b));
        Highlighter { theme, default_fg }
    }

    /// Highlight `content` line by line; each inner Vec is one line's runs.
    /// Unknown language → one plain run per line.
    pub fn highlight(&self, content: &str, extension: Option<&str>) -> Vec<Vec<Run>> {
        let syntaxes = syntaxes();
        let syntax = extension.and_then(|ext| {
            syntaxes
                .find_syntax_by_extension(ext)
                .or_else(|| syntaxes.find_syntax_by_token(ext))
        });
        let Some(syntax) = syntax else {
            return content
                .lines()
                .map(|l| {
                    vec![Run {
                        text: l.to_string(),
                        color: self.default_fg,
                    }]
                })
                .collect();
        };
        let mut h = HighlightLines::new(syntax, &self.theme);
        let mut out = Vec::new();
        for line in LinesWithEndings::from(content) {
            let runs = match h.highlight_line(line, syntaxes) {
                Ok(regions) => regions
                    .into_iter()
                    .map(|(style, text)| Run {
                        text: text.trim_end_matches('\n').to_string(),
                        color: (style.foreground.r, style.foreground.g, style.foreground.b),
                    })
                    .collect(),
                // A grammar error degrades to plain text, never blocks the diff.
                Err(_) => vec![Run {
                    text: line.trim_end_matches('\n').to_string(),
                    color: self.default_fg,
                }],
            };
            out.push(runs);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_tokenizes_into_multiple_colored_runs() {
        let h = Highlighter::new("dark");
        let lines = h.highlight("let x = 1;\n", Some("rs"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() > 1, "rust should split into several runs");
        let joined: String = lines[0].iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, "let x = 1;");
    }

    #[test]
    fn unknown_language_is_plain() {
        let h = Highlighter::new("dark");
        let lines = h.highlight("alpha\nbeta\n", None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].color, h.default_fg);
    }
}
