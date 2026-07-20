# Thermo-nuclear code quality review — herdr-gitview

Scope: full audit of `src/` (16 .rs files, ~6.3k lines) + `tests/` (3 files, ~770 lines).
All 63 tests pass (`cargo test`: 32 + 18 + 7 + 6 unit/integration). Working tree clean apart from untracked `.pi-subagents/`.

Overall verdict: the *layered* parts of this codebase are genuinely good — `App`/`PreviewApp` are
thread-free and render-testable, `git.rs` parsers are pure and unit-tested, `render.rs` is a clean
pipeline, `keymap.rs`/`config.rs` are tight. The rot is concentrated exactly where the task
suspected: the two `run`/`event_loop` functions have become side-channel swamps (request flags,
popup answer slots, debounce state, probe state, external-modal state all live as loose locals and
`pub` fields), and the two panes duplicate each other's plumbing nearly line-for-line. There are
also several real bugs, one of which (stuck popup answers) degrades the product permanently within
a session.

---

## 1. Prioritized code-judo restructurings

### R1 (highest leverage): Replace the App→run-loop "request flag" side channels with an effect queue

**Problem.** The list `App` talks to its run loop through **seven** ad-hoc mailbox fields, each
polled at a different point in the loop:

- `list/app.rs:118-143` — `needs_reshow`, `after_edit`, `editor_close_request`,
  `annotate_request`, `send_notes_request`, `edit_note_request`, `delete_note_request`
- consumed piecemeal at `list/mod.rs:155-160` (needs_reshow), `:169-181` (editor_close_request),
  `:279-296` (annotate_request), `:315-326` (edit_note_request), `:336-338` (delete_note_request),
  `:312-314` (send_notes_request), `:224-230` (after_edit).

The preview has its own parallel set: `popup_request`, `notes_view_request`, `notes_rev`
(`preview/app.rs:129-147`, drained at `preview/mod.rs:110-160, 194-217`). Every new feature has
added one more flag and one more `if let Some(x) = app.x.take()` block at some loop position, which
is why the loops read as spaghetti and why ordering bugs (see B2, B6) crept in.

**Shape.** One outbound channel, drained at one point:

```rust
// app.rs
pub enum Effect {
    Reshow,
    Send(ToPreview),
    OpenPopup(PopupKind),          // Confirm{..}, Annotate{..}, EditNote{..}, PickAgent
    ProbeEditor { then: Option<EditorThen> },
    ResumeAfterEdit(EditorThen),
}
impl App {
    effects: Vec<Effect>,
    fn emit(&mut self, e: Effect) { self.effects.push(e); }
    pub fn drain_effects(&mut self) -> Vec<Effect> { std::mem::take(&mut self.effects) }
}

// mod.rs event loop — the ONLY consumer:
for eff in app.drain_effects() { dispatch(eff, &mut conn, &mut popups, tx); }
```

