//! Where a repo's commands run: this machine, or another one reached through
//! herdr-core (`hc exec --on <machine> --cwd <dir> [-i] -- …`). [`Machine`]
//! is the one place that tells the two apart: git.rs asks it for a git
//! command, a file's bytes, or the facts about untracked files, and never
//! checks which machine it is on.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

/// The `hc` binary: `GITVIEW_HC`, else `hc` on `PATH`.
pub fn hc_bin() -> OsString {
    std::env::var_os("GITVIEW_HC").unwrap_or_else(|| "hc".into())
}

/// The git this machine runs: `GITVIEW_GIT`, else `git` on `PATH` — but on
/// macOS, when that's Apple's `/usr/bin/git` shim (it looks for the real
/// one through xcrun on every call, about 6 ms each), the real one in the
/// selected developer folder (`DEVELOPER_DIR`, else `xcode-select`'s), when
/// it's there.
pub fn git_bin() -> &'static OsStr {
    static BIN: OnceLock<OsString> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(bin) = std::env::var_os("GITVIEW_GIT").filter(|b| !b.is_empty()) {
            return bin;
        }
        if cfg!(target_os = "macos")
            && crate::editor::on_path("git").as_deref() == Some("/usr/bin/git")
        {
            let developer = std::env::var_os("DEVELOPER_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::fs::read_link("/var/db/xcode_select_link").ok())
                .unwrap_or_else(|| "/Library/Developer/CommandLineTools".into());
            let real = developer.join("usr/bin/git");
            if crate::editor::executable(&real) {
                return real.into_os_string();
            }
        }
        "git".into()
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Machine {
    #[default]
    Local,
    /// A machine from `hc machine ls`.
    On(String),
}

/// How a command is run there.
#[derive(Debug, Clone, Copy, Default)]
pub struct Run {
    /// We pipe input in (else its stdin is closed).
    pub stdin: bool,
    /// Seconds before hc ends it (default [`READ_LIMIT`]). Here: no limit.
    pub timeout: Option<u32>,
}

/// What [`Machine::measure`] learns about one file, for the untracked line
/// counts (see `git::untracked_stats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    pub size: u64,
    /// Lines, the last one counted without its newline too; `None` when
    /// not counted (too big, binary).
    pub lines: Option<u32>,
}

/// Seconds a call there may take unless it says otherwise: under the app's
/// own 30 s, so a machine that hangs gives a reason, not the app's stop.
pub const READ_LIMIT: u32 = 25;

/// Untracked files are counted at most this big (bigger: no counts).
pub const MAX_COUNTED: u64 = 1024 * 1024;

/// Files per remote measuring call (keeps the argv well under limits).
const MEASURE_BATCH: usize = 200;

/// Exit code of [`READ`] for a file that isn't there.
const MISSING: i32 = 44;
const READ: &str = r#"[ -e "$1" ] || exit 44; exec cat -- "$1""#;

/// Exit code of [`READ_AT_MOST`] for a file over the limit (its size on
/// stdout).
const TOO_BIG: i32 = 45;
const READ_AT_MOST: &str = r#"[ -f "$1" ] || exit 44; n=$(($(wc -c < "$1"))) || exit 1; [ "$n" -le "$2" ] || { echo "$n"; exit 45; }; exec cat -- "$1""#;

/// A file's bytes, read only up to a limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contents {
    /// No such file (or not a regular file) there.
    Missing,
    /// Over the limit: its size, nothing read.
    TooBig(u64),
    Bytes(Vec<u8>),
}

/// One process for sizes, one for line counts, however many files: `wc -c`
/// gives sizes, `grep -c -I` counts lines (an unterminated last one too)
/// and gives 0 for a binary file. Rust lines the answers up by name.
const MEASURE: &str = r#"wc -c -- "$@" 2>/dev/null; echo '--gitview--'; grep -H -c -I -e '' -- "$@" 2>/dev/null; exit 0"#;

impl Machine {
    /// `--on` as given: `None` or `""` is this machine.
    pub fn parse(on: Option<&str>) -> Machine {
        match on.filter(|m| !m.is_empty()) {
            None => Machine::Local,
            Some(m) => Machine::On(m.to_string()),
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Machine::Local => None,
            Machine::On(m) => Some(m),
        }
    }

