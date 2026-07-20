//! The herdr host environment, resolved once at startup and passed down
//! explicitly. Nothing below `run()` reads process env vars for host
//! interaction — which is what makes the panes drivable in-process by the
//! scenario tests (with a fake herdr binary and temp paths).

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HostEnv {
    /// The herdr CLI to shell out to (`HERDR_BIN_PATH`, default `herdr`).
    pub herdr_bin: OsString,
    /// Our own pane id (`HERDR_PANE_ID`); `None` = standalone mode.
    pub own_pane: Option<String>,
    /// The preview pane's id (`GITVIEW_PREVIEW_PANE`, list side only).
    pub preview_pane: Option<String>,
    /// The view's IPC socket (`GITVIEW_SOCKET`); also the base path for
    /// popup answer files and the nvim remote socket.
    pub socket: Option<PathBuf>,
}

impl HostEnv {
    pub fn from_process() -> HostEnv {
        HostEnv {
            herdr_bin: std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()),
            own_pane: std::env::var("HERDR_PANE_ID").ok(),
            preview_pane: std::env::var("GITVIEW_PREVIEW_PANE").ok(),
            socket: std::env::var_os("GITVIEW_SOCKET").map(PathBuf::from),
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
}
