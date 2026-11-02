// The list's fixed parts: the header of each view (files, history, a
// commit, notes), the commit row and the key hints. Pure render functions;
// the view passes the handlers.

import { div, svg } from "gpui-kit";
import { h_flex, Input } from "gpui-base";
import { iconButton } from "./listview.js";

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
export const HEADER_H = 42;

function pill(t, text, { accent = false, id, onPress } = {}) {
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

function bar(t) {
  return h_flex().h(HEADER_H).flex_none().pl(14).pr(8).gap(8).items_center().border_b(1).border_color(t.border);
}

const spinner = (t) => div().flex_none().w(10).text_color(t.dim).frames(SPINNER, 90).child(SPINNER[0]);

// files: {status, files, notes, loading, wide, tree, onNotes, onTree, onLog}
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
        .when(Boolean(s?.branch), (e) => e.child(svg("mods/gitview/icons/branch.svg").size(12).flex_none().text_color(t.dim)).child(div().min_w(0).truncate().text_size(12).text_color(t.muted).child(s.branch))),
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

// history / a commit / notes: ← title, detail, pills.
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

const HINTS = {
  files: "j/k · enter nvim · s/u/x stage/unstage/discard · t tree · l log · n notes · p send · c commit · w scope",
  log: "j/k · enter files · w branch/all · esc back",
  commit: "j/k · enter nvim · esc back",
  notes: "j/k · enter go · d delete · p send · esc back",
};

export function footer(t, flash, mode) {
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
    .child(div().min_w(0).truncate().child(flash ?? HINTS[mode]));
}
