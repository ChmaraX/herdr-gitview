// gitview in its own tab: a tab named "gitview" whose pane is the native
// gitview mod. Opens it, jumps to it from another tab, closes it from inside
// (back to where you were). Bound in keys.json as "mod.open:gitview-tab";
// draws nothing.

import { View, div } from "gpui-kit";
import { closeOverlay, focused, notify, panes, props, run, setShared, shared, spaces } from "herdr";
import { location } from "./engine.js"; // a link to ../gitview/engine.js (install copies it)

export default class GitviewTab extends View {
  init(_props, cx) {
    const overlay = props()?.overlay;
    try {
      const f = focused();
      const space = spaces().find((s) => s.id === f?.space);
      if (!space) {
        notify({ title: "gitview", body: "Open a space first.", level: "warning" });
        return;
      }
      const tabs = new Set(space.tabs.map((t) => t.id));
      // Its tab: named gitview, the gitview mod its only pane (a gitview
      // sidebar in another tab isn't it).
      const own = space.tabs.find((t) => t.name === "gitview" && t.panes === 1 && panes().some((p) => p.tab === t.id && p.mod === "gitview"));
      const view = own ? { tab: own.id } : null;
      const origins = shared("gitview.origins") ?? {};
      if (view && view.tab === f.tab) {
        run("tab.close", { id: view.tab, confirmed: true });
        if (origins[space.id] && tabs.has(origins[space.id])) run("tab.select", { id: origins[space.id] });
      } else {
        setShared("gitview.origins", { ...origins, [space.id]: f.tab });
        if (view) run("tab.select", { id: view.tab });
        else run("tab.new", { name: "gitview", space: space.id, mod: "gitview", props: { cwd: location().cwd } });
      }
    } finally {
      if (overlay) closeOverlay(overlay);
    }
  }

  render() {
    return div();
  }
}
