# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-07-19

### Added

- Two-pane git view (changed-file list + colored diff) opened by one herdr
  action / keybinding; per-repo state, multiple repos concurrently.
- Worktree and branch scopes (`w`), staged/unstaged diff view (`tab`),
  auto-refresh via cheap status fingerprint polling.
- `Enter` opens the file in real nvim on the preview pane's PTY, at the first
  changed line; the diff view restores on exit.
- Stage/unstage (`s`), discard with confirmation (`x`), commit with the
  message written in nvim (`c`).
- Standalone mode: `herdr-gitview list` works outside herdr.
- Config file (base, split side, editor, poll interval, keybinding
  overrides) with never-fail parsing.
- Release packaging: platform binaries with checksum-verified installer,
  source-build fallback.
