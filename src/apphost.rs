//! The Herdr desktop app host (`GITVIEW_HOST=app`): the same panes and
//! orchestration as under the herdr TUI, with every host call mapped to the
//! app's two programs:
//!
//! - `herdr-app do <command>` changes the window (split, focus, resize,
//!   tabs, overlays). Its binary comes from `GITVIEW_HERDR_APP` (the app mods
//!   pass the path the installer recorded), else `herdr-app` on `PATH`.
//! - `hc` (herdr-core) runs and talks to terminals; `hc connect` gives the
//!   JSON-RPC API for what the CLI doesn't print (full terminal facts, the
//!   app's layout document). Binary: `GITVIEW_HC`, else `hc`.
//!
//! Pane ids under this host are terminal addresses (`t7`): the app focuses
//! panes by terminal and the core kills, reads and types into them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::logx::log;

/// The mod that shows a popup terminal as a dialog (see `herdr-app/mods`).
const POPUP_MOD: &str = "gitview-popup";
/// Label marking a terminal as a gitview popup (value: entrypoint).
pub const POPUP_LABEL: &str = "gitview.popup";
/// Label carrying a popup's wanted size in cells (`60x7`).
pub const POPUP_CELLS_LABEL: &str = "gitview.cells";

fn app_bin() -> std::ffi::OsString {
    std::env::var_os("GITVIEW_HERDR_APP").unwrap_or_else(|| "herdr-app".into())
}

pub use crate::git::hc_bin;

