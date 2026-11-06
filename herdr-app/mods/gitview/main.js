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
import { v_flex, TextareaState } from "gpui-base";
import { focused, notify, props, shared, theme } from "herdr";
import { Tracer, event, run } from "./telemetry.js";
import { endTerminal, gv, labelled, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile, sameShown } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import { answer, askIn, cursorIn, handoffIn, setIgnoredDirs, handoffs, liveView, notesKey, notesOf, setHandoff, setTreeMode, setView, treeMode, viewIn } from "./shared.js";
import { diffPane, panelDiffPanes, panelState, herdrNvimPanes } from "./layout.js";
import { Flash, Loads, commandLoop, debounced, followStamp, stampMoved, watch } from "./modkit.js";
import { listPlacement } from "./placement.js";
import { staleWhy } from "./notice.js";
import { closeEditorOf, editorIn, editorState, liveEditor, quitEditor, setEditor } from "./editors.js";
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
    this.tr = new Tracer("list", () => ({ tab: this.tab(), space: this.ctx?.space }), () => this.traceState());
    this.tr.start(cx);
    this.place = listPlacement(props()); // props(): read here only (placement.js)
    this.cycle = this.place.cycle(); // the panel's show/hide (placement.js)
    this.handle = cx.focus_handle();
    this.scroll = new Scroller("gitview-list");
    // Enter commits, Shift-Enter starts a new line (as the composer); one
    // line high until the message has more (v1 wrote it in nvim).
    this.commitInput = TextareaState.new({ placeholder: "Commit message", rows: 1 });
    this.commitInput.set_auto_grow(1, 8);
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
    this.noSubmodules = new Set(); // repos (keys) whose status needs --ignore-submodules
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    // A lost op (app bug #1) never gets to its finally or its refresh: no
    // commit stays running, its own batches aren't skipped, and the repo
    // is shown as it is.
    this.loads.onLost = (cx) => {
      this.committing = 0;
      this.ownOp = null;
      // The pane's ask it was doing: answered once the repo is shown.
      if (this.acting) this.toAnswer = { ...this.acting, after: Date.now() };
      this.acting = null;
      this.refresh(cx, { quiet: true, why: "revive" });
    };
    this.watcher = {};
    // What render notices (stale(): another repo, space or tab, a stamp,
    // the pane's ask, the panel's edges) is acted on from a task of this
    // copy's own, and from a light timer: a hidden or covered copy isn't
    // drawn, but its timers run.
    this.kick = debounced(cx, 0, (cx) => this.notice(cx));
    cx.timer.every(300, (cx) => {
      const r = this.reviveAt;
      if (r && Date.now() >= r.due) {
        this.reviveAt = null;
        this.loads.revive(cx, r.before);
        cx.notify();
      }
      if (!this.stale()) return;
      this.notice(cx);
      cx.notify();
    });
    this.refresh(cx);
    // Polls, while it's on screen: a remote repo every 4 s while the person
    // works in gitview (the list or the tab's diff pane has the keys), else
    // every 10 s; a local one is the folder watch's, polled only after 30 s
    // without a status.
    cx.timer.every(4000, (cx) => {
      if (!this.ctx?.on || !this.place.shown()) return;
      const here = this.showsFocus() || (focused()?.pane ?? null) === (diffPane(this.tab())?.pane ?? undefined);
      if (here || Date.now() - (this.statusAt ?? 0) >= 10000) this.refresh(cx, { poll: true, why: "poll" });
    });
    cx.timer.every(10000, (cx) => { if (!this.ctx?.on && this.place.shown() && Date.now() - (this.statusAt ?? 0) > 30000) this.refresh(cx, { poll: true, why: "poll" }); });
    commandLoop(cx, { tr: this.tr, loads: this.loads, flash: this.flash, name: "gitview", before: (cx) => this.notice(cx), handle: (c, cx) => runCommand(this, c, cx) });
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
  // no side effects): the facts for notice.js staleWhy.
  stale() {
    const tab = this.tab();
    const shown = this.place.shown();
    const rec = shown && this.place.closesForHerdrNvim ? handoffIn(tab) : null;
    const ctx = shown ? this.context() : null;
    const why = staleWhy({
      panesMoved: this.loads.panesMoved(),
      cycle: this.cycle && { was: this.cycle.shown, aside: this.cycle.asideStale() },
      shown,
      handoff: rec && { seen: rec.seen, nvim: herdrNvimPanes(tab).length > 0, nvimSeen: this.nvimSeen },
      stamp: shown && stampMoved(this.watcher),
      key: ctx?.key,
      wasKey: this.ctx?.key,
      tab: ctx?.tab,
      wasTab: this.ctx?.tab,
      ask: shown ? askIn(tab)?.seq ?? 0 : 0,
      askSeen: this.seen.asks[tab] ?? 0,
    });
    // Each new reason once in the trace (render asks this often).
    if (why !== this.lastWhy) {
      this.lastWhy = why;
      if (why) this.tr.line("state", { stale: why });
    }
    return why !== null;
  }

  // Acts on what stale() looks for (notice.js has the table). Before every
  // command, and from the kick task or the timer after stale().
  notice(cx) {
    this.loads.panesClosed(cx);
    this.cycle?.stepAside();
    this.onEdge(this.cycle?.look(), cx);
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
    if (this.place.closesForHerdrNvim) this.followHandoff(cx); // it hands its diff to herdr-nvim
  }

  // The panel's edge (placement.js cycleEdge): a hide closes the unit in
  // the shown space's tabs (the diff panes first), a show opens the file.
  onEdge(edge, cx) {
    if (edge) event("list", { edge: edge === "shown" ? "shown" : { hidden: [...edge.hidden] } });
    if (edge === "shown") this.openOnShow(cx);
    else if (edge?.hidden) {
      for (const pane of panelDiffPanes()) if (edge.hidden.has(pane.tab)) run("pane.close", { pane: pane.pane });
      this.closeUnit(edge.hidden, cx);
    }
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
    if (a.op === "discard") this.confirmClosing(); // the pane's confirm
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
  // - the panel hidden by the person: nvim's pane closes with the unit
  //   (closeUnit), the record stays, and shown again the diff comes back;
  // - closed by cmd-e or the Files button: the diff comes back when the
  //   panel shows on Changes again;
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
    // Another file shown meanwhile (not the same one written again: a :w
    // there moves its rev, and the view is rewritten): nothing comes back.
    if (!this.restorable(tab, rec)) return setHandoff(tab, null);
    const nvim = herdrNvimPanes(tab).length > 0;
    this.nvimSeen = nvim;
    if (!rec.seen) {
      if (nvim) setHandoff(tab, { ...rec, seen: true });
      else if (Date.now() - rec.at > 10000) setHandoff(tab, null);
      return;
    }
    if (nvim) return;
    setHandoff(tab, null);
    if (diffPane(tab)) return;
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
      if (ed) this.closeEditor(tab, ed.name, cx);
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

  // Unsaved changes: Cancel shows the panel again.
  closeEditor(tab, name, cx) {
    const p = panelState();
    const back = p ? [...(p.stack ? [{ run: ["layout.select", { stack: p.stack, child: p.slot }] }] : []), { run: ["layout.show", { id: p.panel }] }] : [];
    cx.spawn(async () => {
      if ((await closeEditorOf(tab, { cancel: back })) === "unknown") notify({ title: "gitview", body: `${name} stays open: gitview can't ask it about unsaved changes`, level: "info" });
    });
  }

  // Browsing to another file while the nvim Enter opened is there (v1's
  // probe on a move; Enter stays the "edit this" path). Returns whether the
  // diff pane may open now; else it opens once nvim made way.
  // - handed to herdr-nvim: its pane closes (the daemon keeps the buffers,
  //   unsaved edits and cursor, so nothing to ask), the handoff goes (no
  //   :q restore of the old file), and the new file's diff takes the place;
  // - an nvim gitview started: a clean one quits and the diff takes its
  //   place; with unsaved changes it stays, no pane opens, and the next
  //   move asks again;
  // - another editor (it can't be asked) or a herdr-nvim pane the person
  //   opened (no handoff): left alone, as before (one diff pane at most:
  //   ensurePane).
  // What can go wrong: Enter's open still on its way (waited for, so its
  // handoff is there to clear), or its nvim's terminal not found yet
  // (waited for, then asked); nvim's pane not listed yet (closeNvimOf
  // also ends herdr-core's labelled terminals); two fast moves (one
  // leave at a time; the pane opens for the selection at the end); the
  // closing pane dropping the pane's open (it opens after nvim's pane went).
  leaveEditor(next, cx) {
    const tab = this.tab();
    if (!tab) return true;
    const was = liveView(viewIn(tab));
    if (was && sameFile(was, next)) return true;
    if (this.leaving) return false;
    const opening = this.loads.names().includes("nvim");
    const ed = liveEditor(tab);
    const askable = (ed) => Boolean(ed?.terminal && ed.kind === "nvim" && ed.sock);
    const starting = Boolean(ed && !ed.terminal && ed.kind === "nvim" && ed.sock); // its terminal not found yet
    if (!opening && !starting && !handoffIn(tab) && !askable(ed)) return true;
    this.leaving = true;
    cx.spawn(async (cx) => {
      try {
        for (let i = 0; opening && i < 100 && this.loads.names().includes("nvim"); i++) await cx.sleep(100);
        for (let i = 0; i < 40 && liveEditor(tab) && !liveEditor(tab).terminal; i++) await cx.sleep(100);
        if (handoffIn(tab)) {
          setHandoff(tab, null);
          const nvim = herdrNvimPanes(tab)[0];
          const row = this.list()?.row();
          if (nvim && this.tab() === tab && row?.kind === "file" && this.place.opensDiffPane) {
            this.replaceWithDiff(tab, nvim.pane, () => this.closeNvimOf(tab, cx), cx);
          } else {
            this.closeNvimOf(tab, cx);
            for (let i = 0; i < 30 && herdrNvimPanes(tab).length; i++) await cx.sleep(100);
          }
          event("list", { leave: "herdr-nvim" });
        } else if (askable(liveEditor(tab))) {
          const ed = liveEditor(tab);
          const state = await editorState(ed);
          event("list", { leave: ed.name, state });
          if (state === "dirty") return notify({ title: "gitview", body: "editor has unsaved changes — stays open", level: "info" });
          if (state === "clean") {
            // That nvim gone (not a new one Enter in the diff pane started
            // meanwhile: its record stays).
            await quitEditor(ed, "qa");
            const mine = () => editorIn(tab)?.terminal === ed.terminal;
            for (let i = 0; i < 30 && mine() && liveEditor(tab); i++) await cx.sleep(100);
            if (mine()) setEditor(tab, null);
          }
        }
        const row = this.list()?.row();
        if (this.tab() === tab && row?.kind === "file" && this.place.opensDiffPane) this.ensurePane(tab, cx);
      } catch (e) {
        this.flash.set(message(e));
      } finally {
        this.leaving = false;
        cx.notify();
      }
    });
    return false;
  }

  // The handoff can still bring its diff back: no other file shown since.
  restorable(tab, rec) {
    return (viewIn(tab)?.showing ?? 0) === rec.showing;
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
    // A file handed to nvim comes back on its own (followHandoff); a record
    // that can't any more goes.
    const rec = handoffIn(tab);
    if (rec && !this.restorable(tab, rec)) setHandoff(tab, null);
    if (!tab || diffPane(tab) || (rec && this.restorable(tab, rec))) return;
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
    event("list", { refresh: why });
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
        // A repo whose plain status failed on a submodule (the engine said
        // so): straight to the status that works.
        const sub = this.noSubmodules.has(ctx.key) ? ["--ignore-submodules"] : [];
        // A poll of a repo it knows: its root, not looked up again (--root).
        const root = poll && this.loadedKey === ctx.key ? old?.where.root : null;
        const repo = root ? ["--repo", root, "--root"] : ["--repo", ctx.cwd];
        status = await gv(["status", ...repo, "--scope", this.scope, ...known, ...sub, ...onArgs(ctx.on)]);
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
      if (status.ignore_submodules) this.noSubmodules.add(JSON.stringify([status.root, ctx.on ?? null, this.scope]));
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
      if (!ctx.on) {
        watch(this.watcher, status.root, cx, (cx, paths) => {
          if (!this.place.shown()) return false; // followed once it shows
          if (!this.ownBatch(paths)) this.refresh(cx, { quiet: true, why: `watch ${JSON.stringify(paths).slice(0, 200)}` });
        });
      }
      else this.watcher.root = null; // a local repo's watch ends
      this.followSelection();
      // cmd-g's tab: its pane came with it, so its first file shows at once.
      if (this.place.showsFirstFile && this.mode === "files" && !liveView(viewIn(this.tab()))) {
        const first = this.work.shownFile();
        if (first) this.select(first, cx, { open: false, browse: false });
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
      if (!sameShown(now, view)) this.show(there);
      return;
    }
    const next = work.shownFile();
    if (next) this.show(next);
    else this.writeView(tab, null);
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
  // `browse`: the person moved there (not a load picking the first file):
  // the nvim Enter opened makes way (leaveEditor), and the row handed to
  // herdr-nvim brings its diff back.
  select(row, cx, { open = true, line = null, focusPane = false, browse = true } = {}) {
    const list = this.list();
    if (!list || !row) return;
    if (browse && row.kind === "file") {
      if (this.handedOff(row)) return this.bringBack(row, cx);
      if (!this.leaveEditor(list.viewFor(row), cx)) open = false;
    }
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
    const same = was && sameShown(was, next) && !line && !focusPane;
    if (!same) this.writeView(tab, next);
    if (open && cx && this.place.opensDiffPane) this.ensurePane(tab, cx, { focusPane });
  }

  // The one writer of the pane's view (S1), traced: the pane's follow is
  // checked from these lines and the pane's diff execs.
  writeView(tab, view) {
    const rec = setView(tab, view);
    this.tr.line("state", { view: rec.seq, path: view?.path ?? null, section: view?.section ?? null, commit: view?.commit ? view.commit.slice(0, 7) : null });
  }

  // The tab's diff pane, opened on first use: full height at the tab's
  // right edge (`json sidebar`), left of this panel.
  ensurePane(tab, cx, { focusPane = false } = {}) {
    // `opening`: a split on its way (bounded: a dropped exec never ends it).
    if (diffPane(tab) || (this.opening?.tab === tab && Date.now() - this.opening.at < 15000)) return;
    this.opening = { tab, at: Date.now() };
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

  // The diff pane in nvim's place, in one turn: split off nvim's pane (the
  // new pane is in its column, full height), then close it, so the layout
  // changes once, with no moment where neither shows (the terminals
  // widening, then narrowing again) or both squeeze. Then the list keeps
  // the keys, as ensurePane does. A split that didn't take: ensurePane.
  replaceWithDiff(tab, pane, close, cx) {
    if (diffPane(tab)) return close();
    this.opening = { tab, at: Date.now() };
    run("pane.split", { direction: "right", pane, mod: "gitview-diff", props: { tab } });
    close();
    cx.spawn(async (cx) => {
      for (let i = 0; i < 40 && !diffPane(tab); i++) await cx.sleep(25);
      this.opening = null;
      if (diffPane(tab)) this.focusList(cx);
      else if (this.tab() === tab) this.ensurePane(tab, cx);
      cx.notify();
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
      // A move's leave and the diff pane it opens first (its nvim quitting
      // would take this file; the pane would open beside nvim).
      for (let i = 0; (this.leaving || this.opening) && i < 50; i++) await cx.sleep(100);
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
    event("list", { rowAction: op, row: row?.key ?? null, mode: this.mode, scope: this.scope });
    if (!row || !MODES[this.mode].actsOnWork || this.scope !== "worktree") return;
    const section = row.section.id;
    if (!allowed(op, section)) return this.flash.set(`can't ${op} in ${row.section.title}`);
    const paths = list.filesOf(row);
    if (!paths.length) return;
    if (op !== "discard") return this.act([op, "--section", section, "--", ...paths], list.where, cx);
    const what = row.kind === "header" ? `All ${paths.length} changed files` : row.kind === "dir" ? `All ${paths.length} changed files in ${row.path}` : row.file.path;
    confirmDiscard("gitview", what, { where: list.where, paths, section });
  }

  discardConfirmed(command, cx) {
    const p = takeDiscard("gitview", command);
    if (!p) return;
    this.act(["discard", "--section", p.section, "--", ...p.paths], p.where, cx);
    this.confirmClosing();
  }

  // The confirm dialog (a mod) closes right after its command: app bug #1
  // can drop the discard's exec then, though no pane closed. A discard
  // still running 1.5 s on is taken as lost (Loads.revive), and the repo
  // shown as it is (onLost). From the 300 ms timer, not a task's sleep: a
  // sleep resumed after that drop was seen never to come back.
  confirmClosing() {
    const at = Date.now();
    this.reviveAt = { due: at + 1500, before: at + 1000 };
  }

  // One op on the repo the list came from, then a refresh.
  // `asked`: {tab, seq}: the pane's ask, answered when the status after
  // the op has been shown.
  act(args, where, cx, asked = null) {
    // Its own watch batch can come before the op's reply: from now on.
    this.ownOp = { at: Date.now() + 60000, paths: args.slice(args.indexOf("--") + 1) };
    this.acting = asked;
    this.loads.run(cx, args[0], async (cx) => {
      try {
        // (Another git briefly holding the index: the engine tries again.)
        await gv([args[0], "--repo", where.root, "--root", ...onArgs(where.on), ...args.slice(1)]);
      } catch (e) {
        event("list", { opFailed: args[0], error: message(e) });
        notify({ title: "gitview", body: message(e), level: "error" });
        if (asked) answer(asked.tab, asked.seq);
        this.acting = null;
        this.ownOp = null;
        return;
      }
      if (this.acting === asked) this.acting = null;
      if (asked) this.toAnswer = { ...asked, after: Date.now() };
      if (this.ownOp) this.ownOp.at = Date.now(); // (revive may have dropped it)
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
    if (!text || !work || this.scope !== "worktree" || this.commitRunning()) return;
    this.committing = Date.now();
    const started = Date.now();
    this.ownOp = { at: started + 60000, paths: [] }; // its index and refs writes
    this.loads.run(cx, "commit", async (cx) => {
      try {
        const r = await gv(["commit", "--repo", work.where.root, "--root", ...onArgs(work.where.on), "--", text]);
        this.commitInput.set_value("");
        this.focusHere(cx);
        notify({ title: "Committed", body: r.summary ?? text, level: "success" });
      } catch (e) {
        // The message stays in the box either way.
        if (Date.now() - started >= EXEC_LIMIT_MS) {
          notify({ title: "gitview", body: "the commit is still running or timed out: check git log", level: "warning" });
        } else notify({ title: "Commit failed", body: message(e), level: "error" });
      } finally {
        this.committing = 0;
      }
      if (this.ownOp) this.ownOp.at = Date.now(); // (revive may have dropped it)
      this.refresh(cx);
    });
  }

  // A commit on its way (its start; bounded as the exec is).
  commitRunning() {
    return Boolean(this.committing) && Date.now() - this.committing < EXEC_LIMIT_MS + 1000;
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
    if (!row) this.writeView(tab, null);
    else if (!sameFile(view, this.work.viewFor(row))) this.select(row, cx, { open: false, browse: false });
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
    this.tr.line("input", { click: row.key, count: e.click_count ?? 1, mode: this.mode, scope: this.scope });
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

  // What the list draws now, for the trace (Tracer.drew).
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

  // The copy as a snapshot has it (telemetry.js): no commit message.
  traceState() {
    const tab = this.tab();
    const v = viewIn(tab);
    return {
      mode: this.mode,
      scope: this.scope,
      selected: this.list()?.row()?.key ?? null,
      view: v ? { path: v.path, section: v.section, seq: v.seq, showing: v.showing, empty: Boolean(v.empty) } : null,
      shown: this.place.shown(),
      focused: this.showsFocus(),
      ctx: this.ctx?.key ?? null,
      loads: this.loads.names(),
      opening: this.opening ?? null,
      committing: this.committing || 0,
      leaving: Boolean(this.leaving),
      handoff: handoffIn(tab),
      editor: liveEditor(tab),
      diffPane: diffPane(tab)?.pane ?? null,
      error: this.error ?? null,
    };
  }

  render(cx) {
    return this.tr.render(() => this.draw(cx));
  }

  draw(cx) {
    if (this.stale()) this.kick();
    const t = theme();
    const focused = this.showsFocus();
    const notes = this.notes();
    const mode = MODES[this.mode];
    this.tr.drew(() => this.traceFields());
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
        this.tr.line("input", { key: e.key, mode: this.mode, scope: this.scope, row: this.list()?.row()?.key ?? null });
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
