// Where gitview's panes and panel are, from the app's live lists and the
// shown layout (no shared state of gitview's). Shared by the mods (linked;
// install copies it).

import { focused, layout, panes, spaces, terminals } from "herdr";
import { isOwnTab, machineOf } from "./shared.js";

// The tab's diff pane, if it has one.
export function diffPane(tab) {
  return panes().find((p) => p.tab === tab && p.mod === "gitview-diff") ?? null;
}

// The diff panes the Changes panel opened, in every tab (not the ones in
// cmd-g's gitview tabs, which belong to their own list).
export function panelDiffPanes() {
  return panes().filter((p) => p.mod === "gitview-diff" && !isOwnTab(p.tab));
}

// The panes herdr-nvim shows in the tab: its nvim's (a terminal labelled
// herdr-nvim.sidebar=<tab>), or its preview in nvim's place (a herdr-nvim
// mod pane). Closing them keeps nvim running (its daemon keeps buffers,
// unsaved edits and the cursor; herdr-nvim's RECIPE).
export function herdrNvimPanes(tab) {
  const nvims = new Set(terminals().filter((t) => t.state === "live" && t.labels?.["herdr-nvim.sidebar"] === tab).map((t) => t.address));
  return panes().filter((p) => p.tab === tab && ((p.terminal && nvims.has(p.terminal)) || (!p.terminal && p.mod === "herdr-nvim")));
}

// The layout path to the block whose mod is `name` (not a floating one).
function slotPath(node, name, trail = []) {
  if (!node) return null;
  const here = [...trail, node];
  if (node.content?.mod === name) return here;
  for (const child of node.children ?? []) {
    const found = slotPath(child, name, here);
    if (found) return found;
  }
  return null;
}

// The side panel holding gitview (the layout's `changes` slot), as the tab
// bar's Changes button sees it: {stack, slot, panel, shown}. Null when the
// layout has no gitview block.
export function panelState() {
  const path = slotPath(layout(), "gitview");
  if (!path) return null;
  const at = path.findLastIndex((n) => n.type === "stack");
  const stack = at >= 0 ? path[at] : null;
  const slot = path[path.length - 1];
  const panel = at > 0 ? path[at - 1] : slot;
  const visible = path.every((n) => !n.hidden);
  return {
    stack: stack?.id ?? null,
    slot: slot.id,
    panel: panel.id,
    shown: visible && (!stack || stack.active === slot.id),
  };
}

// Whether a pane copy in that tab is on screen: while its tab shows.
export function onScreen(tab) {
  return Boolean(tab) && focused()?.tab === tab;
}

// The shown space's folder, as the stock Changes panel finds it: the focused
// pane's live folder, else the space's. `on`: the machine it's on (null:
// this one). A mod pane (gitview's own diff pane) has no folder, herdr-nvim's
// nvim runs in its daemon's folder (not the file's repo), and a dialog over
// the tab focuses nothing: `keep` (the place before, in the same tab) stays.
export function location(keep) {
  const f = focused();
  const space = spaces().find((s) => s.active);
  const pane = f?.pane ? panes().find((p) => p.pane === f.pane) : null;
  const on = f?.pane === null ? machineOf(f?.terminal) : space?.machine ?? null;
  // A mod pane, herdr-nvim's nvim, or nothing in the tab (a dialog over
  // it): the place stays.
  const nvim = pane?.terminal && herdrNvimPanes(f.tab).some((p) => p.pane === pane.pane);
  if (keep && keep.tab === f?.tab && ((pane && !pane.terminal) || nvim || (!f?.pane && !f?.terminal))) return keep;
  return {
    cwd: pane?.cwd || f?.cwd || space?.folder || null,
    on,
    space: space?.id ?? null,
    tab: f?.tab ?? null,
  };
}
