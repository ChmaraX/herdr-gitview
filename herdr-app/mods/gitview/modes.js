// The list's views, one entry each: its title, which file list it acts
// on, what entering it does, its header, body, the commit row under it and
// its key table (keymap.js: the keys, the footer's keycaps and the keys
// card). The view looks its mode up here instead of asking "which
// mode?" everywhere; only the list's setMode() changes the mode, and it
// runs the entry's `enter(v, cx, how)`.
//   files: the changed files · commit: one commit's files (from history)
//   log: the history · notes: the review notes

import { div } from "gpui-kit";
import { commitBox, filesHeader, subHeader } from "./chrome.js";
import { renderRow, rowSize } from "./listview.js";
import { COMMIT_ROW, commitRow, loadLog, logMove, openCommit, openLog, toggleLogFilter } from "./history.js";
import { treeMode } from "./shared.js";
import { editorLabel } from "./nvim.js";

// Enter's label: the editor it starts there (nvim, vim, open, …).
// On the row handed to herdr-nvim it brings the diff back.
const enterLabel = (v) => (v.handedOff(v.list()?.row()) ? "diff" : editorLabel(v.list()?.where.root, v.list()?.where.on));
import { NOTE_ROW, deleteNote, jumpToNote, noteMove, noteRow } from "./notesview.js";

const down = (v, cx) => v.move(1, cx);
const up = (v, cx) => v.move(-1, cx);
const back = (to) => (v, cx) => v.setMode(to, cx);
// s/u/x and the commit box: the working tree only (not "vs base").
const worktree = (v) => v.scope === "worktree";
// 6 px above and below the rows, like the stock panel.
const padded = (sizes) => [6, ...sizes, 6];
// Takes the growing space as the rows do, so the commit box and the footer
// stay at the bottom with no files, while loading or on an error.
const empty = (t, text) => div().flex_1().min_h(0).px(16).py(12).text_color(t.dim).child(text);

// A list of file rows (the changed files or a commit's).
function filesBody(v, t, focused, list) {
  const rows = list?.rows ?? [];
  if (!rows.length) return empty(t, v.error ?? (list ? "No changes" : v.loads.busy() ? "Loading changes…" : "No folder"));
  v.scroll.measure(padded(rows.map(rowSize)));
  const ctx = {
    t, selected: list.row()?.key, hover: v.hover, focused, tree: list.tree, inNvim: (row) => v.handedOff(row),
    onClick: (row, e, cx) => v.clickRow(row, e, cx),
    onHover: (row, h, cx) => v.hoverRow(row.key, h, cx),
    onAction: (op, row, cx) => { v.focusHere(cx); v.rowAction(op, row, cx); cx.notify(); },
  };
  return v.scroll.view(t, (i) => (i === 0 || i > rows.length ? div().h(6) : renderRow(rows[i - 1], ctx)));
}

