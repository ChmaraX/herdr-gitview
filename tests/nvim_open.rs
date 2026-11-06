//! Enter → an nvim already running: a file name from the repo is opened as a
//! name, never run as keys or commands. A real headless nvim (skipped when
//! there's no `nvim` on `PATH`) gets files named with `|`, `<CR>`, `%`, `'`.
//!
//! Failure modes this catches:
//! - `|…` in a name running an Ex command (`:edit a|call writefile(…)`);
//! - `<CR>`/`<Esc>` in a name read as keys;
//! - `%`/`#` expanded to the current/alternate file;
//! - a `'` ending a quoted string early;
//! - the cursor not on the asked line.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use herdr_gitview::nvim::edit_at;

struct Nvim {
    child: Child,
    socket: PathBuf,
    dir: PathBuf,
}

impl Drop for Nvim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn nvim_server(name: &str) -> Option<Nvim> {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("skipped: no nvim on PATH");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("gitview-nvim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let socket = dir.join("nvim.sock");
    let child = Command::new("nvim")
        .args(["--headless", "--clean", "-n", "--listen"])
        .arg(&socket)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "nvim didn't listen");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(Nvim { child, socket, dir })
}

fn eval(socket: &Path, expr: &str) -> String {
    let out = Command::new("nvim")
        .arg("--server")
        .arg(socket)
        .args(["--remote-expr", expr])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn hostile_file_names_are_opened_as_names() {
    let Some(nvim) = nvim_server("hostile") else {
        return;
    };
    let pwned = nvim.dir.join("pwned");
    let names = [
        "a|call writefile(['x'], 'pwned')|.txt".to_string(),
        "b<CR>:call writefile(['x'], 'pwned')<CR>.txt".to_string(),
        "c%#.txt".to_string(),
        "d'quote'.txt".to_string(),
        "e with spaces.txt".to_string(),
    ];
    for (i, name) in names.iter().enumerate() {
        let file = nvim.dir.join(name);
        let body: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&file, body).unwrap();
        let line = 3 + i as u32;
        assert!(edit_at("nvim", Some(&nvim.socket), &file, line), "{name}");
        assert_eq!(
            eval(&nvim.socket, "expand('%:p')"),
            file.display().to_string(),
            "{name}"
        );
        assert_eq!(eval(&nvim.socket, "line('.')"), line.to_string(), "{name}");
    }
    assert!(!pwned.exists(), "a file name ran a command");
}
