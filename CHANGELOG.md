# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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
