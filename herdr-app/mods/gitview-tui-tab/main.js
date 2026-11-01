// The gitview TUI in its own tab (one per repo): opens it, jumps to it from another
// tab, closes it from inside. Bound in keys.json as "mod.open:gitview-tui-tab".

import { Launcher } from "./launch.js"; // a link to ../gitview-tui-sidebar/launch.js (install copies it)

export default class GitviewTab extends Launcher {
  action() {
    return "toggle-tab";
  }
}
