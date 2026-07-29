use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Worktree,
    Branch,
}

/// One entry of `git log` for the history view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageState {
    Unstaged,
    Staged,
    Partial,
    Untracked,
    /// Branch scope / conflicts: staging is not applicable.
    NA,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Repo-relative; the *new* path for renames.
    pub path: PathBuf,
    pub orig_path: Option<PathBuf>,
    pub kind: ChangeKind,
    pub stage: StageState,
    /// None = binary (numstat reports `-`).
    pub adds: Option<u32>,
    pub dels: Option<u32>,
}

pub struct Repo {
    pub root: PathBuf,
}

impl Repo {
    pub fn discover(dir: &Path) -> Result<Repo> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("spawning git")?;
        if !out.status.success() {
            bail!("{} is not inside a git repository", dir.display());
        }
        let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        Ok(Repo { root })
    }

    // ---- command wrappers -------------------------------------------------

    /// Run git, error on non-zero exit with stderr in the message.
    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = self.git_lenient(args)?;
        if !out.status.success() {
            bail!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }

    /// Run git, return the raw Output regardless of exit code.
    fn git_lenient(&self, args: &[&str]) -> Result<Output> {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .with_context(|| format!("spawning git {args:?}"))
    }

    // ---- branch / base ----------------------------------------------------

    /// Current branch name, None when detached.
    pub fn head_branch(&self) -> Option<String> {
        let out = self
            .git_lenient(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Ancestor branches considered as the fork parent. Each costs one
    /// `rev-list`, and the most recently updated ancestor is almost always
    /// the one; a handful is enough to be sure.
    const ANCESTOR_CANDIDATES: usize = 4;

    /// Trunk names tried when no ancestor branch explains the fork — the
    /// usual case, because the trunk moves on after a branch is cut from it.
    const TRUNKS: &'static [&'static str] = &[
        "origin/main",
        "origin/master",
        "origin/next",
        "origin/develop",
        "main",
        "master",
        "next",
        "develop",
    ];

    /// The branch this one was most likely created from.
    ///
    /// Ranks candidates by *ancestry*: how many commits separate HEAD from
    /// the point where it diverged from that candidate. Fewest wins, because
    /// that is the branch HEAD left most recently — which is the branch it
    /// was cut from. A branch stacked on another feature branch therefore
    /// diffs against that branch, not against the trunk.
    ///
    /// Known limitation: a branch created *from this one* also has a recent
    /// divergence point. Candidates containing all of HEAD are skipped, which
    /// covers the common shape; set `base` in the config to pin it otherwise.
    pub fn detect_fork_base(&self) -> Option<String> {
        let head = self.head_sha()?;
        let branch = self.head_branch();
        // A trunk is not cut *from* anything: every branch ever merged into
        // it is an ancestor, and picking the most recent of those would make
        // "branch scope" mean "the last few commits on main".
        if branch.as_deref().map(is_trunk_name).unwrap_or(false) {
            return None;
        }
        let existing = self.ref_names();
        let mut scored: Vec<(u32, u8, usize, String)> = Vec::new();

        // Branches whose tip is an ancestor of HEAD: the tip *is* the
        // divergence point, so this is the strongest evidence of a fork and
        // it costs one call to enumerate.
        for name in self.ancestor_refs(branch.as_deref()) {
            let distance = self.commits_between(&name, "HEAD");
            if distance > 0 {
                scored.push((distance, conventional_rank(&name), name.len(), name));
            }
        }

        // The trunks, which usually are *not* ancestors: they moved on after
        // this branch was cut, so they need a real merge-base.
        for name in self.trunk_refs(branch.as_deref(), &existing) {
            match self.merge_base(&name) {
                // A trunk containing every commit of HEAD has nothing to diff.
                Ok(mb) if mb != head && !mb.is_empty() => {
                    let distance = self.commits_between(&mb, "HEAD");
                    if distance > 0 {
                        scored.push((distance, conventional_rank(&name), name.len(), name));
                    }
                }
                _ => {}
            }
        }

        // Fewest commits since the divergence; ties (siblings cut from one
        // commit) go to the conventional trunk, then to a stable name order.
        scored.sort();
        scored.into_iter().next().map(|(_, _, _, name)| name)
    }

    /// Branch refs whose tip is an ancestor of HEAD, most recently updated
    /// first, minus this branch's own refs.
    fn ancestor_refs(&self, branch: Option<&str>) -> Vec<String> {
        let raw = self
            .git(&[
                "for-each-ref",
                "--merged",
                "HEAD",
                "--sort=-committerdate",
                "--format=%(refname:short)",
                "refs/heads",
                "refs/remotes",
            ])
            .unwrap_or_default();
        let remotes = self.remotes();
        String::from_utf8_lossy(&raw)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.ends_with("/HEAD"))
            .filter(|name| !is_own_branch(name, branch, &remotes))
            .take(Self::ANCESTOR_CANDIDATES)
            .map(str::to_string)
            .collect()
    }

    /// Every branch ref name, so trunk candidates can be filtered without a
    /// process per guess.
    fn ref_names(&self) -> std::collections::HashSet<String> {
        String::from_utf8_lossy(
            &self
                .git(&[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "refs/heads",
                    "refs/remotes",
                ])
                .unwrap_or_default(),
        )
        .lines()
        .map(str::to_string)
        .collect()
    }

    /// The trunk candidates that exist here, `origin/HEAD` first.
    fn trunk_refs(
        &self,
        branch: Option<&str>,
        existing: &std::collections::HashSet<String>,
    ) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(out) = self.git_lenient(&["symbolic-ref", "refs/remotes/origin/HEAD"])
            && out.status.success()
            && let Some(rest) = String::from_utf8_lossy(&out.stdout)
                .trim()
                .strip_prefix("refs/remotes/")
        {
            names.push(rest.to_string());
        }
        names.extend(Self::TRUNKS.iter().map(|s| s.to_string()));
        let remotes = self.remotes();
        let mut seen = std::collections::HashSet::new();
        names
            .into_iter()
            .filter(|name| existing.contains(name))
            .filter(|name| !is_own_branch(name, branch, &remotes))
            .filter(|name| seen.insert(name.clone()))
            .collect()
    }

    fn remotes(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.git(&["remote"]).unwrap_or_default())
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Commits in `from..to`, capped: the walk stops once a candidate is
    /// clearly too far away to be the parent, which keeps the search from
    /// paying for the whole history of a large repo. `u32::MAX` when git
    /// can't say, so an unanswerable candidate sorts last rather than first.
    fn commits_between(&self, from: &str, to: &str) -> u32 {
        const FAR_ENOUGH: &str = "--max-count=500";
        let range = format!("{from}..{to}");
        self.git(&["rev-list", "--count", FAR_ENOUGH, &range])
            .ok()
            .and_then(|out| String::from_utf8_lossy(&out).trim().parse().ok())
            .unwrap_or(u32::MAX)
    }

    /// Base branch for branch scope: the branch this one forked from, else
    /// origin/HEAD → origin/main → origin/master → main → master. Last
    /// resort: "HEAD".
    pub fn detect_base(&self) -> String {
        if let Some(fork) = self.detect_fork_base() {
            return fork;
        }
        if let Ok(out) = self.git_lenient(&["symbolic-ref", "refs/remotes/origin/HEAD"])
            && out.status.success()
        {
            let full = String::from_utf8_lossy(&out.stdout);
            // refs/remotes/origin/main -> origin/main
            if let Some(rest) = full.trim().strip_prefix("refs/remotes/") {
                return rest.to_string();
            }
        }
        for candidate in ["origin/main", "origin/master", "main", "master"] {
            if let Ok(out) = self.git_lenient(&["rev-parse", "--verify", "--quiet", candidate])
                && out.status.success()
            {
                return candidate.to_string();
            }
        }
        "HEAD".to_string()
    }

    /// The configured or auto-detected base ref plus its merge-base with
    /// HEAD, resolved in one place for both panes.
    pub fn resolve_base(&self, cfg_base: &str) -> Result<(String, String)> {
        let base = if cfg_base.is_empty() {
            self.detect_base()
        } else {
            cfg_base.to_string()
        };
        let mb = self.merge_base(&base)?;
        Ok((base, mb))
    }

    /// The current HEAD commit id (cache key for base resolution).
    pub fn head_sha(&self) -> Option<String> {
        let out = self.git_lenient(&["rev-parse", "HEAD"]).ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn merge_base(&self, base: &str) -> Result<String> {
        let out = self.git(&["merge-base", "HEAD", base])?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    // ---- worktree status --------------------------------------------------

    /// `git status --porcelain=v2`, with a fallback for repositories git
    /// refuses to report on at all.
    ///
    /// A path recorded as a submodule but present as a symlink (which is how
    /// some worktree setups link back to their main checkout) makes status
    /// exit 128 with *no output* — "expected submodule path 'x' not to be a
    /// symbolic link". Retrying with `--ignore-submodules=dirty` gets a full
    /// answer, still listing the submodule path itself as changed; only its
    /// internal dirtiness is hidden. Without this the whole pane had nothing
    /// to show and died.
    fn status_raw(&self, show_untracked: bool) -> Result<Vec<u8>> {
        let untracked = if show_untracked {
            "--untracked-files=all"
        } else {
            "--untracked-files=no"
        };
        let args = ["status", "--porcelain=v2", "-z", untracked];
        let out = self.git_lenient(&args)?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let retry = self.git_lenient(&[&args[..], &["--ignore-submodules=dirty"][..]].concat())?;
        if retry.status.success() {
            crate::logx::log(format!(
                "git status failed ({stderr}); retried ignoring submodule contents"
            ));
            return Ok(retry.stdout);
        }
        bail!("git status failed: {}", first_line(&stderr));
    }

    pub fn worktree_status(&self, show_untracked: bool) -> Result<Vec<FileEntry>> {
        let raw = self.status_raw(show_untracked)?;
        let mut entries = parse_porcelain_v2(&raw)?;

        // Merge line stats from numstat (unstaged + staged, summed per path).
        let mut stats: HashMap<PathBuf, Option<(u32, u32)>> = HashMap::new();
        for args in [
            &["diff", "--numstat", "-z", "-M"][..],
            &["diff", "--numstat", "-z", "-M", "--cached"][..],
        ] {
            for (path, stat) in parse_numstat(&self.git(args)?) {
                merge_stat(&mut stats, path, stat);
            }
        }
        for entry in &mut entries {
            match entry.kind {
                ChangeKind::Untracked => {
                    let (adds, dels) = untracked_stats(&self.root.join(&entry.path));
                    entry.adds = adds;
                    entry.dels = dels;
                }
                _ => {
                    if let Some(stat) = stats.get(&entry.path) {
                        (entry.adds, entry.dels) = match stat {
                            Some((a, d)) => (Some(*a), Some(*d)),
                            None => (None, None), // binary
                        };
                    }
                }
            }
        }
        sort_entries(&mut entries);
        Ok(entries)
    }

    // ---- branch scope -----------------------------------------------------

    pub fn branch_changes(&self, merge_base: &str) -> Result<Vec<FileEntry>> {
        let raw = self.git(&["diff", merge_base, "--name-status", "-z", "-M"])?;
        let mut entries = parse_name_status(&raw)?;

        let mut stats: HashMap<PathBuf, Option<(u32, u32)>> = HashMap::new();
        let numstat = self.git(&["diff", merge_base, "--numstat", "-z", "-M"])?;
        for (path, stat) in parse_numstat(&numstat) {
            merge_stat(&mut stats, path, stat);
        }
        for entry in &mut entries {
            if let Some(stat) = stats.get(&entry.path) {
                (entry.adds, entry.dels) = match stat {
                    Some((a, d)) => (Some(*a), Some(*d)),
                    None => (None, None),
                };
            }
        }
        sort_entries(&mut entries);
        Ok(entries)
    }

    // ---- diff rendering ---------------------------------------------------

    // ---- file content (for the structured diff renderer) ------------------

    /// A file's content at `rev` (e.g. `HEAD`, a sha, `<sha>^`), or at the
    /// index when `rev` is `":0"`. `None` when the path doesn't exist there.
    pub fn file_at(&self, rev: &str, path: &Path) -> Result<Option<String>> {
        let spec = format!("{rev}:{}", path.to_string_lossy());
        let out = self.git_lenient(&["show", &spec])?;
        if !out.status.success() {
            return Ok(None); // path absent at that rev (add/delete/rename)
        }
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    }

    /// The working-tree content of a repo-relative path; `None` when missing.
    pub fn file_in_worktree(&self, path: &Path) -> Option<String> {
        let bytes = std::fs::read(self.root.join(path)).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    // ---- history ----------------------------------------------------------

    /// Recent commits, newest first.
    pub fn log_commits(&self, limit: usize) -> Result<Vec<CommitInfo>> {
        self.log_range(None, limit)
    }

    /// Commits reachable from HEAD but not from `merge_base` — i.e. the
    /// commits this branch added on top of its base.
    pub fn log_branch_commits(&self, merge_base: &str, limit: usize) -> Result<Vec<CommitInfo>> {
        self.log_range(Some(&format!("{merge_base}..HEAD")), limit)
    }

    /// `git log [range]`, newest first. `range` is any revision range
    /// (`<base>..HEAD`); `None` walks all of HEAD's history.
    pub fn log_range(&self, range: Option<&str>, limit: usize) -> Result<Vec<CommitInfo>> {
        let n = limit.to_string();
        let mut args = vec![
            "log",
            "--format=%H%x00%h%x00%an%x00%ad%x00%s%x00",
            "--date=short",
            "-n",
            &n,
        ];
        if let Some(range) = range {
            args.push(range);
        }
        let raw = self.git(&args)?;
        let text = String::from_utf8_lossy(&raw);
        let mut fields = text.split('\0');
        let mut commits = Vec::new();
        while let (Some(sha), Some(short), Some(author), Some(date), Some(subject)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) {
            let sha = sha.trim_start(); // leading \n from the previous record
            if sha.is_empty() {
                break;
            }
            commits.push(CommitInfo {
                sha: sha.to_string(),
                short: short.to_string(),
                author: author.to_string(),
                date: date.to_string(),
                subject: subject.to_string(),
            });
        }
        Ok(commits)
    }

    /// Files changed by one commit (vs its parent; root commits work too).
    pub fn commit_files(&self, sha: &str) -> Result<Vec<FileEntry>> {
        let common = ["diff-tree", "-r", "--root", "--no-commit-id", "-z", "-M"];
        let raw = self.git(&[&common[..], &["--name-status", sha]].concat())?;
        let mut entries = parse_name_status(&raw)?;

        let numstat = self.git(&[&common[..], &["--numstat", sha]].concat())?;
        let mut stats: HashMap<PathBuf, Option<(u32, u32)>> = HashMap::new();
        for (path, stat) in parse_numstat(&numstat) {
            merge_stat(&mut stats, path, stat);
        }
        for entry in &mut entries {
            if let Some(stat) = stats.get(&entry.path) {
                (entry.adds, entry.dels) = match stat {
                    Some((a, d)) => (Some(*a), Some(*d)),
                    None => (None, None),
                };
            }
        }
        sort_entries(&mut entries);
        Ok(entries)
    }

    // ---- write side (phase 5) ---------------------------------------------

    pub fn stage(&self, p: &Path) -> Result<()> {
        self.stage_many(std::slice::from_ref(&p.to_path_buf()))
    }

    pub fn unstage(&self, p: &Path) -> Result<()> {
        self.unstage_many(std::slice::from_ref(&p.to_path_buf()))
    }

    /// Stage several paths in one `git add` (used for whole folders).
    pub fn stage_many(&self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let owned: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into()).collect();
        let mut args = vec!["add", "--"];
        args.extend(owned.iter().map(String::as_str));
        self.git(&args)?;
        Ok(())
    }

    /// Unstage several paths in one `git restore --staged`.
    pub fn unstage_many(&self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let owned: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into()).collect();
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(owned.iter().map(String::as_str));
        self.git(&args)?;
        Ok(())
    }

    /// Throw away all changes to this entry (worktree + index), per kind.
    pub fn discard(&self, e: &FileEntry) -> Result<()> {
        let path = e.path.to_string_lossy().to_string();
        match e.kind {
            ChangeKind::Conflicted => {
                bail!("resolve the conflict in the editor first")
            }
            ChangeKind::Untracked => {
                self.git(&["clean", "-f", "--", &path])?;
            }
            ChangeKind::Renamed => {
                // Unstage restores the old path in the index; then reset the
                // old path's content and sweep the new-path remnant.
                let orig = e
                    .orig_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                self.git(&["restore", "--staged", "--", &orig, &path])?;
                self.git(&["restore", "--", &orig])?;
                self.git(&["clean", "-f", "--", &path])?;
            }
            _ => match e.stage {
                StageState::Unstaged => {
                    self.git(&["restore", "--", &path])?;
                }
                _ => {
                    // Staged or partial: revert index first, then worktree.
                    self.git(&["restore", "--staged", "--", &path])?;
                    // Newly added files have no HEAD version to restore.
                    if e.kind == ChangeKind::Added {
                        self.git(&["clean", "-f", "--", &path])?;
                    } else {
                        self.git(&["restore", "--", &path])?;
                    }
                }
            },
        }
        Ok(())
    }

    /// Number of files with staged changes.
    pub fn staged_count(&self) -> Result<usize> {
        let out = self.git(&["diff", "--cached", "--name-only", "-z"])?;
        Ok(out.split(|b| *b == 0).filter(|s| !s.is_empty()).count())
    }

    /// `git log -1 --format=%h %s` — flavor text after a commit.
    pub fn last_commit_summary(&self) -> Option<String> {
        let out = self.git(&["log", "-1", "--format=%h %s"]).ok()?;
        let s = String::from_utf8_lossy(&out).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    // ---- change detection -------------------------------------------------

    /// Cheap fingerprint of the working tree state; a change means the file
    /// list should refresh. FNV-1a over the raw status bytes. Honors the
    /// untracked setting so hidden untracked churn can't cause refreshes.
    pub fn fingerprint(&self, show_untracked: bool) -> u64 {
        // Same fallback as `worktree_status`, or auto-refresh would go blind
        // in exactly the repositories that need the fallback.
        let raw = self.status_raw(show_untracked).unwrap_or_default();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in &raw {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

// ---- parsers (pure functions, unit-testable) ------------------------------

/// First line of a multi-line git error — git puts the news on top and the
/// advice underneath, and a footer only has room for the news.
pub fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// Parse `git status --porcelain=v2 -z` output. Records are NUL-terminated;
/// rename records (`2 …`) carry one extra NUL-separated field: the orig path.
fn parse_porcelain_v2(raw: &[u8]) -> Result<Vec<FileEntry>> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut entries = Vec::new();

    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let mut entry = match record.chars().next() {
            Some('1') => {
                let (xy, path) = split_ordinary(record, 8)?;
                FileEntry {
                    path: PathBuf::from(path),
                    orig_path: None,
                    kind: kind_from_xy(xy),
                    stage: stage_from_xy(xy),
                    adds: None,
                    dels: None,
                }
            }
            Some('2') => {
                // `2 XY sub mH mI mW hH hI Xscore path` + NUL + origPath
                let (xy, path) = split_ordinary(record, 9)?;
                let orig = fields
                    .next()
                    .context("rename record missing orig path field")?;
                FileEntry {
                    path: PathBuf::from(path),
                    orig_path: Some(PathBuf::from(orig)),
                    kind: ChangeKind::Renamed,
                    stage: stage_from_xy(xy),
                    adds: None,
                    dels: None,
                }
            }
            Some('u') => {
                let (_, path) = split_ordinary(record, 10)?;
                FileEntry {
                    path: PathBuf::from(path),
                    orig_path: None,
                    kind: ChangeKind::Conflicted,
                    stage: StageState::NA,
                    adds: None,
                    dels: None,
                }
            }
            Some('?') => FileEntry {
                path: PathBuf::from(record[2..].to_string()),
                orig_path: None,
                kind: ChangeKind::Untracked,
                stage: StageState::Untracked,
                adds: None,
                dels: None,
            },
            Some('!') => continue, // ignored files
            _ => continue,         // headers (`# …`) or unknown record types
        };
        // Conflicted overrides whatever kind_from_xy said for '1' records
        // (can't happen — u is its own record type — but keep sort stable).
        if entry.kind == ChangeKind::Conflicted {
            entry.stage = StageState::NA;
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Split an ordinary/rename/unmerged record: field 1 is XY, the path is
/// everything from field index `path_field` on (paths may contain spaces).
fn split_ordinary(record: &str, path_field: usize) -> Result<(&str, &str)> {
    let xy = record
        .split(' ')
        .nth(1)
        .context("porcelain record missing XY field")?;
    let mut start = 0;
    for _ in 0..path_field {
        start = record[start..]
            .find(' ')
            .map(|i| start + i + 1)
            .context("porcelain record too short")?;
    }
    Ok((xy, &record[start..]))
}

fn kind_from_xy(xy: &str) -> ChangeKind {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    // Prefer the index (staged) letter; fall back to worktree letter.
    let letter = if x != '.' { x } else { y };
    match letter {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        _ => ChangeKind::Modified,
    }
}

fn stage_from_xy(xy: &str) -> StageState {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    match (x != '.', y != '.') {
        (true, false) => StageState::Staged,
        (false, true) => StageState::Unstaged,
        (true, true) => StageState::Partial,
        (false, false) => StageState::Unstaged, // shouldn't appear
    }
}

/// Parse `git diff --name-status -z -M`: entries are
/// `status NUL path NUL` — renames are `Rnnn NUL orig NUL new NUL`.
fn parse_name_status(raw: &[u8]) -> Result<Vec<FileEntry>> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut entries = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let letter = status.chars().next().unwrap_or('M');
        let (kind, path, orig_path) = match letter {
            'R' | 'C' => {
                let orig = fields.next().context("rename missing orig path")?;
                let new = fields.next().context("rename missing new path")?;
                (ChangeKind::Renamed, new, Some(PathBuf::from(orig)))
            }
            other => {
                let path = fields.next().context("name-status missing path")?;
                let kind = match other {
                    'A' => ChangeKind::Added,
                    'D' => ChangeKind::Deleted,
                    'U' => ChangeKind::Conflicted,
                    _ => ChangeKind::Modified,
                };
                (kind, path, None)
            }
        };
        entries.push(FileEntry {
            path: PathBuf::from(path),
            orig_path,
            kind,
            stage: StageState::NA,
            adds: None,
            dels: None,
        });
    }
    Ok(entries)
}

/// Parse `git diff --numstat -z`. Entry: `adds TAB dels TAB path NUL`;
/// renames instead end with `NUL orig NUL new NUL`. `-` = binary → None.
fn parse_numstat(raw: &[u8]) -> Vec<(PathBuf, Option<(u32, u32)>)> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut out = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let mut cols = field.split('\t');
        let (Some(adds), Some(dels)) = (cols.next(), cols.next()) else {
            continue;
        };
        let path = match cols.next() {
            Some(p) if !p.is_empty() => p.to_string(),
            // rename form: path came as two extra NUL fields (orig, new)
            _ => {
                let _orig = fields.next();
                match fields.next() {
                    Some(new) => new.to_string(),
                    None => continue,
                }
            }
        };
        let stat = match (adds.parse::<u32>(), dels.parse::<u32>()) {
            (Ok(a), Ok(d)) => Some((a, d)),
            _ => None, // `-` = binary
        };
        out.push((PathBuf::from(path), stat));
    }
    out
}

/// How "trunk-like" a branch name is, for breaking ties between candidates
/// that were all cut from the same commit. Lower is more conventional.
fn conventional_rank(name: &str) -> u8 {
    let short = name.rsplit_once('/').map(|(_, b)| b).unwrap_or(name);
    match short {
        "main" => 0,
        "master" => 1,
        "next" => 2,
        "develop" | "development" => 3,
        _ => 4,
    }
}

/// Is this a trunk name rather than a feature branch?
fn is_trunk_name(branch: &str) -> bool {
    conventional_rank(branch) < 4
}

/// Is `name` this branch's own ref? `origin/feature/x` is the same branch as
/// `feature/x`, and diffing a branch against itself shows nothing.
fn is_own_branch(name: &str, branch: Option<&str>, remotes: &[String]) -> bool {
    match branch {
        Some(b) => name == b || remotes.iter().any(|r| name == format!("{r}/{b}")),
        None => false,
    }
}

/// Sum stats per path; binary (None) is contagious.
fn merge_stat(
    stats: &mut HashMap<PathBuf, Option<(u32, u32)>>,
    path: PathBuf,
    stat: Option<(u32, u32)>,
) {
    let slot = stats.entry(path).or_insert(Some((0, 0)));
    *slot = match (*slot, stat) {
        (Some((a1, d1)), Some((a2, d2))) => Some((a1 + a2, d1 + d2)),
        _ => None,
    };
}

/// Untracked file "stats": every line is an add. Skip binaries (NUL byte in
/// the first chunk) and files > 1 MiB.
fn untracked_stats(path: &Path) -> (Option<u32>, Option<u32>) {
    const MAX: u64 = 1024 * 1024;
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX => {}
        _ => return (None, None),
    }
    let Ok(bytes) = std::fs::read(path) else {
        return (None, None);
    };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return (None, None); // binary
    }
    let mut lines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        lines += 1; // unterminated last line still counts
    }
    (Some(lines), Some(0))
}

