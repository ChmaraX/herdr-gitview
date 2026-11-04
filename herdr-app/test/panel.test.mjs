// The panel's show/hide edges (placement.js cycleEdge, PanelCycle): what
// the list closes and opens when the person hides and shows it.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { cycleEdge, listPlacement } = await import("../mods/gitview/placement.js");

const A = { id: "spA", active: true, tabs: [{ id: "t1" }, { id: "t2" }] };
const B = { id: "spB", active: false, tabs: [{ id: "t3" }] };
// A layout whose changes slot shows gitview, open or hidden.
const panel = (hidden) => ({ id: "root", children: [{ id: "panel", hidden, children: [{ id: "changes", content: { mod: "gitview" } }] }] });
const at = (shown, space = A) => ({ shown, space: space?.id ?? null, tabs: space?.tabs.map((t) => t.id) ?? [] });

test("the first look at a shown panel is a show (the list opens its file)", () => {
  assert.equal(cycleEdge({ shown: null, space: null }, at(true)).edge, "shown");
});

test("a hide in the same space closes that space's tabs' unit, not another's", () => {
  const { edge } = cycleEdge({ shown: true, space: "spA" }, at(false));
  assert.deepEqual([...edge.hidden], ["t1", "t2"]);
  assert.equal(edge.space, "spA");
});

test("going to a space whose panel is closed closes nothing (coming back finds the diff as it was)", () => {
  assert.equal(cycleEdge({ shown: true, space: "spA" }, at(false, B)).edge, null);
});

test("no change, no edge: a look every 300 ms doesn't reopen or close anything", () => {
  assert.equal(cycleEdge({ shown: true, space: "spA" }, at(true)).edge, null);
  assert.equal(cycleEdge({ shown: false, space: "spA" }, at(false)).edge, null);
});

// Round 9: the launcher closed the panes itself and the list's look came
// only after the panel was shown again: no edge, no file.
test("a cmd-shift-g hide the look missed: its unit closes, and the next look is a show", () => {
  reset({ spaces: [A, B], layout: panel(false) });
  const cycle = listPlacement().cycle();
  assert.equal(cycle.look(), "shown");
  const hidden = cycle.hiddenBy("spA"); // the panel shown again before any look
  assert.deepEqual([...hidden.hidden], ["t1", "t2"]);
  assert.equal(cycle.look(), "shown");
});

test("cmd-shift-g's hide closes the space it hid, even after a quick switch to another", () => {
  reset({ spaces: [{ ...A, active: false }, { ...B, active: true }], layout: panel(false) });
  const cycle = listPlacement().cycle();
  cycle.look();
  assert.equal(cycle.hiddenBy("spA").space, "spA");
});

test("a hide the look saw first isn't closed a second time by cmd-shift-g's message", () => {
  reset({ spaces: [A], layout: panel(false) });
  const cycle = listPlacement().cycle();
  cycle.look();
  world.layout = panel(true);
  assert.ok(cycle.look()?.hidden);
  assert.equal(cycle.hiddenBy("spA"), null);
});
