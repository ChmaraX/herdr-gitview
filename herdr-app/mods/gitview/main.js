// gitview for the Herdr app: the shown space's changed files and the selected
// file's diff, drawn natively, with herdr-gitview (the TUI's own engine) doing
// the git. It takes the place of the stock Changes panel (install.sh points the
// layout's `changes` block here); the gitview-sidebar and gitview-tab keys open
// it as a pane or a tab of its own, with props { cwd } fixing its repo.
//
// Keys (click the panel first): j/k move · enter edit in nvim · s stage ·
// u unstage · x discard · c write the commit message · w worktree/branch · tab to the diff and
// back · in the diff: j/k line · v select · a note · p send notes · esc back.
// The same keys as commands, for keys.json or scripts: `gitview.key:<key>`
// (e.g. `herdr-app do gitview.key:s`), `gitview.file:<path>` selects a file,
// `gitview.note:<text>` writes a note on the selected lines and
// `gitview.commit:<message>` commits. They reach every gitview on screen.

import { View, div } from "gpui-kit";
import { h_flex, v_flex, InputState, Scrollbar, TextareaState, v_virtual_list, VirtualListScrollHandle } from "gpui-base";
import { agents, layout, nextCommand, notify, openOverlay, props, run, settings, theme, watchFolder } from "herdr";
import { gv, location } from "./engine.js";
import * as chrome from "./chrome.js";
import { handleKey, runCommand } from "./keys.js";
import * as list from "./listview.js";
import * as diffs from "./diffview.js";

const EMPTY_DIFF = { rows: [] };

// Syntax colors: gitview's dark or light set, by the theme's background.
function isDark(hex) {
  const n = parseInt(String(hex).slice(1, 7), 16);
  if (Number.isNaN(n)) return true;
  return 0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255) < 128;
}

// Wide enough (a tab, a wide sidebar) for the list beside the diff.
const SIDE_BY_SIDE = 720;

// Terminals shown in the layout's blocks (an nvim panel, say).
function layoutTerminals(node, out = []) {
  if (node?.content?.terminal) out.push(node.content.terminal);
  for (const c of node?.children ?? []) layoutTerminals(c, out);
  return out;
}

