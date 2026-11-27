import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { default: GitDiff } = await import("../mods/gitview-diff/main.js");
const { notesForView } = await import("../mods/gitview/shared.js");
const { noteForAgent } = await import("../mods/gitview/actions.js");
const { jumpToNote } = await import("../mods/gitview/notesview.js");

const BASE = { label: "origin/main", merge_base: "0123456789abcdef" };

// A diff pane on a "vs base" file, ready to take a note.
function branchPane(rows, selection = {}) {
  const v = new GitDiff();
  Object.assign(v, {
    view: { root: "/repo", on: null, path: "a.rs", section: "branch", cached: false, base: BASE },
    diff: { rows }, cursor: 0, anchor: null, ...selection,
    inPreview: () => false,
    composer: { set_value() {}, focus() {} },
    flash: { set(message) { assert.fail(message); } },
    focusHere() {},
  });
  return v;
}

test("a writes a note on a vs-base line; on new-side lines the agent gets its plain text", () => {
  reset();
  const v = branchPane([{ k: "ins", o: null, n: 12, m: "+new code" }]);
  v.annotate();
  v.saveNote("Why?", { notify() {} });
  const [note] = notesForView(v.notesKey(), v.view);
  assert.equal(noteForAgent(note).text, "Why?");
});

test("a vs-base note on removed lines names the base its numbers refer to", () => {
  reset();
  const v = branchPane([{ k: "del", o: 7, n: null, m: "-old" }, { k: "del", o: 8, n: null, m: "-more" }], { cursor: 1, anchor: 0 });
  v.annotate();
  v.saveNote("Why remove this?", { notify() {} });
  const [note] = notesForView(v.notesKey(), v.view);
  const sent = noteForAgent(note);
  assert.equal(sent.text, "[Removed lines; line numbers refer to origin/main @ 0123456]\nWhy remove this?");
  assert.equal(sent.base, undefined);
});

test("a note on removed lines of the uncommitted diff keeps its plain message", () => {
  assert.equal(noteForAgent({ file: "a", section: "changes", side: "old", text: "Why?" }).text, "Why?");
});

test("a vs-base note on removed lines saved with no known base says the numbers are the base's", () => {
  assert.equal(noteForAgent({ file: "a", section: "branch", side: "old", base: null, text: "Why?" }).text, "[Removed lines; line numbers refer to the base branch]\nWhy?");
});

test("cards: a new-side note of either scope shows in both, an old-side one where it was written, a staged one not in vs base", () => {
  reset();
  world.shared["gitview.notes"] = { repo: [
    { file: "a", section: "changes", side: "new", text: "unstaged new" },
    { file: "a", section: "branch", side: "new", text: "branch new" },
    { file: "a", section: "changes", side: "old", text: "unstaged old" },
    { file: "a", section: "branch", side: "old", text: "branch old" },
    { file: "a", section: "staged", cached: true, side: "new", text: "staged" },
  ] };
  const texts = (view) => notesForView("repo", { path: "a", ...view }).map((n) => n.text);
  assert.deepEqual(texts({ section: "branch" }), ["unstaged new", "branch new", "branch old"]);
  assert.deepEqual(texts({ section: "changes" }), ["unstaged new", "branch new", "unstaged old"]);
});

// A list view whose loaded file list is `scope`'s and has `rows`
// ("section:path" → row). Its own `scope` is the other one, as right after
// `w`: the loaded list's counts.
function listView(scope, rows) {
  const seen = { flash: null, mode: null };
  const v = {
    seen, scope: scope === "branch" ? "worktree" : "branch",
    work: { status: { scope }, findFile: (path, section) => (section ? rows[`${section}:${path}`] : Object.entries(rows).find(([k]) => k.endsWith(`:${path}`))?.[1]) ?? null },
    flash: { set(message) { seen.flash = message; } },
    setMode(mode, _cx, how) { seen.mode = { mode, ...how }; },
  };
  return v;
}

test("go to a note: opens in vs base when that scope shows it", () => {
  const row = {};
  const v = listView("branch", { "branch:a": row });
  jumpToNote(v, { file: "a", section: "changes", side: "new", end: 4 }, {});
  assert.deepEqual(v.seen.mode, { mode: "files", row, line: 4 });
});

test("go to a note: in uncommitted a vs-base note opens on the unstaged or conflicted file, never the staged one", () => {
  const changes = {};
  const v = listView("worktree", { "staged:a": {}, "changes:a": changes });
  jumpToNote(v, { file: "a", section: "branch", side: "new", end: 4 }, {});
  assert.equal(v.seen.mode.row, changes);
  const conflict = {};
  const merging = listView("worktree", { "staged:a": {}, "conflicts:a": conflict });
  jumpToNote(merging, { file: "a", section: "branch", side: "new", end: 4 }, {});
  assert.equal(merging.seen.mode.row, conflict);
});

test("go to a note: in uncommitted, a note only vs base shows says the way there and opens nothing", () => {
  // Committed since: no uncommitted change holds the note's line now.
  let v = listView("worktree", {});
  jumpToNote(v, { file: "a", section: "changes", side: "new", end: 4 }, {});
  assert.equal(v.seen.flash, "it's in vs base: esc, then w");
  // Removed lines of the branch's diff, with the file changed again.
  v = listView("worktree", { "changes:a": {} });
  jumpToNote(v, { file: "a", section: "branch", side: "old", end: 4 }, {});
  assert.equal(v.seen.flash, "it's in vs base: esc, then w");
  assert.equal(v.seen.mode, null);
});

test("go to a note: in vs base, a staged note says the way to uncommitted and opens nothing", () => {
  const v = listView("branch", { "branch:a": {} });
  jumpToNote(v, { file: "a", section: "staged", cached: true, end: 4 }, {});
  assert.equal(v.seen.flash, "it's in uncommitted: esc, then w");
  assert.equal(v.seen.mode, null);
});

test("go to a note: when the loaded scope is the one that shows the note and the file has no changes there, it says so", () => {
  let v = listView("branch", {});
  jumpToNote(v, { file: "a", section: "changes", side: "new", end: 4 }, {});
  assert.equal(v.seen.flash, "a has no changes now");
  v = listView("worktree", {});
  jumpToNote(v, { file: "a", section: "staged", cached: true, end: 4 }, {});
  assert.equal(v.seen.flash, "a has no changes now");
});
