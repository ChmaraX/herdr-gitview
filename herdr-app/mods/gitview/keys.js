// The list's keys and commands. A key press and `gitview.key:<key>` take the
// same path: the shown view's keys (modes.js), then the ones every view
// has. `gitview.file:`, `gitview.commit:`, the mouse for scripts, `focus`
// (from the app) and the confirm dialog's reply (`discard:<token>`) are
// commands only. Commands reach every gitview list on screen.

import { keyEvent, keyName } from "./modkit.js";
import { MODES } from "./modes.js";
import { takeKeyboard } from "./shared.js";

const EVERYWHERE = {
  r: (v, cx) => v.refresh(cx),
  p: (v, cx) => v.send(cx),
};

// Returns false for keys it leaves to the app.
export function handleKey(view, e, cx) {
  const k = keyName(e);
  const act = k && (MODES[view.mode].keys[k] ?? EVERYWHERE[k]);
  if (!act) return false;
  act(view, cx);
  return true;
}

export function runCommand(view, command, cx) {
  const [what, ...rest] = command.split(":");
  const arg = rest.join(":");
  if (command === "focus") takeKeyboard(view, cx);
  // cmd-shift-g showed the panel: the keyboard (its file opens as the panel
  // shows, placement.js).
  else if (command === "focus:panel") {
    if (view.place.panel) takeKeyboard(view, cx);
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
