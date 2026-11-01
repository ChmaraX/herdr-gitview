// The diff, as herdr-gitview renders it for its TUI (syntax colors, red/green
// tints, word emphasis, folds), plus the review notes as cards under the lines
// they're about. Pure functions of the view's state: rows in, elements out.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";

export const ROW = 18;
const NOTE_LINE = 16;

// Diff rows + note cards in display order. A card follows the last row of
// its range (new-side numbers), a whole-file note goes first.
export function displayRows(diff, notes) {
  const out = [];
  const pending = notes.filter((n) => n.end > 0);
  for (const n of notes.filter((n) => n.end === 0)) out.push({ note: n });
  diff.rows.forEach((row, i) => {
    out.push({ row, index: i });
    const here = row.n ?? null;
    if (here === null) return;
    for (const n of pending.filter((n) => n.end === here)) out.push({ note: n });
  });
  return out;
}

export function rowSize(item) {
  if (!item.note) return ROW;
  return 22 + NOTE_LINE * item.note.text.split("\n").length + 8;
}

// The engine's tint roles (`@ins`, …) and marker colors, in the app's theme:
// the diff follows any theme, light or dark, like the rest of the panel.
function color(c, t) {
  switch (c) {
    case "red": return t.ansi1;
    case "green": return t.ansi2;
    case "@ins": return `${t.ansi2}26`;
    case "@del": return `${t.ansi1}26`;
    case "@ins_emph": return `${t.ansi2}5c`;
    case "@del_emph": return `${t.ansi1}5c`;
    case "@gutter": return t.dim;
    case "@fold": return t.raised;
    case "@fold_fg": return t.muted;
    default: return c;
  }
}

export function renderRow(item, ctx) {
  const { t, cursor, selection } = ctx;
  if (item.note) return noteCard(item.note, t);
  const { row, index } = item;
  const selected = selection && index >= selection[0] && index <= selection[1];
  const line = h_flex()
    .id(`d${index}`)
    .w_full()
    .h(ROW)
    .flex_none()
    .overflow_hidden()
    .when(Boolean(row.bg), (e) => e.bg(color(row.bg, t)))
    .when(selected, (e) => e.bg(t.selection))
    .when(index === cursor && ctx.focused, (e) => e.border_l(2).border_color(t.accent))
    .children(
      row.s.map(([text, fg, bg]) => {
        const span = div().flex_none().child(text.replace(/ /g, "\u00a0"));
        return span.when(Boolean(fg), (e) => e.text_color(color(fg, t))).when(Boolean(bg) && !selected, (e) => e.bg(color(bg, t)));
      }),
    );
  return line;
}

function noteCard(note, t) {
  const where = note.end === 0 ? "whole file" : note.start === note.end ? `line ${note.start}` : `lines ${note.start}-${note.end}`;
  return v_flex()
    .w_full()
    .flex_none()
    .py(4)
    .pl(44)
    .pr(8)
    .child(
      v_flex()
        .rounded(6)
        .border_1()
        .border_color(t.accent)
        .bg(t.raised)
        .px(8)
        .py(3)
        .child(div().text_size(11).text_color(t.accent).child(`note · ${where}`))
        .children(note.text.split("\n").map((l) => div().h(NOTE_LINE).text_color(t.foreground).truncate().child(l || " "))),
    );
}

// The selection's anchor for a note: new-side numbers of the rows in
// [a, b], and the TUI's snippet (marker + text, at most 40 lines).
export function noteRange(diff, a, b) {
  const nums = [];
  let snippet = "";
  for (let i = a; i <= b; i++) {
    const r = diff.rows[i];
    if (!r || r.k === "fold") continue;
    nums.push(r.n ?? r.o ?? 0);
    if (r.m && snippet.split("\n").length <= 40) snippet += `${r.m}\n`;
  }
  const real = nums.filter((n) => n > 0);
  if (!real.length) return null;
  return { start: Math.min(...real), end: Math.max(...real), snippet };
}
