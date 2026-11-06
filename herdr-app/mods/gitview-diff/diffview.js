// The diff, as herdr-gitview renders it for its TUI (syntax colors, red/green
// tints, word emphasis, folds, line numbers), plus the review notes as cards
// under the lines they're about and the note composer under a selection.
// Rows in, elements out; the pane passes the handlers.

import { div } from "gpui-kit";
import { h_flex, v_flex, Textarea } from "gpui-base";
import { iconButton, noteLines } from "./chrome.js";

export const ROW = 18;
const NOTE_LINE = 16;
export const COMPOSER = 104;

// Diff rows, note cards and the composer in display order. A card follows
// the last row of its range (new-side numbers), a whole-file note goes
// first; the composer follows the selection's last row; a cut diff ends
// with {cut: rows}.
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
  // The engine sends 5 000 rows at most: say where it stopped.
  if (diff.truncated) out.push({ cut: diff.rows.length });
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

// ---- legible text on tinted rows -------------------------------------------
// The syntax colors are gitview's (One Dark / One Light) and the tints are
// the theme's own, so a dim color (a comment) can vanish on a green or red
// line in some themes. A span's color that doesn't reach MIN_CONTRAST on
// what's really behind it is lifted toward the theme's foreground just
// enough; colors that already read keep their hue.
const MIN_CONTRAST = 3.5;
const legible = new Map();

function rgba(hex) {
  const h = String(hex).replace("#", "");
  if (!/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(h)) return null;
  const n = (i) => parseInt(h.slice(i, i + 2), 16);
  return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
}

// `top` (maybe translucent) over an opaque `under`.
function over(top, under) {
  const a = top[3];
  return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1);
}

function luminance(c) {
  const f = (x) => {
    x /= 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const hex = (c) => `#${c.slice(0, 3).map((x) => Math.round(x).toString(16).padStart(2, "0")).join("")}`;

// `fg` as drawn on the layers in `behind` (bottom first, over t.background).
function readable(fg, behind, t) {
  const key = `${fg}|${behind.join(",")}|${t.background}|${t.foreground}`;
  if (legible.has(key)) return legible.get(key);
  let out = fg;
  const f = rgba(fg), base = rgba(t.background), ink = rgba(t.foreground);
  if (f && base && ink) {
    const bg = behind.map(rgba).filter(Boolean).reduce((under, top) => over(top, under), base);
    let c = f;
    for (let step = 1; step <= 20 && contrast(c, bg) < MIN_CONTRAST; step++) c = [0, 1, 2].map((i) => f[i] + (ink[i] - f[i]) * step / 20).concat(1);
    out = c === f ? fg : hex(c);
  }
  legible.set(key, out);
  return out;
}

// ctx: {t, cursor, selection: [a, b] | null, focused, composer (TextareaState),
// pending (the note being written), onDown(index, event, cx), onMove(index, cx),
// onDeleteNote(note, cx)}
export function renderRow(item, ctx) {
  const { t } = ctx;
  if (item.note) return noteCard(item.note, ctx);
  if (item.composer) return composer(ctx);
  if (item.cut) return div().h(ROW).flex_none().px(14).text_color(t.dim).child(`diff cut at ${String(item.cut).replace(/\B(?=(\d{3})+(?!\d))/g, ",")} lines`);
  const { row, index } = item;
  const selected = ctx.selection && index >= ctx.selection[0] && index <= ctx.selection[1];
  const cursor = index === ctx.cursor ? cursorTint(t, ctx.focused) : null;
  return h_flex()
    .id(`d${index}`)
    .relative()
    .w_full()
    .h(ROW)
    .flex_none()
    .overflow_hidden()
    .when(row.k === "fold", (e) => e.cursor_pointer())
    .when(Boolean(row.bg), (e) => e.bg(color(row.bg, t)))
    .when(selected, (e) => e.bg(t.selection))
    // The cursor: a faint wash of the text colour over the whole row (on top
    // of a green or red row too) and a 2 px bar, both laid over the row so
    // nothing moves. Painted before the text, so the text stays on top.
    .when(Boolean(cursor), (e) =>
      e
        .child(div().absolute().inset_0().bg(cursor))
        .child(div().absolute().top_0().bottom_0().left_0().w(2).bg(ctx.focused ? t.accent : t.dim)))
    .on_mouse_down("left", (e, cx) => ctx.onDown(index, e, cx))
    .on_mouse_move((_e, cx) => ctx.onMove(index, cx))
    .children(
      row.s.map(([text, fg, bg]) => {
        // A span tinted like its row is painted once: twice would stack the
        // translucent tint (darker boxes behind the text).
        const own = Boolean(bg) && !selected && bg !== row.bg;
        const behind = [selected ? t.selection : row.bg && color(row.bg, t), cursor, own && color(bg, t)].filter(Boolean);
        const span = div().flex_none().child(text.replace(/ /g, "\u00a0"));
        return span.when(Boolean(fg), (e) => e.text_color(readable(color(fg, t), behind, t))).when(own, (e) => e.bg(color(bg, t)));
      }),
    );
}

// The cursor row's wash: the theme's text colour at 10% (6% when the pane
// isn't focused), lighter on a dark theme and darker on a light one.
function cursorTint(t, focused) {
  return `${String(t.foreground).slice(0, 7)}${focused ? "1a" : "0f"}`;
}

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
            .child(div().flex_1().text_size(11).text_color(t.accent).child(`note · ${noteLines(note, true)}`))
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
        .child(div().text_size(11).text_color(t.accent).child(`new note · ${noteLines(ctx.pending, true)} · enter saves · esc cancels`))
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
