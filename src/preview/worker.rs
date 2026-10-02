//! The diff worker: fetches a request's old/new contents and builds the
//! styled document off the UI thread, in two phases — an uncolored doc as
//! soon as the diff is known, then the syntax colors for what is on screen.
//!
//! Every `Show` bumps a generation counter; work for an older generation is
//! dropped between highlight runs instead of finishing a build nobody will
//! look at, so holding `j` through large files never queues up rebuilds.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::app::ShowReq;
use super::highlight::Highlighter;
use super::render::{self, HighlightJob};
use super::session::Event;
use crate::config::Config;
use crate::git::{Repo, Scope, first_line};

/// How long highlighting may hold back a *refresh* of the file already on
/// screen: a re-diff colored within this replaces the colored doc in one
/// step instead of flashing uncolored. A newly selected file always paints
/// uncolored first, so browsing never waits on the grammar.
const REFRESH_BUDGET: Duration = Duration::from_millis(20);

enum Job {
    /// Build the diff for this request.
    Show { req: ShowReq, generation: u64 },
    /// Build these into the cache without showing them.
    Prefetch { reqs: Vec<ShowReq>, generation: u64 },
    /// Color lines revealed after the fact (an unfolded run).
    Highlight {
        req: ShowReq,
        job: HighlightJob,
        generation: u64,
    },
}

/// The UI thread's handle on the worker thread.
pub struct Worker {
    jobs: Sender<Job>,
    generation: Arc<AtomicU64>,
}

impl Worker {
    pub fn spawn(events: Sender<Event>, repo: Repo, cfg: Config) -> Worker {
        let (jobs, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = Arc::clone(&generation);
        thread::spawn(move || run(rx, events, repo, cfg, current));
        Worker { jobs, generation }
    }

    /// Build `req`, superseding everything asked for before it.
    pub fn show(&self, req: ShowReq) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.jobs.send(Job::Show { req, generation });
    }

    /// Build `reqs` into the cache, until the next `show` interrupts.
    pub fn prefetch(&self, reqs: Vec<ShowReq>) {
        let generation = self.generation.load(Ordering::SeqCst);
        let _ = self.jobs.send(Job::Prefetch { reqs, generation });
    }

    /// Color `job`'s lines of the doc currently shown for `req`.
    pub fn highlight(&self, req: ShowReq, job: HighlightJob) {
        let generation = self.generation.load(Ordering::SeqCst);
        let _ = self.jobs.send(Job::Highlight {
            req,
            job,
            generation,
        });
    }
}

fn run(rx: Receiver<Job>, events: Sender<Event>, repo: Repo, cfg: Config, current: Arc<AtomicU64>) {
    let mut w = State {
        // The highlighter is expensive to set up — build it once per worker.
        hl: Highlighter::new(cfg.theme),
        repo,
        cfg,
        events,
        base_cache: None,
        last_file: None,
        docs: DocCache::default(),
    };
    while let Ok(first) = rx.recv() {
        // Only the newest generation's work is worth doing.
        let mut queue = vec![first];
        queue.extend(rx.try_iter());
        let newest = current.load(Ordering::SeqCst);
        for job in queue {
            let stale = |g: u64| current.load(Ordering::SeqCst) != g;
            let sent = match job {
                Job::Show { generation, .. }
                | Job::Prefetch { generation, .. }
                | Job::Highlight { generation, .. }
                    if generation != newest =>
                {
                    continue;
                }
                Job::Show { req, generation } => w.show(req, &|| stale(generation)),
                Job::Prefetch { reqs, generation } => reqs
                    .into_iter()
                    .try_for_each(|req| w.prefetch(req, &|| stale(generation))),
                Job::Highlight {
                    req,
                    job,
                    generation,
                } => match job.run(&w.hl, &|| stale(generation)) {
                    Some(highlights) => emit(&w.events, Event::Highlights { req, highlights }),
                    None => Ok(()),
                },
            };
            if sent.is_err() {
                return; // the session is gone
            }
        }
    }
}

