// The shared records two copies talk through (shared.js): the tab's view
// and the pane's asks.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { setView, ask, answer, askIn, liveAsk, ASK_MS } = await import("../mods/gitview/shared.js");

const SPACE = { id: "sp", active: true, tabs: [{ id: "t1" }] };
const file = (path, rev) => ({ root: "/r", on: null, path, section: "changes", commit: null, rev });

// Round 9: a :w in herdr-nvim moves the file's rev and rewrites the same
// view; the handoff took that for "another file shown" and the diff never
// came back.
test("the same file written again (a new rev) is not another file shown", () => {
  reset({ spaces: [SPACE] });
  const a = setView("t1", file("a.rs", "r1"));
  const b = setView("t1", file("a.rs", "r2"));
  assert.equal(b.showing, a.showing);
  assert.ok(b.seq > a.seq);
});

test("another file, or a file after the pane showed nothing, is another showing", () => {
  reset({ spaces: [SPACE] });
  const a = setView("t1", file("a.rs", "r1"));
  const b = setView("t1", file("b.rs", "r1"));
  assert.equal(b.showing, a.showing + 1);
  const empty = setView("t1", null);
  assert.equal(setView("t1", file("b.rs", "r1")).showing, empty.showing + 1);
});

test("a tab that's gone leaves the views (shared.json stays bounded)", () => {
  reset({ spaces: [SPACE] });
  world.shared["gitview.views"] = { gone: { path: "x" } };
  setView("t1", file("a.rs", "r1"));
  assert.deepEqual(Object.keys(world.shared["gitview.views"]), ["t1"]);
});

// Round 10 (B1): shared state outlives the app; a new list copy took the
// last answered discard for a new one and discarded the file again.
test("an answered ask is gone: a new list copy has nothing to redo", () => {
  reset({ spaces: [SPACE] });
  const seq = ask("t1", "discard", file("a.rs", "r1"));
  assert.equal(askIn("t1").op, "discard");
  answer("t1", seq);
  assert.equal(askIn("t1"), null);
  assert.equal(world.shared["gitview.ask"].t1, undefined);
});

test("an ask from before (over 10 s old, or written without a time) is not done", () => {
  const a = { seq: 3, op: "discard", at: 1000 };
  assert.equal(liveAsk(a, null, 1000 + ASK_MS + 1), null);
  assert.equal(liveAsk({ seq: 3, op: "stage" }, null, Date.now()), null);
  assert.equal(liveAsk(a, null, 1000 + ASK_MS), a);
});

test("the ask after an answer counts on from it (the pane never takes the old answer for its own)", () => {
  reset({ spaces: [SPACE] });
  answer("t1", ask("t1", "stage", file("a.rs", "r1")));
  const next = ask("t1", "unstage", file("a.rs", "r2"));
  assert.ok(next > world.shared["gitview.answer"].t1.seq);
  assert.equal(askIn("t1").seq, next);
});

test("an answer to an older ask leaves a newer one in place", () => {
  reset({ spaces: [SPACE] });
  const first = ask("t1", "next", file("a.rs", "r1"));
  const second = ask("t1", "next", file("a.rs", "r1"));
  answer("t1", first);
  assert.equal(askIn("t1").seq, second);
});
