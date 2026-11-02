# gitview

gitview takes the place of Herdr's Changes panel. Press **Changes** in the
tab bar (or `cmd-shift-g`) and the right panel lists the shown space's
changed files the way gitview does: merge conflicts, staged changes and
changes, flat or as a folder tree, with stage `+`, unstage `−` and discard
`↺` buttons on the row under the pointer, on folders and on whole sections.
Click a file and its diff opens in a full-height pane just left of the
panel, drawn by gitview's own engine (syntax colors, red/green tints in the
app's theme, word emphasis, folded context). Enter opens nvim at the line
(herdr-nvim's nvim when it's installed). Drag over lines and press `a` to
write a review note; `p` sends the notes to any agent, placed in its input
or submitted, without the clipboard. `l` shows the history and each
commit's files, `n` the notes. `cmd-g` opens the same in a tab of its own.
Spaces on other machines work the same way.

![gitview](preview.png)

## What you see

- **The panel** (the layout's `changes` block in the right column; 300 px
  when herdr-nvim is installed, resizable 280–900 px), drawn like the stock
  Changes panel: `background`, 1 px `border` (`accent` while it has the
  keyboard), 10 px corners, 13 px text. Top to bottom:
  - a 42 px header: `uncommitted` (or `vs <base>`) in semibold, a 12 px
    branch icon and the branch in 12 px `muted` (it gives way first), an
    `N notes` pill (`accent` on `accent_foreground`, 10.5 px, 5 px corners;
    a click opens the notes), a bordered `N files` pill, a braille spinner in
    `dim` while loading; from 340 px wide two 22 px icon buttons: tree/flat
    and history;
  - the list, 6 px padding above and below:
    - section headers, 28 px: the title in 11 px semibold uppercase `muted`
      and a count chip (`raised`, 8 px corners);
    - file rows, 28 px, 6 px from the sides, 7 px corners: the status letter
      in a 14 px column (11 px semibold: M `blocked`, A/U `ansi2`, D `ansi1`,
      R `ansi4`, ! `ansi1`), the folder in `dim` and the name in
      `foreground` (the folder shrinks first, then the name, both with "…"),
      `+adds` `ansi2` and `−dels` `ansi1` in 11 px. Hover: `hover`; selected:
      `selected` (`hover` while the panel doesn't have the keyboard);
    - on the hovered or selected row the counts give way to 22 px icon
      buttons (14 px Lucide icons in `muted`, `foreground` on `raised` under
      the pointer, with tooltips): changes `+` `↺`, conflicts `+`, staged `−`;
      a hovered section header shows the same for all its files;
    - tree mode: folder rows (12 px chevron in `dim`, 14 px folder icon in
      `muted`, the name, the file count in `dim`), a folder holding just one
      folder folded into it (`src/app`), 16 px indent per level;
    - a 6 px scroll thumb (`border`, `input` under the pointer) when the list
      is longer than the panel;
  - the commit row (changes view, worktree scope): a 32 px input with 8 px
    corners (border `accent` while typing) and a 32 px button, `solid` when
    there's a message: "Commit staged", or "Commit all" when nothing is
    staged ("Committing…" meanwhile);
  - a 26 px footer: the keys that work in this view, 11 px `dim`; a message
    in `blocked` instead for 5 s.
- **History** (`l`): a header with ← and "History", the branch (or `since
  <base>`) and an `all` / `branch` pill; 44 px rows: the subject and up to
  two refs (`accent` on `raised`), under it `<short sha> · <when> ·
  <author>` in 11 px `dim`. **A commit's files**: ← `<short sha>`, the
  subject, `N files`; its files as in the list (flat or tree), no buttons.
- **Notes** (`n`): ← "Notes", the repo, a count and a send button; 44 px
  rows: `folder/name:lines` (lines in `accent`) over the note's first line
  in `muted`, a × on the hovered or selected row.
- **The diff pane** (a mod pane, full height at the tab's right edge, just
  left of the panel, `view_width_percent` (50 %) of the tab; opened on the
  first file click): a 42 px header (the status letter, the folder in `dim`
  and the name in semibold, `+a −d`, a bordered scope tag: `staged`,
  `uncommitted`, `vs <base>` or `commit <sha>`, the spinner, a 22 px ×);
  the diff in 18 px rows in the terminal font at 12 px exactly as gitview's
  TUI draws it (a `▌` marker and line numbers in `dim`, syntax colors from
  gitview's dark or light set by the theme's background, folds `▸ ⋯ N
  unchanged lines` on `raised`), tints in the theme: inserts `ansi2` at 15 %,
  deletes `ansi1` at 15 %, changed words at 36 %. Selected lines are
  `selection`; the cursor line has a 2 px left border (`accent` while the
  pane has the keyboard, else `dim`). Note cards sit under their last line:
  44 px in, 8 px corners, `accent` border on `raised`, `note · lines 4-5` in
  11 px `accent` with an 18 px ×, the text in 12 px. The composer opens under
  the selection: a 104 px card on `surface` with "new note · lines a-b · enter
  saves · esc cancels" and a 52 px text area. A 26 px footer with the keys.
  With nothing selected: "No file selected / Click a file in the gitview
  list." centered in `dim`.
- **cmd-g's tab** "gitview": the diff pane on the left, the list on the right
  (29 %), both framed by the app like any pane.
- **Send notes**: a 460 px modal dialog on `popover`, "Send N notes to…",
  one 30 px row per agent (status dot, name, status, space / tab, address),
  this space's first.
- **Confirm**: the app's own `confirm` dialog ("Discard changes?").

## How it behaves

- The panel follows the shown space: the focused terminal's folder, else the
  space's (focusing gitview's own diff pane keeps the repo). cmd-g's list
  keeps the repo it was opened for. Local repos refresh on a folder watch
  and every 10 s; a space on another machine every 4 s while the panel shows.
