// The file list's rows, in the stock Changes / Files panels' look (28 px
// rows, 7 px corners, `hover` / `selected`), with VS Code's row actions:
// stage +, unstage −, discard ↺ on the hovered or selected row, a folder or
// a whole section. Rows in, elements out; the view passes the handlers.

import { div, svg } from "gpui-kit";
import { h_flex } from "gpui-base";
import { LETTER_COLOR } from "./model.js";

export const ROW = 28;
export const HEADER = 28;
const INDENT = 16;
const ICONS = "mods/gitview/icons";

export const rowSize = (row) => (row.kind === "header" ? HEADER : ROW);

// What + − ↺ do in a section (none in branch scope or a commit).
export function actionsFor(sectionId) {
  if (sectionId === "changes") return ["stage", "discard"];
  if (sectionId === "conflicts") return ["stage"];
  if (sectionId === "staged") return ["unstage"];
  return [];
}

const ACTION = {
  stage: { icon: "plus", tip: "Stage" },
  unstage: { icon: "minus", tip: "Unstage" },
  discard: { icon: "undo", tip: "Discard changes" },
};

// A 22 px icon button. `onPress(cx)`.
export function iconButton(t, id, icon, tip, onPress, { size = 22, color } = {}) {
  return div()
    .id(id)
    .size(size)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(6)
    .cursor_pointer()
    .text_color(color ?? t.muted)
    .hover((s) => s.bg(t.raised).text_color(t.foreground))
    .tooltip(tip)
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .on_click((_e, cx) => {
      cx.stop_propagation();
      onPress(cx);
    })
    .child(svg(`${ICONS}/${icon}.svg`).size(Math.round(size * 0.62)));
}

function actions(t, row, what, ctx) {
  return h_flex()
    .flex_none()
    .gap(1)
    .children(actionsFor(row.section.id).map((a) => iconButton(t, `${a}-${row.key}`, ACTION[a].icon, ACTION[a].tip + what, (cx) => ctx.onAction(a, row, cx))));
}

// ctx: {t, selected, hover, focused, tree, onClick(row, event, cx),
// onHover(row, hovered, cx), onAction(action, row, cx)}
export function renderRow(row, ctx) {
  const { t } = ctx;
  const on = row.key === ctx.selected;
  const hot = row.key === ctx.hover;
  const tools = (hot || on) && actionsFor(row.section.id).length > 0;
  if (row.kind === "header") {
    const n = row.section.files.length;
    return h_flex()
      .id(row.key)
      .h(HEADER)
      .flex_none()
      .pl(10)
      .pr(4)
      .gap(6)
      .items_center()
      .on_hover((h, cx) => ctx.onHover(row, h, cx))
      .child(div().flex_1().min_w(0).truncate().text_size(11).font_semibold().text_color(t.muted).child(row.section.title.toUpperCase()))
      .child(div().flex_none().px(5).rounded(8).bg(t.raised).text_size(10.5).text_color(t.muted).child(String(n)))
      .when(hot && tools, (e) => e.child(actions(t, row, " all", ctx)));
  }
  const base = h_flex()
    .id(row.key)
    .h(ROW)
    .flex_none()
    .mx(6)
    .pl(8 + row.depth * INDENT)
    .pr(tools ? 3 : 8)
    .gap(8)
    .items_center()
    .rounded(7)
    .cursor_pointer()
    .when(on, (e) => e.bg(ctx.focused ? t.selected : t.hover))
    .when(!on, (e) => e.hover((s) => s.bg(t.hover)))
    .on_hover((h, cx) => ctx.onHover(row, h, cx))
    .on_click((e, cx) => ctx.onClick(row, e, cx));
  if (row.kind === "dir") {
    return base
      .child(svg(`${ICONS}/chevron-${row.collapsed ? "right" : "down"}.svg`).size(12).flex_none().text_color(t.dim))
      .child(svg(`${ICONS}/folder.svg`).size(14).flex_none().text_color(t.muted))
      .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(row.name))
      .when(!tools, (e) => e.child(div().flex_none().text_size(11).text_color(t.dim).child(String(row.count))))
      .when(tools, (e) => e.child(actions(t, row, " folder", ctx)));
  }
  const f = row.file;
  const parts = f.path.split("/");
  const name = parts.pop();
  const folder = !ctx.tree && parts.length ? `${parts.join("/")}/` : "";
  return base
    .when(ctx.tree, (e) => e.child(div().w(12).flex_none()))
    .child(div().w(14).flex_none().text_size(11).font_semibold().text_color(t[LETTER_COLOR[f.letter]] ?? t.muted).child(f.letter))
    .child(
      h_flex()
        .flex_1()
        .min_w(0)
        .overflow_hidden()
        // The folder gives way first, then the name: both end in "…".
        .when(Boolean(folder), (e) => e.child(div().flex_shrink(100).min_w(0).truncate().text_color(t.dim).child(folder)))
        .child(div().flex_shrink(1).min_w(24).truncate().text_color(t.foreground).child(name)),
    )
    .when(!tools && f.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${f.add}`)))
    .when(!tools && f.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`−${f.del}`)))
    .when(tools, (e) => e.child(actions(t, row, "", ctx)));
}
