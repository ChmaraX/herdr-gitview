// The actions both gitview mods offer, in one place: which section allows
// which op, the discard confirm, sending notes. Shared (linked; install
// copies it).

import { agents, openOverlay } from "herdr";
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
// `<mod>.discard:<token>`, and every copy of the mod hears it, so only the
// copy holding that token acts, once. Returns the pending discard to keep.
export function confirmDiscard(mod, what, payload) {
  const token = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  console.log(`${mod}: asking to discard ${what} (${mod}.discard:${token})`);
  openOverlay({
    mod: "confirm",
    kind: "dialog",
    modal: true,
    width: 420,
    height: 190,
    props: { title: "Discard changes?", detail: `${what}: this can't be undone.`, command: { name: `${mod}.discard:${token}` } },
  });
  return { ...payload, token };
}

// The pending discard a `discard:<token>` command confirms, or null.
export function takeDiscard(pending, command) {
  const token = command.slice("discard:".length);
  return pending && token && pending.token === token ? pending : null;
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
