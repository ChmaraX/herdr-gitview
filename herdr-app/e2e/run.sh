#!/usr/bin/env bash
# End-to-end run of gitview in a private Herdr app: its own app home, its own
# herdr-core session, a scratch repo, a dummy agent and a fake second machine.
# Nothing touches your own app, core, spaces, terminals, clipboard or
# keyboard focus (see lib.sh).
#
#   herdr-app/e2e/run.sh [artifacts dir]     (default: /tmp/gitview-e2e-<time>)
#
# It installs gitview with herdr-app/install.sh (from this checkout; the
# binary from committed HEAD, or $GITVIEW_BIN), drives every feature the way
# a person does: `herdr-app do` runs the commands the tab bar, keys and
# buttons run, keys.py presses keys in the private window, and the mods'
# script commands (`gitview.click:`, `gitview.button:`, `gitview-diff.drag:`)
# take the same paths as the mouse. It asserts on git, the terminals, the
# app's state and the mods' shared state, saves a screenshot per step plus
# run.log, then uninstalls and shuts the private app and core down.
#
# Needs: the Herdr app (HERDR_APP_BIN, else found like install.sh does), hc,
# git, nvim, python3 with pyobjc (Quartz), cargo (or GITVIEW_BIN).
set -euo pipefail

. "$(dirname "$0")/lib.sh"

S="$BASE/scratch"
mkdir -p "$S/src/app"
git -C "$S" init -q -b main
git -C "$S" config user.email e2e@test && git -C "$S" config user.name e2e
printf 'fn main() {\n    println!("hello");\n    let a = 1;\n    let b = 2;\n}\n' >"$S/src/main.rs"
printf 'a = 1\nb = 2\n' >"$S/config.toml"
printf 'old\nfile\n' >"$S/gone.txt"
printf 'pub fn a() {}\n' >"$S/src/app/a.rs"
printf 'pub fn b() {}\n' >"$S/src/app/b.rs"
git -C "$S" add -A && git -C "$S" commit -qm init
printf 'second\n' >"$S/history.txt" && git -C "$S" add -A && git -C "$S" commit -qm "add history.txt"
printf 'fn main() {\n    println!("hello, gitview");\n    let a = 1;\n    let b = 3;\n    let x = 42;\n}\n' >"$S/src/main.rs"
printf 'a = 1\nb = 3\nc = 4\n' >"$S/config.toml"
printf 'pub fn a() { 1 }\n' >"$S/src/app/a.rs"
printf 'pub fn b() { 2 }\n' >"$S/src/app/b.rs"
git -C "$S" rm -q gone.txt
printf 'brand new\n' >"$S/added.txt" && git -C "$S" add added.txt
printf 'untracked\n' >"$S/notes.md"
staged() { git -C "$S" diff --cached --name-only | tr '\n' ' '; }

MADE=$("$APP" do space.new --name scratch --folder "$S")
TAB=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tab"])')
SPACE=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["space"])')
SHELL_T=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["terminal"])')
do_ pane.split --direction down --argv "[\"/usr/bin/env\",\"DUMMY_LOG=$BASE/dummy.log\",\"python3\",\"-u\",\"$E2E/dummy-agent.py\"]"
expect 'terms | grep -q " dummy "' "dummy agent reports a status"
AGENT=$(terms | awk '$2=="dummy"{print $1}')

step "install (herdr-app/install.sh → herdr-gitview app-setup)"
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh"
ok "install.sh, including herdr-app check of every mod"
expect '[ "$(app_status layout)" = main-copy ]' "the shipped main layout was cloned once as main-copy (personal) and shown"
expect panel_shows "main-copy's Changes slot shows the gitview mod (props.replaces keeps the stock one)"
python3 - "$HERDR_APP_HOME/keys.json" <<'EOF' && ok "keys bound: cmd-g, cmd-shift-g, cmd-alt-g, cmd-alt-shift-g" || fail "keys"
import json, sys
g = json.load(open(sys.argv[1]))["global"]
assert g["cmd-g"] == "mod.open:gitview-tab" and g["cmd-shift-g"] == "mod.open:gitview-sidebar"
assert g["cmd-alt-g"] == "mod.open:gitview-tui-tab" and g["cmd-alt-shift-g"] == "mod.open:gitview-tui-sidebar"
EOF
[ ! -s "$HERDR_APP_HOME/keys.error" ] && [ ! -s "$HERDR_APP_HOME/layout.error" ] && ok "no keys.error / layout.error" || fail "app file errors"
for m in gitview gitview-diff gitview-agents gitview-tab gitview-sidebar; do
  grep -q '"focusable": true' "$HERDR_APP_HOME/mods/$m/mod.json" 2>/dev/null && echo "focusable $m" >>"$LOG"
