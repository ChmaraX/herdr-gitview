// The diff pane's header (the file, its change, the scope, ×) and its key
// hints. Pure render functions.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { iconButton } from "./listview.js";
import { LETTER_COLOR } from "./model.js";

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// view: model.js viewOf(); onClose: null hides ×.
export function header(t, view, { loading, onClose }) {
  const parts = (view?.path ?? "").split("/");
  const name = parts.pop();
  return h_flex()
    .h(42)
    .flex_none()
    .pl(14)
    .pr(8)
    .gap(8)
    .items_center()
    .border_b(1)
    .border_color(t.border)
    .when(!view, (e) => e.child(div().flex_1().font_semibold().text_color(t.muted).child("Diff")))
    .when(Boolean(view), (e) =>
      e
        .child(div().w(14).flex_none().text_size(12).font_semibold().text_color(t[LETTER_COLOR[view.letter]] ?? t.muted).child(view.letter ?? ""))
        .child(
          h_flex()
            .flex_1()
            .min_w(0)
            .when(parts.length > 0, (e) => e.child(div().flex_shrink(100).min_w(0).truncate().text_color(t.dim).child(`${parts.join("/")}/`)))
            .child(div().flex_shrink(1).min_w(24).truncate().font_semibold().child(name)),
        )
        .when(view.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${view.add}`)))
        .when(view.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`−${view.del}`)))
        .child(div().flex_none().px(6).py(1).rounded(5).border_1().border_color(t.border).text_size(10.5).text_color(t.muted).child(view.scope)),
    )
    .when(loading, (e) => e.child(div().flex_none().w(10).text_color(t.dim).frames(SPINNER, 90).child(SPINNER[0])))
    .when(Boolean(onClose), (e) => e.child(iconButton(t, "gitview-diff-close", "x", "Close the diff", onClose)));
}

export function footer(t, flash, writing) {
  const hint = writing
    ? "enter saves the note · esc cancels"
    : "j/k · v or drag select · a note · enter nvim · s/u/x · J/K file · p send · esc";
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
