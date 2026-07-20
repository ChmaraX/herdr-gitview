//! The structured diff model: old-vs-new content as rows with syntax
//! highlighting, red/green line backgrounds, word-level emphasis, and folded
//! context — ported (simplified) from persiyanov/herdr-reviewr `diff.rs` (MIT).

use std::path::Path;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use similar::{ChangeTag, TextDiff};

use super::highlight::{Highlighter, Rgb, Run};

/// A `[start, end)` run of char indices within a line, for word emphasis.
type CharRange = (u32, u32);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Context {
        old_no: u32,
        new_no: u32,
        runs: Vec<Run>,
    },
    Deletion {
        old_no: u32,
        runs: Vec<Run>,
        emphasis: Vec<CharRange>,
    },
    Insertion {
        new_no: u32,
        runs: Vec<Run>,
        emphasis: Vec<CharRange>,
    },
    /// A collapsed run of unchanged lines (kept, so a click can expand them).
    Fold { lines: Vec<Row> },
}

impl Row {
    fn text(&self) -> String {
        match self {
            Row::Context { runs, .. }
            | Row::Deletion { runs, .. }
            | Row::Insertion { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            Row::Fold { .. } => String::new(),
        }
    }
}

/// The built document, ready to render. Folds can be expanded in place
/// (`unfold_at`), which rebuilds `text`.
pub struct DiffDoc {
    rows: Vec<Row>,
    flavor: String,
    default_fg: Rgb,
    /// Rendered line index → index into `rows`.
    line_rows: Vec<usize>,
    pub text: Text<'static>,
    /// First inserted line's new-file number (for the editor jump).
    pub first_change: Option<u32>,
    pub is_empty: bool,
    pub binary: bool,
}

impl DiffDoc {
    /// Expand the fold at rendered line `line` (if that line is a fold).
    /// Returns true when something unfolded (the text was rebuilt).
    pub fn unfold_at(&mut self, line: usize) -> bool {
        let Some(&row_idx) = self.line_rows.get(line) else {
            return false;
        };
        if !matches!(self.rows.get(row_idx), Some(Row::Fold { .. })) {
            return false;
        }
        let Row::Fold { lines } = self.rows.remove(row_idx) else {
            unreachable!();
        };
        self.rows.splice(row_idx..row_idx, lines);
        self.rebuild();
        true
    }

