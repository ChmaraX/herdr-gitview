// cmd-shift-g: show or hide the Changes panel showing gitview (the panel is
// the sidebar now): exactly what the tab bar's Changes button runs
// (`layout.select {stack, child}` + `layout.show {id}`), or `layout.hide`
// when it already shows gitview. Then the list takes the keyboard. Bound in
// keys.json as "mod.open:gitview-sidebar"; draws nothing.

import { View, div } from "gpui-kit";
import { closeOverlay, notify, props, run } from "herdr";
import { panelState } from "./shared.js"; // a link to ../gitview/shared.js (install copies it)

export default class GitviewPanel extends View {
  init(_props, cx) {
    const overlay = props()?.overlay;
    cx.spawn(async (cx) => {
      try {
        const p = panelState();
        if (!p) {
          notify({ title: "gitview", body: "This layout has no gitview Changes panel (run gitview's install.sh).", level: "warning" });
          return;
        }
        if (p.shown) {
          run("layout.hide", { id: p.panel });
          return;
        }
        if (p.stack) run("layout.select", { stack: p.stack, child: p.slot });
        run("layout.show", { id: p.panel });
        // The list hears commands once it's drawn.
        await cx.sleep(150);
        run("gitview.focus:panel");
      } finally {
        if (overlay) closeOverlay(overlay);
      }
    });
  }

  render() {
    return div();
  }
}
