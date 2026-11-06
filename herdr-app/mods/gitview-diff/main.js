// gitview's diff pane: a full-height mod pane at the tab's right edge, left
// of the file list (the Changes panel), or the left half of cmd-g's gitview
// tab. Props: {tab} (or {space, own} in the gitview tab; placement.js holds
// what differs). It shows what the
// list put in shared "gitview.views"[tab] (shared.js): the diff as gitview's
// TUI draws it, with review notes under their lines.
//
// Keys (KEYS below; `?` shows them all): j/k (arrows, wheel, click) move
// the cursor; v or a drag selects; a writes a note on the selection (enter
// saves, esc cancels); enter opens nvim at the cursor (on a fold: unfolds);
// s/u/x stage, unstage, discard the file; J/K next/previous file of the
// list; m Diff or Preview (markdown, images, SVGs: preview.js); p sends the
// notes; esc clears. Commands: `gitview-diff.key:<key>`, `gitview-diff.note:<text>`,
// `gitview-diff.drag:<line>:<line>`, `gitview-diff.close`.

import { View, div } from "gpui-kit";
import { v_flex, TextareaState } from "gpui-base";
import { focused, props, setShared, settings, shared, spaces, theme } from "herdr";
import { Tracer, run } from "./telemetry.js";
import { gv, message } from "./engine.js";
import { diffArgs, isDark, sameFile, sameShown } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import { addNote, answerIn, ask, cursorIn, liveView, notesKey, notesOf, previewOff, removeNotes, setCursor, setPreviewOff, viewIn } from "./shared.js";
import { diffPane } from "./layout.js";
import { Flash, Loads, commandLoop, debounced, keyEvent, keyName, takeKeyboard } from "./modkit.js";
import { panePlacement } from "./placement.js";
import { editorLabel, knowEditor, openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import { diffHeader, footer } from "./chrome.js";
import { actionFor, chipsOf, toggleKeys } from "./keymap.js";
import { loadPreview, previewBody, previewKind } from "./preview.js";
import * as diffs from "./diffview.js";

const EMPTY = { rows: [] };

// Keys that act on the diff's lines: in Preview they say so instead, and
// while the next file's diff is on its way (the old one still shows) they
// wait for it (do nothing).
const inDiff = (run) => (v, cx) => (v.inPreview() ? v.flash.set("that works in Diff: m switches") : v.ready() ? run(v, cx) : null);
const diffShown = (v) => !v.inPreview();

// The pane's keys (keymap.js): the handling, the footer's keycaps and the
// keys card.
const KEYS = [
  { caps: ["j", "k"], label: "move", on: { j: inDiff((v) => v.line(1)), down: inDiff((v) => v.line(1)), k: inDiff((v) => v.line(-1)), up: inDiff((v) => v.line(-1)) } },
  { caps: ["pageup", "pagedown"], label: "a page", on: { pagedown: inDiff((v) => v.line(v.page())), space: inDiff((v) => v.line(v.page())), pageup: inDiff((v) => v.line(-v.page())) } },
  { caps: ["g", "G"], label: "top / bottom", on: { g: inDiff((v) => v.line(-Infinity)), home: inDiff((v) => v.line(-Infinity)), G: inDiff((v) => v.line(Infinity)), end: inDiff((v) => v.line(Infinity)) } },
  { caps: ["s"], label: "stage", rank: 1, on: { s: (v, cx) => v.act("stage", cx) } },
  { caps: ["u"], label: "unstage", rank: 2, on: { u: (v, cx) => v.act("unstage", cx) } },
  { caps: ["m"], label: "diff / preview", rank: 4, when: (v) => Boolean(previewKind(v.view)), on: { m: (v, cx) => v.togglePreview(cx) } },
  { caps: ["v"], label: "select", rank: 5, when: diffShown, on: { v: inDiff((v) => { v.anchor = v.anchor === null ? v.cursor : null; }) } },
  { caps: ["a"], label: "note", rank: 6, when: diffShown, on: { a: inDiff((v) => v.annotate()) } },
  { caps: ["enter"], label: (v) => editorLabel(v.view?.root, v.view?.on), rank: 3, on: { enter: (v, cx) => v.enterKey(cx), e: (v, cx) => v.edit(null, cx) } },
  { caps: ["J", "K"], label: "next / previous file", rank: 7, on: { J: (v, cx) => v.nextFile(1, cx), K: (v, cx) => v.nextFile(-1, cx) } },
  { caps: ["x"], label: "discard", rank: 8, on: { x: (v, cx) => v.act("discard", cx) } },
  { caps: ["p"], label: "send notes", rank: 9, on: { p: (v, cx) => v.send(cx) } },
  { caps: ["r"], label: "reload", on: { r: (v, cx) => v.reload(cx) } },
  { caps: ["escape"], label: "clear the selection", on: { escape: (v) => { v.anchor = null; } } },
  { caps: ["?"], label: "all keys", on: { "?": (v) => toggleKeys("gitview: the diff pane", [{ title: "Diff pane", table: KEYS }], v) } },
];
const NOTE_KEYS = [{ caps: ["enter"], label: "save the note", rank: 1, on: {} }, { caps: ["escape"], label: "cancel", rank: 2, on: {} }];

// Notes are written on staged or unstaged changes, as the TUI's: the side
// (cached or not) of a view that takes them, null for a commit's or the
// branch's diff.
const noteSide = (v) => (!v || v.commit || v.section === "branch" ? null : Boolean(v.cached));

export default class GitDiff extends View {
  init(_props, cx) {
    this.tr = new Tracer("pane", () => {
      const tab = this.tab();
      return { tab, space: spaces().find((s) => s.tabs.some((t) => t.id === tab))?.id ?? null };
    }, () => ({ ...this.traceFields(), loads: this.loads.names(), cursor: this.cursor, error: this.diff?.error ?? null }));
    this.tr.start(cx);
    this.place = panePlacement(props()); // props(): read here only (placement.js)
    this.handle = cx.focus_handle();
    this.scroll = new Scroller("gitview-diff-rows");
    this.composer = TextareaState.new({ placeholder: "Note for the agent", rows: 3 });
    this.composer.on("submit", (_e, cx) => this.saveNote(this.composer.value(), cx));
    this.view = null; // what's shown (a view from the list), null: nothing
    this.seq = 0; // the stored view's seq and showing, as last seen
    this.showing = 0;
    this.diff = EMPTY;
    this.cursor = 0;
    this.anchor = null;
    this.dragging = false;
    this.unfold = false;
    this.width = shared("gitview.paneWidth") ?? 0; // the last pane's: a new pane lays out right away
    this.preview = null; // {view, kind, …} (preview.js), or {view, failed}
    this.diffView = null; // the view `diff` is of
    this.lastBody = null; // "diff" | "preview": what the body showed last
    this.switchedAt = 0; // another file was asked for (the old body dims after 150 ms)
    this.cursorKick = debounced(cx, 250, () => this.publishNow());
    this.cursorMoved = false; // the person moved the cursor in this file
    this.dark = isDark(theme().background);
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    // What render notices (a new view, a theme switch) loads from here.
    this.kick = debounced(cx, 0, (cx) => this.sync(cx));
    this.sync(cx);
    commandLoop(cx, { tr: this.tr, loads: this.loads, flash: this.flash, name: "gitview-diff", before: (cx) => this.sync(cx), handle: (c, cx) => this.command(c, cx) });
  }

  tab() {
    return this.place.tab();
  }

  // This copy's pane (null: not yet in the app's lists).
  myPane() {
    return diffPane(this.tab())?.pane ?? null;
  }

  // The keys here: the handle when the app's focus is on this pane, else
  // the app's focus first (one focus shows).
  focusHere(cx) {
    if (this.myPane() && focused()?.pane === this.myPane()) this.handle.focus();
    else this.focusPane(cx);
  }

  // The keyboard and the app's focus together.
  focusPane(cx) {
    const me = this.myPane();
    if (me) run("pane.focus", { pane: me });
    takeKeyboard(this, cx);
  }

  // The space of this pane's tab.
  space() {
    const tab = this.tab();
    return spaces().find((s) => s.tabs.some((x) => x.id === tab))?.id ?? null;
  }

  // ---- what to show -----------------------------------------------------------

  // A new view from the list (J/K here ask it), or a theme switch: same file
  // keeps the cursor, another starts at its first change (or at the note
  // the list jumped to). The view's `showing` says whether another file was
  // shown since (the list moved on and back while this pane wasn't drawn):
  // then this one starts afresh too.
  sync(cx) {
    this.loads.panesClosed(cx); // Loads.revive
    if (isDark(theme().background) !== this.dark) {
      this.dark = !this.dark;
      this.reload(cx);
    }
    const stored = viewIn(this.tab());
    if ((stored?.seq ?? 0) === this.seq) return;
    const fresh = (stored?.showing ?? 0) !== this.showing;
    this.seq = stored?.seq ?? 0;
    this.showing = stored?.showing ?? 0;
    if (stored?.focus) this.focusPane(cx); // the click-back from nvim: the keys are the diff's
    const view = liveView(stored);
    const same = !fresh && sameFile(view, this.view);
    // The same file written again (the list's j on the last row, a refresh):
    // load again only when its change did (the counts) or a line was asked.
    const changed = !same || view.line || !sameShown(view, this.view);
    this.view = view;
    if (!same) {
      // Another file: the old body stays until the new one is here (dimmed
      // after 150 ms); what belongs to the old rows goes now.
      this.cursorMoved = false;
      this.anchor = null;
      this.unfold = false;
      this.pendingNote = null;
      this.switchedAt = Date.now();
      cx.spawn(async (cx) => { await cx.sleep(160); cx.notify(); });
    }
    if (!view) return;
    knowEditor(view.root, view.on, cx);
    // The list is the one that watches the repo: it writes the view again
    // when the file's rev moves, and this loads again then.
    if (changed) {
      this.loadDiff(cx, { keep: same && !view.line, line: view.line, quiet: same });
      if (this.wantsPreview()) this.loadPreview(cx, { quiet: same });
    }
  }

  // The shown file again (the repo changed): no spinner.
  reload(cx) {
    if (!this.view) return;
    this.loadDiff(cx, { keep: true, quiet: true });
    if (this.wantsPreview()) this.loadPreview(cx, { quiet: true });
  }

  // ---- preview ------------------------------------------------------------------

  // The shown file previews, and the person didn't switch its kind to
  // Diff (kept in shared gitview.previewOff, so a new pane, after nvim say,
  // keeps it).
  wantsPreview() {
    const kind = previewKind(this.view);
    return Boolean(kind) && !previewOff().includes(kind);
  }

  // The mode, decided from the file's path before anything loads: Preview,
  // unless it can't show this file (then the diff, with the reason).
  inPreview() {
    return this.wantsPreview() && !(this.preview?.failed && sameFile(this.preview.view, this.view));
  }

  // What the body shows is of the shown file (else the old one still shows).
  ready() {
    return this.inPreview() ? Boolean(this.preview && !this.preview.failed && sameFile(this.preview.view, this.view)) : sameFile(this.diffView, this.view);
  }

  // What it can't show (too big, not there, a failed read) falls back to
  // Diff, with the reason in the footer.
  loadPreview(cx, { quiet = false } = {}) {
    this.loads.latest(cx, "preview", async () => {
      const view = this.view;
      if (!view) return;
      try {
        const had = this.preview && !this.preview.failed && sameFile(this.preview.view, view) ? this.preview : null;
        const p = await loadPreview(view, had);
        if (!sameFile(this.view, view) || (had && p.same)) return;
        this.preview = { ...p, view };
        this.lastBody = "preview";
        this.publishCursor();
      } catch (e) {
        if (!sameFile(this.view, view)) return;
        this.preview = { view, failed: true };
        this.flash.set(message(e));
      }
    }, { quiet });
  }

  // m, or the header's Diff | Preview: kept per kind of file, for every
  // pane (shared gitview.previewOff: Diff on a markdown file keeps markdown
  // files in Diff).
  togglePreview(cx, preview = !this.inPreview()) {
    const kind = previewKind(this.view);
    if (!kind) return this.flash.set("no preview for this file (markdown, images, SVGs have one)");
    setPreviewOff(kind, !preview);
    if (preview && !sameFile(this.preview?.view, this.view)) this.loadPreview(cx);
    else if (preview && this.preview?.failed) this.flash.set("can't preview this one: the diff shows");
    this.publishCursor();
  }

  // The shown view's diff, one load at a time: asked again meanwhile (the
  // repo changed again, another view), it loads once more afterwards, with
  // the asks merged (`keep`: the cursor stays; else it goes to `line`, or
  // the first change).
  // `quiet`: no spinner (a reload of the same file).
  // A reload of the same rows asks `--unless` their hash: when nothing
  // changed the engine says {same}, and nothing is parsed or redrawn.
  loadDiff(cx, { keep, line = null, quiet = false }) {
    const asked = this.asked;
    this.asked = asked ? { keep: asked.keep && keep, line: line ?? asked.line } : { keep, line };
    this.loads.latest(cx, "diff", async () => {
      const view = this.view;
      const { keep, line } = this.asked ?? { keep: true, line: null };
      this.asked = null;
      if (!view) return;
      const args = diffArgs(view, { dark: this.dark, unfold: this.unfold });
      const again = keep && !line && sameFile(this.diffView, view) && JSON.stringify(args) === this.diffArgs;
      let diff;
      try {
        diff = await gv(again && this.diff.hash ? diffArgs(view, { dark: this.dark, unfold: this.unfold, unless: this.diff.hash }) : args);
        // The rows shown are still right; the cursor record follows the
        // file anyway (back from another file's Preview, the same diff).
        if (diff.same) return this.publishCursor();
        this.diffArgs = JSON.stringify(args);
      } catch (e) {
        if (sameFile(this.view, view) && !sameFile(this.diffView, view)) {
          this.diff = { rows: [], error: message(e) }; // said in the body, after the flash too
          this.diffView = view;
        }
        throw e;
      }
      if (!sameFile(this.view, view)) return;
      const another = !sameFile(this.diffView, view);
      // The same file again (an agent's edit, unfold): the cursor, the
      // selection's anchor and the view stay on their lines, not on their
      // row numbers, which move when lines come or go above them.
      const keepLines = keep && !line && !another ? { cursor: this.lineKey(this.cursor), anchor: this.lineKey(this.anchor), offset: this.rowOffset(this.cursor) } : null;
      this.diff = diff;
      this.diffView = view;
      this.lastBody = "diff";
      const n = diff.rows.length;
      if (line || !keep || another) {
        const at = line ? diff.rows.findIndex((r) => r.n === line) : diff.rows.findIndex((r) => r.k === "add" || r.k === "del");
        this.cursor = Math.max(0, at);
        if (another) this.scroll.top = 0;
        this.pendingReveal = { row: this.cursor, center: true }; // once `display` has these rows
      } else if (keepLines) {
        const at = this.rowOf(keepLines.cursor);
        if (at >= 0) {
          this.cursor = at;
          if (keepLines.offset !== null) this.pendingAnchor = { row: at, offset: keepLines.offset };
        }
        if (this.anchor !== null) this.anchor = Math.max(0, this.rowOf(keepLines.anchor));
      }
      this.cursor = Math.min(this.cursor, Math.max(0, n - 1));
      if (this.anchor !== null) this.anchor = Math.min(this.anchor, Math.max(0, n - 1));
      this.publishCursor();
    });
  }

  // Row i as a line: its new-side number, else its old side's (null: a
  // fold, nothing).
  lineKey(i) {
    const r = i === null ? null : this.diff.rows[i];
    return r ? (r.n ? { n: r.n } : r.o ? { o: r.o } : null) : null;
  }

  // The row of that line in the diff now; a line that went: the nearest.
  rowOf(key) {
    if (!key) return -1;
    const rows = this.diff.rows;
    const exact = rows.findIndex((r) => (key.n ? r.n === key.n : r.o === key.o));
    if (exact >= 0 || !key.n) return exact;
    let best = -1;
    rows.forEach((r, i) => { if (r.n && (best < 0 || Math.abs(r.n - key.n) < Math.abs(rows[best].n - key.n))) best = i; });
    return best;
  }

  // How far row i is below the top of the view (px), null when not drawn.
  rowOffset(i) {
    const at = this.display?.findIndex((d) => d.index === i) ?? -1;
    return at >= 0 && this.scroll.offsets[at] !== undefined ? this.scroll.offsets[at] - this.scroll.top : null;
  }

  // The line under row i (a deleted line: the next line that's there).
  lineAt(i) {
    const rows = this.diff.rows;
    for (let j = i; j < rows.length; j++) if (rows[j]?.n) return rows[j].n;
    for (let j = i; j >= 0; j--) if (rows[j]?.n) return rows[j].n;
    return this.diff.first_change ?? 1;
  }

  // Where the cursor is, for the list's Enter (the file and its line;
  // `preview`: the pane shows the Preview, then Enter opens line 1 unless
  // the cursor was moved). At most every 250 ms: a held j is one write,
  // not thirty (every shared write renders every mod).
  publishCursor() {
    this.cursorKick();
  }

  publishNow() {
    const v = this.view;
    const tab = this.tab();
    const preview = this.inPreview();
    if (!v || !tab || !this.ready() || (!this.diff.rows.length && !preview)) return;
    const line = preview && !this.cursorMoved ? 1 : this.lineAt(this.cursor);
    const was = cursorIn(tab);
    if (was && was.line === line && was.preview === preview && sameFile(was, v)) return;
    setCursor(tab, { root: v.root, on: v.on, path: v.path, section: v.section, commit: v.commit, line, preview });
  }

  // ---- moving -------------------------------------------------------------------

  line(delta) {
    const n = this.diff.rows.length;
    if (!n) return;
    this.cursorMoved = true;
    this.cursor = Math.min(n - 1, Math.max(0, this.cursor + delta));
    this.pendingReveal = { row: this.cursor, margin: diffs.ROW * 2 };
    this.publishCursor();
  }

  // J/K: the next or previous file of the list.
  // Presses before the list answered are dropped: each would ask "next"
  // from the same file (one ask slot), so holding J moved one file anyway.
  // (Bounded as the wait is: a dropped exec never gets to `done`.)
  nextFile(delta, cx) {
    if (!this.view || Date.now() - (this.stepping ?? 0) < 6000) return;
    this.stepping = Date.now();
    this.askList(cx, delta > 0 ? "next" : "prev", this.view, () => (this.stepping = 0));
  }

  // ---- actions --------------------------------------------------------------

  // The lines a page moves.
  page() {
    return Math.max(1, Math.floor((this.scroll.height || 360) / diffs.ROW) - 2);
  }

  // enter: on a fold unfolds it; else nvim (from Preview: at line 1, or
  // where the cursor was moved to in Diff).
  enterKey(cx) {
    if (this.inPreview()) return this.edit(this.cursorMoved ? null : 1, cx);
    if (this.diff.rows[this.cursor]?.k === "fold") return this.unfoldAll(cx);
    this.edit(null, cx);
  }

  edit(line = null, cx = null) {
    const v = this.view;
    if (!v) return;
    if (v.kind === "deleted") return this.flash.set(`${v.path} is deleted`);
    const o = { root: v.root, on: v.on, path: v.path, line: line ?? this.lineAt(this.cursor), tab: this.tab(), space: this.space(), view: v, closeDiff: this.place.closesForHerdrNvim };
    // In a task of its own: the editor it starts is looked for a moment after.
    if (cx) cx.spawn(async (cx) => openNvim({ ...o, cx }));
    else openNvim(o);
  }

  // s / u / x on the shown file (x asks first).
  act(op, cx) {
    const v = this.view;
    if (!v || v.commit) return;
    if (!allowed(op, v.section)) return this.flash.set(`can't ${op} it: it's ${v.scope}`);
    if (op === "discard") confirmDiscard("gitview-diff", v.path, { view: v, tab: this.tab() });
    else this.askList(cx, op, v);
  }

  // s/u/x and J/K: the tab's list (the one writer of its view) does them
  // (gitview.ask) and writes the view they lead to. Until it answers (5 s
  // at most) this counts as loading, so the next key acts on that view.
  askList(cx, op, v, done = null, tab = this.tab()) {
    if (!tab) return done?.();
    const seq = ask(tab, op, v);
    this.loads.run(cx, op, async (cx) => {
      try {
        for (let i = 0; i < 50 && (answerIn(tab)?.seq ?? 0) < seq; i++) await cx.sleep(100);
        const a = answerIn(tab);
        if (a?.seq === seq && a.msg) this.flash.set(a.msg);
        this.sync(cx);
      } finally {
        done?.();
      }
    });
  }

  // (Not `close`: the app calls that one when the pane goes.)
  closePane() {
    const pane = diffPane(this.tab());
    if (pane) run("pane.close", { pane: pane.pane });
  }

  // ---- notes ----------------------------------------------------------------

  notesKey() {
    return this.view ? notesKey(this.view.on, this.view.root) : null;
  }

  selection() {
    return this.anchor === null ? null : [Math.min(this.anchor, this.cursor), Math.max(this.anchor, this.cursor)];
  }

  // As the TUI: notes are about staged or unstaged changes, the lines as
  // they are now; not a commit's or the branch's.
  annotate() {
    const v = this.view;
    if (!v) return;
    if (this.inPreview()) return this.flash.set("notes work in Diff: m switches");
    if (v.commit || v.section === "branch") return this.flash.set("notes work on staged/unstaged changes only");
    const [a, b] = this.selection() ?? [this.cursor, this.cursor];
    const range = diffs.noteRange(this.diff, a, b);
    if (!range) return this.flash.set("select some code to note (v or drag)");
    this.pendingNote = { file: v.path, section: v.section, cached: noteSide(v), ...range, after: b };
    this.composer.set_value("");
    this.composer.focus();
    this.pendingReveal = { row: b, margin: diffs.COMPOSER + 8, after: true };
  }

  saveNote(value, cx) {
    const text = String(value ?? "").trim();
    const p = this.pendingNote;
    if (p && text) addNote(this.notesKey(), { file: p.file, section: p.section, cached: p.cached, start: p.start, end: p.end, snippet: p.snippet, text });
    this.pendingNote = null;
    this.anchor = null;
    this.focusHere(cx);
    cx.notify();
  }

  send(cx) {
    const key = this.notesKey();
    const space = this.space();
    this.loads.run(cx, "send", async () => {
      if (!(await sendNotes(key, space))) this.flash.set("no notes yet: select lines (v or drag), then a");
    });
  }

  // ---- keys -----------------------------------------------------------------

  key(e, cx) {
    const k = keyName(e);
    if (!k) return false;
    if (this.pendingNote) {
      if (k !== "escape") return false; // the composer types
      this.pendingNote = null;
      this.focusHere(cx);
      return true;
    }
    const act = actionFor([KEYS], k);
    if (!act) return false;
    act(this, cx);
    return true;
  }

  unfoldAll(cx) {
    this.unfold = true;
    if (this.view) this.loadDiff(cx, { keep: true });
  }

  command(command, cx) {
    const [what, ...rest] = command.split(":");
    if (command === "focus") takeKeyboard(this, cx);
    else if (command === "close") this.closePane(); // the × button, for scripts
    else if (what === "discard") {
      // The file the person confirmed, through its tab's list (this copy
      // may be another tab's).
      const p = takeDiscard("gitview-diff", command);
      if (p) this.askList(cx, "discard", p.view, null, p.tab);
    } else if (what === "key") this.key(keyEvent(rest.join(":")), cx);
    else if (what === "drag") {
      // A mouse drag over new-side lines <a> to <b>, for scripts.
      const [a, b] = rest.map(Number).map((n) => this.diff.rows.findIndex((r) => r.n === n));
      if (a < 0 || b < 0) return this.flash.set(`no lines ${rest.join("-")}`);
      this.down(a, { click_count: 1, modifiers: {} }, cx);
      this.over(b, cx);
      this.up(cx);
    } else if (what === "note") {
      this.annotate();
      if (this.pendingNote) this.saveNote(rest.join(":"), cx);
    }
  }

  // ---- mouse ----------------------------------------------------------------

  down(index, e, cx) {
    this.focusHere(cx);
    this.flash.clear();
    if (this.diff.rows[index]?.k === "fold") return this.unfoldAll(cx);
    this.cursor = index;
    this.cursorMoved = true;
    if (e.click_count >= 2) {
      this.anchor = null;
      return this.edit(null, cx);
    }
    if (!e.modifiers?.shift) this.anchor = index;
    else if (this.anchor === null) this.anchor = index;
    this.dragging = true;
    this.publishCursor();
    cx.notify();
  }

  over(index, cx) {
    if (!this.dragging || this.cursor === index) return;
    this.cursor = index;
    cx.notify();
  }

  up(cx) {
    if (!this.dragging) return;
    this.dragging = false;
    if (this.anchor === this.cursor) this.anchor = null; // a click, not a drag
    this.publishCursor();
    cx.notify();
  }

  // ---- view -----------------------------------------------------------------

  // What a render notices, woken in the kick task (never work from render).
  renderLook() {
    if ((viewIn(this.tab())?.seq ?? 0) !== this.seq || isDark(theme().background) !== this.dark || this.loads.panesMoved()) this.kick();
  }

  // What the body shows: {kind: "diff" | "preview" | null, old}: the shown
  // file's, or while it's on its way the one shown before (`old`).
  bodyNow() {
    if (this.ready()) return { kind: this.inPreview() ? "preview" : "diff", old: false };
    return { kind: this.lastBody, old: true };
  }

  // What the pane draws now, for the trace (Tracer.drew).
  traceFields() {
    const v = this.view;
    const b = this.bodyNow();
    return {
      tab: this.tab(),
      focused: this.handle.is_focused(),
      ring: Boolean(this.myPane()) && focused()?.pane === this.myPane(),
      seq: this.seq,
      file: v ? `${v.section}:${v.path}${v.commit ? `@${v.commit.slice(0, 7)}` : ""}` : null,
      shown: viewIn(this.tab())?.path ?? null,
      body: !v ? "empty-state"
        : b.kind === "preview" ? "content"
        : b.kind === "diff" ? (this.diff.rows.length ? "content" : this.diff.empty || this.diff.binary ? "empty-state" : "blank")
        : "blank",
      old: b.old,
      mode: v && previewKind(v) ? (this.inPreview() ? "preview" : "diff") : "diff",
      spinner: this.loads.shown(),
      header: v ? `${v.letter ?? ""} ${v.path} +${v.add ?? 0} -${v.del ?? 0} ${v.scope}` : "Diff",
    };
  }

  render(cx) {
    return this.tr.render(() => this.draw(cx));
  }

  draw(cx) {
    const t = theme();
    this.renderLook();
    const focused = this.handle.is_focused();
    const key = this.notesKey();
    // The body shows the shown file, or (just switched) the one before.
    const b = this.bodyNow();
    const of = b.old ? (b.kind === "preview" ? this.preview?.view : this.diffView) : this.view;
    // A note's card shows on the side it was written on (notes from before
    // `cached` are the unstaged side's).
    const side = noteSide(of);
    const notes = key && of && side !== null ? notesOf(key).filter((n) => n.file === of.path && Boolean(n.cached) === side) : [];
    // The display rows and their heights only when what they're made of
    // changed (a render comes on every wheel tick and any shared write).
    const dispKey = [this.diff, notes.map((n) => `${n.id}:${n.text.length}`).join(), this.pendingNote ? this.pendingNote.after : -1];
    if (!this.dispKey || dispKey.some((k, i) => k !== this.dispKey[i])) {
      this.display = diffs.displayRows(this.diff, notes, dispKey[2]);
      this.scroll.measure(this.display.map(diffs.rowSize));
      this.dispKey = dispKey;
    }
    if (this.pendingAnchor) {
      // The kept line back where it was on screen.
      const a = this.pendingAnchor;
      this.pendingAnchor = null;
      const at = this.display.findIndex((d) => d.index === a.row);
      if (at >= 0) {
        this.scroll.top = this.scroll.offsets[at] - a.offset;
        this.scroll.clamp();
      }
    }
    if (this.pendingReveal) {
      // Rows and their display places now agree: scroll to the one asked for.
      const r = this.pendingReveal;
      this.pendingReveal = null;
      const at = Math.max(0, this.display.findIndex((d) => d.index === r.row)) + (r.after ? 1 : 0);
      this.scroll.reveal(at, r);
    }
    const ctx = {
      t,
      cursor: this.cursor,
      selection: this.selection(),
      focused,
      composer: this.composer,
      pending: this.pendingNote,
      uiFont: settings().ui_font ?? "Geist",
      onDown: (i, e, cx) => this.down(i, e, cx),
      onMove: (i, cx) => this.over(i, cx),
      onDeleteNote: (n, cx) => { removeNotes(key, [n.id]); cx.notify(); },
    };
    const display = this.display;
    const rich = Boolean(this.view && previewKind(this.view));
    const preview = this.inPreview();
    const body = (!this.view
      ? v_flex().flex_1().items_center().justify_center().gap(4).text_color(t.dim)
          .child(div().child("No file selected"))
          .child(div().text_size(12).child("Click a file in the gitview list."))
      : b.kind === "preview"
        ? previewBody(this.preview, { t, uiFont: settings().ui_font ?? "Geist", narrow: this.width < 560 })
        : b.kind === "diff"
      ? div()
          .flex_1()
          .min_h(0)
          .flex()
          .flex_col()
          .relative()
          .font_family(settings().font_family ?? "JetBrains Mono")
          .text_size(12)
          .child(this.scroll.view(t, (i) => diffs.renderRow(display[i], ctx)))
          .when(Boolean(this.diff.error || this.diff.empty || this.diff.binary), (e) =>
            e.child(div().absolute().top(10).px(14).text_color(this.diff.error ? t.ansi1 : t.dim).child(this.diff.error ? `Couldn't load the diff: ${this.diff.error}` : this.diff.binary ? "Binary file" : "No text changes")),
          )
      : div().flex_1() // a new pane's first file, on its way
    ).when(b.old && Date.now() - this.switchedAt > 150, (e) => e.opacity(0.55));
    this.tr.drew(() => this.traceFields());
    return v_flex()
      .id("gitview-diff")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .text_color(t.foreground)
      .text_size(13)
      .track_focus(this.handle)
      .on_mouse_down("left", (_e, cx) => { this.focusHere(cx); cx.notify(); })
      .on_mouse_up("left", (_e, cx) => this.up(cx))
      .on_size_change((size, cx) => {
        if (size.width === this.width) return;
        this.width = size.width;
        if (Math.abs((shared("gitview.paneWidth") ?? 0) - size.width) > 20) setShared("gitview.paneWidth", Math.round(size.width));
        cx.notify();
      })
      .on_key_down((e, cx) => {
        if (!this.key(e, cx)) return;
        this.tr.line("input", { key: e.key, shift: Boolean(e.modifiers?.shift), file: this.view?.path ?? null, cursor: this.cursor });
        this.flash.clear();
        cx.stop_propagation();
        cx.notify();
      })
      .child(diffHeader(t, this.view, {
        loading: this.loads.shown(),
        onClose: this.place.closeButton ? () => this.closePane() : null,
        rich: rich ? { preview, compact: this.width < 420, onPick: (on, cx) => { this.togglePreview(cx, on); cx.notify(); } } : null,
      }))
      .child(body)
      .child(footer(t, {
        flash: this.flash.text,
        chips: this.view ? chipsOf([this.pendingNote ? NOTE_KEYS : KEYS], this) : [], // nothing shown: no keys for it
        width: this.width,
        onKeys: (cx) => { KEYS.at(-1).on["?"](this, cx); },
        id: "gitview-diff-keys-button",
      }));
  }
}
