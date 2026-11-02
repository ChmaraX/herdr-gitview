// How the gitview mods talk: shared state only (`setShared` / `shared`),
// so the file list and the diff pane never call each other. Shared by
// gitview and gitview-diff (linked; install copies it).
//
// - "gitview.views": {<tab>: view}: what the tab's diff pane shows (written
//   by the list on a file click or j/k, by the pane on J/K). A view is
//   model.js viewOf() plus `seq` (bumped on every write), `by` ("list" or
//   "pane"), `files` (the list's file order, for J/K) and `line` (a note to
//   jump to).
// - "gitview.cursor": {<tab>: {seq, path, line, first}}: where the pane's
//   cursor is (the list's Enter opens nvim there).
// - "gitview.notes": {"<machine or ''>:<repo root>": [note]}: review notes,
//   kept until sent. A note is {id, file, start, end, text, snippet}.
// - "gitview.dirty": {root, on, at}: the pane staged or discarded something.
// - "gitview.own": {<space>: <tab>}: the space's gitview tab (cmd-g).
// - "gitview.tree": the list shows folders as a tree.

import { focused, layout, panes, setShared, shared, spaces } from "herdr";

export function views() {
  return shared("gitview.views") ?? {};
}

export function viewIn(tab) {
  return tab ? views()[tab] ?? null : null;
}

// Every tab's view, without the tabs that are gone.
export function setView(tab, view) {
  const live = new Set(spaces().flatMap((s) => s.tabs.map((t) => t.id)));
  const all = Object.fromEntries(Object.entries(views()).filter(([t]) => live.has(t)));
  const before = all[tab];
  all[tab] = { ...view, seq: (before?.seq ?? 0) + 1 };
  setShared("gitview.views", all);
  return all[tab];
}

export function cursorIn(tab) {
  return (shared("gitview.cursor") ?? {})[tab] ?? null;
}

export function setCursor(tab, cursor) {
  const live = new Set(spaces().flatMap((s) => s.tabs.map((t) => t.id)));
  const all = Object.fromEntries(Object.entries(shared("gitview.cursor") ?? {}).filter(([t]) => live.has(t)));
  all[tab] = cursor;
  setShared("gitview.cursor", all);
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

// Whether this copy is on screen: a pane copy while its tab shows, the
// panel while it's open on gitview.
export function onScreen(tab) {
  if (tab) return focused()?.tab === tab;
  return Boolean(panelState()?.shown);
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