    fn rebuild(&mut self) {
        let (text, line_rows) = to_text(&self.rows, &Palette::new(&self.flavor), self.default_fg);
        self.text = text;
        self.line_rows = line_rows;
    }
}

/// Colors for the diff chrome, themed light or dark to match the syntax theme.
struct Palette {
    ins_bg: Rgb,
    del_bg: Rgb,
    ins_emph_bg: Rgb,
    del_emph_bg: Rgb,
    gutter: Rgb,
}

impl Palette {
    fn new(flavor: &str) -> Palette {
        if flavor == "light" {
            // GitHub-web-like tints.
            Palette {
                ins_bg: (0xe6, 0xff, 0xec),
                del_bg: (0xff, 0xeb, 0xe9),
                ins_emph_bg: (0xab, 0xf2, 0xbc),
                del_emph_bg: (0xff, 0xc0, 0xc0),
                gutter: (0x9a, 0xa0, 0xa6),
            }
        } else {
            // Catppuccin-ish tints (reviewr's defaults).
            Palette {
                ins_bg: (0x1f, 0x3a, 0x2a),
                del_bg: (0x45, 0x23, 0x2f),
                ins_emph_bg: (0x30, 0x55, 0x3f),
                del_emph_bg: (0x6e, 0x34, 0x46),
                gutter: (0x6c, 0x70, 0x86),
            }
        }
    }
}

/// Context lines kept adjacent to each change; longer runs collapse to a fold.
const FOLD_MARGIN: usize = 3;

/// Build the full document from old and new file content.
pub fn build(path: &Path, old: &str, new: &str, hl: &Highlighter, flavor: &str) -> DiffDoc {
    if old.contains('\0') || new.contains('\0') {
        return DiffDoc {
            rows: Vec::new(),
            flavor: flavor.to_string(),
            default_fg: hl.default_fg,
            line_rows: Vec::new(),
            text: Text::default(),
            first_change: None,
            is_empty: false,
            binary: true,
        };
    }
    let ext = path.extension().and_then(|e| e.to_str());
    let old_lines = hl.highlight(old, ext);
    let new_lines = hl.highlight(new, ext);
    let line = |lines: &[Vec<Run>], i: usize| lines.get(i).cloned().unwrap_or_default();

    let mut rows = Vec::new();
    for change in TextDiff::from_lines(old, new).iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                let (oi, ni) = (change.old_index().unwrap(), change.new_index().unwrap());
                rows.push(Row::Context {
                    old_no: oi as u32 + 1,
                    new_no: ni as u32 + 1,
                    runs: line(&new_lines, ni),
                });
            }
            ChangeTag::Delete => {
                let oi = change.old_index().unwrap();
                rows.push(Row::Deletion {
                    old_no: oi as u32 + 1,
                    runs: line(&old_lines, oi),
                    emphasis: Vec::new(),
                });
            }
            ChangeTag::Insert => {
                let ni = change.new_index().unwrap();
                rows.push(Row::Insertion {
                    new_no: ni as u32 + 1,
                    runs: line(&new_lines, ni),
                    emphasis: Vec::new(),
                });
            }
        }
    }

    let is_empty = !rows
        .iter()
        .any(|r| matches!(r, Row::Deletion { .. } | Row::Insertion { .. }));
    let first_change = rows.iter().find_map(|r| match r {
        Row::Insertion { new_no, .. } => Some(*new_no),
        _ => None,
    });

    compute_emphasis(&mut rows);
    let rows = collapse_context(rows);
    let (text, line_rows) = to_text(&rows, &Palette::new(flavor), hl.default_fg);
    DiffDoc {
        rows,
        flavor: flavor.to_string(),
        default_fg: hl.default_fg,
        line_rows,
        text,
        first_change,
        is_empty,
        binary: false,
    }
}

// ---- word-level emphasis ---------------------------------------------------

/// Two lines below this similarity are different lines, not one line edited.
const MIN_SIMILARITY: f32 = 0.7;

/// For each change block (a run of deletions followed by a run of insertions),
/// pair each deletion with its first similar-enough insertion and mark the
/// changed char ranges on both (git-delta's homolog inference, simplified).
fn compute_emphasis(rows: &mut [Row]) {
    let mut i = 0;
    while i < rows.len() {
        let del_start = i;
        while i < rows.len() && matches!(rows[i], Row::Deletion { .. }) {
            i += 1;
        }
        let ins_start = i;
        while i < rows.len() && matches!(rows[i], Row::Insertion { .. }) {
            i += 1;
        }
        pair_homologs(rows, del_start..ins_start, ins_start..i);
        if del_start == i {
            i += 1; // no change block here; step over the context row
        }
    }
}

fn pair_homologs(rows: &mut [Row], dels: std::ops::Range<usize>, inss: std::ops::Range<usize>) {
    let mut next_ins = inss.start;
    for d in dels {
        let old = rows[d].text();
        let mut p = next_ins;
        while p < inss.end {
            let new = rows[p].text();
            let (ratio, old_e, new_e) = word_emphasis(&old, &new);
            if ratio >= MIN_SIMILARITY {
                if let Row::Deletion { emphasis, .. } = &mut rows[d] {
                    *emphasis = old_e;
                }
                if let Row::Insertion { emphasis, .. } = &mut rows[p] {
                    *emphasis = new_e;
                }
                next_ins = p + 1;
                break;
            }
            p += 1;
        }
    }
}

