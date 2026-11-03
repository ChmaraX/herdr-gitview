//! Typing text into a terminal without pressing Enter, through herdr-core's
//! framed API (`hc connect`, then `hello {framed: true}`): attach as a second
//! viewer, read the screen replay to learn whether the program wants
//! bracketed paste, send the bytes, detach. Ported from herdr-nvim's
//! `src/app/rpc.rs` `paste()`; no clipboard is involved.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
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

/// Whether the screen replay leaves bracketed paste on: the last of the two
/// mode switches wins; none means off.
pub fn bracketed_paste_on(replay: &[u8]) -> bool {
    let last = |needle: &[u8]| replay.windows(needle.len()).rposition(|w| w == needle);
    match (last(PASTE_ON), last(PASTE_OFF)) {
        (Some(on), Some(off)) => on > off,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The bytes placing `text` sends (trailing newlines dropped: placing never
/// presses Enter): wrapped when the program asked for bracketed paste, else
/// as typed. A program without bracketed paste would take every newline as
/// Enter, so multi-line text is refused there.
pub fn paste_bytes(text: &str, bracketed: bool) -> Result<Vec<u8>> {
    let text = text.trim_end_matches(['\n', '\r']);
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        Ok(out)
    } else if text.contains(['\n', '\r']) {
        bail!("this program can't take multi-line text without submitting; use shift+enter to send")
    } else {
        Ok(text.as_bytes().to_vec())
    }
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
        crate::apphost::terminal(terminal).with_context(|| format!("no terminal {terminal}"))?;
    let cols = facts.get("cols").and_then(Value::as_u64).unwrap_or(80);
    let rows = facts.get("rows").and_then(Value::as_u64).unwrap_or(24);

    let bin = crate::apphost::hc_bin();
    let mut child = Command::new(&bin)
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {} connect", bin.to_string_lossy()))?;
    let result = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("hc connect: no stdin")?;
        let mut reader = BufReader::new(child.stdout.take().context("hc connect: no stdout")?);
        stdin.write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{\"framed\":true}}\n",
        )?;
        stdin.flush()?;
        let mut hello = String::new();
        reader.read_line(&mut hello)?;
        reply(
            "hello",
            serde_json::from_str(&hello).context("bad hello reply")?,
        )?;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(frame) = read_frame(&mut reader) {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });

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

        // The replay arrives right after the attach; a short quiet gap ends it.
        let mut replay = Vec::new();
        let replay_deadline = Instant::now() + Duration::from_millis(800);
        while let Ok((frame_channel, kind, payload)) = rx.recv_timeout(
            Duration::from_millis(150)
                .min(replay_deadline.saturating_duration_since(Instant::now())),
        ) {
            if frame_channel == channel && kind == DATA {
                replay.extend_from_slice(&payload);
            }
            if Instant::now() >= replay_deadline {
                break;
            }
        }

        let bytes = paste_bytes(text, bracketed_paste_on(&replay));
        if let Ok(bytes) = &bytes {
            stdin.write_all(&encode(channel, DATA, bytes))?;
        }
        stdin.write_all(&encode(channel, CLOSE, &[]))?;
        stdin.flush()?;
        // Let the core take the input before the connection goes.
        std::thread::sleep(Duration::from_millis(200));
        bytes.map(|_| ())
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_mode_follows_the_last_switch_in_the_replay() {
        assert!(!bracketed_paste_on(b"plain shell output"));
        assert!(bracketed_paste_on(b"\x1b[?1049h\x1b[?2004hprompt> "));
        assert!(!bracketed_paste_on(b"\x1b[?2004h...\x1b[?2004l$ "));
        assert!(bracketed_paste_on(b"\x1b[?2004l...\x1b[?2004h> "));
    }

    #[test]
    fn multi_line_text_is_placed_only_where_newlines_cant_submit() {
        assert_eq!(
            paste_bytes("a\nb\n", true).unwrap(),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        // A payload can't end the paste early.
        assert_eq!(
            paste_bytes("x\x1b[201~y", true).unwrap(),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes("one line\n", false).unwrap(), b"one line");
        let err = paste_bytes("a\nb", false).unwrap_err().to_string();
        assert!(err.contains("use shift+enter to send"), "{err}");
        assert!(paste_bytes("a\rb", false).is_err());
    }
}