done

# --- the panel and its diff pane ---------------------------------------------------------
step "Changes button → gitview's file list; a click opens the diff pane left of it"
do_ pane.focus --terminal "$SHELL_T"
changes_button
expect '"$APP" do gitview.key:r' "the panel is up (it takes gitview.* commands)"
expect '[ "$(panel_state)" = "open changes" ]' "the panel shows Changes"
mod_panes | grep -q "gitview-diff" && fail "a diff pane before any click" || ok "no diff pane until a file is clicked"
shot 01-panel-list
do_ gitview.click:src/main.rs
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$"' "clicking a file opens the diff pane in this tab"
expect '[ "$(view_of $TAB path)" = src/main.rs ] && [ "$(view_of $TAB scope)" = uncommitted ]' "the pane shows src/main.rs (uncommitted)"
shot 02-click-opens-diff 1.5
[ "$(mod_panes | grep -c gitview-diff)" = 1 ] || fail "one diff pane"
do_ gitview.key:k
expect '[ "$(view_of $TAB path)" != src/main.rs ]' "k: the diff follows the selection"
SEQ=$(view_of $TAB seq)
do_ gitview.click:config.toml
expect '[ "$(view_of $TAB path)" = config.toml ]' "another click shows config.toml"
[ "$(mod_panes | grep -c gitview-diff)" = 1 ] && ok "the same diff pane is reused" || fail "a second diff pane"
status_has '"errors": \[\]' && ok "no mod errors in status" || fail "mod errors: $("$APP" status)"

step "J/K in the diff pane move through the list (the list follows)"
do_ gitview-diff.key:J
expect '[ "$(view_of $TAB path)" != config.toml ] && [ "$(view_of $TAB by)" = pane ]' "J: the next file, chosen by the pane"
NEXT=$(view_of $TAB path)
do_ gitview-diff.key:K
expect '[ "$(view_of $TAB path)" = config.toml ]' "K: back to config.toml"
do_ gitview-diff.key:J
expect '[ "$(view_of $TAB path)" = "$NEXT" ]' "J again: $NEXT"
do_ gitview.key:s # the list's own s acts on what it now has selected
expect 'staged | grep -q "$NEXT"' "the list followed: s staged $NEXT"
git -C "$S" reset -q -- "$NEXT"
do_ gitview-diff.close # the pane's × button
expect '! mod_panes | grep -q gitview-diff' "× closes the diff pane"
do_ gitview.click:config.toml
expect 'mod_panes | grep -q gitview-diff && [ "$(view_of $TAB path)" = config.toml ]' "the next click opens it again"

