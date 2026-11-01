// The panel's fixed parts around the list and the diff: header, note
// composer, commit box, key hints. Pure render functions.

import { div } from "gpui-kit";
import { h_flex, v_flex, Input, Textarea } from "gpui-base";

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// "uncommitted · branch" or "vs base · branch", the note count, the file
// count, a spinner while loading.
export function header(t, status, files, notes, loading) {
  const what = !status ? "gitview" : status.scope === "branch" ? `vs ${status.base?.label ?? "base"}` : "uncommitted";
  return h_flex()
    .h(38)
    .flex_none()
    .px(12)
    .gap(8)
    .items_center()
    .border_b(1)
    .border_color(t.border)
    .child(div().font_semibold().child(what))
    .when(Boolean(status?.branch), (e) => e.child(div().min_w(0).truncate().text_color(t.muted).child(status.branch)))
    .child(div().flex_1())
    .when(notes > 0, (e) => e.child(div().px(6).rounded(4).bg(t.accent).text_color(t.accent_foreground).text_size(10.5).child(`${notes} note${notes > 1 ? "s" : ""}`)))
    .child(div().px(5).py(1).rounded(4).border_1().border_color(t.border).text_size(10.5).text_color(t.muted).child(`${files} files`))
    .when(loading, (e) => e.child(div().w(10).text_color(t.dim).frames(SPINNER, 90).child(SPINNER[0])));
}

export function composer(t, note, state) {
  const where = note.start === note.end ? `line ${note.start}` : `lines ${note.start}-${note.end}`;
  return v_flex()
    .flex_none()
    .p(8)
    .gap(4)
    .border_t(1)
    .border_color(t.border)
    .bg(t.surface)
    .child(div().text_size(11).text_color(t.accent).child(`note · ${note.file} · ${where}`))
    .child(div().rounded(6).border_1().border_color(t.accent).px(6).py(4).child(Textarea.new(state).h(54).text_size(12)));
}

// The stock Changes panel's commit row: enter commits the staged changes,
// or everything when nothing is staged.
export function commitBox(t, state, typing, staged, onCommit) {
  const ready = Boolean(state.value().trim());
  return h_flex()
    .flex_none()
    .p(8)
    .gap(8)
    .border_t(1)
    .border_color(t.border)
    .child(
      div()
        .id("gitview-commit-message")
        .flex_1()
        .min_w(0)
        .h(30)
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
        .h(30)
        .px(12)
        .when(ready, (e) => e.cursor_pointer())
        .on_click((_e, cx) => onCommit(cx))
        .flex()
        .items_center()
        .rounded(8)
        .bg(ready ? t.solid : t.raised)
        .text_color(ready ? t.solid_foreground : t.dim)
        .text_size(12)
        .child(staged ? "Commit staged" : "Commit all"),
    );
}

export function footer(t, flash, area) {
  const hint = flash ?? (area === "list"
    ? "j/k · enter edit · s/u stage · x discard · c commit · w scope · tab"
    : "j/k · v select · a note · p send · enter edit · esc");
  return div().h(24).flex_none().px(12).flex().items_center().border_t(1).border_color(t.border).text_size(11).text_color(flash ? t.blocked : t.dim).truncate().child(hint);
}
