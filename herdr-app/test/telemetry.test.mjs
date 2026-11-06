// The trace (telemetry.js): what it writes, how often, and what it leaves out.
import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdir, readFile, rm } from "node:fs/promises";
const { Tracer, shortArgs, traceDir } = await import("../mods/gitview/telemetry.js");

const lines = async () => {
  const out = [];
  for (const f of await readdir(traceDir()).catch(() => [])) out.push(...(await readFile(`${traceDir()}/${f}`, "utf8")).trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)));
  return out;
};
const fresh = async () => {
  reset({ panes: [{ pane: "p1", tab: "t1", mod: "gitview-diff" }, { pane: "p2", tab: "t2", mod: null, terminal: "t9" }] });
  await rm(traceDir(), { recursive: true, force: true });
};

test("lines wait for the flush, and a flush writes them once, in order", async () => {
  await fresh();
  const tr = new Tracer("list", () => ({ tab: "t1", space: "s1" }));
  tr.line("input", { key: "j" });
  tr.line("exec", { op: "status", ms: 12 });
  assert.deepEqual(await lines(), []);
  await tr.flush();
  await tr.flush(); // nothing new: nothing written
  const got = await lines();
  assert.deepEqual(got.map((l) => [l.kind, l.mod, l.tab, l.space, l.copy]), [["input", "list", "t1", "s1", tr.copy], ["exec", "list", "t1", "s1", tr.copy]]);
});

test("off (shared gitview.trace false): nothing", async () => {
  await fresh();
  world.shared["gitview.trace"] = false;
  const tr = new Tracer("list", () => ({}));
  tr.line("input", { key: "j" });
  tr.warn("a warning");
  await tr.flush();
  assert.deepEqual(await lines(), []);
});

test("a warn flushes at once with a snapshot, at most one snapshot per 10 s", async () => {
  await fresh();
  const tr = new Tracer("pane", () => ({ tab: "t1" }), () => ({ mode: "diff" }));
  tr.warn("first");
  tr.warn("second");
  await tr.flush();
  const got = await lines();
  assert.deepEqual(got.map((l) => l.kind), ["warn", "snapshot", "warn"]);
  const snap = got[1];
  assert.deepEqual(snap.panes.map((p) => p.pane), ["p1"]); // the tab's panes only
  assert.deepEqual(snap.state, { mode: "diff" });
});

// Round 12: no file contents, commit messages or note bodies.
test("snapshots carry sizes and counts, never note text; commit messages are cut from args", async () => {
  await fresh();
  world.shared["gitview.notes"] = { ":/repo": [{ id: 1, text: "SECRET NOTE BODY", path: "a.rs" }] };
  world.shared["gitview.views"] = { t1: { path: "a.rs", section: "changes" } };
  const tr = new Tracer("list", () => ({ tab: "t1" }));
  await tr.mark(42);
  const [mark] = await lines();
  assert.equal(mark.kind, "mark");
  assert.equal(mark.id, 42);
  assert.deepEqual(mark.shared.mine.notes, { ":/repo": 1 });
  assert.deepEqual(mark.shared.mine.views, { path: "a.rs", section: "changes" });
  assert.ok(!JSON.stringify(mark).includes("SECRET"));
  assert.equal(shortArgs(["commit", "--repo", "/r", "--", "my secret message"]), "commit --repo /r -- …");
  assert.equal(shortArgs(["stage", "--section", "changes", "--", "a.rs"]), "stage --section changes -- a.rs");
});

test("renders: one summary per 10 s with count and max, a line for a render over 16 ms", async () => {
  await fresh();
  const tr = new Tracer("list", () => ({}));
  for (let i = 0; i < 5; i++) tr.render(() => 1);
  tr.render(() => { const end = Date.now() + 20; while (Date.now() < end); });
  tr.summary(); // under 10 s: not yet
  tr.summary(true);
  await tr.flush();
  const got = await lines();
  assert.equal(got.filter((l) => l.slow).length, 1);
  const sum = got.find((l) => l.n);
  assert.equal(sum.n, 6);
  assert.ok(sum.max >= 16);
});
