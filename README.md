# herdr-gitview

A [herdr](https://herdr.dev) plugin: one shortcut opens a full-tab git view —
changed files on one side, a live colored diff on the other. `Enter` swaps the
diff for **real nvim/LazyVim** on that file (opened at its first changed
line); quitting nvim brings the diff back. Stage, discard, and commit without
leaving the view — the commit message is written in nvim too.

```
┌────────────────────────────────────┬──────────────────────┐
│ src/git.rs  [worktree]             │ main  working tree   │
│ @@ -12,6 +12,9 @@                  │ ●M src/git.rs  +34 -2│
│ +pub fn stage(&self, …)            │  M src/list/app.rs   │
│ +    self.git(&["add", …])         │  ? notes.md    +12 -0│
│ …                                  │                      │
│                                    │ ↵ edit s stage q quit│
└────────────────────────────────────┴──────────────────────┘
```

## Install

```sh
herdr plugin install adamchmara/herdr-gitview
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

From source: `git clone`, `cargo build --release`,
`herdr plugin link /path/to/herdr-gitview`.

## Keys

| Key | Action |
|---|---|
| `j` / `k`, arrows | move through the file list |
| `g` / `G` | top / bottom |
| `enter` | open the file in your editor (left pane, at first change) |
| `J` / `K` | scroll the diff without leaving the list |
| `ctrl+d` / `ctrl+u` | half-page the diff |
| `home` / `end` | diff top / bottom |
| `w` | worktree ↔ branch scope (diff vs merge-base) |
| `tab` | unstaged ↔ staged diff view |
| `s` | stage / unstage the file |
| `x` | discard changes (with confirmation) |
| `c` | commit — message written in nvim in the left pane |
| `r` | refresh |
| `?` | help |
| `q` / `esc` | close the view |

## Config

`~/.config/herdr/plugins/adamchmara.gitview/config.toml` — all keys optional
(see [assets/example-config.toml](assets/example-config.toml) for the fully
commented version):

| Key | Default | Meaning |
|---|---|---|
| `base` | `""` | branch-scope base ref; `""` auto-detects (origin/HEAD → origin/main → …) |
| `split_ratio` | `0.35` | list pane width fraction (0.15–0.6) |
| `list_side` | `"right"` | `"right"` or `"left"` |
| `editor` | `["nvim"]` | editor argv; file and `+<line>` appended |
| `poll_ms` | `2000` | auto-refresh interval; `0` disables |
| `show_untracked` | `true` | include untracked files |
| `theme` | `"dark"` | diff colors: `"dark"` or `"light"` (syntax theme + background tints) |
| `[keybindings]` | — | `action = "key"` overrides (see example config) |

## How it works

Two plugin panes in one herdr tab, talking over a unix socket: the **list**
pane owns git status and keyboard-driven intent; the **preview** pane owns the
diff render *and the PTY* — so `Enter` and `c` suspend its TUI and run real
nvim / `git commit -e` on that terminal, with full fidelity (colors,
statusline, plugins). No embedded-terminal emulation.

Standalone mode: `herdr-gitview list` works in any repo outside herdr
(browse, stage, discard); editing and commit need the preview pane, i.e.
herdr.

## Development

```sh
cargo test          # unit + fixture-repo + render tests
just release-dry    # fmt + clippy + test + release build
GITVIEW_DEBUG=1     # debug log at ~/.local/state/herdr/plugins/adamchmara.gitview/debug.log
```

Design docs live in [docs/plans/](docs/plans/) (phase specs + binding
interface contracts) and [docs/architecture.md](docs/architecture.md).

## License

MIT
