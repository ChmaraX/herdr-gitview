// The keys the footer and the card offer (keymap.js), from the list's
// tables (modes.js).
import "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { chipsOf, cardOf } = await import("../mods/gitview/keymap.js");
const { MODES } = await import("../mods/gitview/modes.js");

const caps = (entries) => entries.flatMap((e) => e.caps);
const view = (scope) => ({ scope, handedOff: () => false, list: () => null });

// Round 10 (U5): in "vs base" the row refuses s/u/x and there's no commit
// box, yet the footer and the card offered them.
test("vs base, the footer and the card don't offer s, u, x or c", () => {
  const v = view("branch");
  for (const k of ["s", "u", "x", "c"]) {
    assert.equal(caps(chipsOf([MODES.files.keys], v)).includes(k), false, k);
    assert.equal(caps(cardOf([{ title: "", table: MODES.files.keys }], v)[0].keys).includes(k), false, k);
  }
});

test("in the working tree they're there, most useful first", () => {
  const chips = caps(chipsOf([MODES.files.keys], view("worktree")));
  assert.deepEqual(chips.slice(0, 2), ["s", "u"]);
  assert.ok(["x", "c"].every((k) => chips.includes(k)));
});
