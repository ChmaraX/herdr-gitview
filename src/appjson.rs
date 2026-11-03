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
//! **Remote repos.** Every op with `--repo` takes `--on MACHINE` (a name from
//! `hc machine ls`; `--on ""` = here): `--repo` is then a path on that
//! machine, and every git command and file read runs there through
//! `hc exec --on MACHINE --cwd DIR [-i] -- …` (`GITVIEW_HC`, else `hc`).
//! Nothing of the repo is read on this machine, and the replies have the
//! same shape as for a local repo.
//!
//! **Environment.** `GITVIEW_HC` (the `hc` binary), `GITVIEW_HERDR_APP`
//! (the `herdr-app` binary), `HERDR_APP_HOME` (the app home, else
//! `$HOME/.config/herdr-app`), `GITVIEW_CONFIG_DIR` (the folder of
//! `config.toml`, else the TUI plugin's own,
//! `~/.config/herdr/plugins/config/chmarax.gitview`).
//!
//! **Arguments.** `--key value` and `--flag`; after `--` everything is
//! positional, so callers put `--` before paths, messages and text (one may
//! start with `--`).
//!
//! **Paths.** `PATH` arguments are relative to the repo's top level. For
//! stage/unstage/discard a PATH may be a folder: `src`, `src/` or `.` (the
//! whole repo) means every listed file under it (per path component) in
//! the section the op acts on; end it in `/` to be sure it's taken as one.
//!
//! Ops (exact usage):
//!
//! - `status --repo DIR [--scope worktree|branch] [--base LABEL --merge-base
//!   SHA --head SHA] [--on M]` → `{root, branch, head, scope, sections: [{id,
//!   title, cached, files: [FILE]}], editor}`; `head` is HEAD's sha (`null`
//!   before the first commit); branch scope adds `base: {label, merge_base}`
//!   and has one section (`id: "branch"`). Worktree sections: `conflicts`,
//!   `staged`, `changes` (empty ones left out). FILE is `{path, orig, kind,
//!   letter, add, del}`. In branch scope, `--base`/`--merge-base`/`--head`
//!   (what the last reply said) skip the base detection, a dozen git calls,
//!   while HEAD is still at `--head`; when it moved (or `--head` is
//!   missing), the base is resolved again.
//! - `diff --repo DIR --path P --kind K [--orig P] [--cached]
//!   [--merge-base SHA --base LABEL] [--commit SHA] [--theme dark|light]
//!   [--unfold] [--on M]` → `{rows: [{k, o, n, m, bg, s}], truncated,
//!   first_change, empty, binary}`: rows as the TUI renders them; the
//!   palette's tints come as roles (`@ins`, `@del`, `@ins_emph`,
//!   `@del_emph`, `@gutter`, `@fold`, `@fold_fg`) so the mod paints them in
//!   the app's theme. `--cached`: HEAD vs index; `--merge-base`: that commit
//!   vs the worktree; `--commit`: that commit's own change to the file (vs
//!   its first parent).
//! - `stage|unstage --repo DIR [--section changes|staged|conflicts] [--on M]
//!   [--] PATH...` → `{ok, count}`: a folder covers what that section holds
//!   under it (default: `changes` for stage, `staged` for unstage); a path
//!   naming no changed file goes to git as given.
//! - `discard --repo DIR [--section changes|staged|conflicts] [--on M] [--]
//!   PATH...` → `{ok, count}`, stopping at the first failure; a file without
//!   changes is an error. Without `--section`: each file's changes go
//!   entirely (index and worktree), and a folder covers both sections. With
//!   `--section changes`: a partly staged file only loses its worktree
//!   changes (`git restore -- P`), keeping what's staged.
//! - `commit --repo DIR [--on M] [--] MESSAGE` → `{ok, all, summary}`: the
//!   engine decides what goes in, at commit time. Unmerged paths: an error
//!   (`resolve the merge conflicts first`), nothing changed. Else, anything
//!   staged: just that. Else everything, staged first: `git add -A`, or
//!   `git add -u` when `show_untracked` is off (`all: true`). `summary`:
//!   `<short sha> <subject>`.
//! - `log --repo DIR [--merge-base SHA] [--limit N] [--on M]` →
//!   `{commits: [{sha, short, subject, author, when, refs}]}`, newest first,
//!   at most N (200). `--merge-base`: only `SHA..HEAD`. `when`: relative
//!   (`3 days ago`); `refs`: decorations (`HEAD -> main`, `tag: v1`).
//! - `commit-files --repo DIR --sha SHA [--on M]` → `{files: [FILE]}` (the
//!   commit vs its first parent).
//! - `nvim --repo DIR --path P --line N [--tab T] [--space S] [--shown
//!   t1,t2] [--on M]` → `{terminal}`: opens the file at that line in
//!   herdr-nvim (`<app home>/mods/herdr-nvim/bin/herdr-nvim open --tab T
//!   [--space S]`, then `terminal` is `"herdr-nvim"`), else in an nvim
//!   already running in tab T or in one of the `shown` terminals (`terminal`
//!   is that terminal); `null` when there is none. With `--on`: herdr-nvim
//!   only (it needs `--tab` and `--space`), never an nvim of this machine.
//! - `notes [--] NOTES_JSON` (`[{file, start, end, text, snippet,
//!   cached}]`; `cached`: written on the staged diff, default false) →
//!   `{message}`: the text an agent gets, as the TUI builds it (which
//!   doesn't mark a note's side in the text).
//! - `send --terminal T [--submit] [--] TEXT` → `{ok}`: without `--submit`,
//!   place TEXT in the terminal's input without pressing Enter (a paste
//!   through a framed `terminal.attach` over `hc connect`; no clipboard;
//!   trailing newlines dropped; control characters but `\n` and `\t`
//!   stripped, so a diff line can't end the paste or press keys). The
//!   program's paste mode is read from the screen replay, waited for up to
//!   800 ms. Multi-line TEXT needs a program in bracketed-paste mode;
//!   otherwise it's an error ("this program can't take multi-line text…",
//!   or "couldn't read the terminal's paste mode…" when no replay came; both
//!   end "use shift+enter to send") and nothing is typed. `--submit`: send
//!   it with `hc send`, which presses Enter.
//! - `sidebar --tab T --mod NAME [--props JSON] [--percent N]` → `{pane}`: a
//!   mod pane at the tab's right edge, N percent of the tab wide (5–95;
//!   default: `view_width_percent`). It's split off the tab's root, spanning the
//!   tab's full height, when the layout document has the tab: this
//!   machine's, or for a remote tab `MACHINE/…` that machine's (`doc.get
//!   {on}`); otherwise off the tab's focused pane (`herdr-app status`).
//!   Neither: an error, nothing split.
//!
//! The app install's layout and key setup is `herdr-gitview app-setup
//! install|uninstall --keys FILE [--home DIR] [--app BIN]` (see
//! [`crate::appsetup`]): app home `$HERDR_APP_HOME`, else
//! `$HOME/.config/herdr-app`; stops before any change when `installed.json`
//! can't be read; both directions also drop the retired TUI-in-the-app keys
//! (`cmd-alt-g`, `cmd-alt-shift-g`, only with exactly their old actions) and
//! mod folders (`gitview-popup`, `gitview-tui-sidebar`, `gitview-tui-tab`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::git::{ChangeKind, CommitInfo, FileEntry, Repo, Scope, StageState};
use crate::ipc::{BranchBase, ShowReq};
use crate::list::rows::Section;

