// cmd-shift-g: show or hide the Changes panel showing gitview (the panel is
// the sidebar): what the tab bar's Changes button runs (`layout.select
// {stack, child}` + `layout.show {id}`), or `layout.hide` when it already
// shows gitview (the diff pane closes with it); then the list takes the
// keyboard and its selected (else first) file opens in the diff pane. Bound in keys.json as
// "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { notify, run } from "herdr";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { panelDiffPanes, panelState, setPanelReopens } from "./shared.js";

export default class GitviewPanel extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      const p = panelState();
      if (!p) return notify({ title: "gitview", body: "This layout has no gitview Changes panel (run gitview's install.sh).", level: "warning" });
      if (p.shown) {
        // Hide both, as the TUI's toggle did: the panel, and the diff pane
        // it opened (which opens again, on the selected file, next time).
        run("layout.hide", { id: p.panel });
        setPanelReopens(true); // after the hide: the list mustn't see itself shown and reopen
        // Every tab's (a pane in a tab not on screen gets no commands).
        for (const pane of panelDiffPanes()) run("pane.close", { pane: pane.pane });
        return;
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
