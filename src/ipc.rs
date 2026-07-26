//! IPC between the two panes over a Unix domain socket.
//!
//! Framing: one JSON object per `\n`-terminated line (`serde_json`).
//! - **preview** is the listener: it binds the socket and accepts one client.
//! - **list** is the connector: it retries every 100 ms until a deadline.
//!
//! `Conn` owns a writer half plus (until `spawn_reader`) a buffered reader
//! half. `send` writes one line; `spawn_reader` moves the reader onto a thread
//! that decodes lines into an mpsc `Sender`. EOF (or a dropped receiver) ends
//! the reader thread — the channel closing is how the app learns the peer went
//! away.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::git::{ChangeKind, Scope};

/// Messages the list pane sends to the preview pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToPreview {
    /// Render the diff for this entry. Sent on every cursor move (debounced).
    Show {
        file: PathBuf,
        orig_path: Option<PathBuf>,
        scope: Scope,
        cached: bool,
        kind: ChangeKind,
        /// History view: show this commit's change to the file instead of the
        /// worktree/branch diff.
        #[serde(default)]
        commit: Option<String>,
    },
    /// Scroll the diff without switching pane focus. delta in lines;
    /// `i32::MIN`/`MAX` = home/end.
    Scroll {
        delta: i32,
    },
    /// Half/full page relative to preview's own viewport height.
    Page {
        down: bool,
        full: bool,
    },
    /// Suspend TUI, run editor on file, restore.
    Edit {
        file: PathBuf,
    },
    /// Suspend TUI, run `git -C <repo> <argv...>` interactively (e.g.
    /// `["commit", "-e"]`).
    GitInPane {
        argv: Vec<String>,
    },
    /// Nothing is selected any more (list emptied) — drop the shown diff.
    Clear,
    /// A whole-file review note added from the list pane.
    AddNote {
        file: PathBuf,
        text: String,
        /// Whether the file was in the staged section when annotated, so the
        /// notes view later re-shows the same (staged vs. worktree) diff.
        cached: bool,
    },
    /// The list asked to send the batched notes (preview owns the store).
    SendNotes,
    /// The list's notes view hovers a note: show its file + scroll to it.
    FocusNote {
        id: u64,
    },
    /// Replace a note's text (edited via the list's notes view).
    EditNote {
        id: u64,
        text: String,
    },
    DeleteNote {
        id: u64,
    },
    Quit,
}

/// Messages the preview pane sends back to the list pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToList {
    /// First message after accept; tells list the preview is alive.
    Ready,
    /// Editor exited (any exit code). List must refresh + re-Show + refocus.
    EditDone { file: PathBuf },
    /// GitInPane finished. `ok` = exit status 0.
    GitDone { ok: bool },
    /// The batched review notes changed; full snapshot for the notes view.
    Notes { notes: Vec<NoteMeta> },
    /// The preview pane asked to open the notes view (n works everywhere).
    ShowNotesView,
    /// Enter in the preview pane: open the current selection in the editor
    /// exactly as if Enter was pressed in the list.
    EditRequest,
}

/// A review note as shared between the panes. The preview owns the store and
/// allocates the ids; the list acts on notes *by id*, so a mutation between
/// snapshot and command can never hit the wrong note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: u64,
    pub file: PathBuf,
    pub start: u32,
    pub end: u32,
    pub text: String,
    /// Whether this note's diff was the staged (cached) side, so the notes
    /// view re-shows the same diff the note was written against.
    pub cached: bool,
}

/// One end of the IPC link: a write half plus an optional reader half.
pub struct Conn {
    writer: UnixStream,
    /// Taken by `spawn_reader`; `None` afterwards.
    reader: Option<BufReader<UnixStream>>,
}

impl Conn {
    /// Preview side: unlink any stale socket, bind, and block until one client
    /// connects.
    pub fn listen(sock: &Path) -> Result<Conn> {
        // A leftover socket file from a crashed run would make bind fail.
        let _ = std::fs::remove_file(sock);
        let listener = UnixListener::bind(sock)
            .with_context(|| format!("binding IPC socket {}", sock.display()))?;
        let (stream, _addr) = listener.accept().context("accepting IPC client")?;
        Conn::from_stream(stream)
    }

