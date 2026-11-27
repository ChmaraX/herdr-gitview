import { world, reset } from "./stub.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
const { default: GitDiff } = await import("../mods/gitview-diff/main.js");
const { notesForView } = await import("../mods/gitview/shared.js");
const { noteForAgent } = await import("../mods/gitview/actions.js");
const { displayRows, noteRange } = await import("../mods/gitview-diff/diffview.js");
const { jumpToNote } = await import("../mods/gitview/notesview.js");
const { MODES } = await import("../mods/gitview/modes.js");

test("historical questions retain their revision and selected snippet", () => {
  reset();
  const v = new GitDiff();
  Object.assign(v, {
    view: { root: "/repo", on: null, path: "a.rs", section: "commit", commit: "abc123" },
    diff: { rows: [{ k: "ins", n: 12, m: "+new code" }] },
    cursor: 0, anchor: null,
    inPreview: () => false,
    composer: { set_value() {}, focus() {} },
    flash: { set(message) { assert.fail(message); } },
    focusHere() {},
  });
  v.annotate();
  v.saveNote("Why this change?", { notify() {} });
  const [note] = world.shared["gitview.notes"][":/repo"];
  assert.equal(note.commit, "abc123");
  assert.equal(note.start, 12);
  assert.equal(note.side, "new");
  assert.equal(note.snippet, "+new code\n");
  assert.match(noteForAgent(note).text, /commit abc123/);
  assert.match(noteForAgent(note).text, /Why this change\?/);
});

test("notes stay on their commit and working-tree side", () => {
  reset();
  world.shared["gitview.notes"] = { repo: [
    { file: "a", commit: "abc", text: "historical" },
    { file: "a", commit: "def", text: "other commit" },
    { file: "a", text: "unstaged" },
    { file: "a", cached: true, text: "staged" },
    { file: "b", commit: "abc", text: "other file" },
  ] };
  const texts = (view) => notesForView("repo", { path: "a", ...view }).map(n => n.text);
  assert.deepEqual(texts({ commit: "abc" }), ["historical"]);
  assert.deepEqual(texts({}), ["unstaged"]);
  assert.deepEqual(texts({ cached: true }), ["staged"]);
  assert.deepEqual(texts({ section: "branch" }), ["unstaged"]);
});

test("ordinary questions retain their existing message", () => {
  assert.equal(noteForAgent({ file: "a", text: "Why?" }).text, "Why?");
});

for (const deletedFile of [true, false]) {
  test(`saved deletion-only notes stay on the old side (${deletedFile ? "deleted file" : "mixed diff"})`, () => {
    reset();
    const rows = [
      { k: "del", o: 2, n: null, m: "-old" },
      { k: "del", o: 3, n: null, m: "-more" },
      ...(deletedFile ? [] : [{ k: "ins", o: null, n: 3, m: "+replacement" }]),
    ];
    const v = new GitDiff();
    Object.assign(v, {
      view: { root: "/repo", path: "gone.rs", section: "commit", commit: "abc123" },
      diff: { rows }, cursor: 1, anchor: 0,
      inPreview: () => false,
      composer: { set_value() {}, focus() {} },
      focusHere() {},
    });
    v.annotate();
    v.saveNote("Why remove this?", { notify() {} });
    const [note] = notesForView(v.notesKey(), v.view);
    assert.equal(note.side, "old");
    assert.equal(note.start, 2);
    assert.equal(note.end, 3);
    const display = displayRows(v.diff, [note], null);
    const at = display.findIndex(item => item.note);
    assert.equal(display[at - 1].index, 1);
    assert.equal(display.filter(item => item.note).length, 1);
  });
}

test("mixed selections use only new-side coordinates, not old-side maxima", () => {
  const range = noteRange({ rows: [
    { k: "del", o: 100, n: null },
    { k: "ins", o: null, n: 2 },
  ] }, 0, 1);
  assert.deepEqual(range, { side: "new", start: 2, end: 2, snippet: "" });
});

for (const state of ["unchanged", "deleted", "changed"]) {
  test(`a historical note opens its saved commit, not the ${state} working file`, async () => {
    reset();
    world.readTextFile = async () => JSON.stringify({ bin: "gitview-test" });
    const calls = [];
    world.exec = async (_bin, args) => {
      calls.push(args);
      return { code: 0, stdout: JSON.stringify({ files: [
        { path: "first.rs", status: "M" },
        { path: "dir/saved.rs", status: state === "deleted" ? "D" : "M" },
      ] }) };
    };
    const cx = {};
    let selected;
    const v = {
      work: {
        where: { root: "/repo", on: "remote" },
        findFile() { assert.fail("historical navigation must not search current changes"); },
      },
      loads: { run: (_cx, _what, work) => work(cx) },
      scroll: { top: 100 },
      flash: { set(message) { assert.fail(message); } },
      setMode(mode, cx, how) {
        assert.equal(mode, "commit");
        this.mode = mode;
        MODES[mode].enter(this, cx, how);
      },
      select(row, _cx, { line }) {
        this.commitFiles.select(row);
        selected = this.commitFiles.viewFor(row, line);
      },
    };
    await jumpToNote(v, { file: "dir/saved.rs", commit: "abc123456", side: "old", end: 3 }, cx);
    assert.equal(calls.length, 1);
    assert.deepEqual(calls[0].slice(-8), ["commit-files", "--repo", "/repo", "--root", "--sha", "abc123456", "--on", "remote"]);
    assert.equal(selected.path, "dir/saved.rs");
    assert.equal(selected.commit, "abc123456");
    assert.deepEqual(selected.line, { o: 3 });
  });
}

test("working-tree notes still select files mode on the saved section", () => {
  const row = {};
  const cx = {};
  const v = {
    work: { findFile(path, section) { assert.equal(path, "a"); assert.equal(section, "staged"); return row; } },
    setMode(mode, context, how) {
      assert.equal(mode, "files");
      assert.equal(context, cx);
      assert.deepEqual(how, { row, line: 4 });
    },
  };
  jumpToNote(v, { file: "a", cached: true, end: 4 }, cx);
});

test("loading a note's diff reveals the saved old-side line", async () => {
  reset();
  const rows = [{ k: "ctx", o: 1, n: 1 }, { k: "del", o: 3, n: null }, { k: "ins", o: null, n: 3 }];
  world.readTextFile = async () => JSON.stringify({ bin: "gitview-test" });
  world.exec = async () => ({ code: 0, stdout: JSON.stringify({ rows }) });
  const v = new GitDiff();
  let loading;
  Object.assign(v, {
    view: { root: "/repo", path: "a", section: "commit", commit: "abc" },
    loads: { latest(_cx, _what, work) { loading = work(); } },
    publishCursor() {}, resetScroll() {}, anchor: null,
  });
  v.loadDiff({}, { keep: false, line: { o: 3 } });
  await loading;
  assert.equal(v.cursor, 1);
  assert.deepEqual(v.pendingReveal, { row: 1, center: true });
});
