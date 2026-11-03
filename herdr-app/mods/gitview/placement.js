// Where a gitview copy lives, decided once from its props: the Changes
// panel (the layout's `changes` slot) or cmd-g's tab of its own (the list's
// props {cwd, on, space, tab, own}, the diff pane's {space, own}).
// Everything that differs between the two is here, so the mods never ask
// "am I the panel?". Shared (linked; install copies it).

import { focused, panes, props, run, spaces } from "herdr";
import { location } from "./engine.js";
import { diffPane, isOwnTab, onScreen, ownTab, panelDiffPanes, panelState } from "./shared.js";
import { takeKeyboard } from "./modkit.js";

// ---- the list ------------------------------------------------------------------

// The panel: the shown space's repo (engine.js location), driving the
// focused tab's diff pane, which it opens on first use (not in cmd-g's
// tabs: their own list drives those). It owns its show/hide cycle.
const PANEL = {
  context: (keep) => location(keep),
  tab: (ctx) => (ctx?.tab && !isOwnTab(ctx.tab) ? ctx.tab : null),
  shown: () => Boolean(panelState()?.shown),
  opensDiffPane: true,
  closesForHerdrNvim: true,
  takesKeyboard: false,
  frame: (e, t, focused) => e.rounded(10).overflow_hidden().border_1().border_color(focused ? t.accent : t.border),
  cycle: () => new PanelCycle(),
  // The keyboard and the app's focus together, the one way the app has for
  // a layout block: `pane.go right` from the pane beside it (the tab's diff
  // pane, at its right edge; else the focused pane), as herdr-nvim hands its
  // tree the keyboard. The app then sends "focus" (the list takes its
  // handle) and moves its focus ring off the pane. A handle taken alone
  // left the ring on the diff pane: two panes looked focused.
  focusSelf: (list, cx) => {
    const dp = diffPane(list.tab());
    if (dp) run("pane.focus", { pane: dp.pane });
    run("pane.go", { direction: "right" });
    cx.spawn(async (cx) => {
      // A pane between (no diff pane): further right, a few times.
      for (let i = 0; i < 3 && !list.viaApp; i++) {
        await cx.sleep(150);
        if (!list.viaApp && !list.handle.is_focused()) run("pane.go", { direction: "right" });
      }
    });
  },
};

// cmd-g's tab: the repo it was opened for, its own tab's diff pane (opened
// with it); the app frames it like any pane. It takes the keyboard when it
// starts.
const TAB = {
  context: () => {
    const p = props() ?? {};
    return { cwd: p.cwd ?? null, on: p.on ?? null, space: p.space ?? null, tab: p.tab ?? null };
  },
  tab: () => props()?.tab ?? null,
  shown: () => onScreen(props()?.tab),
  opensDiffPane: false,
  closesForHerdrNvim: false,
  takesKeyboard: true,
  showsFirstFile: true, // its pane opens with it: the first file shows at once
  frame: (e) => e,
  cycle: () => null,
  // cmd-g's list is a pane: the app's focus onto it, then its handle.
  focusSelf: (list, cx) => {
    const me = panes().find((p) => p.tab === props()?.tab && p.mod === "gitview");
    if (me) run("pane.focus", { pane: me.pane });
    list.viaApp = true;
    takeKeyboard(list, cx);
  },
};

export const listPlacement = () => (props()?.own || props()?.cwd ? TAB : PANEL);

// The panel's show/hide cycle, as the TUI's toggle did. Hidden by the
// person (cmd-shift-g, the Changes or Files button): the diff panes it
// opened in the shown space's tabs close. Hidden because the person went to
// another space whose panel is closed (the tab bar puts each space's panel
// back): nothing closes, so coming back finds the diff as it was. Shown:
// look() says so, and the list opens its file in the tab's diff pane when
// the tab has none. The list looks from a timer too: a hidden copy isn't
// drawn, but its timers run, so a hide is seen whatever tab shows and
// whether or not it has a diff pane.
class PanelCycle {
  constructor() {
    this.shown = null; // not looked yet
    this.space = null;
    this.aside = false; // hidden for cmd-g's tab (stepAside)
  }

  // cmd-g's tab has a list of its own: while it shows, the panel steps
  // aside (its diff panes stay), and it comes back when the person leaves
  // that tab, if it stepped aside.
  stepAside() {
    const p = panelState();
    if (!p) return;
    const own = isOwnTab(focused()?.tab);
    if (own && p.shown && !this.aside) {
      this.aside = true;
      this.shown = false; // not a hide: nothing closes
      run("layout.hide", { id: p.panel });
    } else if (!own && this.aside) {
      this.aside = false;
      if (p.stack) run("layout.select", { stack: p.stack, child: p.slot });
      run("layout.show", { id: p.panel });
    }
  }

  // Whether stepAside has something to do.
  asideStale() {
    return isOwnTab(focused()?.tab) ? Boolean(panelState()?.shown) && !this.aside : this.aside;
  }

  // "shown" on the edge to shown (the first look counts); on a hide the
  // person made, {hidden: the shown space's tabs} (their diff panes are
  // closed; the list closes the rest of the unit); else null.
  look() {
    const shown = PANEL.shown();
    const space = spaces().find((s) => s.active) ?? null;
    const moved = this.space !== null && space?.id !== this.space;
    this.space = space?.id ?? null;
    if (shown === this.shown) return null;
    this.shown = shown;
    if (shown) return "shown";
    if (moved || !space) return null;
    const tabs = new Set(space.tabs.map((t) => t.id));
    for (const pane of panelDiffPanes()) if (tabs.has(pane.tab)) run("pane.close", { pane: pane.pane });
    return { hidden: tabs };
  }
}

// ---- the diff pane ---------------------------------------------------------------

// Opened by the panel in a tab ({tab}): it has × and gives way to
// herdr-nvim's nvim. In cmd-g's tab ({space, own}): half the tab, kept.
const PANEL_PANE = { tab: () => props()?.tab ?? null, closeButton: true, closesForHerdrNvim: true };
const TAB_PANE = { tab: () => ownTab(props()?.space ?? null), closeButton: false, closesForHerdrNvim: false };

export const panePlacement = () => (props()?.own ? TAB_PANE : PANEL_PANE);