/// Conflicted first, then directory-grouped lexicographic by path.
fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        let conflict = |e: &FileEntry| e.kind != ChangeKind::Conflicted;
        conflict(a)
            .cmp(&conflict(b))
            .then_with(|| a.path.cmp(&b.path))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_ordinary_records() {
        // staged-only, unstaged-only, partial
        let raw = b"1 M. N... 100644 100644 100644 abc def staged.rs\0\
                    1 .M N... 100644 100644 100644 abc def unstaged.rs\0\
                    1 MM N... 100644 100644 100644 abc def partial.rs\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].stage, StageState::Staged);
        assert_eq!(entries[1].stage, StageState::Unstaged);
        assert_eq!(entries[2].stage, StageState::Partial);
        assert!(entries.iter().all(|e| e.kind == ChangeKind::Modified));
    }

    #[test]
    fn porcelain_rename_carries_orig_path() {
        let raw = b"2 R. N... 100644 100644 100644 abc def R100 new name.rs\0old name.rs\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("new name.rs"));
        assert_eq!(entries[0].orig_path, Some(PathBuf::from("old name.rs")));
        assert_eq!(entries[0].kind, ChangeKind::Renamed);
        assert_eq!(entries[0].stage, StageState::Staged);
    }

    #[test]
    fn porcelain_untracked_conflicted_ignored() {
        let raw = b"? new file.txt\0\
                    u UU N... 100644 100644 100644 100644 a b c conflict.rs\0\
                    ! ignored.log\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, ChangeKind::Untracked);
        assert_eq!(entries[0].path, PathBuf::from("new file.txt"));
        assert_eq!(entries[1].kind, ChangeKind::Conflicted);
        assert_eq!(entries[1].stage, StageState::NA);
    }

    #[test]
    fn numstat_parses_plain_binary_and_rename() {
        let raw = b"3\t1\tsrc/a.rs\0-\t-\timg.png\x000\t0\0old.rs\0new.rs\0";
        let stats = parse_numstat(raw);
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0], (PathBuf::from("src/a.rs"), Some((3, 1))));
        assert_eq!(stats[1], (PathBuf::from("img.png"), None));
        assert_eq!(stats[2], (PathBuf::from("new.rs"), Some((0, 0))));
    }

    #[test]
    fn name_status_kinds_and_rename() {
        let raw = b"M\0mod.rs\0A\0added.rs\0D\0gone.rs\0R087\0old.rs\0new.rs\0";
        let entries = parse_name_status(raw).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].kind, ChangeKind::Modified);
        assert_eq!(entries[1].kind, ChangeKind::Added);
        assert_eq!(entries[2].kind, ChangeKind::Deleted);
        assert_eq!(entries[3].kind, ChangeKind::Renamed);
        assert_eq!(entries[3].path, PathBuf::from("new.rs"));
        assert_eq!(entries[3].orig_path, Some(PathBuf::from("old.rs")));
    }

    #[test]
    fn merge_stat_sums_and_binary_is_contagious() {
        let mut stats = HashMap::new();
        merge_stat(&mut stats, "a.rs".into(), Some((1, 2)));
        merge_stat(&mut stats, "a.rs".into(), Some((3, 4)));
        merge_stat(&mut stats, "b.png".into(), Some((5, 0)));
        merge_stat(&mut stats, "b.png".into(), None);
        assert_eq!(stats[&PathBuf::from("a.rs")], Some((4, 6)));
        assert_eq!(stats[&PathBuf::from("b.png")], None);
    }

    #[test]
    fn sort_puts_conflicts_first_then_paths() {
        let entry = |path: &str, kind| FileEntry {
            path: path.into(),
            orig_path: None,
            kind,
            stage: StageState::NA,
            adds: None,
            dels: None,
        };
        let mut entries = vec![
            entry("z/deep.rs", ChangeKind::Modified),
            entry("a.rs", ChangeKind::Modified),
            entry("m/conflict.rs", ChangeKind::Conflicted),
        ];
        sort_entries(&mut entries);
        let paths: Vec<_> = entries
            .iter()
            .map(|e| e.path.to_string_lossy().to_string())
            .collect();
        assert_eq!(paths, vec!["m/conflict.rs", "a.rs", "z/deep.rs"]);
    }
}
