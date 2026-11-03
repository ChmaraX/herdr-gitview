//! Typing text into a terminal without pressing Enter, through herdr-core's
//! framed API (`hc connect`, then `hello {framed: true}`): attach as a second
//! viewer, read the screen replay to learn whether the program wants
//! bracketed paste, send the bytes, detach. Ported from herdr-nvim's
//! `src/app/rpc.rs` `paste()`; no clipboard is involved.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

const HEADER: usize = 9;
const DATA: u8 = 0;
const CREDIT: u8 = 1;
const CLOSE: u8 = 2;

fn encode(channel: u32, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&channel.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn read_frame(reader: &mut impl Read) -> Option<(u32, u8, Vec<u8>)> {
    let mut header = [0u8; HEADER];
    reader.read_exact(&mut header).ok()?;
    let channel = u32::from_le_bytes(header[0..4].try_into().ok()?);
    let len = u32::from_le_bytes(header[5..9].try_into().ok()?) as usize;
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload).ok()?;
    Some((channel, header[4], payload))
}

/// Bracketed paste on: the program wants pasted text wrapped, so newlines in
/// it don't submit (every coding agent asks for this).
const PASTE_ON: &[u8] = b"\x1b[?2004h";
const PASTE_OFF: &[u8] = b"\x1b[?2004l";

/// How long the replay may take to start after the credit (a remote or a
/// busy terminal is slow), and the quiet gap that ends it once it has.
const REPLAY_WAIT: Duration = Duration::from_millis(800);
const REPLAY_QUIET: Duration = Duration::from_millis(150);

/// What the terminal's program does with pasted text, as its screen replay
/// says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    /// It asked for bracketed paste: newlines inside a paste don't submit.
    Bracketed,
    /// It didn't: every newline is Enter.
    Plain,
    /// No replay came, so there's no telling.
    Unknown,
}

impl PasteMode {
    /// The mode a replay leaves on: the last of the two switches wins; none
    /// means plain. `None` (no replay at all) is unknown.
    pub fn of(replay: Option<&[u8]>) -> PasteMode {
        let Some(replay) = replay else {
            return PasteMode::Unknown;
        };
        let last = |needle: &[u8]| replay.windows(needle.len()).rposition(|w| w == needle);
        match (last(PASTE_ON), last(PASTE_OFF)) {
            (Some(on), Some(off)) if on > off => PasteMode::Bracketed,
            (Some(_), None) => PasteMode::Bracketed,
            _ => PasteMode::Plain,
        }
    }
}

