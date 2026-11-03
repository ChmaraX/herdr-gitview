// Key tables: what each key does, its keycap and its label, in one place,
// so the key handling, the footer's keycaps and the keys card (`?`) can't
// drift apart. Shared by the list and the diff pane (linked; install
// copies it).
//
// A table is a list of entries:
//   {caps: ["j", "k"], label: "move", rank?, when?(v), on: {j: run, down: run, …}}
// `on` maps key names (modkit.js keyName: "J" for shift-j, "enter", "?")
// to run(v, cx); `caps` are the keys the card and the chips show; `rank`
// makes it a footer chip (lowest first, as many as fit); `when(v)` hides
// the chip when it doesn't apply now; `label` may be `label(v)` (Enter's
// says the editor it starts).

import { closeOverlay, openOverlay, setShared, shared } from "herdr";

// The run for key `k` in the first table that has it, or null.
export function actionFor(tables, k) {
  for (const table of tables) for (const entry of table) if (entry.on[k]) return entry.on[k];
  return null;
}

const labelOf = (e, v) => (typeof e.label === "function" ? e.label(v) : e.label);

// The footer's keycaps: ranked entries that apply now, most useful first.
export function chipsOf(tables, v) {
  return tables
    .flat()
    .filter((e) => e.rank && (!e.when || e.when(v)))
    .sort((a, b) => a.rank - b.rank)
    .map((e) => ({ ...e, label: labelOf(e, v) }));
}

// The keys card's content, plain data for the dialog's props: [{title,
// keys: [{caps, label}]}].
export function cardOf(groups, v) {
  return groups.map(({ title, table }) => ({ title, keys: table.map((e) => ({ caps: e.caps, label: labelOf(e, v) })) }));
}

// The keys card (mod gitview-keys, a dialog): opens it, or closes it when
// it's open (it says so in shared "gitview.keys" while it is).
export function toggleKeys(title, groups, v) {
  const open = shared("gitview.keys");
  if (open?.overlay) {
    closeOverlay(open.overlay);
    setShared("gitview.keys", null);
    return;
  }
  const card = cardOf(groups, v);
  const rows = card.reduce((n, g) => n + 1 + g.keys.length, 0);
  openOverlay({ mod: "gitview-keys", kind: "dialog", width: 500, height: Math.min(640, 96 + rows * 24 + card.length * 12), props: { title, groups: card } });
}
