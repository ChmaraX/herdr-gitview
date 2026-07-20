# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.0]

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
