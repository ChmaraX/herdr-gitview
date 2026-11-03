// The notes list's rows: where (file:lines) over the note's first line,
// with a delete button on the hovered or selected row.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { iconButton, noteLines, pathLabel } from "./chrome.js";
import { removeNotes } from "./shared.js";

export const NOTE_ROW = 44;

// ---- what the list does in it (v: the list's view) ---------------------

export function noteMove(v, delta) {
  const n = v.notes().length;
  if (!n) return;
  v.noteSel = Math.min(n - 1, Math.max(0, v.noteSel + delta));
  v.reveal(v.noteSel, { margin: 8 });
}

export function deleteNote(v, note) {
  if (note) removeNotes(v.notesKey(), [note.id]);
}

// The note's file in the diff pane, its cursor on the note.
export function jumpToNote(v, note, cx) {
  if (!note || !v.work) return;
  v.mode = "files";
  const row = v.work.findFile(note.file, "changes") ?? v.work.findFile(note.file);
  if (!row) return v.flash.set(`${note.file} has no changes now`);
  v.work.selected = row.key;
  v.reveal(v.work.index(row.key), { center: true });
  v.show(row, { open: true, cx, line: note.end || note.start || null });
}

// ctx: {t, selected (index), hover (id), focused, onClick(i, event, cx), onHover, onDelete}
export function noteRow(n, i, ctx) {
  const { t } = ctx;
  const on = i === ctx.selected;
  const tools = on || ctx.hover === n.id;
  return h_flex()
    .id(`note-${n.id}`)
    .h(NOTE_ROW)
    .flex_none()
    .mx(6)
    .pl(10)
    .pr(4)
    .gap(6)
    .items_center()
    .rounded(7)
    .cursor_pointer()
    .when(on, (e) => e.bg(ctx.focused ? t.selected : t.hover))
    .when(!on, (e) => e.hover((s) => s.bg(t.hover)))
    .on_hover((h, cx) => ctx.onHover(n, h, cx))
    .on_click((e, cx) => ctx.onClick(i, e, cx))
    .child(
      v_flex()
        .flex_1()
        .min_w(0)
        .gap(2)
        .child(h_flex().min_w(0).child(pathLabel(t, n.file, { size: 12, grow: false })).child(div().flex_none().text_size(12).text_color(t.accent).child(`:${noteLines(n)}`)))
        .child(div().min_w(0).truncate().text_size(11.5).text_color(t.muted).child(n.text.split("\n")[0])),
    )
    .when(tools, (e) => e.child(iconButton(t, `note-del-${n.id}`, "x", "Delete note (d)", (cx) => ctx.onDelete(n, cx))));
}
