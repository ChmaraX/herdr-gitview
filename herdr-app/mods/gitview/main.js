// gitview for the Herdr app: the file list. It takes the place of the stock
// Changes panel (the layout's `changes` slot; it follows the shown space),
// and is the right half of cmd-g's gitview tab (props {cwd, on, space, tab,
// own}). Clicking a file (or j/k onto it) shows its diff in the tab's diff
// pane (mod gitview-diff), full height left of the panel, opened on first
// use; Enter opens nvim. The two talk only through shared state (shared.js).
//
// Views: the changed files (flat or as a tree, `t`), the history (`l`) and
// a commit's files, the notes (`n`). Herdr-gitview's engine does the git
// (`herdr-gitview json …`, `--on <machine>` for a remote space).
// Commands: `gitview.key:<key>`, `gitview.file:<path>`, `gitview.commit:<msg>`.

import { View, div } from "gpui-kit";
import { v_flex, InputState } from "gpui-base";
import { agents, focused, nextCommand, notify, openOverlay, panes, props, run, setShared, shared, theme, watchFolder } from "herdr";
import { gv, location, message, onArgs } from "./engine.js";
import { diffArgs, entryOf, fileOrder, isDark, listRows, parentDirs, sameFile, viewOf } from "./model.js";
import { diffPane, notesKey, notesOf, onScreen, pump, removeNotes, setView, takeKeyboard, viewIn } from "./shared.js";
import { openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import * as chrome from "./chrome.js";
import { handleKey, runCommand } from "./keys.js";
import { renderRow, rowSize } from "./listview.js";
import { COMMIT_ROW, commitRow } from "./history.js";
import { NOTE_ROW, noteRow } from "./notesview.js";

// Wide enough for the header's tree / history buttons.
const WIDE = 340;

export default class GitList extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.scroll = new Scroller("gitview-list");
    this.commitInput = InputState.new({ placeholder: "Commit message" });
    this.commitInput.on("change", (_e, cx) => cx.notify());
    this.commitInput.on("submit", (_e, cx) => this.commit(cx));
    this.commitInput.on("focus", (_e, cx) => { this.typing = true; cx.notify(); });
    this.commitInput.on("blur", (_e, cx) => { this.typing = false; cx.notify(); });
    this.width = 0;
    this.mode = "files"; // files | log | commit | notes
    this.scope = "worktree";
    this.status = null;
    this.rows = [];
    this.selected = null;
    this.collapsed = new Set();
    this.log = { commits: [], selected: 0, filter: false, loading: false };
    this.commitView = null; // {sha, short, subject, status, rows, selected}
    this.noteSel = 0;
    this.seen = { view: 0, dirty: shared("gitview.dirty")?.at ?? 0 };
    this.inflight = 0; // loads running; commands wait for them (see idle())
    this.waiters = [];
    // What render() notices (another repo or space, the pane changed the
    // repo) is refreshed from here.
    this.kick = pump(cx, (cx) => {
      if (this.context().key !== this.ctx?.key || this.dirty) {
        this.dirty = false;
        this.refresh(cx);
      }
    });
    this.refresh(cx);
    // Local repos: the folder watch, and every 10 s; remote ones: every 4 s.
    // Only while it's on screen.
    // What render() notices, also while the window doesn't draw (covered).
    cx.timer.every(500, (cx) => { if (this.look()) cx.notify(); });
    cx.timer.every(4000, (cx) => { if (this.ctx?.on && this.shown()) this.refresh(cx); });
    cx.timer.every(10000, (cx) => { if (!this.ctx?.on && this.shown()) this.refresh(cx); });
    // A message in the footer goes after a few seconds.
    cx.timer.every(2000, (cx) => {
      if (this.flashText && Date.now() - this.flashAt > 5000) {
        this.flashText = null;
        cx.notify();
      }
    });
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        try {
          await this.idle(cx); // a command acts on what the one before it loaded
          this.followPane(this.tab()); // the pane's J/K, before this draws again
          runCommand(this, command, cx);
        } catch (e) {
          console.error(`gitview: ${command}: ${message(e)}\n${e?.stack ?? ""}`);
          this.flash(message(e));
        }
        cx.notify();
      }
    });
    if (this.isPane()) this.takeKeyboard(cx);
  }

  isPane() {
    return Boolean(props()?.own || props()?.cwd);
  }

  shown() {
    return onScreen(this.isPane() ? props()?.tab : null);
  }

  // The tab whose diff pane this list drives.
  tab() {
    return props()?.tab ?? this.ctx?.tab ?? null;
  }

  takeKeyboard(cx) {
    takeKeyboard(this, cx);
  }

  // ---- data ---------------------------------------------------------------

  load(cx, work) {
    this.inflight++;
    cx.spawn(async (cx) => {
      try {
        await work(cx);
      } catch (e) {
        this.flash(message(e));
      } finally {
        if (--this.inflight === 0) this.waiters.splice(0).forEach((resolve) => resolve());
        cx.notify();
      }
    });
  }

  // Resolves once no load runs (at most 10 s: a lost task never blocks).
  idle(cx) {
    if (this.inflight === 0) return Promise.resolve();
    return Promise.race([new Promise((resolve) => this.waiters.push(resolve)), cx.sleep(10000)]);
  }

  context() {
    const p = props() ?? {};
    const loc = this.isPane() ? { cwd: p.cwd ?? null, on: p.on ?? null, space: p.space ?? null, tab: p.tab ?? null } : location(this.ctx);
    return { ...loc, key: JSON.stringify([loc.cwd, loc.on, this.scope]) };
  }

  where() {
    const s = this.status;
    return { root: s?.root, on: this.ctx?.on ?? null, base: s?.base ?? null, editor: s?.editor ?? null };
  }

  refresh(cx) {
    const ctx = this.context();
    if (ctx.key !== this.ctx?.key) this.ctx = ctx;
    else this.ctx = { ...this.ctx, tab: ctx.tab, space: ctx.space };
    if (this.loading && Date.now() - this.loadingSince < 30000) {
      this.again = true;
      return;
    }
    if (!ctx.cwd) {
      this.status = null;
      this.rows = [];
      this.error = "No folder";
      return;
    }
    this.loading = true;
    this.loadingSince = Date.now();
    this.load(cx, async (cx) => {
      try {
        const status = await gv(["status", "--repo", ctx.cwd, "--scope", this.scope, ...onArgs(ctx.on)]);
        if (ctx.key !== this.ctx.key) return;
        if (status.root !== this.status?.root || status.scope !== this.status?.scope) {
          this.selected = null;
          this.collapsed = new Set();
          if (this.mode !== "files" && status.root !== this.status?.root) this.mode = "files";
        }
        const previous = { key: this.selected, index: this.rows.findIndex((r) => r.key === this.selected) };
        this.status = status;
        this.error = null;
        this.rebuild();
        if (!ctx.on) this.watch(status.root, cx);
        this.keepSelection(previous);
        this.followSelection();
      } catch (e) {
        if (ctx.key === this.ctx.key) {
          this.status = null;
          this.rows = [];
          const m = message(e);
          this.error = m.includes("not inside a git repository") || m.includes("not a git repository") ? "Not a git repository" : m;
        }
      } finally {
        this.loading = false;
        if (this.again) {
          this.again = false;
          this.refresh(cx);
        }
      }
    });
  }

  tree() {
    return Boolean(shared("gitview.tree"));
  }

  rebuild() {
    this.rows = listRows(this.status, { tree: this.tree(), collapsed: this.collapsed });
    if (this.commitView) this.commitView.rows = listRows(this.commitView.status, { tree: this.tree(), collapsed: this.collapsed });
  }

  // Live refresh while the repo changes (a save, a commit, staging elsewhere).
  watch(root, cx) {
    if (this.watched === root) return;
    this.watched = root;
    cx.spawn(async (cx) => {
      try {
        while (this.watched === root) {
          await watchFolder(root);
          if (this.watched === root && this.shown()) this.refresh(cx);
        }
      } catch {
        this.watched = null;
      }
    });
  }

  // After a refresh: the same row, else the same file in its new section
  // (just staged), else whatever is now where it was.
  keepSelection(previous) {
    const rows = this.rows.filter((r) => r.kind !== "header");
    if (rows.some((r) => r.key === previous.key)) return;
    const path = previous.key?.slice(previous.key.indexOf(":") + 1);
    const moved = rows.find((r) => r.kind === "file" && r.file.path === path);
    const near = this.rows.slice(Math.max(0, previous.index)).find((r) => r.kind !== "header") ?? rows[rows.length - 1];
    this.selected = (moved ?? (previous.key ? near : null) ?? null)?.key ?? null;
  }

  // The diff pane shows the selected file: when the selection moved to
  // another section (staged), the pane follows it.
  followSelection() {
    const tab = this.tab();
    const view = viewIn(tab);
    const row = this.row();
    if (!view || !row || row.kind !== "file" || view.root !== this.status?.root || this.mode !== "files") return;
    const next = viewOf(entryOf(row), this.where());
    if (view.path === next.path && !sameFile(view, next)) this.show(row, { open: false });
  }

  // ---- the current list (files, or a commit's files) -----------------------

  list() {
    return this.mode === "commit" ? this.commitView?.rows ?? [] : this.rows;
  }

  selKey() {
    return this.mode === "commit" ? this.commitView?.selected ?? null : this.selected;
  }

  setSel(key) {
    if (this.mode === "commit") this.commitView.selected = key;
    else this.selected = key;
  }

  row() {
    return this.list().find((r) => r.key === this.selKey()) ?? null;
  }

  listWhere() {
    if (this.mode !== "commit") return this.where();
    const c = this.commitView;
    return { root: this.status?.root, on: this.ctx?.on ?? null, commit: c.sha, short: c.short, editor: this.status?.editor ?? null };
  }

  // Select a row; a file shows in the diff pane (`open`: open the pane
  // when the tab has none: a click or j/k, not a refresh).
  select(key, cx, { open = true } = {}) {
    this.setSel(key);
    const index = this.list().findIndex((r) => r.key === key);
    this.reveal(index, { margin: 28 });
    const row = this.row();
    if (row?.kind === "file") this.show(row, { open, cx });
  }

  show(row, { open, cx, line } = {}) {
    const tab = this.tab();
    if (!tab) return;
    const order = fileOrder(this.mode === "commit" ? this.commitView.status : this.status, this.tree());
    setView(tab, { ...viewOf(entryOf(row), this.listWhere()), by: "list", files: order.slice(0, 2000).map(entryOf), line: line ?? null });
    this.seen.view = viewIn(tab)?.seq ?? 0;
    if (open && cx) this.ensurePane(tab, cx);
  }

  // The tab's diff pane, opened on first use: full height at the tab's
  // right edge (`json sidebar`), left of this panel.
  ensurePane(tab, cx) {
    if (diffPane(tab) || this.opening === tab || this.isPane()) return;
    this.opening = tab;
    cx.spawn(async (cx) => {
      try {
        try {
          await gv(["sidebar", "--tab", tab, "--mod", "gitview-diff", "--props", JSON.stringify({ tab })]);
        } catch (e) {
          // The engine finds the tab's root only in this machine's layout: a
          // remote space's tab gets the pane right of its focused pane.
          if (!tab.includes("/")) throw e;
          const f = focused();
          const pane = (f?.tab === tab && f.pane) || panes().filter((p) => p.tab === tab).pop()?.pane;
          if (!pane) throw e;
          run("pane.split", { direction: "right", pane, mod: "gitview-diff", props: { tab } });
        }
        this.takeKeyboard(cx); // the new pane took the app's focus; the list keeps the keys
      } catch (e) {
        this.flash(message(e));
      } finally {
        this.opening = null;
        cx.notify();
      }
    });
  }

  selectPath(path, cx) {
    if (this.mode !== "files") this.mode = "files";
    const row = this.rows.find((r) => r.kind === "file" && r.file.path === path)
      ?? (this.tree() && this.expandTo(path));
    if (row) this.select(row.key, cx);
    else this.flash(`${path} has no changes`);
  }

  // A row by what a script names: a file path, "folder/", "section:<id>".
  findRow(what) {
    const list = this.list();
    if (what.startsWith("section:")) return list.find((r) => r.kind === "header" && r.section.id === what.slice(8)) ?? null;
    if (what.endsWith("/")) return list.find((r) => r.kind === "dir" && r.path === what) ?? null;
    return list.find((r) => r.kind === "file" && r.file.path === what) ?? null;
  }

  // Unfold the folders above a file (tree mode); returns its row.
  expandTo(path, section) {
    const all = listRows(this.status, { tree: true });
    const row = all.find((r) => r.kind === "file" && r.file.path === path && (!section || r.section.id === section));
    if (!row) return null;
    for (const d of parentDirs(row)) this.collapsed.delete(d);
    this.rebuild();
    return this.rows.find((r) => r.key === row.key) ?? null;
  }

  move(delta, cx) {
    const rows = this.list().filter((r) => r.kind !== "header");
    if (!rows.length) return;
    const at = rows.findIndex((r) => r.key === this.selKey());
    const next = rows[at < 0 ? (delta > 0 ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, at + delta))];
    this.select(next.key, cx);
  }

  edge(last, cx) {
    const rows = this.list().filter((r) => r.kind !== "header");
    if (rows.length) this.select(rows[last ? rows.length - 1 : 0].key, cx);
  }

  // Keep list item i in view (the list starts with 6 px of padding).
  reveal(i, how) {
    if (i >= 0) this.scroll.reveal(i + 1, how);
  }

  flash(text) {
    this.flashText = text;
    this.flashAt = Date.now();
  }

  // The diff pane moved to another file (J/K there): select it here.
  followPane(tab) {
    const view = viewIn(tab);
    if (!view || view.seq <= this.seen.view) return;
    this.seen.view = view.seq;
    if (view.by !== "pane" || view.root !== this.status?.root) return;
    if (view.commit && this.mode === "commit" && this.commitView?.sha === view.commit) {
      const row = this.commitView.rows.find((r) => r.kind === "file" && r.file.path === view.path);
      if (row) { this.commitView.selected = row.key; this.reveal(this.list().indexOf(row), { margin: 28 }); }
      return;
    }
    if (view.commit || this.mode !== "files") return;
    let row = this.rows.find((r) => r.kind === "file" && r.file.path === view.path && r.section.id === view.section);
    if (!row && this.tree()) row = this.expandTo(view.path, view.section);
    if (row) {
      this.selected = row.key;
      this.reveal(this.rows.indexOf(row), { margin: 28 });
    }
  }

  // ---- folders ----------------------------------------------------------------

  toggleTree(cx) {
    setShared("gitview.tree", !this.tree());
    this.collapsed = new Set();
    this.rebuild();
    if (!this.row()) {
      // A folder row selected in tree mode: the first file under it.
      const key = this.selKey();
      const path = key?.startsWith("d:") ? key.slice(key.indexOf(":", 2) + 1) : null;
      const f = path && this.list().find((r) => r.kind === "file" && r.file.path.startsWith(path));
      this.setSel(f?.key ?? null);
    }
    this.reveal(this.list().findIndex((r) => r.key === this.selKey()), { center: true });
    cx.notify();
  }

  // Fold (h, left) or unfold (right) the selected folder; on a file, fold
  // goes to its folder.
  fold(close, cx) {
    const row = this.row();
    if (!row) return;
    if (row.kind === "dir") {
      if (close && row.collapsed && row.depth > 0) return this.toParent(row, cx);
      if (close) this.collapsed.add(row.key);
      else this.collapsed.delete(row.key);
      this.rebuild();
    } else if (close && this.tree()) this.toParent(row, cx);
  }

  toParent(row, cx) {
    const list = this.list();
    const at = list.indexOf(row);
    for (let i = at - 1; i >= 0; i--) {
      if (list[i].kind === "dir" && list[i].depth < row.depth) return this.select(list[i].key, cx);
    }
  }

  // ---- actions --------------------------------------------------------------

  enter(cx) {
    const row = this.row();
    if (!row) return;
    if (row.kind === "dir") {
      if (row.collapsed) this.collapsed.delete(row.key);
      else this.collapsed.add(row.key);
      this.rebuild();
      return;
    }
    this.edit(row, cx);
  }

  // nvim at the cursor the diff pane has on this file, else its first change.
  edit(row, cx) {
    if (row.file.kind === "deleted") return this.flash(`${row.file.path} is deleted`);
    const tab = this.tab();
    const where = this.listWhere();
    const ctx = this.ctx;
    this.load(cx, async () => {
      const view = viewOf(entryOf(row), where);
      const cursor = (shared("gitview.cursor") ?? {})[tab];
      let line = cursor && sameFile(cursor, view) ? cursor.line : null;
      if (!line) line = (await gv(diffArgs(view, { dark: isDark(theme().background), unfold: false })))?.first_change ?? 1;
      await openNvim({ root: where.root, on: where.on, path: row.file.path, line, tab, space: ctx?.space, editor: where.editor, own: this.isPane() });
    });
  }

  // + − ↺ on a row, a folder or a section (null: the selected row).
  rowAction(op, row, cx) {
    row = row ?? this.row();
    if (!row || !this.status || this.mode !== "files" || this.scope !== "worktree") return;
    const section = row.section.id;
    const ok = { stage: ["changes", "conflicts"], unstage: ["staged"], discard: ["changes"] }[op];
    if (!ok.includes(section)) return this.flash(`can't ${op} in ${row.section.title}`);
    const paths = row.kind === "header" ? row.section.files.map((f) => f.path) : row.kind === "dir" ? [row.path] : [row.file.path];
    if (op === "discard") return this.discard(paths, section, row);
    this.act([op, ...paths], cx);
  }

  act(args, cx) {
    const root = this.status.root;
    const on = this.ctx?.on;
    this.load(cx, async (cx) => {
      try {
        await gv([args[0], "--repo", root, ...onArgs(on), ...args.slice(1)]);
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      }
      this.refresh(cx);
    });
  }

  discard(paths, section, row) {
    this.pendingDiscard = { root: this.status.root, on: this.ctx?.on, paths, section };
    const what = row.kind === "header" ? `All ${row.section.files.length} changed files` : row.kind === "dir" ? `Everything changed in ${row.path}` : row.file.path;
    openOverlay({
      mod: "confirm",
      kind: "dialog",
      modal: true,
      width: 420,
      height: 190,
      props: { title: "Discard changes?", detail: `${what}: this can't be undone.`, command: { name: "gitview.discard" } },
    });
  }

  discardConfirmed(cx) {
    const p = this.pendingDiscard;
    this.pendingDiscard = null;
    if (!p || p.root !== this.status?.root) return;
    this.act(["discard", "--section", p.section, ...p.paths], cx);
  }

  // Staged changes, or everything when nothing is staged (as the stock
  // Changes panel commits).
  staged() {
    return Boolean(this.status?.sections.some((s) => s.id === "staged"));
  }

  commit(cx) {
    const text = this.commitInput.value().trim();
    if (!text || !this.status || this.scope !== "worktree" || this.committing) return;
    const all = !this.staged();
    this.committing = true;
    const root = this.status.root;
    const on = this.ctx?.on;
    this.load(cx, async (cx) => {
      try {
        const r = await gv(["commit", "--repo", root, ...onArgs(on), ...(all ? ["--all"] : []), text]);
        this.commitInput.set_value("");
        this.handle.focus();
        notify({ title: "Committed", body: r.summary ?? text, level: "success" });
      } catch (e) {
        notify({ title: "Commit failed", body: message(e), level: "error" });
      } finally {
        this.committing = false;
      }
      this.refresh(cx);
    });
  }

  toggleScope(cx) {
    this.scope = this.scope === "worktree" ? "branch" : "worktree";
    this.mode = "files";
    this.refresh(cx);
  }

  setMode(mode, cx) {
    this.mode = mode;
    if (mode === "files") this.reveal(this.rows.findIndex((r) => r.key === this.selected), { center: true });
    if (mode === "log" && !this.log.commits.length) this.loadLog(cx);
    if (mode === "notes") this.noteSel = Math.min(this.noteSel, Math.max(0, this.notes().length - 1));
  }

  // ---- history ----------------------------------------------------------------

  openLog(cx) {
    this.log = { commits: [], selected: 0, filter: this.scope === "branch", loading: false };
    this.setMode("log", cx);
  }

  toggleLogFilter(cx) {
    this.log.filter = !this.log.filter;
    this.log.selected = 0;
    this.loadLog(cx);
  }

  loadLog(cx) {
    if (!this.status) return;
    const root = this.status.root;
    const on = this.ctx?.on;
    const log = this.log;
    log.loading = true;
    this.load(cx, async () => {
      try {
        const args = ["log", "--repo", root, "--limit", "200", ...onArgs(on)];
        if (log.filter) {
          let base = this.status.base?.merge_base;
          if (!base) base = (await gv(["status", "--repo", root, "--scope", "branch", ...onArgs(on)])).base?.merge_base;
          if (base) args.push("--merge-base", base);
          else log.filter = false;
        }
        const r = await gv(args);
        log.commits = r.commits ?? [];
        log.selected = Math.min(log.selected, Math.max(0, log.commits.length - 1));
      } finally {
        log.loading = false;
      }
    });
  }

  logMove(delta) {
    const n = this.log.commits.length;
    if (!n) return;
    this.log.selected = Math.min(n - 1, Math.max(0, this.log.selected + delta));
    this.reveal(this.log.selected, { margin: COMMIT_ROW });
  }

  openCommit(c, cx) {
    if (!c) return;
    const root = this.status.root;
    const on = this.ctx?.on;
    this.load(cx, async () => {
      const r = await gv(["commit-files", "--repo", root, "--sha", c.sha, ...onArgs(on)]);
      const status = { sections: [{ id: "commit", title: "", cached: false, files: r.files ?? [] }] };
      const rows = listRows(status, { tree: this.tree(), collapsed: this.collapsed });
      this.commitView = { sha: c.sha, short: c.short, subject: c.subject, status, rows, selected: rows.find((x) => x.kind === "file")?.key ?? null };
      this.mode = "commit";
      this.scroll.top = 0;
      const first = this.row();
      if (first) this.show(first, { open: true, cx });
    });
  }

  // ---- notes ----------------------------------------------------------------

  notesKey() {
    return this.status ? notesKey(this.ctx?.on, this.status.root) : null;
  }

  notes() {
    const key = this.notesKey();
    return key ? notesOf(key) : [];
  }

  noteMove(delta) {
    const n = this.notes().length;
    if (!n) return;
    this.noteSel = Math.min(n - 1, Math.max(0, this.noteSel + delta));
    this.reveal(this.noteSel, { margin: 8 });
  }

  deleteNote(note) {
    if (note) removeNotes(this.notesKey(), [note.id]);
  }

  // The note's file in the diff pane, its cursor on the note.
  jumpToNote(note, cx) {
    if (!note) return;
    this.mode = "files";
    let row = this.rows.find((r) => r.kind === "file" && r.file.path === note.file && !r.section.cached)
      ?? this.rows.find((r) => r.kind === "file" && r.file.path === note.file);
    if (!row && this.tree()) row = this.expandTo(note.file);
    if (!row) return this.flash(`${note.file} has no changes now`);
    this.setSel(row.key);
    this.reveal(this.rows.indexOf(row), { center: true });
    this.show(row, { open: true, cx, line: note.end || note.start || null });
  }

  send(cx) {
    const notes = this.notes();
    if (!notes.length) return this.flash("no notes yet: select lines in the diff (v or drag), then a");
    const space = this.ctx?.space;
    const key = this.notesKey();
    this.load(cx, async () => {
      const { message: text } = await gv(["notes", JSON.stringify(notes.map(({ id, ...note }) => note))]);
      const height = Math.min(420, 140 + 32 * Math.max(1, agents().length));
      openOverlay({ mod: "gitview-agents", kind: "dialog", modal: true, width: 460, height, props: { message: text, count: notes.length, space, key, ids: notes.map((n) => n.id) } });
    });
  }

  // ---- keys & mouse -----------------------------------------------------------

  key(e, cx) {
    return handleKey(this, e, cx);
  }

  clickRow(row, e, cx) {
    this.handle.focus();
    if (row.kind === "header") return;
    if (row.kind === "dir") {
      this.setSel(row.key);
      if (row.collapsed) this.collapsed.delete(row.key);
      else this.collapsed.add(row.key);
      this.rebuild();
    } else if (e.click_count >= 2) this.edit(row, cx);
    else if (row.key !== this.selKey() || !viewIn(this.tab()) || !diffPane(this.tab())) this.select(row.key, cx);
    cx.notify();
  }

  hoverRow(key, hovered, cx) {
    if (hovered) this.hover = key;
    else if (this.hover === key) this.hover = null;
    else return;
    cx.notify();
  }

  // ---- view -----------------------------------------------------------------

  // Another repo or space, the pane changed the repo or moved (J/K): true
  // when there's something to draw.
  look() {
    const ctx = this.context();
    if (ctx.key !== this.ctx?.key) this.kick();
    else if (ctx.tab !== this.ctx.tab) this.ctx = { ...this.ctx, tab: ctx.tab, space: ctx.space };
    const dirty = shared("gitview.dirty");
    if (dirty && dirty.at > this.seen.dirty) {
      this.seen.dirty = dirty.at;
      if (dirty.root === this.status?.root) {
        this.dirty = true;
        this.kick();
      }
    }
    const seen = this.seen.view;
    this.followPane(this.tab());
    return this.seen.view !== seen;
  }

  render(cx) {
    this.look();
    const t = theme();
    const focused = this.handle.is_focused();
    const notes = this.notes();
    return v_flex()
      .id("gitview")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .text_color(t.foreground)
      .text_size(13)
      .when(!this.isPane(), (e) => e.rounded(10).overflow_hidden().border_1().border_color(focused ? t.accent : t.border))
      .track_focus(this.handle)
      .on_mouse_down("left", (_e, cx) => { this.handle.focus(); cx.notify(); })
      .on_size_change((size, cx) => {
        if (size.width === this.width) return;
        this.width = size.width;
        cx.notify();
      })
      .on_key_down((e, cx) => {
        if (this.typing) {
          if (e.key !== "escape") return;
          this.handle.focus(); // esc leaves the commit message
        } else if (!this.key(e, cx)) return;
        this.flashText = null;
        cx.stop_propagation();
        cx.notify();
      })
      .child(this.header(t, notes))
      .child(this.body(t, focused, notes))
      .when(this.mode === "files" && this.scope === "worktree" && Boolean(this.status), (e) =>
        e.child(chrome.commitBox(t, this.commitInput, this.typing, this.staged(), this.committing, (cx) => this.commit(cx))),
      )
      .child(chrome.footer(t, this.flashText, this.mode));
  }

  header(t, notes) {
    const back = (to) => (cx) => { this.setMode(to, cx); cx.notify(); };
    if (this.mode === "log") {
      return chrome.subHeader(t, {
        title: "History",
        detail: this.log.filter ? `since ${this.status?.base?.label ?? "the base"}` : this.status?.branch ?? "",
        pill: this.log.filter ? "branch" : "all",
        onPill: (cx) => { this.toggleLogFilter(cx); cx.notify(); },
        loading: this.log.loading,
        onBack: back("files"),
      });
    }
    if (this.mode === "commit") {
      const n = this.commitView?.status.sections[0].files.length ?? 0;
      return chrome.subHeader(t, { title: this.commitView?.short ?? "", detail: this.commitView?.subject, pill: `${n} file${n === 1 ? "" : "s"}`, onBack: back("log") });
    }
    if (this.mode === "notes") {
      return chrome.subHeader(t, { title: "Notes", detail: this.status?.root?.split("/").pop(), pill: String(notes.length), onBack: back("files"), onSend: notes.length ? (cx) => { this.send(cx); cx.notify(); } : null });
    }
    return chrome.filesHeader(t, {
      status: this.status,
      files: this.status?.sections.reduce((n, s) => n + s.files.length, 0) ?? 0,
      notes: notes.length,
      loading: this.loading || this.inflight > 0,
      wide: this.width >= WIDE,
      tree: this.tree(),
      onNotes: (cx) => { this.setMode("notes", cx); cx.notify(); },
      onTree: (cx) => this.toggleTree(cx),
      onLog: (cx) => { this.openLog(cx); cx.notify(); },
    });
  }

  empty(t, text) {
    return div().px(16).py(12).text_color(t.dim).child(text);
  }

  body(t, focused, notes) {
    const pad = (sizes) => [6, ...sizes, 6]; // 6 px above and below, like the stock panel
    if (this.mode === "log") {
      const commits = this.log.commits;
      if (!commits.length) return this.empty(t, this.log.loading ? "Loading history…" : "No commits");
      this.scroll.measure(pad(commits.map(() => COMMIT_ROW)));
      const ctx = { t, selected: this.log.selected, focused, onClick: (i, e, cx) => this.clickCommit(i, e, cx) };
      return this.scroll.view(t, (i) => (i === 0 || i > commits.length ? div().h(6) : commitRow(commits[i - 1], i - 1, ctx)));
    }
    if (this.mode === "notes") {
      if (!notes.length) return this.empty(t, "No notes. In the diff: select lines (v or drag), then a.");
      this.noteSel = Math.min(this.noteSel, notes.length - 1);
      this.scroll.measure(pad(notes.map(() => NOTE_ROW)));
      const ctx = {
        t, selected: this.noteSel, hover: this.hover, focused,
        onClick: (i, e, cx) => { this.handle.focus(); this.noteSel = i; if (e.click_count >= 2) this.jumpToNote(notes[i], cx); cx.notify(); },
        onHover: (n, h, cx) => this.hoverRow(n.id, h, cx),
        onDelete: (n, cx) => { this.deleteNote(n); cx.notify(); },
      };
      return this.scroll.view(t, (i) => (i === 0 || i > notes.length ? div().h(6) : noteRow(notes[i - 1], i - 1, ctx)));
    }
    const rows = this.list();
    if (!rows.length) return this.empty(t, this.error ?? (this.loading ? "Loading changes…" : this.status ? "No changes" : "No folder"));
    this.scroll.measure(pad(rows.map(rowSize)));
    const ctx = {
      t, selected: this.selKey(), hover: this.hover, focused, tree: this.tree(),
      onClick: (row, e, cx) => this.clickRow(row, e, cx),
      onHover: (row, h, cx) => this.hoverRow(row.key, h, cx),
      onAction: (op, row, cx) => { this.handle.focus(); this.rowAction(op, row, cx); cx.notify(); },
    };
    return this.scroll.view(t, (i) => (i === 0 || i > rows.length ? div().h(6) : renderRow(rows[i - 1], ctx)));
  }

  clickCommit(i, e, cx) {
    this.handle.focus();
    this.log.selected = i;
    if (e.click_count >= 1) this.openCommit(this.log.commits[i], cx);
    cx.notify();
  }
}
