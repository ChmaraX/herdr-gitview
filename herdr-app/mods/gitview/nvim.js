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
// 3. Else a new nvim right of the diff pane (or of the focused pane), in the
//    repo; it closes when nvim exits and the diff is back. (The app's
//    `terminal.show` can't put a terminal in a mod pane, so the diff pane
//    can't become nvim itself.) In a remote space it runs on that machine.

import { layout, notify, run } from "herdr";
import { gv, message, onArgs } from "./engine.js";
import { diffPane } from "./shared.js";

// Terminals shown in the layout's blocks (an nvim panel, say).
function layoutTerminals(node, out = []) {
  if (node?.content?.terminal) out.push(node.content.terminal);
  for (const c of node?.children ?? []) layoutTerminals(c, out);
  return out;
}

// {root, on, path, line, tab, space, editor, own}
export async function openNvim(o) {
  try {
    const args = ["nvim", "--repo", o.root, "--path", o.path, "--line", String(o.line || 1)];
    if (o.tab) args.push("--tab", o.tab);
    if (o.space) args.push("--space", o.space);
    if (!o.on) args.push("--shown", layoutTerminals(layout()).join(","));
    const r = await gv([...args, ...onArgs(o.on)]);
    const pane = diffPane(o.tab);
    if (r.terminal === "herdr-nvim") {
      if (pane && !o.own) run("pane.close", { pane: pane.pane });
      return "herdr-nvim";
    }
    if (r.terminal) {
      run("pane.focus", { terminal: r.terminal });
      return r.terminal;
    }
    if (pane) run("pane.focus", { pane: pane.pane });
    const editor = o.editor?.length ? o.editor : ["nvim"];
    run("terminal.open", { argv: [...editor, `+${o.line || 1}`, `${o.root}/${o.path}`], folder: o.root, place: "right" });
    return "new";
  } catch (e) {
    notify({ title: "gitview", body: message(e), level: "error" });
    return null;
  }
}
