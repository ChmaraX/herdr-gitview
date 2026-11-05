//! The app's trace: with `GITVIEW_TRACE=<file>` set (the mods pass it),
//! each `json <op>` appends one JSON line to that file, so the lead can see
//! what the engine did for the person, in order and with timings, beside
//! the mods' own lines. Unset: nothing is written.
//!
//! A line: `{t, mod: "engine", kind: "exec", pid, op, args, ms, git,
//! execs, remote, ok, same, error?}`.
//! - `t`: when the op ended, ms since the epoch; `ms`: how long it took.
//! - `args`: its `--key value`s (each value clipped) and `--flag`s (true);
//!   stage/unstage/discard's paths as `paths`; any other op's positional
//!   text (a commit message, notes, what's sent) only as `text_bytes`.
//! - `git`: the git commands it started (each over `hc exec` on a remote);
//!   `execs`: its `hc exec` round trips; `remote`: whether `--on` a machine
//!   (`args.on` names it).
//! - `ok`, and `error`'s first line; `same`: a `--unless` hit.
//!
//! One write per line, appended, so lines from ops at once never mix. Past
//! [`MAX_BYTES`] the file moves to `<name>.1.jsonl` (one old file kept); two
//! ops rotating at once cost a line or two at most.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

static GIT: AtomicUsize = AtomicUsize::new(0);
static EXECS: AtomicUsize = AtomicUsize::new(0);

/// A git command started.
pub fn count_git() {
    GIT.fetch_add(1, Ordering::Relaxed);
}

/// An `hc exec` round trip started.
pub fn count_exec() {
    EXECS.fetch_add(1, Ordering::Relaxed);
}

/// The ops whose positional arguments are paths (fine to trace); every
/// other op's are text: only sized.
const PATH_OPS: [&str; 3] = ["stage", "unstage", "discard"];

/// Most paths a line lists.
const MAX_PATHS: usize = 20;

/// Most bytes of a value a line keeps.
const MAX_VALUE: usize = 160;

/// The trace file moves aside past this size.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// One op being traced: [`start`], then [`Op::end`] with its outcome.
pub struct Op {
    file: PathBuf,
    started: Instant,
}

/// Start tracing an op, when `GITVIEW_TRACE` is set.
pub fn start() -> Option<Op> {
    let file = std::env::var_os("GITVIEW_TRACE").filter(|f| !f.is_empty())?;
    Some(Op {
        file: file.into(),
        started: Instant::now(),
    })
}

impl Op {
    /// Append the op's line: its name, its `--key value`s (`None` for a
    /// flag) and positional arguments, and its reply or error. Never
    /// fails: a trace must not break the op.
    pub fn end(
        self,
        op: &str,
        pairs: &[(String, Option<String>)],
        positional: &[String],
        outcome: Result<&Value, &anyhow::Error>,
    ) {
        let ms = (self.started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0;
        let args: Map<String, Value> = pairs
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_deref().map_or(json!(true), |v| json!(short(v))),
                )
            })
            .collect();
        let mut line = Map::new();
        line.extend([
            ("t".into(), json!(now_ms())),
            ("mod".into(), json!("engine")),
            ("kind".into(), json!("exec")),
            ("pid".into(), json!(std::process::id())),
            ("op".into(), json!(op)),
            ("remote".into(), json!(args.contains_key("on"))),
            ("args".into(), Value::Object(args)),
        ]);
        if PATH_OPS.contains(&op) {
            let mut paths: Vec<Value> = positional
                .iter()
                .take(MAX_PATHS)
                .map(|p| json!(short(p)))
                .collect();
            if positional.len() > MAX_PATHS {
                paths.push(json!(format!("+{}", positional.len() - MAX_PATHS)));
            }
            line.insert("paths".into(), Value::Array(paths));
        } else if !positional.is_empty() {
            let bytes: usize = positional.iter().map(String::len).sum();
            line.insert("text_bytes".into(), json!(bytes));
        }
        let same = outcome.is_ok_and(|r| r.get("same") == Some(&json!(true)));
        line.extend([
            ("ms".into(), json!(ms)),
            ("git".into(), json!(GIT.load(Ordering::Relaxed))),
            ("execs".into(), json!(EXECS.load(Ordering::Relaxed))),
            ("ok".into(), json!(outcome.is_ok())),
            ("same".into(), json!(same)),
        ]);
        if let Err(err) = outcome {
            let msg = format!("{err:#}");
            let first = msg.lines().next().unwrap_or_default();
            line.insert("error".into(), json!(short(first)));
        }
        let mut text = Value::Object(line).to_string();
        text.push('\n');
        append(&self.file, &text);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// `s` on one line, at most [`MAX_VALUE`] bytes of it.
fn short(s: &str) -> String {
    let s = s.trim();
    let mut end = s.len().min(MAX_VALUE);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].replace('\n', "⏎");
    if end < s.len() {
        out.push('…');
    }
    out
}

/// Append `text` to `file` in one write, after moving it to
/// `<name>.1.jsonl` (over the last one) once it's past [`MAX_BYTES`].
fn append(file: &Path, text: &str) {
    if std::fs::metadata(file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let _ = std::fs::rename(file, file.with_file_name(format!("{stem}.1.jsonl")));
    }
    let open = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
    };
    let opened = open().or_else(|_| {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        open()
    });
    if let Ok(mut f) = opened {
        // One write: lines from ops at once never mix.
        let _ = f.write_all(text.as_bytes());
    }
}
