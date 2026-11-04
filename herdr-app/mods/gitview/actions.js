// The actions both gitview mods offer, in one place: which section allows
// which op, the discard confirm, sending notes. Shared (linked; install
// copies it).

import { agents, openOverlay, setShared, shared } from "herdr";
import { gv } from "./engine.js";
import { notesOf } from "./shared.js";

// Where stage, unstage and discard apply (the engine checks it too).
const SECTIONS = { stage: ["changes", "conflicts"], unstage: ["staged"], discard: ["changes"] };

export function allowed(op, section) {
  return Boolean(SECTIONS[op]?.includes(section));
}

// The ops a row of that section offers, in button order.
export function opsFor(section) {
  return ["stage", "unstage", "discard"].filter((op) => allowed(op, section));
}

// Asks first (the app's confirm dialog). Its Confirm runs
// `<mod>.discard:<token>`, which reaches one copy of the mod, not always
// the one that asked (another tab's, while the app's focus lags): what the
// discard is goes in shared "gitview.discards" by token, so whichever copy
// hears it does it, once. A token without its record (cancelled long ago,
// or made up) does nothing.
const DISCARD_MS = 10 * 60 * 1000;
export function confirmDiscard(mod, what, payload) {
  const token = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  const now = Date.now();
  const all = Object.fromEntries(Object.entries(shared("gitview.discards") ?? {}).filter(([, r]) => now - r.at < DISCARD_MS));
  setShared("gitview.discards", { ...all, [token]: { mod, at: now, payload } });
  console.log(`${mod}: asking to discard ${what} (${mod}.discard:${token})`);
  openOverlay({
    mod: "confirm",
    kind: "dialog",
    modal: true,
    width: 420,
    height: 190,
    props: { title: "Discard changes?", detail: `${what}: this can't be undone.`, command: { name: `${mod}.discard:${token}` } },
  });
}

// The discard a `discard:<token>` command confirms (taken: it runs once),
// or null.
export function takeDiscard(mod, command) {
  const token = command.slice("discard:".length);
  const all = { ...(shared("gitview.discards") ?? {}) };
  const rec = token ? all[token] : null;
  if (!rec || rec.mod !== mod || Date.now() - rec.at > DISCARD_MS) return null;
  delete all[token];
  setShared("gitview.discards", all);
  return rec.payload;
}

// The send dialog for a repo's notes (`key`: shared.js notesKey). Resolves
// false when there are none.
export async function sendNotes(key, space) {
  const notes = key ? notesOf(key) : [];
  if (!notes.length) return false;
  // The engine takes each note's side (`cached`), not our id or section.
  const plain = notes.map(({ id, section, ...note }) => ({ ...note, cached: Boolean(note.cached) }));
  const { message } = await gv(["notes", "--", JSON.stringify(plain)]);
  const height = Math.min(420, 140 + 32 * Math.max(1, agents().length));
  openOverlay({ mod: "gitview-agents", kind: "dialog", modal: true, width: 460, height, props: { message, count: notes.length, space, key, ids: notes.map((n) => n.id) } });
  return true;
}
