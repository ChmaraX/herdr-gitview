# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Sidebar toggle (`toggle` / typically `cmd+shift+g`) no longer jumps to
  another tab that already has gitview open. Each tab can host its own
  sidebar instance; pressing the shortcut again in that tab closes it.
  The dedicated-tab toggle (`toggle-tab` / typically `cmd+g`) still
  focuses the existing view tab from elsewhere.
- Sidebar preview no longer sticks on "waiting for file list…": per-tab
  socket paths were long enough to hit macOS's AF_UNIX `sun_path` limit
  (bind failed; the list pane timed out). Tab keys in those paths are
  now 8 hex chars.

### Changed

- The sidebar (`toggle`) now splits the tab evenly: half for the panes you
  already had open, half for gitview. It was 40%, which read as lopsided
  against a full-width editor. `view_width_percent` still overrides it, and
  it is documented in the config table now — along with
  `list_width_percent`, which was never listed.
- Branch scope diffs against **the branch you actually branched off**, not
  always `main`/`master`. A branch cut from `develop` compares against
  `develop`; a branch stacked on another feature branch compares against that
  branch and shows only its own commits. Detection is by ancestry — every
  other branch is ranked by how recently this one diverged from it — so it
  does not depend on branch naming. Ties between siblings go to the trunk, a
  branch created *from* this one is ignored, and a trunk itself still uses the
  `origin/HEAD` → `origin/main` → … chain. `base` in the config still wins.

- Note and comment boxes are narrower: they leave air at the right edge and
  stop growing at 60 columns, so a note reads as prose instead of stretching
  across a wide pane.
- Notes are written **inline in the diff**, not in a popup pane. `a` opens a
  composer box under the lines you selected, prefilled when editing; you type
  where the note will live and watch the box grow. `enter` saves, `esc`
  cancels, `shift+enter` (or `ctrl+j`) adds a newline, and the caret keys are
  the ones you'd expect. An empty note is a cancel.
  - Whole-file notes and note edits started in the file list now hand off to
    the same composer (and the focus with them), so there is one place notes
    are written instead of two.
  - An empty composer shows `write a note…` behind the caret.
  - The `annotate` popup entrypoint is gone; `ask` and `pick-agent` remain.
- Review notes look like review comments now (shape borrowed from
  [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr)):
  - In the diff, a note is a boxed card spliced under the line it comments
    on — `╭─ note · lines 12-20 ─╮` with the text wrapped inside — instead of
    a single truncated row. Multi-line notes keep their lines inside one box,
    and the cards re-box themselves when the pane is resized.
  - A commented line's number is accented in the gutter, so an annotated
    line stays recognizable after its card scrolls out of view.
  - Note cards are decoration, not content: the cursor steps over a whole
    card instead of into it, clicks on one are inert, a drag extends past
    it, and a selection spanning a card leaves it untinted. Annotating a
    range that covers no source line is refused rather than silently
    becoming a whole-file note.
  - The notes panel groups notes under a heading per file and gives each
    note two lines — its anchor (`lines 12-20`, `line 7`, `whole file`) and
    its text — rather than one crowded row.

### Fixed

- **The view came up empty in some worktrees.** A path recorded as a
  submodule but present on disk as a symlink — how some worktree setups link
  back to their main checkout — makes `git status` exit with *no output*
  ("expected submodule path 'x' not to be a symbolic link"). That killed the
  file list outright, leaving the diff pane on "waiting for file list"
  forever. Status now retries ignoring submodule contents, which gets a full
  answer and still lists the submodule path itself as changed.
- A file list that cannot be loaded no longer takes the pane down with it:
  it starts empty, says why, and offers `r` to retry — rather than exiting
  and stranding the diff pane. The explanation persists instead of expiring
  into a false "working tree clean".
- **Folder actions could touch a file from another section.** "merge
  conflicts" and "changes" are both unstaged, so a folder row could not tell
  them apart — pressing `s` on `src/` under *merge conflicts* staged a
  modified file from *changes* that wasn't even visible under that row, and
  `x` offered to discard it. Rows now carry their section, so a folder action
  is confined to the section you selected and a conflicted folder is refused.
- Hovering a note in the notes view scrolled to the wrong note while another
  was being edited.
- On a diff past the 20 000-line render cap the note composer was truncated
  away while still taking keystrokes — you typed into a box that wasn't
  drawn. Cards are spliced into an already-capped document now.
- The `… diff truncated` notice was a legal cursor target reporting a source
  line number it had nothing to do with.
- A note whose line no longer exists in the diff silently became a whole-file
  note pinned at the top; it now keeps its range and is marked `anchor lost`.
- Annotating immediately after moving the cursor failed with "open that
  file's diff first" (and stole the focus): the compose request raced the
  list's debounced diff update. It now carries that update with it.
- A panic no longer leaves the terminal in kitty-keyboard mode with broken
  arrow keys; terminal modes are owned by a guard that runs on every exit
  path.
- The footer advertised `stage`/`unstage`/`discard` on selections that then
  refused to run, because it re-derived eligibility with a repo-wide
  predicate instead of asking the model.
- `shift+enter` inserts a newline in the note composer again. The diff pane
  never asked for the kitty keyboard protocol, which is the only way a
  terminal can report `shift+enter` as distinct from `enter`, so it was
  saving the note instead. (`ctrl+j` always worked and still does.)
