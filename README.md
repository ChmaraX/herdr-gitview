# herdr-gitview

[![CI](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml/badge.svg)](https://github.com/ChmaraX/herdr-gitview/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ChmaraX/herdr-gitview)](https://github.com/ChmaraX/herdr-gitview/releases/latest)
[![License](https://img.shields.io/github/license/ChmaraX/herdr-gitview)](LICENSE)

<p align="center">
  <a href="#features">features</a> · <a href="#install">install</a> · <a href="#quick-start">quick start</a> · <a href="#keys">keys</a> · <a href="#configuration">configuration</a> · <a href="#limitations">limitations</a> · <a href="CHANGELOG.md">changelog</a>
</p>

A git view for the [Herdr](https://herdr.dev) app. Review what your agent
changed: the changed files in the Changes panel, a syntax-highlighted diff
beside them. Press `Enter` and the diff pane *becomes* your real nvim, at the
line you're on. You never leave Herdr.

> [!TIP]
> Using the **herdr TUI** (terminal)? Its guide is [TUI.md](TUI.md).

<img width="2560" height="1600" alt="image" src="https://github.com/user-attachments/assets/5dd57c9c-fad9-41af-8959-dda3fabc750c" />

## Features

- **Takes over the Changes panel** - the tab bar's **Changes** button (or
  `cmd-shift-g`) shows gitview's file list for the space you're in, and a
  click opens the file's diff in a full-height pane right beside it. Moving
  through the list, the diff follows.
- **Grouped changes** - conflicts, staged, and unstaged changes in separate
  sections, flat or as a folder tree, with `+ added −deleted` counts per file
  and in total.
- **Stage, discard, commit inline** - `s`/`u`/`x` or the `+` `−` `↺` buttons
  on a file, a folder or a whole section; discards confirm first. Write the
  commit message right in the panel (`shift+enter` for more lines) and
  commit what's staged, or everything when nothing is.
- **Readable diffs** - syntax highlighting (dark or light, to match your
  app theme), added and removed lines tinted in the theme's colors,
  word-level emphasis on edited lines, and collapsible context folds.
- **Preview** - markdown renders, images show, and a changed image shows
  before and after side by side. One click switches back to the diff.
- **Edit in real nvim** - `Enter` turns the diff pane into your nvim, at the
  line ([herdr-nvim](https://github.com/ChmaraX/herdr-nvim)'s when it's
  installed). `:q` brings the diff back. Moving to another file swaps nvim
  for that file's diff again; with unsaved changes nvim stays.
- **Commit history** - `l` opens the log; pick a commit to browse its files
  and diffs. `w` filters it to the commits your branch added.
- **Knows what your branch was cut from** - branch scope diffs against the
  branch you actually branched off ([Branch scope](TUI.md#branch-scope)).
- **Review notes to any agent** - select diff lines, write a note under them,
  and send the batch into the input of any agent in Herdr. It types, without
  the clipboard; you decide when to press enter.
- **A tab of its own** - `cmd-g` opens gitview in a tab for the repo you're
  in: the diff on the left, the list on the right.
- **Remote spaces** - in a space on another machine, git runs there.
- **Mouse support** - click, double-click to open, drag-select diff lines,
  hover buttons.

## Requirements

- The **Herdr desktop app** (herdr-app ≥ 0.1.12).
- **git** on `PATH`.
- An editor for `Enter`: [herdr-nvim](https://github.com/ChmaraX/herdr-nvim),
  else nvim, vim, `$VISUAL`/`$EDITOR`, or the system's default app.
- macOS. Linux should work but is untested. Windows: the app version, built
  from source (below); the TUI plugin isn't supported there.

## Install

With the Herdr app open:

```bash
curl -fsSL https://raw.githubusercontent.com/ChmaraX/herdr-gitview/main/install.sh | sh -s -- --app
```

It downloads the prebuilt gitview for your machine, checks it, adds its
mods, adds `cmd-shift-g` and `cmd-g` where they're free, and puts gitview in
the Changes panel. No git clone, no Rust. Or ask the agent in your Herdr
window to "add the mod at github.com/ChmaraX/herdr-gitview".

- **Update:** run the same command again.
- **Uninstall:** the same command with `--uninstall`; the stock Changes
  panel comes back.

### Windows

No release bundle yet: build it with Rust and run the Windows installer
(Python 3) with the Herdr app open:

```powershell
git clone https://github.com/ChmaraX/herdr-gitview
cd herdr-gitview
python -I herdr-app/install-windows.py             # --uninstall to remove
```

Keys: `ctrl-shift-g` (the panel) and `alt-shift-g` (gitview's tab);
Linux's `ctrl-alt-…` keys are AltGr on Windows.

## Quick start

1. **Open it.** Press **Changes** in the tab bar, or `cmd-shift-g`. The
   space's changed files show in the panel.
2. **Browse.** Click a file, or `j`/`k`: its diff opens in a full-height pane
   beside the panel and follows your selection.
3. **Edit.** `Enter` turns the diff pane into nvim at the line. `:q`, and
   you're back on the diff.
4. **Stage & commit.** `s` to stage (the file moves up), `x` to discard (asks
   first), `c` to write the commit message, `enter` to commit.
5. **See what your branch did.** `w` switches to "vs the base branch"; `l`
   opens the log filtered to your branch's commits.
6. **Review for your agent.** In the diff, drag over lines (or `v` +
   `j`/`k`), `a` to write a note, `enter` to save it. `p` → pick an agent →
   the notes land in its input.

The footer shows the keys that work right now; `?` lists them all.

## Keys

| Key | Where | What |
| --- | --- | --- |
| `cmd-shift-g` | anywhere | Show or hide the Changes panel with gitview (Linux: `ctrl-alt-shift-g`) |
| `cmd-g` | anywhere | gitview in a tab of its own; again to go back (Linux: `ctrl-alt-g`) |
| `j` / `k`, arrows | list, diff | Move; in the list the diff follows |
| `enter` | list, diff | Open the file in the editor at the line |
| `s` / `u` / `x` | list, diff | Stage, unstage, discard |
| `c` | list | Write the commit message (`enter` commits, `shift+enter` new line) |
| `t` | list | Flat or tree |
| `w` | list | Uncommitted or vs the base branch |
| `l` / `n` | list | History / notes |
| `J` / `K` | diff | Next / previous file |
| `v`, `a` | diff | Select lines, write a note |
| `p` | list, diff | Send the notes to an agent |
| `m` | diff | Diff or Preview |
| `?` | list, diff | All keys |

## Configuration

gitview reads the same `config.toml` as the TUI plugin,
`~/.config/herdr/plugins/config/chmarax.gitview/config.toml`. In the app
these apply: `editor`, `base`, `context_lines`, `show_untracked` and
`view_width_percent` (the diff pane's width). See
[TUI.md](TUI.md#configuration) for each key. Colors follow the app's theme.

## Limitations

- **nvim is the editor gitview knows** - with herdr-nvim's nvim or your own
  nvim, gitview checks for unsaved changes and closes it for you. Other
  editors open the file too; you close them yourself.
- **Notes attach to working-tree changes** - you can't annotate historical
  commits.
- **herdr-nvim on remote spaces** - in a space on another machine, Enter
  starts a plain editor there instead.

## Development

```bash
git clone https://github.com/ChmaraX/herdr-gitview
cd herdr-gitview
herdr-app/install.sh               # builds from source and installs into the running app
herdr-app/install.sh --uninstall
```

`cargo test` runs the engine's tests and `node --test herdr-app/test/*.test.mjs`
the mods'. The e2e suites in `herdr-app/e2e/` (`run.sh`, `coexist.sh`,
`follow.sh`, `jank.sh`, `reload.sh`, `package.sh`) drive a private app in the
background. How the mods are built: [herdr-app/RECIPE.md](herdr-app/RECIPE.md).

## Attribution

The structured diff renderer and highlighting approach are ported, in
simplified form, from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr)
(MIT).

## License

[MIT](LICENSE)
