// The changed files, grouped like the TUI list: merge conflicts, staged
// changes, changes (or one flat list in branch scope). Rows in, elements out.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";

export const ROW = 24;

// Section headers and files as one list; `key` identifies a file row
// across refreshes (a partly staged file is in two sections).
export function listRows(status) {
  const out = [];
  for (const section of status?.sections ?? []) {
    if (section.title) out.push({ header: section, key: `h:${section.id}` });
    for (const file of section.files) out.push({ file, section, key: `${section.id}:${file.path}` });
  }
  return out;
}

const LETTER_COLOR = { M: "blocked", A: "ansi2", U: "ansi2", D: "ansi1", R: "ansi4", "!": "ansi1" };

export function renderRow(item, ctx) {
  const { t, selected, focused } = ctx;
  if (item.header) {
    return h_flex()
      .h(ROW)
      .px(10)
      .gap(6)
      .items_center()
      .text_size(10.5)
      .font_semibold()
      .text_color(t.muted)
      .child(div().child(item.header.title.toUpperCase()))
      .child(div().text_color(t.dim).child(String(item.header.files.length)));
  }
  const f = item.file;
  const parts = f.path.split("/");
  const name = parts.pop();
  const on = item.key === selected;
  return h_flex()
    .h(ROW)
    .mx(6)
    .px(8)
    .gap(8)
    .items_center()
    .rounded(6)
    .when(on, (e) => e.bg(focused ? t.selected : t.hover))
    .child(div().w(12).flex_none().text_size(11).text_color(t[LETTER_COLOR[f.letter]] ?? t.muted).child(f.letter))
    .child(
      h_flex()
        .flex_1()
        .min_w(0)
        .child(div().min_w(0).truncate().text_color(t.dim).child(parts.length ? `${parts.join("/")}/` : ""))
        .child(div().flex_none().text_color(t.foreground).child(name)),
    )
    .when(f.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${f.add}`)))
    .when(f.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`−${f.del}`)));
}
