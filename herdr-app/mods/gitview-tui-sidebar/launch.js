// Shared by the gitview TUI key launchers: run the gitview orchestrator
// (`herdr-gitview toggle` / `toggle-tab`) for where the key was pressed, then
// close. The Rust side does the window work through `herdr-app do` and `hc`;
// this mod only hands it the context it can't see.

import { View, div } from "gpui-kit";
import { closeOverlay, exec, focused, notify, panes, props, spaces } from "herdr";
import { hostEnv, loadConfig } from "./engine.js"; // a link to ../gitview/engine.js (install copies it)

// focused() plus the folders gitview may find the repo in, best first:
// the focused terminal's, the tab's other terminals', the space's.
function context() {
  const f = focused();
  if (!f?.tab) return null;
  const space = spaces().find((s) => s.id === f.space);
  const folders = panes()
    .filter((p) => p.tab === f.tab && p.cwd)
    .sort((a, b) => Number(b.focused) - Number(a.focused))
    .map((p) => p.cwd);
  if (space?.folder) folders.push(space.folder);
  return { space: f.space, tab: f.tab, pane: f.pane, terminal: f.terminal, cwd: f.cwd, folders };
}

export class Launcher extends View {
  // "toggle" or "toggle-tab".
  action() {
    return "toggle";
  }

  init(_props, cx) {
    cx.spawn(async () => {
      const overlay = props()?.overlay;
      try {
        const ctx = context();
        if (!ctx) {
          notify({ title: "gitview", body: "Open a space first.", level: "warning" });
          return;
        }
        const cfg = await loadConfig();
        const env = [...hostEnv(cfg), `GITVIEW_APP_CONTEXT=${JSON.stringify(ctx)}`];
        const r = await exec("/usr/bin/env", [...env, cfg.bin, this.action()]);
        if (r.code !== 0) {
          notify({ title: "gitview", body: (r.stderr || r.stdout).trim().split("\n").pop(), level: "error" });
        }
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
