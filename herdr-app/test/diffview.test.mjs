// The diff pane's rows as drawn (diffview.js displayRows): where notes and
// the composer go.
import "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { displayRows } = await import("../mods/gitview-diff/diffview.js");

const rows = [{ k: "ctx", o: 1, n: 1 }, { k: "del", o: 2, n: null }, { k: "ins", o: null, n: 2 }, { k: "ctx", o: 3, n: 3 }];
const shape = (out) => out.map((x) => (x.note ? `note:${x.note.text}` : x.composer ? "composer" : `row:${x.index}`));

test("a file's notes come first; a line's note right under its last line (by the new side)", () => {
  const notes = [{ end: 2, text: "on 2" }, { end: 0, text: "file" }];
  assert.deepEqual(shape(displayRows({ rows }, notes, null)), ["note:file", "row:0", "row:1", "row:2", "note:on 2", "row:3"]);
});

test("a deleted line (no new number) gets no note; the composer goes under the row it's for", () => {
  assert.deepEqual(shape(displayRows({ rows }, [], 1)), ["row:0", "row:1", "composer", "row:2", "row:3"]);
});
