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

export const spinner = (t) => div().flex_none().w(10).text_color(t.dim).frames(SPINNER, 90).child(SPINNER[0]);

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
    .when(h.loading, (e) => e.child(spinner(t)))
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
    .when(h.loading, (e) => e.child(spinner(t)))
    .when(Boolean(h.onSend), (e) => e.child(iconButton(t, "gitview-send", "send", "Send to an agent (p)", h.onSend)));
}

// The diff pane's: the file, its change, the scope tag, ×. `view` null:
// "Diff". `onClose` null hides ×.
export function diffHeader(t, view, { loading, onClose }) {
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
    .when(loading, (e) => e.child(spinner(t)))
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

// The key hints, or a message (`flash`) for a moment.
export function footer(t, flash, hint) {
  return div()
    .h(26)
    .flex_none()
    .px(14)
    .flex()
    .items_center()
    .border_t(1)
    .border_color(t.border)
    .text_size(11)
    .text_color(flash ? t.blocked : t.dim)
    .child(div().min_w(0).truncate().child(flash ?? hint));
}
