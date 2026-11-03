// cmd-g: gitview in a tab of its own, "gitview": the diff pane on the left
// and the file list on the right (29 %), for the repo the key was pressed
// in. From another tab it jumps to that tab; from inside it closes it and
// goes back to where you were. Bound in keys.json as "mod.open:gitview-tab";
// draws nothing.

import { View, div } from "gpui-kit";
import { focused, notify, run, spaces } from "herdr";
import { gv, location } from "./engine.js"; // links to ../gitview (install copies them)
import { oneShot } from "./modkit.js";
import { origins, ownTab, setOrigin, setOwnTab } from "./shared.js";

const LIST_PERCENT = 29;

export default class GitviewTab extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      const f = focused();
      const space = spaces().find((s) => s.id === f?.space);
      if (!space) return notify({ title: "gitview", body: "Open a space first.", level: "warning" });
      const own = ownTab(space.id);
      if (own && own === f.tab) {
        run("tab.close", { id: own, confirmed: true });
        const back = origins()[space.id];
        if (back && space.tabs.some((t) => t.id === back)) run("tab.select", { id: back });
        return;
      }
      setOrigin(space.id, f.tab);
      if (own) return run("tab.select", { id: own });
      const { cwd, on } = location();
      const before = new Set(space.tabs.map((t) => t.id));
      run("tab.new", { name: "gitview", space: space.id, mod: "gitview-diff", props: { space: space.id, own: true } });
      // The new tab shows up in spaces() a moment later (seconds, in a
      // busy app).
      let tab = null;
      for (let i = 0; i < 200 && !tab; i++) {
        tab = spaces().find((s) => s.id === space.id)?.tabs.find((t) => !before.has(t.id) && t.name === "gitview")?.id ?? null;
        if (!tab) await cx.sleep(50);
      }
      if (!tab) throw new Error("the gitview tab didn't open");
      setOwnTab(space.id, tab);
      const props = JSON.stringify({ cwd, on, space: space.id, tab, own: true });
      const { pane } = await gv(["sidebar", "--tab", tab, "--mod", "gitview", "--percent", String(LIST_PERCENT), "--props", props]);
      run("pane.focus", { pane });
    });
  }

  render() {
    return div();
  }
}
