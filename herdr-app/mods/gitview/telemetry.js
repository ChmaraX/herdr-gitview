// The trace: what each gitview copy did, in order, with timings, for when
// something is off in the person's own app (its stdout goes nowhere). One
// JSON object per line, {t, mod, copy, tab, space, kind, ...}, kinds:
// input, exec, pane, state, render, warn, mark, snapshot. The lead reads it
// with herdr-app/tools/trace.py. Shared (linked; install copies it).
//
// Where: <appHome>/data/gitview/trace/. The app's fs has no append, so each
// copy rewrites its own segment file (at most 512 KB, then a new one) once a
// second when it has lines, and at once on a warn or a mark; past 40 files
// the oldest go (about 20 MB). The engine appends its own engine.jsonl
// there (GITVIEW_TRACE, engine.js).
//
// On unless shared "gitview.trace" is false (the gitview-tracing mod). No
// file contents, commit messages, note bodies or diff text: paths, keys
// and counts only.

import { appHome, panes, run as appRun, shared } from "herdr";
import * as fs from "fs/promises";

const SEGMENT = 512 * 1024;
const FILES = 40;
const SNAPSHOT_MS = 10000;
const SUMMARY_MS = 10000;
const SLOW_RENDER_MS = 16;

export const traceDir = () => `${appHome()}/data/gitview/trace`;
export const tracing = () => shared("gitview.trace") !== false;
const clock = typeof performance !== "undefined" && performance.now ? () => performance.now() : () => Date.now();
const round = (ms) => Math.round(ms * 10) / 10;

// The shared gitview records: their sizes, and the tab's own entries
// (views, handoff, editor, ask and answer hold paths, not text). Notes:
// a count. The commit message is never in shared.
const KEYS = ["views", "ask", "answer", "handoff", "editors", "cursor", "discards", "notes", "origins", "own", "changed", "ignored", "keys", "tree", "mark", "trace"];
export function sharedSnapshot(tab) {
  const sizes = {};
  const mine = {};
  for (const k of KEYS) {
    const v = shared(`gitview.${k}`);
    if (v == null) continue;
    sizes[k] = JSON.stringify(v).length;
    if (k === "notes") mine.notes = Object.fromEntries(Object.entries(v).map(([key, list]) => [key, Array.isArray(list) ? list.length : 0]));
    else if (k === "discards") mine.discards = Object.keys(v).length;
    else if (tab && v && typeof v === "object" && tab in v && ["views", "ask", "answer", "handoff", "editors", "cursor"].includes(k)) mine[k] = v[tab];
  }
  return { sizes, mine };
}

let current = null;
const NONE = { line() {}, warn(text) { console.warn(text); }, flush() {} };
// The copy shared helpers (gv, run, setHandoff) write through: the last
// one started in this mod.
export const tracer = () => current ?? NONE;

export class Tracer {
  // where(): {tab, space}; state(): the copy's own state for snapshots.
  constructor(mod, where, state = () => ({})) {
    this.mod = mod;
    this.copy = Math.random().toString(36).slice(2, 6);
    this.where = where;
    this.state = state;
    this.text = "";
    this.dirty = false;
    this.file = null;
    this.writing = null;
    this.renders = { n: 0, ms: 0, max: 0, at: Date.now() };
    this.snapAt = 0;
    this.markSeen = shared("gitview.mark")?.id ?? null;
    current = this;
  }

  // The copy's task: flushes each second, a render summary each 10 s, and
  // its part of a mark another copy made.
  start(cx) {
    cx.timer.every(1000, () => {
      const mark = shared("gitview.mark");
      if (mark?.id && mark.id !== this.markSeen) {
        this.markSeen = mark.id;
        this.line("mark", { id: mark.id, ...this.snapshot() });
      }
      this.summary();
      this.flush();
    });
  }

  line(kind, fields = {}) {
    if (!tracing()) return;
    let w = {};
    try { w = this.where() ?? {}; } catch { /* a copy starting */ }
    this.text += `${JSON.stringify({ t: Date.now(), mod: this.mod, copy: this.copy, tab: w.tab ?? null, space: w.space ?? null, kind, ...fields })}\n`;
    this.dirty = true;
  }