/// Word-level similarity of `(old, new)` plus the changed char ranges on each
/// side, whitespace-trimmed and coalesced across pure-whitespace gaps.
fn word_emphasis(old: &str, new: &str) -> (f32, Vec<CharRange>, Vec<CharRange>) {
    let diff = TextDiff::from_words(old, new);
    let (mut old_ranges, mut new_ranges) = (Vec::new(), Vec::new());
    let (mut old_pos, mut new_pos) = (0u32, 0u32);
    for change in diff.iter_all_changes() {
        let len = change.value().chars().count() as u32;
        match change.tag() {
            ChangeTag::Equal => {
                old_pos += len;
                new_pos += len;
            }
            ChangeTag::Delete => {
                push_range(&mut old_ranges, old_pos, len);
                old_pos += len;
            }
            ChangeTag::Insert => {
                push_range(&mut new_ranges, new_pos, len);
                new_pos += len;
            }
        }
    }
    let old_e = trim_edges(coalesce(old_ranges, old), old);
    let new_e = trim_edges(coalesce(new_ranges, new), new);
    (diff.ratio(), old_e, new_e)
}

fn push_range(ranges: &mut Vec<CharRange>, pos: u32, len: u32) {
    if len == 0 {
        return;
    }
    match ranges.last_mut() {
        Some(last) if last.1 == pos => last.1 = pos + len,
        _ => ranges.push((pos, pos + len)),
    }
}

/// Merge ranges whose gap is all whitespace, so a changed phrase is one block.
fn coalesce(ranges: Vec<CharRange>, text: &str) -> Vec<CharRange> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<CharRange> = Vec::new();
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last)
                if chars[last.1 as usize..start as usize]
                    .iter()
                    .all(|c| c.is_whitespace()) =>
            {
                last.1 = end;
            }
            _ => out.push((start, end)),
        }
    }
    out
}

/// Shrink each range off leading/trailing whitespace; drop all-whitespace ones.
fn trim_edges(ranges: Vec<CharRange>, text: &str) -> Vec<CharRange> {
    let chars: Vec<char> = text.chars().collect();
    ranges
        .into_iter()
        .filter_map(|(mut a, mut b)| {
            while a < b && chars[a as usize].is_whitespace() {
                a += 1;
            }
            while b > a && chars[b as usize - 1].is_whitespace() {
                b -= 1;
            }
            (a < b).then_some((a, b))
        })
        .collect()
}

// ---- folding ---------------------------------------------------------------

/// Collapse runs of unchanged context beyond `FOLD_MARGIN` around each change.
fn collapse_context(rows: Vec<Row>) -> Vec<Row> {
    let n = rows.len();
    let mut keep = vec![false; n];
    for (i, row) in rows.iter().enumerate() {
        if matches!(row, Row::Context { .. }) {
            continue;
        }
        let lo = i.saturating_sub(FOLD_MARGIN);
        let hi = (i + FOLD_MARGIN).min(n.saturating_sub(1));
        keep[lo..=hi].iter_mut().for_each(|k| *k = true);
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if keep[i] {
            out.push(rows[i].clone());
            i += 1;
            continue;
        }
        let start = i;
        while i < n && !keep[i] {
            i += 1;
        }
        if i - start > 1 {
            out.push(Row::Fold {
                lines: rows[start..i].to_vec(),
            });
        } else {
            out.extend(rows[start..i].iter().cloned());
        }
    }
    out
}

// ---- painting --------------------------------------------------------------

/// Chars of gutter + marker before the content on a change line
/// (`"{:>4}     "` or `"     {:>4}"` = 9, plus `"- "`/`"+ "` = 2).
const GUTTER_CHARS: u32 = 11;

