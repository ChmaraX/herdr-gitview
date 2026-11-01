// gitview as a sidebar in the shown tab: the native gitview mod as a pane,
// full height on the right (`herdr-gitview json sidebar` splits the tab's
// root). Pressed again in that tab, it closes. Bound in keys.json as
// "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { closeOverlay, focused, notify, panes, props, run } from "herdr";
import { gv, location } from "./engine.js"; // a link to ../gitview/engine.js (install copies it)

export default class GitviewSidebar extends View {
  init(_props, cx) {
    cx.spawn(async () => {
      const overlay = props()?.overlay;
      try {
        const f = focused();
        if (!f?.tab) {
          notify({ title: "gitview", body: "Open a space first.", level: "warning" });
          return;
        }
        const open = panes().find((p) => p.tab === f.tab && p.mod === "gitview");
        if (open) {
          run("pane.close", { pane: open.pane });
          return;
        }
        const { cwd } = location();
        const { pane } = await gv(["sidebar", "--tab", f.tab, "--mod", "gitview", "--props", JSON.stringify({ cwd })]);
        run("pane.focus", { pane });
      } catch (e) {
        notify({ title: "gitview", body: String(e?.message ?? e), level: "error" });
      } finally {
        if (overlay) closeOverlay(overlay);
      }
    });
  }

  render() {
    return div();
  }
}
