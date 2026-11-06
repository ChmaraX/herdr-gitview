// The editors gitview started (a plain terminal right of the diff pane, when
// herdr-nvim isn't there): as in v1, the editor is part of gitview, so
// closing gitview (cmd-shift-g, the Changes button, closing cmd-g's tab)
// closes it too. A local nvim listens on a socket of ours as well as its
// own (`--cmd serverstart`), so it can be asked for unsaved buffers and
// told to quit. Shared (linked; install copies it).
//
// shared "gitview.editors": {<tab>: {terminal, kind, name, bin, sock, file}}.

import { appHome, exec, openOverlay, setShared, shared, spaces, terminals } from "herdr";
import * as fs from "fs/promises";
import { event, tracer } from "./telemetry.js";

export function editorIn(tab) {
  return (shared("gitview.editors") ?? {})[tab] ?? null;
}

// Tabs that are gone go too (shared.json stays bounded).
export function setEditor(tab, rec) {
  const live = new Set(spaces().flatMap((s) => s.tabs.map((t) => t.id)));
  const all = Object.fromEntries(Object.entries(shared("gitview.editors") ?? {}).filter(([t]) => live.has(t) || t === tab));
  if (rec) all[tab] = rec;
  else if (tab in all) delete all[tab];
  else return;
  setShared("gitview.editors", all);
}

// Running: "live" (a terminal just started may not say so yet: listed
// within 3 s of its start counts).
const live = (address, at = 0) => terminals().some((t) => t.address === address && (String(t.state ?? "live").toLowerCase() === "live" || Date.now() - at < 3000));

// The tab's editor, while its terminal runs (a gone one is forgotten).
// Its terminal is found here, by whichever copy asks first (the app's
// terminal.open doesn't say which it started): the diff pane that opened
// it may be gone by then (the editor took its place).
export function liveEditor(tab) {
  let rec = editorIn(tab);
  if (rec && !rec.terminal) {
    const t = terminals().find((t) => !(rec.before ?? []).includes(t.address) && (t.argv ?? []).includes(rec.file));
    if (!t) {
      if (Date.now() - (rec.at ?? 0) < 10000) return rec; // still starting
      setEditor(tab, null);
      return null;
    }
    const { before: _, ...found } = rec;
    rec = { ...found, terminal: t.address };
    setEditor(tab, rec);
    return rec; // (just started: maybe not "live" in the list yet)
  }
  if (rec && live(rec.terminal, rec.at)) return rec;
  if (rec) setEditor(tab, null);
  return null;
}

// Where a tab's nvim listens for us: short (a socket path has ~100 bytes),
// and new for each start: an nvim still quitting removes its socket as it
// exits, which took the next one's when they shared the path. The tab's
// older ones go (their editor isn't the tab's any more).
export async function socketFor(tab) {
  let h = 0;
  for (const c of String(tab)) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const dir = `${appHome()}/data/gitview`;
  const name = `nv-${h.toString(36)}`;
  try {
    await fs.mkdir(dir, { recursive: true });
    for (const f of await fs.readdir(dir)) if (f === `${name}.sock` || f.startsWith(`${name}-`)) await fs.unlink(`${dir}/${f}`).catch(() => {});
  } catch {
    // none there
  }
  return `${dir}/${name}-${(Date.now() % 1e8).toString(36)}.sock`;
}


// Asking nvim: traced as an exec (op "nvim-ask").
async function ask(rec, expr) {
  const t0 = Date.now();
  const r = await exec(rec.bin, ["--server", rec.sock, "--remote-expr", expr]);
  tracer().line("exec", { op: "nvim-ask", start: t0, ms: Date.now() - t0, ok: r.code === 0 });
  return r;
}

// "clean" (nothing unsaved), "dirty", or "unknown" (it can't be asked: not
// an nvim, or it doesn't answer).
export async function editorState(rec) {
  if (rec.kind !== "nvim" || !rec.sock) return "unknown";
  try {
    const r = await ask(rec, "len(filter(getbufinfo({'bufloaded': 1}), 'v:val.changed'))");
    if (r.code !== 0) return "unknown";
    return Number(r.stdout.trim()) > 0 ? "dirty" : "clean";
  } catch {
    return "unknown";
  }
}

// "qa" (quit; clean), "wqa" (save all, quit), "qa!" (discard, quit).
const quitArgv = (rec, how) => ["--server", rec.sock, "--remote-send", `<C-\\><C-N>:${how}<CR>`];

export async function quitEditor(rec, how) {
  const t0 = Date.now();
  const r = await exec(rec.bin, quitArgv(rec, how));
  tracer().line("exec", { op: "nvim-quit", how, start: t0, ms: Date.now() - t0, ok: r.code === 0 });
}

// The tab's editor closes with gitview: "none" (there's none), "clean"
// (it quit), "dirty" (the question is asked: `cancel` / `after` steps, see
// askToClose) or "unknown" (it can't be asked; it stays).
export async function closeEditorOf(tab, { cancel = [], after = [] } = {}) {
  const ed = liveEditor(tab);
  if (!ed) return "none";
  const state = await editorState(ed);
  event("editors", { closing: ed.name, state });
  if (state === "clean") {
    await quitEditor(ed, "qa");
    setEditor(tab, null);
  } else if (state === "dirty") askToClose(ed, cancel, after);
  return state;
}

// Unsaved buffers when gitview closes: the dialog (mod gitview-ask) asks
// Save / Discard / Cancel and does it itself (the list may be hidden, and
// hidden copies get no commands). `cancel`: steps run on Cancel (show the
// panel again); `after`: steps after Save or Discard (close cmd-g's tab).
function askToClose(rec, cancel, after = []) {
  const quit = (how) => [{ exec: [rec.bin, quitArgv(rec, how)] }, ...after];
  openOverlay({
    mod: "gitview-ask",
    kind: "dialog",
    modal: true,
    width: 440,
    height: 180,
    props: {
      title: "The editor has unsaved changes",
      detail: `Save them? (${rec.file.split("/").pop()} in ${rec.name})`,
      choices: [
        { label: "Save", key: "s", steps: quit("wqa") },
        { label: "Discard", key: "d", steps: quit("qa!") },
        { label: "Cancel", key: "escape", steps: [...cancel, { notify: { title: "gitview", body: "The editor has unsaved changes: it stays open.", level: "info" } }] },
      ],
    },
  });
}
