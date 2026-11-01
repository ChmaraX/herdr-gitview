// gitview's dialogs (confirm, send to agent). The panes run them as hidden
// core terminals labeled gitview.popup=<entrypoint> and gitview.cells=<cols>x<rows>,
// then run "mod.open:gitview-popup". Opened that way this mod shows the
// newest such terminal in a dialog and closes; the dialog closes by itself
// when the popup program answers and exits. Inside the dialog, the same mod
// draws its title bar (props.bar).

import { View, div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { closeOverlay, openOverlay, props, settings, shared, setShared, terminals, theme } from "herdr";

const TITLES = { ask: "gitview: confirm", "pick-agent": "gitview: send to agent" };
const BAR = 28;

function pending() {
  const shown = shared("gitview.popups") ?? [];
  return terminals()
    .filter((t) => t.state === "live" && t.labels?.["gitview.popup"] && !shown.includes(t.address))
    .sort((a, b) => (b.created_ms ?? 0) - (a.created_ms ?? 0))[0];
}

// The popup's size in cells, as px around the terminal's font.
function size(t) {
  const [cols, rows] = String(t.labels["gitview.cells"] ?? "60x8").split("x").map(Number);
  const s = settings();
  const font = Number(s.font_size ?? 14);
  const pad = s.terminal_padding ?? [10, 6];
  return {
    width: Math.round(cols * font * 0.62 + 2 * (pad[0] ?? 10) + 8),
    height: Math.round(rows * font * 1.3 + 2 * (pad[1] ?? 6) + BAR + 8),
  };
}

export default class GitviewPopup extends View {
  init(_props, cx) {
    if (props()?.bar) return;
    let tries = 0;
    // The app hears of the new terminal a moment after `hc run` returns.
    const show = (cx) => {
      const t = pending();
      if (!t && tries++ < 30) {
        cx.timer.after(100, show);
        return;
      }
      if (t) {
        const live = terminals().filter((x) => x.state === "live").map((x) => x.address);
        setShared("gitview.popups", [...(shared("gitview.popups") ?? []).filter((a) => live.includes(a)), t.address]);
        openOverlay({
          kind: "dialog",
          modal: true,
          ...size(t),
          layout: {
            type: "column",
            children: [
              { type: "block", size: { px: BAR }, content: { mod: "gitview-popup", props: { bar: true, terminal: t.address, entry: t.labels["gitview.popup"] } } },
              { type: "block", content: { terminal: t.address } },
            ],
          },
        });
      }
      closeOverlay(props().overlay);
    };
    show(cx);
  }

  render() {
    const p = props() ?? {};
    if (!p.bar) return div();
    const t = theme();
    return h_flex()
      .size_full()
      .px(12)
      .items_center()
      .bg(t.surface)
      .border_b(1)
      .border_color(t.border)
      .text_size(12)
      .child(div().font_semibold().text_color(t.foreground).child(TITLES[p.entry] ?? "gitview"))
      .child(div().flex_1())
      .child(div().text_color(t.dim).child("esc cancels"));
  }
}
