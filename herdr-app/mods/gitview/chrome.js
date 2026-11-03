// The fixed parts of both gitview mods, in one look: icon buttons, pills,
// the spinner, a path (folder dim, name bright), the headers (the list's
// views and the diff pane's), the commit row and the footer. Pure render
// functions; the views pass the handlers. Shared (linked; install copies it).

import { div, svg } from "gpui-kit";
import { h_flex, Input } from "gpui-base";
import { LETTER_COLOR } from "./model.js";

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ICONS = "mods/gitview/icons";
export const HEADER_H = 42;

// A square icon button (default 22 px) with a tooltip. `onPress(cx)`.
export function iconButton(t, id, icon, tip, onPress, { size = 22 } = {}) {
  return div()
    .id(id)
    .size(size)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(6)
    .cursor_pointer()
    .text_color(t.muted)
    .hover((s) => s.bg(t.raised).text_color(t.foreground))
    .tooltip(tip)
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .on_click((_e, cx) => {
      cx.stop_propagation();
      onPress(cx);
    })
    .child(svg(`${ICONS}/${icon}.svg`).size(Math.round(size * 0.62)));
}

export const icon = (name, size, color) => svg(`${ICONS}/${name}.svg`).size(size).flex_none().text_color(color);

export function pill(t, text, { accent = false, id, onPress } = {}) {
  const p = div()
    .flex_none()
    .px(6)
    .py(1)
    .rounded(5)
    .text_size(10.5)
    .when(accent, (e) => e.bg(t.accent).text_color(t.accent_foreground))
    .when(!accent, (e) => e.border_1().border_color(t.border).text_color(t.muted))
    .child(text);
  return onPress ? p.id(id).cursor_pointer().on_click((_e, cx) => onPress(cx)) : p;
}

// The spinner's place is always there (10 px): showing it moves nothing.
export const spinner = (t, on = true) =>
  on ? div().flex_none().w(10).text_color(t.dim).frames(SPINNER, 90).child(SPINNER[0]) : div().flex_none().w(10);

// "folder/" in dim, then the name: the folder gives way first, then the
// name, both ending in "…". `folder: false` shows the name only; `grow:
// false` takes only the width it needs (something follows it).
export function pathLabel(t, path, { folder = true, bold = false, size, grow = true } = {}) {
  const parts = path.split("/");
  const name = parts.pop();
  return h_flex()
    .when(grow, (e) => e.flex_1())
    .when(!grow, (e) => e.flex_shrink(1))
    .min_w(0)
    .overflow_hidden()
    .when(Boolean(size), (e) => e.text_size(size))
    .when(folder && parts.length > 0, (e) => e.child(div().flex_shrink(100).min_w(0).truncate().text_color(t.dim).child(`${parts.join("/")}/`)))
    .child(div().flex_shrink(1).min_w(24).truncate().text_color(t.foreground).when(bold, (e) => e.font_semibold()).child(name));
}

// Where a note is: "4-5", "4", "whole file"; `words`: "lines 4-5", "line 4".
export function noteLines(n, words = false) {
  if (n.end === 0) return "whole file";
  const one = n.start === n.end;
  const nums = one ? `${n.start}` : `${n.start}-${n.end}`;
  return words ? `${one ? "line" : "lines"} ${nums}` : nums;
}

export const letter = (t, l, size = 11) =>
  div().w(14).flex_none().text_size(size).font_semibold().text_color(t[LETTER_COLOR[l]] ?? t.muted).child(l ?? "");

function bar(t) {
  return h_flex().h(HEADER_H).flex_none().pl(14).pr(8).gap(8).items_center().border_b(1).border_color(t.border);
}