    /// `hc exec --on <m> --cwd <cwd> [--timeout N] [-i] -- <argv…>`.
    fn hc_exec(machine: &str, cwd: &Path, run: Run, argv: &[&str]) -> Command {
        crate::trace::count_exec();
        let mut cmd = Command::new(hc_bin());
        cmd.args(["exec", "--on", machine]).arg("--cwd").arg(cwd);
        let secs = run.timeout.unwrap_or(READ_LIMIT);
        cmd.args(["--timeout", &secs.to_string()]);
        if run.stdin {
            cmd.arg("-i");
        }
        cmd.arg("--").args(argv);
        cmd
    }

    /// `argv` in the folder `dir` there.
    pub fn command(&self, dir: &Path, argv: &[&str], run: Run) -> Command {
        match self {
            Machine::Local => {
                let mut cmd = Command::new(argv[0]);
                cmd.args(&argv[1..]).current_dir(dir);
                cmd
            }
            Machine::On(m) => Machine::hc_exec(m, dir, run, argv),
        }
    }

    /// `git <args>` in the repo (or folder) `dir`. Reads never take optional
    /// locks: a poll holding the index lock made the person's own
    /// merge/commit fail ("Unable to write index").
    pub fn git(&self, dir: &Path, args: &[&str], run: Run) -> Command {
        crate::trace::count_git();
        match self {
            Machine::Local => {
                let mut cmd = Command::new(git_bin());
                cmd.arg("-C")
                    .arg(dir)
                    .env("GIT_OPTIONAL_LOCKS", "0")
                    .args(args);
                cmd
            }
            Machine::On(m) => {
                let mut argv = vec!["git", "--no-optional-locks"];
                argv.extend(args);
                Machine::hc_exec(m, dir, run, &argv)
            }
        }
    }

    /// Run a command built by this machine; the error names hc's own
    /// failure (unknown machine, unreachable) rather than the program's.
    pub fn output(&self, mut cmd: Command, what: &str) -> Result<Output> {
        let program = cmd.get_program().to_string_lossy().into_owned();
        cmd.output()
            .with_context(|| format!("spawning {program} for {what}"))
    }

    /// The bytes of the file `path` (relative to `dir`) there; `None` when
    /// there's no such file. Here any read error counts as missing (as the
    /// TUI always had it); there, only a missing file does, and a failed
    /// call is an error (not a deleted file).
    pub fn read(&self, dir: &Path, path: &Path) -> Result<Option<Vec<u8>>> {
        match self {
            Machine::Local => Ok(std::fs::read(dir.join(path)).ok()),
            Machine::On(m) => {
                let name = path.to_string_lossy();
                let cmd =
                    Machine::hc_exec(m, dir, Run::default(), &["sh", "-c", READ, "sh", &name]);
                let out = self.output(cmd, "a file read")?;
                match out.status.code() {
                    Some(0) => Ok(Some(out.stdout)),
                    Some(MISSING) => Ok(None),
                    _ => bail!(
                        "reading {} on {m}: {}",
                        path.display(),
                        remote_error(&String::from_utf8_lossy(&out.stderr))
                    ),
                }
            }
        }
    }

    /// The bytes of the regular file `path` (relative to `dir`) there, exact,
    /// unless it's over `max` bytes. There it's one call, which checks the
    /// size before reading; a failed call is an error.
    pub fn read_at_most(&self, dir: &Path, path: &Path, max: u64) -> Result<Contents> {
        match self {
            Machine::Local => {
                let full = dir.join(path);
                let Ok(meta) = std::fs::metadata(&full) else {
                    return Ok(Contents::Missing);
                };
                if !meta.is_file() {
                    return Ok(Contents::Missing);
                }
                if meta.len() > max {
                    return Ok(Contents::TooBig(meta.len()));
                }
                let bytes =
                    std::fs::read(&full).with_context(|| format!("reading {}", full.display()))?;
                Ok(match bytes.len() as u64 {
                    n if n > max => Contents::TooBig(n), // grew meanwhile
                    _ => Contents::Bytes(bytes),
                })
            }
            Machine::On(m) => {
                let name = path.to_string_lossy();
                let max = max.to_string();
                let argv = ["sh", "-c", READ_AT_MOST, "sh", &name, &max];
                let out = self.output(
                    Machine::hc_exec(m, dir, Run::default(), &argv),
                    "a file read",
                )?;
                match out.status.code() {
                    Some(0) => Ok(Contents::Bytes(out.stdout)),
                    Some(MISSING) => Ok(Contents::Missing),
                    Some(TOO_BIG) => Ok(Contents::TooBig(
                        String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .parse()
                            .context("the size of a file over the limit")?,
                    )),
                    _ => bail!(
                        "reading {} on {m}: {}",
                        path.display(),
                        remote_error(&String::from_utf8_lossy(&out.stderr))
                    ),
                }
            }
        }
    }