export default class GitView extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.listScroll = VirtualListScrollHandle.new();
    this.diffScroll = VirtualListScrollHandle.new();
    this.composer = TextareaState.new({ placeholder: "Note for the agent · enter saves · esc cancels", rows: 3 });
    this.composer.on("submit", (_e, cx) => this.saveNote(cx, this.composer.value()));
    // The commit message, as in the stock Changes panel: enter commits.
    this.commitInput = InputState.new({ placeholder: "Commit message" });
    this.commitInput.on("change", (_e, cx) => cx.notify());
    this.commitInput.on("submit", (_e, cx) => this.commit(cx));
    this.commitInput.on("focus", (_e, cx) => { this.typing = true; cx.notify(); });
    this.commitInput.on("blur", (_e, cx) => { this.typing = false; cx.notify(); });
    this.width = 0;
    this.dark = isDark(theme().background);
    if (props()?.cwd) this.handle.focus(); // opened as a pane or tab: take the keyboard
    this.status = null;
    this.items = [];
    this.selected = null;
    this.diff = EMPTY_DIFF;
    this.cursor = 0;
    this.anchor = null;
    this.area = "list";
    this.notes = {}; // repo root → notes, kept while the panel lives
    this.scope = "worktree";
    this.noteSeq = 0;
    this.inflight = 0; // loads running; commands wait for them (see idle())
    this.waiters = [];
    this.refresh(cx);
    cx.timer.every(10000, (cx) => this.refresh(cx));
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        await this.idle(); // a command acts on what the one before it loaded
        runCommand(this, command, cx);
        cx.notify();
      }
    });
  }

  // ---- data ---------------------------------------------------------------

  // Runs an async load, counted so commands can wait for it.
  load(cx, work) {
    this.inflight++;
    cx.spawn(async (cx) => {
      try {
        await work(cx);
      } finally {
        if (--this.inflight === 0) this.waiters.splice(0).forEach((resolve) => resolve());
        cx.notify();
      }
    });
  }

  idle() {
    return this.inflight === 0 ? Promise.resolve() : new Promise((resolve) => this.waiters.push(resolve));
  }

  context() {
    const loc = location();
    if (props()?.cwd) loc.cwd = props().cwd; // a pane or tab of its own: its repo
    return { ...loc, key: JSON.stringify([loc.space, loc.cwd, loc.on, this.scope]) };
  }

  refresh(cx) {
    const ctx = this.context();
    this.shownKey = ctx.key; // another folder may still be the same repo
    if (this.loading) {
      this.again = true;
      return;
    }
    if (!ctx.cwd || ctx.on) {
      this.status = null;
      this.items = [];
      this.error = ctx.on ? `gitview runs on this computer; this space is on ${ctx.on}.` : "No folder";
      return;
    }
    this.loading = true;
    this.load(cx, async (cx) => {
      try {
        const previous = { key: this.selected, index: this.items.findIndex((i) => i.key === this.selected) };
        const status = await gv(["status", "--repo", ctx.cwd, "--scope", this.scope]);
        if (ctx.key !== this.shownKey) return;
        if (status.root !== this.status?.root || status.scope !== this.status?.scope) {
          this.selected = null;
          this.diff = EMPTY_DIFF;
          this.diffKey = null;
          this.anchor = null;
        }
        this.status = status;
        this.error = null;
        this.items = list.listRows(status);
        this.watch(status.root, cx);
        this.keepSelection(previous);
        await this.loadDiff();
      } catch (e) {
        if (ctx.key === this.shownKey) {
          this.status = null;
          this.items = [];
          this.error = String(e.message ?? e).includes("not inside a git repository") ? "Not a git repository" : String(e.message ?? e);
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

  // Live refresh while the repo changes (a save, a commit, staging elsewhere).
  watch(root, cx) {
    if (this.watched === root) return;
    this.watched = root;
    cx.spawn(async (cx) => {
      try {
        while (this.watched === root) {
          await watchFolder(root);
          if (this.watched === root) this.refresh(cx);
        }
      } catch {
        this.watched = null;
      }
    });
  }

  // After a refresh: the same row, else the same file in its new section
  // (just staged), else whatever is now where it was.
  keepSelection(previous) {
    const files = this.items.filter((i) => i.file);
    if (files.some((i) => i.key === previous.key)) return;
    const path = previous.key?.slice(previous.key.indexOf(":") + 1);
    const moved = files.find((i) => i.file.path === path);
    const near = this.items.slice(Math.max(0, previous.index)).find((i) => i.file) ?? files[files.length - 1];
    this.selected = (moved ?? near ?? null)?.key ?? null;
  }

  item() {
    return this.items.find((i) => i.key === this.selected) ?? null;
  }

  async loadDiff() {
    const item = this.item();
    if (!item) {
      this.diff = EMPTY_DIFF;
      return;
    }
    const { file, section } = item;
    const args = ["diff", "--repo", this.status.root, "--path", file.path, "--kind", file.kind, "--theme", this.dark ? "dark" : "light"];
    if (file.orig) args.push("--orig", file.orig);
    if (section.cached) args.push("--cached");
    if (this.status.base) args.push("--merge-base", this.status.base.merge_base, "--base", this.status.base.label);
    if (this.unfolded === item.key) args.push("--unfold");
    const key = item.key;
    const diff = await gv(args);
    if (this.selected !== key) return;
    const same = this.diffKey === key;
    this.diff = diff;
    this.diffKey = key;
    if (!same) {
      this.anchor = null;
      const first = diff.rows.findIndex((r) => r.k === "add" || r.k === "del");
      this.cursor = Math.max(0, first);
      this.diffScroll.scroll_to_item(this.cursor, "center");
    }
    this.cursor = Math.min(this.cursor, Math.max(0, diff.rows.length - 1));
  }

  select(key, cx) {
    if (key === this.selected) return;
    this.selected = key;
    this.anchor = null;
    const index = this.items.findIndex((i) => i.key === key);
    if (index >= 0) this.listScroll.scroll_to_item(index);
    this.load(cx, async () => {
      try { await this.loadDiff(); } catch (e) { this.flash(String(e.message ?? e)); }
    });
  }

  selectPath(path, cx) {
    const item = this.items.find((i) => i.file?.path === path);
    if (item) {
      this.area = "list";
      this.select(item.key, cx);
    }
    else this.flash(`${path} has no changes`);
  }

  move(delta, cx) {
    const files = this.items.filter((i) => i.file);
    if (!files.length) return;
    const at = files.findIndex((i) => i.key === this.selected);
    const next = files[Math.min(files.length - 1, Math.max(0, at + delta))];
    this.select(next.key, cx);
  }

  flash(text) {
    this.flashText = text;
  }

  // ---- actions --------------------------------------------------------------

  act(op, args, cx) {
    this.load(cx, async (cx) => {
      try {
        await gv([op, "--repo", this.status.root, ...args]);
      } catch (e) {
        notify({ title: "gitview", body: String(e.message ?? e), level: "error" });
      }
      this.refresh(cx);
    });
  }

  stage(on, cx) {
    const item = this.item();
    if (!item || this.scope !== "worktree") return;
    this.act(on ? "stage" : "unstage", [item.file.path], cx);
  }

  discard() {
    const item = this.item();
    if (!item || this.scope !== "worktree") return;
    this.pendingDiscard = { root: this.status.root, path: item.file.path };
    openOverlay({
      mod: "confirm",
      kind: "dialog",
      modal: true,
      width: 420,
      height: 190,
      props: { title: "Discard changes?", detail: `${item.file.path}: this can't be undone.`, command: { name: "gitview.discard" } },
    });
  }

  discardConfirmed(cx) {
    const p = this.pendingDiscard;
    this.pendingDiscard = null;
    if (!p || p.root !== this.status?.root) return;
    this.act("discard", [p.path], cx);
  }

  // The configured editor at the cursor's line: an nvim already open in this
  // tab or in the layout (herdr-nvim's panel) takes the file; else a new pane.
  edit(cx) {
    const item = this.item();
    if (!item || item.file.kind === "deleted") return;
    const row = this.diff.rows[this.cursor];
    const line = row?.n ?? this.diff.first_change ?? 1;
    const root = this.status.root;
    const { tab } = location();
    this.load(cx, async () => {
      try {
        const shown = layoutTerminals(layout()).join(",");
        const r = await gv(["nvim", "--repo", root, "--path", item.file.path, "--line", String(line), ...(tab ? ["--tab", tab] : []), "--shown", shown]);
        if (r.terminal === "herdr-nvim") return; // it focuses its nvim itself
        if (r.terminal) {
          run("pane.focus", { terminal: r.terminal });
          return;
        }
        const editor = this.status.editor ?? ["nvim"];
        run("terminal.open", { argv: [...editor, `+${line}`, `${root}/${item.file.path}`], folder: root, place: "right" });
      } catch (e) {
        notify({ title: "gitview", body: String(e.message ?? e), level: "error" });
      }
    });
  }

  // Staged changes, or everything when nothing is staged (as the stock
  // Changes panel commits).
  staged() {
    return Boolean(this.status?.sections.some((s) => s.id === "staged"));
  }

  commit(cx) {
    const message = this.commitInput.value().trim();
    if (!message || !this.status || this.scope !== "worktree") return;
    const all = !this.staged();
    this.load(cx, async (cx) => {
      try {
        const r = await gv(["commit", "--repo", this.status.root, ...(all ? ["--all"] : []), message]);
        this.commitInput.set_value("");
        this.handle.focus();
        notify({ title: "Committed", body: r.summary ?? message, level: "success" });
      } catch (e) {
        notify({ title: "Commit failed", body: String(e.message ?? e), level: "error" });
      }
      this.refresh(cx);
    });
  }

  toggleScope(cx) {
    this.scope = this.scope === "worktree" ? "branch" : "worktree";
    this.refresh(cx);
  }

  // ---- notes ----------------------------------------------------------------

  repoNotes() {
    return this.status ? (this.notes[this.status.root] ??= []) : [];
  }

  annotate() {
    const item = this.item();
    if (!item || this.scope !== "worktree") return;
    const [a, b] = this.anchor === null ? [this.cursor, this.cursor] : [Math.min(this.anchor, this.cursor), Math.max(this.anchor, this.cursor)];
    const range = diffs.noteRange(this.diff, a, b);
    if (!range) {
      this.flash("select some code to annotate");
      return;
    }
    this.pendingNote = { file: item.file.path, ...range };
    this.composer.set_value("");
    this.composer.focus();
  }

  saveNote(cx, value) {
    const text = value.trim();
    if (this.pendingNote && text) this.repoNotes().push({ ...this.pendingNote, text, id: String(++this.noteSeq) });
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
    const notes = this.repoNotes();
    if (!notes.length) {
      this.flash("no notes yet: v selects lines, a writes a note");
      return;
    }
    const { space } = location();
    this.load(cx, async () => {
      try {
        const { message } = await gv(["notes", JSON.stringify(notes.map(({ id, ...note }) => note))]);
        const height = Math.min(420, 140 + 32 * Math.max(1, agents().length));
        const ids = notes.map((n) => n.id).join(",");
        openOverlay({ mod: "gitview-agents", kind: "dialog", modal: true, width: 460, height, props: { message, count: notes.length, space, ids } });
      } catch (e) {
        notify({ title: "gitview", body: String(e.message ?? e), level: "error" });
      }
    });
  }

  // The notes that went out (ones written meanwhile stay).
  clearNotes(ids) {
    for (const root of Object.keys(this.notes)) this.notes[root] = this.notes[root].filter((n) => !ids.includes(n.id));
  }

  // ---- keys -----------------------------------------------------------------

  key(e, cx) {
    return handleKey(this, e, cx);
  }

  line(delta) {
    const n = this.diff.rows.length;
    if (!n) return;
    this.cursor = Math.min(n - 1, Math.max(0, this.cursor + delta));
    this.diffScroll.scroll_to_item(this.displayIndex(this.cursor));
  }

  unfold(cx) {
    this.unfolded = this.selected;
    this.load(cx, async () => {
      try { await this.loadDiff(); } catch (e) { this.flash(String(e.message ?? e)); }
    });
  }

  displayIndex(rowIndex) {
    return Math.max(0, this.display.findIndex((d) => d.index === rowIndex));
  }

  // ---- view -----------------------------------------------------------------

  render(cx) {
    if (this.context().key !== this.shownKey) this.refresh(cx);
    const t = theme();
    if (isDark(t.background) !== this.dark) {
      // Syntax colors come in a dark and a light set: a theme switch reloads.
      this.dark = isDark(t.background);
      if (this.status) this.load(cx, () => this.loadDiff());
    }
    const focused = this.handle.is_focused();
    const notes = this.status ? this.repoNotes().filter((n) => n.file === this.item()?.file.path) : [];
    this.display = diffs.displayRows(this.diff, notes);
    const wide = this.width >= SIDE_BY_SIDE;
    const files = this.filesView(t, focused, wide);
    const diff = this.diffView(t, focused);
    return v_flex()
      .id("gitview")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .text_color(t.foreground)
      .text_size(12.5)
      .track_focus(this.handle)
      .on_size_change((size, cx) => {
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
      .child(chrome.header(t, this.status, this.items.filter((i) => i.file).length, this.status ? this.repoNotes().length : 0, this.loading))
      .child(
        wide
          ? h_flex().flex_1().min_h(0).child(files.w(Math.max(260, this.width * 0.32)).h_full().border_r(1).border_color(t.border)).child(diff)
          : v_flex().flex_1().min_h(0).child(files.h(Math.min(260, Math.max(48, this.items.length * list.ROW + 12))).border_b(1).border_color(t.border)).child(diff),
      )
      .when(Boolean(this.pendingNote), (e) => e.child(chrome.composer(t, this.pendingNote, this.composer)))
      .when(this.scope === "worktree" && Boolean(this.status), (e) => e.child(chrome.commitBox(t, this.commitInput, this.typing, this.staged(), (cx) => this.commit(cx))))
      .child(chrome.footer(t, this.flashText, this.area));
  }

  filesView(t, focused, wide) {
    const items = this.items; // this frame's rows, whatever loads meanwhile
    return div()
      .relative()
      .flex_none()
      .child(
        v_virtual_list("gitview-files", items.length, list.ROW, (i) => items[i].key, (range) =>
          items.slice(range.start, range.end).map((item) => list.renderRow(item, { t, selected: this.selected, focused: focused && this.area === "list" })),
        )
          .track_scroll(this.listScroll)
          .on_item_click((key, cx) => {
            this.area = "list";
            this.handle.focus();
            if (!key.startsWith("h:")) this.select(key, cx);
            cx.notify();
          })
          .size_full(),
      )
      .child(Scrollbar.vertical("gitview-files").viewport_from_layout().absolute().inset_0())
      .when(!items.length, (e) => e.child(div().absolute().top(10).px(14).text_color(t.dim).child(this.error ?? (this.loading ? "Loading…" : "No changes"))))
      .when(wide, (e) => e.pt(4));
  }

  diffView(t, focused) {
    const selection = this.anchor === null ? null : [Math.min(this.anchor, this.cursor), Math.max(this.anchor, this.cursor)];
    const display = this.display;
    return div()
      .relative()
      .flex_1()
      .min_h(0)
      .min_w(0)
      .font_family(settings().font_family ?? "JetBrains Mono")
      .text_size(12)
      .child(
        v_virtual_list("gitview-diff", display.length, display.map(diffs.rowSize), (i) => (display[i].note ? `n${i}` : `r${display[i].index}`), (range) =>
          display.slice(range.start, range.end).map((item) => diffs.renderRow(item, { t, cursor: this.cursor, selection, focused: focused && this.area === "diff" })),
        )
          .track_scroll(this.diffScroll)
          .on_item_click((key, cx) => {
            this.area = "diff";
            this.handle.focus();
            if (key.startsWith("r")) {
              this.cursor = Number(key.slice(1));
              if (this.diff.rows[this.cursor]?.k === "fold") this.unfold(cx);
            }
            cx.notify();
          })
          .size_full(),
      )
      .child(Scrollbar.vertical("gitview-diff").viewport_from_layout().absolute().inset_0())
      .when(this.diff.empty || this.diff.binary, (e) => e.child(div().absolute().top(8).px(14).text_color(t.dim).child(this.diff.binary ? "Binary file" : "No text changes")));
  }
}
