//! Running a program with a deadline: for the app's calls to programs that
//! could hang (herdr-core's `hc connect`, `herdr-app`, herdr-nvim, an
//! nvim's `--remote-expr`), so an op the mods wait on always ends.

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

/// `cmd`'s output, like [`Command::output`], with `input` on its stdin
/// (none: stdin is closed) — unless it takes longer than `limit`: then it
/// and everything it started in its process group are killed, and it's an
/// error naming the program.
pub fn output_within(cmd: &mut Command, input: Option<&[u8]>, limit: Duration) -> Result<Output> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let deadline = Instant::now() + limit;
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    new_group(cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot run {program}"))?;
    let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let input = input.to_vec();
        // A program that doesn't read its input mustn't block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    for (is_out, pipe) in [
        (
            true,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            false,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ] {
        let tx = tx.clone();
        let mut pipe = pipe.context("no pipe")?;
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx.send((is_out, buf));
        });
    }
    drop(tx);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((true, buf)) => stdout = buf,
            Ok((false, buf)) => stderr = buf,
            Err(_) => return Err(kill(&mut child, &program, limit)),
        }
    }
    // Both pipes closed; the exit is a moment away.
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            return Err(kill(&mut child, &program, limit));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Make `cmd` its own process group's leader (Unix) / without a console
/// window, so [`kill_group`] can take it and its children down (Windows).
pub fn new_group(cmd: &mut Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Kill `child` and its process group, and reap it. The child must have
/// been spawned as its group's leader (`process_group(0)`), and not reaped
/// yet (so its id is still its own).
pub fn kill_group(child: &mut std::process::Child) {
    // SAFETY: kill(2) with a negative pid signals that process group: the
    // child's own, which nothing else of ours belongs to.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    // Windows: the child and its whole tree.
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// [`kill_group`], and say what took too long.
fn kill(child: &mut std::process::Child, program: &str, limit: Duration) -> anyhow::Error {
    kill_group(child);
    let name = std::path::Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    anyhow!(
        "{name} took more than {} s and was stopped",
        limit.as_secs()
    )
}
