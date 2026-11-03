// gitview for the Herdr app: the file list. It takes the place of the stock
// Changes panel (the layout's `changes` slot; it follows the shown space),
// and is the right half of cmd-g's gitview tab (props {cwd, on, space, tab,
// own}). Clicking a file (or j/k onto it) shows its diff in the tab's diff
// pane (mod gitview-diff), full height left of the panel, opened on first
// use; Enter opens nvim. The two talk only through shared state (shared.js).
//
// Views (modes.js): the changed files (flat or as a tree, `t`), the history
// (`l`) and a commit's files, the notes (`n`). Herdr-gitview's engine does
// the git (`herdr-gitview json …`, `--on <machine>` for a remote space).
// Commands: `gitview.key:<key>`, `gitview.file:<path>`, `gitview.commit:<msg>`.

import { View } from "gpui-kit";
import { v_flex, InputState } from "gpui-base";
import { notify, props, theme } from "herdr";
import { gv, location, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import {
  cursorIn, diffPane, dirty, liveView, notesKey, notesOf, onScreen, panelReopens, pump, setPanelReopens, setTreeMode, setView, takeKeyboard,
  treeMode, viewIn,
} from "./shared.js";
import { Flash, LOST_MS, Loads, commandLoop, watch } from "./modkit.js";
import { FileList } from "./filelist.js";
import { History, loadLog } from "./history.js";
import { openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import { footer } from "./chrome.js";
import { handleKey, runCommand } from "./keys.js";
import { MODES } from "./modes.js";

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
    this.mode = "files";
    this.scope = "worktree";
    this.work = null; // FileList: the changed files
    this.commitFiles = null; // FileList: a commit's files (history)
    this.history = new History(false);
    this.noteSel = 0;
    this.seen = { view: 0, dirty: dirty()?.at ?? 0 };
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    this.watcher = {};
    // What render notices (another repo or space, the pane changed the
    // repo) is refreshed from a task of this copy's own.
    this.kick = pump(cx, (cx) => this.notice(cx));
    this.refresh(cx);
    // Local repos: the folder watch, and every 10 s; remote ones: every 4 s.
    // Only while it's on screen.
    cx.timer.every(4000, (cx) => { if (this.ctx?.on && this.shown()) this.refresh(cx, { poll: true }); });
    cx.timer.every(10000, (cx) => { if (!this.ctx?.on && this.shown()) this.refresh(cx, { poll: true }); });
    commandLoop(cx, { loads: this.loads, flash: this.flash, name: "gitview", before: (cx) => this.notice(cx), handle: (c, cx) => runCommand(this, c, cx) });
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

  tree() {
    return treeMode();
  }

  // The file list the shown view acts on (none in history and notes).
  list() {
    return MODES[this.mode].list(this);
  }

  // ---- data ---------------------------------------------------------------

  context() {
    const p = props() ?? {};
    const loc = this.isPane() ? { cwd: p.cwd ?? null, on: p.on ?? null, space: p.space ?? null, tab: p.tab ?? null } : location(this.ctx);
    return { ...loc, key: JSON.stringify([loc.cwd, loc.on, this.scope]) };
  }

  // Another repo or space, the pane changed the repo or moved (J/K).
  // Before every command, and from the pump after a draw noticed it.
  notice(cx) {
    const ctx = this.context();
    const d = dirty();
    const repoChanged = Boolean(d && d.at > this.seen.dirty && d.root === this.work?.where.root);
    if (d) this.seen.dirty = Math.max(this.seen.dirty, d.at);
    if (ctx.key !== this.ctx?.key || repoChanged) this.refresh(cx);
    else if (ctx.tab !== this.ctx.tab) this.ctx = { ...this.ctx, tab: ctx.tab, space: ctx.space };
    this.followPane();
    this.openOnShow(cx);
  }

  // The panel just showed (cmd-shift-g, the Changes button) after its diff
  // pane closed with it, or for the first time: the selected file, else the
  // first one, opens in the diff pane, as the TUI's toggle did.
  openOnShow(cx) {
    if (!this.wantOpen || this.loading || this.isPane()) return;
    if (!this.shown()) {
      this.wantOpen = false; // hidden again meanwhile
      return;
    }
    if (!this.work && !this.error) return; // not loaded yet: refresh calls again
    this.wantOpen = false;
    setPanelReopens(false);
    if (this.mode !== "files" || !this.work) return;
    const selected = this.work.row();
    const row = selected?.kind === "file" ? selected : this.work.rows.find((r) => r.kind === "file");
    if (row) this.select(row.key, cx);
  }

  // `poll`: a timer's look; in branch scope the base found last time is
  // kept (finding it is a dozen git calls).
  refresh(cx, { poll = false } = {}) {
    const ctx = this.context();
    this.ctx = ctx;
    if (this.loading && Date.now() - this.loadingSince < LOST_MS) {
      this.again = true;
      return;
    }
    if (!ctx.cwd) {
      this.work = null;
      this.error = "No folder";
      return;
    }
    this.loading = true;
    this.loadingSince = Date.now();
    this.loads.run(cx, async (cx) => {
      try {
        const base = poll && this.scope === "branch" && this.loadedKey === ctx.key ? this.work?.where.base : null;
        const known = base ? ["--base", base.label, "--merge-base", base.merge_base] : [];
        const status = await gv(["status", "--repo", ctx.cwd, "--scope", this.scope, ...known, ...onArgs(ctx.on)]);
        if (ctx.key !== this.ctx.key) return;
        const where = { root: status.root, on: ctx.on ?? null, base: status.base ?? null, editor: status.editor ?? null };
        const same = this.work && this.work.where.root === where.root && this.work.where.on === where.on && this.work.status.scope === status.scope;
        if (same) this.work.update(status, where);
        else {
          this.work = new FileList(status, where);
          this.work.rebuild(this.tree());
          if (this.mode !== "files") this.mode = "files";
          this.commitFiles = null;
        }
        this.error = null;
        this.loadedKey = ctx.key;
        if (!ctx.on) watch(this.watcher, status.root, cx, (cx) => { if (this.shown()) this.refresh(cx); });
        this.followSelection();
      } catch (e) {
        if (ctx.key === this.ctx.key) {
          this.work = null;
          const m = message(e);
          this.error = /not (inside )?a git repository/.test(m) ? "Not a git repository" : m;
        }
      } finally {
        this.loading = false;
        if (this.again) {
          this.again = false;
          this.refresh(cx);
        } else this.openOnShow(cx);
      }
    });
  }

  // The selection moved to another section (just staged): the pane follows.
  followSelection() {
    const view = liveView(viewIn(this.tab()));
    const row = this.work?.row();
    if (!view || row?.kind !== "file" || view.root !== this.work.where.root || this.mode !== "files") return;
    const next = this.work.viewFor(row);
    if (view.path === next.path && !sameFile(view, next)) this.show(row, { open: false });
  }

  // The diff pane moved to another file (J/K there, or s/u/x): select it.
  followPane() {
    const view = viewIn(this.tab());
    if (!view || view.seq <= this.seen.view) return;
    this.seen.view = view.seq;
    const list = this.list();
    if (view.by !== "pane" || !liveView(view) || !list || view.root !== list.where.root || (view.commit ?? null) !== (list.where.commit ?? null)) return;
    const row = list.findFile(view.path, view.section);
    if (row) {
      list.selected = row.key;
      this.reveal(list.index(row.key), { margin: 28 });
    }
  }

  // ---- selecting ----------------------------------------------------------------

  // Select a row; a file shows in the diff pane (`open`: open the pane when
  // the tab has none: a click or j/k, not a refresh).
  select(key, cx, { open = true } = {}) {
    const list = this.list();
    if (!list) return;
    list.selected = key;
    this.reveal(list.index(key), { margin: 28 });
    const row = list.row();
    if (row?.kind === "file") this.show(row, { open, cx });
  }

  show(row, { open, cx, line } = {}) {
    const tab = this.tab();
    if (!tab) return;
    setView(tab, this.list().viewFor(row, line ?? null));
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
        await gv(["sidebar", "--tab", tab, "--mod", "gitview-diff", "--props", JSON.stringify({ tab })]);
        this.takeKeyboard(cx); // the new pane took the app's focus; the list keeps the keys
      } catch (e) {
        this.flash.set(message(e));
      } finally {
        this.opening = null;
        cx.notify();
      }
    });
  }

  selectPath(path, cx) {
    this.mode = "files";
    const row = this.work?.findFile(path);
    if (row) this.select(row.key, cx);
    else this.flash.set(`${path} has no changes`);
  }

  move(delta, cx) {
    const row = this.list()?.step(delta);
    if (row) this.select(row.key, cx);
  }

  edge(last, cx) {
    const row = this.list()?.edge(last);
    if (row) this.select(row.key, cx);
  }

  // Keep list item i in view (the list starts with 6 px of padding).
  reveal(i, how) {
    if (i >= 0) this.scroll.reveal(i + 1, how);
  }

  toggleTree(cx) {
    setTreeMode(!this.tree());
    for (const list of [this.work, this.commitFiles].filter(Boolean)) {
      list.collapsed = new Set();
      list.rebuild(this.tree());
      if (!list.row() && list.selected?.startsWith("d:")) {
        // A folder row selected in tree mode: the first file under it.
        const dir = list.selected.slice(list.selected.indexOf(":", 2) + 1);
        list.selected = list.rows.find((r) => r.kind === "file" && r.file.path.startsWith(dir))?.key ?? null;
      }
    }
    if (this.list()) this.reveal(this.list().index(this.list().selected), { center: true });
    cx.notify();
  }

  // Fold (h, left) or unfold (right) the selected folder; on a file, fold
  // goes to its folder.
  fold(close, cx) {
    const list = this.list();
    const row = list?.row();
    if (!row) return;
    if (row.kind === "dir" && !(close && row.collapsed)) {
      if (close !== row.collapsed) list.toggleFold(row);
    } else if (close && list.tree) {
      const up = list.parent(row);
      if (up) this.select(up.key, cx);
    }
  }

  // ---- actions --------------------------------------------------------------

  enter(cx) {
    const list = this.list();
    const row = list?.row();
    if (row?.kind === "dir") list.toggleFold(row);
    else if (row) this.edit(row, cx);
  }

  // nvim at the cursor the diff pane has on this file, else its first change.
  edit(row, cx) {
    if (row.file.kind === "deleted") return this.flash.set(`${row.file.path} is deleted`);
    const tab = this.tab();
    const list = this.list();
    const space = this.ctx?.space;
    this.loads.run(cx, async () => {
      const view = list.viewFor(row);
      const cursor = cursorIn(tab);
      let line = cursor && sameFile(cursor, view) ? cursor.line : null;
      if (!line) line = (await gv(diffArgs(view, { dark: isDark(theme().background), unfold: false })))?.first_change ?? 1;
      await openNvim({ root: list.where.root, on: list.where.on, path: row.file.path, line, tab, space, editor: list.where.editor, own: this.isPane() });
    });
  }

  // + − ↺ on a row, a folder or a section (null: the selected row).
  rowAction(op, row, cx) {
    const list = this.work;
    row = row ?? list?.row();
    if (!row || this.mode !== "files" || this.scope !== "worktree") return;
    const section = row.section.id;
    if (!allowed(op, section)) return this.flash.set(`can't ${op} in ${row.section.title}`);
    const paths = row.kind === "header" ? row.section.files.map((f) => f.path) : row.kind === "dir" ? [row.path] : [row.file.path];
    if (op !== "discard") return this.act([op, "--section", section, "--", ...paths], list.where, cx);
    const what = row.kind === "header" ? `All ${row.section.files.length} changed files` : row.kind === "dir" ? `Everything changed in ${row.path}` : row.file.path;
    this.pendingDiscard = confirmDiscard("gitview", what, { where: list.where, paths, section });
  }

  discardConfirmed(command, cx) {
    const p = takeDiscard(this.pendingDiscard, command);
    if (!p) return;
    this.pendingDiscard = null;
    this.act(["discard", "--section", p.section, "--", ...p.paths], p.where, cx);
  }

  // One op on the repo the list came from, then a refresh.
  act(args, where, cx) {
    this.loads.run(cx, async (cx) => {
      try {
        await gv([args[0], "--repo", where.root, ...onArgs(where.on), ...args.slice(1)]);
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      }
      this.refresh(cx);
    });
  }

  // Staged changes, or everything when nothing is staged (as the stock
  // Changes panel commits).
  commit(cx) {
    const text = this.commitInput.value().trim();
    const work = this.work;
    if (!text || !work || this.scope !== "worktree" || this.committing) return;
    const all = !work.status.sections.some((s) => s.id === "staged");
    this.committing = true;
    this.loads.run(cx, async (cx) => {
      try {
        const r = await gv(["commit", "--repo", work.where.root, ...onArgs(work.where.on), ...(all ? ["--all"] : []), "--", text]);
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
    const leaving = this.mode;
    this.mode = mode;
    if (mode === "files" && leaving !== "files") this.reshowWork(cx);
    if (this.list()) this.reveal(this.list().index(this.list().selected), { center: true });
    if (mode === "log" && !this.history.commits.length) loadLog(this, cx);
    if (mode === "notes") this.noteSel = Math.min(this.noteSel, Math.max(0, this.notes().length - 1));
  }

  // Back from history (or the notes list): the diff pane showed a commit's
  // file; it shows the list's selected (else first) changed file again, or
  // nothing when there are no changes.
  reshowWork(cx) {
    const tab = this.tab();
    const view = liveView(viewIn(tab));
    if (!tab || !this.work || !view) return;
    const selected = this.work.row();
    const row = selected?.kind === "file" ? selected : this.work.rows.find((r) => r.kind === "file");
    if (!row) return setView(tab, null);
    const next = this.work.viewFor(row);
    if (!sameFile(view, next)) this.select(row.key, cx, { open: false });
  }

  // ---- notes ----------------------------------------------------------------

  notesKey() {
    return this.work ? notesKey(this.work.where.on, this.work.where.root) : null;
  }

  notes() {
    const key = this.notesKey();
    return key ? notesOf(key) : [];
  }

  send(cx) {
    const key = this.notesKey();
    const space = this.ctx?.space;
    this.loads.run(cx, async () => {
      if (!(await sendNotes(key, space))) this.flash.set("no notes yet: select lines in the diff (v or drag), then a");
    });
  }

  // ---- keys & mouse -----------------------------------------------------------

  key(e, cx) {
    return handleKey(this, e, cx);
  }

  clickRow(row, e, cx) {
    this.handle.focus();
    const list = this.list();
    if (!list || row.kind === "header") return;
    if (row.kind === "dir") {
      list.selected = row.key;
      list.toggleFold(row);
    } else if (e.click_count >= 2) this.edit(row, cx);
    else if (row.key !== list.selected || !liveView(viewIn(this.tab())) || !diffPane(this.tab())) this.select(row.key, cx);
    cx.notify();
  }

  hoverRow(key, hovered, cx) {
    if (hovered) this.hover = key;
    else if (this.hover === key) this.hover = null;
    else return;
    cx.notify();
  }

  // ---- view -----------------------------------------------------------------

  render(cx) {
    const ctx = this.context();
    const d = dirty();
    if (ctx.key !== this.ctx?.key || (d && d.at > this.seen.dirty)) this.kick();
    this.followPane();
    if (!this.isPane() && !this.wantOpen && panelReopens() && this.shown()) {
      this.wantOpen = true;
      this.kick();
    }
    const t = theme();
    const focused = this.handle.is_focused();
    const notes = this.notes();
    const mode = MODES[this.mode];
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
        this.flash.clear();
        cx.stop_propagation();
        cx.notify();
      })
      .child(mode.header(this, t, notes))
      .child(mode.body(this, t, focused, notes))
      .children([mode.under?.(this, t)].filter(Boolean))
      .child(footer(t, this.flash.text, mode.hints));
  }
}
