// The Scroller's sideways range and clamp (scroller.js): the rules the diff
// pane's horizontal scrolling leans on, as pure state (no element built).
import "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { Scroller } = await import("../mods/gitview/scroller.js");

const cx = { notify() {} };

// A horizontal scroller with a known viewport.
function scroller(vw) {
  const s = new Scroller("t", { horizontal: true });
  s.vw = vw;
  return s;
}

test("a line that fits the pane never scrolls sideways", () => {
  const s = scroller(400);
  s.measureWidth(300, cx); // narrower than the pane
  assert.equal(s.range(), 400); // never under the viewport
  assert.equal(s.scrollBy(40), false); // nothing to move to
  assert.equal(s.left, 0);
});

test("a wide line sets the range to its width plus the room, and clamps the offset to it", () => {
  const s = scroller(400);
  s.setPad(14); // 2 columns of room
  s.measureWidth(1000, cx);
  assert.equal(s.range(), 1014);
  assert.equal(s.scrollBy(10000), true); // far right
  assert.equal(s.left, 1014 - 400); // clamped to range - viewport
  assert.equal(s.scrollBy(-10000), true); // far left
  assert.equal(s.left, 0);
});

test("the range only grows: a narrower row after a wide one is ignored (no oscillation)", () => {
  const s = scroller(400);
  assert.equal(s.measureWidth(1000, cx), true);
  assert.equal(s.measureWidth(600, cx), false); // narrower: no change
  assert.equal(s.measureWidth(1000, cx), false); // same width: no change
  assert.equal(s.contentW, 1000);
  assert.equal(s.measureWidth(1200, cx), true); // a wider row grows it
  assert.equal(s.contentW, 1200);
});

test("another file resets the range and offset to zero", () => {
  const s = scroller(400);
  s.measureWidth(1000, cx);
  s.scrollBy(200);
  assert.ok(s.left > 0 && s.contentW === 1000);
  s.resetX();
  assert.equal(s.left, 0);
  assert.equal(s.contentW, 0);
  assert.equal(s.range(), 400); // back to just the viewport
});

test("more room (wider columns) extends the range and re-clamps the offset", () => {
  const s = scroller(400);
  s.measureWidth(1000, cx);
  s.scrollBy(10000);
  assert.equal(s.left, 600);
  s.setPad(100); // columns got wider
  assert.equal(s.range(), 1100);
  assert.equal(s.left, 600); // still in range (range - viewport = 700)
  s.scrollBy(10000);
  assert.equal(s.left, 700);
});
