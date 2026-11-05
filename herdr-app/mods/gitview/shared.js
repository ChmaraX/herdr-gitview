// How the gitview mods talk: shared state only (`setShared` / `shared`),
// so the file list and the diff pane never call each other, and every key
// is read and written here. Shared by the gitview mods (linked; install
// copies it).
//
// - "gitview.views": {<tab>: view}: what the tab's diff pane shows, written
//   only by the tab's list (a click, j/k, after a change, on the pane's
//   ask). A view is model.js viewOf() (with the file's `rev`, which moves
//   when its diff can: the pane reloads then) plus `seq` (bumped on every
//   write), `showing` (bumped when the write shows another file than the
//   one before: the pane starts it afresh), `line` (a note to jump to) and
//   `focus` (the pane takes the keyboard); `{empty: true, seq, showing}`
//   when there's nothing to show.
// - "gitview.ask": {<tab>: {seq, op, view, at}}: the diff pane asks the
//   tab's list to stage, unstage or discard its file (`op`), or to show the
//   next or previous one ("next", "prev": J/K). The answer removes it, and
//   one older than 10 s is ignored: shared state outlives the app, and a
//   new list copy must never do an old ask again (a discard).
// - "gitview.answer": {<tab>: {seq, msg}}: the list did the ask `seq` (and
//   wrote the view it led to); `msg`: what to say ("last file").
// - "gitview.cursor": {<tab>: {root, on, path, section, commit, line,
//   preview}}: where the pane's cursor is (the list's Enter opens nvim
//   there); `preview`: the pane shows the file's Preview (the pane's own
//   dedupe, and what the e2e reads).
// - "gitview.keys": {overlay}: the keys card is open (keymap.js).
// - "gitview.discards": {<token>: {mod, at, payload}}: discards waiting on
//   the confirm dialog (actions.js), for whichever copy hears its Confirm.
// - "gitview.notes": {"<machine or ''>:<repo root>": [note]}: review notes,
//   kept until sent. A note is {id, file, section, cached, start, end,
//   text, snippet}; `cached`: written on the staged diff.
// - "gitview.changed": {"<machine or ''>:<repo root>": {at, prev, paths}}: a
//   folder watch saw the repo change; `paths`: the batch's repo-relative
//   paths that matter (at most 50), or "all"; `prev`: the stamp before. With
//   two copies watching one folder only the newest wakes (herdr-app
//   0.1.12), so the copy that woke says so here and the others follow
//   (modkit.js watch).
// - "gitview.ignored": {"<machine or ''>:<repo root>": [dir/]}: the repo's
//   outermost ignored folders, from the last status (a watch batch under
//   them is no change).
// - "gitview.handoff": {<tab>: {root, on, path, section, commit, line,
//   showing, at, seen}}: Enter handed the tab's diff to herdr-nvim's nvim
//   (its pane took the diff pane's place). `showing`: the tab's view's then
//   (another file shown since: no restore); `seen`: the
//   list saw nvim's pane. When that pane goes, the list brings the diff
//   back (main.js followHandoff).
// - "gitview.previewOff": [kind]: kinds of file shown as Diff, not Preview.
// - "gitview.paneWidth": px: the diff pane's last width (a new one lays out
//   at it right away).
// - "gitview.tree": the list shows folders as a tree.
// - "gitview.own": {<space>: <tab>}: the space's gitview tab (cmd-g).
// - "gitview.origins": {<space>: <tab>}: where cmd-g was pressed (it goes
//   back there).

import { setShared, shared, spaces } from "herdr";
import { sameFile } from "./model.js";
import { tracer } from "./telemetry.js";

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

// A per-tab (or per-space) map without the tabs (spaces) that are gone, so
// shared.json stays bounded.
const onlyLive = (all, live) => Object.fromEntries(Object.entries(all ?? {}).filter(([k]) => live.has(k)));
const liveSpaces = () => new Set(spaces().map((s) => s.id));

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

