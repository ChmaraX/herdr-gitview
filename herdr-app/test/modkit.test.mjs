// The list's runtime (modkit.js): which watch paths count, and how loads
// of one kind coalesce and come back after a lost exec.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { batchPaths, Loads } = await import("../mods/gitview/modkit.js");

test("watch batches: git's own churn and ignored folders don't count; the repo itself or 50+ paths is everything", () => {
  reset();
  world.shared["gitview.ignored"] = { ":/r": ["target/"] };
  assert.deepEqual(batchPaths("/r", ["/r/src/a.rs", "/r/.git/objects/ab/cd", "/r/.git/index.lock", "/r/target/x", "/elsewhere/y", "/r/.git/index"]), ["src/a.rs", ".git/index"]);
  assert.equal(batchPaths("/r", ["/r"]), "all");
  assert.equal(batchPaths("/r", Array.from({ length: 51 }, (_, i) => `/r/f${i}`)), "all");
  assert.deepEqual(batchPaths("/r", ["/r/.git/logs/HEAD"]), []);
});

// A cx as the app gives a copy: spawn runs a task, sleep waits.
const cx = { spawn: (f) => f(cx), sleep: (ms) => new Promise((r) => setTimeout(r, Math.min(ms, 5))), notify() {} };
const gate = () => {
  let open;
  const p = new Promise((r) => (open = r));
  return { p, open };
};

test("a status asked three times while one runs runs once more, with the last ask", async () => {
  reset();
  const loads = new Loads();
  const ran = [];
  const first = gate();
  loads.latest(cx, "status", async () => { ran.push(1); await first.p; });
  for (const n of [2, 3, 4]) loads.latest(cx, "status", async () => { ran.push(n); });
  first.open();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(ran, [1, 4]);
});

test("the run after a loud ask and a quiet one shows the spinner (quiet only if every ask was)", async () => {
  reset();
  const loads = new Loads();
  const first = gate();
  loads.latest(cx, "status", () => first.p, { quiet: true });
  let quiet = null;
  loads.latest(cx, "status", async () => {}, { quiet: false });
  loads.latest(cx, "status", async () => { quiet = [...loads.running.values()].at(-1).quiet; }, { quiet: true });
  first.open();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(quiet, false);
});

// App bug #1: an exec dropped when a mod pane closed never settles; revive
// runs the load again, and if the old reply comes after all it's ignored.
test("after a lost status, the late old reply is not taken over the new one", async () => {
  reset();
  const loads = new Loads();
  const old = gate();
  const taken = [];
  let calls = 0;
  loads.latest(cx, "status", async (cx, current) => {
    const me = ++calls;
    if (me === 1) await old.p;
    if (current()) taken.push(me);
  });
  assert.equal(loads.busy(), true);
  loads.revive(cx, Date.now());
  await new Promise((r) => setTimeout(r, 10));
  old.open();
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(taken, [2]);
});

test("a lost load tells its copy (onLost), so its own flags (a commit running) end", () => {
  reset();
  const loads = new Loads();
  let lost = null;
  loads.onLost = (cx, whats) => (lost = whats);
  loads.run(cx, "commit", () => new Promise(() => {}));
  loads.revive(cx, Date.now());
  assert.deepEqual(lost, ["commit"]);
  assert.equal(loads.busy(), false);
});