- **List keys** (once it has the keyboard: a click anywhere in it, `pane.go`,
  cmd-shift-g): `j`/`k` or arrows move and the diff follows; `enter` or a
  double-click opens nvim (on a folder: folds it); `h`/`←` folds or goes to
  the parent folder, `→` unfolds; `s` stage, `u` unstage, `x` discard
  (confirm first) the row, folder or file; `t` flat ↔ tree; `w` uncommitted
  ↔ vs base; `l` history; `n` notes; `p` send notes; `c` the commit message
  (enter commits, esc leaves it); `g`/`G` first/last; `r` refresh. Chords
  with cmd, ctrl or alt are left to the app.
- **History**: `j`/`k`, `enter` (or a click) lists the commit's files and
  shows the first one's diff; `w` switches between the branch's own commits
  (`merge-base..HEAD`, the start in `vs base`) and all; `esc`/`h` go back up.
- **Notes**: `enter` (or a double-click) shows the note's file in the diff
  pane with the cursor on the note; `d` deletes; `p` sends; `esc`/`n` back.
- **Diff pane keys**: `j`/`k`, arrows, page up/down, `g`/`G`, the wheel and a
  click move the cursor; `v` starts or ends a selection, a mouse drag selects
  (shift-click extends); `a` writes a note on the selection (enter saves, esc
  cancels); `enter` or a double-click opens nvim at the cursor's line (on a
  fold: unfolds it, as a click does); `s`/`u`/`x` stage, unstage or discard
  the shown file, which then shows where it went (or the next file); `J`/`K`
  the next/previous file in the list's order (the list follows); `p` sends;
  `esc` clears the selection; × closes the pane (the next click opens it).