- The diff pane now shows where you are. The cursor line had a tint so
  faint it was invisible in practice, and scrolling (the wheel, or
  `ctrl+d`/`ctrl+u` forwarded from the list) moved the viewport *without*
  the cursor — so focusing the diff pane often showed no cursor at all and
  the first `j` jumped somewhere unrelated. The cursor line is now a real
  cursorline, scrolling carries the cursor with it (unless a selection is
  live, which it would silently extend), and the header reports the cursor's
  file line as `ln 42` instead of the viewport's scroll offset.
- Note input no longer hides what you type. It is a real text area now:
  the text soft-wraps to the popup width and the view follows the caret,
  so a long note scrolls instead of being drawn past the bottom edge
  (previously anything past the second line simply vanished).
  - `shift+enter` (or `ctrl+j`, which every terminal can deliver) inserts a
    newline; `enter` still saves and `esc` still cancels.
  - Arrow keys, `home`/`end` and `ctrl+a`/`ctrl+e` move the caret, which is
    drawn where it actually is, so text can be edited mid-note rather than
    only appended to. `delete` removes forwards, `ctrl+w` /
    `ctrl+backspace` delete a word, `ctrl+u` clears.
  - The popup is bigger (72×14, was 64×8) and the title row shows a
    character count plus `↑`/`↓` when rows are scrolled out of view.
  - Multi-line notes render as one line in the file list and in the diff's
    note cards (line breaks shown as `⏎`), keep their real line breaks when
    sent to an agent, and survive being reopened for editing.

### Added

- Branch-only commit history: `w` in the log view toggles between all of
  `HEAD`'s commits and just the ones this branch added on top of its base
  (`<merge-base>..HEAD`). Opening the log (`l`) while the file list is in
  branch scope starts filtered, so "what did my agent commit here?" is one
  keypress. The header shows `log — <branch> vs <base>` or `log — <branch>
  all`, and the footer advertises the toggle.
- Folder-wide stage / unstage / discard: with the cursor on a directory row,
  `s`, `u` and `x` apply to every file under it instead of doing nothing.
  They stay section-aware — `s` on `src/` under *staged changes* unstages the
  subtree, under *changes* it stages it — and never touch conflicted files.
  Discarding a folder confirms with the file count first.

- Collapsible file-tree directories in the list pane: directory rows are
  now selectable (keyboard and mouse); `Enter` or a single click collapses
  or expands the subtree. Collapse state survives refreshes and is kept
  per section (staged vs. changes).
- `list_width_percent` config (default `25`): how much of the gitview area
  the file list takes (was a fixed 50/50 split).
- `view_width_percent` config (default `40`): how much of the tab the
  whole gitview sidebar takes.
- Sidebar mode: the `toggle` action now opens the view as a full-height
  sidebar on the right of the *current* tab (herdr-nvim style) — the
  existing pane layout is squeezed to the left and preview + list take
  the right `view_width_percent`. Toggling again closes the sidebar and
  gives the space back; an interrupted open is recovered on the next
  toggle (parked panes are moved back).
- `toggle-tab` action: the original dedicated-tab view, kept alongside
  the sidebar. Closing it refocuses the tab it was invoked from.
- Reuse an existing nvim (`reuse_tab_nvim`, default on): in sidebar
  mode, when another pane of the tab already runs nvim (plain or the
  herdr-nvim sidebar), Enter opens the file there — the diff preview
  stays up.
- Enter in the diff pane opens the editor on the current selection,
  exactly like Enter in the file list.

## [0.1.0] - 2026-07-22

### Added

- Two-pane git view (grouped file list + structured diff) opened by one
  herdr action / keybinding; per-repo state, multiple repos concurrently;
  the toggle focuses the view from other tabs and closes it from inside.
- VSCode-style sections (merge conflicts / staged / changes); a partially
  staged file appears in both, and the section decides which diff is shown.
- Structured diff renderer: syntax highlighting (syntect + two-face),
  red/green line tints, word-level change emphasis, line-number gutter,
  click-to-expand context folds, dark and light themes.
- Real-PTY editor loop: `Enter` opens nvim on the diff pane at the first
  changed line; remote file switching while nvim is open; graceful close
  with a save/discard dialog when quitting with unsaved changes.
- Stage/unstage (`s`/`u`), discard with confirmation (`x`), commit with
  the message written in nvim (`c`).
- Commit history view (`l`): commit list → per-commit files → diffs.
- Review notes: visual line selection (`v`/mouse drag), annotate (`a`),
  batched notes with inline cards and a notes view (`n`), edit/delete,
  and sending to an AI agent pane in the workspace (`p`) with the diff
  snippet attached.
- Native herdr floating popup dialogs (herdr ≥ 0.7.4) with in-pane
  fallbacks; full mouse support in both panes.
- Worktree ↔ branch scope, auto-refresh via status fingerprint polling,
  width-aware footers, standalone `herdr-gitview list` mode.
- Release packaging: platform binaries with checksum-verified installer
  and source-build fallback.

[Unreleased]: https://github.com/ChmaraX/herdr-gitview/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ChmaraX/herdr-gitview/releases/tag/v0.1.0