step "stage / unstage / discard: keys and buttons, list and pane"
do_ gitview.click:config.toml
do_ gitview.key:s
expect 'git -C "$S" status --short | grep -q "^M  config.toml"' "list s stages the file"
expect '[ "$(view_of $TAB cached)" = true ] && [ "$(view_of $TAB scope)" = staged ]' "the pane follows it into staged changes"
do_ gitview-diff.key:u
expect 'git -C "$S" status --short | grep -q "^ M config.toml"' "pane u unstages it"
do_ gitview-diff.key:s
expect 'git -C "$S" status --short | grep -q "^M  config.toml"' "pane s stages it"
do_ gitview.button:unstage:config.toml
expect 'git -C "$S" status --short | grep -q "^ M config.toml"' "the row's − button unstages it"
do_ gitview.button:stage:config.toml
expect 'git -C "$S" status --short | grep -q "^M  config.toml"' "the row's + button stages it"
shot 03-staged
do_ gitview.button:discard:notes.md
expect 'overlay_open confirm' "↺ asks first (the app's confirm dialog)"
shot 04-confirm
do_ gitview.discard # what the dialog's Confirm button runs
do_ overlay.close
expect '[ ! -e "$S/notes.md" ]' "confirmed discard removes the untracked file"
printf 'scratch\n' >"$S/tmp.txt"
do_ gitview.key:r
do_ gitview.click:tmp.txt
expect '[ "$(view_of $TAB path)" = tmp.txt ]' "tmp.txt in the pane"
do_ gitview-diff.key:x
expect 'overlay_open confirm' "pane x asks first"
do_ gitview-diff.discard
do_ overlay.close
expect '[ ! -e "$S/tmp.txt" ]' "pane x then confirm discards it"
expect '[ "$(view_of $TAB path)" != tmp.txt ]' "and the pane moves on to another file"
do_ gitview.button:unstage:section:staged
expect '[ -z "$(staged)" ]' "the staged section's − unstages all"
do_ gitview.button:stage:section:changes
expect '[ -n "$(staged)" ] && [ -z "$(git -C "$S" diff --name-only)" ]' "the changes section's + stages all"
do_ gitview.button:unstage:section:staged

step "tree mode (t): folders, folder staging"
do_ gitview.key:t
expect '[ "$(shared "s.get(\"gitview.tree\")")" = true ]' "t shows the tree (kept in shared state)"
shot 05-tree 1
do_ gitview.button:stage:src/app/
expect 'staged | grep -q "src/app/a.rs" && staged | grep -q "src/app/b.rs" && ! staged | grep -q "src/main.rs"' "the folder's + stages everything under src/app/ only"
do_ gitview.click:src/app/ # a click folds it and selects it
do_ gitview.key:u
expect '! staged | grep -q "src/app/"' "u on the folder unstages it"
do_ gitview.key:t
expect '[ "$(shared "s.get(\"gitview.tree\")")" = false ]' "t again: flat"

step "commit row"
do_ gitview.click:config.toml
do_ gitview.key:s
do_ "gitview.commit:e2e: commit the staged config"
expect 'git -C "$S" log -1 --format=%s | grep -qx "e2e: commit the staged config"' "the commit row commits the staged changes"
git -C "$S" status --short | grep -q "src/main.rs" && ok "unstaged changes stay out of it" || fail "commit took unstaged changes"
HEAD_SHA=$(git -C "$S" rev-parse HEAD)

step "history (l): commits, a commit's files, its diff"
do_ gitview.key:l
shot 06-history 1
do_ gitview.key:enter
expect '[ "$(view_of $TAB commit)" = "$HEAD_SHA" ] && [ "$(view_of $TAB path)" = config.toml ]' "enter lists the newest commit's files; its file shows in the pane"
expect '[ "$(view_of $TAB scope)" = "commit ${HEAD_SHA:0:7}" ]' "scope tag: commit <sha>"
shot 07-commit-files 1.5
do_ gitview.key:escape
do_ gitview.key:j
do_ gitview.key:enter
expect '[ "$(view_of $TAB path)" = history.txt ]' "the commit before it: history.txt"
do_ gitview.key:escape
do_ gitview.key:escape

step "notes: drag-select, note, the notes list"
do_ gitview.click:src/main.rs
expect '[ "$(view_of $TAB path)" = src/main.rs ]' "src/main.rs in the pane"
sleep 0.5
do_ gitview-diff.drag:4:5
do_ "gitview-diff.note:please name x something meaningful"
KEY=":$(cd "$S" && pwd -P)"
notes_n() { shared "len(s.get('gitview.notes',{}).get('$KEY',[]))"; }
expect '[ "$(notes_n)" = 1 ] && shared "s[\"gitview.notes\"][\"$KEY\"][0][\"start\"], s[\"gitview.notes\"][\"$KEY\"][0][\"end\"]" | grep -q "\[4, 5\]"' "a drag over lines 4-5 and a note: kept in shared gitview.notes for this repo"
shot 08-note-card 1
do_ gitview-diff.key:j
do_ gitview-diff.key:v
do_ gitview-diff.key:j
do_ gitview-diff.key:a
shot 09-composer 1
do_ gitview-diff.key:escape
[ "$(notes_n)" = 1 ] && ok "esc cancels the composer" || fail "esc saved a note"
do_ "gitview-diff.note:and drop this one"
expect '[ "$(notes_n)" = 2 ]' "v j a: a second note"
do_ gitview.key:n
shot 10-notes-list 1
do_ gitview.key:j
do_ gitview.key:d
expect '[ "$(notes_n)" = 1 ]' "d deletes the selected note"
do_ gitview.key:k
do_ gitview.key:enter
expect '[ "$(view_of $TAB path)" = src/main.rs ] && [ "$(view_of $TAB line)" = 5 ]' "enter on a note jumps there (src/main.rs, line 5)"

