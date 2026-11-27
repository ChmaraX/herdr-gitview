import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { default: GitDiff } = await import("../mods/gitview-diff/main.js");

const ROWS = [
  { k: "ctx", o: 1, n: 1 },
  { k: "del", o: 2, n: null },
  { k: "ins", o: null, n: 2 },
  { k: "ins", o: null, n: 3 },
  { k: "ctx", o: 3, n: 4 },
];
const NOTE = { file: "a.rs", section: "changes", cached: false };

// A diff pane on an unstaged file with `notes` saved for it; keys go
// through the pane's key table.
function pane(notes, cursor, more = {}) {
  reset();
  world.shared["gitview.notes"] = { ":/repo": notes.map((n) => ({ ...NOTE, ...n })) };
  const v = new GitDiff();
  const view = { root: "/repo", on: null, path: "a.rs", section: "changes", cached: false };
  const seen = { flash: null, value: null, editor: 0 };
  Object.assign(v, {
    seen, view, diffView: view,
    diff: { rows: ROWS }, cursor, anchor: null, pendingNote: null,
    inPreview: () => false,
    composer: { set_value(text) { seen.value = text; }, focus() {} },
    flash: { set(message) { seen.flash = message; } },
    focusHere() {},
    edit() { seen.editor += 1; },
    ...more,
  });
  return v;
}
const press = (v, key) => v.command(`key:${key}`, { notify() {} });
const saved = () => world.shared["gitview.notes"]?.[":/repo"] ?? [];

const TWO = [
  { id: "n1", side: "new", start: 2, end: 3, snippet: "+x\n", text: "old text" },
  { id: "n2", side: "new", start: 4, end: 4, text: "another line's" },
];

test("e on a note's lines puts its text in the composer where its card was, not in the editor", () => {
  const v = pane(TWO, 2);
  press(v, "e");
  assert.equal(v.seen.editor, 0);
  assert.equal(v.seen.value, "old text");
  assert.equal(v.pendingNote.edit, "n1");
  assert.equal(v.pendingNote.after, 3);
});

test("a saved rewrite changes that note's text and nothing else", () => {
  const v = pane(TWO, 2);
  press(v, "e");
  v.saveNote("new text", { notify() {} });
  assert.deepEqual(saved(), [
    { ...NOTE, id: "n1", side: "new", start: 2, end: 3, snippet: "+x\n", text: "new text" },
    { ...NOTE, id: "n2", side: "new", start: 4, end: 4, text: "another line's" },
  ]);
  assert.equal(v.pendingNote, null);
});

test("an edit saved empty keeps the note as it was", () => {
  const v = pane([{ id: "n1", side: "new", start: 2, end: 2, text: "old text" }], 2);
  press(v, "e");
  v.saveNote("   ", { notify() {} });
  assert.equal(saved()[0].text, "old text");
});

test("a rewrite of a note that's gone is saved as a new note on the same lines, and says so", () => {
  const v = pane([{ id: "n1", side: "new", start: 2, end: 2, snippet: "+x\n", text: "old text" }], 2);
  press(v, "e");
  world.shared["gitview.notes"] = {};
  v.saveNote("new text", { notify() {} });
  assert.equal(v.seen.flash, "the note was gone: saved as a new one");
  const [{ id, ...note }] = saved();
  assert.notEqual(id, "n1");
  assert.deepEqual(note, { ...NOTE, side: "new", start: 2, end: 2, snippet: "+x\n", text: "new text" });
  assert.equal(v.pendingNote, null);
});

test("d deletes the note on the cursor's lines, and only it", () => {
  const v = pane([
    { id: "n1", side: "new", start: 2, end: 3, text: "this one" },
    { id: "n2", side: "new", start: 4, end: 4, text: "another line's" },
  ], 3);
  press(v, "d");
  assert.deepEqual(saved().map((n) => n.id), ["n2"]);
});

test("d on the line above a note says there is none and deletes nothing", () => {
  const v = pane([{ id: "n1", side: "new", start: 3, end: 3, text: "the next line's" }], 2);
  press(v, "d");
  assert.equal(v.seen.flash, "no note on this line");
  assert.equal(saved().length, 1);
});

test("e edits a removed-line note only from its removed row, not from the added row with the same number", () => {
  // Row 1 is old line 2; row 2 is new line 2.
  const v = pane([{ id: "n1", side: "old", start: 2, end: 2, text: "removed" }], 2);
  press(v, "e");
  assert.equal(v.seen.editor, 1);
  v.cursor = 1;
  press(v, "e");
  assert.equal(v.pendingNote.after, 1);
});

test("d on the line below a note says there is none and deletes nothing", () => {
  const v = pane([{ id: "n1", side: "new", start: 2, end: 2, text: "elsewhere" }], 4);
  press(v, "d");
  assert.equal(v.seen.flash, "no note on this line");
  assert.equal(saved().length, 1);
});

test("d does not delete a note whose card this diff doesn't show (its last line is in no row)", () => {
  const v = pane([{ id: "n1", side: "new", start: 3, end: 9, text: "ends below the hunk" }], 3);
  press(v, "d");
  assert.equal(v.seen.flash, "no note on this line");
  assert.equal(saved().length, 1);
});

test("d on the unstaged diff does not delete a staged note on the same line", () => {
  const v = pane([{ id: "n1", section: "staged", cached: true, side: "new", start: 2, end: 2, text: "staged" }], 2);
  press(v, "d");
  assert.equal(saved().length, 1);
});

const HERE = { id: "n1", side: "new", start: 2, end: 2, text: "here" };

test("in a shown Preview e opens the editor and d deletes nothing, on a note's line too", () => {
  const v = pane([HERE], 2, { inPreview: () => true });
  v.preview = { view: v.view, failed: false };
  press(v, "e");
  assert.equal(v.seen.editor, 1);
  assert.equal(v.pendingNote, null);
  press(v, "d");
  assert.equal(v.seen.flash, "that works in Diff: m switches");
  assert.equal(saved().length, 1);
});

test("e opens the editor while another file's diff is still the one shown", () => {
  const v = pane([HERE], 2, { diffView: { root: "/repo", on: null, path: "before.rs", section: "changes" } });
  press(v, "e");
  assert.equal(v.seen.editor, 1);
  assert.equal(v.pendingNote, null);
});
