// gitview's diff pane: a full-height mod pane at the tab's right edge, left
// of the file list (the Changes panel), or the left half of cmd-g's gitview
// tab. Props: {tab} (or {space, own} in the gitview tab; placement.js holds
// what differs). It shows what the
// list put in shared "gitview.views"[tab] (shared.js): the diff as gitview's
// TUI draws it, with review notes under their lines.
//
// Keys: j/k (arrows, wheel, click) move the cursor; v or a drag selects; a
// writes a note on the selection (enter saves, esc cancels); enter opens
// nvim at the cursor (on a fold: unfolds); s/u/x stage, unstage, discard the
// file; J/K next/previous file of the list; p sends the notes; esc clears.
// Commands: `gitview-diff.key:<key>`, `gitview-diff.note:<text>`,
// `gitview-diff.drag:<line>:<line>`, `gitview-diff.close`.

import { View, div } from "gpui-kit";
import { v_flex, TextareaState } from "gpui-base";
import { run, settings, spaces, theme, notify } from "herdr";
import { gv, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile, viewOf } from "./model.js";
import { allowed, confirmDiscard, sendNotes, takeDiscard } from "./actions.js";
import {
  addNote, cursorIn, diffPane, liveView, notesKey, notesOf, onScreen, pump, removeNotes, setCursor, setDirty, setView, takeKeyboard, viewIn,
} from "./shared.js";
import { Flash, Loads, commandLoop, keyEvent, keyName, watch } from "./modkit.js";
import { panePlacement } from "./placement.js";
import { openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import { diffHeader, footer } from "./chrome.js";
import * as diffs from "./diffview.js";

const EMPTY = { rows: [] };
const HINTS = "j/k · v or drag select · a note · enter nvim · s/u/x · J/K file · p send · esc";

// Notes are written on staged or unstaged changes, as the TUI's: the side
// (cached or not) of a view that takes them, null for a commit's or the
// branch's diff.
const noteSide = (v) => (!v || v.commit || v.section === "branch" ? null : Boolean(v.cached));

export default class GitDiff extends View {
  init(_props, cx) {
    this.place = panePlacement();
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
    this.dark = isDark(theme().background);
    this.flash = new Flash(cx);
    this.loads = new Loads((m) => this.flash.set(m));
    this.watcher = {};
    // What render notices (a new view, a theme switch) loads from here.
    this.kick = pump(cx, (cx) => this.sync(cx));
    this.sync(cx);
    cx.timer.every(4000, (cx) => { if (this.view?.on && onScreen(this.tab())) this.reload(cx); });
    commandLoop(cx, { loads: this.loads, flash: this.flash, name: "gitview-diff", before: (cx) => this.sync(cx), handle: (c, cx) => this.command(c, cx) });
  }

  tab() {
    return this.place.tab();
  }

  // The space of this pane's tab.
  space() {
    const tab = this.tab();
    return spaces().find((s) => s.tabs.some((x) => x.id === tab))?.id ?? null;
  }

  // ---- what to show -----------------------------------------------------------

  // A new view from the list (or J/K here), or a theme switch: same file
  // keeps the cursor, another starts at its first change (or at the note
  // the list jumped to). The view's `showing` says whether another file was
  // shown since (the list moved on and back while this pane wasn't drawn):
  // then this one starts afresh too.
  sync(cx) {
    if (isDark(theme().background) !== this.dark) {
      this.dark = !this.dark;
      this.reload(cx);
    }
    const stored = viewIn(this.tab());
    if ((stored?.seq ?? 0) === this.seq) return;
    const fresh = (stored?.showing ?? 0) !== this.showing;
    this.seq = stored?.seq ?? 0;
    this.showing = stored?.showing ?? 0;
    const view = liveView(stored);
    const same = !fresh && sameFile(view, this.view);
    this.view = view;
    if (!same) {
      this.diff = EMPTY;
      this.anchor = null;
      this.unfold = false;
      this.pendingNote = null;
      this.scroll.top = 0;
    }
    if (!view) return;
    this.loadDiff(cx, { keep: same && !view.line, line: view.line });
    if (!view.on) watch(this.watcher, view.root, cx, (cx) => { if (onScreen(this.tab())) this.reload(cx); });
    else this.watcher.root = null; // a local repo's watch ends
  }

  reload(cx) {
    if (this.view) this.loadDiff(cx, { keep: true });
  }

  // The shown view's diff, one load at a time: asked again meanwhile (the
  // repo changed again, another view), it loads once more afterwards, with
  // the asks merged (`keep`: the cursor stays; else it goes to `line`, or
  // the first change).
  loadDiff(cx, { keep, line = null }) {
    const asked = this.asked;
    this.asked = asked ? { keep: asked.keep && keep, line: line ?? asked.line } : { keep, line };
    this.loads.latest(cx, "diff", async () => {
      const view = this.view;
      const { keep, line } = this.asked ?? { keep: true, line: null };
      this.asked = null;
      if (!view) return;
      let diff;
      try {
        diff = await gv(diffArgs(view, { dark: this.dark, unfold: this.unfold }));
      } catch (e) {
        if (this.view === view) this.diff = EMPTY;
        throw e;
      }
      if (this.view !== view) return;
      this.diff = diff;
      const n = diff.rows.length;
      if (line || !keep) {
        const at = line ? diff.rows.findIndex((r) => r.n === line) : diff.rows.findIndex((r) => r.k === "add" || r.k === "del");
        this.cursor = Math.max(0, at);
        this.pendingReveal = { row: this.cursor, center: true }; // once `display` has these rows
      }
      this.cursor = Math.min(this.cursor, Math.max(0, n - 1));
      if (this.anchor !== null) this.anchor = Math.min(this.anchor, Math.max(0, n - 1));
      this.publishCursor();
    });
  }

  // The line under row i (a deleted line: the next line that's there).
  lineAt(i) {
    const rows = this.diff.rows;
    for (let j = i; j < rows.length; j++) if (rows[j]?.n) return rows[j].n;
    for (let j = i; j >= 0; j--) if (rows[j]?.n) return rows[j].n;
    return this.diff.first_change ?? 1;
  }

  // Where the cursor is, for the list's Enter.
  publishCursor() {
    const v = this.view;
    const tab = this.tab();
    if (!v || !tab || !this.diff.rows.length) return;
    const line = this.lineAt(this.cursor);
    const was = cursorIn(tab);
    if (was && was.line === line && was.row === this.cursor && sameFile(was, v)) return;
    setCursor(tab, { root: v.root, on: v.on, path: v.path, section: v.section, commit: v.commit, line, row: this.cursor, first: this.diff.first_change ?? 1 });
  }

  // ---- moving -------------------------------------------------------------------

  line(delta) {
    const n = this.diff.rows.length;
    if (!n) return;
    this.cursor = Math.min(n - 1, Math.max(0, this.cursor + delta));
    this.pendingReveal = { row: this.cursor, margin: diffs.ROW * 2 };
    this.publishCursor();
  }

  // J/K: the next or previous file of the list.
  nextFile(delta) {
    const v = this.view;
    const files = v?.files ?? [];
    const at = files.findIndex((f) => f.path === v?.path && f.section === v?.section);
    const next = at >= 0 ? files[at + delta] : null;
    if (!next) return this.flash.set(delta > 0 ? "last file" : "first file");
    setView(this.tab(), { ...viewOf(next, v), files: v.files, by: "pane", line: null });
  }

  // ---- actions --------------------------------------------------------------

  edit() {
    const v = this.view;
    if (!v) return;
    if (v.kind === "deleted") return this.flash.set(`${v.path} is deleted`);
    openNvim({ root: v.root, on: v.on, path: v.path, line: this.lineAt(this.cursor), tab: this.tab(), space: this.space(), editor: v.editor, closeDiff: this.place.closesForHerdrNvim });
  }

  // s / u / x on the shown file (x asks first).
  act(op, cx) {
    const v = this.view;
    if (!v || v.commit) return;
    if (!allowed(op, v.section)) return this.flash.set(`can't ${op} it: it's ${v.scope}`);
    if (op === "discard") this.pendingDiscard = confirmDiscard("gitview-diff", v.path, { view: v });
    else this.applyOp(op, v, cx);
  }

  // The op, and the repo is marked changed: the list refreshes and shows
  // here where the file went (staged), or the next file when it's gone.
  // Until it has (5 s at most), this counts as loading, so the next command
  // acts on that file, not on the one that moved.
  applyOp(op, v, cx) {
    const tab = this.tab();
    this.loads.run(cx, op, async (cx) => {
      const seq = viewIn(tab)?.seq ?? 0;
      try {
        await gv([op, "--repo", v.root, ...onArgs(v.on), "--section", v.section, "--", v.path]);
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
        return;
      } finally {
        setDirty(v.root, v.on);
      }
      for (let i = 0; i < 50 && (viewIn(tab)?.seq ?? 0) === seq; i++) await cx.sleep(100);
      this.sync(cx);
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
    this.handle.focus();
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
      this.handle.focus();
      return true;
    }
    const page = Math.max(1, Math.floor((this.scroll.height || 360) / diffs.ROW) - 2);
    const fold = this.diff.rows[this.cursor]?.k === "fold";
    const KEYS = {
      j: () => this.line(1), down: () => this.line(1), k: () => this.line(-1), up: () => this.line(-1),
      pagedown: () => this.line(page), space: () => this.line(page), pageup: () => this.line(-page),
      g: () => this.line(-Infinity), home: () => this.line(-Infinity), G: () => this.line(Infinity), end: () => this.line(Infinity),
      v: () => { this.anchor = this.anchor === null ? this.cursor : null; },
      a: () => this.annotate(),
      enter: () => (fold ? this.unfoldAll(cx) : this.edit()), e: () => this.edit(),
      s: () => this.act("stage", cx), u: () => this.act("unstage", cx), x: () => this.act("discard", cx),
      J: () => this.nextFile(1), K: () => this.nextFile(-1),
      p: () => this.send(cx), r: () => this.reload(cx),
      escape: () => { this.anchor = null; },
    };
    if (!KEYS[k]) return false;
    KEYS[k]();
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
      const p = takeDiscard(this.pendingDiscard, command);
      if (!p) return;
      this.pendingDiscard = null;
      if (sameFile(p.view, this.view)) this.applyOp("discard", p.view, cx);
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
    this.handle.focus();
    this.flash.clear();
    if (this.diff.rows[index]?.k === "fold") return this.unfoldAll(cx);
    this.cursor = index;
    if (e.click_count >= 2) {
      this.anchor = null;
      return this.edit();
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

  render(cx) {
    const t = theme();
    if ((viewIn(this.tab())?.seq ?? 0) !== this.seq || isDark(t.background) !== this.dark) this.kick();
    const focused = this.handle.is_focused();
    const key = this.notesKey();
    // A note's card shows on the side it was written on (notes from before
    // `cached` are the unstaged side's).
    const side = noteSide(this.view);
    const notes = key && side !== null ? notesOf(key).filter((n) => n.file === this.view.path && Boolean(n.cached) === side) : [];
    this.display = diffs.displayRows(this.diff, notes, this.pendingNote ? this.pendingNote.after : -1);
    this.scroll.measure(this.display.map(diffs.rowSize));
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
    const body = this.view
      ? div()
          .flex_1()
          .min_h(0)
          .flex()
          .flex_col()
          .relative()
          .font_family(settings().font_family ?? "JetBrains Mono")
          .text_size(12)
          .child(this.scroll.view(t, (i) => diffs.renderRow(display[i], ctx)))
          .when(Boolean(this.diff.empty || this.diff.binary) && !this.loads.busy(), (e) =>
            e.child(div().absolute().top(10).px(14).text_color(t.dim).child(this.diff.binary ? "Binary file" : "No text changes")),
          )
      : v_flex().flex_1().items_center().justify_center().gap(4).text_color(t.dim)
          .child(div().child("No file selected"))
          .child(div().text_size(12).child("Click a file in the gitview list."));
    return v_flex()
      .id("gitview-diff")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .text_color(t.foreground)
      .text_size(13)
      .track_focus(this.handle)
      .on_mouse_down("left", (_e, cx) => { this.handle.focus(); cx.notify(); })
      .on_mouse_up("left", (_e, cx) => this.up(cx))
      .on_key_down((e, cx) => {
        if (!this.key(e, cx)) return;
        this.flash.clear();
        cx.stop_propagation();
        cx.notify();
      })
      .child(diffHeader(t, this.view, { loading: this.loads.busy(), onClose: this.place.closeButton ? () => this.closePane() : null }))
      .child(body)
      .child(footer(t, this.flash.text, this.pendingNote ? "enter saves the note · esc cancels" : HINTS));
  }
}
