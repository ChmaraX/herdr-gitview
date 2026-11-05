// Where a gitview copy lives, decided once from its props (read in init,
// kept: `props()` names the copy whose turn the app is in, and a task woken
// by a promise another callback resolved can run in another copy's turn,
// where it's null; the diff pane lost 1 move in 8 to that): the Changes
// panel (the layout's `changes` slot) or cmd-g's tab of its own (the list's
// props {cwd, on, space, tab, own}, the diff pane's {space, own}).
// Everything that differs between the two is here, so the mods never ask
// "am I the panel?". Shared (linked; install copies it).

import { focused, panes, spaces } from "herdr";
import { run } from "./telemetry.js";
import { isOwnTab, ownTab } from "./shared.js";
import { diffPane, location, onScreen, panelState } from "./layout.js";
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
const tabList = (p) => ({
  context: () => ({ cwd: p.cwd ?? null, on: p.on ?? null, space: p.space ?? null, tab: p.tab ?? null }),
  tab: () => p.tab ?? null,
  shown: () => onScreen(p.tab),
  opensDiffPane: false,
  closesForHerdrNvim: false,
  takesKeyboard: true,
  showsFirstFile: true, // its pane opens with it: the first file shows at once
  frame: (e) => e,
  cycle: () => null,
  // cmd-g's list is a pane: the app's focus onto it, then its handle.
  focusSelf: (list, cx) => {
    const me = panes().find((x) => x.tab === p.tab && x.mod === "gitview");
    if (me) run("pane.focus", { pane: me.pane });
    list.viaApp = true;
    takeKeyboard(list, cx);
  },
});

// `p`: the copy's props, from its init.
export const listPlacement = (p) => (p?.own || p?.cwd ? tabList(p) : PANEL);

// The panel's show/hide edges, from the last look (`was`: {shown, space})
// and this one (`now`: {shown, space: the active space's id or null, tabs:
// its tab ids}). Pure; returns {was: the next look's, edge}:
// - "shown": the panel showed (the first look counts): the list opens its
//   file when the tab has no diff pane;
// - {hidden: tabs, space}: the person hid it (cmd-shift-g, the Changes or
//   Files button): the unit in the shown space's tabs closes (the diff
//   panes, nvim, an editor);
// - null: nothing, or hidden because the person went to another space
//   whose panel is closed (the tab bar puts each space's panel back):
//   nothing closes, so coming back finds the diff as it was.
export function cycleEdge(was, now) {
  const next = { shown: now.shown, space: now.space };
  if (now.shown === was.shown) return { was: next, edge: null };
  if (now.shown) return { was: next, edge: "shown" };
  const moved = was.space !== null && now.space !== was.space;
  return { was: next, edge: moved || !now.space ? null : { hidden: new Set(now.tabs), space: now.space } };
}

// The panel's cycle, as the TUI's toggle did (cycleEdge). The list looks
// from a timer too: a hidden copy isn't drawn, but its timers run, so a
// hide is seen whatever tab shows and whether or not it has a diff pane.
// cmd-shift-g also says so itself (`hiddenBy`), before it hides the panel:
// the list may look only after it's shown again.
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

  // This look's edge (cycleEdge).
  look() {
    const space = spaces().find((s) => s.active) ?? null;
    const { was, edge } = cycleEdge(this, { shown: PANEL.shown(), space: space?.id ?? null, tabs: space?.tabs.map((t) => t.id) ?? [] });
    Object.assign(this, was);
    return edge;
  }

  // cmd-shift-g hides the panel in `space`: the hide edge, unless a look
  // saw it already (then null). The next look that finds it shown is a
  // "shown" edge.
  hiddenBy(spaceId) {
    if (this.shown !== true) return null;
    this.shown = false;
    const space = spaces().find((s) => s.id === spaceId);
    return space ? { hidden: new Set(space.tabs.map((t) => t.id)), space: space.id } : null;
  }
}

// ---- the diff pane ---------------------------------------------------------------

// Opened by the panel in a tab ({tab}): it has × and gives way to
// herdr-nvim's nvim. In cmd-g's tab ({space, own}): half the tab, kept.
const panelPane = (p) => ({ tab: () => p.tab ?? null, closeButton: true, closesForHerdrNvim: true });
const tabPane = (p) => ({ tab: () => ownTab(p.space ?? null), closeButton: false, closesForHerdrNvim: false });

// `p`: the copy's props, from its init.
export const panePlacement = (p) => (p?.own ? tabPane(p) : panelPane(p ?? {}));
