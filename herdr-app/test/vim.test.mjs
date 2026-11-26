// The diff pane's vim motions (vim.js): an opt-in bundle, off by default.
import { reset, world } from "./stub.mjs";
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
const { Vim, nextChange, relativeOn, rowOfLine, searchRows, setVim, vimOn, yankText } = await import("../mods/gitview-diff/vim.js");
const { relativeGutter } = await import("../mods/gitview-diff/diffview.js");

// A diff as the engine sends it: a fold, context, a change of two lines into
// one, context, an added line.
const rows = [
  { k: "fold", o: null, n: null, m: "" },
  { k: "ctx", o: 3, n: 3, m: "   alpha" },
  { k: "del", o: 4, n: null, m: "-  Beta" },
  { k: "del", o: 5, n: null, m: "-  gamma" },
  { k: "add", o: null, n: 4, m: "+  beta" },
  { k: "ctx", o: 6, n: 5, m: "   delta" },
  { k: "add", o: null, n: 6, m: "+  epsilon" },
];

// The pane as vim.js sees it.
function pane() {
  return {
    diff: { rows },
    cursor: 1,
    anchor: null,
    display: rows.map((_, index) => ({ index })),
    scroll: { left: 40, top: 0, height: 54, offsets: rows.map((_, i) => i * 18).concat(rows.length * 18), range: () => 300, clampX() { this.left = Math.min(this.left, 200); }, clamp() { this.top = Math.max(0, this.top); } },
    line(d) { this.cursor = Math.max(0, Math.min(rows.length - 1, this.cursor + d)); },
    page: () => 4,
    selection() { return this.anchor === null ? null : [Math.min(this.anchor, this.cursor), Math.max(this.anchor, this.cursor)]; },
    inPreview: () => false,
    ready: () => true,
  };
}
const ev = (key, extra = {}) => ({ key, key_char: key.length === 1 ? key : undefined, modifiers: {}, ...extra });
const keyName = (e) => (e.modifiers.control ? null : e.key_char ?? e.key);
function press(vim, v, keys, cx = {}) {
  return keys.map((e) => vim.key(v, typeof e === "string" ? ev(e) : e, keyName(typeof e === "string" ? ev(e) : e), cx));
}

beforeEach(() => reset());

test("off by default; the toggle is shared and relative numbers come with it", () => {
  assert.equal(vimOn(), false);
  assert.equal(relativeOn(), false);
  setVim(true);
  assert.equal(world.shared["gitview.vim"], true);
  assert.equal(relativeOn(), true);
});

test("a line goes to its new-side row, a removed one to its old side, a missing one to the nearest", () => {
  assert.equal(rowOfLine(rows, 4), 4); // new 4, not old 4
  assert.equal(rowOfLine(rows, 6), 6);
  assert.equal(rowOfLine(rows, 1), 1); // folded away: nearest shown
  assert.equal(rowOfLine([{ k: "fold" }], 3), -1);
});

test("]c / [c go to the first row of each run of changes", () => {
  assert.equal(nextChange(rows, 1, 1), 2);
  assert.equal(nextChange(rows, 2, 1), 6); // inside a run: the next run
  assert.equal(nextChange(rows, 6, 1), -1);
  assert.equal(nextChange(rows, 6, -1), 2);
});

test("search: smart case, wraps, skips folds", () => {
  assert.equal(searchRows(rows, "beta", 1, 1), 2); // lower case: any case
  assert.equal(searchRows(rows, "Beta", 2, 1), 2); // capital: exact, wraps to itself
  assert.equal(searchRows(rows, "alpha", 5, 1), 1); // wraps
  assert.equal(searchRows(rows, "alpha", 1, -1), 1);
  assert.equal(searchRows(rows, "zeta", 1, 1), -1);
});

test("yank: the code without markers, folds left out", () => {
  assert.equal(yankText(rows, 0, 2), "  alpha\n  Beta\n");
  assert.equal(yankText(rows, 0, 0), "");
});

test("relative gutter keeps the engine's width; the cursor row keeps its number", () => {
  assert.deepEqual(relativeGutter("   12 ", 5, 5), ["   12 ", "@gutter_now"]);
  assert.deepEqual(relativeGutter("   12 ", 2, 5), ["    3 ", "@gutter"]);
});

test("counts, gg, G and :<line>", () => {
  const v = pane(), vim = new Vim();
  press(vim, v, ["3", "j"]);
  assert.equal(v.cursor, 4);
  press(vim, v, ["2", "k"]);
  assert.equal(v.cursor, 2);
  press(vim, v, ["g", "g"]);
  assert.equal(v.cursor, 0);
  press(vim, v, ["G"]);
  assert.equal(v.cursor, 6);
  press(vim, v, ["5", "G"]);
  assert.equal(v.cursor, 5);
  press(vim, v, [":", "4"]);
  assert.equal(vim.prompt(), ":4");
  press(vim, v, ["enter"]);
  assert.equal(v.cursor, 4);
  assert.equal(vim.prompt(), null);
});

test("]c with a count, zt, 0 / $ and ctrl-d", () => {
  const v = pane(), vim = new Vim();
  press(vim, v, ["]", "c"]);
  assert.equal(v.cursor, 2);
  press(vim, v, ["[", "c"]);
  assert.equal(v.cursor, 2);
  assert.equal(vim.prompt(), "no earlier change");
  v.cursor = 1;
  press(vim, v, ["2", "]", "c"]);
  assert.equal(v.cursor, 6);
  press(vim, v, ["z", "t"]);
  assert.equal(v.scroll.top, 6 * 18);
  press(vim, v, ["0"]);
  assert.equal(v.scroll.left, 0);
  press(vim, v, [ev("$")]);
  assert.equal(v.scroll.left, 200);
  v.cursor = 0;
  press(vim, v, [ev("d", { key_char: undefined, modifiers: { control: true } })]);
  assert.equal(v.cursor, 2);
});

test("/ search, n / N, and :set nornu", () => {
  setVim(true);
  const v = pane(), vim = new Vim();
  press(vim, v, ["/", "d", "e", "l", "enter"]);
  assert.equal(v.cursor, 5);
  press(vim, v, ["n"]);
  assert.equal(v.cursor, 5); // the only match: wraps to itself
  press(vim, v, ["/", "a", "enter"]);
  assert.equal(v.cursor, 1); // past the last match: wraps to the first
  press(vim, v, ["N"]);
  assert.equal(v.cursor, 5); // and back, wrapping the other way
  press(vim, v, [":", "s", "e", "t", " ", "n", "o", "r", "n", "u", "enter"]);
  assert.equal(relativeOn(), false);
});

test("y copies the selection and clears it; keys it doesn't know are left to the pane", () => {
  const v = pane(), vim = new Vim();
  let copied = null;
  v.anchor = 1;
  v.cursor = 2;
  press(vim, v, ["y"], { write_to_clipboard: (t) => (copied = t) });
  assert.equal(copied, "  alpha\n  Beta\n");
  assert.equal(v.anchor, null);
  assert.deepEqual(press(vim, v, ["s", "J", "escape", "?"]), [false, false, false, false]);
});