step "send notes: placed in the agent's input, no clipboard; then submitted"
CLIP0=$(pbpaste 2>/dev/null | shasum)
do_ gitview.key:p
expect 'overlay_open gitview-agents' "p opens the send dialog"
shot 11-send 1
do_ gitview-agents.key:enter
expect 'agent_input | grep -q "src/main.rs:4-5 — please name x"' "enter places the notes in the agent's input"
[ -z "$(agent_submitted)" ] && ok "not submitted" || fail "placing must not submit"
[ "$(pbpaste 2>/dev/null | shasum)" = "$CLIP0" ] && ok "the clipboard is unchanged" || fail "the clipboard changed"
expect '! overlay_open gitview-agents' "the dialog closes"
expect '[ "$(notes_n)" = 0 ]' "the sent notes leave the list"
do_ "gitview-diff.note:and send this one now"
do_ gitview-diff.key:p
expect 'overlay_open gitview-agents' "p in the pane opens it too"
do_ gitview-agents.key:shift-enter
expect 'agent_submitted | grep -q "and send this one now"' "shift+enter submits them" 15
[ "$(pbpaste 2>/dev/null | shasum)" = "$CLIP0" ] && ok "the clipboard is still unchanged" || fail "the clipboard changed"
expect '! overlay_open gitview-agents' "the dialog closes"
shot 12-sent 1

step "enter → nvim (no herdr-nvim): a new nvim right of the diff pane"
do_ gitview.click:src/main.rs
do_ gitview-diff.key:G
do_ gitview-diff.key:k
do_ gitview-diff.key:enter
expect '[ -n "$(terms | awk '"'"'$2=="nvim"'"'"')" ]' "nvim runs in a new pane"
NVIM=$(terms | awk '$2=="nvim"{print $1}')
hc get "$NVIM" --json | grep -q '"+5"' && ok "at the cursor's line (+5)" || fail "nvim line: $(hc get "$NVIM" --json | head -c 300)"
mod_panes | grep -q gitview-diff && ok "the diff pane stays beside it" || fail "the diff pane went"
shot 13-nvim 2
do_ gitview.click:src/app/a.rs
do_ gitview.key:enter
expect 'nvim_buf $NVIM | grep -q "src/app/a.rs$"' "the list's enter on another file goes to that same nvim"
do_ gitview.dblclick:src/app/b.rs
expect 'nvim_buf $NVIM | grep -q "src/app/b.rs$"' "a double-click on a file opens it there too"
[ "$(terms | awk '$2=="nvim"' | wc -l | tr -d ' ')" = 1 ] && ok "no second nvim" || fail "second nvim"
hc keys "$NVIM" esc shift+z shift+q >/dev/null
expect '[ -z "$(terms | awk '"'"'$2=="nvim"'"'"')" ]' "ZQ closes nvim and its pane; the diff is back"

