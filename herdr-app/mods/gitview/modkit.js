// What both gitview mods do the same way: count running loads (commands
// wait for them), a footer message that fades, the folder watch, the
// command loop and key names. Shared (linked; install copies it).

import { closeOverlay, nextCommand, notify, props, watchFolder } from "herdr";
import { message } from "./engine.js";

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

  async idle(cx) {
    while (this.busy()) await Promise.race([new Promise((resolve) => this.waiters.push(resolve)), cx.sleep(1000)]);
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
export function watch(watcher, root, cx, changed) {
  if (watcher.root === root) return;
  watcher.root = root;
  cx.spawn(async (cx) => {
    try {
      while (watcher.root === root) {
        await watchFolder(root);
        if (watcher.root === root) changed(cx);
      }
    } catch {
      if (watcher.root === root) watcher.root = null;
    }
  });
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
        await loads.idle(cx);
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
