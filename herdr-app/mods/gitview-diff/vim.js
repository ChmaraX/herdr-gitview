// Vim motions for the diff pane: an opt-in bundle, off by default. The
// footer's `vim` toggle (or shared "gitview.vim") turns it on; with it off
// the pane's keys are exactly KEYS in main.js.
//
// With it on, before KEYS:
//   5j 5k        a count before j/k/arrows moves that many rows
//   gg G         top / bottom; with a count (42G, 42gg) line 42
//   :42 enter    line 42; :set rnu / :set nornu relative numbers
//   ]c [c        next / previous change (with a count: that many)
//   ctrl-d/u     half a page down / up
//   zz zt zb     the cursor's line to the middle / top / bottom
//   0 $          sideways to the start / end of the lines
//   y            copy the selection (or the cursor's line), without markers
//   /text enter  search; n / N next / previous match
// Rows are the pane's rows (a removed line is a row, a fold is a row): the
// relative numbers in the gutter are what j/k counts.

import { setShared, shared } from "herdr";
import { ROW } from "./diffview.js";

export const vimOn = () => shared("gitview.vim") === true;
export const setVim = (on) => setShared("gitview.vim", on);
export const relativeOn = () => vimOn() && shared("gitview.vim.relative") !== false;

// ---- the rows (pure) --------------------------------------------------------

// The row showing file line n: its new-side number, else its old-side one (a
// removed line); a line folded away or outside the diff goes to the nearest
// one shown. -1 when no row has a number.
export function rowOfLine(rows, n) {
  let best = -1, gap = Infinity;
  rows.forEach((r, i) => {
    const at = r.n ?? r.o;
    if (at == null) return;
    const d = Math.abs(at - n) + (r.n == null ? 0.5 : 0);
    if (d < gap) { gap = d; best = i; }
  });
  return best;
}

const changed = (r) => Boolean(r) && r.k !== "ctx" && r.k !== "fold";

// The first row of the next (dir 1) or previous (dir -1) run of changed
// rows from `from`, or -1.
export function nextChange(rows, from, dir) {
  for (let i = from + dir; i >= 0 && i < rows.length; i += dir) {
    const start = changed(rows[i]) && !changed(rows[i - 1]);
    if (start) return i;
  }
  return -1;
}

// A row's code: its marker column dropped. Folds have none.
const code = (r) => (r.k === "fold" ? null : (r.m ?? "").slice(1));

// The next row (dir 1, -1) after `from` whose code holds `query`, wrapping;
// smart case (any capital: case-sensitive). -1 when none does.
export function searchRows(rows, query, from, dir) {
  if (!query || !rows.length) return -1;
  const exact = query !== query.toLowerCase();
  const holds = (r) => {
    const c = code(r);
    return c !== null && (exact ? c : c.toLowerCase()).includes(query);
  };
  for (let step = 1; step <= rows.length; step++) {
    const i = (((from + dir * step) % rows.length) + rows.length) % rows.length;
    if (holds(rows[i])) return i;
  }
  return -1;
}

// Rows a..b as text for the clipboard: each line's code, folds left out.
export function yankText(rows, a, b) {
  const out = [];
  for (let i = a; i <= b; i++) {
    const c = rows[i] ? code(rows[i]) : null;
    if (c !== null) out.push(c);
  }
  return out.length ? `${out.join("\n")}\n` : "";
}

// ---- the keys ---------------------------------------------------------------

const DIGIT = /^[0-9]$/;
// Shifted keys by what they type; `key_char` says it, this is for a
// platform that leaves it out.
const SHIFTED = { ";": ":", 4: "$" };
const charOf = (e, k) => e.key_char ?? (e.modifiers?.shift ? SHIFTED[e.key] : null) ?? k;

// The bundle's key state for one pane. `key(v, e, k, cx)` answers true when
// it took the key; `v` is the pane (main.js): rows, cursor, line(delta),
// page(), scroll, display, selection(), inPreview(), ready().
export class Vim {
  constructor() {
    this.reset();
    this.query = ""; // the last search
  }

  reset() {
    this.count = ""; // a count being typed (the 5 of 5j)
    this.prefix = null; // the first key of gg ]c zz …
    this.typing = null; // {lead: ":" | "/", text} while a command line is typed
    this.said = null; // a message for the footer until the next key
  }

  // What the footer shows: the command line, a count or prefix, a message.
  prompt() {
    if (this.typing) return `${this.typing.lead}${this.typing.text}`;
    if (this.count || this.prefix) return `${this.count}${this.prefix ?? ""}`;
    return this.said;
  }

