// gitview for the Herdr app: the file list. It takes the place of the stock
// Changes panel (the layout's `changes` slot; it follows the shown space),
// and is the right half of cmd-g's gitview tab (props {cwd, on, space, tab,
// own}); placement.js holds what differs. Clicking a file (or j/k onto it)
// shows its diff in the tab's diff pane (mod gitview-diff), full height
// left of the panel, opened on first use; Enter opens nvim. The two talk
// only through shared state (shared.js).
//
// Views (modes.js): the changed files (flat or as a tree, `t`), the history
// (`l`) and a commit's files, the notes (`n`). Herdr-gitview's engine does
// the git (`herdr-gitview json …`, `--on <machine>` for a remote space).
// Commands: `gitview.key:<key>`, `gitview.file:<path>`, `gitview.commit:<msg>`.

import { View } from "gpui-kit";
import { v_flex, InputState } from "gpui-base";
import { notify, theme } from "herdr";
import { gv, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import {
  cursorIn, diffPane, dirty, liveView, notesKey, notesOf, pump, setTreeMode, setView, takeKeyboard, treeMode, viewIn,
} from "./shared.js";
import { Flash, Loads, ModPanes, commandLoop, followStamp, stampMoved, watch } from "./modkit.js";
import { listPlacement } from "./placement.js";
import { FileList } from "./filelist.js";
import { History } from "./history.js";
import { openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import { footer } from "./chrome.js";
import { chipsFor, handleKey, runCommand, showKeys } from "./keys.js";
import { MODES } from "./modes.js";

// exec gives up on a program after 30 s: a commit that took that long may
// still be running (a slow hook) or have finished.
const EXEC_LIMIT_MS = 29000;

export default class GitList extends View {
  init(_props, cx) {
    this.place = listPlacement();
    this.cycle = this.place.cycle(); // the panel's show/hide (placement.js)
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
    this.seen = { views: {}, dirty: dirty()?.at ?? 0 };
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    this.modPanes = new ModPanes();
    this.watcher = {};
    // What render notices (another repo or space, the pane changed the
    // repo or moved, the panel showed) is acted on from a task of this
    // copy's own, and from a light timer: a hidden or covered copy isn't
    // drawn, but its timers run.
    this.kick = pump(cx, (cx) => this.notice(cx));
    cx.timer.every(300, (cx) => {
      if (!this.stale()) return;
      this.notice(cx);
      cx.notify();
    });
    this.refresh(cx);
    // Local repos: the folder watch, and every 10 s; remote ones: every 4 s.
    // Only while it's on screen.
    cx.timer.every(4000, (cx) => { if (this.ctx?.on && this.place.shown()) this.refresh(cx, { poll: true }); });
    cx.timer.every(10000, (cx) => { if (!this.ctx?.on && this.place.shown()) this.refresh(cx, { poll: true }); });
    commandLoop(cx, { loads: this.loads, flash: this.flash, name: "gitview", before: (cx) => this.notice(cx), handle: (c, cx) => runCommand(this, c, cx) });
    if (this.place.takesKeyboard) takeKeyboard(this, cx);
  }

  // The tab whose diff pane this list drives.
  tab() {
    return this.place.tab(this.ctx);
  }

  // The file list the shown view acts on (none in history and notes).
  list() {
    return MODES[this.mode].list(this);
  }

  // ---- data ---------------------------------------------------------------

  context() {
    const loc = this.place.context(this.ctx);
    return { ...loc, key: JSON.stringify([loc.cwd, loc.on, this.scope]) };
  }

  // Whether something notice() acts on changed (render and the timer ask;
  // no side effects).
  stale() {
    if (this.modPanes.changed()) return true;
    if (this.cycle && this.cycle.shown !== this.place.shown()) return true;
    if (!this.place.shown()) return false;
    if (stampMoved(this.watcher)) return true;
    const ctx = this.context();
    const d = dirty();
    const view = viewIn(this.tab());
    return ctx.key !== this.ctx?.key || ctx.tab !== this.ctx?.tab || Boolean(d && d.at > this.seen.dirty) || (view?.seq ?? 0) > (this.seen.views[this.tab()] ?? 0);
  }

  // The panel showed; another repo or space or tab; the pane changed the
  // repo or moved (J/K); a mod pane closed (Loads.revive). Before every command, and from the pump or the
  // timer after stale().
  notice(cx) {
    if (this.modPanes.gone()) this.loads.revive(cx);
    if (this.cycle?.look() === "shown") this.openOnShow(cx);
    if (!this.place.shown()) return;
    const ctx = this.context();
    const d = dirty();
    const repoChanged = Boolean(d && d.at > this.seen.dirty && d.root === this.work?.where.root);
    if (d) this.seen.dirty = Math.max(this.seen.dirty, d.at);
    const tabChanged = ctx.tab !== this.ctx?.tab;
    const stamped = followStamp(this.watcher); // another copy's watch saw the repo change
    if (ctx.key !== this.ctx?.key || repoChanged || stamped) this.refresh(cx);
    else if (tabChanged) this.ctx = { ...this.ctx, tab: ctx.tab, space: ctx.space };
    this.followPane({ adopt: tabChanged });
  }

  // The panel just showed (cmd-shift-g, the Changes button), and the tab
  // has no diff pane (hiding it closed them): from a fresh status, the
  // selected file, else the first one, opens there, as the TUI's toggle did.
  openOnShow(cx) {
    this.refresh(cx);
    const tab = this.tab();
    if (!tab || diffPane(tab)) return;
    cx.spawn(async (cx) => {
      await this.loads.idle(cx);
      const row = this.list()?.shownFile();
      if (row && this.place.shown() && this.tab() === tab && !diffPane(tab)) this.select(row, cx);
      cx.notify();
    });
  }

  // `poll`: a timer's look; in branch scope the base found last time is
  // passed back (finding it is a dozen git calls), with the HEAD it was
  // found at: the engine finds it again once HEAD moved. One status load at
  // a time (Loads.latest); a newer ask runs after it, and a reply that
  // isn't the newest is dropped.
  refresh(cx, { poll = false } = {}) {
    this.ctx = this.context();
    if (!this.ctx.cwd) {
      this.work = null;
      this.error = "No folder";
      return;
    }
    this.loads.latest(cx, "status", async (cx, current) => {
      const ctx = this.ctx;
      const old = this.work;
      const base = poll && this.scope === "branch" && this.loadedKey === ctx.key ? old?.where.base : null;
      const known = base ? ["--base", base.label, "--merge-base", base.merge_base, ...(old.status.head ? ["--head", old.status.head] : [])] : [];
      let status;
      try {
        status = await gv(["status", "--repo", ctx.cwd, "--scope", this.scope, ...known, ...onArgs(ctx.on)]);
      } catch (e) {
        if (current() && ctx.key === this.ctx.key) {
          this.work = null;
          const m = message(e);
          this.error = /not (inside )?a git repository/.test(m) ? "Not a git repository" : m;
        }
        return;
      }
      if (!current() || ctx.key !== this.ctx.key) return;
      const where = { root: status.root, on: ctx.on ?? null, base: status.base ?? null, editor: status.editor ?? null };
      const same = old && old.where.root === where.root && old.where.on === where.on && old.status.scope === status.scope;
      if (same) old.update(status, where);
      else {
        this.work = new FileList(status, where);
        this.work.rebuild(treeMode());
        this.commitFiles = null;
        this.setMode("files", cx);
      }
      this.error = null;
      this.loadedKey = ctx.key;
      if (!ctx.on) watch(this.watcher, status.root, cx, (cx) => { if (this.place.shown()) this.refresh(cx); });
      else this.watcher.root = null; // a local repo's watch ends
      this.followPane();
      this.followSelection();
    });
  }

  // After a new status: when the file the pane shows went from where it
  // was (staged, unstaged, discarded, committed: here, from the pane or
  // anywhere), the pane shows where the list's selection went
  // (FileList.update: the same file in its new section, else the file now
  // where it was), else nothing. A list with nothing selected takes the
  // pane's file as its selection.
  followSelection() {
    const tab = this.tab();
    const work = this.work;
    if (!tab || !work || !MODES[this.mode].actsOnWork) return;
    const view = liveView(viewIn(tab));
    if (!view || view.commit || view.root !== work.where.root || view.on !== work.where.on) return;
    const there = work.findFile(view.path, view.section);
    if (there) {
      if (!work.row()) work.select(there);
      return;
    }
    const next = work.shownFile();
    if (next) this.show(next);
    else this.seen.views[tab] = setView(tab, null).seq;
  }

  // The diff pane moved to another file (J/K there): select it. `adopt`:
  // the shown tab changed: select the file its pane shows, whoever wrote it.
  followPane({ adopt = false } = {}) {
    const tab = this.tab();
    const view = viewIn(tab);
    if (!view || (!adopt && view.seq <= (this.seen.views[tab] ?? 0))) return;
    this.seen.views[tab] = view.seq;
    const list = this.list();
    if ((!adopt && view.by !== "pane") || !liveView(view) || !list || view.root !== list.where.root || (view.commit ?? null) !== (list.where.commit ?? null)) return;
    const row = list.findFile(view.path, view.section);
    if (row) {
      list.select(row);
      this.reveal(list.index(row), { margin: 28 });
    }
  }

  // ---- selecting ----------------------------------------------------------------

  // Select a row; a file shows in the diff pane (`open`: open the pane when
  // the tab has none: a click or j/k, not a refresh; `line`: a note's).
  select(row, cx, { open = true, line = null } = {}) {
    const list = this.list();
    if (!list || !row) return;
    list.select(row);
    this.reveal(list.index(row), { margin: 28 });
    if (row.kind === "file") this.show(row, { open, cx, line });
  }

  show(row, { open = false, cx, line = null } = {}) {
    const tab = this.tab();
    if (!tab) return;
    this.seen.views[tab] = setView(tab, this.list().viewFor(row, line)).seq;
    if (open && cx && this.place.opensDiffPane) this.ensurePane(tab, cx);
  }

  // The tab's diff pane, opened on first use: full height at the tab's
  // right edge (`json sidebar`), left of this panel.
  ensurePane(tab, cx) {
    if (diffPane(tab) || this.opening === tab) return;
    this.opening = tab;
    cx.spawn(async (cx) => {
      try {
        await gv(["sidebar", "--tab", tab, "--mod", "gitview-diff", "--props", JSON.stringify({ tab })]);
        takeKeyboard(this, cx); // the new pane took the app's focus; the list keeps the keys
      } catch (e) {
        this.flash.set(message(e));
      } finally {
        this.opening = null;
        cx.notify();
      }
    });
  }

  selectPath(path, cx) {
    const row = this.work?.findFile(path);
    if (row) this.setMode("files", cx, { row });
    else this.flash.set(`${path} has no changes`);
  }

  move(delta, cx) {
    this.select(this.list()?.step(delta), cx);
  }

  edge(last, cx) {
    this.select(this.list()?.edge(last), cx);
  }

  // Keep list item i in view (the list starts with 6 px of padding).
  reveal(i, how) {
    if (i >= 0) this.scroll.reveal(i + 1, how);
  }

  toggleTree(cx) {
    const tree = !treeMode();
    setTreeMode(tree);
    for (const list of [this.work, this.commitFiles].filter(Boolean)) {
      list.collapsed = new Set();
      list.rebuild(tree);
      list.selectIntoFolder(); // a folder selected in tree mode: the first file under it
    }
    if (this.list()) this.reveal(this.list().selectedIndex(), { center: true });
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
      this.select(list.parent(row), cx);
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
    this.loads.run(cx, "nvim", async () => {
      const view = list.viewFor(row);
      const cursor = cursorIn(tab);
      let line = cursor && sameFile(cursor, view) ? cursor.line : null;
      if (!line) line = (await gv(diffArgs(view, { dark: isDark(theme().background), unfold: false })))?.first_change ?? 1;
      await openNvim({ root: list.where.root, on: list.where.on, path: row.file.path, line, tab, space, editor: list.where.editor, closeDiff: this.place.closesForHerdrNvim });
    });
  }

  // + − ↺ on a row, a folder or a section (null: the selected row). A
  // folder or a section sends the files it lists, so a discard confirmed
  // later never takes a file that showed up meanwhile.
  rowAction(op, row, cx) {
    const list = this.work;
    row = row ?? list?.row();
    if (!row || !MODES[this.mode].actsOnWork || this.scope !== "worktree") return;
    const section = row.section.id;
    if (!allowed(op, section)) return this.flash.set(`can't ${op} in ${row.section.title}`);
    const paths = list.filesOf(row);
    if (!paths.length) return;
    if (op !== "discard") return this.act([op, "--section", section, "--", ...paths], list.where, cx);
    const what = row.kind === "header" ? `All ${paths.length} changed files` : row.kind === "dir" ? `All ${paths.length} changed files in ${row.path}` : row.file.path;
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
    this.loads.run(cx, args[0], async (cx) => {
      try {
        await gv([args[0], "--repo", where.root, ...onArgs(where.on), ...args.slice(1)]);
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      }
      this.refresh(cx);
    });
  }

  // The engine decides at commit time (as the stock Changes panel commits):
  // the staged changes, or everything when nothing is staged; never while
  // files are in conflict. The button's label is the last status's guess.
  commit(cx) {
    const text = this.commitInput.value().trim();
    const work = this.work;
    if (!text || !work || this.scope !== "worktree" || this.committing) return;
    this.committing = true;
    const started = Date.now();
    this.loads.run(cx, "commit", async (cx) => {
      try {
        const r = await gv(["commit", "--repo", work.where.root, ...onArgs(work.where.on), "--", text]);
        this.commitInput.set_value("");
        this.handle.focus();
        notify({ title: "Committed", body: r.summary ?? text, level: "success" });
      } catch (e) {
        // The message stays in the box either way.
        if (Date.now() - started >= EXEC_LIMIT_MS) {
          notify({ title: "gitview", body: "the commit is still running or timed out: check git log", level: "warning" });
        } else notify({ title: "Commit failed", body: message(e), level: "error" });
      } finally {
        this.committing = false;
      }
      this.refresh(cx);
    });
  }

  toggleScope(cx) {
    this.scope = this.scope === "worktree" ? "branch" : "worktree";
    this.setMode("files", cx);
    this.refresh(cx);
  }

  // The one way the view changes: the mode's `enter` runs (modes.js) when
  // it changes, or when `how` says what to enter with.
  setMode(mode, cx, how = null) {
    const changed = mode !== this.mode;
    this.mode = mode;
    const list = this.list();
    if (list) this.reveal(list.selectedIndex(), { center: true });
    if (changed || how) MODES[mode].enter?.(this, cx, how ?? {});
  }

  // Back from history (or the notes list): the diff pane showed a commit's
  // file; it shows the list's selected (else first) changed file again, or
  // nothing when there are no changes.
  reshowWork(cx) {
    const tab = this.tab();
    const view = liveView(viewIn(tab));
    if (!tab || !this.work || !view) return;
    const row = this.work.shownFile();
    if (!row) this.seen.views[tab] = setView(tab, null).seq;
    else if (!sameFile(view, this.work.viewFor(row))) this.select(row, cx, { open: false });
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
    this.loads.run(cx, "send", async () => {
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
      list.select(row);
      list.toggleFold(row);
    } else if (e.click_count >= 2) this.edit(row, cx);
    else if (!list.isSelected(row) || !liveView(viewIn(this.tab())) || !diffPane(this.tab())) this.select(row, cx);
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
    if (this.stale()) this.kick();
    const t = theme();
    const focused = this.handle.is_focused();
    const notes = this.notes();
    const mode = MODES[this.mode];
    return this.place
      .frame(v_flex().id("gitview").size_full().min_h(0).bg(t.background).text_color(t.foreground).text_size(13), t, focused)
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
      .child(footer(t, { flash: this.flash.text, chips: chipsFor(this), width: this.width, onKeys: () => showKeys(this) }));
  }
}
