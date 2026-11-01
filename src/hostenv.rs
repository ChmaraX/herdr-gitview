//! The host environment, resolved once at startup and passed down
//! explicitly. Nothing below `run()` reads process env vars for host
//! interaction — which is what makes the panes drivable in-process by the
//! scenario tests (with a fake herdr binary and temp paths).
//!
//! gitview runs under one of two hosts:
//! - [`Host::Herdr`]: the herdr TUI (0.7–0.9), as a plugin. Pane ids are
//!   herdr pane ids and every host call goes through the `herdr` CLI.
//! - [`Host::App`]: the Herdr desktop app. Panes are herdr-core terminals
//!   (`HC_TERMINAL_ID`, e.g. `t7`); window changes go through `herdr-app do`
//!   and terminals through `hc` (see [`crate::apphost`]). Selected with
//!   `GITVIEW_HOST=app`, which the app mods and the orchestrator set.

use std::ffi::OsString;
use std::path::PathBuf;

/// Which herdr is hosting the panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Host {
    /// The herdr TUI plugin host (the default, unchanged behavior).
    #[default]
    Herdr,
    /// The Herdr desktop app + herdr-core (`herdr-app`, `hc`).
    App,
}

impl Host {
    /// `GITVIEW_HOST=app` selects the desktop app; anything else is herdr.
    pub fn from_process() -> Host {
        match std::env::var("GITVIEW_HOST").as_deref() {
            Ok("app") => Host::App,
            _ => Host::Herdr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostEnv {
    /// The herdr CLI to shell out to (`HERDR_BIN_PATH`, default `herdr`).
    pub herdr_bin: OsString,
    /// Our own pane id (`HERDR_PANE_ID`; under the app our terminal address,
    /// `HC_TERMINAL_ID`); `None` = standalone mode.
    pub own_pane: Option<String>,
    /// The preview pane's id (`GITVIEW_PREVIEW_PANE`, list side only).
    pub preview_pane: Option<String>,
    /// The view's IPC socket (`GITVIEW_SOCKET`); also the base path for
    /// popup answer files and the nvim remote socket.
    pub socket: Option<PathBuf>,
    /// Which host the panes live in.
    pub host: Host,
}

impl HostEnv {
    pub fn from_process() -> HostEnv {
        let host = Host::from_process();
        let own_pane = match host {
            Host::Herdr => std::env::var("HERDR_PANE_ID").ok(),
            Host::App => std::env::var("HC_TERMINAL_ID").ok(),
        };
        HostEnv {
            herdr_bin: std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()),
            own_pane,
            preview_pane: std::env::var("GITVIEW_PREVIEW_PANE").ok(),
            socket: std::env::var_os("GITVIEW_SOCKET").map(PathBuf::from),
            host,
        }
    }

    pub fn in_herdr(&self) -> bool {
        self.own_pane.is_some()
    }

    /// The nvim remote-control socket, derived from the IPC socket path.
    pub fn nvim_server(&self) -> Option<PathBuf> {
        let mut path = self.socket.clone()?;
        path.set_extension("nvim");
        Some(path)
    }

    /// Focus one of our panes (best-effort, fire-and-forget). Used for the
    /// editor handoff: list → preview when an edit starts, preview → list
    /// when it ends.
    pub fn focus_pane(&self, pane: &str) {
        match self.host {
            Host::Herdr => crate::herdr_cli::focus_pane(&self.herdr_bin, pane),
            Host::App => crate::apphost::focus_terminal(pane),
        }
    }

    /// An nvim already running in another pane of our tab.
    pub fn find_nvim_in_tab(&self, exclude: &[&str]) -> Option<crate::herdr_cli::TabNvim> {
        let own = self.own_pane.as_deref()?;
        match self.host {
            Host::Herdr => crate::herdr_cli::find_nvim_in_tab(&self.herdr_bin, own, exclude),
            Host::App => crate::apphost::find_nvim_in_tab(own, exclude),
        }
    }

    /// The agents next to us (same herdr workspace / app space).
    pub fn agents(&self) -> Vec<crate::popup::AgentRow> {
        match self.host {
            Host::Herdr => crate::popup::workspace_agents(),
            Host::App => crate::apphost::space_agents(self.own_pane.as_deref()),
        }
    }

    /// Type `text` into an agent pane's input; `submit` also presses enter.
    pub fn send_text(&self, pane: &str, text: &str, submit: bool) -> anyhow::Result<()> {
        match self.host {
            Host::Herdr => {
                let out = std::process::Command::new(&self.herdr_bin)
                    .args(["pane", "send-text", pane, text])
                    .output()?;
                if !out.status.success() {
                    anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
                }
                if submit {
                    let _ = std::process::Command::new(&self.herdr_bin)
                        .args(["pane", "send-keys", pane, "enter"])
                        .output();
                }
                Ok(())
            }
            Host::App => crate::apphost::send_text(pane, text, submit),
        }
    }
}
