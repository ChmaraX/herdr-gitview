// The diff pane's rows as drawn (diffview.js displayRows): where notes and
// the composer go.
import "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { displayRows, displayColumns, widestColumns } = await import("../mods/gitview-diff/diffview.js");

const rows = [{ k: "ctx", o: 1, n: 1 }, { k: "del", o: 2, n: null }, { k: "ins", o: null, n: 2 }, { k: "ctx", o: 3, n: 3 }];
const shape = (out) => out.map((x) => (x.note ? `note:${x.note.text}` : x.composer ? "composer" : `row:${x.index}`));

test("a file's notes come first; a line's note right under its last line (by the new side)", () => {
  const notes = [{ end: 2, text: "on 2" }, { end: 0, text: "file" }];
  assert.deepEqual(shape(displayRows({ rows }, notes, null)), ["note:file", "row:0", "row:1", "row:2", "note:on 2", "row:3"]);
});

test("a deleted line (no new number) gets no note; the composer goes under the row it's for", () => {
  assert.deepEqual(shape(displayRows({ rows }, [], 1)), ["row:0", "row:1", "composer", "row:2", "row:3"]);
});

// #26: the sideways range comes from the data, not from measuring drawn
// rows. A column count drives it, so a wide glyph (CJK, full-width, emoji)
// has to count as the two columns it draws, or the tail of a line of them
// would be cut. This is the one piece of pure logic the range leans on.
test("display columns: ASCII is one per code point, CJK / full-width / emoji are two", () => {
  assert.equal(displayColumns("hello"), 5);
  assert.equal(displayColumns(""), 0);
  assert.equal(displayColumns("\u4e16\u754c"), 4); // 世界: two wide
  assert.equal(displayColumns("a\u4e16b"), 4); // 1 + 2 + 1
  assert.equal(displayColumns("\uff21\uff22"), 4); // ＡＢ full-width
  assert.equal(displayColumns("\u{1f600}"), 2); // 😀 one code point, two columns
  assert.equal(displayColumns("a\u{1f600}b"), 4);
});

test("widest row: the row with the most display columns, summed across its spans", () => {
  const rows = [
    { s: [["short", null, null]] }, // 5
    { s: [["a", null, null], ["\u4e16\u754c", null, null]] }, // 1 + 4 = 5
    { s: [["0123456789", null, null]] }, // 10: the winner
    { k: "fold", s: [] }, // no text
  ];
  assert.equal(widestColumns(rows), 10);
  assert.equal(widestColumns([]), 0);
});

// Round 10 (U1): the engine stops at 5 000 rows and said so; the pane just
// ended, as if the file did.
test("a diff the engine cut ends with a row saying so", () => {
  const out = displayRows({ rows, truncated: true }, [], null);
  assert.deepEqual(out.at(-1), { cut: 4 });
  assert.equal(displayRows({ rows }, [], null).some((x) => x.cut), false);
});
