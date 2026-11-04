// What the list's look acts on (notice.js staleWhy).
import "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { staleWhy } = await import("../mods/gitview/notice.js");

// A shown panel list with nothing new.
const calm = { panesMoved: false, cycle: { was: true, aside: false }, shown: true, handoff: null, stamp: false, key: "k", wasKey: "k", tab: "t1", wasTab: "t1", ask: 0, askSeen: 0 };

test("nothing new: a look every 300 ms does nothing", () => {
  assert.equal(staleWhy(calm), null);
});

test("a hidden list sees its panel's edge and lost panes, nothing else (a hidden copy isn't drawn)", () => {
  const hidden = { ...calm, shown: false, cycle: { was: false, aside: false } };
  assert.equal(staleWhy({ ...hidden, key: "other", stamp: true, ask: 5 }), null);
  assert.equal(staleWhy({ ...hidden, panesMoved: true }), "panes");
  assert.equal(staleWhy({ ...hidden, shown: true }), "panel");
});

test("while nvim has the file, no look acts until its pane goes (no redraw every look)", () => {
  assert.equal(staleWhy({ ...calm, handoff: { seen: true, nvim: true, nvimSeen: true } }), null);
  assert.equal(staleWhy({ ...calm, handoff: { seen: true, nvim: false, nvimSeen: true } }), "handoff");
  assert.equal(staleWhy({ ...calm, handoff: { seen: false, nvim: false, nvimSeen: false } }), "handoff");
});

test("a pane ask counts once: a new one acts, a done or answered one (0) doesn't", () => {
  assert.equal(staleWhy({ ...calm, ask: 4, askSeen: 3 }), "ask");
  assert.equal(staleWhy({ ...calm, ask: 4, askSeen: 4 }), null);
  assert.equal(staleWhy({ ...calm, ask: 0, askSeen: 4 }), null);
});

test("another repo, tab or stamp is noticed; the panel's edge first", () => {
  assert.equal(staleWhy({ ...calm, key: "k2" }), "place");
  assert.equal(staleWhy({ ...calm, tab: "t2" }), "tab");
  assert.equal(staleWhy({ ...calm, stamp: true }), "stamp");
  assert.equal(staleWhy({ ...calm, stamp: true, cycle: { was: false, aside: false } }), "panel");
});

test("cmd-g's list (no cycle) acts when its tab shows", () => {
  assert.equal(staleWhy({ ...calm, cycle: null, stamp: true }), "stamp");
  assert.equal(staleWhy({ ...calm, cycle: null, shown: false, stamp: true }), null);
});