This collapses ~120 lines of scattered take()/poll blocks in `list/mod.rs` into a single match,
makes ordering explicit, and makes `App` behavior fully unit-testable ("pressing d in Notes mode
emits Send(DeleteNote{idx})"). Same treatment for `PreviewApp`.

### R2: One popup-answer subsystem instead of five hand-rolled answer slots

**Problem.** The spawn→stash-path→poll-every-tick→dispatch pattern is written out five times:
`list/mod.rs:117-121, 279-338, 341-371` (`popup_answer`, `annotate_answer`, `edit_note_answer`)
and `preview/mod.rs:105-190` (`annotate_answer`, `pick_answer`). Each copy has its own subtle
lifecycle (list's annotate poll even constructs a throwaway `Option` clone at `list/mod.rs:298`
just to satisfy `popup::poll`'s signature). None of the copies handles a popup that dies without
writing its answer file (see B1).

**Shape.**

```rust
// popup.rs
pub struct PendingPopup<K> { path: PathBuf, key: K, opened: Instant }
pub struct Popups<K> { pending: Option<PendingPopup<K>> }
impl<K> Popups<K> {
    pub fn open(&mut self, kind: &str, envs: &[..], size: (u16,u16), key: K) -> Result<..>;
    /// Returns (key, Answer) — Answer::Text(..) | Answer::Cancelled | Answer::Dead (timeout /
    /// pane gone), so the caller can clear `modal_external` and re-enable popups.
    pub fn poll(&mut self) -> Option<(K, Answer)>;
}
```

with `K` an enum per pane (`Confirm`, `Annotate{file}`, `EditNote{idx}`, `PickAgent`). One
implementation, one liveness policy, one place to fix B1.

### R3: Route keys through a pure classifier instead of stacked guard chains

**Problem.** `list/mod.rs:129-192` is the poster child: `free = conn.is_some() && modal.is_none()
&& busy.is_none()` computed up front, then an `if/else if` ladder where each arm re-tests a
*different subset* of `modal`/`busy`/`conn` — which is precisely how bug B4 (scroll keys bypass an
open modal) slipped in. The busy/probe editor logic at `:169-190` adds a second interleaved state
machine in the same block.

**Shape.**

```rust
enum KeyRoute { Modal, Activate, Commit, Reconnect, ForwardScroll(ToPreview), App }
fn route(action: Option<Action>, app: &App, has_conn: bool) -> KeyRoute { ... } // pure, testable
```

The event loop matches once on `KeyRoute`. The modal case comes **first** unconditionally, which
kills the whole class of "guard missing on one arm" bugs. Add unit tests for the routing table
(modal open + ctrl+d → Modal, busy + Enter → Activate, etc.).

### R4: Extract the shared pane runtime (dedupe the twin loops)

**Problem.** Near-identical code in `list/mod.rs` vs `preview/mod.rs`:

- `spawn_input_thread` — `list/mod.rs:624-644` vs `preview/mod.rs:391-421` (only difference: the
  pause flag; the list version can take an always-false flag)
- `spawn_ipc_forwarder` — `list/mod.rs:648-657` vs `preview/mod.rs:425-434` (identical modulo the
  message type; make it generic: `fn spawn_ipc_forwarder<T, E>(rx, tx, wrap: fn(T)->E, closed: E)`)
- `resolve_repo` — `list/mod.rs:728-733` vs `preview/mod.rs:573-578` (identical)
- `first_line` — `list/app.rs:806-808` vs `preview/mod.rs:565-567` (identical)
- terminal init/mouse-enable/restore bracket — `list/mod.rs:86-91` vs `preview/mod.rs:85-92`

**Shape.** A `src/pane.rs` (or `runtime.rs`) with `spawn_input_thread(tx, paused:
Arc<AtomicBool>)`, generic forwarder, `resolve_repo`, `first_line`, and a
`with_terminal(|term| ...)` bracket. ~120 duplicated lines disappear and the two loops shrink to
their actually-different logic.

### R5: One home for the nvim editor-session logic

**Problem.** The "remote-controlled nvim on the preview PTY" concept is smeared across four files:

- `preview/mod.rs:286-315` (`run_editor`, `editor_server_path`)
- `preview/editor.rs` (PTY suspend/run)
- `list/app.rs:516-533, 812-836` (`close_editor`, `editor_remote`, `editor_has_unsaved`)
- `list/mod.rs:432-446, 471-499` (`spawn_editor_probe`, `remote_open`)

The list side reaches into `crate::preview::editor_server_path()` (list/app.rs:817,
list/mod.rs:479) — a layering inversion: the *list* pane depends on the *preview* module for
editor internals, and `list/mod.rs` also calls `crate::preview::enable_mouse` (list/mod.rs:87).

**Shape.** `src/editor.rs` owning: `server_path()`, `remote(editor, args)`, `has_unsaved()`,
`request_close(save: bool)`, `open_file(path)`, plus the PTY runner. Both panes import it; neither
imports the other's internals. Move `enable_mouse`/`disable_mouse` into the shared runtime (R4).

### R6: Make `Note` a first-class shared type with stable IDs

**Problem.** The note is `(PathBuf, u32, u32, String)` in four places (`ipc.rs:120-122`,
`list/app.rs:139`, `list/ui.rs:216`, plus the snapshot mapping at `preview/mod.rs:199-204`) and a
`struct Note` only inside `preview/app.rs:53-63`. Cross-pane operations use raw indices
(`EditNote{idx}`, `DeleteNote{idx}`, `FocusNote{idx}`), which is racy: the list acts on a
*snapshot*; if the preview mutates the store between snapshot and command, the index dereferences
the wrong note (press `d` twice quickly in the notes view → second delete hits the note that
slid into slot idx).

**Shape.** Move `Note { id: u64, file, start, end, text, snippet }` into `ipc.rs` (serde-derived);
`ToPreview::{EditNote,DeleteNote,FocusNote}` carry `id`; the preview allocates monotonically
increasing ids. `ToList::Notes` sends `Vec<Note>` directly. The `workspace_agents()` 5-tuple
(`popup.rs:113`) deserves the same struct treatment (it is already JSON-serialized to the picker
popup anyway).

### R7: Cache branch-scope base resolution; kill the triplication

**Problem.** The "cfg.base empty → `detect_base()` → `merge_base()`" dance exists in three places:
`list/app.rs:743-757` (toggle_scope), `preview/mod.rs:521-529` (fetch_contents), and
`git.rs:288-291` (diff_ansi). Worse, `fetch_contents` re-runs it for **every debounced Show**: in
branch scope, each cursor move costs up to 5 extra git spawns (`symbolic-ref` + up to 4
`rev-parse` in detect_base) + `merge-base` before the two content fetches. Holding `j` in branch
scope is a process-spawn storm.

**Shape.** `Repo::resolve_base(cfg_base: &str) -> Result<BaseInfo { base, merge_base }>` plus a
per-worker `OnceCell<BaseInfo>` (invalidated on Refresh or scope change — the merge-base only
moves when HEAD or the remote head moves, so even a "recompute when fingerprint changes" policy is
fine). Both panes call the one method.

### R8: `sync_doc`/`restyle` — deduplicate and de-quadratize

**Problem.** `preview/app.rs`:

- `sync_doc` (`:239-296`) computes the card anchor list **twice** — once to insert labels
  (`:247-270`), once to recompute indices (`:271-283`). Compute `anchored: Vec<(line, label)>`
  once, sort once, derive `card_lines` from the same vector.
- `doc_to_built` (`:334-341`) is duplicated as a closure inside `begin_annotate` (`:507-513`).
- `restyle` (`:441-483`) calls `sync_doc`, which **clones the entire built `Text`**
  (`:243 doc = built.text.clone()`) on *every* `j`/`k`/drag. With the 20k-line cap that is 20k
  `Line`s × several `Span` `String`s cloned per keypress — a visible perf cliff on big diffs.

**Shape.** Keep `doc` as the clean copy; on cursor move, un-tint the previously tinted range and
tint the new one (store `last_tint: Range<usize>`), only rebuilding via `sync_doc` when the
built doc or the note set actually changes (`notes_rev` / unfold / new diff).

### R9: Delete the dead ANSI diff pipeline

`Repo::diff_ansi` (`git.rs:236-311`, plus `git_tristate` at `:101-110` which only it uses) has no
production caller — the preview renders via `fetch_contents` + `render::build` since the
structured renderer landed. Only `tests/git_repo.rs:187-280` keep it alive (six tests asserting
dead behavior). Same for `ShowReq::to_entry` (`preview/app.rs:35-46`) — zero callers; its doc
comment even refers to `diff_ansi`. Either delete both (~130 lines + 95 test lines) or document
why they're retained. Dead "contract" comments (`git.rs:214-234` referencing
`00-shared-contracts.md`) go with it.

### R10: Shared UI helpers

`bar_bg` (list/ui.rs:113-119 vs preview/ui.rs:78-84), `dim()`, the centered-message widget
(list/ui.rs:135-158 vs preview/ui.rs:113-127), and the footer hint-pair layout
(list/ui.rs:330-421 vs preview/ui.rs:132-193, the preview version being a weaker reimplementation
without the keep-help guarantee) are duplicated. One `ui_common.rs` with `bar_bg(theme)`, `dim()`,
`centered_msg`, `elide_head/tail`, and a single `hint_line(pairs, width, keep: &str)`.

---

## 2. Real bugs (verified from code)

- **B1 — Popup death permanently wedges the popup subsystem.** `popup::poll` (`popup.rs:82-89`)
  only ever returns when the answer file appears. If a popup pane is killed/crashes without
  writing (herdr closes it, user closes the pane, spawn succeeded but the process died), the
  pending slot stays `Some` forever. Consequences: list — `popup_answer` stuck ⇒
  `modal_external` stays true (`list/mod.rs:341-355`), the in-pane overlay is suppressed
  (`list/ui.rs:62`), so an *invisible* modal eats all keys until the user blindly hits Esc, and
  external confirms never work again this session; preview — `annotate_answer` stuck ⇒ every
  subsequent `a` press hits `Some(_) => {}` at `preview/mod.rs:161`, which **takes and drops** the
  `popup_request`, so annotation silently stops working for the rest of the session (and
  `pending_note` was already overwritten by `begin_annotate`, so a late answer would attach the
  old popup's text to the new pending note). Needs a liveness policy (timeout and/or
  `herdr pane get` check) — fits naturally in R2. Severity: **high** (permanent in-session
  degradation, no recovery path shown to the user).

- **B2 — `EditDone`/`EditorProbe` race can fire a stale `QuitView`/`Commit` later.**
  `spawn_editor_probe` (`list/mod.rs:436-446`) tells a clean nvim to quit, *then* posts
  `EditorProbe` to the list channel; the preview independently posts `EditDone` over IPC once
  nvim exits. If `EditDone` arrives first (`list/mod.rs:220-230`), `after_edit` is still `None`,
  so the `then` action is lost — and when the `EditorProbe{Some(false), then}` event lands
  afterwards (`:193-196`), it sets `app.after_edit = then` with **no editor running**. The stale
  `after_edit` sits there until the *next* `EditDone`, at which point the view suddenly quits or
  a commit starts, long after the user cancelled. Fix: clear `after_edit`/ignore probe results
  when `busy.is_none()`, or key the probe to an edit-session id. Severity: medium (timing-
  dependent, but the failure mode is "spontaneous quit/commit").

- **B3 — Hardcoded `'n'` key injection breaks under rebinding.** `list/mod.rs:239-246` (handling
  `ShowNotesView`) and `:256-260` (empty-notes exit) synthesize `KeyEvent::Char('n')` to toggle
  the notes view. If the user rebinds `notes_view` (`[keybindings] notes_view = "N"`), the
  injected `'n'` maps to nothing (notes view never opens from the preview, and the list is left
  stranded in an empty Notes view) — or worse, `'n'` may be rebound to a *different* action which
  then executes. Fix: make `toggle_notes_view` `pub` and call it directly. Severity: medium.

- **B4 — Open modal does not capture diff-scroll keys.** In the key ladder at
  `list/mod.rs:129-153`, the forward-scroll arm (`:150-153`) only checks `app.busy.is_none()` —
  not `app.modal.is_none()`. With the help or confirm overlay open, `ctrl+d`/`ctrl+u`/`home`/`end`
  scroll the preview pane instead of being swallowed by the modal (every other arm checks
  `modal`). Symptomatic of the guard stacking R3 addresses. Severity: low.

- **B5 — Deleting the last note flashes "notes sent".** `list/mod.rs:249-262`: any `Notes`
  snapshot that arrives empty while the list is in Notes mode prints `"notes sent"` — including
  the case where the user just pressed `d` on the last note. Track *why* the store emptied
  (the preview knows: `clear_notes` vs `delete_note`) or have the send flow flash its own message
  (the preview already flashes "notes sent to {agent}" at `preview/mod.rs:180`; the list message
  is redundant as well as wrong). Severity: low.

- **B6 — `q`/`c` during an in-flight browse-probe is silently dropped.** `list/mod.rs:186-190`:
  when `busy` is set and `probe_pending` is true, the `else` branch discards
  `editor_close_request` ("probe already in flight") — but the in-flight probe may carry
  `then: None` (it was started by cursor movement), so the user's explicit quit/commit intent is
  thrown away and nothing happens; they must press the key again after the probe settles. Fix:
  stash the request and attach it when the probe result arrives. Severity: low.

- **B7 — IPC reader prints to stderr while the TUI owns the terminal.** `ipc.rs:182`
  `eprintln!("ipc: skipping undecodable line: {err}")` runs on the reader thread of a live
  ratatui alt-screen/raw-mode app — it will smear garbage over the UI. Use `logx::log`.
  Severity: low (only fires on protocol corruption).

- **B8 — Blocking connects on the UI path.** `Conn::connect_retry` blocks; the startup call
  (`list/mod.rs:81`, 10 s budget) runs before `ratatui::init`, so a slow/hung preview leaves a
  blank pane for up to 10 s with no feedback; the `r`-retry path (`list/mod.rs:139`, 2 s) blocks
  the *event loop itself* — the UI freezes mid-frame for up to 2 s per press. The preview side
  got this right (listener on a thread posting `Event::Connected`, `preview/mod.rs:66-76`); the
  list should mirror it. Severity: low-medium (UX under failure).

- **B9 — Conflicted files preview as a full-file insertion.** `fetch_contents`
  (`preview/mod.rs:544-549`) diffs `:0` vs worktree; for an unmerged path stage 0 doesn't exist,
  so `file_at(":0", …)` returns `None` → old = "" → the whole conflicted file (markers included)
  renders as inserted lines. The old `diff_ansi` path showed git's combined conflict diff. At
  minimum special-case `ChangeKind::Conflicted` (e.g. diff `:1`/HEAD vs worktree, or show a
  "resolve in editor" notice). Severity: low-medium (misleading display in exactly the
  situation where accuracy matters).

## 3. Smaller local issues worth fixing

- `git.rs:361-371, 373-383` (`commit_files`): the first-NUL/40-bytes-no-tab heuristic to skip
  diff-tree's commit-id line is fragile hand-parsing; `git diff-tree --no-commit-id` removes the
  need entirely (twice).
- `preview/app.rs:255-258` + `:475-481` (`sync_doc`/`scroll_to_note`): a note whose anchor line is
  inside a fold gets `unwrap_or(0)` — the card silently renders at the very top of the doc,
  detached from its context. Consider auto-unfolding the anchor's fold or anchoring to the fold
  line.
- `preview/app.rs:186-201` (`set_diff`): resets `cursor_line`/`select_anchor` on *every* diff
  arrival, including same-file auto-refresh re-Shows — a background file change (build output,
  agent writes) during a `v` selection destroys the selection. Preserve cursor/selection on
  same-file re-Show, like `begin_show` already preserves scroll.
- `preview/mod.rs:337-359` (`deliver_notes`): the whole message (up to 40 snippet lines/note ×
  N notes) is passed as a single argv to `herdr pane send-text`; large batches risk ARG_MAX and
  shell-boundary weirdness. Consider stdin or a temp file if herdr supports it.
- `list/mod.rs:281-284` and `popup.rs:60-66`: popup env plumbing sends file paths inside env
  values via `--env K=V`; a path containing `=` is fine, but a newline would break framing.
  Sanitize or accept the risk consciously.
- `config.rs:28-30`: `theme` is a free string compared against `"light"` in five places
  (`highlight.rs:49`, `render.rs:120`, `list/ui.rs:114`, `preview/ui.rs:79`, `preview/app.rs:447`).
  Make it an enum like `ListSide` and pass a `Palette`/`ThemeColors` struct down instead.
- `preview/mod.rs:224-227`: the rhetorical comment ("Split the reader… ? No —") describes what the
  code *doesn't* do; rewrite to state the actual decision.
- `list/mod.rs:358-364`: the "y"/"n"/else→Esc string-to-KeyEvent bridge is stringly; with R1/R2,
  popup answers become typed events fed to `on_modal_key`-equivalent logic directly.
- `list/mod.rs:298, 330`: the `let mut pending = Some(path.clone())` throwaway to satisfy
  `popup::poll(&mut Option<PathBuf>)` shows the poll API has the wrong shape (fixed by R2).
- `git.rs:456` (`fingerprint`): runs a full `--untracked-files=all` status *and* the reload runs it
  again on change; cheap enough at 2 s cadence, but note the poll thread ignores
  `show_untracked=false` for fingerprinting (fingerprint changes on untracked churn even when the
  list hides untracked files → spurious refreshes).
- `list/mod.rs:220-230` vs `:264-276` (`EditDone` vs `GitDone`): both do
  on_edit_done + reshow + focus_self; only the tail differs — fold into one handler.
- `tests/`: no test covers the list event-loop routing at all (everything interesting found by
  this review lives there). R1/R3 would make `route()` and `Effect` emission directly testable —
  add tests then.
- `herdr-plugin.toml` / `main.rs:14`: the `bail!` help string omits `annotate|pick-agent` modes.

## 4. What is already good (keep it)

- `App`/`PreviewApp` deliberately own no channels/terminals — the render tests
  (`tests/render.rs`, `tests/preview.rs`) exploit this well.
- `git.rs` parser layer (porcelain v2 / name-status / numstat) is pure, defensive, and tested,
  including the nasty cases (renames with spaces, binary contagion, `u` records).
- Latest-wins diff worker (`preview/mod.rs:439-460`) and the stale-result guard
  (`preview/app.rs:176-183`) are the right shape and are tested.
- `ipc.rs` framing is simple and fully round-trip tested; EOF-as-channel-close is a clean idiom.
- `keymap.rs` collision detection, SHIFT folding, and override semantics are thorough and tested.
- `orchestrate.rs` is self-contained with sensible fallbacks (pane-id diffing, stale-state
  cleanup) and tests for the context-JSON mining.
- `editor.rs` restore-before-spawn / re-init-before-error-propagation ordering is correct and
  documented.

## 5. Residual risks

- Runtime behavior with a live herdr (popup panes, focus handoff, nvim remote control) was
  reviewed statically only; the race findings (B1, B2) are code-derived, not reproduced.
- The `.pi-subagents/` untracked directory is tooling state, not repo code; ignored per policy.
- I did not audit `herdr/` (vendored/submodule dir), `scripts/`, or the GitHub workflow beyond
  their existence; the task scoped `src/` + `tests/`.