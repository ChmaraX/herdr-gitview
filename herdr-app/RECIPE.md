# gitview

gitview takes the place of Herdr's Changes panel. Press **Changes** in the tab
bar and the right panel shows the shown space's repo the way gitview does:
changed files grouped into merge conflicts, staged changes and changes, and
under them the selected file's diff with syntax colors, red/green line tints
in the app's theme and word-level emphasis. Enter opens the file in nvim at
the changed line (in herdr-nvim's nvim when it's installed, else a plain nvim
in a pane). `s`/`u` stage and unstage, `x` discards after the app's confirm
dialog, the commit box commits, `w` shows everything the branch changed.
Select lines, write a note, send the notes to any agent: placed in its input,
or submitted. `cmd-shift-g` opens the same view as a full-height sidebar,
`cmd-g` in a tab of its own; `cmd-alt-shift-g` / `cmd-alt-g` open the
original gitview TUI (with its commit history view) as terminal panes.

![gitview](preview.png)

## What you see

- **The panel** (the main layout's `changes` block, right column, 368 px by
  default, resizable to 900), top to bottom: a 38 px header (`uncommitted`
  or `vs <base>` in semibold, the branch in `muted`, an `N notes` pill on
  `accent`, a bordered `N files` pill, a braille spinner in `dim` while
  loading); the file list (24 px rows, at most 260 px tall); the diff; the
  note composer while writing a note; the commit row; a 24 px footer with
  the keys that work now in `dim` (a message in `blocked`).
- **As a pane or tab** 720 px or wider, the list (32 %, at least 260 px) sits
  left of the diff instead of above it.
- **File list**: section headers in 10.5 px uppercase `muted` with their count;
  rows with a status letter (M `blocked`, A/U `ansi2`, D `ansi1`, R `ansi4`,
  ! `ansi1`), the folder in `dim`, the name in `foreground`, `+adds` `ansi2`,
  `−dels` `ansi1`. Selected: `selected` (`hover` while unfocused).
- **Diff**: 18 px rows in the terminal font as gitview's TUI renders them (a
  `▌` marker and line number in `dim`, syntax colors from gitview's dark or
  light set by the theme's background, folds `▸ ⋯ N unchanged lines` on
  `raised`). Tints follow the theme: insert `ansi2` at 15 %, delete `ansi1`
  at 15 %, changed words at 36 %. Selected lines are `selection`; the cursor
  line gets a 2 px `accent` left border while the diff has the keyboard.
  Notes are cards under their lines: `accent` border on `raised`,
  `note · lines 2-3` in `accent`, the text in `foreground`.
- **Commit row**: the stock Changes panel's: a 30 px input (border `accent`
  while typing) and a button, `solid` when there's a message: "Commit staged",
  or "Commit all" when nothing is staged.
- **Send notes**: a 460 px modal dialog on `popover`, "Send N notes to…", one
  30 px row per agent (status dot in its status color, name, status, space /
  tab, address), this space's first.
- **Confirm**: the app's own `confirm` dialog ("Discard changes?").
- **The TUI** (`cmd-alt-…`): gitview's two terminal panes ("gitview: diff",
  "gitview: files"); its dialogs open as app dialogs holding a terminal under
  a 28 px title bar.

## How it behaves

- The panel follows the shown space (the focused pane's folder, else the
  space's; another folder in the same repo keeps the selection). A pane or
  tab copy shows the repo it was opened for (`props.cwd`). It refreshes when
  anything in the repo changes (`watchFolder`) and every 10 s; a theme
  switch between light and dark reloads the diff's syntax colors.
- Keys (after clicking it): `j`/`k` or arrows move, `tab` switches list ↔
  diff, `enter` edits (on a fold: unfolds), `s` stage, `u` unstage, `x`
  discard (confirm first), `c` goes to the commit message (enter commits,
  esc leaves it), `w` worktree ↔ branch, `r` refresh, `p` send notes; in the
  diff `v` starts/ends a selection, `a` writes a note (enter saves, esc
  cancels), `esc` clears the selection, then goes back to the list. Chords
  with cmd/ctrl/alt are left to the app.
- Mouse: click a file, a diff line (cursor), a fold (unfold), the Commit
  button.
- Enter: `herdr-nvim open --tab <tab> <file>:<line>` when
  `<app home>/mods/herdr-nvim/bin/herdr-nvim` exists (herdr-nvim opens the
  file in that tab's nvim and focuses it); else an nvim already running in
  the tab or in a layout block takes the file; else `terminal.open` starts
  the configured editor (`nvim +N file`) right of the focused pane.
- Send: enter in the dialog places the notes in the agent's input (the app's
  paste into that terminal; the clipboard is restored); shift/ctrl/cmd+enter
  submits them (`hc send`). The notes that went out leave the panel.
- `cmd-shift-g` in a tab without gitview splits the tab's root (full height,
  right, `view_width_percent`) with the gitview mod; again: closes it.
  `cmd-g` opens a tab named gitview holding the mod, jumps to it from another
  tab, and from inside closes it and goes back to the tab you came from.