    /// Size and line count of each of `paths` (relative to `dir`), in order;
    /// `None` for a file that can't be read. Here each file is read in
    /// process; there, every batch is one `hc exec` running two programs.
    pub fn measure(&self, dir: &Path, paths: &[&Path]) -> Vec<Option<FileFacts>> {
        match self {
            Machine::Local => paths.iter().map(|p| measure_local(&dir.join(p))).collect(),
            Machine::On(m) => {
                let mut facts = Vec::with_capacity(paths.len());
                for (names, asked) in batches(paths) {
                    let mut argv = vec!["sh", "-c", MEASURE, "sh"];
                    argv.extend(asked.iter().map(String::as_str));
                    let out = if asked.is_empty() {
                        String::new()
                    } else {
                        self.output(
                            Machine::hc_exec(m, dir, Run::default(), &argv),
                            "line counts",
                        )
                        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                        .unwrap_or_default()
                    };
                    facts.extend(parse_measure(&out, &names));
                }
                facts
            }
        }
    }
}

/// [`Machine::fingerprints`] there, for the files named after `sh`: a line
/// of `1`/`0` (a regular file or not), then `git hash-object`'s ids of the
/// files, one per line.
const FINGERPRINT: &str = r#"for f; do if [ -f "$f" ]; then printf 1; else printf 0; fi; done; echo
for f; do if [ -f "$f" ]; then printf '%s\n' "$f"; fi; done | "${GITVIEW_GIT_BIN:-git}" hash-object --no-filters --stdin-paths"#;

impl Machine {
    /// A fingerprint of each of `paths` (relative to `dir`), in order, that
    /// changes when the file's content does: its git object id (`git
    /// hash-object --no-filters`, one `sh` per batch, through `hc exec`
    /// there), so it's the same here and there. `-` when it isn't a file.
    /// One that can't be told (a failed call, a newline in the name) is a
    /// value never given before, so it never looks unchanged.
    pub fn fingerprints(&self, dir: &Path, paths: &[&Path]) -> Vec<String> {
        let mut out = Vec::with_capacity(paths.len());
        for (names, asked) in batches(paths) {
            let mut argv = vec!["sh", "-c", FINGERPRINT, "sh"];
            argv.extend(asked.iter().map(String::as_str));
            let answer = if asked.is_empty() {
                None
            } else {
                let mut cmd = self.command(dir, &argv, Run::default());
                if *self == Machine::Local {
                    cmd.env("GITVIEW_GIT_BIN", git_bin());
                }
                self.output(cmd, "fingerprints")
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            };
            let mut told = answer
                .as_deref()
                .and_then(|a| parse_fingerprints(a, asked.len()));
            out.extend(names.iter().map(|n| match told.as_mut() {
                Some(told) if !n.contains('\n') => told.next().unwrap_or_else(unknown),
                _ => unknown(),
            }));
        }
        out
    }
}

/// `paths` as names, [`MEASURE_BATCH`] at a time, each batch with the names
/// a script there may be given: not one with a newline (it would split the
/// answers' lines).
fn batches<'a>(paths: &'a [&Path]) -> impl Iterator<Item = (Vec<String>, Vec<String>)> + 'a {
    paths.chunks(MEASURE_BATCH).map(|batch| {
        let names: Vec<String> = batch
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let asked = names
            .iter()
            .filter(|n| !n.contains('\n'))
            .cloned()
            .collect();
        (names, asked)
    })
}

/// [`FINGERPRINT`]'s answer for `n` names: each one's id, or `-`. `None`
/// when it doesn't add up (a file went in between).
fn parse_fingerprints(answer: &str, n: usize) -> Option<std::vec::IntoIter<String>> {
    let mut lines = answer.lines();
    let flags = lines.next()?;
    if flags.len() != n {
        return None;
    }
    let mut ids = lines.map(str::to_string);
    let told: Option<Vec<String>> = flags
        .chars()
        .map(|c| {
            if c == '1' {
                ids.next()
            } else {
                Some("-".to_string())
            }
        })
        .collect();
    if ids.next().is_some() {
        return None;
    }
    told.map(Vec::into_iter)
}

