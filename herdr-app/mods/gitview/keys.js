// The list's keys and commands. A key press and `gitview.key:<key>` take the
// same path: the shown view's keys (modes.js), then the ones every view
// has. `gitview.file:`, `gitview.commit:`, the mouse for scripts, `focus`
// (from the app) and the confirm dialog's reply (`discard:<token>`) are
// commands only. Commands reach every gitview list on screen.

import { keyEvent, keyName } from "./modkit.js";
import { MODES } from "./modes.js";
import { actionFor, chipsOf, toggleKeys } from "./keymap.js";
import { takeKeyboard } from "./modkit.js";

// The keys every view has.
const EVERYWHERE = [
  { caps: ["p"], label: "send notes", rank: 9, on: { p: (v, cx) => v.send(cx) } },
  { caps: ["r"], label: "refresh", on: { r: (v, cx) => v.refresh(cx) } },
  { caps: ["?"], label: "all keys", on: { "?": (v) => showKeys(v) } },
];

const tables = (view) => [MODES[view.mode].keys, EVERYWHERE];

// Returns false for keys it leaves to the app.
export function handleKey(view, e, cx) {
  const k = keyName(e);
  const act = k && actionFor(tables(view), k);
  if (!act) return false;
  act(view, cx);
  return true;
}

// The footer's keycaps for the shown view.
export const chipsFor = (view) => chipsOf(tables(view), view);

// Every key of the list, the shown view's first.
export function showKeys(view) {
  const order = [view.mode, ...Object.keys(MODES).filter((m) => m !== view.mode)];
  toggleKeys("gitview: the file list", [...order.map((m) => ({ title: MODES[m].title, table: MODES[m].keys })), { title: "Everywhere", table: EVERYWHERE }], view);
}

export function runCommand(view, command, cx) {
  const [what, ...rest] = command.split(":");
  const arg = rest.join(":");
  // The app moved its focus here (pane.go, a popup over it closed).
  if (command === "focus") {
    view.viaApp = true;
    if (view.afterFocus === "input") {
      view.afterFocus = null;
      view.commitInput.focus();
    } else takeKeyboard(view, cx);
  }
  // cmd-shift-g showed the panel: the keyboard and the app's focus (its
  // file opens as the panel shows, placement.js).
  else if (command === "focus:panel") {
    if (view.place.panel) view.focusList(cx);
  } else if (what === "discard") view.discardConfirmed(command, cx);
  else if (what === "key") view.key(keyEvent(arg), cx);
  else if (what === "file") view.selectPath(arg, cx);
  else if (what === "commit") {
    view.commitInput.set_value(arg);
    view.commit(cx);
  }
  // The mouse, for scripts: `click:<path>` / `dblclick:<path>` a file row,
  // `button:<stage|unstage|discard>:<path | folder/ | section:<id>>` a row's button.
  else if (what === "click" || what === "dblclick") {
    const row = view.list()?.find(arg);
    if (row) view.clickRow(row, { click_count: what === "dblclick" ? 2 : 1 }, cx);
  } else if (what === "button") {
    const [op, ...target] = rest;
    const row = view.list()?.find(target.join(":"));
    if (row) view.rowAction(op, row, cx);
  }
}
