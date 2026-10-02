//! The diff worker: fetches a request's old/new contents and builds the
//! styled document off the UI thread, in two phases — an uncolored doc as
//! soon as the diff is known, then the syntax colors for what is on screen.
//!
//! Every `Show` bumps a generation counter; work for an older generation is
//! dropped between highlight runs instead of finishing a build nobody will
//! look at, so holding `j` through large files never queues up rebuilds.

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
    // The highlighter is expensive to set up — build it once per worker.
    let hl = Highlighter::new(cfg.theme);
    // Branch-scope base resolution cached per HEAD (it only moves when
    // HEAD does) so holding j doesn't spawn a git storm.
    let mut base_cache: Option<(String, String)> = None; // (head, merge_base)
    // The file of the last Show handled: a repeat is a refresh.
    let mut last_file = None;
    while let Ok(first) = rx.recv() {
        // Only the newest generation's work is worth doing.
        let mut queue = vec![first];
        queue.extend(rx.try_iter());
        let newest = current.load(Ordering::SeqCst);
        for job in queue {
            let stale = |g: u64| current.load(Ordering::SeqCst) != g;
            let sent = match job {
                Job::Show { generation, .. } | Job::Highlight { generation, .. }
                    if generation != newest =>
                {
                    continue;
                }
                Job::Show { req, generation } => {
                    let cancelled = || stale(generation);
                    let budget = if last_file.as_ref() == Some(&req.file) {
                        REFRESH_BUDGET
                    } else {
                        Duration::ZERO
                    };
                    last_file = Some(req.file.clone());
                    let ctx = Ctx {
                        repo: &repo,
                        cfg: &cfg,
                        hl: &hl,
                        events: &events,
                    };
                    show(&ctx, &mut base_cache, req, budget, &cancelled)
                }
                Job::Highlight {
                    req,
                    job,
                    generation,
                } => match job.run(&hl, &|| stale(generation)) {
                    Some(highlights) => emit(&events, Event::Highlights { req, highlights }),
                    None => Ok(()),
                },
            };
            if sent.is_err() {
                return; // the session is gone
            }
        }
    }
}

/// What every Show needs from the worker thread.
struct Ctx<'a> {
    repo: &'a Repo,
    cfg: &'a Config,
    hl: &'a Highlighter,
    events: &'a Sender<Event>,
}

/// Fetch, diff, and deliver one request: uncolored first if coloring runs
/// past `budget`, colored once it is done. Stops early once `cancelled`
/// says a newer request has arrived.
fn show(
    ctx: &Ctx,
    base_cache: &mut Option<(String, String)>,
    req: ShowReq,
    budget: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Gone> {
    let Ctx {
        repo,
        cfg,
        hl,
        events,
    } = *ctx;
    let (old, new) = match fetch_contents(repo, cfg, &req, base_cache) {
        Ok(pair) => pair,
        Err(msg) => {
            return emit(
                events,
                Event::Diff {
                    req,
                    result: Err(msg),
                },
            );
        }
    };
    if cancelled() {
        return Ok(());
    }
    let mut doc = render::build_plain(
        &req.file,
        &old,
        &new,
        hl,
        cfg.theme,
        cfg.context_lines,
        cfg.tab_width,
    );
    let Some(job) = doc.highlight_job() else {
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
    if plain_sent.is_some() {
        emit(events, Event::Highlights { req, highlights })
    } else {
        doc.apply_highlights(&highlights);
        send_doc(events, req, doc)
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

/// The (old, new) content pair a request diffs, per scope/staged/commit.
/// `base_cache` holds `(head_sha, merge_base)` across calls.
fn fetch_contents(
    repo: &Repo,
    cfg: &Config,
    req: &ShowReq,
    base_cache: &mut Option<(String, String)>,
) -> Result<(String, String), String> {
    let path = &req.file;
    let old_path = req.orig_path.as_deref().unwrap_or(path);
    let err = |e: anyhow::Error| first_line(&e.to_string());
    let some =
        |r: Result<Option<String>, anyhow::Error>| r.map_err(err).map(Option::unwrap_or_default);

    if let Some(sha) = &req.commit {
        // One commit's change: parent vs commit (root commit → empty old).
        let old = some(repo.file_at(&format!("{sha}^"), old_path))?;
        let new = some(repo.file_at(sha, path))?;
        return Ok((old, new));
    }
    match req.scope {
        Scope::Branch => {
            let head = repo.head_sha().unwrap_or_default();
            let mb = match base_cache {
                Some((cached_head, mb)) if *cached_head == head => mb.clone(),
                _ => {
                    let (_, mb) = repo.resolve_base(&cfg.base).map_err(err)?;
                    *base_cache = Some((head, mb.clone()));
                    mb
                }
            };
            let old = some(repo.file_at(&mb, old_path))?;
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((old, new))
        }
        Scope::Worktree if req.cached => {
            // Staged view: HEAD vs index.
            let old = some(repo.file_at("HEAD", old_path))?;
            let new = some(repo.file_at(":0", path))?;
            Ok((old, new))
        }
        Scope::Worktree if req.kind == crate::git::ChangeKind::Conflicted => {
            // Unmerged paths have no stage-0 entry; diff "ours" (stage 2,
            // falling back to HEAD) against the conflicted worktree file.
            let ours = match repo.file_at(":2", path).map_err(err)? {
                Some(content) => content,
                None => some(repo.file_at("HEAD", path))?,
            };
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((ours, new))
        }
        Scope::Worktree => {
            // Unstaged view: index vs working tree (untracked → empty old).
            let old = some(repo.file_at(":0", path))?;
            let new = repo.file_in_worktree(path).unwrap_or_default();
            Ok((old, new))
        }
    }
}
