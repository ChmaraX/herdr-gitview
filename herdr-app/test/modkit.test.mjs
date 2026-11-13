// The list's runtime (modkit.js): which watch paths count, and how loads
// of one kind coalesce and come back after a lost exec.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { batchPaths, Loads, commandLoop } = await import("../mods/gitview/modkit.js");

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

// Feed commands into nextCommand (stub.mjs): each one in turn, then block.
const feed = (...commands) => {
  let i = 0;
  world.nextCommand = () => (i < commands.length ? Promise.resolve(commands[i++]) : new Promise(() => {}));
};
// A tracer that only records, for asserting what the loop would write.
const recorder = () => {
  const lines = [];
  const warns = [];
  return { lines, warns, line: (kind, fields = {}) => lines.push({ kind, ...fields }), warn: (text, fields = {}) => warns.push({ text, ...fields }) };
};
const tick = async (n = 30) => { for (let i = 0; i < n; i++) await Promise.resolve(); };

// Round 23: the programmatic commit/note commands carry text. The loop must
// log a redacted representation at every sink while the handler still gets
// the original (keys.js rebuilds the message from it).
const CANARY = "commit:AUDIT_CANARY_no_real_secret";

test("commandLoop input line: the handler gets the original command, the trace does not", async () => {
  reset();
  feed(CANARY);
  const tr = recorder();
  let handled = null;
  commandLoop(cx, { loads: new Loads(), handle: (command) => { handled = command; }, name: "gitview", tr });
  await tick();
  assert.equal(handled, CANARY); // original reaches the handler
  const input = tr.lines.find((l) => l.kind === "input");
  assert.equal(input.command, "commit:<redacted>");
  assert.ok(!JSON.stringify(tr.lines).includes("CANARY"));
});

test("commandLoop waiting warn: neither its text nor its field exposes the payload", async () => {
  reset();
  feed(CANARY);
  const tr = recorder();
  // Fake loads that stay busy until the waiting callback fires, with time
  // advanced past the 3 s threshold so the warn path runs.
  const realNow = Date.now;
  let t = 1_000_000;
  Date.now = () => t;
  try {
    const loads = { names: () => "status", idle: async (_cx, waiting) => { t += 4000; waiting(); } };
    commandLoop(cx, { loads, handle: () => {}, name: "gitview", tr });
    await tick();
  } finally {
    Date.now = realNow;
  }
  const warn = tr.warns.find((w) => w.waiting !== undefined);
  assert.ok(warn, "the waiting warn fired");
  assert.equal(warn.waiting, "commit:<redacted>"); // the field
  assert.ok(warn.text.includes("commit:<redacted>")); // the text
  assert.ok(!JSON.stringify(tr.warns).includes("CANARY"));
});

test("commandLoop error path: a throwing handler cannot leak the payload via console.error or the warn line", async () => {
  reset();
  feed(CANARY);
  const tr = recorder();
  const errors = [];
  const realError = console.error;
  console.error = (...a) => errors.push(a.join(" "));
  try {
    commandLoop(cx, { loads: new Loads(), handle: () => { throw new Error("boom"); }, name: "gitview", tr });
    await tick();
  } finally {
    console.error = realError;
  }
  assert.ok(errors.length, "the handler threw and was logged");
  assert.ok(!errors.join("\n").includes("CANARY"));
  assert.ok(errors.join("\n").includes("commit:<redacted>"));
  const warn = tr.warns.find((w) => w.level === "error");
  assert.ok(warn.text.includes("commit:<redacted>"));
  assert.ok(!warn.text.includes("CANARY"));
});

// The focus fast path keeps its own args (they are safe) but still routes
// through the redactor, so a future focus: variant cannot reopen the hole.
test("commandLoop focus fast path: traced and handled, args intact", async () => {
  reset();
  feed("focus:panel");
  const tr = recorder();
  let handled = null;
  commandLoop(cx, { loads: new Loads(), handle: (command) => { handled = command; }, name: "gitview", tr });
  await tick();
  assert.equal(handled, "focus:panel");
  assert.equal(tr.lines.find((l) => l.kind === "input").command, "focus:panel");
});
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
