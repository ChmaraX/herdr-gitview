// zoom.js: on_size_change reports zoomed px, the layout APIs take layout px,
// so the mod converts with unzoom(). The guard (triage risk 2) is the point:
// a settings.json without a sane "zoom" must stay identity, never divide by
// zero or NaN a list down to one row.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { zoom, unzoom } = await import("../mods/gitview/zoom.js");

test("a real zoom: unzoom undoes it, so zoomed px become layout px", () => {
  reset({ settings: { zoom: 0.9 } });
  assert.equal(zoom(), 0.9);
  assert.equal(unzoom(90), 100); // a 90-zoomed-px viewport is 100 layout px
  reset({ settings: { zoom: 1.25 } });
  assert.equal(unzoom(250), 200);
});

test("zoom 1 is exact identity (every existing expectation runs here)", () => {
  reset({ settings: { zoom: 1 } });
  assert.equal(unzoom(340), 340);
  reset({ settings: { zoom: 1.0 } });
  assert.equal(unzoom(340), 340);
});

test("a missing, zero or junk zoom is identity, never 0 or NaN", () => {
  for (const bad of [undefined, 0, null, "x", NaN, -0]) {
    reset(bad === undefined ? {} : { settings: { zoom: bad } });
    assert.equal(zoom(), 1, `zoom=${String(bad)}`);
    assert.equal(unzoom(340), 340, `unzoom with zoom=${String(bad)}`);
  }
  // settings() itself returning nullish must not throw either.
  world.settings = null;
  assert.equal(zoom(), 1);
});
