// How the gitview mods talk: shared state only (`setShared` / `shared`),
// so the file list and the diff pane never call each other, and every key
// is read and written here. Shared by the gitview mods (linked; install
// copies it).
//
// - "gitview.views": {<tab>: view}: what the tab's diff pane shows (written
//   by the list on a file click, j/k or after a change, by the pane on J/K).
//   A view is model.js viewOf() plus `seq` (bumped on every write),
//   `showing` (bumped when the write shows another file than the one
//   before: the pane starts it afresh), `by` ("list" or "pane"), `files`
//   (the list's file order, for J/K) and `line` (a note to jump to);
//   `{empty: true, seq, showing}` when there's nothing to show.
// - "gitview.cursor": {<tab>: {root, on, path, section, commit, line, row,
//   first, preview}}: where the pane's cursor is (the list's Enter opens
//   nvim there); `preview`: the pane shows the file's Preview.
// - "gitview.keys": {overlay}: the keys card is open (keymap.js).
// - "gitview.notes": {"<machine or ''>:<repo root>": [note]}: review notes,
//   kept until sent. A note is {id, file, section, cached, start, end,
//   text, snippet}; `cached`: written on the staged diff.
// - "gitview.dirty": {root, on, at}: the pane staged or discarded something
//   (the list refreshes and picks the file the pane shows next).
// - "gitview.changed": {"<machine or ''>:<repo root>": at}: a folder watch
//   saw the repo change. With two copies watching one folder only the
//   newest wakes (herdr-app 0.1.12), so the copy that woke says so here and
//   the others follow (modkit.js watch).
// - "gitview.handoff": {<tab>: {root, on, path, section, commit, line, seq,
//   at, seen}}: Enter handed the tab's diff to herdr-nvim's nvim (its pane
//   took the diff pane's place). `seq`: the tab's view then; `seen`: the
//   list saw nvim's pane. When that pane goes, the list brings the diff
//   back (main.js followHandoff).
// - "gitview.tree": the list shows folders as a tree.
// - "gitview.own": {<space>: <tab>}: the space's gitview tab (cmd-g).
// - "gitview.origins": {<space>: <tab>}: where cmd-g was pressed (it goes
//   back there).

import { focused, layout, panes, setShared, shared, spaces, terminals } from "herdr";
import { sameFile } from "./model.js";

export function views() {
  return shared("gitview.views") ?? {};
}

// The tab's view as stored (with its seq), empty or not.
export function viewIn(tab) {
  return tab ? views()[tab] ?? null : null;
}

// A view with something to show, or null.
export const liveView = (view) => (view && !view.empty && view.path ? view : null);

const liveTabs = () => new Set(spaces().flatMap((s) => s.tabs.map((t) => t.id)));

// Every tab's view, without the tabs that are gone. `view` null: the pane
// shows nothing.
export function setView(tab, view) {
  const live = liveTabs();
  const all = Object.fromEntries(Object.entries(views()).filter(([t]) => live.has(t)));
  const before = all[tab];
  const seq = (before?.seq ?? 0) + 1;
  const showing = (before?.showing ?? 0) + (sameFile(liveView(before), view) ? 0 : 1);
  all[tab] = view ? { ...view, empty: false, seq, showing } : { empty: true, seq, showing };
  setShared("gitview.views", all);
  return all[tab];
}

export function cursorIn(tab) {
  return (shared("gitview.cursor") ?? {})[tab] ?? null;
}

export function setCursor(tab, cursor) {
  const live = liveTabs();
  const all = Object.fromEntries(Object.entries(shared("gitview.cursor") ?? {}).filter(([t]) => live.has(t)));
  all[tab] = cursor;
  setShared("gitview.cursor", all);
}

export function dirty() {
  return shared("gitview.dirty") ?? null;
}

export function setDirty(root, on) {
  setShared("gitview.dirty", { root, on: on ?? null, at: Date.now() });
}

export function treeMode() {
  return Boolean(shared("gitview.tree"));
}

export function setTreeMode(on) {
  setShared("gitview.tree", Boolean(on));
}

export function origins() {
  return shared("gitview.origins") ?? {};
}

export function setOrigin(space, tab) {
  setShared("gitview.origins", { ...origins(), [space]: tab });
}

// The machine of an app address ("vps/tb12", "vps/t3"); null for this one.
export function machineOf(address) {
  const s = String(address ?? "");
  return s.includes("/") ? s.split("/")[0] : null;
}

// When a watch last saw the (local) repo change.
export function changedAt(root) {
  return (shared("gitview.changed") ?? {})[notesKey(null, root)] ?? 0;
}