- Commands, for keys.json and scripts (they reach every gitview on screen and
  wait for the previous one's loading): `gitview.key:<key>`,
  `gitview.file:<path>`, `gitview.note:<text>`, `gitview.commit:<message>`;
  the send dialog takes `gitview-agents.key:<keystroke>`.
- A space on another machine shows "gitview runs on this computer".

## Built from

- **Layout**: only the `changes` block of the shown layout changes, to
  `{mod: "gitview", props: {replaces: <old content>}}`; the stock tab bar's
  Changes button (`layout.select {stack: "panel-body", child: "changes"}` +
  `layout.show {id: "panel"}`) then shows gitview. Shipped layouts refuse
  content changes, so `layout_slot.py` clones a shipped one once as
  `<name>-mods` (or switches to that clone); every write re-reads the file
  and renames a temp file in only if it didn't change meanwhile. Uninstall
  puts `props.replaces` back and deletes a `-mods` clone that matches its
  origin apart from `cloned_from`. herdr-nvim does exactly the same for
  `files`, so both share the clone, in either order.
- **mods/gitview**: `main.js` (state, loading, actions), `keys.js` (keys and
  commands), `listview.js`, `diffview.js` (pure row renderers for
  `v_virtual_list`; clicks via `on_item_click`), `chrome.js` (header,
  composer, commit row, footer), `engine.js` (runs the binary; finds the
  shown space's folder like the stock Changes panel). It uses `exec`,
  `watchFolder`, `openOverlay` (`confirm`, `gitview-agents`), `run`
  (`terminal.open`, `pane.focus`), `nextCommand`, `notify`, `props`,
  `theme()`, `settings()`, `focused()`, `panes()`, `spaces()`, `agents()`,
  `layout()`, `InputState`/`Input`, `TextareaState`/`Textarea`, `.frames`.
- **mods/gitview-agents**: the send dialog (`agents()`, then the binary's
  `send`, then `run("gitview.sent:<note ids>")`).
- **mods/gitview-sidebar**, **mods/gitview-tab**: key launchers (2×2 px
  frameless windows that act and close).
- **mods/gitview-tui-sidebar**, **mods/gitview-tui-tab**, **mods/gitview-popup**:
  the TUI's launchers and its dialog host (the newest live terminal labeled
  `gitview.popup`, sized by its `gitview.cells` label, in a modal dialog).
- **The binary** (`mods/gitview/bin/herdr-gitview`, built from this repo;
  paths in `mods/gitview/config.json`), with `GITVIEW_HOST=app`:
  `json status|diff|stage|unstage|discard|commit|notes|nvim|send|sidebar`
  for the mods (git status grouping, the TUI's diff rows with tints as
  roles `@ins`/`@del`/…, branch-base detection, the notes message, nvim
  reuse and the herdr-nvim call); `toggle`, `toggle-tab`, `list`, `preview`,
  `ask`, `pick-agent` for the TUI (window changes through `herdr-app do`,
  terminals through `hc` and `hc connect`).
- Files shared between mod folders (`engine.js`, `launch.js`) are symlinks
  in the repo; install copies them (`cp -L`): a mod can't import from
  another mod's folder.
- **Keys** (`keys.json`): `cmd-shift-g` gitview-sidebar, `cmd-g` gitview-tab,
  `cmd-alt-shift-g` / `cmd-alt-g` the TUI. Linux: `keys.linux.json`.
- It runs programs (`herdr-gitview`, `git`, `nvim`, `hc`, `herdr-app`,
  `pbcopy`/`pbpaste`, herdr-nvim's CLI) and writes to the repo only when you
  stage, discard or commit. No network.

## Watch out

- A layout terminal block can't host a TUI in the panel: a named terminal
  attaches only when the block first draws, and when it ends the app deletes
  the block from the layout file.
- `hc` has no "type without enter": placing notes goes through the app's
  paste and the clipboard (macOS `pbcopy`); submit works everywhere.
- `herdr-app do <mod>.<command>` only reaches copies on screen.
- Rows inside a virtual list can't have handlers; capture the rows array per
  frame (loads replace it).
- `exec` has a 30 s limit and no stdin: notes go to the binary as an argument.
- A 3,000-line diff takes the engine about 0.5 s (1.8 MB of rows): the
  spinner shows meanwhile.
- The stock tab bar remembers the panel per space and restores it when it
  draws; scripts that open the panel in a hidden window must let it draw
  first.
- New terminals in a window that isn't visible keep 120×40 until it is.

## Done when

- `herdr-app check` passes for all seven mods.
- The Changes button shows gitview for the shown space; another space shows
  its own repo.
- `s` stages (git agrees); `x` then Confirm removes an untracked file; the
  commit box commits only the staged changes.
- Enter opens nvim at the first changed line; on another file it opens it in
  that same nvim; with herdr-nvim installed it calls
  `herdr-nvim open --tab <tab> <file>:<line>` and starts no nvim itself.
- A note sent with enter appears in the agent's input, not submitted;
  shift+enter submits it.
- `cmd-shift-g` opens and closes the sidebar; `cmd-g` opens the tab, jumps to
  it, closes it and goes back.
- Uninstall brings back the stock Changes panel; with herdr-nvim installed
  too, either can go first and the last one removes the `main-mods` clone.
- `herdr-app/e2e/run.sh` and `herdr-app/e2e/coexist.sh` pass.
