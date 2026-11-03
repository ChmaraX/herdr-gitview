// Enter → nvim, from the list or the diff pane. Shared (linked; install
// copies it).
//
// 1. herdr-nvim installed: the engine runs `herdr-nvim open --tab <tab>
//    --space <space> <file>:<line>`; its full-height nvim pane takes the diff
//    pane's place at the tab's edge, so the diff pane closes (the next file
//    click opens it again). Not in a gitview tab of its own (cmd-g), where
//    the diff is half the tab.
// 2. An nvim already running in the tab (or in a layout block) takes the
//    file (the engine finds it; never on another machine).
// 3. Else the editor the engine picks on the repo's machine (`json editor`:
//    the config's, $VISUAL, $EDITOR, nvim, vim, vi; here, else the system's
//    opener), looked up once per repo and machine: a new terminal right of
//    the diff pane (or of the focused pane), in the repo, `+line` for nvim
//    and vim kinds; it closes when the editor exits and the diff is back.
//    The system's opener runs without a pane (no line). No editor at all: a
//    toast with the engine's message, no pane. (The app's `terminal.show`
//    can't put a terminal in a mod pane, so the diff pane can't become the
//    editor itself.) In a remote space it runs on that machine.

import { exec, layout, notify, run } from "herdr";
import { gv, message, onArgs } from "./engine.js";
import { diffPane, setHandoff, viewIn } from "./shared.js";

// Terminals shown in the layout's blocks (an nvim panel, say).
function layoutTerminals(node, out = []) {
  if (node?.content?.terminal) out.push(node.content.terminal);
  for (const c of node?.children ?? []) layoutTerminals(c, out);
  return out;
}

// The editor per "<machine>:<repo>": {at, editor: {argv, kind, name} |
// null, error}. A miss is asked again after 30 s.
const editors = new Map();
const editorKey = (root, on) => `${on ?? ""}:${root}`;

export async function editorFor(root, on) {
  const hit = editors.get(editorKey(root, on));
  if (hit && (hit.editor || Date.now() - hit.at < 30000)) return hit;
  const r = await gv(["editor", "--repo", root, ...onArgs(on)]);
  const found = { at: Date.now(), editor: r.editor ?? null, error: r.error ?? null };
  editors.set(editorKey(root, on), found);
  return found;
}

// Look the repo's editor up once, for Enter's label (redraws when known).
const asking = new Set();
export function knowEditor(root, on, cx) {
  const key = editorKey(root, on);
  if (!root || editors.has(key) || asking.has(key)) return;
  asking.add(key);
  cx.spawn(async (cx) => {
    try {
      await editorFor(root, on);
    } catch {
      // asked again on Enter
    } finally {
      asking.delete(key);
      cx.notify();
    }
  });
}

// What Enter starts there, for the keycaps and the keys card.
export const editorLabel = (root, on) => editors.get(editorKey(root, on))?.editor?.name ?? "editor";

// {root, on, path, line, tab, space, closeDiff, view}; `view`: the diff
// shown for the file (with closeDiff, the handoff remembers it: the diff
// comes back when nvim's pane goes).
export async function openNvim(o) {
  try {
    const args = ["nvim", "--repo", o.root, "--path", o.path, "--line", String(o.line || 1)];
    if (o.tab) args.push("--tab", o.tab);
    if (o.space) args.push("--space", o.space);
    if (!o.on) args.push("--shown", layoutTerminals(layout()).join(","));
    const r = await gv([...args, ...onArgs(o.on)]);
    const pane = diffPane(o.tab);
    if (r.terminal === "herdr-nvim") {
      if (o.closeDiff && o.view && o.tab) {
        const v = o.view;
        setHandoff(o.tab, { root: v.root, on: v.on ?? null, path: v.path, section: v.section, commit: v.commit ?? null, line: o.line || 1, seq: viewIn(o.tab)?.seq ?? 0, at: Date.now(), seen: false });
      }
      if (pane && o.closeDiff) run("pane.close", { pane: pane.pane });
      return "herdr-nvim";
    }
    if (r.terminal) {
      run("pane.focus", { terminal: r.terminal });
      return r.terminal;
    }
    const { editor, error } = await editorFor(o.root, o.on);
    if (!editor) {
      notify({ title: "gitview", body: error ?? "No editor found", level: "error" });
      return null;
    }
    const file = `${o.root}/${o.path}`;
    if (editor.kind === "system") {
      const opened = await exec(editor.argv[0], [...editor.argv.slice(1), file]);
      if (opened.code !== 0) throw new Error(`${editor.name}: ${(opened.stderr || `exit ${opened.code}`).trim()}`);
      return "system";
    }
    if (pane) run("pane.focus", { pane: pane.pane });
    const line = editor.kind === "nvim" || editor.kind === "vim" ? [`+${o.line || 1}`] : [];
    run("terminal.open", { argv: [...editor.argv, ...line, file], folder: o.root, place: "right" });
    return "new";
  } catch (e) {
    notify({ title: "gitview", body: message(e), level: "error" });
    return null;
  }
}
