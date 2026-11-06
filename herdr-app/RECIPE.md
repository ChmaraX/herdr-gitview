# gitview

Review what your agents changed, stage it and commit it, without leaving
Herdr. gitview takes over the Changes panel: it lists the repo's changed
files (staged, unstaged, conflicts; flat or as a tree), and a click opens
the file's diff in a full-height pane beside it. Stage, unstage and discard
with a key or a button, commit from the panel, write review notes on lines
and send them to an agent, and press Enter to edit the file in nvim.

![gitview](preview.png)

## Adding it (for the app's agent)

Don't copy the folders: gitview needs its binary, and it edits the person's
layout (the Changes panel), which copying doesn't do. With the app open,
run its installer:

```sh
curl -fsSL https://raw.githubusercontent.com/ChmaraX/herdr-gitview/main/install.sh | sh -s -- --app
```

Tell the person first what it does:

- downloads the prebuilt `herdr-gitview` for this machine from the GitHub
  release, checked against its SHA-256 (macOS and Linux);
- runs programs (`git`, `hc`, `herdr-app`, an editor) and writes to the repo
  only when they stage, discard or commit; no network of its own (a space on
  another machine is reached through herdr-core);
- adds the gitview mods, adds `cmd-shift-g` and `cmd-g` where they are free,
  and points the layout's Changes panel at gitview (on one of Herdr's own
  layouts, a copy of it, which it switches to).

It needs git. Running the installer again upgrades; `--uninstall` takes all
of it out and brings back the stock Changes panel.

## What you see