// The changed files: {status, files, notes, loading, wide, tree, onNotes, onTree, onLog}.
export function filesHeader(t, h) {
  const s = h.status;
  const what = !s ? "gitview" : s.scope === "branch" ? `vs ${s.base?.label ?? "base"}` : "uncommitted";
  return bar(t)
    .child(div().flex_none().font_semibold().child(what))
    .child(
      h_flex()
        .flex_1()
        .min_w(0)
        .gap(4)
        .items_center()
        .when(Boolean(s?.branch), (e) => e.child(icon("branch", 12, t.dim)).child(div().min_w(0).truncate().text_size(12).text_color(t.muted).child(s.branch))),
    )
    .when(h.notes > 0, (e) => e.child(pill(t, `${h.notes} note${h.notes > 1 ? "s" : ""}`, { accent: true, id: "gitview-notes-pill", onPress: h.onNotes })))
    .child(pill(t, `${h.files} file${h.files === 1 ? "" : "s"}`))
    .child(spinner(t, h.loading))
    .when(h.wide, (e) =>
      e
        .child(iconButton(t, "gitview-tree", h.tree ? "list" : "list-tree", h.tree ? "Show as list (t)" : "Show as tree (t)", h.onTree))
        .child(iconButton(t, "gitview-log", "history", "History (l)", h.onLog)),
    );
}

// History, a commit, notes: ← title, detail, a pill, send.
export function subHeader(t, h) {
  return bar(t)
    .pl(6)
    .child(iconButton(t, "gitview-back", "arrow-left", "Back (esc)", h.onBack, { size: 24 }))
    .child(div().flex_none().font_semibold().child(h.title))
    .child(div().flex_1().min_w(0).truncate().text_size(12).text_color(t.muted).child(h.detail ?? ""))
    .when(Boolean(h.pill), (e) => e.child(pill(t, h.pill, { id: "gitview-sub-pill", onPress: h.onPill })))
    .child(spinner(t, h.loading))
    .when(Boolean(h.onSend), (e) => e.child(iconButton(t, "gitview-send", "send", "Send to an agent (p)", h.onSend)));
}

// Diff | Preview, the shown one raised; `onPick(preview, cx)`; `compact`:
// icons (diff, eye) instead of the words.
function segmented(t, preview, onPick, compact) {
  const seg = (label, on, value) =>
    div()
      .id(`gitview-show-${label.toLowerCase()}`)
      .px(compact ? 4 : 7)
      .h(20)
      .flex()
      .items_center()
      .rounded(5)
      .text_size(11)
      .cursor_pointer()
      .when(on, (e) => e.bg(t.raised).text_color(t.foreground))
      .when(!on, (e) => e.text_color(t.muted).hover((s) => s.text_color(t.foreground)))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => onPick(value, cx));
  return h_flex()
    .flex_none()
    .p(1)
    .gap(1)
    .rounded(6)
    .border_1()
    .border_color(t.border)
    .tooltip("Diff or Preview (m)")
    .child(seg("Diff", !preview, false).child(compact ? icon("diff", 13, !preview ? t.foreground : t.muted) : "Diff"))
    .child(seg("Preview", preview, true).child(compact ? icon("eye", 13, preview ? t.foreground : t.muted) : "Preview"));
}

