//! Syntax highlighting via syntect + two-face (bat's broad syntax set),
//! after persiyanov/herdr-reviewr's `highlight.rs` (MIT), plus the
//! grammars two-face lacks (Gherkin), embedded from `assets/syntaxes/`.
//!
//! Produces per-line runs of `(text, fg-rgb)`; the renderer adds diff
//! backgrounds on top, so only token colors come from the syntax theme.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::parsing::{SyntaxDefinition, SyntaxReference, SyntaxSet, SyntaxSetBuilder};
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

/// Grammars two-face doesn't have, embedded as `.sublime-syntax` sources.
const EXTRA_GRAMMARS: &[&str] = &[include_str!("../../assets/syntaxes/Gherkin.sublime-syntax")];

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

/// `EXTRA_GRAMMARS`, compiled once per process into a set of their own:
/// adding them to the broad set would rebuild all of it, and the app starts
/// a new process for every op. A grammar is linked against its own set, so
/// each lookup returns the set that holds it.
fn extra_syntaxes() -> &'static SyntaxSet {
    static EXTRA: OnceLock<SyntaxSet> = OnceLock::new();
    EXTRA.get_or_init(|| {
        let mut builder = SyntaxSetBuilder::new();
        for source in EXTRA_GRAMMARS {
            // Embedded at build time and covered by the tests below.
            let grammar = SyntaxDefinition::load_from_str(source, true, None)
                .expect("an embedded grammar parses");
            builder.add(grammar);
        }
        builder.build()
    })
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

    /// The grammar for a file extension and the set it belongs to, if we
    /// know the language. An exact extension match in either set wins over
    /// the broad set's looser token match; the extra set is only built when
    /// the broad one has no such extension.
    fn syntax_for(
        extension: Option<&str>,
    ) -> Option<(&'static SyntaxReference, &'static SyntaxSet)> {
        let ext = extension?;
        let broad = syntaxes();
        if let Some(syntax) = broad.find_syntax_by_extension(ext) {
            return Some((syntax, broad));
        }
        let extra = extra_syntaxes();
        if let Some(syntax) = extra.find_syntax_by_extension(ext) {
            return Some((syntax, extra));
        }
        broad
            .find_syntax_by_token(ext)
            .map(|syntax| (syntax, broad))
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
        let Some((syntax, set)) = Self::syntax_for(extension) else {
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
                let runs = self.highlight_one(&mut h, line, set);
                if want.peek() == Some(&i) {
                    want.next();
                    out.push((i, runs));
                }
            }
        }
        Some(out)
    }

    fn highlight_one(&self, h: &mut HighlightLines, line: &str, set: &SyntaxSet) -> Vec<Run> {
        match h.highlight_line(line, set) {
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
        let Some((syntax, set)) = Self::syntax_for(extension) else {
            return content.lines().map(|l| self.plain(l)).collect();
        };
        let mut h = HighlightLines::new(syntax, &self.theme);
        LinesWithEndings::from(content)
            .map(|line| self.highlight_one(&mut h, line, set))
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

    /// The color of the first run whose text is `word` on `line`.
    fn color_of(line: &[Run], word: &str) -> Rgb {
        line.iter()
            .find(|r| r.text.trim() == word)
            .unwrap_or_else(|| panic!("no run {word:?} in {line:?}"))
            .color
    }

    /// `highlight` parses with the set that holds the grammar.
    #[test]
    fn gherkin_file_gets_colors() {
        let h = Highlighter::new(crate::config::Theme::Dark);
        let lines = h.highlight("Feature: x\n  Given a user\n", Some("feature"));
        assert_ne!(color_of(&lines[1], "Given"), h.default_fg);
    }

    /// `highlight_lines` parses with the set that holds the grammar.
    #[test]
    fn gherkin_diff_lines_get_colors() {
        let h = Highlighter::new(crate::config::Theme::Light);
        let lines = ["Feature: x\n", "  Then done\n"];
        let out = h
            .highlight_lines(&lines, Some("feature"), &[1], &mut || true)
            .expect("not abandoned");
        assert_ne!(color_of(&out[0].1, "Then"), h.default_fg);
    }

    /// The scopes of the Gherkin grammar at the first `needle` on line
    /// `line` (0-based) of `src`, outermost first. Theme-independent, so
    /// the grammar tests below say what each token is, not how it looks.
    fn gherkin_scopes(src: &str, line: usize, needle: &str) -> String {
        use syntect::parsing::{ParseState, ScopeStack};
        let set = extra_syntaxes();
        let syntax = set.find_syntax_by_extension("feature").expect("gherkin");
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        for (i, text) in LinesWithEndings::from(src).enumerate() {
            let ops = state.parse_line(text, set).expect("parses");
            // Other lines apply all their ops; this one those up to the needle.
            let at = if i == line {
                text.find(needle)
                    .unwrap_or_else(|| panic!("no {needle:?} in {text:?}"))
            } else {
                usize::MAX
            };
            for (_, op) in ops.iter().take_while(|(pos, _)| *pos <= at) {
                stack.apply(op).expect("valid scope op");
            }
            if i == line {
                let names: Vec<String> =
                    stack.as_slice().iter().map(|s| s.build_string()).collect();
                return names.join(" ");
            }
        }
        panic!("{src:?} has no line {line}");
    }

    #[track_caller]
    fn assert_scope(src: &str, line: usize, needle: &str, scope: &str) {
        let got = gherkin_scopes(src, line, needle);
        assert!(got.contains(scope), "{needle:?}: want {scope}, got {got:?}");
    }

    #[track_caller]
    fn assert_no_scope(src: &str, line: usize, needle: &str, scope: &str) {
        let got = gherkin_scopes(src, line, needle);
        assert!(
            !got.contains(scope),
            "{needle:?}: unwanted {scope} in {got:?}"
        );
    }

    #[test]
    fn gherkin_sections_are_keywords_with_a_title() {
        let src = "Feature: Log in\n  Scenario Outline: as <role>\n  Feature : not one\n";
        assert_scope(src, 0, "Feature", "keyword.declaration");
        assert_scope(src, 0, "Log in", "entity.name.section");
        assert_scope(src, 1, "<role>", "variable.parameter");
        assert_no_scope(src, 2, "Feature", "keyword");
    }

    #[test]
    fn gherkin_steps_start_with_a_keyword_and_a_space() {
        let src = "Feature: x\n  Given a user\n  * any\n";
        assert_scope(src, 1, "Given", "keyword.other.step");
        assert_eq!(gherkin_scopes(src, 1, "a user"), "source.gherkin");
        assert_scope(src, 2, "*", "keyword.other.step");
    }

    #[test]
    fn gherkin_tags_start_a_line_and_may_end_in_a_comment() {
        let src = "@smoke @c#d # flaky\nFeature: x\n  # Given y\n  Mail support@example.com\n";
        assert_scope(src, 0, "#d", "entity.name.tag");
        assert_scope(src, 0, "# flaky", "comment.line");
        assert_scope(src, 2, "Given", "comment.line");
        assert_no_scope(src, 3, "@example", "entity.name.tag");
    }

    #[test]
    fn gherkin_step_arguments() {
        let src = "Feature: x\n  Given \"bob\" has <n> items, x < 5 and y > 3 for user1 at 7.\n";
        assert_scope(src, 1, "bob", "string.quoted.double");
        assert_no_scope(src, 1, "has", "string");
        assert_scope(src, 1, "<n>", "variable.parameter");
        assert_no_scope(src, 1, "< 5", "variable.parameter");
        assert_no_scope(src, 1, "1 at", "constant.numeric");
        assert_scope(src, 1, "7.", "constant.numeric");
    }

    #[test]
    fn gherkin_table_cells_split_on_unescaped_pipes() {
        let src = "Feature: x\n  | a \\| b | 42 |\n  | 5\" | 7 |\n  Then y\n";
        assert_scope(src, 1, "| b", "constant.character.escape");
        assert_scope(src, 1, "| 42", "punctuation.separator.table-cell");
        assert_scope(src, 2, "7", "constant.numeric");
        assert_scope(src, 3, "Then", "keyword.other.step");
    }

    #[test]
    fn gherkin_strings_end_with_their_line_and_docstrings_span_lines() {
        let src = "Feature: x\n  Given \"open\n  When y\n    \"\"\"\n    Then inside\n    \"\"\"\n  Then out\n    ```json\n    \"\"\"\n    ```\n  And done\n";
        assert_scope(src, 2, "When", "keyword.other.step");
        assert_scope(src, 4, "Then", "string.quoted.triple");
        assert_scope(src, 6, "Then", "keyword.other.step");
        assert_scope(src, 10, "And", "keyword.other.step");
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