export const MODES = {
  files: {
    title: "Changes",
    list: (v) => v.work,
    // s/u/x, the commit row and following the repo act on it.
    actsOnWork: true,
    // `row`: show that one (a note's file, `line`: the note); else the diff
    // pane goes back from a commit's file to the list's.
    enter: (v, cx, { row, line } = {}) => (row ? v.select(row, cx, { line }) : v.reshowWork(cx)),
    header: (v, t, notes) => filesHeader(t, {
      status: v.work?.status,
      files: v.work?.files() ?? 0,
      ...v.work?.totals(),
      notes: notes.length,
      loading: v.loads.shown(),
      wide: v.width >= 340,
      tree: treeMode(),
      onNotes: (cx) => { v.setMode("notes", cx); cx.notify(); },
      onTree: (cx) => v.toggleTree(cx),
      onLog: (cx) => { openLog(v, cx); cx.notify(); },
    }),
    body: (v, t, focused) => filesBody(v, t, focused, v.work),
    under: (v, t) => (v.scope === "worktree" && v.work
      ? commitBox(t, v.commitInput, v.typing, v.work.status.sections.some((s) => s.id === "staged"), v.commitRunning(), (cx) => v.commit(cx), v.width < 320)
      : null),
    keys: [
      { caps: ["j", "k"], label: "move", on: { j: down, down, k: up, up } },
      { caps: ["s"], label: "stage", rank: 1, when: worktree, on: { s: (v, cx) => v.rowAction("stage", null, cx) } },
      { caps: ["u"], label: "unstage", rank: 2, when: worktree, on: { u: (v, cx) => v.rowAction("unstage", null, cx) } },
      { caps: ["enter"], label: enterLabel, rank: 3, on: { enter: (v, cx) => v.enter(cx), e: (v, cx) => v.enter(cx) } },
      { caps: ["x"], label: "discard", rank: 4, when: worktree, on: { x: (v, cx) => v.rowAction("discard", null, cx) } },
      { caps: ["c"], label: "commit", rank: 5, when: worktree, on: { c: (v) => worktree(v) && v.commitInput.focus() } },
      { caps: ["l"], label: "history", rank: 6, on: { l: (v, cx) => openLog(v, cx) } },
      { caps: ["t"], label: "tree", rank: 7, on: { t: (v, cx) => v.toggleTree(cx) } },
      { caps: ["n"], label: "notes", rank: 8, on: { n: back("notes") } },
      { caps: ["w"], label: "uncommitted / vs base", on: { w: (v, cx) => v.toggleScope(cx) } },
      { caps: ["h", "right"], label: "fold / unfold a folder", on: { h: (v, cx) => v.fold(true, cx), left: (v, cx) => v.fold(true, cx), right: (v, cx) => v.fold(false, cx) } },
      { caps: ["g", "G"], label: "first / last", on: { g: (v, cx) => v.edge(false, cx), G: (v, cx) => v.edge(true, cx) } },
    ],
  },
  commit: {
    title: "A commit's files",
    list: (v) => v.commitFiles,
    // `list`: the commit's files; the first one shows.
    enter: (v, cx, { list } = {}) => {
      if (!list) return;
      v.commitFiles = list;
      v.scroll.top = 0;
      const first = list.shownFile();
      if (first) v.select(first, cx);
    },
    header: (v, t) => {
      const c = v.commitFiles;
      const n = c?.files() ?? 0;
      return subHeader(t, { title: c?.where.short ?? "", detail: c?.where.subject, ...c?.totals(), pill: `${n} file${n === 1 ? "" : "s"}`, onBack: (cx) => { v.setMode("log", cx); cx.notify(); } });
    },
    body: (v, t, focused) => filesBody(v, t, focused, v.commitFiles),
    keys: [
      { caps: ["j", "k"], label: "move", on: { j: down, down, k: up, up } },
      { caps: ["enter"], label: enterLabel, rank: 1, on: { enter: (v, cx) => v.enter(cx), e: (v, cx) => v.enter(cx) } },
      { caps: ["t"], label: "tree", rank: 2, on: { t: (v, cx) => v.toggleTree(cx) } },
      { caps: ["escape"], label: "back", rank: 3, on: { escape: back("log"), h: back("log"), left: back("log") } },
    ],
  },
  log: {
    title: "History",
    list: () => null,
    // `history`: a fresh one (`l`); loaded when it has no commits yet.
    enter: (v, cx, { history } = {}) => {
      if (history) v.history = history;
      if (!v.history.commits.length) loadLog(v, cx);
    },
    header: (v, t) => subHeader(t, {
      title: "History",
      detail: v.history.filter ? `since ${v.work?.where.base?.label ?? "the base"}` : v.work?.status.branch ?? "",
      pill: v.history.filter ? "branch" : "all",
      onPill: (cx) => { toggleLogFilter(v, cx); cx.notify(); },
      loading: v.loads.shown(),
      onBack: (cx) => { v.setMode("files", cx); cx.notify(); },
    }),
    body: (v, t, focused) => {
      const commits = v.history.commits;
      // "Loading history…" only once it takes a while (the spinner's 250 ms).
      if (!commits.length) return empty(t, v.history.loading ? (v.loads.shown() ? "Loading history…" : "") : "No commits");
      v.scroll.measure(padded(commits.map(() => COMMIT_ROW)));
      const ctx = { t, selected: v.history.selected, focused, onClick: (i, cx) => { v.focusHere(cx); v.history.selected = i; openCommit(v, commits[i], cx); cx.notify(); } };
      return v.scroll.view(t, (i) => (i === 0 || i > commits.length ? div().h(6) : commitRow(commits[i - 1], i - 1, ctx)));
    },
    keys: [
      { caps: ["j", "k"], label: "move", on: { j: (v) => logMove(v, 1), down: (v) => logMove(v, 1), k: (v) => logMove(v, -1), up: (v) => logMove(v, -1) } },
      { caps: ["enter"], label: "files", rank: 1, on: { enter: (v, cx) => openCommit(v, v.history.current(), cx), right: (v, cx) => openCommit(v, v.history.current(), cx) } },
      { caps: ["w"], label: "branch / all", rank: 2, on: { w: (v, cx) => toggleLogFilter(v, cx) } },
      { caps: ["escape"], label: "back", rank: 3, on: { escape: back("files"), h: back("files"), left: back("files"), l: back("files") } },
    ],
  },
  notes: {
    title: "Notes",
    list: () => null,
    enter: (v) => { v.noteSel = Math.min(v.noteSel, Math.max(0, v.notes().length - 1)); },
    header: (v, t, notes) => subHeader(t, {
      title: "Notes",
      detail: v.work?.where.root?.split("/").pop(),
      pill: String(notes.length),
      onBack: (cx) => { v.setMode("files", cx); cx.notify(); },
      onSend: notes.length ? (cx) => { v.send(cx); cx.notify(); } : null,
    }),
    body: (v, t, focused, notes) => {
      if (!notes.length) return empty(t, "No notes. In the diff: select lines (v or drag), then a.");
      v.noteSel = Math.min(v.noteSel, notes.length - 1);
      v.scroll.measure(padded(notes.map(() => NOTE_ROW)));
      const ctx = {
        t, selected: v.noteSel, hover: v.hover, focused,
        onClick: (i, e, cx) => { v.focusHere(cx); v.noteSel = i; if (e.click_count >= 2) jumpToNote(v, notes[i], cx); cx.notify(); },
        onHover: (n, h, cx) => v.hoverRow(n.id, h, cx),
        onDelete: (n, cx) => { deleteNote(v, n); cx.notify(); },
      };
      return v.scroll.view(t, (i) => (i === 0 || i > notes.length ? div().h(6) : noteRow(notes[i - 1], i - 1, ctx)));
    },
    keys: [
      { caps: ["j", "k"], label: "move", on: { j: (v) => noteMove(v, 1), down: (v) => noteMove(v, 1), k: (v) => noteMove(v, -1), up: (v) => noteMove(v, -1) } },
      { caps: ["enter"], label: "go to it", rank: 1, on: { enter: (v, cx) => jumpToNote(v, v.notes()[v.noteSel], cx) } },
      { caps: ["d"], label: "delete", rank: 2, on: { d: (v) => deleteNote(v, v.notes()[v.noteSel]), x: (v) => deleteNote(v, v.notes()[v.noteSel]) } },
      { caps: ["escape"], label: "back", rank: 4, on: { escape: back("files"), h: back("files"), left: back("files"), n: back("files") } },
    ],
  },
};