// The diff pane's: the file, its change, the scope tag, Diff | Preview
// for a file that previews (`rich`: {preview, onPick, compact}), ×. `view` null:
// "Diff". `onClose` null hides ×.
export function diffHeader(t, view, { loading, onClose, rich }) {
  return bar(t)
    .when(!view, (e) => e.child(div().flex_1().font_semibold().text_color(t.muted).child("Diff")))
    .when(Boolean(view), (e) =>
      e
        .child(letter(t, view.letter, 12))
        .child(pathLabel(t, view.path, { bold: true }))
        .when(view.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${view.add}`)))
        .when(view.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`−${view.del}`)))
        .child(pill(t, view.scope)),
    )
    .when(Boolean(view && rich), (e) => e.child(segmented(t, rich.preview, rich.onPick, rich.compact)))
    .child(spinner(t, loading))
    .when(Boolean(onClose), (e) => e.child(iconButton(t, "gitview-diff-close", "x", "Close the diff", onClose)));
}

// The stock Changes panel's commit row: enter commits the staged changes,
// or everything when nothing is staged.
export function commitBox(t, state, typing, staged, busy, onCommit) {
  const ready = Boolean(state.value().trim()) && !busy;
  return h_flex()
    .flex_none()
    .p(10)
    .gap(8)
    .border_t(1)
    .border_color(t.border)
    .child(
      div()
        .id("gitview-commit-message")
        .flex_1()
        .min_w(0)
        .h(32)
        .px(10)
        .flex()
        .items_center()
        .rounded(8)
        .border_1()
        .border_color(typing ? t.accent : t.border)
        .cursor_text()
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .child(Input.new(state).h_full().text_size(12).text_color(t.foreground)),
    )
    .child(
      div()
        .id("gitview-commit")
        .h(32)
        .px(12)
        .flex_none()
        .when(ready, (e) => e.cursor_pointer())
        .on_click((_e, cx) => onCommit(cx))
        .flex()
        .items_center()
        .rounded(8)
        .bg(ready ? t.solid : t.raised)
        .text_color(ready ? t.solid_foreground : t.dim)
        .text_size(12)
        .child(busy ? "Committing…" : staged ? "Commit staged" : "Commit all"),
    );
}

// ---- keys ---------------------------------------------------------------------

const SYMBOL = {
  enter: "⏎", escape: "esc", tab: "⇥", up: "↑", down: "↓", left: "←", right: "→",
  space: "␣", pageup: "PgUp", pagedown: "PgDn", home: "Home", end: "End",
};
export const capText = (k) => SYMBOL[k] ?? k;
const MONO = "JetBrains Mono";
// Letters in the mono font; the symbols (⏎ ↑ ←) read better in the UI's.
const symbolic = (k) => k.length > 1 && capText(k).length === 1;

// One key as a keycap: a small rounded box, the key in the mono font.
export function keycap(t, k, { size = 10.5 } = {}) {
  return div()
    .flex_none()
    .min_w(16)
    .h(16)
    .px(4)
    .flex()
    .items_center()
    .justify_center()
    .rounded(4)
    .border_1()
    .border_color(t.border)
    .bg(t.raised)
    .when(!symbolic(k), (e) => e.font_family(MONO))
    .text_size(symbolic(k) ? size + 1 : size)
    .text_color(t.foreground)
    .child(capText(k));
}

// A key table entry's keycaps then its label.
export function keyChip(t, entry) {
  return h_flex()
    .flex_none()
    .gap(4)
    .items_center()
    .child(h_flex().flex_none().gap(2).children(entry.caps.map((k) => keycap(t, k))))
    .child(div().flex_none().text_size(11).text_color(t.dim).child(entry.label));
}

// About how wide a chip draws (mono ~6.4 px, labels ~6 px a character).
const chipWidth = (e) => e.caps.reduce((w, k) => w + Math.max(16, capText(k).length * 6.4 + 10) + 2, 0) + 4 + e.label.length * 6;

// The footer: a message (`flash`) for a moment, else the most useful keys
// of the view as keycaps, as many as fit `width`; always a `?` keycap at the
// right edge that opens every key (`onKeys(cx)`).
export function footer(t, { flash, chips = [], width = 0, onKeys, id = "gitview-keys-button" }) {
  let room = (width || 300) - 28 - 30;
  const shown = [];
  for (const c of chips) {
    const w = chipWidth(c) + 12;
    if (w > room) break;
    room -= w;
    shown.push(c);
  }
  return h_flex()
    .h(28)
    .flex_none()
    .px(14)
    .gap(12)
    .items_center()
    .border_t(1)
    .border_color(t.border)
    .text_size(11)
    .child(
      flash
        ? div().flex_1().min_w(0).truncate().text_color(t.blocked).child(flash)
        : h_flex().flex_1().min_w(0).gap(12).overflow_hidden().children(shown.map((c) => keyChip(t, c))),
    )
    .child(
      div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .rounded(4)
        .hover((s) => s.bg(t.hover))
        .tooltip("All keys (?)")
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click((_e, cx) => {
          cx.stop_propagation();
          onKeys?.(cx);
        })
        .child(keycap(t, "?")),
    );
}
