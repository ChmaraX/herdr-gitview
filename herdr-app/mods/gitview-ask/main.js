// gitview's question dialog: a title, a line, and buttons that each do a
// few steps themselves (props {title, detail, choices: [{label, key, steps}]};
// a step is {exec: [program, args]}, {run: [command, args]} or {notify:
// {...}}). It acts on its own because the copy that asked may be hidden by
// then (a hidden copy gets no commands). Esc, or closing it any other way,
// is the last choice (Cancel). Keys: each choice's `key`, enter the first;
// `gitview-ask.choose:<label>` for scripts.

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { closeOverlay, exec, nextCommand, notify, props, run, theme } from "herdr";

export default class GitviewAsk extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.done = false;
    cx.spawn(async (cx) => {
      for (;;) {
        const c = await nextCommand();
        if (c.startsWith("choose:")) this.choose(props().choices.find((x) => x.label === c.slice(7)));
        else if (c.startsWith("key:")) this.key(c.slice(4));
        cx.notify();
      }
    });
  }

  key(k) {
    const choices = props()?.choices ?? [];
    const c = k === "enter" ? choices[0] : choices.find((x) => x.key === k);
    if (!c) return false;
    this.choose(c);
    return true;
  }

  async doSteps(steps) {
    for (const s of steps ?? []) {
      try {
        if (s.exec) await exec(s.exec[0], s.exec[1] ?? []);
        else if (s.run) run(s.run[0], s.run[1] ?? {});
        else if (s.notify) notify(s.notify);
      } catch (e) {
        notify({ title: "gitview", body: String(e?.message ?? e), level: "error" });
      }
    }
  }

  choose(c) {
    if (!c || this.done) return;
    this.done = true;
    this.doSteps(c.steps);
    closeOverlay(props().overlay);
  }

  // Closed without a choice (Esc, a click beside it is ignored: modal):
  // the last one, Cancel.
  async close() {
    if (this.done) return;
    this.done = true;
    const choices = props()?.choices ?? [];
    await this.doSteps(choices[choices.length - 1]?.steps);
  }

  render() {
    const t = theme();
    const p = props() ?? {};
    return v_flex()
      .id("gitview-ask")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (!this.key(e.key)) return;
        cx.stop_propagation();
      })
      .size_full()
      .p(18)
      .gap(10)
      .bg(t.popover)
      .text_color(t.foreground)
      .text_size(13)
      .child(div().text_size(15).font_semibold().child(p.title ?? ""))
      .child(div().flex_1().text_color(t.muted).child(p.detail ?? ""))
      .child(
        h_flex()
          .gap(8)
          .justify_end()
          .children(
            (p.choices ?? []).map((c, i) =>
              div()
                .id(`gitview-ask-${i}`)
                .h(30)
                .px(14)
                .flex()
                .items_center()
                .rounded(8)
                .cursor_pointer()
                .when(i === 0, (e) => e.bg(t.solid).text_color(t.solid_foreground))
                .when(i !== 0, (e) => e.border_1().border_color(t.border).hover((s) => s.bg(t.hover)))
                .on_click(() => this.choose(c))
                .child(c.label),
            ),
          ),
      );
  }
}
