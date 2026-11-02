//! `herdr-gitview json <op>`: gitview's git engine as JSON, for the Herdr
//! desktop app's native gitview mods (`herdr-app/mods/gitview`, the file
//! list, and `gitview-diff`, the diff pane). The mods draw; everything about
//! git — the grouped status, the highlighted diff with word emphasis and
//! folds, stage/unstage/discard, history, the base a branch was cut from,
//! the notes message an agent gets — is the same code the TUI panes run.
//!
//! Every op prints one JSON object on stdout; on failure `{"error": "..."}`
//! and exit 1.
//!
//! **Remote repos.** The ops marked *(on)* take `--on MACHINE` (a name from
//! `hc machine ls`; `--on ""` = here): `--repo` is then a path on that
//! machine, and every git command and file read runs there through
//! `hc exec --on MACHINE --cwd DIR [-i] -- …` (`GITVIEW_HC`, else `hc`).
//! Nothing of the repo is read on this machine, and the replies have the
//! same shape as for a local repo.
//!
//! **Paths.** `PATH` arguments are relative to the repo's top level. For
//! stage/unstage/discard a PATH may be a folder: `src`, `src/` or `.` (the
//! whole repo) means every listed file under it (per path component) in
//! the section the op acts on; end it in `/` to be sure it's taken as one.
//!
//! Ops (exact usage):
//!
//! - `status --repo DIR [--scope worktree|branch] [--on M]`
//!   → `{root, branch, scope, sections: [{id, title, cached, files: [FILE]}],
//!   editor, git_editor}`; branch scope adds `base: {label, merge_base}` and
//!   `ahead`, and has one section (`id: "branch"`). Worktree sections:
//!   `conflicts`, `staged`, `changes` (empty ones left out). FILE is
//!   `{path, orig, kind, letter, stage, add, del}`.
//! - `diff --repo DIR --path P --kind K [--orig P] [--cached]
//!   [--merge-base SHA --base LABEL] [--commit SHA] [--theme dark|light]
//!   [--unfold] [--on M]` → `{rows: [{k, o, n, m, bg, s}], truncated,
//!   first_change, empty, binary}`: rows as the TUI renders them; the
//!   palette's tints come as roles (`@ins`, `@del`, `@ins_emph`,
//!   `@del_emph`, `@gutter`, `@fold`, `@fold_fg`) so the mod paints them in
//!   the app's theme. `--cached`: HEAD vs index; `--merge-base`: that commit
//!   vs the worktree; `--commit`: that commit's own change to the file (vs
//!   its first parent).
//! - `stage --repo DIR [--on M] PATH...` / `unstage --repo DIR [--on M]
//!   PATH...` → `{ok, count}`; a folder stages what "changes" holds under
//!   it, unstages what "staged changes" holds.
//! - `discard --repo DIR [--section changes|staged] [--on M] PATH...` →
//!   `{ok, count}`: throws away each file's changes (index and worktree),
//!   stopping at the first failure; a folder covers what `--section` holds
//!   under it (default: both sections). A file without changes is an error.
//! - `commit --repo DIR [--all] [--on M] MESSAGE` → `{ok, summary}` (`--all`:
//!   `git add -A` first; `summary`: `<short sha> <subject>`).
//! - `log --repo DIR [--merge-base SHA] [--limit N] [--on M]` →
//!   `{commits: [{sha, short, subject, author, when, date, time, refs}]}`,
//!   newest first, at most N (200). `--merge-base`: only `SHA..HEAD`.
//!   `when`: relative (`3 days ago`); `date`: `YYYY-MM-DD`; `time`: Unix
//!   seconds; `refs`: decorations (`HEAD -> main`, `origin/main`, `tag: v1`).
//! - `commit-files --repo DIR --sha SHA [--on M]` → `{files: [FILE]}` (the
//!   commit vs its first parent; `stage` is `"na"`).
//! - `nvim --repo DIR --path P --line N [--tab T] [--space S] [--shown
//!   t1,t2] [--on M]` → `{terminal}`: opens the file at that line in
//!   herdr-nvim (`<app home>/mods/herdr-nvim/bin/herdr-nvim open --tab T
//!   [--space S]`, then `terminal` is `"herdr-nvim"`), else in an nvim
//!   already running in tab T or in one of the `shown` terminals (`terminal`
//!   is that terminal); `null` when there is none. With `--on`: herdr-nvim
//!   only (it needs `--tab` and `--space`), never an nvim of this machine.
//! - `notes NOTES_JSON` (`[{file, start, end, text, snippet}]`) →
//!   `{message}`: the text an agent gets.
//! - `send --terminal T [--submit] TEXT` → `{ok}`: place TEXT in the
//!   terminal's input without pressing Enter (a paste through a framed
//!   `terminal.attach` over `hc connect`, bracketed when the program asks;
//!   no clipboard) or, with `--submit`, send it (`hc send`, which presses
//!   Enter).
//! - `sidebar --tab T --mod NAME [--props JSON] [--percent N]` → `{pane}`: a
//!   mod pane spanning the tab's full height at its right edge, N percent of
//!   the tab wide (default: `view_width_percent`).
//!
//! The app install's layout and key setup is `herdr-gitview app-setup
//! install|uninstall --keys FILE [--home DIR] [--app BIN]` (see
//! [`crate::appsetup`]).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::git::{ChangeKind, FileEntry, Repo, Scope, StageState};
use crate::ipc::{BranchBase, ShowReq};
use crate::list::rows::Section;