/// Rows → ratatui text plus a rendered-line → row-index map (for clicks):
/// `old new marker` gutter, tinted backgrounds on change lines, stronger tint
/// on emphasized ranges.
fn to_text(rows: &[Row], p: &Palette, default_fg: Rgb) -> (Text<'static>, Vec<usize>) {
    let gutter_style = Style::new().fg(rgb(p.gutter));
    let mut lines = Vec::with_capacity(rows.len());
    let mut line_rows = Vec::with_capacity(rows.len());
    for (row_idx, row) in rows.iter().enumerate() {
        let line = match row {
            Row::Fold { lines: hidden } => Line::from(Span::styled(
                format!(
                    "      ▸ ⋯ {} unchanged lines — click to expand",
                    hidden.len()
                ),
                gutter_style,
            )),
            Row::Context {
                old_no,
                new_no,
                runs,
            } => {
                let mut spans = vec![Span::styled(
                    format!("{old_no:>4} {new_no:>4}   "),
                    gutter_style,
                )];
                spans.extend(paint(runs, None, &[], default_fg));
                Line::from(spans)
            }
            Row::Deletion {
                old_no,
                runs,
                emphasis,
            } => {
                let mut spans = vec![
                    Span::styled(format!("{old_no:>4}      "), gutter_style),
                    Span::styled(
                        "- ".to_string(),
                        Style::new().fg(Color::Red).bg(rgb(p.del_bg)),
                    ),
                ];
                spans.extend(paint(runs, Some(p.del_bg), emphasis, default_fg));
                line_with_bg(spans, p.del_bg)
            }
            Row::Insertion {
                new_no,
                runs,
                emphasis,
            } => {
                let mut spans = vec![
                    Span::styled(format!("     {new_no:>4}"), gutter_style),
                    Span::styled(
                        "+ ".to_string(),
                        Style::new().fg(Color::Green).bg(rgb(p.ins_bg)),
                    ),
                ];
                spans.extend(paint(runs, Some(p.ins_bg), emphasis, default_fg));
                line_with_bg(spans, p.ins_bg)
            }
        };
        let line = match row {
            Row::Deletion { emphasis, .. } if !emphasis.is_empty() => {
                repaint_emphasis(line, GUTTER_CHARS, emphasis, p.del_emph_bg)
            }
            Row::Insertion { emphasis, .. } if !emphasis.is_empty() => {
                repaint_emphasis(line, GUTTER_CHARS, emphasis, p.ins_emph_bg)
            }
            _ => line,
        };
        lines.push(line);
        line_rows.push(row_idx);
    }
    (Text::from(lines), line_rows)
}

/// Syntax runs → spans, with an optional background tint.
fn paint(
    runs: &[Run],
    bg: Option<Rgb>,
    _emphasis: &[CharRange],
    _default_fg: Rgb,
) -> Vec<Span<'static>> {
    runs.iter()
        .map(|r| {
            let mut style = Style::new().fg(rgb(r.color));
            if let Some(bg) = bg {
                style = style.bg(rgb(bg));
            }
            Span::styled(r.text.clone(), style)
        })
        .collect()
}

/// Extend a change line's background across the full width by styling the line.
fn line_with_bg(spans: Vec<Span<'static>>, bg: Rgb) -> Line<'static> {
    Line::from(spans).style(Style::new().bg(rgb(bg)))
}

/// Re-style the emphasized char ranges of a change line with a stronger
/// background. `prefix` is the gutter+marker char count before content.
fn repaint_emphasis(
    line: Line<'static>,
    prefix: u32,
    ranges: &[CharRange],
    emph_bg: Rgb,
) -> Line<'static> {
    let bg = rgb(emph_bg);
    let style = line.style;
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut pos: u32 = 0; // char position across the whole line
    for span in line.spans {
        let chars: Vec<char> = span.content.chars().collect();
        let len = chars.len() as u32;
        let start = pos;
        // Split this span wherever emphasis ranges (shifted by prefix) cut it.
        let mut cursor = 0u32;
        while cursor < len {
            let abs = start + cursor;
            let content_pos = abs.saturating_sub(prefix);
            let in_emph = abs >= prefix
                && ranges
                    .iter()
                    .any(|&(a, b)| content_pos >= a && content_pos < b);
            // Find the run length with the same emphasis status.
            let mut end = cursor + 1;
            while end < len {
                let abs2 = start + end;
                let cp2 = abs2.saturating_sub(prefix);
                let in2 = abs2 >= prefix && ranges.iter().any(|&(a, b)| cp2 >= a && cp2 < b);
                if in2 != in_emph {
                    break;
                }
                end += 1;
            }
            let text: String = chars[cursor as usize..end as usize].iter().collect();
            let mut st = span.style;
            if in_emph {
                st = st.bg(bg);
            }
            out.push(Span::styled(text, st));
            cursor = end;
        }
        pos += len;
    }
    Line::from(out).style(style)
}

fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(old: &str, new: &str) -> DiffDoc {
        let hl = Highlighter::new("dark");
        build(&PathBuf::from("a.rs"), old, new, &hl, "dark")
    }

    #[test]
    fn empty_when_content_equal() {
        let d = doc("same\n", "same\n");
        assert!(d.is_empty);
        assert!(!d.binary);
    }

    #[test]
    fn binary_detected_by_nul() {
        let d = doc("ok\n", "bin\0ary\n");
        assert!(d.binary);
    }

    #[test]
    fn first_change_is_first_inserted_line() {
        let d = doc("a\nb\nc\n", "a\nb\nX\nc\n");
        assert_eq!(d.first_change, Some(3));
    }

    #[test]
    fn change_lines_carry_backgrounds_and_gutter_numbers() {
        let d = doc("alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n");
        let flat: Vec<String> = d
            .text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let del = flat.iter().find(|l| l.contains("beta")).unwrap();
        let ins = flat.iter().find(|l| l.contains("BETA")).unwrap();
        assert!(del.contains("- "), "deletion marker: {del:?}");
        assert!(ins.contains("+ "), "insertion marker: {ins:?}");
        assert!(del.trim_start().starts_with('2'), "old line no: {del:?}");
        // Deletion line has the red tint as line style.
        let del_line = d
            .text
            .lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("beta")));
        assert!(del_line.unwrap().style.bg.is_some());
    }

    #[test]
    fn long_context_folds() {
        let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 20", "LINE 20");
        let d = doc(&old, &new);
        let flat: String = d
            .text
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(flat.contains("unchanged lines"), "should fold: {flat}");
    }

    #[test]
    fn clicking_a_fold_expands_it() {
        let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 20", "LINE 20");
        let mut d = doc(&old, &new);
        let folded_len = d.text.lines.len();
        // First rendered line is the leading fold.
        assert!(d.unfold_at(0), "line 0 should be a fold");
        assert!(d.text.lines.len() > folded_len, "unfolding adds lines");
        // A content line is not a fold.
        assert!(!d.unfold_at(1));
    }

    #[test]
    fn word_emphasis_marks_changed_words() {
        let (ratio, old_e, new_e) = word_emphasis("let x = foo(a);", "let x = bar(a);");
        assert!(ratio >= MIN_SIMILARITY);
        assert_eq!(old_e.len(), 1);
        assert_eq!(new_e.len(), 1);
        let seg = |text: &str, (a, b): CharRange| -> String {
            text.chars()
                .skip(a as usize)
                .take((b - a) as usize)
                .collect()
        };
        // similar tokenizes on whitespace, so the changed "word" is foo(a);
        assert!(seg("let x = foo(a);", old_e[0]).contains("foo"));
        assert!(seg("let x = bar(a);", new_e[0]).contains("bar"));
        // the shared prefix is never emphasized
        assert!(old_e[0].0 > 0);
    }

    #[test]
    fn dissimilar_lines_get_no_emphasis() {
        let (ratio, _, _) = word_emphasis(
            "let start = scroll.min(len);",
            "return (row - inner.y) as usize;",
        );
        assert!(ratio < MIN_SIMILARITY);
    }
}
