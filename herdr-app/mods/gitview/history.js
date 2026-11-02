// The history view's rows: one commit each, subject over "sha · when ·
// author", refs (branches, tags) as small pills. Rows in, elements out.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";

export const COMMIT_ROW = 44;

// ctx: {t, selected (index), focused, onClick(i, event, cx)}
export function commitRow(c, i, ctx) {
  const { t } = ctx;
  const on = i === ctx.selected;
  const refs = (c.refs ?? []).map((r) => r.replace(/^HEAD -> /, "")).filter((r) => r !== "HEAD");
  return v_flex()
    .id(`commit-${c.sha}`)
    .h(COMMIT_ROW)
    .flex_none()
    .mx(6)
    .px(10)
    .justify_center()
    .gap(2)
    .rounded(7)
    .cursor_pointer()
    .when(on, (e) => e.bg(ctx.focused ? t.selected : t.hover))
    .when(!on, (e) => e.hover((s) => s.bg(t.hover)))
    .on_click((e, cx) => ctx.onClick(i, e, cx))
    .child(
      h_flex()
        .gap(6)
        .items_center()
        .min_w(0)
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(c.subject || "(no message)"))
        .children(refs.slice(0, 2).map((r) => div().flex_none().max_w(110).truncate().px(5).rounded(5).bg(t.raised).text_size(10.5).text_color(t.accent).child(r))),
    )
    .child(
      h_flex()
        .gap(6)
        .min_w(0)
        .text_size(11)
        .text_color(t.dim)
        .child(div().flex_none().font_family("JetBrains Mono").child(c.short))
        .child(div().flex_none().child(c.when ?? ""))
        .child(div().min_w(0).truncate().child(c.author ?? "")),
    );
}
