// The list's keys and commands. A key press and `gitview.key:<key>` take the
// same path; `gitview.file:`, `gitview.commit:`, `focus` (from the app) and
// the confirm dialog's reply (`discard`) are commands only.

// "J" for shift-j, as the commands spell it.
export function keyName(e) {
  const m = e.modifiers ?? {};
  return m.shift && /^[a-z]$/.test(e.key) ? e.key.toUpperCase() : e.key;
}

// Returns false for keys it leaves to the app.
export function handleKey(view, e, cx) {
  const m = e.modifiers ?? {};
  if (m.platform || m.control || m.alt) return false; // the app's shortcuts
  const k = keyName(e);
  const down = k === "j" || k === "down";
  const up = k === "k" || k === "up";
  if (k === "r") view.refresh(cx);
  else if (k === "p") view.send(cx);
  else if (view.mode === "files") {
    if (down) view.move(1, cx);
    else if (up) view.move(-1, cx);
    else if (k === "enter" || k === "e") view.enter(cx);
    else if (k === "left" || k === "h") view.fold(true, cx);
    else if (k === "right") view.fold(false, cx);
    else if (k === "s") view.rowAction("stage", null, cx);
    else if (k === "u") view.rowAction("unstage", null, cx);
    else if (k === "x") view.rowAction("discard", null, cx);
    else if (k === "c") view.commitInput.focus();
    else if (k === "w") view.toggleScope(cx);
    else if (k === "t") view.toggleTree(cx);
    else if (k === "l") view.openLog(cx);
    else if (k === "n") view.setMode("notes", cx);
    else if (k === "g") view.edge(false, cx);
    else if (k === "G") view.edge(true, cx);
    else return false;
  } else if (view.mode === "log") {
    if (down) view.logMove(1);
    else if (up) view.logMove(-1);
    else if (k === "enter" || k === "right") view.openCommit(view.log.commits[view.log.selected], cx);
    else if (k === "w") view.toggleLogFilter(cx);
    else if (k === "escape" || k === "h" || k === "left" || k === "l") view.setMode("files", cx);
    else return false;
  } else if (view.mode === "commit") {
    if (down) view.move(1, cx);
    else if (up) view.move(-1, cx);
    else if (k === "enter" || k === "e") view.enter(cx);
    else if (k === "escape" || k === "h" || k === "left") view.setMode("log", cx);
    else if (k === "t") view.toggleTree(cx);
    else return false;
  } else if (view.mode === "notes") {
    if (down) view.noteMove(1);
    else if (up) view.noteMove(-1);
    else if (k === "enter") view.jumpToNote(view.notes()[view.noteSel], cx);
    else if (k === "d" || k === "x") view.deleteNote(view.notes()[view.noteSel]);
    else if (k === "escape" || k === "h" || k === "left" || k === "n") view.setMode("files", cx);
    else return false;
  } else return false;
  return true;
}

// One `gitview.<command>` (keys.json, a script, the confirm dialog, the app).
// The same commands reach every gitview list on screen.
export function runCommand(view, command, cx) {
  if (command === "focus") view.takeKeyboard(cx);
  else if (command === "focus:panel") {
    if (!view.isPane()) view.takeKeyboard(cx);
  } else if (command === "discard") view.discardConfirmed(cx);
  else if (command.startsWith("key:")) {
    const k = command.slice(4);
    view.key({ key: k.length === 1 ? k.toLowerCase() : k, modifiers: { shift: k.length === 1 && k !== k.toLowerCase() } }, cx);
  } else if (command.startsWith("file:")) view.selectPath(command.slice(5), cx);
  // The mouse, for scripts: `click:<path>` / `dblclick:<path>` a file row,
  // `button:<stage|unstage|discard>:<path | folder/ | section:<id>>` a row's button.
  else if (command.startsWith("click:") || command.startsWith("dblclick:")) {
    const [what, ...rest] = command.split(":");
    const row = view.findRow(rest.join(":"));
    if (row) view.clickRow(row, { click_count: what === "dblclick" ? 2 : 1 }, cx);
  } else if (command.startsWith("button:")) {
    const [, op, ...rest] = command.split(":");
    const row = view.findRow(rest.join(":"));
    if (row) view.rowAction(op, row, cx);
  }
  else if (command.startsWith("commit:")) {
    view.commitInput.set_value(command.slice(7));
    view.commit(cx);
  }
}