step "enter → herdr-nvim (its CLI, as a stand-in): it takes the diff pane's place"
STAND="$HERDR_APP_HOME/mods/herdr-nvim"
mkdir -p "$STAND/bin" && echo '{ "place": "none" }' >"$STAND/mod.json"
printf 'import { View, div } from "gpui-kit";\nexport default class S extends View { render() { return div(); } }\n' >"$STAND/main.js"
printf '#!/bin/sh\necho "$@" >>"%s/herdr-nvim.log"\necho "{\\"terminal\\":\\"t99\\"}"\n' "$BASE" >"$STAND/bin/herdr-nvim" && chmod +x "$STAND/bin/herdr-nvim"
do_ gitview.click:src/main.rs
do_ gitview-diff.key:enter
expect 'grep -q "^open --tab $TAB --space $SPACE .*/src/main.rs:2$" "$BASE/herdr-nvim.log"' "enter asks herdr-nvim: open --tab <tab> --space <space> <file>:<line>"
[ -z "$(terms | awk '$2=="nvim"')" ] && ok "and starts no nvim of its own" || fail "started nvim anyway"
expect '! mod_panes | grep -q gitview-diff' "the diff pane closes (herdr-nvim's nvim sits there)"
do_ gitview.click:src/app/a.rs
expect 'mod_panes | grep -q gitview-diff' "the next file click opens the diff pane again"
rm -rf "$STAND"

step "focus: pane.go reaches the diff pane, a click takes the keyboard (keys.py, private window only)"
do_ pane.focus --terminal "$SHELL_T"
do_ pane.go --direction right
sleep 0.6
L0=$(shared "s['gitview.cursor']['$TAB']['row']")
keys 38 # j
expect '[ "$(shared "s[\"gitview.cursor\"][\"$TAB\"][\"row\"]")" != "$L0" ]' "pane.go then j: the diff pane has the keyboard (the cursor moved)"

step "cmd-shift-g toggles the Changes panel; cmd-g opens gitview's own tab"
keys 5:cmd,shift
expect '[ "$(panel_state)" = "hidden changes" ]' "cmd-shift-g hides the panel"
keys 5:cmd,shift
expect '[ "$(panel_state)" = "open changes" ]' "cmd-shift-g shows it, on gitview"
sleep 0.5
P0=$(view_of $TAB path)
keys 38 # j
expect '[ "$(view_of $TAB path)" != "$P0" ]' "and the list has the keyboard: j moves the diff"
shot 14-cmd-shift-g 1
keys 5:cmd,shift
expect '[ "$(panel_state)" = "hidden changes" ]' "hidden again"
keys 5:cmd
expect '[ "$(tab_active)" = gitview ]' "cmd-g opens the gitview tab"
OWN=$(tab_id)
expect 'mod_panes | grep -q "^$OWN .* gitview-diff$" && mod_panes | grep -q "^$OWN .* gitview$"' "diff pane and list pane"
do_ gitview.click:src/main.rs
expect '[ "$(view_of $OWN path)" = src/main.rs ]' "a click in its list shows the diff on the left"
shot 15-own-tab 1.5
do_ tab.select --index 1
keys 5:cmd
expect '[ "$(tab_active)" = gitview ]' "from another tab cmd-g jumps to it"
keys 5:cmd
expect '[ "$(tab_active)" = 1 ]' "from inside it closes, back to where you were"
[ -n "$(terms | awk '$1=="'"$SHELL_T"'"')" ] && ok "the shell's tab is untouched" || fail "closed the wrong tab"

