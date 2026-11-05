// gitview's keys card: every key of the mod that opened it (the list or the
// diff pane), grouped by view, the shown one first. A dialog opened by
// keymap.js toggleKeys; props {title, groups: [{title, keys: [{caps,
// label}]}]}, made from the same tables the keys run from. Esc, `?` or a
// click beside it closes it. While it's open, shared "gitview.keys" says
// so (`?` in the mod closes it again).

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { closeOverlay, nextCommand, props, setShared, shared, theme } from "herdr";
import { keycap } from "./chrome.js"; // links to ../gitview (install copies them)

export default class GitviewKeys extends View {
  init(_props, cx) {
    this.given = props() ?? {}; // read here only: later it can be another turn's (placement.js)
    this.handle = cx.focus_handle();
    this.handle.focus();
    setShared("gitview.keys", { overlay: this.given?.overlay ?? null });
    // `gitview-keys.key:<k>` for scripts.
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c.startsWith("key:")) this.key(c.slice(4));
        cx.notify();
      }
    });
  }

  key(k) {
    if (k !== "escape" && k !== "?") return false;
    closeOverlay(this.given.overlay);
    return true;
  }

  async close() {
    if (shared("gitview.keys")?.overlay === this.given?.overlay) setShared("gitview.keys", null);
  }

  render() {
    const t = theme();
    const p = this.given ?? {};
    return v_flex()
      .id("gitview-keys")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        const k = e.modifiers?.shift && e.key === "/" ? "?" : e.key;
        if (!this.key(k)) return;
        cx.stop_propagation();
      })
      .size_full()
      .p(16)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .text_color(t.foreground)
      .child(
        h_flex()
          .items_center()
          .gap(8)
          .child(div().flex_1().min_w(0).truncate().text_size(15).font_semibold().child(p.title ?? "Keys"))
          .child(keycap(t, "escape"))
          .child(keycap(t, "?")),
      )
      .child(
        v_flex()
          .id("gitview-keys-groups")
          .flex_1()
          .min_h(0)
          .gap(12)
          .overflow_y_scrollbar()
          .children(
            (p.groups ?? []).map((g, gi) =>
              v_flex()
                .gap(2)
                .child(div().h(20).text_size(11).font_semibold().text_color(gi === 0 ? t.accent : t.muted).child(g.title.toUpperCase()))
                .children(
                  g.keys.map((k) =>
                    h_flex()
                      .h(22)
                      .gap(10)
                      .items_center()
                      .child(h_flex().w(96).flex_none().gap(3).children(k.caps.map((c) => keycap(t, c, { size: 11 }))))
                      .child(div().flex_1().min_w(0).truncate().text_color(t.muted).child(k.label)),
                  ),
                ),
            ),
          ),
      )
      // Where to point at a problem (gitview's trace, telemetry.js).
      .child(div().flex_none().pt(8).text_size(11).text_color(t.dim).child(`Something wrong? The command palette's "gitview: mark (something's wrong)" marks it in the trace (${shared("gitview.trace") === false ? "off" : "on"}; "gitview: tracing on/off").`));
  }
}
