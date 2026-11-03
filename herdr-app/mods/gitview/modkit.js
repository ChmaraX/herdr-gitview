// What both gitview mods do the same way: count running loads (commands
// wait for them), a footer message that fades, the folder watch, the
// command loop and key names. Shared (linked; install copies it).

import { closeOverlay, nextCommand, notify, props, watchFolder } from "herdr";
import { message } from "./engine.js";

// Loads in flight. `run(cx, work)` counts one; `idle(cx)` resolves when none
// runs (at most 10 s, so a lost task never blocks the commands).
export class Loads {
  constructor(onError) {
    this.count = 0;
    this.waiters = [];
    this.onError = onError;
  }

  busy() {
    return this.count > 0;
  }

  run(cx, work) {
    this.count++;
    cx.spawn(async (cx) => {
      try {
        await work(cx);
      } catch (e) {
        this.onError?.(message(e));
      } finally {
        if (--this.count === 0) this.waiters.splice(0).forEach((resolve) => resolve());
        cx.notify();
      }
    });
  }

  idle(cx) {
    if (this.count === 0) return Promise.resolve();
    return Promise.race([new Promise((resolve) => this.waiters.push(resolve)), cx.sleep(10000)]);
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
        if (loads.busy()) console.warn(`${name}: ${command}: ran after waiting 10 s for ${loads.count} load(s)`);
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
      notify({ title: "gitview", body: message(e), level: "error" });
    } finally {
      if (overlay) closeOverlay(overlay);
    }
  });
}
