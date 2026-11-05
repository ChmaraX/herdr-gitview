// The file list's rows, in the stock Changes / Files panels' look (28 px
// rows, 7 px corners, `hover` / `selected`), with VS Code's row actions:
// stage +, unstage −, discard ↺ on the hovered or selected row, a folder or
// a whole section. Rows in, elements out; the view passes the handlers.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { opsFor } from "./actions.js";
import { icon, iconButton, letter, lineCounts, pathLabel } from "./chrome.js";

export const ROW = 28;
export const HEADER = 28;
const INDENT = 16;

export const rowSize = (row) => (row.kind === "header" ? HEADER : ROW);

const ACTION = {
  stage: { icon: "plus", tip: "Stage" },
  unstage: { icon: "minus", tip: "Unstage" },
  discard: { icon: "undo", tip: "Discard changes" },
};

function actions(t, row, what, ctx) {
  return h_flex()
    .flex_none()
    .gap(1)
    .children(opsFor(row.section.id).map((a) => iconButton(t, `${a}-${row.key}`, ACTION[a].icon, ACTION[a].tip + what, (cx) => ctx.onAction(a, row, cx))));
}

// ctx: {t, selected (the selected row's key), hover, focused, tree, inNvim(row), onClick(row, event, cx),
// onHover(row, hovered, cx), onAction(op, row, cx)}
export function renderRow(row, ctx) {
  const { t } = ctx;
  const on = row.key === ctx.selected;
  const hot = row.key === ctx.hover;
  const tools = (hot || on) && opsFor(row.section.id).length > 0;
  if (row.kind === "header") {
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
      .child(div().flex_none().px(5).rounded(8).bg(t.raised).text_size(10.5).text_color(t.muted).child(String(row.section.files.length)))
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
      .child(icon(`chevron-${row.collapsed ? "right" : "down"}`, 12, t.dim))
      .child(icon("folder", 14, t.muted))
      .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(row.name))
      .when(!tools, (e) => e.child(div().flex_none().text_size(11).text_color(t.dim).child(String(row.count))))
      .when(tools, (e) => e.child(actions(t, row, " folder", ctx)));
  }
  const f = row.file;
  return base
    .when(ctx.tree, (e) => e.child(div().w(12).flex_none()))
    .child(letter(t, f.letter))
    .child(pathLabel(t, f.path, { folder: !ctx.tree }))
    .when(Boolean(ctx.inNvim?.(row)), (e) => e.child(div().flex_none().px(5).rounded(5).border_1().border_color(t.border).text_size(10).text_color(t.muted).child("in nvim")))
    .when(!tools, (e) => e.children(lineCounts(t, f.add, f.del)))
    .when(tools, (e) => e.child(actions(t, row, "", ctx)));
}
