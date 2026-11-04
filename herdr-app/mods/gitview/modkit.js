// What both gitview mods do the same way: count running loads (commands
// wait for them), a footer message that fades, the folder watch, the
// command loop and key names. Shared (linked; install copies it).

import { closeOverlay, focused, nextCommand, notify, panes, props, watchFolder } from "herdr";
import { message } from "./engine.js";
import { shared } from "herdr";
import { changedAt, ignoredDirs, stampChanged, stampOf } from "./shared.js";

// At most one "gitview.changed" stamp per repo this often.
const STAMP_MS = 250;

// Loads in flight. `run(cx, what, work)` counts one; `latest(cx, what,
// work)` runs one at a time per name, coalescing; `idle(cx)` resolves when
// none runs. `exec` settles within 30 s, so a load that hasn't finished
// after LOST_MS is taken as lost (its task never came back) and no longer
// counts, so it can't hold the commands or the spinner for good; the log
// names it.
const LOST_MS = 35000;
// A load shows the spinner once it has run this long.
const SPIN_MS = 250;

export class Loads {
  constructor(onError) {
    this.running = new Map(); // id → {what, at}
    this.seq = 0;
    this.waiters = [];
    this.onError = onError;
    this.slots = new Map(); // what → {started, at, next}: latest()'s
    this.panes = new ModPanes(); // for revive: app bug #1 (a dropped exec)
  }

  // A mod pane went since the last look: the loads running are taken as
  // lost and loaded again (revive). Call where the copy notices things.
  panesClosed(cx) {
    if (this.panes.gone()) this.afterClose(cx);
  }

  // Whether panesClosed has something to see (no side effects).
  panesMoved() {
    return this.panes.changed();
  }

  live() {
    const now = Date.now();
    for (const [id, l] of this.running) {
      if (now - l.at > LOST_MS) {
        console.warn(`gitview: a load (${l.what}) hasn't finished in ${LOST_MS / 1000} s; not waiting for it`);
        this.running.delete(id);
      }
    }
    return this.running.size;
  }

  busy() {
    return this.live() > 0;
  }

  // Whether to show the spinner: a load that isn't quiet has run SPIN_MS
  // (the short ones never blink it).
  shown() {
    const now = Date.now();
    return [...this.running.values()].some((l) => !l.quiet && now - l.at >= SPIN_MS && now - l.at < LOST_MS);
  }

  // `what` names it in the log; `quiet`: never shows the spinner (a poll, a
  // reload of what's shown).
  run(cx, what, work, { quiet = false } = {}) {
    const id = ++this.seq;
    this.running.set(id, { what, at: Date.now(), quiet });
    if (!quiet) cx.spawn(async (cx) => { await cx.sleep(SPIN_MS + 10); if (this.running.has(id)) cx.notify(); });
    cx.spawn(async (cx) => {
      try {
        await work(cx);
      } catch (e) {
        this.onError?.(message(e));
      } finally {
        this.running.delete(id);
        if (!this.busy()) this.waiters.splice(0).forEach((resolve) => resolve());
        cx.notify();
      }
    });
  }

  // The newest `work` named `what`, one run at a time: asked again while
  // one runs, it runs once more afterwards (the last ask's work; the ones
  // between are dropped). `work(cx, current)`: `current()` turns false once
  // a later run started (after a lost one), so an older reply is ignored,
  // never written over a newer one.
  latest(cx, what, work, { quiet = false } = {}) {
    let slot = this.slots.get(what);
    if (!slot) this.slots.set(what, (slot = { started: 0, at: 0, next: null }));
    if (slot.at && Date.now() - slot.at < LOST_MS) {
      // Quiet only if every ask it stands for was.
      slot.nextQuiet = (slot.next ? slot.nextQuiet : true) && quiet;
      slot.next = work;
      return;
    }
    const seq = ++slot.started;
    slot.at = Date.now();
    slot.next = null;
    slot.work = work;
    this.run(cx, what, async (cx) => {
      try {
        await work(cx, () => slot.started === seq);
      } finally {
        if (slot.started === seq) {
          slot.at = 0;
          if (slot.next) this.latest(cx, what, slot.next, { quiet: slot.nextQuiet ?? false });
        }
      }
    }, { quiet });
  }

  // The loads started by `before` (ms) are taken as lost, and each such
  // `latest` one runs again (a late reply of the old run is ignored). For
  // when a mod pane closed: herdr-app 0.1.12 can drop an exec another copy
  // is waiting on then (the list's `status` never settled when the diff
  // pane closed for herdr-nvim's nvim), which held the commands for
  // LOST_MS. A pane leaves panes() a moment before its copy goes, so
  // `afterClose` looks again a moment later.
  revive(cx, before = Date.now()) {
    const lost = [...this.running].filter(([, l]) => l.at <= before);
    if (!lost.length) return;
    console.warn(`gitview: a mod pane closed during ${lost.map(([, l]) => l.what).join(", ")}: not waiting for that, loading again`);
    for (const [id] of lost) this.running.delete(id);
    for (const [what, slot] of this.slots) {
      if (!slot.at || slot.at > before) continue;
      const work = slot.next ?? slot.work;
      slot.at = 0;
      slot.next = null;
      if (work) this.latest(cx, what, work);
    }
    if (!this.busy()) this.waiters.splice(0).forEach((resolve) => resolve());
  }

