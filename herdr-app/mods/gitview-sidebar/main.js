// cmd-shift-g: show or hide the Changes panel showing gitview (the panel is
// the sidebar): what the tab bar's Changes button runs (`layout.select
// {stack, child}` + `layout.show {id}`), or `layout.hide` when it already
// shows gitview; then the list takes the keyboard. The list itself closes
// the diff panes when the panel hides and opens its file when it shows,
// whichever way that happened (placement.js). Bound in keys.json as
// "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { exec, notify, run, spaces } from "herdr";
import { hostEnv, loadConfig } from "./engine.js";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { panelDiffPanes, panelState } from "./shared.js";

// The overlays open now (`herdr-app status`), [] when it can't be asked.
async function overlays() {
  try {
    const cfg = await loadConfig();
    if (!cfg.herdr_app) return [];
    const r = await exec("/usr/bin/env", [...hostEnv(cfg), cfg.herdr_app, "status"]);
    return JSON.parse(r.stdout).overlays ?? [];
  } catch {
    return [];
  }
}

export default class GitviewPanel extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      // A question of gitview's is open (a discard's confirm, the editor's
      // unsaved changes): it answers first, as v1's modal took every key.
      const open = await overlays();
      if (open.includes("confirm") || open.includes("gitview-ask")) return;
      const p = panelState();
      if (!p) return notify({ title: "gitview", body: "This layout has no gitview Changes panel (run gitview's install.sh).", level: "warning" });
      if (p.shown) {
        // The diff panes go with it at once (the list, hidden from now on,
        // would see the hide only on its next look; it still closes the rest
        // of the unit then: nvim, an editor).
        const space = spaces().find((s) => s.active);
        const tabs = new Set(space?.tabs.map((t) => t.id) ?? []);
        for (const pane of panelDiffPanes()) if (tabs.has(pane.tab)) run("pane.close", { pane: pane.pane });
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
