// gitview's git engine, the herdr-gitview binary (`herdr-gitview json <op>`),
// and where the panel looks: the shown space's repo, on its machine.
// Shared by the gitview mods (the other folders link to this file; install
// copies it).

import { appHome, exec, focused, panes, readTextFile, spaces } from "herdr";

let config = null;

// Written by install: { bin, herdr_app, hc }.
export async function loadConfig() {
  if (config) return config;
  const text = await readTextFile(`${appHome()}/mods/gitview/config.json`);
  if (!text) throw new Error("gitview isn't installed: run herdr-app/install.sh from the herdr-gitview repo");
  config = JSON.parse(text);
  return config;
}

// The environment the binary needs under the app (it can't see appHome()).
export function hostEnv(cfg) {
  const env = ["GITVIEW_HOST=app", `HERDR_APP_HOME=${appHome()}`];
  if (cfg.herdr_app) env.push(`GITVIEW_HERDR_APP=${cfg.herdr_app}`);
  if (cfg.hc) env.push(`GITVIEW_HC=${cfg.hc}`);
  return env;
}

// Runs one `json` op here (it reaches another machine itself, `--on`);
// resolves with its object, rejects with its error.
export async function gv(args) {
  const cfg = await loadConfig();
  const r = await exec("/usr/bin/env", [...hostEnv(cfg), cfg.bin, "json", ...args]);
  let out = null;
  try { out = JSON.parse(r.stdout); } catch { /* reported below */ }
  if (r.code !== 0 || !out || out.error) throw new Error(out?.error ?? (r.stderr || r.stdout || `exit ${r.code}`).trim());
  return out;
}

// `--on <machine>` for a repo on another machine, nothing here.
export const onArgs = (on) => (on ? ["--on", on] : []);

export const message = (e) => String(e?.message ?? e);

// The shown space's folder, as the stock Changes panel finds it: the focused
// pane's live folder, else the space's. `on`: the machine it's on (null:
// this one). A mod pane (gitview's own diff pane) has no folder: `keep` (the
// place before, in the same tab) stays.
export function location(keep) {
  const f = focused();
  const space = spaces().find((s) => s.active);
  const pane = f?.pane ? panes().find((p) => p.pane === f.pane) : null;
  const on = f?.pane === null ? (f?.terminal?.includes("/") ? f.terminal.split("/")[0] : null) : space?.machine ?? null;
  if (pane && !pane.terminal && keep && keep.tab === f?.tab) return keep;
  return {
    cwd: pane?.cwd || f?.cwd || space?.folder || null,
    on,
    space: space?.id ?? null,
    tab: f?.tab ?? null,
  };
}
