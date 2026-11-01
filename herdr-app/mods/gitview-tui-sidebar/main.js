// The gitview TUI as a sidebar in the shown tab: changed files + diff on the right,
// full height. Bound in keys.json as "mod.open:gitview-tui-sidebar"; pressed again in
// that tab, it closes. Draws nothing: see launch.js.

import { Launcher } from "./launch.js";

export default class GitviewSidebar extends Launcher {
  action() {
    return "toggle";
  }
}
