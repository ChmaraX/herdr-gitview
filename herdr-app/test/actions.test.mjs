// The discard confirm (actions.js): what the dialog's Confirm runs.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { confirmDiscard, takeDiscard } = await import("../mods/gitview/actions.js");

const tokenOf = () => Object.keys(world.shared["gitview.discards"]).at(-1);

// Round 10: the Confirm's command reached another tab's diff pane, which
// didn't hold the token, and nothing was discarded.
test("whichever copy hears the Confirm does the discard it asked for, once", () => {
  reset();
  confirmDiscard("gitview-diff", "a.rs", { view: { path: "a.rs" }, tab: "t1" });
  const token = tokenOf();
  assert.deepEqual(takeDiscard("gitview-diff", `discard:${token}`), { view: { path: "a.rs" }, tab: "t1" });
  assert.equal(takeDiscard("gitview-diff", `discard:${token}`), null);
});

test("another confirm's token, a made-up one or the other mod's does nothing", () => {
  reset();
  confirmDiscard("gitview", "keep.txt", { paths: ["keep.txt"] });
  const keep = tokenOf();
  confirmDiscard("gitview", "drop.txt", { paths: ["drop.txt"] });
  const drop = tokenOf();
  assert.deepEqual(takeDiscard("gitview", `discard:${drop}`).paths, ["drop.txt"]);
  assert.equal(takeDiscard("gitview", "discard:nottheone"), null);
  assert.equal(takeDiscard("gitview", "discard:"), null);
  assert.equal(takeDiscard("gitview-diff", `discard:${keep}`), null);
});
