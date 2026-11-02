// The diff, as herdr-gitview renders it for its TUI (syntax colors, red/green
// tints, word emphasis, folds, line numbers), plus the review notes as cards
// under the lines they're about and the note composer under a selection.
// Rows in, elements out; the pane passes the handlers.

import { div } from "gpui-kit";
import { h_flex, v_flex, Textarea } from "gpui-base";
import { iconButton } from "./listview.js";

export const ROW = 18;
const NOTE_LINE = 16;
export const COMPOSER = 104;

// Diff rows, note cards and the composer in display order. A card follows
// the last row of its range (new-side numbers), a whole-file note goes
// first; the composer follows the selection's last row.
export function displayRows(diff, notes, composerAfter) {
  const out = [];
  const pending = notes.filter((n) => n.end > 0);
  for (const n of notes.filter((n) => n.end === 0)) out.push({ note: n });
  diff.rows.forEach((row, i) => {
    out.push({ row, index: i });
    const here = row.n ?? null;
    if (here !== null) for (const n of pending.filter((n) => n.end === here)) out.push({ note: n });
    if (i === composerAfter) out.push({ composer: true });
  });
  return out;
}

export function rowSize(item) {
  if (item.composer) return COMPOSER;
  if (!item.note) return ROW;
  return 30 + NOTE_LINE * item.note.text.split("\n").length + 8;
}

// The engine's tint roles (`@ins`, …) and marker colors, in the app's theme:
// the diff follows any theme, light or dark.
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

// ctx: {t, cursor, selection: [a, b] | null, focused, composer (TextareaState),
// pending (the note being written), onDown(index, event, cx), onMove(index, cx),
// onDeleteNote(note, cx)}
export function renderRow(item, ctx) {
  const { t } = ctx;
  if (item.note) return noteCard(item.note, ctx);
  if (item.composer) return composer(ctx);
  const { row, index } = item;
  const selected = ctx.selection && index >= ctx.selection[0] && index <= ctx.selection[1];
  return h_flex()
    .id(`d${index}`)
    .w_full()
    .h(ROW)
    .flex_none()
    .overflow_hidden()
    .when(row.k === "fold", (e) => e.cursor_pointer())
    .when(Boolean(row.bg), (e) => e.bg(color(row.bg, t)))
    .when(selected, (e) => e.bg(t.selection))
    .when(index === ctx.cursor, (e) => e.border_l(2).border_color(ctx.focused ? t.accent : t.dim))
    .on_mouse_down("left", (e, cx) => ctx.onDown(index, e, cx))
    .on_mouse_move((_e, cx) => ctx.onMove(index, cx))
    .children(
      row.s.map(([text, fg, bg]) => {
        const span = div().flex_none().child(text.replace(/ /g, "\u00a0"));
        return span.when(Boolean(fg), (e) => e.text_color(color(fg, t))).when(Boolean(bg) && !selected, (e) => e.bg(color(bg, t)));
      }),
    );
}

export const where = (n) => (n.end === 0 ? "whole file" : n.start === n.end ? `line ${n.start}` : `lines ${n.start}-${n.end}`);

function noteCard(note, ctx) {
  const { t } = ctx;
  return v_flex()
    .id(`note-${note.id}`)
    .w_full()
    .flex_none()
    .py(4)
    .pl(44)
    .pr(10)
    .font_family(ctx.uiFont)
    .child(
      v_flex()
        .rounded(8)
        .border_1()
        .border_color(t.accent)
        .bg(t.raised)
        .pl(10)
        .pr(4)
        .py(4)
        .child(
          h_flex()
            .h(20)
            .items_center()
            .child(div().flex_1().text_size(11).text_color(t.accent).child(`note · ${where(note)}`))
            .child(iconButton(t, `note-x-${note.id}`, "x", "Delete note", (cx) => ctx.onDeleteNote(note, cx), { size: 18 })),
        )
        .children(note.text.split("\n").map((l) => div().h(NOTE_LINE).text_size(12).text_color(t.foreground).truncate().child(l || " "))),
    );
}

function composer(ctx) {
  const { t } = ctx;
  return v_flex()
    .id("gitview-composer")
    .w_full()
    .h(COMPOSER)
    .flex_none()
    .py(4)
    .pl(44)
    .pr(10)
    .font_family(ctx.uiFont)
    .child(
      v_flex()
        .rounded(8)
        .border_1()
        .border_color(t.accent)
        .bg(t.surface)
        .px(10)
        .py(6)
        .gap(4)
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .child(div().text_size(11).text_color(t.accent).child(`new note · ${where(ctx.pending)} · enter saves · esc cancels`))
        .child(Textarea.new(ctx.composer).h(52).text_size(12).text_color(t.foreground)),
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