/// The worker thread's long-lived state.
struct State {
    hl: Highlighter,
    repo: Repo,
    cfg: Config,
    events: Sender<Event>,
    /// Branch-scope base resolution cached per HEAD (it only moves when
    /// HEAD does) so holding j doesn't spawn a git storm.
    base_cache: Option<(String, String)>, // (head, merge_base)
    /// The file of the last Show handled: a repeat is a refresh.
    last_file: Option<PathBuf>,
    docs: DocCache,
}

impl State {
    /// Build `req` into the cache only — nothing is sent but a note of the
    /// time it took. Skipped when already cached; dropped on `cancelled`.
    fn prefetch(&mut self, req: ShowReq, cancelled: &dyn Fn() -> bool) -> Result<(), Gone> {
        let started = Instant::now();
        let Ok(fetched) = fetch(&self.repo, &self.cfg, &req, &mut self.base_cache) else {
            return Ok(());
        };
        let Some((old, new)) = fetched.stamps.clone() else {
            return Ok(()); // uncacheable — nothing to gain
        };
        let key = Key {
            req: req.clone(),
            old,
            new,
        };
        if cancelled() || self.docs.contains(&key) {
            return Ok(());
        }
        let Some(doc) = self.build_colored(&req, &fetched, cancelled) else {
            return Ok(());
        };
        self.docs.put(key, doc);
        emit(
            &self.events,
            Event::Prefetched {
                file: req.file,
                took: started.elapsed(),
            },
        )
    }

    /// Diff and fully color `fetched`, or `None` once `cancelled`.
    fn build_colored(
        &self,
        req: &ShowReq,
        fetched: &Fetched,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<render::DiffDoc> {
        let cfg = &self.cfg;
        let mut doc = render::build_plain(
            &req.file,
            &fetched.old,
            &fetched.new,
            &self.hl,
            cfg.theme,
            cfg.context_lines,
        );
        if let Some(job) = doc.highlight_job() {
            doc.apply_highlights(&job.run(&self.hl, cancelled)?);
        }
        Some(doc)
    }

    /// Fetch, diff, and deliver one request: from the cache when its
    /// content is unchanged, else uncolored first if coloring runs past the
    /// budget and colored once it is done. Stops early once `cancelled`
    /// says a newer request has arrived.
    fn show(&mut self, req: ShowReq, cancelled: &dyn Fn() -> bool) -> Result<(), Gone> {
        let budget = if self.last_file.as_ref() == Some(&req.file) {
            REFRESH_BUDGET
        } else {
            Duration::ZERO
        };
        self.last_file = Some(req.file.clone());
        let fetched = match fetch(&self.repo, &self.cfg, &req, &mut self.base_cache) {
            Ok(fetched) => fetched,
            Err(msg) => {
                return emit(
                    &self.events,
                    Event::Diff {
                        req,
                        result: Err(msg),
                    },
                );
            }
        };
        let key = fetched.stamps.map(|(old, new)| Key {
            req: req.clone(),
            old,
            new,
        });
        if let Some(doc) = key.as_ref().and_then(|k| self.docs.get(k)) {
            return send_doc(&self.events, req, doc);
        }
        if cancelled() {
            return Ok(());
        }
        let (hl, cfg, events) = (&self.hl, &self.cfg, &self.events);
        let mut doc = render::build_plain(
            &req.file,
            &fetched.old,
            &fetched.new,
            hl,
            cfg.theme,
            cfg.context_lines,
            cfg.tab_width,
        );
        let Some(job) = doc.highlight_job() else {
            if let Some(key) = key {
                self.docs.put(key, doc.clone());
            }
            return send_doc(events, req, doc);
        };

        // Highlight against the budget: the first check past it sends the
        // plain doc, so a slow grammar costs colors, never the first paint.
        let started = Instant::now();
        let plain_sent = std::cell::RefCell::new(None);
        let highlights = job.run(hl, &|| {
            let mut sent = plain_sent.borrow_mut();
            if sent.is_none() && started.elapsed() >= budget && !cancelled() {
                *sent = Some(send_doc(events, req.clone(), doc.clone()));
            }
            cancelled()
        });
        let plain_sent = plain_sent.into_inner();
        if let Some(Err(err)) = plain_sent {
            return Err(err);
        }
        let Some(highlights) = highlights else {
            return Ok(()); // superseded mid-highlight
        };
        doc.apply_highlights(&highlights);
        if let Some(key) = key {
            self.docs.put(key, doc.clone());
        }
        if plain_sent.is_some() {
            emit(&self.events, Event::Highlights { req, highlights })
        } else {
            send_doc(&self.events, req, doc)
        }
    }
}

/// Docs kept for re-visits.
const CACHED_DOCS: usize = 32;

/// What one side of a diff was built from: a git object id, or a hash of
/// a worktree file's content.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stamp {
    Absent,
    Blob(String),
    File(u64),
}