/// `text` as it may be typed: line breaks become `\n`, and every other
/// control character goes (C0 but `\n` and `\t`, DEL, C1). Text from a
/// diff is the repo's, so an ESC in it could otherwise end the paste early
/// (`\x1b[201~`, however nested) or press keys in the program.
fn printable(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

/// The bytes placing `text` sends (control characters stripped, see
/// [`printable`]; trailing newlines dropped: placing never presses Enter):
/// wrapped when the program asked for bracketed paste, else as typed. A
/// program without bracketed paste would take every newline as Enter, so
/// multi-line text is refused there, and where the mode is unknown.
pub fn paste_bytes(text: &str, mode: PasteMode) -> Result<Vec<u8>> {
    let text = printable(text);
    let text = text.trim_end_matches('\n');
    let one_line = !text.contains('\n');
    match mode {
        PasteMode::Bracketed => {
            let mut out = b"\x1b[200~".to_vec();
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(b"\x1b[201~");
            Ok(out)
        }
        _ if one_line => Ok(text.as_bytes().to_vec()),
        PasteMode::Plain => bail!(
            "this program can't take multi-line text without submitting; use shift+enter to send"
        ),
        PasteMode::Unknown => bail!(
            "couldn't read the terminal's paste mode, so multi-line text isn't placed; use shift+enter to send"
        ),
    }
}

/// The screen replay on `channel` after the credit: wait up to
/// [`REPLAY_WAIT`] for its first frame, then until a [`REPLAY_QUIET`] gap
/// (still within the deadline). `None`: no frame came at all.
fn read_replay(rx: &Receiver<(u32, u8, Vec<u8>)>, channel: u32) -> Option<Vec<u8>> {
    let deadline = Instant::now() + REPLAY_WAIT;
    let mut replay: Option<Vec<u8>> = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = match replay {
            Some(_) => REPLAY_QUIET.min(left),
            None => left,
        };
        if wait.is_zero() {
            break;
        }
        match rx.recv_timeout(wait) {
            Ok((ch, DATA, payload)) if ch == channel => {
                replay.get_or_insert_with(Vec::new).extend(payload);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    replay
}

fn reply(method: &str, message: Value) -> Result<Value> {
    if let Some(error) = message.get("error") {
        let kind = error
            .pointer("/data/kind")
            .and_then(Value::as_str)
            .unwrap_or("error");
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        bail!("{method} failed: {kind} {text}");
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

/// Paste `text` into `terminal` without pressing Enter, like a person
/// pasting: attach at the terminal's own size (so nothing resizes), read the
/// replay for the program's paste mode, send the bytes, detach.
pub fn paste(terminal: &str, text: &str) -> Result<()> {
    let facts =
        crate::apphost::terminal(terminal).with_context(|| format!("terminal {terminal}"))?;
    let cols = facts.get("cols").and_then(Value::as_u64).unwrap_or(80);
    let rows = facts.get("rows").and_then(Value::as_u64).unwrap_or(24);

    let bin = crate::apphost::hc_bin();
    let mut child = Command::new(&bin)
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("spawning {} connect", bin.to_string_lossy()))?;
    let result = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("hc connect: no stdin")?;
        let mut reader = BufReader::new(child.stdout.take().context("hc connect: no stdout")?);
        stdin.write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{\"framed\":true}}\n",
        )?;
        stdin.flush()?;
        // The hello line and then the frames, read on a thread so a core
        // that doesn't answer can't hold us past the deadlines below.
        let (hello_tx, hello_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut hello = String::new();
            let ok = reader.read_line(&mut hello).is_ok_and(|n| n > 0);
            let _ = hello_tx.send(hello);
            if !ok {
                return;
            }
            while let Some(frame) = read_frame(&mut reader) {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });

        let hello = hello_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("hello: no reply from herdr-core"))?;
        reply(
            "hello",
            serde_json::from_str(&hello).context("bad hello reply")?,
        )?;

        let attach = json!({"jsonrpc": "2.0", "id": 1, "method": "terminal.attach",
            "params": {"terminal": terminal, "cols": cols, "rows": rows, "history": 0}});
        stdin.write_all(&encode(0, DATA, attach.to_string().as_bytes()))?;
        stdin.flush()?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let channel = loop {
            let (channel, _kind, payload) = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| anyhow!("terminal.attach: no reply from herdr-core"))?;
            if channel != 0 {
                continue;
            }
            let message: Value = serde_json::from_slice(&payload)?;
            if message.get("id").and_then(Value::as_u64) != Some(1) {
                continue;
            }
            let result = reply("terminal.attach", message)?;
            break result
                .get("channel")
                .and_then(Value::as_u64)
                .context("terminal.attach returned no channel")? as u32;
        };
        stdin.write_all(&encode(channel, CREDIT, &(1u32 << 20).to_le_bytes()))?;
        stdin.flush()?;

        let replay = read_replay(&rx, channel);
        let bytes = paste_bytes(text, PasteMode::of(replay.as_deref()));
        if let Ok(bytes) = &bytes {
            stdin.write_all(&encode(channel, DATA, bytes))?;
        }
        stdin.write_all(&encode(channel, CLOSE, &[]))?;
        stdin.flush()?;
        // Let the core take the input before the connection goes.
        std::thread::sleep(Duration::from_millis(200));
        bytes.map(|_| ())
    })();
    crate::deadline::kill_group(&mut child);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    use PasteMode::*;

    #[test]
    fn paste_mode_follows_the_last_switch_in_the_replay() {
        let of = |r: &[u8]| PasteMode::of(Some(r));
        assert_eq!(of(b"plain shell output"), Plain);
        assert_eq!(of(b""), Plain);
        assert_eq!(of(b"\x1b[?1049h\x1b[?2004hprompt> "), Bracketed);
        assert_eq!(of(b"\x1b[?2004h...\x1b[?2004l$ "), Plain);
        assert_eq!(of(b"\x1b[?2004l...\x1b[?2004h> "), Bracketed);
        assert_eq!(PasteMode::of(None), Unknown);
    }

    #[test]
    fn multi_line_text_is_placed_only_where_newlines_cant_submit() {
        assert_eq!(
            paste_bytes("a\r\nb\n", Bracketed).unwrap(),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes("one line\n", Plain).unwrap(), b"one line");
        assert_eq!(paste_bytes("one line", Unknown).unwrap(), b"one line");
        let err = paste_bytes("a\nb", Plain).unwrap_err().to_string();
        assert!(err.contains("use shift+enter to send"), "{err}");
        assert!(
            paste_bytes("a\rb", Plain).is_err(),
            "a lone CR is Enter too"
        );
        let err = paste_bytes("a\nb", Unknown).unwrap_err().to_string();
        assert!(
            err.contains("couldn't read the terminal's paste mode"),
            "{err}"
        );
    }

    #[test]
    fn text_from_a_repo_can_neither_end_the_paste_nor_press_keys() {
        // The end sequence, plain and nested so one removal pass would
        // rebuild it, and keys a program reacts to (Esc, shift-tab, ^C).
        let hostile = "a\x1b[201~b \x1b[20\x1b[201~1~ \x1b[Z\x03\x7f\u{9b}201~\tz";
        let bytes = paste_bytes(hostile, Bracketed).unwrap();
        let inner = &bytes[6..bytes.len() - 6];
        assert_eq!(inner, b"a[201~b [20[201~1~ [Z201~\tz".as_slice());
        assert!(!inner.iter().any(|&b| b == 0x1b || b == 0x03));
        // Typed as is, it's stripped the same way.
        assert_eq!(paste_bytes("x\x1b[Zy", Plain).unwrap(), b"x[Zy");
    }
}
