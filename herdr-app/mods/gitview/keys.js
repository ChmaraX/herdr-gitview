// gitview's keys and commands. A key press and `gitview.key:<key>` take the
// same path; `gitview.file:`, `gitview.note:`, `gitview.commit:` and the
// dialogs' replies (`discard`, `sent:<ids>`) are commands only.

export function handleKey(view, e, cx) {
  const k = e.key;
  const m = e.modifiers ?? {};
  if (m.platform || m.control || m.alt) return false; // the app's shortcuts
  if (view.pendingNote) {
    if (k === "escape") view.cancelNote();
    else return false;
    return true;
  }
  if (k === "tab") view.area = view.area === "list" ? "diff" : "list";
  else if (k === "enter" && view.area === "diff" && view.diff.rows[view.cursor]?.k === "fold") view.unfold(cx);
  else if (k === "enter" || k === "e") view.edit(cx);
  else if (k === "w") view.toggleScope(cx);
  else if (k === "r") view.refresh(cx);
  else if (k === "p") view.send(cx);
  else if (k === "s") view.stage(true, cx);
  else if (k === "u") view.stage(false, cx);
  else if (k === "x") view.discard();
  else if (k === "c") view.commitInput.focus();
  else if (view.area === "list") {
    if (k === "j" || k === "down") view.move(1, cx);
    else if (k === "k" || k === "up") view.move(-1, cx);
    else if (k === "a") { view.area = "diff"; view.annotate(); }
    else return false;
  } else {
    if (k === "j" || k === "down") view.line(1);
    else if (k === "k" || k === "up") view.line(-1);
    else if (k === "v") view.anchor = view.anchor === null ? view.cursor : null;
    else if (k === "a") view.annotate();
    else if (k === "escape") { if (view.anchor !== null) view.anchor = null; else view.area = "list"; }
    else return false;
  }
  return true;
}

// One `gitview.<command>` (from keys.json, a script, or gitview's dialogs).
export function runCommand(view, command, cx) {
  if (command === "discard") view.discardConfirmed(cx);
  else if (command.startsWith("sent:")) view.clearNotes(command.slice(5).split(","));
  else if (command.startsWith("key:")) view.key({ key: command.slice(4), modifiers: {} }, cx);
  else if (command.startsWith("file:")) view.selectPath(command.slice(5), cx);
  else if (command.startsWith("commit:")) {
    view.commitInput.set_value(command.slice(7));
    view.commit(cx);
  } else if (command.startsWith("note:")) {
    view.annotate();
    if (view.pendingNote) view.saveNote(cx, command.slice(5));
  }
  view.flashText = command.startsWith("key:") ? null : view.flashText;
}
