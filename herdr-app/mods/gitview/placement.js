// Where a gitview copy lives, decided once from its props: the Changes
// panel (the layout's `changes` slot) or cmd-g's tab of its own (the list's
// props {cwd, on, space, tab, own}, the diff pane's {space, own}).
// Everything that differs between the two is here, so the mods never ask
// "am I the panel?". Shared (linked; install copies it).

import { props, run } from "herdr";
import { location } from "./engine.js";
import { isOwnTab, onScreen, ownTab, panelDiffPanes, panelState } from "./shared.js";

// ---- the list ------------------------------------------------------------------

// The panel: the shown space's repo (engine.js location), driving the
// focused tab's diff pane, which it opens on first use (not in cmd-g's
// tabs: their own list drives those). It owns its show/hide cycle.
const PANEL = {
  panel: true,
  context: (keep) => location(keep),
  tab: (ctx) => (ctx?.tab && !isOwnTab(ctx.tab) ? ctx.tab : null),
  shown: () => Boolean(panelState()?.shown),
  opensDiffPane: true,
  closesForHerdrNvim: true,
  takesKeyboard: false,
  frame: (e, t, focused) => e.rounded(10).overflow_hidden().border_1().border_color(focused ? t.accent : t.border),
  cycle: () => new PanelCycle(),
};

// cmd-g's tab: the repo it was opened for, its own tab's diff pane (opened
// with it); the app frames it like any pane. It takes the keyboard when it
// starts.
const TAB = {
  panel: false,
  context: () => {
    const p = props() ?? {};
    return { cwd: p.cwd ?? null, on: p.on ?? null, space: p.space ?? null, tab: p.tab ?? null };
  },
  tab: () => props()?.tab ?? null,
  shown: () => onScreen(props()?.tab),
  opensDiffPane: false,
  closesForHerdrNvim: false,
  takesKeyboard: true,
  frame: (e) => e,
  cycle: () => null,
};

export const listPlacement = () => (props()?.own || props()?.cwd ? TAB : PANEL);

// The panel's show/hide cycle, as the TUI's toggle did. Hidden (cmd-shift-g,
// the Changes or Files button, a space whose panel is closed): the diff
// panes it opened close, in every tab. Shown: look() says so, and the list
// opens its file in the tab's diff pane. The list looks from a timer too:
// a hidden copy isn't drawn, but its timers run, so a hide is seen whatever
// tab shows and whether or not it has a diff pane.
class PanelCycle {
  constructor() {
    this.shown = null; // not looked yet
  }

  // "shown" on the edge to shown (the first look counts), else null.
  look() {
    const shown = PANEL.shown();
    if (shown === this.shown) return null;
    this.shown = shown;
    if (shown) return "shown";
    for (const pane of panelDiffPanes()) run("pane.close", { pane: pane.pane });
    return null;
  }
}

// ---- the diff pane ---------------------------------------------------------------

// Opened by the panel in a tab ({tab}): it has × and gives way to
// herdr-nvim's nvim. In cmd-g's tab ({space, own}): half the tab, kept.
const PANEL_PANE = { tab: () => props()?.tab ?? null, closeButton: true, closesForHerdrNvim: true };
const TAB_PANE = { tab: () => ownTab(props()?.space ?? null), closeButton: false, closesForHerdrNvim: false };

export const panePlacement = () => (props()?.own ? TAB_PANE : PANEL_PANE);