- **Enter → nvim**: with herdr-nvim installed (`<app home>/mods/herdr-nvim/
  bin/herdr-nvim`), `herdr-nvim open --tab <tab> --space <space>
  <file>:<line>`; its full-height nvim pane takes the diff pane's place, so
  gitview closes its diff pane (not in cmd-g's tab). herdr-nvim's sync
  controller leaves the panel on Changes. Without it: an nvim already
  running in the tab (or a layout block) opens the file; else a new nvim
  (the configured editor) starts right of the diff pane, in the repo, and
  closes with `:q`. On another machine it starts there.
- **Send**: enter in the dialog places the notes in the agent's input (a
  paste through the agent's terminal, bracketed when it asks; no clipboard),
  shift/ctrl/cmd+enter submits them. Sent notes leave the list; notes are
  kept per machine and repo until sent or deleted, across restarts.
- **cmd-shift-g** shows the panel on gitview (and gives it the keyboard), or
  hides it when it already shows gitview. **cmd-g** opens the gitview tab for
  the repo you're in, jumps to it from another tab, and from inside closes it
  and goes back. **cmd-alt-shift-g** / **cmd-alt-g** open gitview's TUI as
  terminal panes (the fallback).
- Commands, for keys.json and scripts (they reach every copy on screen):
  `gitview.key:<key>`, `gitview.file:<path>`, `gitview.commit:<message>`,
  `gitview.click:<path>`, `gitview.dblclick:<path>`,
  `gitview.button:<stage|unstage|discard>:<path | folder/ | section:<id>>`;
  `gitview-diff.key:<key>`, `gitview-diff.note:<text>` (a note on the
  selection), `gitview-diff.drag:<line>:<line>`, `gitview-diff.close`;
  `gitview-agents.key:<keystroke>`.

## Built from

- **Layout**: only the `changes` block of the person's personal layout
  changes, to `{mod: "gitview", props: {replaces: <old content>}}`; the tab
  bar's Changes button (`layout.select {stack: "panel-body", child:
  "changes"}` + `layout.show {id: "panel"}`) then shows gitview.
  `herdr-gitview app-setup install` (run by `install.sh`) picks the layout
  the way the app's guide says: a personal layout is edited in place; on a
  shipped one, the person's copy (`<name>-copy`, else the only layout cloned
  from it) is used and shown, else `layout.clone` makes one. Every write
  re-reads the file and renames a temp file in only if it's unchanged. It
  adds keys only where they're free. Uninstall (`app-setup uninstall`) puts
  `props.replaces` back in every personal layout and removes only our keys;
  it never deletes or switches a layout. herdr-nvim does the same for
  `files`, in either order.
- **mods/gitview** (the list): `main.js` (state, loading, actions),
  `keys.js` (keys and commands), `listview.js` (rows and icon buttons),
  `history.js` (commit rows), `notesview.js` (note rows), `chrome.js`
  (headers, commit row, footer), `model.js` (status → flat or tree rows, the
  view for a file, `diff` arguments), `shared.js` (the shared state, the
  panel in the layout, taking the keyboard), `engine.js` (runs the binary;
  finds the shown space's folder and machine), `nvim.js` (Enter),
  `scroller.js` (a list that draws only its visible rows as ordinary
  elements), `icons/` (Lucide: plus, minus, undo, x, chevrons, folder,
  history, list, list-tree, send, arrow-left, branch). `mod.json`:
  `{"place": "none", "title": "gitview", "focusable": true}`.
- **mods/gitview-diff** (the diff pane): `main.js`, `diffview.js` (diff
  rows, note cards, the composer, a note's range), `parts.js` (header,
  footer); props `{tab}` (`{space, own}` in cmd-g's tab). `focusable`.
- **The two talk only through shared state**: the list writes
  `gitview.views[<tab>]` (the file, its section and scope, the base or
  commit, the list's file order for J/K, a `seq`); the pane writes
  `gitview.cursor[<tab>]` (the list's Enter uses its line),
  `gitview.views[<tab>]` on J/K and `gitview.dirty` after s/u/x; notes are
  `gitview.notes["<machine or ''>:<repo root>"]`; `gitview.tree` the list
  mode; `gitview.own[<space>]` cmd-g's tab.
- **mods/gitview-agents**: the send dialog (`agents()`, then the binary's
  `send`, then the sent notes leave `gitview.notes`).
- **mods/gitview-sidebar** (cmd-shift-g) and **mods/gitview-tab** (cmd-g):
  2×2 px frameless windows that act and close. **mods/gitview-tui-sidebar**,
  **mods/gitview-tui-tab**, **mods/gitview-popup**: the TUI's launchers and
  its dialog host.
- Files shared between mod folders (`engine.js`, `model.js`, `shared.js`,
  `nvim.js`, `scroller.js`, `listview.js`, `launch.js`) are symlinks in the
  repo; install copies them (`cp -L`): a mod can't import from another
  mod's folder.
- **The binary** (`mods/gitview/bin/herdr-gitview`, built from this repo;
  paths in `mods/gitview/config.json`), with `GITVIEW_HOST=app`: `json
  status|diff|stage|unstage|discard|commit|log|commit-files|notes|nvim|send|
  sidebar` (every repo op takes `--on <machine>` and then runs git and file
  reads there through `hc exec --on`), `app-setup install|uninstall`, and the
  TUI's `toggle`, `toggle-tab`, `list`, `preview`, `ask`, `pick-agent`.
- **herdr** functions: `exec`, `watchFolder`, `shared`/`setShared`,
  `nextCommand`, `openOverlay`/`closeOverlay` (`confirm`, `gitview-agents`),
  `run` (`pane.split`, `pane.close`, `pane.focus`, `terminal.open`,
  `tab.new`, `tab.select`, `tab.close`, `layout.select`, `layout.show`,
  `layout.hide`), `notify`, `props`, `theme()`, `settings()`, `focused()`,
  `panes()`, `spaces()`, `agents()`, `layout()`, `InputState`/`Input`,
  `TextareaState`/`Textarea`, `svg`, `.frames`, `.tooltip`.
- **Keys** (`keys.json`): `cmd-shift-g` gitview-sidebar, `cmd-g` gitview-tab,
  `cmd-alt-shift-g` / `cmd-alt-g` the TUI. Linux (`keys.linux.json`):
  `ctrl-alt-shift-g`, `ctrl-alt-g`. None of herdr-nvim's (`cmd-e`, `cmd-o`,
  `cmd-shift-e`).
- It runs programs (`herdr-gitview`, `git`, `nvim`, `hc`, `herdr-app`,
  herdr-nvim's CLI) and writes to the repo only when you stage, discard or
  commit. No network, no clipboard.

## Watch out

- Rows inside the app's virtual lists can't have handlers (one
  `on_item_click` per list), so row buttons, hover and drag need the mod's
  own scrolling list (`scroller.js`: rows on screen only, the wheel and a
  thumb).
- `terminal.show {terminal, pane}` answers ok for a mod pane but shows
  nothing there, so the diff pane can't become nvim; a new nvim opens right
  of it instead.
- Never start work from `render()` (a task spawned there may be dropped and
  leave a load counted forever): render only notices and wakes a task made
  in `init`. A fully covered window doesn't draw at all, so the mods also
  look at shared state twice a second.
- A method named `close` on the View is the app's cleanup hook: it runs when
  the pane goes.
- `json sidebar` finds a tab's root only in this machine's layout: in a
  remote space the diff pane splits the focused pane instead (and cmd-g's
  list splits the diff pane in half).
- herdr-nvim's `open` checks the file on this machine, so in a remote space
  it fails and a plain nvim starts there.
- `<mod>.<command>` reaches only copies on screen; `exec` has a 30 s limit
  and no stdin (notes go to the binary as an argument).
- A click equal to the layout file's own value isn't kept in
  `layouts/.state`: read the file for the default.
- The stock tab bar remembers the panel per space and restores it when it
  draws.

## Done when

- `herdr-app check` passes for every gitview mod; `herdr-app status` shows
  no errors.
- The Changes button shows gitview's list for the shown space; another
  space shows its own repo; no diff pane opens until a file is clicked.
- A click opens one diff pane left of the panel; j/k and further clicks
  reuse it; J/K in the pane move the list's selection.
- `s`/`u`/`x` and the row, folder and section buttons change exactly what
  git says; discard asks first; the commit row commits only staged changes.
- `t` shows folders; a folder's `+` stages only what's under it.
- `l` lists the commits; enter lists a commit's files and the pane shows
  `commit <sha>`.
- A drag and a note show a card under the lines; the notes list jumps to it
  and deletes it; enter in the send dialog puts the notes in the agent's
  input without submitting and leaves the clipboard as it was; shift+enter
  submits.
- Enter opens nvim at the line (one nvim, reused); with herdr-nvim it calls
  `herdr-nvim open --tab <tab> --space <space> <file>:<line>`, starts no
  nvim and closes the diff pane.
- cmd-shift-g hides and shows the panel; cmd-g opens the tab, jumps to it,
  closes it and goes back.
- A space on another machine lists, diffs and stages there, picks up new
  files within 4 s, and starts nvim there.
- Uninstall brings back the stock Changes panel; with herdr-nvim installed
  too, either can go first and each uninstall leaves the layout and keys
  byte-identical to before its install.
- `herdr-app/e2e/run.sh` and `herdr-app/e2e/coexist.sh` pass.