// The ask to do, if any: not answered yet and under 10 s old.
export const ASK_MS = 10000;
export function liveAsk(a, answered, now) {
  return a && a.seq > (answered?.seq ?? 0) && now - (a.at ?? 0) <= ASK_MS ? a : null;
}
export function askIn(tab) {
  return liveAsk((shared("gitview.ask") ?? {})[tab], answerIn(tab), Date.now());
}

// Returns the ask's seq (after the last ask and the last answer: an answer
// removes its ask).
export function ask(tab, op, view) {
  const all = onlyLive(shared("gitview.ask"), liveTabs());
  const seq = Math.max(all[tab]?.seq ?? 0, answerIn(tab)?.seq ?? 0) + 1;
  setShared("gitview.ask", { ...all, [tab]: { seq, op, view, at: Date.now() } });
  tracer().line("state", { ask: op, seq, path: view?.path ?? null });
  return seq;
}

export function answerIn(tab) {
  return (shared("gitview.answer") ?? {})[tab] ?? null;
}

export function answer(tab, seq, msg = null) {
  setShared("gitview.answer", { ...onlyLive(shared("gitview.answer"), liveTabs()), [tab]: { seq, msg } });
  tracer().line("state", { answer: seq, msg, for: tab });
  const asks = { ...(shared("gitview.ask") ?? {}) };
  if (asks[tab] && asks[tab].seq <= seq) {
    delete asks[tab];
    setShared("gitview.ask", asks);
  }
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
  setShared("gitview.origins", { ...onlyLive(origins(), liveSpaces()), [space]: tab });
}

// The machine of an app address ("vps/tb12", "vps/t3"); null for this one.
export function machineOf(address) {
  const s = String(address ?? "");
  return s.includes("/") ? s.split("/")[0] : null;
}

// The last change a watch saw in the (local) repo: {at, prev, paths} or null.
export function stampOf(root) {
  const s = (shared("gitview.changed") ?? {})[notesKey(null, root)];
  return s ?? null;
}

export const changedAt = (root) => stampOf(root)?.at ?? 0;

// Stamps older than an hour go (repos nobody watches any more).
export function stampChanged(root, at, paths) {
  const all = Object.fromEntries(Object.entries(shared("gitview.changed") ?? {}).filter(([, s]) => at - (s?.at ?? s ?? 0) < 3600000));
  all[notesKey(null, root)] = { at, prev: changedAt(root), paths };
  setShared("gitview.changed", all);
}

export function ignoredDirs(root) {
  return (shared("gitview.ignored") ?? {})[notesKey(null, root)] ?? [];
}

export function setIgnoredDirs(root, dirs) {
  const key = notesKey(null, root);
  const all = { ...(shared("gitview.ignored") ?? {}) };
  if (JSON.stringify(all[key] ?? []) === JSON.stringify(dirs)) return;
  // The 20 repos written last (newest last; a write moves its repo there).
  delete all[key];
  all[key] = dirs;
  setShared("gitview.ignored", Object.fromEntries(Object.entries(all).slice(-20)));
}

// ---- the diff pane's Diff | Preview ------------------------------------------------

// The kinds of file (preview.js: markdown, image, html) the person switched
// to Diff; a new pane (after nvim, in another tab) keeps the choice.
export function previewOff() {
  return shared("gitview.previewOff") ?? [];
}

export function setPreviewOff(kind, off) {
  const now = previewOff().filter((k) => k !== kind);
  setShared("gitview.previewOff", off ? [...now, kind] : now);
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
  tracer().line("state", { handoff: rec ? (rec.seen ? "seen" : "set") : "cleared", path: rec?.path ?? null, for: tab });
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

// ---- cmd-g's tabs -----------------------------------------------------------

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
  setShared("gitview.own", { ...onlyLive(shared("gitview.own"), liveSpaces()), [space]: tab });
}
