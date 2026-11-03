// What both gitview mods do the same way: count running loads (commands
// wait for them), a footer message that fades, the folder watch, the
// command loop and key names. Shared (linked; install copies it).

import { closeOverlay, nextCommand, notify, panes, props, watchFolder } from "herdr";
import { message } from "./engine.js";
import { changedAt, stampChanged } from "./shared.js";

// At most one "gitview.changed" stamp per repo this often.
const STAMP_MS = 250;

// Loads in flight. `run(cx, what, work)` counts one; `latest(cx, what,
// work)` runs one at a time per name, coalescing; `idle(cx)` resolves when
// none runs. `exec` settles within 30 s, so a load that hasn't finished
// after LOST_MS is taken as lost (its task never came back) and no longer
// counts, so it can't hold the commands or the spinner for good; the log
// names it.
const LOST_MS = 35000;

export class Loads {
  constructor(onError) {
    this.running = new Map(); // id → {what, at}
    this.seq = 0;
    this.waiters = [];
    this.onError = onError;
    this.slots = new Map(); // what → {started, at, next}: latest()'s
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

  // `what` names it in the log.
  run(cx, what, work) {
    const id = ++this.seq;
    this.running.set(id, { what, at: Date.now() });
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
  latest(cx, what, work) {
    let slot = this.slots.get(what);
    if (!slot) this.slots.set(what, (slot = { started: 0, at: 0, next: null }));
    if (slot.at && Date.now() - slot.at < LOST_MS) {
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
          if (slot.next) this.latest(cx, what, slot.next);
        }
      }
    });
  }

  // Every load running now is taken as lost, and each `latest` one runs
  // again (a late reply of the old run is ignored). For when a mod pane
  // closed: herdr-app 0.1.12 can drop an exec another copy is waiting on
  // then (the list's `status` never settled when the diff pane closed for
  // herdr-nvim's nvim), which held the commands for LOST_MS.
  revive(cx) {
    if (!this.running.size) return;
    console.warn(`gitview: a mod pane closed during ${[...this.running.values()].map((l) => l.what).join(", ")}: not waiting for that, loading again`);
    this.running.clear();
    for (const [what, slot] of this.slots) {
      if (!slot.at) continue;
      const work = slot.next ?? slot.work;
      slot.at = 0;
      slot.next = null;
      if (work) this.latest(cx, what, work);
    }
    if (!this.busy()) this.waiters.splice(0).forEach((resolve) => resolve());
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

// Calls `changed(cx)` whenever something under `root` changes, until
// `watcher.root` is something else. A local repo only (no watch elsewhere).
// Only the newest copy waiting on a folder wakes (herdr-app 0.1.12): the
// one that wakes stamps shared "gitview.changed" for the repo (at most once
// per STAMP_MS; a batch inside that waits for the next stamp, never
// dropped), and every copy follows the stamp too (`followStamp`). A
// refresh never stamps, so nothing loops.
export function watch(watcher, root, cx, changed) {
  if (watcher.root === root) return;
  watcher.root = root;
  watcher.seen = changedAt(root);
  cx.spawn(async (cx) => {
    try {
      while (watcher.root === root) {
        await watchFolder(root);
        const wait = STAMP_MS - (Date.now() - changedAt(root));
        if (wait > 0) await cx.sleep(wait);
        if (watcher.root !== root) break;
        const at = Date.now();
        stampChanged(root, at);
        watcher.seen = at;
        changed(cx);
      }
    } catch {
      if (watcher.root === root) watcher.root = null;
    }
  });
}

// Another copy's watch stamped this watcher's repo since this copy looked.
export const stampMoved = (watcher) => Boolean(watcher.root) && changedAt(watcher.root) > (watcher.seen ?? 0);

// True (once) when the stamp moved: the repo changed, refresh it.
export function followStamp(watcher) {
  if (!stampMoved(watcher)) return false;
  watcher.seen = changedAt(watcher.root);
  return true;
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
        before?.(cx);
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