/// A built doc is reusable while the request and both sides are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    req: ShowReq,
    old: Stamp,
    new: Stamp,
}

/// Least-recently-used docs, most recent last.
#[derive(Default)]
struct DocCache {
    entries: Vec<(Key, render::DiffDoc)>,
}

impl DocCache {
    fn contains(&self, key: &Key) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    fn get(&mut self, key: &Key) -> Option<render::DiffDoc> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(at);
        let doc = entry.1.clone();
        self.entries.push(entry);
        Some(doc)
    }

    fn put(&mut self, key: Key, doc: render::DiffDoc) {
        self.entries.retain(|(k, _)| k.req != key.req); // superseded content
        if self.entries.len() >= CACHED_DOCS {
            self.entries.remove(0);
        }
        self.entries.push((key, doc));
    }
}

fn send_doc(events: &Sender<Event>, req: ShowReq, doc: render::DiffDoc) -> Result<(), Gone> {
    emit(
        events,
        Event::Diff {
            req,
            result: Ok(Box::new(doc)),
        },
    )
}

/// The session hung up; the worker should stop.
struct Gone;

fn emit(events: &Sender<Event>, event: Event) -> Result<(), Gone> {
    events.send(event).map_err(|_| Gone)
}

/// One request's two sides, plus what identifies them for the cache
/// (`None` when they can't be identified: the legacy fallback).
struct Fetched {
    old: String,
    new: String,
    stamps: Option<(Stamp, Stamp)>,
}

/// Where one side of a diff comes from.
enum Source {
    /// `(rev, path)` pairs (rev `:0` = the index); the first present wins.
    Rev(Vec<(String, String)>),
    Worktree(PathBuf),
}

/// The two sides a request diffs, per scope/staged/commit.
/// `base_cache` holds `(head_sha, merge_base)` across calls.
fn sources(
    repo: &Repo,
    cfg: &Config,
    req: &ShowReq,
    base_cache: &mut Option<(String, String)>,
) -> Result<(Source, Source), String> {
    let path = req.file.to_string_lossy();
    let old_path = req
        .orig_path
        .as_deref()
        .map_or(path.clone(), |p| p.to_string_lossy());
    let rev = |rev: &str, path: &str| Source::Rev(vec![(rev.to_string(), path.to_string())]);
    let worktree = Source::Worktree(req.file.clone());
    if let Some(sha) = &req.commit {
        // One commit's change: parent vs commit (root commit → empty old).
        return Ok((rev(&format!("{sha}^"), &old_path), rev(sha, &path)));
    }
    Ok(match req.scope {
        Scope::Branch => {
            let head = repo.head_sha().unwrap_or_default();
            let mb = match base_cache {
                Some((cached_head, mb)) if *cached_head == head => mb.clone(),
                _ => {
                    let (_, mb) = repo
                        .resolve_base(&cfg.base)
                        .map_err(|e| first_line(&e.to_string()))?;
                    *base_cache = Some((head, mb.clone()));
                    mb
                }
            };
            (rev(&mb, &old_path), worktree)
        }
        // Staged view: HEAD vs index.
        Scope::Worktree if req.cached => (rev("HEAD", &old_path), rev(":0", &path)),
        // Unmerged paths have no stage-0 entry; diff "ours" (stage 2,
        // falling back to HEAD) against the conflicted worktree file.
        Scope::Worktree if req.kind == crate::git::ChangeKind::Conflicted => (
            Source::Rev(vec![
                (":2".to_string(), path.to_string()),
                ("HEAD".to_string(), path.to_string()),
            ]),
            worktree,
        ),
        // Unstaged view: index vs working tree (untracked → empty old).
        Scope::Worktree => (rev(":0", &old_path), worktree),
    })
}