/// `herdr-app do <command> <json args>`; the reply object, or its error.
pub fn app_do(command: &str, args: Value) -> Result<Value> {
    let bin = app_bin();
    let out = Command::new(&bin)
        .args(["do", command, &args.to_string()])
        .output()
        .with_context(|| format!("spawning {}", bin.to_string_lossy()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log(format!(
        "herdr-app do {command} {args} -> {} | {}{}",
        out.status,
        stdout.trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    ));
    let reply: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        bail!("{command}: {err}");
    }
    if !out.status.success() {
        bail!(
            "{command} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(reply)
}

/// One herdr-core API call over `hc connect` (JSON-RPC, one line each).
pub fn hc_rpc(method: &str, params: Value) -> Result<Value> {
    let bin = hc_bin();
    let mut child = Command::new(&bin)
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {} connect", bin.to_string_lossy()))?;
    {
        let mut stdin = child.stdin.take().context("hc connect stdin")?;
        let hello =
            json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {"generation": 1}});
        let call = json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params});
        writeln!(stdin, "{hello}\n{call}")?;
    } // EOF ends the session once both are answered
    let out = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let reply = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v.get("id") == Some(&json!(2)))
        .with_context(|| {
            format!(
                "hc {method}: no reply ({})",
                String::from_utf8_lossy(&out.stderr).trim()
            )
        })?;
    if let Some(err) = reply.get("error") {
        bail!("hc {method}: {err}");
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

/// Run `hc <args> --json` and parse its reply.
fn hc_json(args: &[&str]) -> Result<Value> {
    let bin = hc_bin();
    let out = Command::new(&bin).args(args).arg("--json").output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    log(format!(
        "hc {args:?} -> {} | {}{}",
        out.status,
        stdout.trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    ));
    if !out.status.success() {
        bail!(
            "hc {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(serde_json::from_str(stdout.trim()).unwrap_or(Value::Null))
}

/// Name our pane: the app's pane header shows the terminal's title (the
/// herdr plugin manifest named the panes instead).
pub fn set_title(title: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]2;{title}\x07");
    let _ = out.flush();
}

// ---- terminals --------------------------------------------------------------

/// A terminal's full facts (`terminal.get`), live or finished.
pub fn terminal(address: &str) -> Option<Value> {
    hc_rpc("terminal.get", json!({ "terminal": address })).ok()
}

pub fn terminal_live(address: &str) -> bool {
    terminal(address)
        .and_then(|t| t.get("state").and_then(Value::as_str).map(|s| s == "live"))
        .unwrap_or(false)
}

/// Live terminals carrying all `labels` (full facts).
fn terminals_with(labels: Value) -> Vec<Value> {
    hc_rpc("terminal.list", json!({ "labels": labels }))
        .ok()
        .and_then(|r| r.get("terminals").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// Every live terminal address on the core (for the stale-view sweep).
pub fn live_terminals() -> Result<Vec<String>> {
    let reply = hc_rpc("terminal.list", json!({}))?;
    Ok(reply
        .get("terminals")
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .filter_map(|t| t.get("address").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

fn label<'a>(terminal: &'a Value, key: &str) -> Option<&'a str> {
    terminal.get("labels")?.get(key)?.as_str()
}

/// End a terminal (its pane closes with it).
pub fn kill(address: &str) {
    let _ = hc_rpc("terminal.kill", json!({ "terminal": address }));
}

/// Focus the pane (or layout block) showing `terminal`. Best-effort.
pub fn focus_terminal(terminal: &str) {
    if let Err(err) = app_do("pane.focus", json!({ "terminal": terminal })) {
        log(format!("focus {terminal} failed: {err}"));
    }
}

// ---- nvim in the tab --------------------------------------------------------

/// An nvim running in another terminal of `own`'s tab (the app's version
/// of [`crate::herdr_cli::find_nvim_in_tab`]: the core reports each
/// terminal's foreground process, so no per-pane process query is needed).
pub fn find_nvim_in_tab(own: &str, exclude: &[&str]) -> Option<crate::herdr_cli::TabNvim> {
    let me = terminal(own)?;
    let tab = label(&me, "herdr.tab")?.to_string();
    let mut skip = exclude.to_vec();
    skip.push(own);
    find_nvim(terminals_with(json!({ "herdr.tab": tab })), &skip)
}

/// The first remote-controllable nvim among `candidates` (terminal facts).
pub fn find_nvim(candidates: Vec<Value>, exclude: &[&str]) -> Option<crate::herdr_cli::TabNvim> {
    for t in candidates {
        let Some(address) = t.get("address").and_then(Value::as_str) else {
            continue;
        };
        if exclude.contains(&address) || t.get("state").and_then(Value::as_str) != Some("live") {
            continue;
        }
        let Some(fg) = t.get("foreground") else {
            continue;
        };
        if fg.get("name").and_then(Value::as_str) != Some("nvim") {
            continue;
        }
        let argv: Vec<&str> = fg
            .get("argv")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let pid = fg.get("pid").and_then(Value::as_u64);
        if let Some(socket) = crate::herdr_cli::nvim_socket_from(&argv, pid) {
            log(format!(
                "found nvim in terminal {address} ({})",
                socket.display()
            ));
            return Some(crate::herdr_cli::TabNvim {
                pane_id: address.to_string(),
                socket,
            });
        }
    }
    None
}

/// Open `file` at `line` in this tab's nvim: the herdr-nvim mod's when it
/// takes the request, else an nvim already running in `tab` or in one of the
/// `shown` terminals (layout blocks). Returns how it went, `None` when there
/// is no nvim to reuse (the caller starts a plain one).
pub fn open_in_running_nvim(
    tab: Option<&str>,
    space: Option<&str>,
    shown: &[String],
    file: &Path,
    line: u32,
) -> Option<String> {
    if let Some(tab) = tab
        && open_in_herdr_nvim(tab, space, file, line)
    {
        return Some("herdr-nvim".to_string());
    }
    let mut candidates = Vec::new();
    if let Some(tab) = tab {
        candidates.extend(terminals_with(json!({ "herdr.tab": tab })));
    }
    candidates.extend(shown.iter().filter_map(|t| terminal(t)));
    let nvim = find_nvim(candidates, &[])?;
    let keys = format!(
        "<C-\\><C-n>:edit +{line} {}<CR>",
        file.display().to_string().replace(' ', "\\ ")
    );
    let out = crate::nvim::remote("nvim", Some(&nvim.socket), &["--remote-send", &keys])?;
    out.status.success().then_some(nvim.pane_id)
}

/// The herdr-nvim link-up, in one place. herdr-nvim's contract:
/// `<app home>/mods/herdr-nvim/bin/herdr-nvim open --tab <tab id> [--space <id>]
/// <file>:<line>`
/// opens the file in that tab's nvim (starting its pane and daemon when
/// needed), focuses it, prints `{"terminal":"tN"}` and exits 0; nonzero with
/// the reason on stderr. No binary there: herdr-nvim isn't installed, and the
/// caller falls back to plain nvim.
pub fn open_in_herdr_nvim(tab: &str, space: Option<&str>, file: &Path, line: u32) -> bool {
    let home = std::env::var_os("HERDR_APP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::logx::xdg_dir("XDG_CONFIG_HOME", ".config").join("herdr-app"));
    let bin = home.join("mods/herdr-nvim/bin/herdr-nvim");
    if !bin.exists() {
        return false;
    }
    let target = format!("{}:{line}", file.display());
    let mut cmd = Command::new(&bin);
    cmd.args(["open", "--tab", tab]);
    if let Some(space) = space.filter(|s| !s.is_empty()) {
        cmd.args(["--space", space]);
    }
    let Ok(out) = cmd
        .arg(&target)
        .env_remove("HC_SCOPE") // a scope that isn't ours gets its hc calls ended
        .output()
    else {
        return false;
    };
    log(format!(
        "herdr-nvim open {target} -> {} {}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    ));
    out.status.success()
}

// ---- agents -----------------------------------------------------------------

/// The agents in `own`'s space: terminals whose program reports a status,
/// as `(address, agent, status, tab, cwd_basename)` rows for the picker.
pub fn space_agents(own: Option<&str>) -> Vec<crate::popup::AgentRow> {
    let Some(me) = own.and_then(terminal) else {
        return Vec::new();
    };
    let Some(space) = label(&me, "herdr.space").map(str::to_string) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for t in terminals_with(json!({ "herdr.space": space })) {
        let Some(status) = t.get("status").filter(|s| s.is_object()) else {
            continue; // not an agent: never reported a status
        };
        let get = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let address = get(&t, "address");
        let mut agent = get(status, "app");
        if agent.is_empty() {
            agent = t
                .pointer("/foreground/name")
                .and_then(Value::as_str)
                .unwrap_or("agent")
                .to_string();
        }
        // `where` is "space / tab"; the picker wants the tab.
        let tab = label(&t, "where")
            .and_then(|w| w.rsplit_once(" / ").map(|(_, tab)| tab.to_string()))
            .unwrap_or_default();
        let cwd = get(&t, "cwd").rsplit('/').next().unwrap_or("").to_string();
        rows.push((address, agent, get(status, "state"), tab, cwd));
    }
    rows
}

/// Deliver review notes to an agent terminal. `submit`: `hc send` (paste +
/// enter, once the person pauses typing). Otherwise the notes are *placed*
/// in its input without enter: pasted through a framed `terminal.attach`
/// (see [`crate::hcpaste`]), so no clipboard is involved.
pub fn send_text(terminal: &str, text: &str, submit: bool) -> Result<()> {
    if submit {
        // `send` presses enter itself; trailing newlines would add blank turns.
        hc_json(&["send", terminal, text.trim_end_matches('\n')])?;
        return Ok(());
    }
    crate::hcpaste::paste(terminal, text)
}

// ---- popups -----------------------------------------------------------------

/// Run a popup entrypoint (`ask`, `pick-agent`) in a hidden terminal and
/// have the app show it as a dialog (the `gitview-popup` mod reads the
/// labels and opens an overlay holding that terminal; the overlay closes by
/// itself when the popup exits). Returns the popup's terminal address.
pub fn open_popup(
    entrypoint: &str,
    envs: &[(String, String)],
    (width, height): (u16, u16),
) -> Result<String> {
    let exe = std::env::current_exe()?;
    let mut argv: Vec<String> = vec!["/usr/bin/env".into()];
    argv.extend(passthrough_env());
    argv.extend(envs.iter().map(|(k, v)| format!("{k}={v}")));
    argv.push(exe.display().to_string());
    argv.push(entrypoint.to_string());

    let entry_label = format!("{POPUP_LABEL}={entrypoint}");
    let cells_label = format!("{POPUP_CELLS_LABEL}={width}x{height}");
    let mut args: Vec<&str> = vec![
        "run",
        "--bg",
        // Our own terminal is its parent (it ends with us), but must not get
        // notes about it typed into the list/preview TUI.
        "--no-notify",
        "--rm",
        "--label",
        &entry_label,
        "--label",
        &cells_label,
        "--",
    ];
    args.extend(argv.iter().map(String::as_str));
    let reply = hc_json(&args)?;
    let address = reply
        .get("address")
        .and_then(Value::as_str)
        .context("no address in `hc run` reply")?
        .to_string();
    app_do("mod.open", json!({ "name": POPUP_MOD }))?;
    Ok(address)
}

/// Run `herdr-gitview <mode>` in a hidden terminal that outlives ours,
/// passing our host env and the `keep` variables on.
pub fn run_detached(mode: &str, keep: &[&str]) -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut argv: Vec<String> = vec!["/usr/bin/env".into()];
    argv.extend(passthrough_env());
    argv.extend(
        keep.iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| format!("{k}={v}"))),
    );
    argv.push(exe.display().to_string());
    argv.push(mode.to_string());
    let mut args = vec!["run", "--bg", "--no-parent", "--no-notify", "--rm", "--"];
    args.extend(argv.iter().map(String::as_str));
    hc_json(&args).map(|_| ())
}

// ---- views: orchestration primitives ---------------------------------------

/// Where a toggle was pressed, as the app mod saw it (`focused()` plus the
/// space's folders), from `GITVIEW_APP_CONTEXT`. From inside one of our own
/// panes there is no context: our terminal's labels name space and tab.
#[derive(Debug, Default, Clone)]
pub struct AppCtx {
    pub space: String,
    pub tab: String,
    /// Folders to look for the repo in, best first.
    pub folders: Vec<PathBuf>,
}

pub fn context() -> Result<AppCtx> {
    if let Ok(raw) = std::env::var("GITVIEW_APP_CONTEXT") {
        log(format!("app context: {raw}"));
        let v: Value = serde_json::from_str(&raw).context("bad GITVIEW_APP_CONTEXT")?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut folders: Vec<PathBuf> = Vec::new();
        for key in ["cwd"] {
            if !s(key).is_empty() {
                folders.push(PathBuf::from(s(key)));
            }
        }
        if let Some(more) = v.get("folders").and_then(Value::as_array) {
            folders.extend(more.iter().filter_map(Value::as_str).map(PathBuf::from));
        }
        let ctx = AppCtx {
            space: s("space"),
            tab: s("tab"),
            folders,
        };
        if ctx.tab.is_empty() {
            bail!("no tab in GITVIEW_APP_CONTEXT (open a space first)");
        }
        return Ok(ctx);
    }
    let own =
        std::env::var("HC_TERMINAL_ID").context("no GITVIEW_APP_CONTEXT or HC_TERMINAL_ID")?;
    let me = terminal(&own).context("own terminal unknown to hc")?;
    Ok(AppCtx {
        space: label(&me, "herdr.space").unwrap_or("").to_string(),
        tab: label(&me, "herdr.tab")
            .context("own terminal has no herdr.tab label")?
            .to_string(),
        folders: me
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .into_iter()
            .collect(),
    })
}

/// The app's layout document (spaces → tabs → split trees), from the core.
fn layout_doc() -> Result<Value> {
    Ok(hc_rpc("doc.get", json!({ "name": "herdr-app" }))?
        .get("value")
        .cloned()
        .unwrap_or(Value::Null))
}

fn find_tab<'a>(doc: &'a Value, tab: &str) -> Option<&'a Value> {
    doc.get("spaces")?
        .as_array()?
        .iter()
        .filter_map(|s| s.get("tabs")?.as_array())
        .flatten()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(tab))
}

/// The id of a tab's root split node. Splitting it gives a pane that spans
/// the whole tab, which is how the sidebar gets full height without the
/// herdr TUI's park-and-rebuild dance.
pub fn tab_root(tab: &str) -> Result<String> {
    let doc = layout_doc()?;
    find_tab(&doc, tab)
        .and_then(|t| t.pointer("/root/id")?.as_str())
        .map(str::to_string)
        .with_context(|| format!("tab {tab} not in the app's layout"))
}

/// The pane id showing `terminal`, from the layout document.
pub fn pane_of(terminal: &str) -> Result<String> {
    fn walk(node: &Value, terminal: &str) -> Option<String> {
        if node.pointer("/content/terminal").and_then(Value::as_str) == Some(terminal) {
            return node.get("id")?.as_str().map(str::to_string);
        }
        node.get("children")?
            .as_array()?
            .iter()
            .find_map(|c| walk(c, terminal))
    }
    let doc = layout_doc()?;
    doc.get("spaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("tabs")?.as_array())
        .flatten()
        .find_map(|t| walk(t.get("root")?, terminal))
        .with_context(|| format!("no pane shows {terminal}"))
}

/// Env every gitview process under the app needs, passed through argv
/// (the app's pane commands take no env).
fn passthrough_env() -> Vec<String> {
    [
        "GITVIEW_HERDR_APP",
        "GITVIEW_HC",
        "GITVIEW_STATE_DIR",
        "GITVIEW_CONFIG_DIR",
        "HERDR_APP_HOME",
        "PATH",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| format!("{k}={v}")))
    .chain(std::iter::once("GITVIEW_HOST=app".to_string()))
    .collect()
}

/// argv for one of our panes: `env K=V… <self> <entrypoint>`.
fn pane_argv(
    repo: &Path,
    socket: &Path,
    entrypoint: &str,
    extra: &[(&str, &str)],
) -> Result<Vec<String>> {
    let exe = std::env::current_exe()?;
    let mut argv: Vec<String> = vec!["/usr/bin/env".into()];
    argv.extend(passthrough_env());
    argv.push(format!("GITVIEW_REPO={}", repo.display()));
    argv.push(format!("GITVIEW_SOCKET={}", socket.display()));
    argv.extend(extra.iter().map(|(k, v)| format!("{k}={v}")));
    argv.push(exe.display().to_string());
    argv.push(entrypoint.to_string());
    Ok(argv)
}

/// `pane.split` with a new terminal; returns `(pane, terminal)`.
fn split(target: &str, argv: &[String]) -> Result<(String, String)> {
    let reply = app_do(
        "pane.split",
        json!({ "direction": "right", "pane": target, "argv": argv }),
    )?;
    let get = |k: &str| {
        reply
            .get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("no {k} in pane.split reply"))
    };
    Ok((get("pane")?, get("terminal")?))
}

fn weigh(node: &str, weight: f64) {
    let _ = app_do("layout.resize", json!({ "id": node, "weight": weight }));
}

/// The two panes of an open view.
pub struct Opened {
    pub tab: String,
    pub preview: String,
    pub list: String,
}

/// Cut the preview/list pair (already side by side, sharing `share` of
/// their row as weights) at `list_width_percent`, list on the configured
/// side, and give the list the keyboard.
fn arrange(cfg: &crate::config::Config, preview: (&str, &str), list: (&str, &str), share: f64) {
    let l = f64::from(cfg.list_width_percent) / 100.0;
    weigh(preview.0, share * (1.0 - l));
    weigh(list.0, share * l);
    focus_terminal(list.1);
    if cfg.list_side == crate::config::ListSide::Left {
        let _ = app_do("pane.swap", json!({ "direction": "left" }));
    }
}

/// Sidebar: split the tab's root, so the view spans the full tab height to
/// the right of everything already there, at `view_width_percent`.
pub fn open_sidebar(
    repo: &Path,
    socket: &Path,
    cfg: &crate::config::Config,
    ctx: &AppCtx,
) -> Result<Opened> {
    let root = tab_root(&ctx.tab)?;
    let (pp, pt) = split(&root, &pane_argv(repo, socket, "preview", &[])?)?;
    let (lp, lt) = split(
        &pp,
        &pane_argv(repo, socket, "list", &[("GITVIEW_PREVIEW_PANE", &pt)])?,
    )?;
    // Root split: [old layout = 1, sidebar]; sidebar share w gives
    // w / (1 + w) = view fraction.
    let f = f64::from(cfg.view_width_percent) / 100.0;
    arrange(cfg, (&pp, &pt), (&lp, &lt), f / (1.0 - f));
    Ok(Opened {
        tab: ctx.tab.clone(),
        preview: pt,
        list: lt,
    })
}

/// Tab: a new tab named "gitview" holding just the two panes. `tab.new`
/// with a program wraps it in a login shell that stays after it exits, so
/// the tab starts with a plain shell that our panes replace.
pub fn open_tab(
    repo: &Path,
    socket: &Path,
    cfg: &crate::config::Config,
    ctx: &AppCtx,
) -> Result<Opened> {
    let mut args = json!({ "name": "gitview", "folder": repo.display().to_string() });
    if !ctx.space.is_empty() {
        args["space"] = json!(ctx.space);
    }
    let reply = app_do("tab.new", args)?;
    let tab = reply
        .get("tab")
        .and_then(Value::as_str)
        .context("no tab in tab.new reply")?
        .to_string();
    let shell = reply
        .get("terminal")
        .and_then(Value::as_str)
        .context("no terminal in tab.new reply")?
        .to_string();
    let shell_pane = wait_for(|| pane_of(&shell))?;
    let (pp, pt) = split(&shell_pane, &pane_argv(repo, socket, "preview", &[])?)?;
    let (lp, lt) = split(
        &pp,
        &pane_argv(repo, socket, "list", &[("GITVIEW_PREVIEW_PANE", &pt)])?,
    )?;
    let _ = app_do("pane.close", json!({ "pane": shell_pane }));
    arrange(cfg, (&pp, &pt), (&lp, &lt), 1.0);
    Ok(Opened {
        tab,
        preview: pt,
        list: lt,
    })
}

/// The layout document trails the command that changed it by a moment.
fn wait_for<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last = None;
    for _ in 0..20 {
        match f() {
            Ok(v) => return Ok(v),
            Err(err) => last = Some(err),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(last.unwrap())
}

/// A mod pane (`pane.split {mod}`) split off the tab's root: full height on
/// the right, `width_percent` of the tab. Returns its pane id.
pub fn open_mod_sidebar(
    tab: &str,
    module: &str,
    props: Value,
    width_percent: u8,
) -> Result<String> {
    let root = tab_root(tab)?;
    let mut args = json!({ "direction": "right", "pane": root, "mod": module });
    if !props.is_null() {
        args["props"] = props;
    }
    let reply = app_do("pane.split", args)?;
    let pane = reply
        .get("pane")
        .and_then(Value::as_str)
        .context("no pane in pane.split reply")?
        .to_string();
    let f = f64::from(width_percent) / 100.0;
    weigh(&pane, f / (1.0 - f));
    Ok(pane)
}

pub fn select_tab(tab: &str) -> Result<()> {
    app_do("tab.select", json!({ "id": tab })).map(|_| ())
}

pub fn close_tab(tab: &str) -> Result<()> {
    app_do("tab.close", json!({ "id": tab, "confirmed": true })).map(|_| ())
}