    /// List side: retry connecting every 100 ms until `timeout` elapses.
    pub fn connect_retry(sock: &Path, timeout: Duration) -> Result<Conn> {
        let deadline = Instant::now() + timeout;
        loop {
            match UnixStream::connect(sock) {
                Ok(stream) => return Conn::from_stream(stream),
                Err(err) => {
                    if Instant::now() >= deadline {
                        return Err(anyhow::Error::new(err)).with_context(|| {
                            format!("connecting to IPC socket {} timed out", sock.display())
                        });
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn from_stream(stream: UnixStream) -> Result<Conn> {
        // Clone the fd so the reader thread and the writer can act independently.
        let reader = BufReader::new(stream.try_clone().context("cloning IPC socket")?);
        Ok(Conn {
            writer: stream,
            reader: Some(reader),
        })
    }

    /// Serialize `msg` to one JSON line and flush it.
    pub fn send<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let mut line = serde_json::to_string(msg).context("serializing IPC message")?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .context("writing IPC message")?;
        self.writer.flush().context("flushing IPC message")?;
        Ok(())
    }

    /// Move the reader half onto a background thread that decodes each line
    /// into `T` and pushes it into `tx`. Undecodable lines (e.g. a half-written
    /// frame) are logged and skipped; EOF or a dropped `tx` ends the thread.
    /// Returns the `Conn` so the caller can keep sending.
    /// A connected in-process pair (real unix sockets, no filesystem path).
    /// Used by the scenario tests to wire the two panes together directly.
    pub fn pair() -> std::io::Result<(Conn, Conn)> {
        let (a, b) = UnixStream::pair()?;
        let make = |s: UnixStream| -> std::io::Result<Conn> {
            let reader = s.try_clone()?;
            Ok(Conn {
                writer: s,
                reader: Some(BufReader::new(reader)),
            })
        };
        Ok((make(a)?, make(b)?))
    }

    pub fn spawn_reader<T: DeserializeOwned + Send + 'static>(mut self, tx: Sender<T>) -> Conn {
        if let Some(reader) = self.reader.take() {
            std::thread::spawn(move || {
                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break, // EOF / broken pipe
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<T>(&line) {
                        Ok(msg) => {
                            if tx.send(msg).is_err() {
                                break; // receiver gone
                            }
                        }
                        Err(err) => {
                            crate::logx::log(format!("ipc: skipping undecodable line: {err}"));
                        }
                    }
                }
                // Falling out of the loop drops `tx`, closing the channel:
                // that is how the app detects the peer disconnected (EOF).
            });
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn round_trip_over_socket() {
        let dir = std::env::temp_dir().join(format!("gitview-ipc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("test.sock");

        // Preview listens on a thread (accept blocks); list connects.
        let sock_l = sock.clone();
        let listener = std::thread::spawn(move || Conn::listen(&sock_l).unwrap());
        let mut list_conn = Conn::connect_retry(&sock, Duration::from_secs(5)).unwrap();
        let mut preview_conn = listener.join().unwrap();

        let (ptx, prx) = mpsc::channel::<ToPreview>();
        preview_conn = preview_conn.spawn_reader(ptx);
        let (ltx, lrx) = mpsc::channel::<ToList>();
        list_conn = list_conn.spawn_reader(ltx);

        // list -> preview: every ToPreview variant.
        let to_preview = vec![
            ToPreview::Show {
                file: "a.rs".into(),
                orig_path: Some("old.rs".into()),
                scope: Scope::Worktree,
                cached: false,
                kind: ChangeKind::Modified,
                commit: Some("abc123".into()),
            },
            ToPreview::Scroll { delta: -3 },
            ToPreview::Page {
                down: true,
                full: false,
            },
            ToPreview::Edit {
                file: "b.rs".into(),
            },
            ToPreview::GitInPane {
                argv: vec!["commit".into(), "-e".into()],
            },
            ToPreview::Quit,
        ];
        for msg in &to_preview {
            list_conn.send(msg).unwrap();
        }
        for expected in &to_preview {
            let got = prx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(format!("{got:?}"), format!("{expected:?}"));
        }

        // preview -> list: every ToList variant.
        let to_list = vec![
            ToList::Ready,
            ToList::EditDone {
                file: "a.rs".into(),
            },
            ToList::GitDone { ok: true },
        ];
        for msg in &to_list {
            preview_conn.send(msg).unwrap();
        }
        for expected in &to_list {
            let got = lrx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(format!("{got:?}"), format!("{expected:?}"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
