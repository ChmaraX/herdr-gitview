// gitview's git engine, the herdr-gitview binary (`herdr-gitview json <op>`),
// and where the panel looks: the shown space's repo, on its machine.
// Shared by the gitview mods (the other folders link to this file; install
// copies it).

import { appHome, exec, readTextFile } from "herdr";
import { shortArgs, traceDir, tracer, tracing } from "./telemetry.js";

let config = null;

// Written by install: { bin, herdr_app, hc }.
async function loadConfig() {
  if (config) return config;
  const text = await readTextFile(`${appHome()}/mods/gitview/config.json`);
  if (!text) throw new Error("gitview isn't installed: run herdr-app/install.sh from the herdr-gitview repo");
  config = JSON.parse(text);
  return config;
}

// The environment the binary needs under the app (it can't see appHome()).
function hostEnv(cfg) {
  const env = ["GITVIEW_HOST=app", `HERDR_APP_HOME=${appHome()}`];
  if (cfg.herdr_app) env.push(`GITVIEW_HERDR_APP=${cfg.herdr_app}`);
  if (cfg.hc) env.push(`GITVIEW_HC=${cfg.hc}`);
  if (tracing()) env.push(`GITVIEW_TRACE=${traceDir()}/engine.jsonl`);
  return env;
}

// Runs one `json` op here (it reaches another machine itself, `--on`);
// resolves with its object, rejects with its error. Callers put "--" before
// paths, messages and text, so one starting with "--" stays an argument.
// Traced: the op, its ms here (the app's queue time too), ok or the error.
export async function gv(args) {
  const t0 = Date.now();
  const tr = tracer();
  try {
    const cfg = await loadConfig();
    const r = await exec("/usr/bin/env", [...hostEnv(cfg), cfg.bin, "json", ...args]);
    let out = null;
    try { out = JSON.parse(r.stdout); } catch { /* reported below */ }
    if (r.code !== 0 || !out || out.error) throw new Error(out?.error ?? (r.stderr || r.stdout || `exit ${r.code}`).trim());
    tr.line("exec", { op: args[0], args: shortArgs(args), start: t0, ms: Date.now() - t0, ok: true, ...(out.same ? { same: true } : {}) });
    return out;
  } catch (e) {
    tr.line("exec", { op: args[0], args: shortArgs(args), start: t0, ms: Date.now() - t0, ok: false, error: message(e).split("\n")[0].slice(0, 200) });
    throw e;
  }
}

// `herdr-app <args>` (the app's own CLI: status, ls) as JSON, null when it
// can't be asked. What the app has now: a window that isn't drawn can leave
// a copy's live data a step behind.
export async function appCli(args) {
  try {
    const cfg = await loadConfig();
    if (!cfg.herdr_app) return null;
    const t0 = Date.now();
    const r = await exec("/usr/bin/env", [...hostEnv(cfg), cfg.herdr_app, ...args]);
    tracer().line("exec", { op: `app ${args[0]}`, start: t0, ms: Date.now() - t0, ok: r.code === 0 });
    return JSON.parse(r.stdout);
  } catch {
    return null;
  }
}

// The live terminals carrying a label (`hc ls --label k=v`), by address:
// what herdr-core has now (a window that isn't drawn can leave a copy's
// terminals() behind). [] when it can't be asked.
export async function labelled(label) {
  try {
    const cfg = await loadConfig();
    const r = await exec(cfg.hc || "hc", ["ls", "--json", "--label", label]);
    return (JSON.parse(r.stdout).terminals ?? []).filter((t) => t.state === "live").map((t) => t.address);
  } catch {
    return [];
  }
}

// Ends a terminal through herdr-core itself (`hc kill`): its pane goes.
export async function endTerminal(address) {
  try {
    const cfg = await loadConfig();
    await exec(cfg.hc || "hc", ["kill", address]);
  } catch {
    // gone already
  }
}

// `--on <machine>` for a repo on another machine, nothing here.
export const onArgs = (on) => (on ? ["--on", on] : []);

export const message = (e) => String(e?.message ?? e);