/// Diff lines sent to the panel at most; longer diffs end in a note.
const MAX_ROWS: usize = 5000;

/// Commits `log` lists by default (as the TUI's history view).
const LOG_LIMIT: usize = 200;

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
        "log" => log(&a),
        "commit-files" => commit_files(&a),
        "notes" => {
            Ok(json!({ "message": notes(a.positional.first().context("notes: no JSON")?)? }))
        }
        "nvim" => {
            let repo = a.repo()?;
            let file = repo.root.join(a.value("path").context("--path")?);
            let line = a.value("line").and_then(|l| l.parse().ok()).unwrap_or(1);
            let (tab, space) = (a.value("tab"), a.value("space"));
            if repo.on.is_some() {
                // A remote repo's file only exists there: herdr-nvim with the
                // space (whose machine it is), never an nvim of this machine.
                let terminal = match (tab, space) {
                    (Some(tab), Some(space)) => {
                        crate::apphost::open_in_herdr_nvim(tab, Some(space), &file, line)
                            .then_some("herdr-nvim")
                    }
                    _ => None,
                };
                return Ok(json!({ "terminal": terminal }));
            }
            let shown: Vec<String> = a
                .value("shown")
                .map(|s| {
                    s.split(',')
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let terminal = crate::apphost::open_in_running_nvim(tab, space, &shown, &file, line);
            Ok(json!({ "terminal": terminal }))
        }
        "commit" => commit(&a),
        "sidebar" => {
            let cfg = crate::config::Config::load();
            let props: Value = match a.value("props") {
                Some(raw) => serde_json::from_str(raw).context("--props")?,
                None => Value::Null,
            };
            let pane = crate::apphost::open_mod_sidebar(
                a.value("tab").context("--tab")?,
                a.value("mod").context("--mod")?,
                props,
                match a.value("percent") {
                    Some(n) => n.parse::<u8>().context("--percent")?.clamp(5, 95),
                    None => cfg.view_width_percent,
                },
            )?;
            Ok(json!({ "pane": pane }))
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
        const FLAGS: [&str; 4] = ["cached", "submit", "unfold", "all"];
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

    /// `--repo DIR`, on `--on MACHINE` when given.
    fn repo(&self) -> Result<Repo> {
        Repo::discover_on(
            Path::new(self.value("repo").context("--repo")?),
            self.value("on"),
        )
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
    repo.config_get("core.editor")
        .is_none()
        .then(|| cfg.editor.join(" "))
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
        .map(file_json)
        .collect();
    json!({ "id": id, "title": title, "cached": section.cached(), "files": files })
}

/// One file of a list, as `status` and `commit-files` give it.
fn file_json(f: &FileEntry) -> Value {
    json!({
        "path": f.path,
        "orig": f.orig_path,
        "kind": f.kind,
        "letter": letter(f.kind),
        "stage": stage_name(f.stage),
        "add": f.adds,
        "del": f.dels,
    })
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
    req.commit = a.value("commit").map(str::to_string);
    if let (Some(merge_base), Some(label)) = (a.value("merge-base"), a.value("base")) {
        req.scope = Scope::Branch;
        req.base = Some(BranchBase {
            label: label.to_string(),
            merge_base: merge_base.to_string(),
        });
    }
    let doc = crate::preview::worker::load_doc(&repo, &cfg, &req).map_err(anyhow::Error::msg)?;
    let roles = crate::preview::render::tint_roles(cfg.theme);
    let hex = |c| hex(c, &roles);
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

fn hex(color: Option<ratatui::style::Color>, roles: &[((u8, u8, u8), &str)]) -> Value {
    use ratatui::style::Color;
    match color {
        Some(Color::Rgb(r, g, b)) => match roles.iter().find(|(rgb, _)| *rgb == (r, g, b)) {
            Some((_, role)) => json!(format!("@{role}")),
            None => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        },
        Some(Color::Red) => json!("red"),
        Some(Color::Green) => json!("green"),
        _ => Value::Null,
    }
}

// ---- actions ------------------------------------------------------------------

/// What one PATH argument of stage/unstage/discard names.
#[derive(Debug, PartialEq)]
enum Named<'a> {
    /// A changed file (its status entry).
    File(&'a FileEntry),
    /// A folder (`src`, `src/`, `.`): the entries under it in the section.
    Folder(Vec<&'a FileEntry>),
    /// Neither: passed to git as given.
    Other,
}

/// Resolve `raw` against the status `entries`: an exact path is that file;
/// a path ending in `/`, `.` (the whole repo), or the parent of changed files
/// is a folder, holding the entries under it that `holds` accepts. The match
/// is per path component (`src` never reaches `src-old/`).
fn name<'a>(entries: &'a [FileEntry], raw: &str, holds: impl Fn(&FileEntry) -> bool) -> Named<'a> {
    let raw = raw.strip_prefix("./").unwrap_or(raw);
    let trimmed = raw.trim_end_matches('/');
    let whole = trimmed.is_empty() || trimmed == ".";
    let folder_syntax = whole || raw.ends_with('/');
    if !folder_syntax && let Some(e) = entries.iter().find(|e| e.path == Path::new(raw)) {
        return Named::File(e);
    }
    let prefix = if whole {
        String::new()
    } else {
        format!("{trimmed}/")
    };
    let under: Vec<&FileEntry> = entries
        .iter()
        .filter(|e| e.path.to_string_lossy().starts_with(&prefix) && holds(e))
        .collect();
    if folder_syntax || !under.is_empty() {
        Named::Folder(under)
    } else {
        Named::Other
    }
}

fn stage(a: &Args, on: bool) -> Result<Value> {
    let repo = a.repo()?;
    if a.positional.is_empty() {
        bail!("no paths");
    }
    let cfg = crate::config::Config::load();
    let entries = repo.worktree_status(cfg.show_untracked)?;
    // A folder stages what "changes" holds under it, unstages what "staged
    // changes" holds: as `s`/`u` on a folder row in the TUI.
    let section = if on {
        Section::Changes
    } else {
        Section::Staged
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for raw in &a.positional {
        match name(&entries, raw, |e| section.holds(e)) {
            Named::Folder(under) => paths.extend(under.iter().map(|e| e.path.clone())),
            Named::File(e) => paths.push(e.path.clone()),
            Named::Other => paths.push(PathBuf::from(raw)),
        }
    }
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    if !paths.is_empty() {
        if on {
            repo.stage_many(&paths)?;
        } else {
            repo.unstage_many(&paths)?;
        }
    }
    Ok(json!({ "ok": true, "count": paths.len() }))
}

/// Throw away every named file's changes (index and worktree), stopping at
/// the first that fails. `--section changes|staged` limits folders to what
/// that section holds (default: both).
fn discard(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    if a.positional.is_empty() {
        bail!("discard: no path");
    }
    let section = match a.value("section") {
        None => None,
        Some("changes") => Some(Section::Changes),
        Some("staged") => Some(Section::Staged),
        Some(other) => bail!("--section {other}: changes or staged"),
    };
    let cfg = crate::config::Config::load();
    let entries = repo.worktree_status(cfg.show_untracked)?;
    let holds = |e: &FileEntry| match section {
        Some(s) => s.holds(e),
        None => Section::Changes.holds(e) || Section::Staged.holds(e),
    };
    let mut targets: Vec<&FileEntry> = Vec::new();
    for raw in &a.positional {
        match name(&entries, raw, holds) {
            Named::Folder(under) => targets.extend(under),
            Named::File(e) => targets.push(e),
            Named::Other => bail!("{raw} has no changes"),
        }
    }
    let mut seen = std::collections::HashSet::new();
    targets.retain(|e| seen.insert(e.path.clone()));
    for entry in &targets {
        repo.discard(entry)
            .with_context(|| format!("discarding {}", entry.path.display()))?;
    }
    Ok(json!({ "ok": true, "count": targets.len() }))
}

// ---- history ------------------------------------------------------------------

/// Newest first; with `--merge-base`, only the commits on top of it.
fn log(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let limit = match a.value("limit") {
        Some(n) => n.parse().context("--limit")?,
        None => LOG_LIMIT,
    };
    let commits = match a.value("merge-base") {
        Some(mb) => repo.log_branch_commits(mb, limit)?,
        None => repo.log_commits(limit)?,
    };
    let commits: Vec<Value> = commits
        .iter()
        .map(|c| {
            json!({
                "sha": c.sha,
                "short": c.short,
                "subject": c.subject,
                "author": c.author,
                "when": c.relative,
                "date": c.date,
                "time": c.time,
                "refs": c.refs,
            })
        })
        .collect();
    Ok(json!({ "commits": commits }))
}

fn commit_files(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let sha = a.value("sha").context("commit-files: --sha")?;
    let files: Vec<Value> = repo.commit_files(sha)?.iter().map(file_json).collect();
    Ok(json!({ "files": files }))
}

fn commit(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let message = a.positional.first().context("commit: no message")?;
    repo.commit(message, a.flag("all"))?;
    Ok(json!({ "ok": true, "summary": repo.last_commit_summary() }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, stage: StageState) -> FileEntry {
        FileEntry {
            path: path.into(),
            orig_path: None,
            kind: ChangeKind::Modified,
            stage,
            adds: None,
            dels: None,
        }
    }

    fn paths(named: Named) -> Vec<String> {
        match named {
            Named::Folder(under) => under.iter().map(|e| e.path.display().to_string()).collect(),
            other => panic!("not a folder: {other:?}"),
        }
    }

    #[test]
    fn a_path_names_a_file_a_folder_or_neither() {
        let entries = [
            entry("src/a.rs", StageState::Unstaged),
            entry("src/deep/b.rs", StageState::Staged),
            entry("src-old/c.rs", StageState::Unstaged),
            entry("top.rs", StageState::Partial),
        ];
        let any = |_: &FileEntry| true;
        assert_eq!(name(&entries, "top.rs", any), Named::File(&entries[3]));
        assert_eq!(name(&entries, "./top.rs", any), Named::File(&entries[3]));
        // Per component: `src` never reaches `src-old/`.
        assert_eq!(
            paths(name(&entries, "src", any)),
            ["src/a.rs", "src/deep/b.rs"]
        );
        assert_eq!(
            paths(name(&entries, "src/", any)),
            ["src/a.rs", "src/deep/b.rs"]
        );
        assert_eq!(paths(name(&entries, ".", any)).len(), 4);
        // Only what the section holds.
        let staged = |e: &FileEntry| Section::Staged.holds(e);
        assert_eq!(paths(name(&entries, "src", staged)), ["src/deep/b.rs"]);
        assert_eq!(
            paths(name(&entries, ".", staged)),
            ["src/deep/b.rs", "top.rs"]
        );
        // A folder with nothing in the section is still a folder, so git
        // never gets it (it would stage hidden untracked files).
        assert_eq!(
            paths(name(&entries, "src-old/", staged)),
            Vec::<String>::new()
        );
        assert_eq!(name(&entries, "clean.rs", any), Named::Other);
    }
}
