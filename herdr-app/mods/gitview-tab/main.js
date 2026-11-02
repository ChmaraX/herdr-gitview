// cmd-g: gitview in a tab of its own, "gitview": the diff pane on the left
// and the file list on the right (list_width_percent, 29 %), for the repo
// the key was pressed in. From another tab it jumps to that tab; from inside
// it closes it and goes back to where you were. Bound in keys.json as
// "mod.open:gitview-tab"; draws nothing.

import { View, div } from "gpui-kit";
import { closeOverlay, focused, notify, props, run, setShared, shared, spaces } from "herdr";
import { gv, location, message } from "./engine.js"; // links to ../gitview (install copies them)
import { ownTab, setOwnTab } from "./shared.js";

const LIST_PERCENT = 29;

export default class GitviewTab extends View {
  init(_props, cx) {
    const overlay = props()?.overlay;
    cx.spawn(async (cx) => {
      try {
        const f = focused();
        const space = spaces().find((s) => s.id === f?.space);
        if (!space) {
          notify({ title: "gitview", body: "Open a space first.", level: "warning" });
          return;
        }
        const own = ownTab(space.id);
        const origins = shared("gitview.origins") ?? {};
        if (own && own === f.tab) {
          run("tab.close", { id: own, confirmed: true });
          const back = origins[space.id];
          if (back && space.tabs.some((t) => t.id === back)) run("tab.select", { id: back });
          return;
        }
        setShared("gitview.origins", { ...origins, [space.id]: f.tab });
        if (own) {
          run("tab.select", { id: own });
          return;
        }
        const { cwd, on } = location();
        const before = new Set(space.tabs.map((t) => t.id));
        run("tab.new", { name: "gitview", space: space.id, mod: "gitview-diff", props: { space: space.id, own: true } });
        // The new tab shows up in spaces() a moment later.
        let tab = null;
        for (let i = 0; i < 50 && !tab; i++) {
          tab = spaces().find((s) => s.id === space.id)?.tabs.find((t) => !before.has(t.id) && t.name === "gitview")?.id ?? null;
          if (!tab) await cx.sleep(50);
        }
        if (!tab) throw new Error("the gitview tab didn't open");
        setOwnTab(space.id, tab);
        const { pane } = await gv(["sidebar", "--tab", tab, "--mod", "gitview", "--percent", String(LIST_PERCENT),
          "--props", JSON.stringify({ cwd, on, space: space.id, tab, own: true })]);
        run("pane.focus", { pane });
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      } finally {
        if (overlay) closeOverlay(overlay);
      }
    });
  }

  render() {
    return div();
  }
}
