//! Where a repo's commands run: this machine, or another one reached through
//! herdr-core (`hc exec --on <machine> --cwd <dir> [-i] -- …`). [`Machine`]
//! is the one place that tells the two apart: git.rs asks it for a git
//! command, a file's bytes, or the facts about untracked files, and never
//! checks which machine it is on.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

/// The `hc` binary: `GITVIEW_HC`, else `hc` on `PATH`.
pub fn hc_bin() -> OsString {
    std::env::var_os("GITVIEW_HC").unwrap_or_else(|| "hc".into())
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
    /// Seconds before hc ends it (hc's default: 60). Here: no limit.
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

/// Untracked files are counted at most this big (bigger: no counts).
pub const MAX_COUNTED: u64 = 1024 * 1024;

/// Files per remote measuring call (keeps the argv well under limits).
const MEASURE_BATCH: usize = 200;

/// Exit code of [`READ`] for a file that isn't there.
const MISSING: i32 = 44;
const READ: &str = r#"[ -e "$1" ] || exit 44; exec cat -- "$1""#;

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
        let mut cmd = Command::new(hc_bin());
        cmd.args(["exec", "--on", machine]).arg("--cwd").arg(cwd);
        if let Some(secs) = run.timeout {
            cmd.args(["--timeout", &secs.to_string()]);
        }
        if run.stdin {
            cmd.arg("-i");
        }
        cmd.arg("--").args(argv);
        cmd
    }

    /// `git <args>` in the repo (or folder) `dir`. Reads never take optional
    /// locks: a poll holding the index lock made the person's own
    /// merge/commit fail ("Unable to write index").
    pub fn git(&self, dir: &Path, args: &[&str], run: Run) -> Command {
        match self {
            Machine::Local => {
                let mut cmd = Command::new("git");
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

    /// Size and line count of each of `paths` (relative to `dir`), in order;
    /// `None` for a file that can't be read. Here each file is read in
    /// process; there, every batch is one `hc exec` running two programs.
    pub fn measure(&self, dir: &Path, paths: &[&Path]) -> Vec<Option<FileFacts>> {
        match self {
            Machine::Local => paths.iter().map(|p| measure_local(&dir.join(p))).collect(),
            Machine::On(m) => {
                let mut facts = Vec::with_capacity(paths.len());
                for batch in paths.chunks(MEASURE_BATCH) {
                    let names: Vec<String> = batch
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    // A newline in a name would split the answers' lines.
                    let asked: Vec<&str> = names
                        .iter()
                        .map(String::as_str)
                        .filter(|n| !n.contains('\n'))
                        .collect();
                    let mut argv = vec!["sh", "-c", MEASURE, "sh"];
                    argv.extend(&asked);
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
    let parsed: Option<serde_json::Value> = serde_json::from_str(stderr.trim()).ok();
    let message = parsed.as_ref().and_then(|v| {
        let err = v.get("error")?;
        let message = err.get("message")?.as_str()?;
        Some(match err.get("hint").and_then(|h| h.as_str()) {
            Some(hint) => format!("{message} ({hint})"),
            None => message.to_string(),
        })
    });
    match message {
        Some(m) => m,
        None if stderr.trim().is_empty() => "no answer".to_string(),
        None => crate::git::first_line(stderr),
    }
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
