// cmd-shift-g: show or hide the Changes panel showing gitview (the panel is
// the sidebar): what the tab bar's Changes button runs (`layout.select
// {stack, child}` + `layout.show {id}`), or `layout.hide` when it already
// shows gitview; then the list takes the keyboard. The list itself closes
// the diff panes when the panel hides and opens its file when it shows,
// whichever way that happened (placement.js). Bound in keys.json as
// "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { notify, run, spaces } from "herdr";
import { appCli } from "./engine.js";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { panelDiffPanes, panelState } from "./layout.js";

export default class GitviewPanel extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      // A question of gitview's is open (a discard's confirm, the editor's
      // unsaved changes): it answers first, as v1's modal took every key.
      const open = (await appCli(["status"]))?.overlays ?? [];
      if (open.includes("confirm") || open.includes("gitview-ask")) return;
      const p = panelState();
      if (!p) return notify({ title: "gitview", body: "This layout has no gitview Changes panel (run gitview's install.sh).", level: "warning" });
      if (p.shown) {
        // The diff panes go with it at once, and the list hears of the hide
        // (it closes the rest of the unit: nvim, an editor): hidden, it
        // would see it only on its next look, maybe after a show.
        const space = spaces().find((s) => s.active);
        const tabs = new Set(space?.tabs.map((t) => t.id) ?? []);
        for (const pane of panelDiffPanes()) if (tabs.has(pane.tab)) run("pane.close", { pane: pane.pane });
        if (space) run(`gitview.hidden:${space.id}`);
        return run("layout.hide", { id: p.panel });
      }
      if (p.stack) run("layout.select", { stack: p.stack, child: p.slot });
      run("layout.show", { id: p.panel });
      await cx.sleep(150); // the list hears commands once it's drawn
      run("gitview.focus:panel");
    });
  }

  render() {
    return div();
  }
}