export function stampChanged(root, at) {
  setShared("gitview.changed", { ...(shared("gitview.changed") ?? {}), [notesKey(null, root)]: at });
}

// ---- the handoff to herdr-nvim ----------------------------------------------------

export function handoffs() {
  return shared("gitview.handoff") ?? {};
}

export const handoffIn = (tab) => (tab ? handoffs()[tab] ?? null : null);

// `rec` null: the tab has none.
export function setHandoff(tab, rec) {
  const all = { ...handoffs() };
  if (rec) all[tab] = rec;
  else if (tab in all) delete all[tab];
  else return;
  setShared("gitview.handoff", all);
}

// The panes herdr-nvim shows in the tab: its nvim's (a terminal labelled
// herdr-nvim.sidebar=<tab>), or its preview in nvim's place (a herdr-nvim
// mod pane). Closing them keeps nvim running (its daemon keeps buffers,
// unsaved edits and the cursor; herdr-nvim's RECIPE).
export function herdrNvimPanes(tab) {
  const nvims = new Set(terminals().filter((t) => t.state === "live" && t.labels?.["herdr-nvim.sidebar"] === tab).map((t) => t.address));
  return panes().filter((p) => p.tab === tab && ((p.terminal && nvims.has(p.terminal)) || (!p.terminal && p.mod === "herdr-nvim")));
}

// ---- notes ------------------------------------------------------------------

export const notesKey = (on, root) => `${on ?? ""}:${root}`;

export function notesOf(key) {
  return (shared("gitview.notes") ?? {})[key] ?? [];
}

export function setNotes(key, list) {
  const all = { ...(shared("gitview.notes") ?? {}) };
  if (list.length) all[key] = list;
  else delete all[key];
  setShared("gitview.notes", all);
}

export function addNote(key, note) {
  const id = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  setNotes(key, [...notesOf(key), { ...note, id }]);
}

export function removeNotes(key, ids) {
  setNotes(key, notesOf(key).filter((n) => !ids.includes(n.id)));
}

// ---- panes ------------------------------------------------------------------

// The tab's diff pane, if it has one.
export function diffPane(tab) {
  return panes().find((p) => p.tab === tab && p.mod === "gitview-diff") ?? null;
}

// The diff panes the Changes panel opened, in every tab (not the ones in
// cmd-g's gitview tabs, which belong to their own list).
export function panelDiffPanes() {
  return panes().filter((p) => p.mod === "gitview-diff" && !isOwnTab(p.tab));
}

// One of cmd-g's gitview tabs.
export function isOwnTab(tab) {
  return Object.values(shared("gitview.own") ?? {}).includes(tab);
}

// The space's gitview tab (cmd-g), while it's there.
export function ownTab(space) {
  const tab = (shared("gitview.own") ?? {})[space];
  const s = spaces().find((x) => x.id === space);
  return tab && s?.tabs.some((t) => t.id === tab) ? tab : null;
}

export function setOwnTab(space, tab) {
  setShared("gitview.own", { ...(shared("gitview.own") ?? {}), [space]: tab });
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

// Work that render() asks for runs here, in a task of the copy's own (from
// init), never in a task started from a render: returns kick(), which wakes
// it. Several kicks before it runs are one.
export function pump(cx, work) {
  let wake = null;
  let pending = false;
  cx.spawn(async (cx) => {
    for (;;) {
      if (!pending) await new Promise((resolve) => (wake = resolve));
      wake = null;
      pending = false;
      try {
        work(cx);
      } catch (e) {
        console.error(`gitview: ${e?.message ?? e}`);
      }
    }
  });
  return () => {
    pending = true;
    if (wake) wake();
  };
}

// Take the keyboard and make sure it stays: the app may hand it over a
// moment later than it draws the focus (a "focus" from pane.go, a click).
// Checks for a second, stops once held twice or when focus went elsewhere.
export function takeKeyboard(view, cx) {
  const where = JSON.stringify(focused());
  const arrival = (view.arrival = {});
  try { view.handle.focus(); } catch { /* not drawn yet */ }
  cx.spawn(async (cx) => {
    let held = 0;
    for (let step = 0; step < 10 && view.arrival === arrival; step++) {
      await cx.sleep(100);
      if (view.arrival !== arrival || JSON.stringify(focused()) !== where) break;
      if (view.handle.is_focused()) {
        if (++held >= 2) break;
        continue;
      }
      held = 0;
      try { view.handle.focus(); } catch { /* not drawn yet */ }
      cx.notify();
    }
    if (view.arrival === arrival) view.arrival = null;
  });
}
