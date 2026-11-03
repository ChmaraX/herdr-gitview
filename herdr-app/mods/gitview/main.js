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
import { notify, run, shared, theme } from "herdr";
import { endTerminal, gv, labelled, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import {
  answer, askIn, cursorIn, diffPane, panelState, handoffIn, setIgnoredDirs, handoffs, herdrNvimPanes, liveView, notesKey, notesOf, setHandoff, setTreeMode, setView,
  treeMode, viewIn,
} from "./shared.js";
import { Flash, Loads, ModPanes, commandLoop, debounced, debugLook, followStamp, stampMoved, trace, watch } from "./modkit.js";
import { listPlacement } from "./placement.js";
import { askToClose, editorState, liveEditor, quitEditor, setEditor } from "./editors.js";
import { FileList } from "./filelist.js";
import { History } from "./history.js";
import { knowEditor, openNvim } from "./nvim.js";
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
    this.commitInput.on("focus", (_e, cx) => {
      this.typing = true;
      // A click into the message: the app's focus comes here too, then the
      // message gets the keys again.
      if (!this.viaApp) this.focusList(cx, { then: "input" });
      cx.notify();
    });
    this.commitInput.on("blur", (_e, cx) => { this.typing = false; cx.notify(); });
    this.width = 0;
    this.mode = "files";
    this.scope = "worktree";
    this.work = null; // FileList: the changed files
    this.commitFiles = null; // FileList: a commit's files (history)
    this.history = new History(false);
    this.noteSel = 0;
    this.seen = { asks: {} };
    this.roots = new Map(); // "<machine>:<folder>" → its repo's root
    this.tops = {}; // each view's scroll (setMode)
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    this.modPanes = new ModPanes();
    this.watcher = {};
    // What render notices (another repo or space, the pane changed the
    // repo or moved, the panel showed) is acted on from a task of this
    // copy's own, and from a light timer: a hidden or covered copy isn't
    // drawn, but its timers run.
    this.kick = debounced(cx, 0, (cx) => this.notice(cx));
    cx.timer.every(300, (cx) => {
      if (!this.stale()) return;
      this.notice(cx);
      cx.notify();
    });
    this.refresh(cx);
    // Local repos: the folder watch (and a poll after 30 s without a
    // status); remote ones: every 4 s. Only while it's on screen.
    cx.timer.every(4000, (cx) => { if (this.ctx?.on && this.place.shown()) this.refresh(cx, { poll: true, why: "poll" }); });
    // The watch shows a local repo's changes; its poll is a backstop, only
    // after 30 s without a status.
    cx.timer.every(10000, (cx) => { if (!this.ctx?.on && this.place.shown() && Date.now() - (this.statusAt ?? 0) > 30000) this.refresh(cx, { poll: true, why: "poll" }); });
    // As a render would: only while it's on screen (a hidden copy isn't
    // drawn; its 300 ms look is all it has then).
    debugLook(cx, () => { if (this.place.shown() && this.stale()) this.kick(); trace("list", () => this.traceFields(), this); });
    commandLoop(cx, { loads: this.loads, flash: this.flash, name: "gitview", before: (cx) => this.notice(cx), handle: (c, cx) => runCommand(this, c, cx) });
    if (this.place.takesKeyboard) this.focusList(cx);
  }

  // The keyboard and the app's focus, together (placement.js focusSelf).
  focusList(cx, { then = null } = {}) {
    this.viaApp = false;
    this.afterFocus = then;
    this.place.focusSelf(this, cx);
  }

  // The keys here: within the list when the app's focus is already on it
  // (the message box back to the rows), else through the app.
  focusHere(cx) {
    if (this.viaApp) this.handle.focus();
    else this.focusList(cx);
  }

  // Whether the list shows itself focused: it has the keys, and the app's
  // focus came with them (else the app's ring is on another pane).
  showsFocus() {
    const has = this.handle.is_focused() || this.typing;
    if (!has) this.viaApp = false;
    return has && Boolean(this.viaApp);
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

  // Keyed by the repo, not the folder: two terminals in one repo's
  // folders (or /tmp and /private/tmp) are the same list. A folder not seen
  // yet keys by itself until its status says which repo it's in.
  context() {
    const loc = this.place.context(this.ctx);
    const repo = this.roots.get(`${loc.on ?? ""}:${loc.cwd}`) ?? loc.cwd;
    return { ...loc, key: JSON.stringify([repo, loc.on, this.scope]) };
  }

  // Whether something notice() acts on changed (render and the timer ask;
  // no side effects).
  stale() {
    if (this.modPanes.changed()) return true;
    if (this.cycle && (this.cycle.shown !== this.place.shown() || this.cycle.asideStale())) return true;
    if (!this.place.shown()) return false;
    if (this.place.panel) {
      // followHandoff, on an edge only: the record not seen yet, or nvim's
      // pane came or went (not a redraw every look while nvim is open).
      const rec = handoffIn(this.tab());
      if (rec && (!rec.seen || (herdrNvimPanes(this.tab()).length > 0) !== this.nvimSeen)) return true;
    }
    if (stampMoved(this.watcher)) return true;
    const ctx = this.context();
    return ctx.key !== this.ctx?.key || ctx.tab !== this.ctx?.tab || (askIn(this.tab())?.seq ?? 0) > (this.seen.asks[this.tab()] ?? 0);
  }

  // What stale() looks for: the panel showed; another repo, space or tab;
  // another list's watch stamped the repo; the pane asked something
  // (gitview.ask); a mod pane closed (Loads.revive); nvim's pane came or
  // went (the handoff). Before every command, and from the kick task or the
  // timer after stale().
  notice(cx) {
    if (this.modPanes.gone()) this.loads.afterClose(cx);
    this.cycle?.stepAside();
    const edge = this.cycle?.look();
    if (edge === "shown") this.openOnShow(cx);
    else if (edge?.hidden) this.closeUnit(edge.hidden, cx);
    if (!this.place.shown()) return;
    const ctx = this.context();
    const tabChanged = ctx.tab !== this.ctx?.tab;
    const stamped = Boolean(followStamp(this.watcher)); // another list's watch saw the repo change
    if (ctx.key !== this.ctx?.key || stamped) this.refresh(cx, { quiet: ctx.key === this.ctx?.key, why: ctx.key !== this.ctx?.key ? `place ${ctx.key}` : "stamp" });
    else if (tabChanged) this.ctx = { ...this.ctx, tab: ctx.tab, space: ctx.space };
    // Another tab: its pane's file, and whether that changed meanwhile (its
    // view is rewritten when its rev moved: the pane reloads it).
    if (tabChanged) {
      this.adoptPane();
      this.followSelection();
    }
    this.followAsk(cx);
    if (this.place.panel) this.followHandoff(cx);
  }

  // The diff pane's ask (gitview.ask): s/u/x on its file, or J/K. Done
  // here, the one writer of the tab's view, then answered (gitview.answer)
  // once the view it leads to is written.
  followAsk(cx) {
    const tab = this.tab();
    const a = askIn(tab);
    if (!tab || !a || a.seq <= (this.seen.asks[tab] ?? 0)) return;
    this.seen.asks[tab] = a.seq;
    const v = a.view;
    const list = this.list();
    if ((a.op === "next" || a.op === "prev") && list && v) {
      const row = list.stepFile(v, a.op === "next" ? 1 : -1);
      if (row) this.select(row, cx, { open: false });
      return answer(tab, a.seq, row ? null : a.op === "next" ? "last file" : "first file");
    }
    if (!["stage", "unstage", "discard"].includes(a.op) || !v || v.commit) return answer(tab, a.seq);
    this.act([a.op, "--section", v.section, "--", v.path], { root: v.root, on: v.on }, cx, { tab, seq: a.seq });
  }

  // ---- the handoff to herdr-nvim ----------------------------------------------
  //
  // Enter with herdr-nvim closes the diff pane and its nvim pane takes the
  // place (nvim.js records the handoff). When that pane goes (`:q`), the diff
  // comes back on the file, its cursor on the line, with the keyboard, as
  // the TUI's diff did when nvim inside it quit. What can go wrong, and what
  // happens then:
  // - nvim's pane isn't in the app's lists yet right after the open: wait
  //   until it's seen (10 s at most, then forget it);
  // - it's closed by cmd-e or the Files button: the panel leaves gitview,
  //   and a hidden panel forgets every handoff (notice);
  // - it's closed while the panel is hidden: the same;
  // - the person left the tab: forgotten, so coming back doesn't restore;
  // - another diff opened meanwhile (the tab's view moved, or a diff pane is
  //   there): forgotten;
  // - the file left the list: forgotten, nothing shown;
  // - nvim's pane closing drops an exec in flight: Loads.afterClose;
  // - herdr-nvim's sync at the same moment: with the panel on Changes it
  //   leaves the panel as it is (its sync-rule.js), and this acts only on
  //   the next look.

  // The row Enter handed to herdr-nvim in this tab.
  handedOff(row) {
    const rec = handoffIn(this.tab());
    return Boolean(rec && row?.kind === "file" && this.list() && sameFile(rec, this.list().viewFor(row)));
  }

  followHandoff(cx) {
    const tab = this.tab();
    for (const t of Object.keys(handoffs())) if (t !== tab) setHandoff(t, null); // the person left that tab
    const rec = handoffIn(tab);
    if (!rec) return;
    const nvim = herdrNvimPanes(tab).length > 0;
    this.nvimSeen = nvim;
    if (!rec.seen) {
      if (nvim) setHandoff(tab, { ...rec, seen: true });
      else if (Date.now() - rec.at > 10000) setHandoff(tab, null);
      return;
    }
    if (nvim) return;
    setHandoff(tab, null);
    // Another file shown meanwhile (not the same one written again: a :w
    // there moves its rev, and the view is rewritten): nothing comes back.
    if (diffPane(tab) || (viewIn(tab)?.showing ?? 0) !== rec.showing) return;
    const list = this.list();
    const row = list && list.where.root === rec.root && (list.where.commit ?? null) === rec.commit ? list.findFile(rec.path, rec.section) : null;
    // Back on the diff, the keys on the list (v1: :q came back to the list).
    if (row) this.select(row, cx, { line: rec.line });
  }

  // The person hid the panel: the rest of gitview's unit closes with it, as
  // v1's toggle closed the editor with the view. The nvim gitview handed a
  // file to (herdr-nvim's daemon keeps its buffers, unsaved edits and
  // cursor; shown again, the diff of that file comes back, followHandoff);
  // an editor gitview started: a clean nvim quits, one with unsaved
  // changes asks first (Save, Discard, Cancel: the panel back), another
  // editor stays open and says so. A herdr-nvim pane the person opened
  // themselves (no handoff) stays.
  closeUnit(tabs, cx) {
    for (const tab of tabs) {
      const rec = handoffIn(tab);
      if (rec) {
        // Ours (gitview handed the file over): closed now, its diff shown
        // again when the panel is (followHandoff: nvim's pane is gone).
        setHandoff(tab, { ...rec, seen: true });
        this.closeNvimOf(tab, cx);
      }
      const ed = liveEditor(tab);
      if (ed) this.closeEditor(tab, ed, cx);
    }
  }

  // herdr-nvim's panes in the tab: the ones the app lists, and its nvim's
  // terminals as herdr-core has them (ended: its pane goes; the daemon
  // stays).
  closeNvimOf(tab, cx) {
    const panes = herdrNvimPanes(tab);
    for (const p of panes) run("pane.close", { pane: p.pane });
    cx.spawn(async () => {
      const known = new Set(panes.map((p) => p.terminal).filter(Boolean));
      const local = tab.includes("/") ? null : tab;
      if (!local) return;
      // Ones the app doesn't list yet end through herdr-core (the app can be
      // a moment behind it, and then doesn't know the terminal to kill).
      for (const t of await labelled(`herdr-nvim.sidebar=${local}`)) if (!known.has(t)) await endTerminal(t);
    });
  }

  closeEditor(tab, ed, cx) {
    cx.spawn(async () => {
      const state = await editorState(ed);
      if (state === "clean") {
        await quitEditor(ed, "qa");
        setEditor(tab, null);
      } else if (state === "dirty") {
        const p = panelState();
        const back = p ? [...(p.stack ? [{ run: ["layout.select", { stack: p.stack, child: p.slot }] }] : []), { run: ["layout.show", { id: p.panel }] }] : [];
        askToClose(ed, back);
      } else notify({ title: "gitview", body: `${ed.name} stays open: gitview can't ask it about unsaved changes`, level: "info" });
    });
  }

  // A click or Enter on the handed-off row: the diff again in nvim's place.
  // nvim's pane closes; nvim keeps running (its daemon keeps the buffers,
  // unsaved edits and cursor), and Enter in the diff goes back to it.
  bringBack(row, cx) {
    const tab = this.tab();
    const rec = handoffIn(tab);
    setHandoff(tab, null);
    for (const p of herdrNvimPanes(tab)) run("pane.close", { pane: p.pane });
    this.select(row, cx, { line: rec?.line ?? null, focusPane: true });
  }

  // The panel just showed (cmd-shift-g, the Changes button), and the tab
  // has no diff pane (hiding it closed them): from a fresh status, the
  // selected file, else the first one, opens there, as the TUI's toggle did.
  openOnShow(cx) {
    // A status for this place under a second old is fresh enough.
    const same = this.loadedKey === this.context().key;
    if (!same || Date.now() - (this.statusAt ?? 0) > 1000) this.refresh(cx);
    else this.ctx = { ...this.ctx, ...this.context() };
    const tab = this.tab();
    // A file handed to nvim comes back on its own (followHandoff).
    if (!tab || diffPane(tab) || handoffIn(tab)) return;
    // The list it kept while hidden: its file at once (the refresh corrects
    // it if it moved meanwhile: followSelection).
    const known = same && this.list()?.shownFile();
    if (known) return this.select(known, cx);
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
  // `quiet`: the same repo again (a watch batch, a stamp, a poll): no
  // spinner.
  refresh(cx, { poll = false, quiet = poll, why = "" } = {}) {
    if (shared("gitview.debug")) console.log(`GVEVENT ${JSON.stringify({ t: Date.now(), mod: "list", refresh: why })}`);
    this.ctx = this.context();
    if (!this.ctx.cwd) {
      this.work = null;
      this.error = "No folder";
      return;
    }
    this.loads.latest(cx, "status", async (cx, current) => {
      const started = Date.now();
      this.statusAt = started;
      const ctx = this.ctx;
      const old = this.work;
      const base = poll && this.scope === "branch" && this.loadedKey === ctx.key ? old?.where.base : null;
      const known = base ? ["--base", base.label, "--merge-base", base.merge_base, ...(old.status.head ? ["--head", old.status.head] : [])] : [];
      let status;
      try {
        status = await gv(["status", "--repo", ctx.cwd, "--scope", this.scope, ...known, ...onArgs(ctx.on)]);
      } catch (e) {
        if (current() && ctx.key === this.ctx.key) {
          const m = message(e);
          // The same repo as shown: a passing failure (a remote hiccup, a
          // timeout) keeps the list and says so in the footer.
          if (this.work && this.loadedKey === ctx.key && !/not (inside )?a git repository/.test(m)) this.flash.set(m);
          else {
            this.work = null;
            this.error = /not (inside )?a git repository/.test(m) ? "Not a git repository" : m;
          }
        }
        return;
      }
      if (!current() || ctx.key !== this.ctx.key) return;
      this.roots.set(`${ctx.on ?? ""}:${ctx.cwd}`, status.root);
      // The key by repo from now on (the same list for this folder).
      ctx.key = this.ctx.key = JSON.stringify([status.root, ctx.on ?? null, this.scope]);
      const where = { root: status.root, on: ctx.on ?? null, base: status.base ?? null };
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
      if (!ctx.on) setIgnoredDirs(status.root, status.ignored_dirs ?? []);
      if (!ctx.on) watch(this.watcher, status.root, cx, (cx, paths) => { if (this.place.shown() && !this.ownBatch(paths)) this.refresh(cx, { quiet: true, why: `watch ${JSON.stringify(paths).slice(0, 200)}` }); });
      else this.watcher.root = null; // a local repo's watch ends
      this.followSelection();
      // cmd-g's tab: its pane came with it, so its first file shows at once.
      if (this.place.showsFirstFile && this.mode === "files" && !liveView(viewIn(this.tab()))) {
        const first = this.work.shownFile();
        if (first) this.select(first, cx, { open: false });
      }
      // The pane's s/u/x: answered once the status after it is shown.
      if (this.toAnswer && started >= this.toAnswer.after) {
        answer(this.toAnswer.tab, this.toAnswer.seq);
        this.toAnswer = null;
      }
      knowEditor(where.root, where.on, cx);
    }, { quiet });
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
      // Its rev moved (an agent wrote to it, it was staged in part): the
      // pane reloads it, and its header follows the counts.
      const now = work.viewFor(there);
      if (["rev", "add", "del", "kind", "letter"].some((k) => now[k] !== view[k])) this.show(there);
      return;
    }
    const next = work.shownFile();
    if (next) this.show(next);
    else setView(tab, null);
  }

  // Another tab shows: the list selects the file its pane shows.
  adoptPane() {
    const view = liveView(viewIn(this.tab()));
    const list = this.list();
    if (!view || !list || view.root !== list.where.root || (view.commit ?? null) !== (list.where.commit ?? null)) return;
    const row = list.findFile(view.path, view.section);
    if (row) {
      list.select(row);
      this.reveal(list.index(row), { margin: 28 });
    }
  }

  // ---- selecting ----------------------------------------------------------------

  // Select a row; a file shows in the diff pane (`open`: open the pane when
  // the tab has none: a click or j/k, not a refresh; `line`: a note's).
  // `focusPane`: the diff pane gets the keyboard (coming back from nvim).
  select(row, cx, { open = true, line = null, focusPane = false } = {}) {
    const list = this.list();
    if (!list || !row) return;
    list.select(row);
    this.reveal(list.index(row), { margin: 28 });
    if (row.kind === "file") this.show(row, { open, cx, line, focusPane });
  }

  show(row, { open = false, cx, line = null, focusPane = false } = {}) {
    const tab = this.tab();
    if (!tab) return;
    const next = { ...this.list().viewFor(row, line), focus: focusPane };
    // The same file as shown, the same way (j on the last row, a click on
    // the selected one): nothing to write, nothing for the pane to reload.
    const was = liveView(viewIn(tab));
    const same = was && sameFile(was, next) && !line && !focusPane && ["rev", "add", "del", "kind", "letter", "scope"].every((k) => was[k] === next[k]);
    if (!same) setView(tab, next);
    if (open && cx && this.place.opensDiffPane) this.ensurePane(tab, cx, { focusPane });
  }

  // The tab's diff pane, opened on first use: full height at the tab's
  // right edge (`json sidebar`), left of this panel.
  ensurePane(tab, cx, { focusPane = false } = {}) {
    if (diffPane(tab) || this.opening === tab) return;
    this.opening = tab;
    cx.spawn(async (cx) => {
      try {
        const { pane } = await gv(["sidebar", "--tab", tab, "--mod", "gitview-diff", "--props", JSON.stringify({ tab })]);
        if (focusPane && pane) run("pane.focus", { pane }); // the pane takes the keys (its view says so)
        else this.focusList(cx); // the new pane took the app's focus; the list keeps it
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
    if (this.handedOff(row)) return this.bringBack(row, cx);
    if (row.file.kind === "deleted") return this.flash.set(`${row.file.path} is deleted`);
    const tab = this.tab();
    const list = this.list();
    const space = this.ctx?.space;
    this.loads.run(cx, "nvim", async (cx) => {
      const view = list.viewFor(row);
      const cursor = cursorIn(tab);
      let line = cursor && sameFile(cursor, view) ? cursor.line : null;
      if (!line) line = (await gv(diffArgs(view, { dark: isDark(theme().background), unfold: false })))?.first_change ?? 1;
      await openNvim({ root: list.where.root, on: list.where.on, path: row.file.path, line, tab, space, view, closeDiff: this.place.closesForHerdrNvim, cx });
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
  // `asked`: {tab, seq}: the pane's ask, answered when the status after
  // the op has been shown.
  act(args, where, cx, asked = null) {
    // Its own watch batch can come before the op's reply: from now on.
    this.ownOp = { at: Date.now() + 60000, paths: args.slice(args.indexOf("--") + 1) };
    this.loads.run(cx, args[0], async (cx) => {
      try {
        await gv([args[0], "--repo", where.root, ...onArgs(where.on), ...args.slice(1)]);
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
        if (asked) answer(asked.tab, asked.seq);
        this.ownOp = null;
        return;
      }
      if (asked) this.toAnswer = { ...asked, after: Date.now() };
      this.ownOp.at = Date.now();
      this.refresh(cx);
    });
  }

  // A watch batch that's only this list's own op (git's files, the op's
  // paths), while it runs or within 1.5 s after: the refresh after the op
  // shows it already.
  ownBatch(paths) {
    const op = this.ownOp;
    if (!op || Date.now() - op.at > 1500 || paths === "all") return false;
    return paths.every((p) => p.startsWith(".git/") || op.paths.some((q) => p === q || (q.endsWith("/") && p.startsWith(q))));
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
    this.ownOp = { at: started + 60000, paths: [] }; // its index and refs writes
    this.loads.run(cx, "commit", async (cx) => {
      try {
        const r = await gv(["commit", "--repo", work.where.root, ...onArgs(work.where.on), "--", text]);
        this.commitInput.set_value("");
        this.focusHere(cx);
        notify({ title: "Committed", body: r.summary ?? text, level: "success" });
      } catch (e) {
        // The message stays in the box either way.
        if (Date.now() - started >= EXEC_LIMIT_MS) {
          notify({ title: "gitview", body: "the commit is still running or timed out: check git log", level: "warning" });
        } else notify({ title: "Commit failed", body: message(e), level: "error" });
      } finally {
        this.committing = false;
      }
      this.ownOp.at = Date.now();
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
  // Each view keeps its own scroll: back in one, it's where it was.
  setMode(mode, cx, how = null) {
    const changed = mode !== this.mode;
    if (changed) {
      this.tops[this.mode] = this.scroll.top;
      this.scroll.top = this.tops[mode] ?? 0;
    }
    this.mode = mode;
    const list = this.list();
    if (list && !(changed && mode in this.tops)) this.reveal(list.selectedIndex(), { center: true });
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
    if (!row) setView(tab, null);
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
    this.focusHere(cx);
    const list = this.list();
    if (!list || row.kind === "header") return;
    if (row.kind === "dir") {
      list.select(row);
      list.toggleFold(row);
    } else if (e.click_count >= 2) this.edit(row, cx);
    else if (this.handedOff(row)) this.bringBack(row, cx);
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

  // What the list draws now, for the debug trace (modkit.js trace).
  traceFields() {
    const list = this.list();
    const body = this.mode === "log"
      ? (this.history.commits.length ? "content" : this.history.loading ? (this.loads.shown() ? "loading-text" : "blank") : "empty-state")
      : this.mode === "notes" ? (this.notes().length ? "content" : "empty-state")
      : list?.rows.length ? "content" : list ? "empty-state" : this.loads.busy() ? "loading-text" : "empty-state";
    return {
      tab: this.tab(),
      mode: this.mode,
      focused: this.handle.is_focused() || Boolean(this.typing),
      viaApp: Boolean(this.viaApp),
      body,
      spinner: this.loads.shown(),
      header: this.mode === "files" ? `${this.work?.status.scope ?? "-"} ${this.work?.files() ?? 0} files` : this.mode,
      selected: list?.row()?.key ?? null,
    };
  }

  render(cx) {
    if (this.stale()) this.kick();
    const t = theme();
    const focused = this.showsFocus();
    const notes = this.notes();
    const mode = MODES[this.mode];
    trace("list", () => this.traceFields(), this);
    return this.place
      .frame(v_flex().id("gitview").size_full().min_h(0).bg(t.background).text_color(t.foreground).text_size(13), t, focused)
      .track_focus(this.handle)
      .on_mouse_down("left", (_e, cx) => { this.focusHere(cx); cx.notify(); })
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