/// A fingerprint never given before (this process's id, the time, a count).
fn unknown() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "?{}.{now}.{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

/// [`FileFacts`] for a file here: lines counted only up to
/// [`MAX_COUNTED`] bytes and without a NUL in the first 8 KiB.
fn measure_local(path: &Path) -> Option<FileFacts> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_COUNTED {
        return Some(FileFacts { size, lines: None });
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Some(FileFacts { size, lines: None });
    }
    let mut lines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        lines += 1; // unterminated last line still counts
    }
    Some(FileFacts {
        size,
        lines: Some(lines),
    })
}

/// Line [`MEASURE`]'s answers up with `names`: `wc -c` lines are
/// `<size> <name>` (plus a `total`), `grep -c` lines `<name>:<count>`. A
/// name missing from either answer was unreadable there.
fn parse_measure(out: &str, names: &[String]) -> Vec<Option<FileFacts>> {
    let (sizes, counts) = out.split_once("--gitview--\n").unwrap_or((out, ""));
    let size_of = |name: &str| {
        sizes.lines().find_map(|l| {
            let (n, rest) = l.trim_start().split_once(' ')?;
            (rest == name).then(|| n.parse::<u64>().ok()).flatten()
        })
    };
    let count_of = |name: &str| {
        counts.lines().find_map(|l| {
            let (rest, n) = l.rsplit_once(':')?;
            (rest == name)
                .then(|| n.trim().parse::<u32>().ok())
                .flatten()
        })
    };
    names
        .iter()
        .map(|name| {
            let size = size_of(name)?;
            let count = count_of(name)?;
            // grep counts no line in a binary file; an empty file has none.
            let lines = if size > MAX_COUNTED || (count == 0 && size > 0) {
                None
            } else {
                Some(count)
            };
            Some(FileFacts { size, lines })
        })
        .collect()
}

/// What went wrong in a remote call, in one line: hc's own errors come as
/// JSON (`{"error": {"message", "hint"}}`), the program's as plain text.
pub fn remote_error(stderr: &str) -> String {
    match hc_failure(stderr) {
        Some(m) => m,
        None if stderr.trim().is_empty() => "no answer".to_string(),
        None => crate::git::first_line(stderr),
    }
}

/// hc's own error (`{"error": {message, hint?}}` on stderr: an unknown or
/// unreachable machine), as `message (hint)`; `None` when `stderr` is the
/// program's.
pub fn hc_failure(stderr: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(stderr.trim()).ok()?;
    let err = parsed.get("error")?;
    let message = err.get("message")?.as_str()?;
    Some(match err.get("hint").and_then(|h| h.as_str()) {
        Some(hint) => format!("{message} ({hint})"),
        None => message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_errors_read_hcs_json_or_the_programs_text() {
        assert_eq!(
            remote_error(
                r#"{"error":{"hint":"hc machine add <ssh-target>","kind":"machine_unknown","message":"unknown machine \"x\""}}"#
            ),
            "unknown machine \"x\" (hc machine add <ssh-target>)"
        );
        assert_eq!(
            remote_error("fatal: not a git repository (or any parent)\nmore"),
            "fatal: not a git repository (or any parent)"
        );
    }

    #[test]
    fn measured_answers_line_up_by_name() {
        let names: Vec<String> = ["a b.txt", "bin", "empty", "gone", "x:y", "big"]
            .map(String::from)
            .to_vec();
        let out = "      10 a b.txt\n       3 bin\n       0 empty\n       2 x:y\n 2000000 big\n 2000015 total\n--gitview--\na b.txt:2\nbin:0\nempty:0\nx:y:1\nbig:9\n";
        let facts = parse_measure(out, &names);
        let lines: Vec<Option<Option<u32>>> = facts.iter().map(|f| f.map(|f| f.lines)).collect();
        assert_eq!(
            lines,
            [
                Some(Some(2)),
                Some(None), // binary
                Some(Some(0)),
                None, // unreadable there
                Some(Some(1)),
                Some(None), // too big to count
            ]
        );
    }
}
