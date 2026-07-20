# herdr-gitview

A git UI plugin for [herdr](https://herdr.dev). One shortcut opens a
full-tab view: changed files grouped VSCode-style on one side, a
syntax-highlighted diff on the other. `Enter` opens the file in **real
nvim** on the diff pane's PTY — at the first changed line — and quitting
brings the diff back. Stage, discard, and commit without leaving the view;
browse the commit history; annotate lines and send review notes straight
to an AI agent running in your workspace.

```
┌─ gitview: diff ────────────────────────┬─ gitview: files ─────────────┐
│ src/git.rs  [worktree]          12/340 │ main  working tree   5 files │
│  118  118    fn stage(&self) {         │ ▾ STAGED CHANGES  2          │
│       119  +     self.git(&["add"])    │   A  src/notes.rs       +214 │
│  119       -     todo!()               │   M  src/git.rs     +34  −2  │
│ ▸ ⋯ 24 unchanged lines — click to expand ▾ CHANGES  3                 │
│  ▎ 118-119 · use add -A here?          │   M  src/git.rs      +6  −6  │
│                                        │   U  notes.md           +12  │
│ j/k move · v select · a note · q quit  │ ↵ edit · s stage · ? help    │
└────────────────────────────────────────┴──────────────────────────────┘
```

## Features

- **Two-pane git view** in its own tab: grouped file list (conflicts /
  staged / changes) + a structured diff with syntax highlighting,
  red/green line tints, word-level change emphasis, line numbers, and
  click-to-expand context folds.
- **Real editor, real PTY** — `Enter` suspends the diff and runs your
  nvim (full plugins/colors) on that pane; `:q` returns to the diff.
  While editing, `Enter` on another file switches nvim to it remotely.
- **Stage / unstage / discard / commit** — `s` moves files between the
  staged and changes groups, `x` asks first (native floating confirm),
  `c` writes the commit message in nvim.
- **History** — `l` lists recent commits; pick one to browse its files
  and per-commit diffs.
- **Review notes for AI agents** — select diff lines (`v` or mouse drag),
  annotate (`a`), batch notes with inline cards, then `p` sends them —
  `path:12-20 — note` plus the diff snippet — into the input of any agent
  pane in the workspace (enter places, shift/ctrl+enter also submits).
- **Mouse everywhere**: click to select, double-click to open, wheel to
  scroll, drag to select diff lines, click folds to expand.
- Auto-refresh on worktree changes, worktree ↔ branch scope, multiple
  repos concurrently, standalone CLI mode.

## Requirements

- herdr ≥ 0.7.0 (≥ 0.7.4 for native floating dialogs)
- git, and nvim (or any editor; nvim enables the remote-control niceties)

## Install

```sh
herdr plugin install <owner>/herdr-gitview
```

Then bind a key in `~/.config/herdr/config.toml` (herdr's built-in `goto`
uses `prefix+g` by default — move it or pick another key):

```toml
[keys]
goto = "prefix+k"          # free up prefix+g

[[keys.command]]
key = "prefix+g"
type = "plugin_action"
command = "adamchmara.gitview.toggle"
description = "git view"
```

The shortcut toggles: it opens the view for the repo you're in, jumps to
it from any other tab, and closes it when pressed inside.

From source: `git clone`, `cargo build --release`,
`herdr plugin link /path/to/herdr-gitview`.

## Keys

File list:

| Key | Action |
|---|---|
| `j` / `k`, arrows, wheel | move through the list |
| `enter`, double-click | open the file in your editor / show a commit |
| `s` | stage / unstage (section-aware) |
| `u` | explicitly unstage |
| `x` | discard changes (with confirmation) |
| `c` | commit — message written in nvim in the diff pane |
| `w` | worktree ↔ branch scope |
| `l` | commit history |
| `a` | add a review note for the selected file |
| `n` | notes view (from either pane) |
| `p` | send the batched notes to an agent |
| `r` | refresh (also reconnects the preview link) |
| `?` | help |
| `q` / `esc` | back / close |

Diff pane:

| Key | Action |
|---|---|
| `j` / `k` | move the cursor (view follows) |
| `ctrl+d` / `ctrl+u` | half page |
| `g` / `G`, `home`/`end` | top / bottom |
| `v`, mouse drag | visual line selection |
| `a` | annotate the selection (or cursor line) |
| `p` | send notes to an agent |
| click on `⋯ n unchanged lines` | expand the fold |

Every action can be rebound; see the config below.

## Config

`$HERDR_PLUGIN_CONFIG_DIR/config.toml` — all keys optional (see
[assets/example-config.toml](assets/example-config.toml) for the fully
commented version):

| Key | Default | Meaning |
|---|---|---|
| `base` | `""` | branch-scope base ref; `""` auto-detects (origin/HEAD → origin/main → …) |
| `split_ratio` | `0.35` | list pane width fraction (0.15–0.6) |
| `list_side` | `"right"` | `"right"` or `"left"` |
| `editor` | `["nvim"]` | editor argv; file and `+<line>` appended |
| `poll_ms` | `2000` | auto-refresh interval; `0` disables |
| `show_untracked` | `true` | include untracked files |
| `theme` | `"dark"` | diff colors: `"dark"` or `"light"` (syntax theme + tints) |
| `[keybindings]` | — | `action = "key"` overrides |

## How it works

Two plugin panes in one herdr tab, talking over a unix socket: the
**list** pane owns git state and keyboard intent; the **preview** pane
owns the diff render *and the PTY* — so editing and committing run real
nvim on that terminal with full fidelity. No embedded-terminal emulation.
Dialogs (confirm, note input, agent picker) are herdr floating popup
panes. Diffs are built in-process from the old/new file contents:
structured rows, syntect highlighting, and git-delta-style word emphasis.

Standalone mode: `herdr-gitview list` works in any repo outside herdr
(browse, stage, discard); editing, commits, and notes need the preview
pane, i.e. herdr.

## Development

```sh
cargo test          # unit + fixture-repo + render tests
just release-dry    # fmt + clippy + test + release build
GITVIEW_DEBUG=1     # debug log in the plugin state dir
```

## Attribution

The diff renderer and syntax-highlighting approach are ported (simplified)
from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr) (MIT).

## License

MIT