/// Read both sides of `req` — every git side in a single `cat-file` call —
/// stamping each for the cache.
fn fetch(
    repo: &Repo,
    cfg: &Config,
    req: &ShowReq,
    base_cache: &mut Option<(String, String)>,
) -> Result<Fetched, String> {
    let (old_src, new_src) = sources(repo, cfg, req, base_cache)?;
    let specs: Vec<String> = [&old_src, &new_src]
        .into_iter()
        .flat_map(|s| match s {
            Source::Rev(revs) => revs.iter().map(|(r, p)| format!("{r}:{p}")).collect(),
            Source::Worktree(_) => Vec::new(),
        })
        .collect();
    let blobs = if specs.is_empty() {
        Vec::new()
    } else {
        match repo.read_blobs(&specs) {
            Ok(blobs) => blobs,
            // Unsendable path: read each side the slow way, uncached.
            Err(_) => return fetch_uncached(repo, &old_src, &new_src),
        }
    };
    let mut blobs = blobs.into_iter();
    let mut side = |src: &Source| -> (String, Stamp) {
        match src {
            Source::Rev(revs) => {
                // Consume this side's answers in full, first present wins.
                let answers: Vec<_> = blobs.by_ref().take(revs.len()).collect();
                let found = answers.into_iter().flatten().next();
                match found {
                    Some(blob) => (blob.content, Stamp::Blob(blob.oid)),
                    None => (String::new(), Stamp::Absent),
                }
            }
            Source::Worktree(path) => worktree_side(repo, path),
        }
    };
    let (old, old_stamp) = side(&old_src);
    let (new, new_stamp) = side(&new_src);
    Ok(Fetched {
        old,
        new,
        stamps: Some((old_stamp, new_stamp)),
    })
}

/// A worktree file, stamped by a hash of the content read: it is in hand
/// anyway, and unlike mtime+size it can't miss a same-size edit landing
/// within one timestamp tick.
fn worktree_side(repo: &Repo, path: &std::path::Path) -> (String, Stamp) {
    use std::hash::{Hash, Hasher};
    match repo.file_in_worktree(path) {
        Some(content) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            content.hash(&mut h);
            (content, Stamp::File(h.finish()))
        }
        None => (String::new(), Stamp::Absent),
    }
}

/// The per-side `git show` route, for paths `cat-file --batch` can't take.
fn fetch_uncached(repo: &Repo, old: &Source, new: &Source) -> Result<Fetched, String> {
    let read = |src: &Source| -> Result<String, String> {
        match src {
            Source::Rev(revs) => {
                for (rev, path) in revs {
                    let found = repo
                        .file_at(rev, std::path::Path::new(path))
                        .map_err(|e| first_line(&e.to_string()))?;
                    if let Some(content) = found {
                        return Ok(content);
                    }
                }
                Ok(String::new())
            }
            Source::Worktree(path) => Ok(repo.file_in_worktree(path).unwrap_or_default()),
        }
    };
    Ok(Fetched {
        old: read(old)?,
        new: read(new)?,
        stamps: None,
    })
}