  key(v, e, k, cx) {
    this.said = null;
    const m = e.modifiers ?? {};
    if (this.typing) return m.platform || m.alt || m.control ? false : this.typeKey(v, e, k);
    if (m.platform || m.alt) return false;
    if (v.inPreview() || !v.ready()) return false;
    if (m.control) {
      if (e.key !== "d" && e.key !== "u") return false;
      this.half(v, e.key === "d" ? 1 : -1);
      return true;
    }
    const ch = charOf(e, k);
    if (this.prefix) return this.chord(v, ch);
    if (ch === ":" || ch === "/") {
      this.count = "";
      this.typing = { lead: ch, text: "" };
      return true;
    }
    if (k && DIGIT.test(k) && (k !== "0" || this.count)) {
      this.count = (this.count + k).slice(0, 6);
      return true;
    }
    const count = this.count ? Number(this.count) : null;
    if (ch === "g" || ch === "]" || ch === "[" || ch === "z") {
      this.prefix = ch;
      return true; // the count waits for the second key
    }
    this.count = "";
    const rows = v.diff.rows;
    switch (ch) {
      case "j": case "down": return v.line(count ?? 1), true;
      case "k": case "up": return v.line(-(count ?? 1)), true;
      case "G": return count ? this.goLine(v, count) : v.line(Infinity), true;
      case "0": return (v.scroll.left = 0), true;
      case "$": return (v.scroll.left = v.scroll.range()), v.scroll.clampX(), true;
      case "n": case "N": return this.find(v, ch === "n" ? 1 : -1), true;
      case "y": {
        const [a, b] = v.selection() ?? [v.cursor, v.cursor];
        const text = yankText(rows, a, b);
        if (text) cx.write_to_clipboard(text);
        const lines = text ? text.split("\n").length - 1 : 0;
        this.said = `copied ${lines} line${lines === 1 ? "" : "s"}`;
        v.anchor = null;
        return true;
      }
      default: return count !== null; // a count before another key: dropped with it
    }
  }

  // The second key of gg ]c [c zz zt zb.
  chord(v, ch) {
    const first = this.prefix, count = this.count ? Number(this.count) : null;
    this.prefix = null;
    this.count = "";
    if (first === "g" && ch === "g") return count ? this.goLine(v, count) : v.line(-Infinity), true;
    if ((first === "]" || first === "[") && ch === "c") {
      let at = v.cursor;
      for (let i = 0; i < (count ?? 1); i++) {
        const next = nextChange(v.diff.rows, at, first === "]" ? 1 : -1);
        if (next < 0) break;
        at = next;
      }
      if (at === v.cursor) this.said = first === "]" ? "no later change" : "no earlier change";
      else v.line(at - v.cursor);
      return true;
    }
    if (first === "z" && (ch === "z" || ch === "t" || ch === "b")) return this.place(v, ch), true;
    return true; // an unknown chord is dropped, as vim does
  }

  // A key while `:` or `/` is typed: text, backspace, enter, escape.
  typeKey(v, e, k) {
    const t = this.typing;
    if (k === "escape") this.typing = null;
    else if (k === "backspace") {
      if (t.text) t.text = t.text.slice(0, -1);
      else this.typing = null;
    } else if (k === "enter") {
      this.typing = null;
      if (v.inPreview() || !v.ready()) return true;
      if (t.lead === "/") {
        if (t.text) this.query = t.text;
        this.find(v, 1);
      } else this.command(v, t.text.trim());
    } else if (e.key_char && e.key_char.length === 1) t.text = (t.text + e.key_char).slice(0, 200);
    return true; // the command line takes every plain key
  }

  // `:` commands: a line number, `$`, and set (no)rnu / (no)nu.
  command(v, text) {
    if (/^\d+$/.test(text)) return this.goLine(v, Number(text));
    if (text === "$") return v.line(Infinity);
    const set = /^set?\s+(no)?(rnu|relativenumber|nu|number)(!)?$/.exec(text);
    if (set) {
      const relative = set[2].startsWith("r");
      const on = set[3] ? !relativeOn() : relative ? !set[1] : Boolean(set[1]);
      setShared("gitview.vim.relative", on);
      return;
    }
    if (text) this.said = `not a command: ${text}`;
  }

  goLine(v, n) {
    const at = rowOfLine(v.diff.rows, n);
    if (at >= 0) v.line(at - v.cursor);
  }

  find(v, dir) {
    if (!this.query) return (this.said = "no search yet: /text");
    const at = searchRows(v.diff.rows, this.query, v.cursor, dir);
    if (at < 0) return (this.said = `not found: ${this.query}`);
    v.line(at - v.cursor);
    this.said = `/${this.query}`;
  }

  // ctrl-d / ctrl-u: the cursor and the view half a page.
  half(v, dir) {
    const rows = Math.max(1, Math.floor(v.page() / 2));
    const before = v.cursor;
    v.line(dir * rows);
    v.scroll.top += (v.cursor - before) * ROW;
    v.scroll.clamp();
  }

  // zz / zt / zb: scroll so the cursor's row is in the middle, at the top or
  // at the bottom.
  place(v, where) {
    const s = v.scroll;
    const at = v.display.findIndex((d) => d.index === v.cursor);
    if (at < 0 || !s.offsets[at + 1]) return;
    const a = s.offsets[at], b = s.offsets[at + 1];
    s.top = where === "t" ? a : where === "b" ? b - s.height : a - (s.height - (b - a)) / 2;
    s.clamp();
    v.pendingReveal = null;
  }
}