/// Diff lines sent to the panel at most; longer diffs end in a note.
const MAX_ROWS: usize = 5000;

/// Commits `log` lists by default (as the TUI's history view).
const LOG_LIMIT: usize = 200;

pub fn run(args: &[String]) -> Result<()> {
    match dispatch(args) {
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

/// The person's gitview config: the TUI plugin's own `config.toml`, which
/// herdr keeps in `<config>/herdr/plugins/config/chmarax.gitview`
/// (`GITVIEW_CONFIG_DIR` overrides the folder), so both share one.
fn config() -> crate::config::Config {
    let dir = std::env::var_os("GITVIEW_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                .unwrap_or_default()
                .join("herdr/plugins/config/chmarax.gitview")
        });
    crate::config::Config::load_from(Some(&dir.join("config.toml")))
}

fn reply(value: impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

fn dispatch(args: &[String]) -> Result<Value> {
    let (op, rest) = args.split_first().context("json: which op?")?;
    let a = Args::parse(rest);
    match op.as_str() {
        "status" => status(&a),
        "diff" => diff(&a),
        "stage" => change_index(&a, IndexOp::Stage),
        "unstage" => change_index(&a, IndexOp::Unstage),
        "discard" => discard(&a),
        "log" => log(&a),
        "commit-files" => commit_files(&a),
        "commit" => commit(&a),
        "notes" => {
            let message = notes(a.positional.first().context("notes: no JSON")?)?;
            Ok(json!({ "message": message }))
        }
        "nvim" => nvim(&a),
        "sidebar" => sidebar(&a),
        "send" => {
            let terminal = a.value("terminal").context("send: --terminal")?;
            let text = a.positional.first().context("send: no text")?;
            crate::apphost::send_text(terminal, text, a.flag("submit"))?;
            Ok(json!({ "ok": true }))
        }
        other => bail!("json: unknown op {other}"),
    }
}

/// `--key value`, `--flag` and positional arguments; after `--`, everything
/// is positional (a message or path may start with `--`).
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
                Some("") => {
                    positional.extend(it.by_ref().cloned());
                    break;
                }
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
            crate::remote::Machine::parse(self.value("on")),
        )
    }

    /// `--section changes|staged|conflicts`.
    fn section(&self) -> Result<Option<Section>> {
        Ok(match self.value("section") {
            None => None,
            Some("changes") => Some(Section::Changes),
            Some("staged") => Some(Section::Staged),
            Some("conflicts") => Some(Section::Conflicts),
            Some(other) => bail!("--section {other}: changes, staged or conflicts"),
        })
    }

    fn paths(&self) -> Result<&[String]> {
        if self.positional.is_empty() {
            bail!("no paths");
        }
        Ok(&self.positional)
    }
}