  // A mod pane closed: revive now, and the loads started in the next
  // second that are still running 1.5 s after it.
  afterClose(cx) {
    const at = Date.now();
    this.revive(cx, at);
    cx.spawn(async (cx) => {
      await cx.sleep(1500);
      this.revive(cx, at + 1000);
      cx.notify();
    });
  }

  // `waiting()` is called about once a second while it waits.
  async idle(cx, waiting) {
    while (this.busy()) {
      await Promise.race([new Promise((resolve) => this.waiters.push(resolve)), cx.sleep(1000)]);
      if (this.busy()) waiting?.();
    }
  }

  names() {
    return [...this.running.values()].map((l) => l.what).join(", ");
  }
}

// While shared "gitview.debug" is set (every e2e run sets it), what a copy
// draws, one line in the app's log each time it changes (GVTRACE): for
// counting blank bodies, mode flips and spinner blinks without frames. A
// window that isn't drawn (started in the background) never renders, so a
// debug copy also looks every 25 ms from a timer (`debugLook`), doing what
// its render does: notice, and trace. Off: nothing, no timer.
// `fields()` is called only then: off, a render builds nothing for it.
export function trace(mod, fields, holder) {
  if (!shared("gitview.debug")) return;
  const key = JSON.stringify(fields());
  if (holder.lastTrace === key) return;
  holder.lastTrace = key;
  console.log(`GVTRACE ${JSON.stringify({ t: Date.now(), mod, ...JSON.parse(key) })}`);
}

export function debugLook(cx, look) {
  if (shared("gitview.debug")) cx.timer.every(25, () => look());
}

// work() at most once per `ms`, after the last kick (trailing): returns
// kick(). A task of the copy's own (made in init): render() only kicks it,
// never works itself; at 0 ms, any kicks before it runs are one run.
export function debounced(cx, ms, work) {
  let wake = null;
  let due = 0;
  cx.spawn(async (cx) => {
    for (;;) {
      if (!due) await new Promise((resolve) => (wake = resolve));
      wake = null;
      while (Date.now() < due) await cx.sleep(Math.max(10, due - Date.now()));
      due = 0;
      try {
        work(cx);
      } catch (e) {
        console.error(`gitview: ${e?.message ?? e}`);
      }
    }
  });
  return () => {
    due = Date.now() + ms;
    if (wake) wake();
  };
}

// Whether a mod pane went since the last look (any mod's: the app's drop
// of in-flight execs isn't limited to ours); `changed()` only peeks.
export class ModPanes {
  constructor() {
    this.ids = new Set(this.now());
  }

  now() {
    return panes().filter((p) => p.mod).map((p) => p.pane);
  }

  changed() {
    const now = new Set(this.now());
    return [...this.ids].some((id) => !now.has(id)) || now.size !== this.ids.size;
  }

  // True when one went; remembers the panes now.
  gone() {
    const now = new Set(this.now());
    const gone = [...this.ids].some((id) => !now.has(id));
    this.ids = now;
    return gone;
  }
}

// The footer's message: shown for 5 s, then the key hints again.
export class Flash {
  constructor(cx) {
    this.text = null;
    this.at = 0;
    cx.timer.every(2000, (cx) => {
      if (this.text && Date.now() - this.at > 5000) {
        this.text = null;
        cx.notify();
      }
    });
  }

  set(text) {
    this.text = text;
    this.at = Date.now();
  }

  clear() {
    this.text = null;
  }
}

// A watch batch's paths that can change what gitview shows: repo-relative,
// without git's object, log and lock writes or anything under the repo's
// ignored folders (macOS watches those too: a build's output); "all" when
// the batch names the root itself (the app's "refresh everything") or has
// more than 50.
export function batchPaths(root, batch) {
  const base = root.endsWith("/") ? root : `${root}/`;
  const ignored = ignoredDirs(root);
  const out = [];
  for (const p of batch ?? []) {
    if (p === root || p === base) return "all";
    if (!p.startsWith(base)) continue;
    const rel = p.slice(base.length);
    if (rel.startsWith(".git/objects/") || rel.startsWith(".git/logs/") || rel.endsWith(".lock")) continue;
    if (ignored.some((d) => rel.startsWith(d))) continue;
    out.push(rel);
  }
  return out.length > 50 ? "all" : out;
}

