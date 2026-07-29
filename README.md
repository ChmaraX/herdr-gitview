# herdr-gitview

[![CI](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml/badge.svg)](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ChmaraX/herdr-gitview)](https://github.com/ChmaraX/herdr-gitview/releases/latest)
[![License](https://img.shields.io/github/license/ChmaraX/herdr-gitview)](LICENSE)

<p align="center">
  <a href="#features">features</a> · <a href="#install">install</a> · <a href="#quick-start">quick start</a> · <a href="#configuration">configuration</a> · <a href="#limitations">limitations</a> · <a href="CHANGELOG.md">changelog</a>
</p>

A git view for [herdr](https://herdr.dev). Changed files on one side, a
syntax-highlighted diff on the other. Press `Enter` and the diff pane
*becomes* your real nvim - opened at the first changed line. You never leave
the terminal.

Inspired by [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr),
rebuilt around a real editor - edit diffs in place with full nvim + LSP
support, not just view them.

https://github.com/user-attachments/assets/1cfdf22a-fc5a-40e2-af6c-546156d05c3b

## Features

- **Grouped changes** - conflicts, staged, and unstaged changes in separate
  sections. `s` moves a file between staged and unstaged.
  <img width="1541" height="780" alt="image" src="https://github.com/user-attachments/assets/bf0bbe92-817f-4db2-b023-04cd35fe90f8" />

- **Readable diffs** - syntax highlighting, word-level emphasis on edited
  lines, and collapsible context folds you expand with a click.
- **Edit in real nvim** - `Enter` turns the diff pane into your actual nvim:
  full LSP, plugins, colors. Edit the file right there at the changed line,
  `:wq`, and the diff refreshes. No embedded-terminal emulation - it runs on
  the pane's own PTY.
  <img width="1546" height="779" alt="image" src="https://github.com/user-attachments/assets/b6f05932-38b0-470d-9541-c5c24246987f" />

- **Stage, discard, commit inline** - the commit message opens in nvim too;
  discards confirm first. `s`/`u`/`x` work on a directory row too, applying to
  every file under it in that section.
- **Commit history** - `l` opens a `git log` view; pick a commit to browse its
  files and per-commit diffs. `w` filters it to just the commits your branch
  added on top of its base (and opening the log from branch scope starts
  there).
- **Review notes to any agent** - select diff lines, annotate, and send the
  batch into the input of any agent pane you pick in the workspace. It types;
  you decide when to press enter. The note editor is a wrapping text area:
  `shift+enter` (or `ctrl+j`) for a new line, arrows/`home`/`end` to move
  around, `ctrl+w` to drop a word.
  <img width="1539" height="778" alt="image" src="https://github.com/user-attachments/assets/23ee0639-4a6e-42c5-a003-6e71ab619c43" />
  <img width="1544" height="777" alt="image" src="https://github.com/user-attachments/assets/9b615b5b-3202-4ad4-8967-3bab03050e6c" />

- **Branch vs. worktree scope** - `w` toggles between your working-tree
  changes and everything on your branch (diff against the merge base).
- **Mouse support** - click to select, double-click to open, drag-select diff
  lines, wheel to scroll, click folds to expand.

## Requirements

- **herdr ≥ 0.7.0** (≥ **0.7.4** for the native floating dialogs; older
  versions fall back to in-pane overlays).
- **git** on `PATH`.
- **nvim** for the editor loop (any editor works via config; the
  remote-control niceties - file switching, save/discard prompts - are
  nvim-only).
- A truecolor terminal. Pick the `theme` matching its background
  ([Theme](#theme)).
- macOS or Linux.

## Install

Prebuilt binaries, no Rust toolchain needed:

```bash
herdr plugin install ChmaraX/herdr-gitview
```

Bind a key in `~/.config/herdr/config.toml` (`cmd+g` is free of herdr's
defaults; any key works):

```toml
[[keys.command]]
key = "cmd+g"
type = "plugin_action"
command = "chmarax.gitview.toggle"   # <plugin_id>.<action_id>
description = "git view"
```

The shortcut is a smart toggle: it opens the view for the repo you're in,
jumps to it from any other tab, and closes it when pressed inside.

**To update**, reinstall - your config survives:

```bash
herdr plugin uninstall chmarax.gitview && herdr plugin install ChmaraX/herdr-gitview
```

**Without herdr**, the file list runs as a plain terminal app in any repo -
browse, stage, unstage, discard:

```bash
herdr-gitview list
```

Editing, commits, history diffs, and notes need the second pane, i.e. herdr.

## Quick start

1. **Open it.** `cmd+g` in any repo. Changed files on the right, the
   selected file's diff on the left.
2. **Browse.** `j`/`k` (or the wheel) - the diff follows your cursor.
   `Tab`-free: selecting a file under *staged changes* shows its staged diff,
   under *changes* the unstaged one.
3. **Edit.** `Enter` - nvim opens in the diff pane at the first changed line.
   `:wq`, and you're back on the refreshed diff, focus on the list.
4. **Stage & commit.** `s` to stage (the file moves up), `x` to discard
   (asks first), `c` to commit - write the message in nvim, `:wq` commits,
   `:q!` aborts. On a directory row the same keys apply to the whole folder.
5. **See what your branch did.** `w` switches the file list to "vs the base
   branch"; `l` then opens the log already filtered to your branch's commits
   (`w` toggles that filter in the log view too).
6. **Review for your agent.** Focus the diff pane, `v` + `j`/`k` (or drag) to
   select lines, `a` to annotate. Notes pile up as cards under the code.
   `p` → pick an agent → the batch lands in its input.

The footer in each pane shows only the keys that currently work, so you learn
it by using it.

## Configuration

`$HERDR_PLUGIN_CONFIG_DIR/config.toml`, usually
`~/.config/herdr/plugins/config/chmarax.gitview/config.toml`. Every key is
optional; [assets/example-config.toml](assets/example-config.toml) shows all
defaults, commented.

| Key | Default | Meaning |
| --- | --- | --- |
| `theme` | `"dark"` | `"dark"` or `"light"` - syntax theme + all UI tints |
| `editor` | `["nvim"]` | Editor argv; the file (and `+<line>`) is appended |
| `base` | `""` | Branch-scope base ref; `""` auto-detects `origin/HEAD` → `origin/main` → `origin/master` → `main` → `master` |
| `list_side` | `"right"` | `"right"` or `"left"` |
| `default_scope` | `"worktree"` | `"worktree"` or `"branch"` - which scope the view opens in |
| `context_lines` | `3` | Unchanged lines kept around each change before folding (0–20) |
| `poll_ms` | `2000` | Auto-refresh interval in ms; `0` disables, non-zero floored at 250 |
| `show_untracked` | `true` | Include untracked files |

### Theme

`theme = "dark"` pairs a dark syntax palette with dark red/green diff tints;
`"light"` is GitHub-web-flavored. Match your terminal's background - diff
tints are painted as real background colors.

### Keybindings

`[keybindings]` maps action names to keys. An override replaces all default
keys for that action; binding a key another action owns is reported at
startup and the table is ignored until fixed.

```toml
[keybindings]
stage = "space"
discard = "ctrl+x"
```

Grammar: `[ctrl+][alt+][shift+]<key>` where `<key>` is a single character or
`enter`, `esc`, `tab`, `space`, `up`, `down`, `left`, `right`, `pgup`,
`pgdn`, `home`, `end`. Action names: `down up top bottom edit stage unstage
discard commit log annotate select send_notes notes_view delete toggle_scope
refresh help quit half_page_down half_page_up diff_top diff_bottom
scroll_down scroll_up`.

## Limitations

- **Editing needs herdr** - standalone `herdr-gitview list` covers browsing
  and staging only.
- **Remote editor tricks need nvim** (`--listen`/`--server`). Other editors
  work for plain editing; you close them yourself.
- **Notes live in memory** - closing the view discards unsent notes.
- **Notes attach to working-tree changes** - you can't annotate historical
  commits.
- Native floating dialogs need herdr ≥ 0.7.4; older versions get in-pane
  overlays.

## Building from source

```bash
git clone https://github.com/ChmaraX/herdr-gitview
cd herdr-gitview
cargo build --release
herdr plugin link "$PWD"
```

`cargo test` runs the full suite - parser units, fixture git repos, and
ratatui render tests. `just release-dry` mirrors CI (fmt, clippy, tests,
release build). `GITVIEW_DEBUG=1` writes a debug log to the plugin state dir.

## Attribution

The structured diff renderer and highlighting approach are ported, in
simplified form, from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr)
(MIT).

## License

[MIT](LICENSE)