# --- a remote space ------------------------------------------------------------------------
step "remote space (a fake machine: a second core reached with 'sh -c')"
R="$BASE/remote-repo"
mkdir -p "$R" && git -C "$R" init -q -b main && printf 'one\n' >"$R/r.txt"
git -C "$R" -c user.email=e2e@test -c user.name=e2e add -A && git -C "$R" -c user.email=e2e@test -c user.name=e2e commit -qm init
printf 'one\ntwo\n' >"$R/r.txt"
if perl -e 'alarm 40; exec @ARGV' hc machine add --via 'sh -c' --name fake --session "${HC_SESSION}r" --json >>"$LOG" 2>&1; then
  ok "hc machine add --via 'sh -c' --name fake (a second local core session)"
  # The app connects to the new machine a moment later.
  remote_space() { RM=$("$APP" do space.new "{\"name\":\"remote\",\"folder\":\"$R\",\"machine\":\"fake\"}" 2>>"$LOG"); }
  expect remote_space "a space on machine fake" 30
  RTAB=$(echo "$RM" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tab"])')
  rpanes() { HC_SESSION="${HC_SESSION}r" mod_panes; }
  sleep 1
  changes_button
  expect '"$APP" do gitview.key:r' "the panel is up in the remote space"
  do_ gitview.click:r.txt
  expect '[ "$(view_of $RTAB path)" = r.txt ] && [ "$(view_of $RTAB on)" = fake ]' "a click shows r.txt from machine fake"
  expect 'rpanes | grep -q gitview-diff' "the diff pane opens in the remote tab"
  shot 16-remote 2
  do_ gitview.key:s
  expect 'git -C "$R" status --short | grep -q "^M  r.txt"' "s stages on the machine"
  printf 'fresh\n' >"$R/fresh.txt"
  expect '"$APP" do gitview.file:fresh.txt >/dev/null && [ "$(view_of $RTAB path)" = fresh.txt ]' "the list polls (4 s): a new file there shows up" 15
  do_ gitview.click:r.txt
  do_ gitview-diff.key:enter
  expect 'HC_SESSION="${HC_SESSION}r" hc ls --json | grep -q "\"agent\":\"nvim\""' "enter starts nvim on that machine (herdr-nvim isn't there)" 15
  shot 17-remote-nvim 2
  do_ space.select --index 1
else
  fail "hc machine add --via 'sh -c' (the fake machine)"
fi

# --- the full TUI as panes --------------------------------------------------------------
step "TUI sidebar (mod.open:gitview-tui-sidebar, cmd-alt-shift-g)"
do_ layout.hide --id panel
do_ pane.focus --terminal "$SHELL_T"
do_ mod.open --name gitview-tui-sidebar
expect '[ $(terms | awk '"'"'$2=="env"'"'"' | wc -l) -ge 2 ]' "preview + list panes start"
VIEW=$(ls "$XDG_STATE_HOME"/herdr-gitview/app/views/*.json | grep -v tab.json | head -1)
PREV=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['preview_pane'])" "$VIEW")
LIST=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['list_pane'])" "$VIEW")
expect 'screen $LIST | grep -q "▾"' "the list shows the change sections"
shot 18-tui-sidebar 2
hc keys "$LIST" q >/dev/null
expect '! terms | grep -q "^$LIST "' "q closes the TUI sidebar"
step "TUI in its own tab (mod.open:gitview-tui-tab, cmd-alt-g)"
do_ mod.open --name gitview-tui-tab
expect '[ "$(tab_active)" = gitview ]' "a gitview TUI tab opens"
shot 19-tui-tab 2
do_ mod.open --name gitview-tui-tab
expect '[ "$(tab_active)" = 1 ]' "from inside it closes, back to where you were"

# --- looks: widths and a light theme ---------------------------------------------------------
step "looks: the panel at 280, 300 and 900 px; a light theme"
changes_button
do_ gitview.click:src/main.rs
for w in 280 300 900; do
  do_ layout.resize "{\"id\":\"panel\",\"px\":$w}"
  shot "20-width-$w" 1.2
done
do_ layout.resize '{"id":"panel","px":300}'
do_ gitview.key:t
echo '{"zoom": 1, "theme": "paper", "appearance": "light"}' >"$HERDR_APP_HOME/settings.json"
shot 21-light-tree 2
echo '{"zoom": 1, "theme": "graphite", "appearance": "dark"}' >"$HERDR_APP_HOME/settings.json"
do_ gitview.key:t
status_has '"errors": \[\]' && ok "no mod errors in status" || fail "mod errors: $("$APP" status)"

# --- uninstall -------------------------------------------------------------------------
step "uninstall: our edits go, the layout stays"
"$REPO/herdr-app/install.sh" --uninstall >>"$LOG" 2>&1 || fail "uninstall"
[ "$(app_status layout)" = main-copy ] && ok "still on main-copy (never switched)" || fail "layout switched"
[ -e "$HERDR_APP_HOME/layouts/main-copy.json" ] && ok "main-copy kept (never deleted)" || fail "layout deleted"
expect '[ "$(panel_slot)" = "{\"mod\": \"changes\"}" ]' "its Changes slot is the stock one again"
ls "$HERDR_APP_HOME/mods" | grep -q gitview && fail "mods left" || ok "mods removed"
grep -q gitview "$HERDR_APP_HOME/keys.json" && fail "keys left" || ok "keys removed"
changes_button
shot 22-uninstalled-stock-changes 1.5
passed