- **The panel** (the layout's `changes` block), drawn like the stock one: a
  42px header with `uncommitted` (or `vs <base>`), the branch in `muted`, an
  `N notes` pill, a bordered `N files` pill with the `+A −D` totals, a
  spinner place that never moves anything, and from 340px wide tree/flat
  and history buttons.
- **The list**: section headers (Merge conflicts, Staged, Changes) with a
  count chip; 28px file rows with the status letter (M `blocked`, A/U
  `ansi2`, D `ansi1`, R `ansi4`), the folder in `dim`, the name, `+adds`
  `ansi2` and `−dels` `ansi1`. Hover `hover`, selected `selected`. On the
  hovered or selected row the counts give way to `+`, `−` and `↺` buttons; a
  hovered section header shows them for all its files. Tree mode adds folder
  rows (chevron, folder icon, file count), a folder holding one folder folded
  into it.
- **The commit row** (uncommitted view): a message box that grows to 8 lines
  and a button: "Commit staged", or "Commit all" with nothing staged.
- **The footer**: the view's most useful keys as keycaps (`[s] stage [u]
  unstage [⏎] nvim …`) and a `?` keycap for the keys card; a message replaces
  them for 5s. The commit row and the footer stay at the panel's bottom,
  also when the list is empty or loading.
- **History** (`l`): commits with subject, refs, short sha, age and author;
  a commit opens its files. **Notes** (`n`): each note's `file:lines` and
  first line.
- **The diff pane**, full height at the tab's right edge, just left of the
  panel: a header (status, path, `+a −d`, a scope tag: `staged`,
  `uncommitted`, `vs <base>` or `commit <sha>`, ×) and the diff in the
  terminal font: line numbers, syntax colors, inserts and deletes tinted
  `ansi2`/`ansi1` at 15% with changed words at 36%, unchanged runs folded
  (`▸ ⋯ N unchanged lines`). The cursor line is a faint full-width band
  with a 2px bar; colors too dim on a tint are lifted to 3.5 contrast.
  Notes show as cards under their lines. A diff longer than 5,000 lines
  ends with "diff cut at 5,000 lines".
- **Preview**, for markdown, images, SVG and HTML: a `Diff | Preview`
  toggle in the header; markdown rendered, an image fitted, a changed image
  before | after side by side.
- **cmd-g's tab** "gitview": the diff on the left, the list on the right.
- **Dialogs**: "Discard changes?" (the app's confirm), "Send N notes to…"
  (the agents, this space's first), the keys card (`?`), and "Save them?"
  when an editor gitview started has unsaved changes.

## How it behaves

- **The panel follows the shown space**: the repo of the focused terminal's
  folder, else the space's. Focusing gitview's diff pane or herdr-nvim's
  nvim keeps the repo. It refreshes on a folder watch; a space on another
  machine is polled every 4s while gitview has the keys, else every 10s.
- **Browsing**: a click, `j`/`k` or the arrows select a file and its diff
  shows at once; the pane opens on the first file clicked and is reused.
  Between files the old diff stays (dimmed after 150ms) until the new one is
  there: the pane never blanks.
- **List keys**: `s` stage, `u` unstage, `x` discard (asks first) the row,
  folder or section; `enter` opens the file in the editor (on a folder:
  folds it); `h`/`←` and `→` fold and unfold; `t` flat or tree; `w`
  uncommitted or vs base; `l` history; `n` notes; `p` send notes; `c` the
  commit message (enter commits, shift-enter a new line, esc leaves it);
  `g`/`G` first and last; `r` refresh; `?` all keys. Chords with cmd, ctrl
  or alt are left to the app.
- **Diff pane keys**: `j`/`k`, page keys, `g`/`G`, the wheel and clicks move
  the cursor; `v` or a drag selects; `a` writes a note on the selection
  (staged or unstaged diffs only); `s`/`u`/`x` act on the shown file; `J`/`K`
  the next or previous file (the list follows); `m` Diff or Preview; `enter`
  the editor at the cursor's line; `esc` clears the selection.
- **Enter → the editor**:
  - with herdr-nvim installed, its nvim opens the file at the line and takes
    the diff pane's place. `:q` brings the diff back on that file with the
    keys on the list; the file's row shows `in nvim`, and a click on it does
    the same while nvim keeps the buffer;
  - moving to another file while nvim is open: herdr-nvim's pane makes way
    for the new diff (the daemon keeps unsaved edits); an nvim gitview
    started quits when clean, and stays with "editor has unsaved changes —
    stays open" when not;
  - without herdr-nvim: an nvim already in the tab, else the editor
    configured (`editor` in config.toml, `$VISUAL`, `$EDITOR`, nvim, vim,
    vi) in a pane right of the diff, else the system's opener; with no editor,
    a toast says what to install.
- **Discard** asks first. From Changes it keeps a partly staged file's staged
  part.
- **Commit** commits what's staged, else everything (`git add -A`); with
  conflicts it refuses and changes nothing. A slow hook past 30s says "still
  running or timed out: check git log" and keeps the message.
- **Notes** are kept per repo until sent or deleted. Send places them in the
  agent's input as a bracketed paste (no clipboard), or submits them with
  shift-enter; a program without bracketed paste gets nothing and the dialog
  says why.
- **cmd-shift-g** shows the panel with the keys on the list, or hides it.
  Hiding it (also by the Changes or Files button) closes gitview's unit in
  the space: the diff panes, the herdr-nvim pane gitview handed a file to,
  and an editor gitview started (a clean one quits; unsaved changes ask Save,
  Discard or Cancel). Showing it again opens the selected file. **cmd-g**
  opens the gitview tab for the repo, jumps to it, and from inside closes it.
- **One focus**: the list takes the keyboard through `pane.go right`, so
  only one thing looks focused.
- **Tracing**: gitview keeps a trace of what it did for bug reports (no file
  contents, messages, notes or diff text). The palette has "gitview: mark
  (something's wrong)" and "gitview: tracing on/off".

## Built from

- **Layout**: only the `changes` block of the person's personal layout
  changes, to `{mod: "gitview", props: {replaces: <old content>}}`; the tab
  bar's Changes button then shows gitview. On one of Herdr's own layouts the
  installer clones it once (`<name>-copy`) and switches to the copy. Keys go
  in only where free. Uninstall puts `replaces` back and removes only its
  keys; it never deletes or switches a layout. herdr-nvim edits `files` the
  same way, in either order.
- **Mods**:
  - `gitview`: the list, in the panel and in cmd-g's tab (`placement.js`
    decides which from the props);
  - `gitview-diff`: the diff pane (`props: {tab}`), its Preview, and an image
    cache in `mods/gitview-diff/.cache` (hidden, the newest 50 files);
  - `gitview-sidebar` (cmd-shift-g) and `gitview-tab` (cmd-g): one-shot
    launchers;
  - `gitview-agents` (the send dialog), `gitview-ask` (Save them?),
    `gitview-keys` (the keys card), `gitview-mark` and `gitview-tracing`
    (the palette commands).

  Files the mods share are symlinks in the repo; the installer copies them.
- **The list is the only writer of what the pane shows**: shared
  `gitview.views[<tab>]` (the file, section, scope, base or commit, its
  `rev`, a `seq` per write and a `showing` per new file). The pane asks the
  list for s/u/x and J/K (`gitview.ask` / `gitview.answer`) and publishes its
  cursor (`gitview.cursor`). Other shared keys: `gitview.notes`,
  `gitview.handoff` (the file handed to herdr-nvim), `gitview.editors` (the
  editors gitview started), `gitview.discards` (pending discards by token),
  `gitview.changed` (a repo changed). Per-tab keys drop tabs that are gone.
- **The binary** `mods/gitview/bin/herdr-gitview`, paths in
  `mods/gitview/config.json`:
  `json status|diff|file|stage|unstage|discard|commit|log|commit-files|notes|nvim|editor|send|sidebar`
  and `app-setup install|uninstall`. Every repo op takes `--on <machine>`
  (git runs there through `hc exec --on`) and `--root`. The contract is the
  module doc of `src/appjson.rs`.
- **Keys** (`keys.json`): `cmd-shift-g` gitview-sidebar, `cmd-g`
  gitview-tab; on Linux (`keys.linux.json`) `ctrl-alt-shift-g`,
  `ctrl-alt-g`.
- **Install**: `install.sh` at the repo root (release bundle), or
  `herdr-app/install.sh` in a clone (builds from source).

## Watch out

- Read `props()` only in `init` and keep it: in a task resumed by a promise,
  `props()` can belong to another copy (null 3–12% of the time), and the
  pane skipped files.
- Never start work from `render()`: render notices a change and wakes a task
  made in `init`.
- Rows in the app's virtual lists can't have their own handlers, so the list
  is the mod's own scroller (`scroller.js`).
- A hidden copy (the panel's list while the panel is hidden, or any copy in
  a covered window) isn't drawn: the list also looks every 300ms.
- herdr-app 0.1.12–0.1.13 issues with workarounds in the mods:
  - an exec in flight can be dropped when a mod pane or dialog closes; the
    loads treat it as lost and run again (`Loads.revive`), and a confirmed
    discard still running after 1.5s counts as lost;
  - with two copies waiting on `watchFolder` for one folder, only the newest
    wakes; the waking list stamps `gitview.changed` and every list follows
    it;
  - a copy not waiting on `nextCommand()` when a command comes never gets
    it; one task only queues commands;
  - every hot reload keeps the old code for the app's life: the installer
    swaps each changed mod in with one rename and leaves unchanged ones
    alone;
  - `pane.go` does nothing in a window that never drew.
- `terminal.show` can't put a terminal in a mod pane, so the editor opens
  beside the diff pane, not in it.
- Swap the diff pane for herdr-nvim's pane in one turn (split off it, then
  close it), or the terminals widen and narrow again.
- herdr-nvim runs in its daemon's folder: don't take its pane's folder as the
  repo.
- `exec` has a 30s limit and no stdin; every call the engine makes to
  herdr-core, herdr-app or nvim has its own deadline.
- macOS is tested end to end; the app version is untested on Linux (the
  release ships a Linux bundle, and CI tests the engine there); Windows
  isn't supported.

## Done when

- `herdr-app check` passes for every gitview mod and `herdr-app status`
  shows no errors.
- The Changes button shows the shown space's changes; a click opens one diff
  pane beside the panel, and j/k, clicks and J/K reuse it, each move showing
  its diff.
- `s`/`u`/`x` and the buttons change exactly what git says; discard asks
  first; the commit row commits the staged files, or everything with none
  staged, and refuses with conflicts.
- History lists commits and a commit's files; notes show as cards and reach
  an agent's input without the clipboard.
- Enter opens the editor at the line; with herdr-nvim, `:q` brings the diff
  back and moving to another file swaps nvim for its diff.
- cmd-shift-g hides the panel with its unit and shows it with the selected
  file; cmd-g opens, jumps to and closes the gitview tab.
- A markdown file opens in Preview; a changed image shows before and after.
- A space on another machine lists, diffs, stages and edits there.
- An agent's write shows in the list within 2s.
- Uninstall brings back the stock Changes panel and leaves keys.json as it
  was.
- `node --test herdr-app/test/*.test.mjs` and the e2e suites
  (`herdr-app/e2e/run.sh`, `coexist.sh`, `follow.sh`, `jank.sh`,
  `reload.sh`, `package.sh`) pass.
