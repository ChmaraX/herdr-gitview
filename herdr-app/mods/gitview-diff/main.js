// gitview's diff pane: a full-height mod pane at the tab's right edge, left
// of the file list (the Changes panel), or the left half of cmd-g's gitview
// tab. Props: {tab} (or {space, own} in the gitview tab). It shows what the
// list put in shared "gitview.views"[tab] (shared.js): the diff as gitview's
// TUI draws it, with review notes under their lines.
//
// Keys: j/k (arrows, wheel, click) move the cursor; v or a drag selects; a
// writes a note on the selection (enter saves, esc cancels); enter opens
// nvim at the cursor (on a fold: unfolds); s/u/x stage, unstage, discard the
// file; J/K next/previous file of the list; p sends the notes; esc clears.
// Commands: `gitview-diff.key:<key>`, `gitview-diff.note:<text>`.

import { View, div } from "gpui-kit";
import { v_flex, TextareaState } from "gpui-base";
import { agents, nextCommand, notify, openOverlay, props, run, setShared, settings, spaces, theme, watchFolder } from "herdr";
import { gv, message, onArgs } from "./engine.js";
import { diffArgs, isDark, sameFile, scopeLabel, viewOf } from "./model.js";
import { addNote, diffPane, notesKey, notesOf, onScreen, ownTab, removeNotes, setCursor, setView, takeKeyboard, viewIn } from "./shared.js";
import { openNvim } from "./nvim.js";
import { Scroller } from "./scroller.js";
import * as diffs from "./diffview.js";
import { footer, header } from "./parts.js";

const EMPTY = { rows: [] };

