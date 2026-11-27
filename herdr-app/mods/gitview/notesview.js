// The notes list's rows: where (file:lines) over the note's first line,
// with a delete button on the hovered or selected row.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { iconButton, noteLines, pathLabel } from "./chrome.js";
import { noteShowsIn, removeNotes } from "./shared.js";
import { openCommit } from "./history.js";

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

// The section a note was written in (notes from before `section`: by side).
const noteSection = (note) => note.section ?? (note.cached ? "staged" : "changes");

// Historical notes reopen their commit; working notes use their saved
// section, else wherever the file is now. Keep the anchor's line side.
// A note only the other scope shows (shared.js noteShowsIn) says the way
// there (`w` is the file list's key); the scope is the loaded list's.
export function jumpToNote(v, note, cx) {
  if (!note || !v.work) return;
  const end = note.end || note.start || null;
  const line = end && note.side === "old" ? { o: end } : end;
  if (note.commit) return openCommit(v, { sha: note.commit, short: note.commit.slice(0, 7) }, cx, { path: note.file, line });
  const scope = v.work.status?.scope === "branch" ? "branch" : "worktree";
  const row = noteShowsIn(note, scope) ? noteFile(v.work, note, scope) : null;
  if (row) return v.setMode("files", cx, { row, line });
  if (scope === "worktree" && noteShowsIn(note, "branch")) return v.flash.set("it's in vs base: esc, then w");
  if (scope === "branch" && !noteShowsIn(note, "branch")) return v.flash.set("it's in uncommitted: esc, then w");
  v.flash.set(`${note.file} has no changes now`);
}

// The note's file in that scope's list. "vs base" has one section; a note
// written there is about the file on disk, so uncommitted shows it on the
// unstaged side.
function noteFile(work, note, scope) {
  if (scope === "branch") return work.findFile(note.file);
  if (note.section === "branch") return work.findFile(note.file, "changes") ?? work.findFile(note.file, "conflicts");
  return work.findFile(note.file, noteSection(note)) ?? work.findFile(note.file);
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
