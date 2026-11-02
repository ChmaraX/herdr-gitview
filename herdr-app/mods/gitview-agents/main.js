// gitview's "send notes to an agent" dialog. Props: { message, count, space,
// key, ids }. Lists the agents, this space's first; enter places the notes in
// the chosen agent's input (you press enter there; no clipboard), shift+enter
// sends them right away. The notes that went out leave the shared notes
// (`key`: the repo's notes, `ids`: the ones sent).

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { agents, closeOverlay, nextCommand, notify, props, theme } from "herdr";
import { gv } from "./engine.js"; // links to ../gitview (install copies them)
import { removeNotes } from "./shared.js";

export default class SendNotes extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.cursor = 0;
    // Its keys as commands too (`gitview-agents.key:enter`, `key:shift-enter`).
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        if (command.startsWith("key:")) this.key(command.slice(4), cx);
        cx.notify();
      }
    });
  }

  key(stroke, cx) {
    const shown = this.list();
    const send = /-enter$/.test(stroke); // shift-, ctrl-, cmd-enter
    if (stroke === "escape") closeOverlay(props().overlay);
    else if (stroke === "down" || stroke === "j") this.cursor = Math.min(shown.length - 1, this.cursor + 1);
    else if (stroke === "up" || stroke === "k") this.cursor = Math.max(0, this.cursor - 1);
    else if (stroke === "enter" || send) this.choose(shown[this.cursor], send, cx);
    else return false;
    return true;
  }

  list() {
    const space = props()?.space;
    const all = agents();
    return [...all.filter((a) => a.space === space), ...all.filter((a) => a.space !== space)];
  }

  choose(agent, submit, cx) {
    if (!agent || this.busy) return;
    this.busy = true;
    const { message, overlay, ids, key } = props();
    cx.spawn(async (cx) => {
      try {
        await gv(["send", "--terminal", agent.address, ...(submit ? ["--submit"] : []), message]);
        if (key && ids?.length) removeNotes(key, ids);
        notify({ title: "gitview", body: `notes ${submit ? "sent" : "placed"}: ${agent.program?.name ?? agent.app ?? agent.address}`, level: "success" });
        closeOverlay(overlay);
      } catch (e) {
        this.busy = false;
        this.error = String(e.message ?? e);
        cx.notify();
      }
    });
  }

  render() {
    const t = theme();
    const p = props() ?? {};
    const shown = this.list();
    this.cursor = Math.min(this.cursor, Math.max(0, shown.length - 1));
    return v_flex()
      .id("gitview-agents")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (!this.key(e.keystroke, cx)) return;
        cx.stop_propagation();
        cx.notify();
      })
      .size_full()
      .p(16)
      .gap(10)
      .bg(t.popover)
      .text_size(13)
      .child(h_flex().items_center().child(div().flex_1().text_size(15).font_semibold().child(`Send ${p.count ?? 0} note${p.count === 1 ? "" : "s"} to…`)).child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .gap(2)
          .overflow_y_scrollbar()
          .when(!shown.length, (e) => e.child(div().text_color(t.dim).child("No agents running. Start one, then send again.")))
          .children(
            shown.map((a, i) =>
              h_flex()
                .id(`agent-${a.address}`)
                .h(30)
                .px(10)
                .gap(8)
                .items_center()
                .rounded(7)
                .cursor_pointer()
                .when(i === this.cursor, (e) => e.bg(t.selected))
                .hover((s) => s.bg(t.hover))
                .on_click((ev, cx) => this.choose(a, ev.modifiers.shift || ev.modifiers.platform, cx))
                .child(div().size(7).rounded(4).bg(t[a.status] ?? t.unknown))
                .child(div().font_semibold().child(a.program?.name ?? a.app ?? "agent"))
                .child(div().text_color(t.muted).child(a.status))
                .child(div().flex_1().min_w(0).truncate().text_color(t.dim).child(`${a.space_name} / ${a.tab_name}`))
                .child(div().text_color(t.dim).child(a.address)),
            ),
          ),
      )
      .when(Boolean(this.error), (e) => e.child(div().text_size(11).text_color(t.ansi1).child(this.error)))
      .child(div().text_size(11).text_color(t.dim).child("enter places them in its input · shift+enter sends now"));
  }
}
