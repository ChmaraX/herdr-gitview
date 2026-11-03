// cmd-shift-g: show or hide the Changes panel showing gitview (the panel is
// the sidebar): what the tab bar's Changes button runs (`layout.select
// {stack, child}` + `layout.show {id}`), or `layout.hide` when it already
// shows gitview; then the list takes the keyboard. The list itself closes
// the diff panes when the panel hides and opens its file when it shows,
// whichever way that happened (placement.js). Bound in keys.json as
// "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { notify, run } from "herdr";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { panelState } from "./shared.js";

export default class GitviewPanel extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      const p = panelState();
      if (!p) return notify({ title: "gitview", body: "This layout has no gitview Changes panel (run gitview's install.sh).", level: "warning" });
      if (p.shown) return run("layout.hide", { id: p.panel });
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
