// The notes list's rows: where (file:lines) over the note's first line,
// with a delete button on the hovered or selected row.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { iconButton } from "./listview.js";

export const NOTE_ROW = 44;

export const lines = (n) => (n.end === 0 ? "whole file" : n.start === n.end ? `${n.start}` : `${n.start}-${n.end}`);

// ctx: {t, selected (index), hover (id), focused, onClick, onHover, onDelete}
export function noteRow(n, i, ctx) {
  const { t } = ctx;
  const on = i === ctx.selected;
  const tools = on || ctx.hover === n.id;
  const parts = n.file.split("/");
  const name = parts.pop();
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
        .child(
          h_flex()
            .min_w(0)
            .text_size(12)
            .when(parts.length > 0, (e) => e.child(div().flex_shrink(100).min_w(0).truncate().text_color(t.dim).child(`${parts.join("/")}/`)))
            .child(div().flex_shrink(1).min_w(24).truncate().text_color(t.foreground).child(name))
            .child(div().flex_none().text_color(t.accent).child(`:${lines(n)}`)),
        )
        .child(div().min_w(0).truncate().text_size(11.5).text_color(t.muted).child(n.text.split("\n")[0])),
    )
    .when(tools, (e) => e.child(iconButton(t, `note-del-${n.id}`, "x", "Delete note (d)", (cx) => ctx.onDelete(n, cx))));
}
