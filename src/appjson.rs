//! `herdr-gitview json <op>`: gitview's git engine as JSON, for the Herdr
//! desktop app's native gitview panel (`herdr-app/mods/gitview`). The panel
//! draws; everything about git — the grouped status, the highlighted diff
//! with word emphasis and folds, stage/unstage/discard, the base a branch
//! was cut from, the notes message an agent gets — is the same code the
//! TUI panes run.
//!
//! Every op prints one JSON object on stdout (`{"error": "..."}` and exit 1
//! on failure). Ops:
//!
//! - `status --repo DIR [--scope worktree|branch]`
//! - `diff --repo DIR --path P --kind K [--orig P] [--cached] [--theme dark|light]
//!   [--merge-base SHA --base LABEL] [--unfold]`
//! - `stage|unstage --repo DIR PATH...`
//! - `discard --repo DIR PATH` (as the file list shows it now)
//! - `notes NOTES_JSON` (`[{file, start, end, text, snippet}]` → the message)
//! - `nvim --repo DIR --path P --line N [--tab T] [--shown t1,t2]` (open it
//!   in an nvim already running there; `{"terminal": null}` when none)
//! - `send --terminal T [--submit] TEXT` (place in / submit to an agent)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::git::{ChangeKind, FileEntry, Repo, Scope, StageState};
use crate::ipc::{BranchBase, ShowReq};
use crate::list::rows::Section;

/// Diff lines sent to the panel at most; longer diffs end in a note.
const MAX_ROWS: usize = 5000;

pub fn run(args: &[String]) -> Result<()> {
    let result = dispatch(args);
    match result {
        Ok(value) => {
            println!("{value}");
            Ok(())
        }
        Err(err) => {
            println!("{}", json!({ "error": format!("{err:#}") }));
            std::process::exit(1);
        }
    }
}

fn dispatch(args: &[String]) -> Result<Value> {
    let (op, rest) = args.split_first().context("json: which op?")?;
    let a = Args::parse(rest);
    match op.as_str() {
        "status" => status(&a),
        "diff" => diff(&a),
        "stage" | "unstage" => stage(&a, op == "stage"),
        "discard" => discard(&a),
        "notes" => {
            Ok(json!({ "message": notes(a.positional.first().context("notes: no JSON")?)? }))
        }
        "nvim" => {
            let repo = a.repo()?;
            let file = repo.root.join(a.value("path").context("--path")?);
            let line = a.value("line").and_then(|l| l.parse().ok()).unwrap_or(1);
            let shown: Vec<String> = a
                .value("shown")
                .map(|s| {
                    s.split(',')
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let terminal =
                crate::apphost::open_in_running_nvim(a.value("tab"), &shown, &file, line);
            Ok(json!({ "terminal": terminal }))
        }
        "send" => {
            let terminal = a.value("terminal").context("send: --terminal")?;
            let text = a.positional.first().context("send: no text")?;
            crate::apphost::send_text(terminal, text, a.flag("submit"))?;
            Ok(json!({ "ok": true }))
        }
        other => bail!("json: unknown op {other}"),
    }
}

/// `--key value`, `--flag` and positional arguments.
struct Args {
    pairs: Vec<(String, Option<String>)>,
    positional: Vec<String>,
}

impl Args {
    fn parse(args: &[String]) -> Args {
        const FLAGS: [&str; 3] = ["cached", "submit", "unfold"];
        let mut pairs = Vec::new();
        let mut positional = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.strip_prefix("--") {
                Some(key) if FLAGS.contains(&key) => pairs.push((key.to_string(), None)),
                Some(key) => pairs.push((key.to_string(), it.next().cloned())),
                None => positional.push(arg.clone()),
            }
        }
        Args { pairs, positional }
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_deref())
    }

    fn flag(&self, key: &str) -> bool {
        self.pairs.iter().any(|(k, _)| k == key)
    }

    fn repo(&self) -> Result<Repo> {
        Repo::discover(Path::new(self.value("repo").context("--repo")?))
    }
}

// ---- status -------------------------------------------------------------------

fn status(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let cfg = crate::config::Config::load();
    let branch = repo.head_branch();
    if a.value("scope") == Some("branch") {
        let (label, merge_base) = repo.resolve_base(&cfg.base)?;
        let files = repo.branch_changes(&merge_base, cfg.show_untracked)?;
        return Ok(json!({
            "root": repo.root,
            "branch": branch,
            "scope": "branch",
            "base": { "label": label, "merge_base": merge_base },
            "ahead": repo.commits_ahead(&merge_base),
            "sections": [section_json(Section::Flat, "", &files)],
            "editor": cfg.editor,
            "git_editor": git_editor(&repo, &cfg),
        }));
    }
    let files = repo.worktree_status(cfg.show_untracked)?;
    let sections: Vec<Value> = [
        (Section::Conflicts, "merge conflicts"),
        (Section::Staged, "staged changes"),
        (Section::Changes, "changes"),
    ]
    .into_iter()
    .map(|(s, title)| section_json(s, title, &files))
    .filter(|s| s["files"].as_array().is_some_and(|f| !f.is_empty()))
    .collect();
    Ok(json!({
        "root": repo.root,
        "branch": branch,
        "scope": "worktree",
        "sections": sections,
        "editor": cfg.editor,
        "git_editor": git_editor(&repo, &cfg),
    }))
}