export default class GitDiff extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.scroll = new Scroller("gitview-diff-rows");
    this.composer = TextareaState.new({ placeholder: "Note for the agent", rows: 3 });
    this.composer.on("submit", (_e, cx) => this.saveNote(this.composer.value(), cx));
    this.view = null; // what's shown (a view from the list)
    this.seq = 0;
    this.diff = EMPTY;
    this.cursor = 0;
    this.anchor = null;
    this.dragging = false;
    this.unfold = false;
    this.dark = isDark(theme().background);
    cx.timer.every(4000, (cx) => { if (this.view?.on && onScreen(this.tab())) this.reload(cx); });
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        await this.idle();
        this.command(command, cx);
        cx.notify();
      }
    });
  }

  tab() {
    const p = props() ?? {};
    return p.tab ?? (p.space ? ownTab(p.space) : null);
  }

  idle() {
    return this.loading ? new Promise((resolve) => (this.waiters ??= []).push(resolve)) : Promise.resolve();
  }

  flash(text) {
    this.flashText = text;
  }

  // ---- what to show -----------------------------------------------------------

  // A new view from the list (or J/K here): same file keeps the cursor,
  // another starts at its first change (or the note the list jumped to).
  take(view, cx) {
    this.seq = view?.seq ?? 0;
    const same = sameFile(view, this.view);
    this.view = view;
    if (!same) {
      this.diff = EMPTY;
      this.anchor = null;
      this.unfold = false;
      this.pendingNote = null;
      this.scroll.top = 0;
    }
    if (view) this.loadDiff(cx, { keep: same && !view.line, line: view.line });
    if (view && !view.on) this.watch(view.root, cx);
  }

  reload(cx) {
    if (this.view && !this.loading) this.loadDiff(cx, { keep: true });
  }

  loadDiff(cx, { keep, line }) {
    const view = this.view;
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        const diff = await gv(diffArgs(view, { dark: this.dark, unfold: this.unfold }));
        if (this.view !== view) return;
        this.diff = diff;
        const n = diff.rows.length;
        if (line) {
          const at = diff.rows.findIndex((r) => r.n === line);
          this.cursor = Math.max(0, at);
          this.scroll.reveal(this.displayIndex(this.cursor), { center: true });
        } else if (!keep) {
          const first = diff.rows.findIndex((r) => r.k === "add" || r.k === "del");
          this.cursor = Math.max(0, first);
          this.scroll.reveal(this.displayIndex(this.cursor), { center: true });
        }
        this.cursor = Math.min(this.cursor, Math.max(0, n - 1));
        if (this.anchor !== null) this.anchor = Math.min(this.anchor, Math.max(0, n - 1));
        this.publishCursor();
      } catch (e) {
        if (this.view === view) {
          this.diff = EMPTY;
          this.flash(message(e));
        }
      } finally {
        this.loading = false;
        (this.waiters ?? []).splice(0).forEach((resolve) => resolve());
        cx.notify();
      }
    });
  }

  // A local repo's diff follows saves (remote ones: the 4 s timer).
  watch(root, cx) {
    if (this.watched === root) return;
    this.watched = root;
    cx.spawn(async (cx) => {
      try {
        while (this.watched === root) {
          await watchFolder(root);
          if (this.watched === root && onScreen(this.tab())) this.reload(cx);
        }
      } catch {
        this.watched = null;
      }
    });
  }

  // Where the cursor is, for the list's Enter.
  lineAt(i) {
    const rows = this.diff.rows;
    for (let j = i; j < rows.length; j++) if (rows[j]?.n) return rows[j].n;
    for (let j = i; j >= 0; j--) if (rows[j]?.n) return rows[j].n;
    return this.diff.first_change ?? 1;
  }

  publishCursor() {
    const v = this.view;
    const tab = this.tab();
    if (!v || !tab || !this.diff.rows.length) return;
    const line = this.lineAt(this.cursor);
    const was = this.published;
    if (was && was.line === line && sameFile(was, v)) return;
    this.published = { root: v.root, on: v.on, path: v.path, section: v.section, commit: v.commit, line, first: this.diff.first_change ?? 1 };
    setCursor(tab, this.published);
  }

  // ---- moving -------------------------------------------------------------------

  displayIndex(rowIndex) {
    return Math.max(0, (this.display ?? []).findIndex((d) => d.index === rowIndex));
  }

  line(delta) {
    const n = this.diff.rows.length;
    if (!n) return;
    this.cursor = Math.min(n - 1, Math.max(0, this.cursor + delta));
    this.scroll.reveal(this.displayIndex(this.cursor), { margin: diffs.ROW * 2 });
    this.publishCursor();
  }

  // J/K: the next or previous file of the list.
  nextFile(delta) {
    const v = this.view;
    const files = v?.files ?? [];
    if (!files.length) return;
    const at = files.findIndex((f) => f.path === v.path && f.section === v.section);
    const next = files[Math.min(files.length - 1, Math.max(0, at + delta))];
    if (!next || at + delta < 0 || at + delta >= files.length) return this.flash(delta > 0 ? "last file" : "first file");
    setView(this.tab(), { ...v, ...next, scope: scopeLabel(next, v), base: next.section === "branch" ? v.base : null, by: "pane", line: null });
  }

  // ---- actions --------------------------------------------------------------

  edit(cx) {
    const v = this.view;
    if (!v) return;
    if (v.kind === "deleted") return this.flash(`${v.path} is deleted`);
    openNvim({ root: v.root, on: v.on, path: v.path, line: this.lineAt(this.cursor), tab: this.tab(), space: this.space(), editor: v.editor, own: Boolean(props()?.own) });
  }

  // The space of this pane's tab.
  space() {
    const tab = this.tab();
    return spaces().find((s) => s.tabs.some((x) => x.id === tab))?.id ?? null;
  }

  // s / u / x on the shown file, then show where it went (staged), or the
  // next file when it's gone.
  act(op, cx) {
    const v = this.view;
    if (!v || v.commit || v.section === "branch") return;
    const ok = { stage: ["changes", "conflicts"], unstage: ["staged"], discard: ["changes"] }[op];
    if (!ok.includes(v.section)) return this.flash(`can't ${op} it: it's ${v.scope}`);
    if (op === "discard") {
      openOverlay({
        mod: "confirm",
        kind: "dialog",
        modal: true,
        width: 420,
        height: 190,
        props: { title: "Discard changes?", detail: `${v.path}: this can't be undone.`, command: { name: "gitview-diff.discard" } },
      });
      this.pendingDiscard = v;
      return;
    }
    this.run(op, v, cx);
  }

  run(op, v, cx) {
    const tab = this.tab();
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        await gv([op, "--repo", v.root, ...onArgs(v.on), ...(op === "discard" ? ["--section", v.section] : []), v.path]);
        setShared("gitview.dirty", { root: v.root, on: v.on, at: Date.now() });
        const status = await gv(["status", "--repo", v.root, ...onArgs(v.on)]);
        const entries = status.sections.flatMap((s) => s.files.map((file) => ({ file, section: s })));
        const want = op === "stage" ? "staged" : op === "unstage" ? "changes" : null;
        const found = entries.find((e) => e.file.path === v.path && e.section.id === want) ?? entries.find((e) => e.file.path === v.path);
        const files = (v.files ?? []).filter((f) => !(f.path === v.path && f.section === v.section));
        if (found) {
          const entry = { path: found.file.path, orig: found.file.orig ?? null, kind: found.file.kind, letter: found.file.letter, add: found.file.add, del: found.file.del, cached: Boolean(found.section.cached), section: found.section.id };
          setView(tab, { ...viewOf(entry, v), files: [...files, entry], by: "pane", line: null });
        } else {
          const at = (v.files ?? []).findIndex((f) => f.path === v.path && f.section === v.section);
          const next = files[Math.min(at, files.length - 1)];
          if (next) setView(tab, { ...viewOf(next, v), files, by: "pane", line: null });
          else setView(tab, null);
        }
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      } finally {
        this.loading = false;
        cx.notify();
      }
    });
  }

  close() {
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

  annotate() {
    if (!this.view) return;
    const [a, b] = this.selection() ?? [this.cursor, this.cursor];
    const range = diffs.noteRange(this.diff, a, b);
    if (!range) return this.flash("select some code to note (v or drag)");
    this.pendingNote = { file: this.view.path, ...range, after: b };
    this.composer.set_value("");
    this.composer.focus();
    this.scroll.reveal(this.displayIndex(b) + 1, { margin: 8 });
  }

  saveNote(value, cx) {
    const text = String(value ?? "").trim();
    const p = this.pendingNote;
    if (p && text) addNote(this.notesKey(), { file: p.file, start: p.start, end: p.end, snippet: p.snippet, text });
    this.pendingNote = null;
    this.anchor = null;
    this.handle.focus();
    cx.notify();
  }

  cancelNote() {
    this.pendingNote = null;
    this.handle.focus();
  }

  send(cx) {
    const key = this.notesKey();
    const notes = key ? notesOf(key) : [];
    if (!notes.length) return this.flash("no notes yet: select lines (v or drag), then a");
    const space = this.space();
    cx.spawn(async () => {
      try {
        const { message: text } = await gv(["notes", JSON.stringify(notes.map(({ id, ...note }) => note))]);
        const height = Math.min(420, 140 + 32 * Math.max(1, agents().length));
        openOverlay({ mod: "gitview-agents", kind: "dialog", modal: true, width: 460, height, props: { message: text, count: notes.length, space, key, ids: notes.map((n) => n.id) } });
      } catch (e) {
        notify({ title: "gitview", body: message(e), level: "error" });
      }
    });
  }

  // ---- keys -----------------------------------------------------------------

  key(e, cx) {
    const m = e.modifiers ?? {};
    if (m.platform || m.control || m.alt) return false;
    const k = m.shift && /^[a-z]$/.test(e.key) ? e.key.toUpperCase() : e.key;
    if (this.pendingNote) {
      if (k !== "escape") return false; // the composer types
      this.cancelNote();
      return true;
    }
    const page = Math.max(1, Math.floor((this.scroll.height || 360) / diffs.ROW) - 2);
    if (k === "j" || k === "down") this.line(1);
    else if (k === "k" || k === "up") this.line(-1);
    else if (k === "pagedown" || k === "space") this.line(page);
    else if (k === "pageup") this.line(-page);
    else if (k === "g" || k === "home") this.line(-Infinity);
    else if (k === "G" || k === "end") this.line(Infinity);
    else if (k === "v") this.anchor = this.anchor === null ? this.cursor : null;
    else if (k === "a") this.annotate();
    else if (k === "enter" && this.diff.rows[this.cursor]?.k === "fold") this.unfoldAll(cx);
    else if (k === "enter" || k === "e") this.edit(cx);
    else if (k === "s") this.act("stage", cx);
    else if (k === "u") this.act("unstage", cx);
    else if (k === "x") this.act("discard", cx);
    else if (k === "J") this.nextFile(1);
    else if (k === "K") this.nextFile(-1);
    else if (k === "p") this.send(cx);
    else if (k === "r") this.reload(cx);
    else if (k === "escape") this.anchor = null;
    else return false;
    return true;
  }

  unfoldAll(cx) {
    this.unfold = true;
    if (this.view) this.loadDiff(cx, { keep: true });
  }

  command(command, cx) {
    if (command === "focus") takeKeyboard(this, cx);
    else if (command === "discard") {
      const v = this.pendingDiscard;
      this.pendingDiscard = null;
      if (v && sameFile(v, this.view)) this.run("discard", v, cx);
    } else if (command.startsWith("key:")) {
      const k = command.slice(4);
      const one = k.length === 1;
      this.key({ key: one ? k.toLowerCase() : k, modifiers: { shift: one && k !== k.toLowerCase() } }, cx);
    } else if (command.startsWith("note:")) {
      this.annotate();
      if (this.pendingNote) this.saveNote(command.slice(5), cx);
    }
  }

  // ---- mouse ----------------------------------------------------------------

  down(index, e, cx) {
    this.handle.focus();
    this.flashText = null;
    if (this.diff.rows[index]?.k === "fold") {
      this.unfoldAll(cx);
      return;
    }
    if (e.click_count >= 2) {
      this.cursor = index;
      this.anchor = null;
      this.edit(cx);
      return;
    }
    if (e.modifiers?.shift) {
      if (this.anchor === null) this.anchor = this.cursor;
    } else this.anchor = index;
    this.cursor = index;
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
    const view = viewIn(this.tab());
    if ((view?.seq ?? 0) !== this.seq || (!view && this.view)) this.take(view, cx);
    if (isDark(t.background) !== this.dark) {
      this.dark = isDark(t.background);
      this.reload(cx);
    }
    const focused = this.handle.is_focused();
    const key = this.notesKey();
    const all = key ? notesOf(key) : [];
    const notes = this.view ? all.filter((n) => n.file === this.view.path) : [];
    this.display = diffs.displayRows(this.diff, notes, this.pendingNote ? this.pendingNote.after : -1);
    this.scroll.measure(this.display.map(diffs.rowSize));
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
          .when(Boolean(this.diff.empty || this.diff.binary) && !this.loading, (e) =>
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
        this.flashText = null;
        cx.stop_propagation();
        cx.notify();
      })
      .child(header(t, this.view, { loading: this.loading, notes: notes.length, onClose: props()?.own ? null : () => this.close() }))
      .child(body)
      .child(footer(t, this.flashText, Boolean(this.pendingNote)));
  }
}
