# herdr-gitview

[![CI](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml/badge.svg)](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ChmaraX/herdr-gitview)](https://github.com/ChmaraX/herdr-gitview/releases/latest)
[![License](https://img.shields.io/github/license/ChmaraX/herdr-gitview)](LICENSE)

<p align="center">
  <a href="#features">features</a> · <a href="#install">install</a> · <a href="#quick-start">quick start</a> · <a href="#controls">controls</a> · <a href="#review-notes">review notes</a> · <a href="#configuration">configuration</a> · <a href="#limitations">limitations</a> · <a href="CHANGELOG.md">changelog</a>
</p>

A git view for [herdr](https://herdr.dev). Changed files on one side, a
syntax-highlighted diff on the other. Press `Enter` and the diff pane
*becomes* your real nvim - opened at the first changed line. You never leave
the terminal.

Inspired by [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr),
rebuilt around a real editor - edit diffs in place with full nvim + LSP
support, not just view them.

![demo](assets/demo.gif)

## Features

- **Grouped changes** - conflicts, staged, and unstaged changes in separate
  sections. `s` moves a file between staged and unstaged.
- **Readable diffs** - syntax highlighting, word-level emphasis on edited
  lines, and collapsible context folds you expand with a click.
- **Edit in real nvim** - `Enter` turns the diff pane into your actual nvim:
  full LSP, plugins, colors. Edit the file right there at the changed line,
  `:wq`, and the diff refreshes. No embedded-terminal emulation - it runs on
  the pane's own PTY.
- **Stage, discard, commit inline** - the commit message opens in nvim too;
  discards confirm first.
- **Commit history** - `l` opens a `git log` view; pick a commit to browse its
  files and per-commit diffs.
- **Review notes to any agent** - select diff lines, annotate, and send the
  batch into the input of any agent pane you pick in the workspace. It types;
  you decide when to press enter.
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

Bind a key in `~/.config/herdr/config.toml`. Note that herdr's built-in
`goto` uses `prefix+g` by default - move it first or pick another key:

```toml
[keys]
goto = "prefix+k"          # free up prefix+g

[[keys.command]]
key = "prefix+g"
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

1. **Open it.** `prefix+g` in any repo. Changed files on the right, the
   selected file's diff on the left.
2. **Browse.** `j`/`k` (or the wheel) - the diff follows your cursor.
   `Tab`-free: selecting a file under *staged changes* shows its staged diff,
   under *changes* the unstaged one.
3. **Edit.** `Enter` - nvim opens in the diff pane at the first changed line.
   `:wq`, and you're back on the refreshed diff, focus on the list.
4. **Stage & commit.** `s` to stage (the file moves up), `x` to discard
   (asks first), `c` to commit - write the message in nvim, `:wq` commits,
   `:q!` aborts.
5. **Review for your agent.** Focus the diff pane, `v` + `j`/`k` (or drag) to
   select lines, `a` to annotate. Notes pile up as cards under the code.
   `p` → pick an agent → the batch lands in its input.

The footer in each pane shows only the keys that currently work, so you learn
it by using it.

## Controls

Defaults; every action can be rebound ([Keybindings](#keybindings)).

**File list**

| Key | Action |
| --- | --- |
| `j` `k` · `↑` `↓` · wheel | Move (the diff follows) |
| `Enter` · double-click | Open in the editor / show a commit |
| `s` | Stage - or unstage, when the file sits in the staged section |
| `u` | Unstage explicitly |
| `x` | Discard changes, with confirmation |
| `c` | Commit - the message opens in nvim on the diff pane |
| `w` | Worktree ↔ branch scope (diff vs the merge base) |
| `l` | Commit history |
| `a` | Annotate the selected file |
| `n` | Notes view (works from either pane) |
| `p` | Send the batched notes to an agent |
| `r` | Refresh - also reconnects the diff pane if the link dropped |
| `?` | Help |
| `q` · `Esc` | Back - commit files → log → files → closed |

**Diff pane**

| Key | Action |
| --- | --- |
| `j` `k` | Move the cursor line (the view follows) |
| `Ctrl+d` `Ctrl+u` | Half page |
| `g` `G` · `Home` `End` | Top / bottom |
| `v` · drag | Visual line selection |
| `a` | Annotate the selection - or the cursor line without one |
| `p` | Send notes |
| click a `⋯ n unchanged lines` line | Expand the fold |
| `q` · `Esc` | Leave selection, then close the view |

**While nvim is open** - `:q` returns to the diff. In the list: `Enter` on
another file switches nvim to it; moving the cursor closes a clean session
automatically; `q` or `c` close it too, asking save/discard first if there
are unsaved changes.

## Review notes

The workflow this plugin exists for: your agent wrote code, you're reading
the diff, and you want to say *"this part - do it differently"* without
retyping context.

1. Select the lines (`v` or drag) and press `a`. A floating input opens,
   titled with the exact range: `note for src/git.rs:118-119`.
2. The note stays visible as a card under those lines. Add more across as
   many files as you like - the count rides in the footer.
3. `n` shows all pending notes; hovering one jumps its diff into the preview.
   `Enter` edits, `d` deletes.
4. `p` opens the agent picker - every agent pane in the current workspace,
   with its status and tab. `Enter` types the batch into that agent's input
   and leaves it there for you to amend and send. `Shift+Enter` (or
   `Ctrl+Enter`) submits it directly.

Each note is delivered as `path:start-end - your note` followed by the
selected lines as a fenced ```diff block, so the agent sees exactly what you
saw. Notes attach to *current* (staged/unstaged) changes - history is
read-only.

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

## How it works

Two plugin panes in one herdr tab, talking over a unix socket. The **list**
pane owns git state and intent; the **preview** pane owns the diff render
*and the PTY* - which is why `Enter` gives you the real nvim instead of a
re-implementation. Diffs are built in-process from old/new file contents:
structured rows, [syntect] highlighting, git-delta-style word emphasis.
Dialogs are herdr floating popup panes. One state file per repo means several
repos can have views open at once.

[syntect]: https://github.com/trishume/syntect

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
- No hunk-level staging yet ([roadmap](#roadmap)).

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

## Roadmap

- Hunk-level stage / discard
- Jump-to-hunk keys, diff search
- Persisted notes
- More themes

## Attribution

The structured diff renderer and highlighting approach are ported, in
simplified form, from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr)
(MIT).

## License

[MIT](LICENSE)