/// `GIT_EDITOR` for `git commit -e`: the configured editor, unless the
/// person set one for git itself (as the TUI's commit does).
fn git_editor(repo: &Repo, cfg: &crate::config::Config) -> Option<String> {
    if std::env::var_os("GIT_EDITOR").is_some() {
        return None;
    }
    let set = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(["config", "--get", "core.editor"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    (!set).then(|| cfg.editor.join(" "))
}

fn section_json(section: Section, title: &str, files: &[FileEntry]) -> Value {
    let id = match section {
        Section::Conflicts => "conflicts",
        Section::Staged => "staged",
        Section::Changes => "changes",
        Section::Flat => "branch",
    };
    let files: Vec<Value> = files
        .iter()
        .filter(|f| section.holds(f))
        .map(|f| {
            json!({
                "path": f.path,
                "orig": f.orig_path,
                "kind": f.kind,
                "letter": letter(f.kind),
                "stage": stage_name(f.stage),
                "add": f.adds,
                "del": f.dels,
            })
        })
        .collect();
    json!({ "id": id, "title": title, "cached": section.cached(), "files": files })
}

fn letter(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Modified => "M",
        ChangeKind::Added => "A",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Untracked => "U",
        ChangeKind::Conflicted => "!",
    }
}

fn stage_name(s: StageState) -> &'static str {
    match s {
        StageState::Unstaged => "unstaged",
        StageState::Staged => "staged",
        StageState::Partial => "partial",
        StageState::Untracked => "untracked",
        StageState::NA => "na",
    }
}

// ---- diff ---------------------------------------------------------------------

fn diff(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let mut cfg = crate::config::Config::load();
    match a.value("theme") {
        Some("light") => cfg.theme = crate::config::Theme::Light,
        Some("dark") => cfg.theme = crate::config::Theme::Dark,
        _ => {}
    }
    if a.flag("unfold") {
        cfg.context_lines = usize::MAX / 4; // nothing folds
    }
    let kind: ChangeKind =
        serde_json::from_value(json!(a.value("kind").unwrap_or("modified"))).context("--kind")?;
    let mut req = ShowReq::worktree(
        PathBuf::from(a.value("path").context("--path")?),
        a.value("orig").map(PathBuf::from),
        a.flag("cached"),
        kind,
    );
    if let (Some(merge_base), Some(label)) = (a.value("merge-base"), a.value("base")) {
        req.scope = Scope::Branch;
        req.base = Some(BranchBase {
            label: label.to_string(),
            merge_base: merge_base.to_string(),
        });
    }
    let doc = crate::preview::worker::load_doc(&repo, &cfg, &req).map_err(anyhow::Error::msg)?;
    let mut rows = Vec::new();
    for (i, line) in doc.text.lines.iter().enumerate().take(MAX_ROWS) {
        let marker = doc.marker_text_of_line(i).unwrap_or_default();
        let (old, new) = doc.numbers_of_line(i).unwrap_or((None, None));
        let kind = match marker.chars().next() {
            Some('-') => "del",
            Some('+') => "add",
            Some(_) => "ctx",
            None => "fold",
        };
        let spans: Vec<Value> = line
            .spans
            .iter()
            .map(|s| json!([s.content, hex(s.style.fg), hex(s.style.bg)]))
            .collect();
        rows.push(json!({
            "k": kind, "o": old, "n": new, "m": marker,
            "bg": hex(line.style.bg), "s": spans,
        }));
    }
    Ok(json!({
        "rows": rows,
        "truncated": doc.text.lines.len() > MAX_ROWS,
        "first_change": doc.first_change,
        "empty": doc.is_empty,
        "binary": doc.binary,
    }))
}

fn hex(color: Option<ratatui::style::Color>) -> Value {
    use ratatui::style::Color;
    match color {
        Some(Color::Rgb(r, g, b)) => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        Some(Color::Red) => json!("red"),
        Some(Color::Green) => json!("green"),
        _ => Value::Null,
    }
}

// ---- actions ------------------------------------------------------------------

fn stage(a: &Args, on: bool) -> Result<Value> {
    let repo = a.repo()?;
    let paths: Vec<PathBuf> = a.positional.iter().map(PathBuf::from).collect();
    if paths.is_empty() {
        bail!("no paths");
    }
    if on {
        repo.stage_many(&paths)?;
    } else {
        repo.unstage_many(&paths)?;
    }
    Ok(json!({ "ok": true }))
}

fn discard(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let path = PathBuf::from(a.positional.first().context("discard: no path")?);
    let cfg = crate::config::Config::load();
    let entry = repo
        .worktree_status(cfg.show_untracked)?
        .into_iter()
        .find(|e| e.path == path)
        .with_context(|| format!("{} has no changes", path.display()))?;
    repo.discard(&entry)?;
    Ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct NoteIn {
    file: PathBuf,
    #[serde(default)]
    start: u32,
    #[serde(default)]
    end: u32,
    text: String,
    #[serde(default)]
    snippet: String,
}

fn notes(raw: &str) -> Result<String> {
    let notes: Vec<NoteIn> = serde_json::from_str(raw).context("notes JSON")?;
    let notes: Vec<crate::preview::app::Note> = notes
        .into_iter()
        .enumerate()
        .map(|(i, n)| crate::preview::app::Note {
            id: i as u64,
            file: n.file,
            start: n.start,
            end: n.end,
            text: n.text,
            snippet: n.snippet,
            cached: false,
        })
        .collect();
    Ok(crate::preview::session::notes_message(&notes))
}
