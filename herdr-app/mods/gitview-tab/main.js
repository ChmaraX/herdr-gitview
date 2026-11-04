// cmd-g: gitview in a tab of its own, "gitview": the diff pane on the left
// and the file list on the right (29 %), for the repo the key was pressed
// in. From another tab it jumps to that tab; from inside it closes it and
// goes back to where you were. Bound in keys.json as "mod.open:gitview-tab";
// draws nothing.

import { View, div } from "gpui-kit";
import { focused, notify, run, spaces } from "herdr";
import { appCli, gv } from "./engine.js"; // links to ../gitview (install copies them)
import { location } from "./layout.js";
import { oneShot } from "./modkit.js";
import { origins, ownTab, setOrigin, setOwnTab } from "./shared.js";
import { closeEditorOf, setEditor } from "./editors.js";

const LIST_PERCENT = 29;

export default class GitviewTab extends View {
  init(_props, cx) {
    oneShot(cx, async (cx) => {
      // Where the person is, as the app has it now (`herdr-app status`): a
      // window that isn't drawn can leave this copy's focused() a step
      // behind (a tab just selected).
      const now = await appCli(["status"]);
      const f = { ...focused(), ...(now?.space && now?.tab ? { space: now.space, tab: now.tab } : {}) };
      const space = spaces().find((s) => s.id === f?.space);
      if (!space) return notify({ title: "gitview", body: "Open a space first.", level: "warning" });
      const own = ownTab(space.id);
      if (own && own === f.tab) {
        const back = origins()[space.id];
        const close = [{ run: ["tab.close", { id: own, confirmed: true }] }, ...(back && space.tabs.some((t) => t.id === back) ? [{ run: ["tab.select", { id: back }] }] : [])];
        // Its editor closes with it (v1): clean, it quits; unsaved, the
        // question first (Cancel keeps the tab and the editor).
        if ((await closeEditorOf(own, { after: close })) === "dirty") return;
        setEditor(own, null);
        for (const s of close) run(s.run[0], s.run[1]);
        return;
      }
      setOrigin(space.id, f.tab);
      if (own) return run("tab.select", { id: own });
      const { cwd, on } = location();
      const before = new Set(space.tabs.map((t) => t.id));
      run("tab.new", { name: "gitview", space: space.id, mod: "gitview-diff", props: { space: space.id, own: true } });
      // The new tab shows up in spaces() a moment later; in a window that
      // isn't drawn this copy's spaces() can lag for good, so after 2 s
      // the app is asked (`herdr-app ls`).
      const isNew = (t) => !before.has(t.id) && t.name === "gitview";
      let tab = null;
      for (let i = 0; i < 40 && !tab; i++) {
        tab = spaces().find((s) => s.id === space.id)?.tabs.find(isNew)?.id ?? null;
        if (!tab) await cx.sleep(50);
      }
      for (let i = 0; i < 16 && !tab; i++) {
        tab = ((await appCli(["ls"]))?.spaces ?? []).find((s) => s.id === space.id)?.tabs.find(isNew)?.id ?? null;
        if (!tab) await cx.sleep(500);
      }
      if (!tab) throw new Error("the gitview tab didn't open");
      setOwnTab(space.id, tab);
      const props = JSON.stringify({ cwd, on, space: space.id, tab, own: true });
      // Right after it shows up, the window may not have the tab's panes
      // yet ("no pane …"): again for a few seconds.
      let pane = null;
      for (let i = 0; ; i++) {
        try {
          ({ pane } = await gv(["sidebar", "--tab", tab, "--mod", "gitview", "--percent", String(LIST_PERCENT), "--props", props]));
          break;
        } catch (e) {
          if (i >= 10 || !/no pane/.test(String(e?.message ?? e))) throw e;
          await cx.sleep(500);
        }
      }
      run("pane.focus", { pane });
    });
  }

  render() {
    return div();
  }
}
