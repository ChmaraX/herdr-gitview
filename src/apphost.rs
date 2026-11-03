//! The Herdr desktop app, as `herdr-gitview json` and `app-setup` reach it:
//!
//! - `herdr-app` changes and reads the window (`do pane.split`, `status`),
//!   through one client, [`App`]: the binary and the app home, read from the
//!   environment once ([`App::from_env`]) or given (`app-setup --app
//!   --home`).
//! - `hc` (herdr-core) runs and talks to terminals; `hc connect` gives the
//!   JSON-RPC API for what the CLI doesn't print (terminal facts, the app's
//!   layout documents). Binary: `GITVIEW_HC`, else `hc`.
//!
//! The gitview TUI itself runs only under the herdr TUI; nothing here
//! starts or drives its panes.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::logx::log;

pub use crate::remote::hc_bin;

/// The Herdr app, as gitview reaches it: its `herdr-app` binary, and its
/// home (layouts, keys, mods), which every call is made against.
pub struct App {
    pub bin: OsString,
    pub home: PathBuf,
}

impl App {
    /// The binary: `GITVIEW_HERDR_APP` (the mods pass the path the installer
    /// recorded), else `herdr-app` on `PATH`. The home, as the app defines
    /// it: `$HERDR_APP_HOME`, else `$HOME/.config/herdr-app`.
    pub fn from_env() -> App {
        let bin = std::env::var_os("GITVIEW_HERDR_APP").unwrap_or_else(|| "herdr-app".into());
        let home = std::env::var_os("HERDR_APP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(".config/herdr-app")
            });
        App { bin, home }
    }

    /// `herdr-app <args>` → its JSON reply. An `{"error"}` reply or a failed
    /// exit is an error.
    fn call(&self, args: &[&str]) -> Result<Value> {
        let out = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .env("HERDR_APP_HOME", &self.home)
            .output()
            .with_context(|| format!("cannot run {}", self.bin.to_string_lossy()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        log(format!(
            "herdr-app {} -> {} | {}{}",
            args.join(" "),
            out.status,
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
        let reply: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
        if let Some(err) = reply.get("error").and_then(Value::as_str) {
            bail!("herdr-app {what}: {err}");
        }
        if !out.status.success() {
            bail!(
                "herdr-app {what} failed (is the Herdr app open?): {}{}",
                stdout.trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(reply)
    }

    /// `herdr-app do <command> <json args>`.
    pub fn command(&self, command: &str, args: Value) -> Result<Value> {
        self.call(&["do", command, &args.to_string()])
    }

    /// What the window shows (`herdr-app status`): layout, focused space,
    /// tab, pane.
    pub fn status(&self) -> Result<Value> {
        self.call(&["status"])
    }

    /// The herdr-nvim mod's command-line entry, when it's installed.
    fn herdr_nvim(&self) -> PathBuf {
        self.home.join("mods/herdr-nvim/bin/herdr-nvim")
    }
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

// ---- terminals and nvim -----------------------------------------------------

/// A terminal's full facts (`terminal.get`), live or finished.
pub fn terminal(address: &str) -> Option<Value> {
    hc_rpc("terminal.get", json!({ "terminal": address })).ok()
}

/// Live terminals carrying all `labels` (full facts).
fn terminals_with(labels: Value) -> Vec<Value> {
    hc_rpc("terminal.list", json!({ "labels": labels }))
        .ok()
        .and_then(|r| r.get("terminals").and_then(Value::as_array).cloned())
        .unwrap_or_default()
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
        if let Some(socket) = nvim_socket_from(&argv, pid) {
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

/// The remote socket of an nvim process, from its argv and pid:
/// `nvim --server <sock> --remote-ui` (herdr-nvim sidebar and co.) names it;
/// a plain nvim auto-listens on a socket derived from its pid (nvim ≥0.10's
/// TUI runs its server as a `nvim --embed` child: that pid owns it).
fn nvim_socket_from(argv: &[&str], pid: Option<u64>) -> Option<PathBuf> {
    if let Some(pos) = argv.iter().position(|a| *a == "--server")
        && let Some(sock) = argv.get(pos + 1)
    {
        let sock = PathBuf::from(sock);
        if sock.exists() {
            return Some(sock);
        }
    }
    let pid = pid?;
    let socket = crate::herdr_cli::socket_for_pid;
    socket(pid).or_else(|| child_pids(pid).into_iter().find_map(socket))
}

fn child_pids(pid: u64) -> Vec<u64> {
    Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Open `file` at `line` in this tab's nvim: the herdr-nvim mod's when it
/// takes the request (asked only when `herdr_nvim`: there's an nvim for it
/// to run), else an nvim already running in `tab` or in one of the
/// `shown` terminals (layout blocks). Returns how it went, `None` when there
/// is no nvim to reuse (the caller starts a plain one).
pub fn open_in_running_nvim(
    app: &App,
    herdr_nvim: bool,
    tab: Option<&str>,
    space: Option<&str>,
    shown: &[String],
    file: &Path,
    line: u32,
) -> Option<String> {
    if herdr_nvim
        && let Some(tab) = tab
        && open_in_herdr_nvim(app, tab, space, file, line)
    {
        return Some("herdr-nvim".to_string());
    }
    let mut candidates = Vec::new();
    if let Some(tab) = tab {
        candidates.extend(terminals_with(json!({ "herdr.tab": tab })));
    }
    candidates.extend(shown.iter().filter_map(|t| terminal(t)));
    let nvim = find_nvim(candidates, &[])?;
    crate::nvim::edit_at("nvim", Some(&nvim.socket), file, line).then_some(nvim.pane_id)
}

/// The herdr-nvim link-up, in one place. herdr-nvim's contract:
/// `<app home>/mods/herdr-nvim/bin/herdr-nvim open --tab <tab id> [--space <id>]
/// <file>:<line>`
/// opens the file in that tab's nvim (starting its pane and daemon when
/// needed), focuses it, prints `{"terminal":"tN"}` and exits 0; nonzero with
/// the reason on stderr. No binary there: herdr-nvim isn't installed, and the
/// caller falls back to plain nvim.
pub fn open_in_herdr_nvim(
    app: &App,
    tab: &str,
    space: Option<&str>,
    file: &Path,
    line: u32,
) -> bool {
    let bin = app.herdr_nvim();
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

// ---- panes ------------------------------------------------------------------

/// The app's layout document (spaces → tabs → split trees), from the core.
/// `on`: another machine's (a remote space's tabs live there).
fn layout_doc(on: Option<&str>) -> Result<Value> {
    let mut params = json!({ "name": "herdr-app" });
    if let Some(machine) = on {
        params["on"] = json!(machine);
    }
    Ok(hc_rpc("doc.get", params)?
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

fn weigh(app: &App, node: &str, weight: f64) {
    let _ = app.command("layout.resize", json!({ "id": node, "weight": weight }));
}

/// A mod pane (`pane.split {mod}`) at the tab's right edge, `width_percent`
/// of the tab wide: split off the tab's root, so it spans the tab's full
/// height, when the layout document has the tab (this machine's, or for a
/// remote tab `machine/…` that machine's); else split off the tab's focused
/// pane. Returns its pane id.
pub fn open_mod_sidebar(
    app: &App,
    tab: &str,
    module: &str,
    props: Value,
    width_percent: u8,
) -> Result<String> {
    let mut tried = Vec::new();
    for target in split_targets(app, tab) {
        let mut args = json!({ "direction": "right", "pane": target, "mod": module });
        if !props.is_null() {
            args["props"] = props.clone();
        }
        let pane = app.command("pane.split", args).and_then(|reply| {
            reply
                .get("pane")
                .and_then(Value::as_str)
                .map(str::to_string)
                .context("no pane in pane.split reply")
        });
        match pane {
            Ok(pane) => {
                let f = f64::from(width_percent) / 100.0;
                weigh(app, &pane, f / (1.0 - f));
                return Ok(pane);
            }
            Err(err) => tried.push(format!("{target}: {err:#}")),
        }
    }
    if tried.is_empty() {
        bail!("tab {tab} isn't in the app's layout and isn't focused");
    }
    bail!("no pane of tab {tab} could be split ({})", tried.join("; "))
}

/// The panes to split for a pane at `tab`'s right edge, best first.
fn split_targets(app: &App, tab: &str) -> Vec<String> {
    let root_in = |doc: &Value, id: &str| {
        find_tab(doc, id)
            .and_then(|t| t.pointer("/root/id")?.as_str())
            .map(str::to_string)
    };
    let mut targets = Vec::new();
    if let Some(root) = layout_doc(None).ok().and_then(|doc| root_in(&doc, tab)) {
        targets.push(root);
    }
    if let Some((machine, id)) = tab.split_once('/')
        && let Ok(doc) = layout_doc(Some(machine))
        && let Some(root) = root_in(&doc, tab).or_else(|| root_in(&doc, id))
    {
        // Ids from that machine's own document, named as this app names
        // that machine's panes (never a bare id: it could be a pane here).
        targets.push(if root.contains('/') {
            root
        } else {
            format!("{machine}/{root}")
        });
    }
    if let Ok(status) = app.status()
        && status.get("tab").and_then(Value::as_str) == Some(tab)
        && let Some(pane) = status.get("focused_pane").and_then(Value::as_str)
    {
        targets.push(pane.to_string());
    }
    targets.dedup();
    targets
}