// Calls `changed(cx, paths)` whenever something that matters under `root`
// changes (`batchPaths`), until `watcher.root` is something else. A local
// repo only (no watch elsewhere). Only the newest copy waiting on a folder
// wakes (herdr-app 0.1.12): the one that wakes stamps shared
// "gitview.changed" for the repo, with the paths (at most once per
// STAMP_MS; a batch inside that waits for the next stamp, never dropped),
// and every copy follows the stamp too (`followStamp`). A refresh never
// stamps, so nothing loops.
export function watch(watcher, root, cx, changed) {
  if (watcher.root === root) return;
  watcher.root = root;
  watcher.seen = changedAt(root);
  cx.spawn(async (cx) => {
    try {
      while (watcher.root === root) {
        const paths = batchPaths(root, await watchFolder(root));
        if (paths !== "all" && !paths.length) continue; // nothing that matters
        const wait = STAMP_MS - (Date.now() - changedAt(root));
        if (wait > 0) await cx.sleep(wait);
        if (watcher.root !== root) break;
        const at = Date.now();
        stampChanged(root, at, paths);
        // `changed` returns false when it didn't act on it (its copy isn't
        // on screen): then the stamp stays unseen, so the copy follows it
        // once it shows (stale(), followStamp).
        if (changed(cx, paths) !== false) watcher.seen = at;
      }
    } catch {
      if (watcher.root === root) watcher.root = null;
    }
  });
}

// Another copy's watch stamped this watcher's repo since this copy looked.
export const stampMoved = (watcher) => Boolean(watcher.root) && changedAt(watcher.root) > (watcher.seen ?? 0);

// Once per stamp: the paths it names (`batchPaths`), "all" when a stamp in
// between was missed, null when there's none new.
export function followStamp(watcher) {
  if (!stampMoved(watcher)) return null;
  const s = stampOf(watcher.root);
  const missed = (watcher.seen ?? 0) < (s.prev ?? 0);
  watcher.seen = s.at;
  return missed ? "all" : s.paths;
}

// The mod's commands (`<mod>.<command>`), one at a time: `before(cx)`
// reads what changed meanwhile (and may start loading it), then, once no
// load runs, `handle` acts. An error is shown, never ends the loop. One task only takes the
// commands, so this copy is always waiting for the next one: with two
// copies on screen (the panel and cmd-g's list), a copy that was busy when
// a command came stopped getting any (herdr-app 0.1.12).
export function commandLoop(cx, { loads, before, handle, flash, name }) {
  const queue = [];
  let wake = null;
  cx.spawn(async () => {
    for (;;) {
      queue.push(await nextCommand());
      if (wake) wake();
    }
  });
  cx.spawn(async (cx) => {
    for (;;) {
      if (!queue.length) await new Promise((resolve) => (wake = resolve));
      wake = null;
      const command = queue.shift();
      try {
        // Focus doesn't wait for loads: the keys must arrive with it.
        if (command === "focus" || command.startsWith("focus:")) {
          handle(command, cx);
          cx.notify();
          continue;
        }
        before?.(cx);
        if (shared("gitview.debug")) console.log(`GVEVENT ${JSON.stringify({ t: Date.now(), mod: name, command, busy: loads.names() })}`);
        const asked = Date.now();
        let said = false;
        await loads.idle(cx, () => {
          if (said || Date.now() - asked < 3000) return;
          said = true;
          console.warn(`${name}: ${command} is waiting for ${loads.names()}`);
        });
        handle(command, cx);
        cx.notify();
      } catch (e) {
        console.error(`${name}: ${command}: ${message(e)}\n${e?.stack ?? ""}`);
        flash?.set(message(e));
      }
    }
  });
}

// "J" for shift-j, as commands spell it; null for chords left to the app.
export function keyName(e) {
  const m = e.modifiers ?? {};
  if (m.platform || m.control || m.alt) return null;
  if (m.shift && e.key === "/") return "?";
  return m.shift && /^[a-z]$/.test(e.key) ? e.key.toUpperCase() : e.key;
}

// `key:<k>` from a command, as a key event.
export function keyEvent(k) {
  const one = k.length === 1;
  return { key: one ? k.toLowerCase() : k, modifiers: { shift: one && k !== k.toLowerCase() } };
}

// A key's launcher (`mod.open:<mod>`, a 2×2 px frameless window): does
// `work(cx)` once, says what went wrong, then closes itself.
export function oneShot(cx, work) {
  const overlay = props()?.overlay;
  cx.spawn(async (cx) => {
    try {
      await work(cx);
    } catch (e) {
      console.error(`gitview: ${message(e)}`);
      notify({ title: "gitview", body: message(e), level: "error" });
    } finally {
      if (overlay) closeOverlay(overlay);
    }
  });
}

// Take the keyboard and make sure it stays: the app may hand it over a
// moment later than it draws the focus (a "focus" from pane.go, a click).
// Checks for a second, stops once held twice or when focus went elsewhere.
export function takeKeyboard(view, cx) {
  const where = JSON.stringify(focused());
  const arrival = (view.arrival = {});
  try { view.handle.focus(); } catch { /* not drawn yet */ }
  cx.spawn(async (cx) => {
    let held = 0;
    for (let step = 0; step < 10 && view.arrival === arrival; step++) {
      await cx.sleep(100);
      if (view.arrival !== arrival || JSON.stringify(focused()) !== where) break;
      if (view.handle.is_focused()) {
        if (++held >= 2) break;
        continue;
      }
      held = 0;
      try { view.handle.focus(); } catch { /* not drawn yet */ }
      cx.notify();
    }
    if (view.arrival === arrival) view.arrival = null;
  });
}