// ---- replies ------------------------------------------------------------------

/// One file of a list, as `status` and `commit-files` give it.
#[derive(Serialize)]
struct FileOut<'a> {
    path: &'a Path,
    orig: Option<&'a Path>,
    kind: ChangeKind,
    letter: &'static str,
    add: Option<u32>,
    del: Option<u32>,
}

impl<'a> From<&'a FileEntry> for FileOut<'a> {
    fn from(f: &'a FileEntry) -> Self {
        FileOut {
            path: &f.path,
            orig: f.orig_path.as_deref(),
            kind: f.kind,
            letter: letter(f.kind),
            add: f.adds,
            del: f.dels,
        }
    }
}

#[derive(Serialize)]
struct SectionOut<'a> {
    id: &'static str,
    title: &'static str,
    cached: bool,
    files: Vec<FileOut<'a>>,
}

impl<'a> SectionOut<'a> {
    fn of(section: Section, title: &'static str, files: &'a [FileEntry]) -> Self {
        let id = match section {
            Section::Conflicts => "conflicts",
            Section::Staged => "staged",
            Section::Changes => "changes",
            Section::Flat => "branch",
        };
        SectionOut {
            id,
            title,
            cached: section.cached(),
            files: files
                .iter()
                .filter(|f| section.holds(f))
                .map(FileOut::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct StatusOut<'a> {
    root: &'a Path,
    branch: Option<String>,
    /// The commit HEAD is at (`None` before the first commit), for the
    /// caller to pass back as `--head` with a branch-scope base.
    head: Option<String>,
    scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<BranchBase>,
    sections: Vec<SectionOut<'a>>,
    editor: &'a [String],
}

#[derive(Serialize)]
struct Done {
    ok: bool,
    count: usize,
}

#[derive(Serialize)]
struct CommitOut<'a> {
    sha: &'a str,
    short: &'a str,
    subject: &'a str,
    author: &'a str,
    when: &'a str,
    refs: &'a [String],
}

impl<'a> From<&'a CommitInfo> for CommitOut<'a> {
    fn from(c: &'a CommitInfo) -> Self {
        CommitOut {
            sha: &c.sha,
            short: &c.short,
            subject: &c.subject,
            author: &c.author,
            when: &c.relative,
            refs: &c.refs,
        }
    }
}

#[derive(Serialize)]
struct RowOut {
    k: &'static str,
    o: Option<u32>,
    n: Option<u32>,
    m: String,
    bg: Option<String>,
    /// `[text, fg, bg]` per span.
    s: Vec<(String, Option<String>, Option<String>)>,
}

#[derive(Serialize)]
struct DiffOut {
    rows: Vec<RowOut>,
    truncated: bool,
    first_change: Option<u32>,
    empty: bool,
    binary: bool,
}

// ---- status -------------------------------------------------------------------

fn status(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let cfg = config();
    let branch = repo.head_branch();
    let head = repo.head_sha();
    if a.value("scope") == Some("branch") {
        // The caller may pass back the base it got last time: base detection
        // costs a dozen git calls, too many for every poll. It holds only
        // while HEAD is where it was resolved for: after a rebase or a merge
        // of the base, it's resolved again (the TUI's HeadMoved).
        let given = (a.value("base"), a.value("merge-base"), a.value("head"));
        let base = match given {
            (Some(label), Some(merge_base), Some(was)) if head.as_deref() == Some(was) => {
                BranchBase {
                    label: label.to_string(),
                    merge_base: merge_base.to_string(),
                }
            }
            _ => {
                let (label, merge_base) = repo.resolve_base(&cfg.base)?;
                BranchBase { label, merge_base }
            }
        };
        let files = repo.branch_changes(&base.merge_base, cfg.show_untracked)?;
        return reply(StatusOut {
            root: &repo.root,
            branch,
            head,
            scope: Scope::Branch,
            base: Some(base.clone()),
            sections: vec![SectionOut::of(Section::Flat, "", &files)],
            editor: &cfg.editor,
        });
    }
    let files = repo.worktree_status(cfg.show_untracked)?;
    let sections = [
        (Section::Conflicts, "merge conflicts"),
        (Section::Staged, "staged changes"),
        (Section::Changes, "changes"),
    ]
    .into_iter()
    .map(|(s, title)| SectionOut::of(s, title, &files))
    .filter(|s| !s.files.is_empty())
    .collect();
    reply(StatusOut {
        root: &repo.root,
        branch,
        head,
        scope: Scope::Worktree,
        base: None,
        sections,
        editor: &cfg.editor,
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

// ---- diff ---------------------------------------------------------------------

fn diff(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let mut cfg = config();
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
    let text = doc.tagged_text();
    let rows = text
        .lines
        .iter()
        .enumerate()
        .take(MAX_ROWS)
        .map(|(i, line)| {
            let marker = doc.marker_text_of_line(i).unwrap_or_default();
            let (o, n) = doc.numbers_of_line(i).unwrap_or((None, None));
            let k = match marker.chars().next() {
                Some('-') => "del",
                Some('+') => "add",
                Some(_) => "ctx",
                None => "fold",
            };
            RowOut {
                k,
                o,
                n,
                bg: hex(line.style.bg),
                s: line
                    .spans
                    .iter()
                    .map(|s| (s.content.to_string(), hex(s.style.fg), hex(s.style.bg)))
                    .collect(),
                m: marker.to_string(),
            }
        })
        .collect();
    reply(DiffOut {
        rows,
        truncated: text.lines.len() > MAX_ROWS,
        first_change: doc.first_change,
        empty: doc.is_empty,
        binary: doc.binary,
    })
}

/// A tagged text's color as the mod takes it: a tint's role (`@ins`, …),
/// `#rrggbb`, or the change bars' `red`/`green`.
fn hex(color: Option<ratatui::style::Color>) -> Option<String> {
    use ratatui::style::Color;
    let color = color?;
    if let Some(tint) = crate::preview::render::Tint::of(color) {
        return Some(format!("@{}", tint.name()));
    }
    match color {
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Red => Some("red".to_string()),
        Color::Green => Some("green".to_string()),
        _ => None,
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

/// The entries `raws` name (folders resolved against `holds`), each once;
/// `other` decides what a path naming no changed file becomes.
fn resolve<'a>(
    entries: &'a [FileEntry],
    raws: &[String],
    holds: impl Fn(&FileEntry) -> bool,
    mut other: impl FnMut(&str) -> Result<()>,
) -> Result<Vec<&'a FileEntry>> {
    let mut found: Vec<&FileEntry> = Vec::new();
    for raw in raws {
        match name(entries, raw, &holds) {
            Named::Folder(under) => found.extend(under),
            Named::File(e) => found.push(e),
            Named::Other => other(raw)?,
        }
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|e| seen.insert(e.path.clone()));
    Ok(found)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexOp {
    Stage,
    Unstage,
}

/// `stage` / `unstage`: a folder covers what `--section` holds under it
/// (default: "changes" for stage, "staged changes" for unstage), as `s`/`u`
/// on a folder row of that section in the TUI.
fn change_index(a: &Args, op: IndexOp) -> Result<Value> {
    let repo = a.repo()?;
    let raws = a.paths()?;
    let section = a.section()?.unwrap_or(match op {
        IndexOp::Stage => Section::Changes,
        IndexOp::Unstage => Section::Staged,
    });
    let cfg = config();
    let entries = repo.worktree_status(cfg.show_untracked)?;
    let mut others: Vec<PathBuf> = Vec::new();
    let found = resolve(
        &entries,
        raws,
        |e| section.holds(e),
        |raw| {
            others.push(PathBuf::from(raw));
            Ok(())
        },
    )?;
    let mut paths: Vec<PathBuf> = found.iter().map(|e| e.path.clone()).collect();
    for other in others {
        if !paths.contains(&other) {
            paths.push(other);
        }
    }
    if !paths.is_empty() {
        match op {
            IndexOp::Stage => repo.stage_many(&paths)?,
            IndexOp::Unstage => repo.unstage_many(&paths)?,
        }
    }
    reply(Done {
        ok: true,
        count: paths.len(),
    })
}

/// Throw away the named files' changes, stopping at the first that fails.
/// Without `--section`: all of each file's changes (index and worktree), and
/// a folder covers both sections. `--section changes`: only what that row
/// shows, so a partly staged file keeps its staged part.
fn discard(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let raws = a.paths()?;
    let section = a.section()?;
    let cfg = config();
    let entries = repo.worktree_status(cfg.show_untracked)?;
    let holds = |e: &FileEntry| match section {
        Some(s) => s.holds(e),
        None => Section::Changes.holds(e) || Section::Staged.holds(e),
    };
    let targets = resolve(&entries, raws, holds, |raw| bail!("{raw} has no changes"))?;
    for entry in &targets {
        let done = if section == Some(Section::Changes) && entry.stage == StageState::Partial {
            repo.restore_worktree(&entry.path)
        } else {
            repo.discard(entry)
        };
        done.with_context(|| format!("discarding {}", entry.path.display()))?;
    }
    reply(Done {
        ok: true,
        count: targets.len(),
    })
}

fn commit(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let message = a.positional.first().context("commit: no message")?;
    let all = repo.commit(message, config().show_untracked)?;
    Ok(json!({ "ok": true, "all": all, "summary": repo.last_commit_summary() }))
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
    let commits: Vec<CommitOut> = commits.iter().map(CommitOut::from).collect();
    Ok(json!({ "commits": commits }))
}

fn commit_files(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let sha = a.value("sha").context("commit-files: --sha")?;
    let files = repo.commit_files(sha)?;
    let files: Vec<FileOut> = files.iter().map(FileOut::from).collect();
    Ok(json!({ "files": files }))
}

// ---- the app ------------------------------------------------------------------

fn nvim(a: &Args) -> Result<Value> {
    let repo = a.repo()?;
    let file = repo.root.join(a.value("path").context("--path")?);
    let line = a.value("line").and_then(|l| l.parse().ok()).unwrap_or(1);
    let (tab, space) = (a.value("tab"), a.value("space"));
    let app = crate::apphost::App::from_env();
    if repo.machine != crate::remote::Machine::Local {
        // A remote repo's file only exists there: herdr-nvim with the space
        // (whose machine it is), never an nvim of this machine.
        let terminal = match (tab, space) {
            (Some(tab), Some(space)) => {
                crate::apphost::open_in_herdr_nvim(&app, tab, Some(space), &file, line)
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
    let terminal = crate::apphost::open_in_running_nvim(&app, tab, space, &shown, &file, line);
    Ok(json!({ "terminal": terminal }))
}

fn sidebar(a: &Args) -> Result<Value> {
    let cfg = config();
    let props: Value = match a.value("props") {
        Some(raw) => serde_json::from_str(raw).context("--props")?,
        None => Value::Null,
    };
    let percent = match a.value("percent") {
        Some(n) => n.parse::<u8>().context("--percent")?.clamp(5, 95),
        None => cfg.view_width_percent,
    };
    let pane = crate::apphost::open_mod_sidebar(
        &crate::apphost::App::from_env(),
        a.value("tab").context("--tab")?,
        a.value("mod").context("--mod")?,
        props,
        percent,
    )?;
    Ok(json!({ "pane": pane }))
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
    /// Written on the staged diff.
    #[serde(default)]
    cached: bool,
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
            cached: n.cached,
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