  // Times one render; a summary every 10 s, and a line for a slow one.
  render(draw) {
    if (!tracing()) return draw();
    const t0 = clock();
    try {
      return draw();
    } finally {
      const ms = clock() - t0;
      const r = this.renders;
      r.n++;
      r.ms += ms;
      r.max = Math.max(r.max, ms);
      if (ms > SLOW_RENDER_MS) this.line("render", { slow: round(ms) });
    }
  }

  summary(force = false) {
    const r = this.renders;
    if (!r.n || (!force && Date.now() - r.at < SUMMARY_MS)) return;
    this.line("render", { n: r.n, ms: round(r.ms), max: round(r.max), over: Date.now() - r.at });
    this.renders = { n: 0, ms: 0, max: 0, at: Date.now() };
  }

  snapshot() {
    let w = {};
    let state = {};
    try { w = this.where() ?? {}; state = this.state() ?? {}; } catch (e) { state = { error: String(e?.message ?? e) }; }
    const tab = w.tab ?? null;
    const all = panes().filter((p) => !tab || p.tab === tab).map((p) => ({ pane: p.pane, mod: p.mod ?? null, terminal: p.terminal ?? null, focused: Boolean(p.focused) }));
    return { panes: all, shared: sharedSnapshot(tab), state };
  }

  // console.warn as before, a warn line and (once per 10 s) a snapshot.
  warn(text, fields = {}) {
    console.warn(text);
    this.line("warn", { text, ...fields });
    if (Date.now() - this.snapAt > SNAPSHOT_MS) {
      this.snapAt = Date.now();
      this.line("snapshot", this.snapshot());
    }
    this.flush();
  }

  mark(id) {
    this.markSeen = id;
    this.line("mark", { id, ...this.snapshot() });
    this.summary(true);
    return this.flush();
  }

  // One write at a time; lines that come meanwhile go in the next.
  flush() {
    if (!this.dirty || this.writing) return this.writing;
    this.dirty = false;
    this.writing = (async () => {
      try {
        if (!this.file) {
          await fs.mkdir(traceDir(), { recursive: true });
          this.file = `${traceDir()}/${Date.now()}-${this.mod}-${this.copy}.jsonl`;
        }
        await fs.writeFile(this.file, this.text);
        if (this.text.length > SEGMENT) {
          this.file = null;
          this.text = "";
          await prune();
        }
      } catch (e) {
        console.warn(`gitview: trace: ${e?.message ?? e}`);
      } finally {
        this.writing = null;
        if (this.dirty) this.flush();
      }
    })();
    return this.writing;
  }
}

// The oldest segments go past FILES (names start with their start time).
async function prune() {
  const segs = (await fs.readdir(traceDir())).filter((f) => /^\d+-.*\.jsonl$/.test(f)).sort();
  for (const f of segs.slice(0, Math.max(0, segs.length - FILES))) await fs.unlink(`${traceDir()}/${f}`).catch(() => {});
}

// The app's run(), with the pane and layout ones traced (and their error).
export function run(name, args = {}) {
  const traced = /^(pane|layout)\./.test(name);
  try {
    const r = appRun(name, args);
    if (traced) tracer().line("pane", { cmd: name, ...args, ok: true });
    return r;
  } catch (e) {
    if (traced) tracer().line("pane", { cmd: name, ...args, ok: false, error: String(e?.message ?? e).split("\n")[0] });
    throw e;
  }
}

// A decision, for the trace (and, with gitview.debug, the app's log as
// GVEVENT, as before).
export function event(mod, fields) {
  if (shared("gitview.debug")) console.log(`GVEVENT ${JSON.stringify({ t: Date.now(), mod, ...fields })}`);
  tracer().line("state", { by: mod, ...fields });
}

// A command's args as traced: no text after "--" for the ops that send
// text (a commit message, notes, an agent's message); 300 chars at most.
const TEXT_OPS = new Set(["commit", "notes", "send"]);
export function shortArgs(args) {
  const cut = TEXT_OPS.has(args[0]) && args.includes("--") ? [...args.slice(0, args.indexOf("--")), "--", "…"] : args;
  const s = cut.join(" ");
  return s.length > 300 ? `${s.slice(0, 300)}…` : s;
}
