#!/usr/bin/env bash
# End-to-end run of gitview in a private Herdr app: its own app home, its own
# herdr-core session, a scratch repo and a dummy agent. Nothing touches your
# own app, core, spaces or terminals.
#
#   herdr-app/e2e/run.sh [artifacts dir]     (default: /tmp/gitview-e2e-<time>)
#
# It installs gitview with herdr-app/install.sh, drives every feature through
# `herdr-app do` (the same commands keys and buttons run) and `hc`, asserts
# on git, on the terminals' screens and on the app's state, and saves a
# screenshot per step plus run.log. Then it uninstalls (checking the stock
# Changes panel is back) and shuts the private app and core down.
#
# The window only draws while it's visible, so each screenshot raises it
# for a moment. Needs: the Herdr app (HERDR_APP_BIN, else found like
# install.sh does), hc, git, nvim, python3, cargo (or GITVIEW_BIN).
set -euo pipefail


. "$(dirname "$0")/lib.sh"

S="$BASE/scratch"
mkdir -p "$S/src"
git -C "$S" init -q -b main
git -C "$S" config user.email e2e@test && git -C "$S" config user.name e2e
printf 'fn main() {\n    println!("hello");\n}\n' >"$S/src/main.rs"
printf 'a = 1\nb = 2\n' >"$S/config.toml"
printf 'old\nfile\n' >"$S/gone.txt"
git -C "$S" add -A && git -C "$S" commit -qm init
printf 'fn main() {\n    println!("hello, gitview");\n    let x = 42;\n}\n' >"$S/src/main.rs"
printf 'a = 1\nb = 3\nc = 4\n' >"$S/config.toml"
git -C "$S" rm -q gone.txt
printf 'brand new\n' >"$S/added.txt" && git -C "$S" add added.txt
printf 'untracked\n' >"$S/notes.md"
O="$BASE/other"
mkdir -p "$O" && git -C "$O" init -q -b main && printf 'x\n' >"$O/other.txt"
git -C "$O" -c user.email=e2e@test -c user.name=e2e add -A
git -C "$O" -c user.email=e2e@test -c user.name=e2e commit -qm init && printf 'y\n' >>"$O/other.txt"

do_ space.new --name scratch --folder "$S"
SHELL_T=$(hc ls --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["terminals"][0]["address"])')
do_ pane.split --direction down --argv "[\"/usr/bin/env\",\"DUMMY_LOG=$BASE/dummy.log\",\"python3\",\"-u\",\"$REPO/herdr-app/e2e/dummy-agent.py\"]"
expect 'terms | grep -q " dummy "' "dummy agent reports a status"
AGENT=$(terms | awk '$2=="dummy"{print $1}')

step "install (herdr-app/install.sh)"
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh"
ok "install.sh, including herdr-app check of every mod"
expect panel_shows "the layout's Changes slot shows the gitview mod"
grep -q '"cmd-g": "mod.open:gitview-tab"' "$HERDR_APP_HOME/keys.json" && grep -q '"cmd-alt-g": "mod.open:gitview-tui-tab"' "$HERDR_APP_HOME/keys.json" && ok "keys bound" || fail "keys"
[ ! -s "$HERDR_APP_HOME/keys.error" ] && [ ! -s "$HERDR_APP_HOME/layout.error" ] && ok "no keys.error / layout.error" || fail "app file errors"

# --- the panel (the Changes button) ---------------------------------------------------
step "Changes button → gitview panel"
# Let the new layout's tab bar draw once first: it restores each space's
# panel choice when it first renders, which in a hidden window is only now.
raise
sleep 1.5
do_ pane.focus --terminal "$SHELL_T"
# What the tab bar's Changes button runs:
do_ layout.select '{"stack":"panel-body","child":"changes"}'
do_ layout.show --id panel
expect '"$APP" do gitview.key:r' "the panel is up (it takes gitview.* commands)"
do_ gitview.file:src/main.rs
shot 01-panel
status_has '"errors": \[\]' && ok "no mod errors in status" || fail "mod errors: $("$APP" status)"

step "panel: note on lines, send to the agent (place, then submit)"
do_ gitview.key:tab
do_ gitview.key:v
do_ gitview.key:j
do_ "gitview.note:please name x something meaningful"
shot 02-panel-note
do_ gitview.key:p
expect 'overlay_open gitview-agents' "the send-notes dialog opens"
shot 03-panel-send
do_ gitview-agents.key:enter
expect 'agent_input | grep -q "src/main.rs:2-2 — please name x"' "enter places the notes in the agent's input"
[ -z "$(agent_submitted)" ] && ok "not submitted" || fail "placing must not submit"
expect '! overlay_open gitview-agents' "the dialog closes"
do_ "gitview.note:and send this one now"
do_ gitview.key:p
expect 'overlay_open gitview-agents' "the dialog opens again"
do_ gitview-agents.key:shift-enter
expect 'agent_submitted | grep -q "and send this one now"' "shift+enter submits them" 15
[ -z "$(agent_input)" ] && ok "the input is empty again" || fail "input left over"
expect '! overlay_open gitview-agents' "the dialog closes"
shot 04-panel-sent

step "panel: confirm, discard, stage"
do_ gitview.file:notes.md
do_ gitview.key:x
expect 'overlay_open confirm' "x asks first (the app's confirm dialog)"
shot 05-panel-confirm
do_ gitview.discard # what the dialog's Confirm button runs
do_ overlay.close
expect '[ ! -e "$S/notes.md" ]' "confirmed discard removes the untracked file"
do_ gitview.file:config.toml
do_ gitview.key:s
expect 'git -C "$S" status --short | grep -q "^M  config.toml"' "s stages the file"
do_ "gitview.commit:panel: commit the staged config"
expect 'git -C "$S" log -1 --format=%s | grep -qx "panel: commit the staged config"' "the commit box commits the staged changes"
git -C "$S" status --short | grep -q "src/main.rs" && ok "unstaged changes stay out of it" || fail "commit took unstaged changes"

step "panel: enter opens nvim, and the next file goes to that nvim"
do_ pane.focus --terminal "$SHELL_T"
do_ gitview.file:src/main.rs
do_ gitview.key:enter
expect '[ -n "$(terms | awk '"'"'$2=="nvim"'"'"')" ]' "nvim runs in a new pane"
NVIM=$(terms | awk '$2=="nvim"{print $1}')
hc get "$NVIM" --json | grep -q '"+2"' && ok "at the first changed line (+2)" || fail "nvim line"
shot_settled 06-panel-nvim
printf 'a second file\n' >"$S/second.txt"
do_ gitview.key:r
do_ gitview.file:second.txt
do_ gitview.key:enter
expect 'nvim_buf $NVIM | grep -q "second.txt$"' "enter on another file opens it in that nvim"
[ "$(terms | awk '$2=="nvim"' | wc -l | tr -d ' ')" = 1 ] && ok "no second nvim" || fail "second nvim"
hc keys "$NVIM" esc shift+z shift+q >/dev/null
expect '[ -z "$(terms | awk '"'"'$2=="nvim"'"'"')" ]' "ZQ closes nvim and its pane"
# herdr-nvim present (a stand-in for its CLI): enter goes there, no nvim pane.
STAND="$HERDR_APP_HOME/mods/herdr-nvim"
mkdir -p "$STAND/bin" && echo '{ "place": "none" }' >"$STAND/mod.json"
printf 'import { View, div } from "gpui-kit";\nexport default class S extends View { render() { return div(); } }\n' >"$STAND/main.js"
printf '#!/bin/sh\necho "$@" >>"%s/herdr-nvim.log"\necho "{\\"terminal\\":\\"t99\\"}"\n' "$BASE" >"$STAND/bin/herdr-nvim" && chmod +x "$STAND/bin/herdr-nvim"
do_ gitview.file:src/main.rs
do_ gitview.key:enter
expect 'grep -q "^open --tab tb[0-9a-f]* .*/src/main.rs:2$" "$BASE/herdr-nvim.log"' "with herdr-nvim installed, enter asks it: herdr-nvim open --tab <tab> <file>:<line>"
[ -z "$(terms | awk '$2=="nvim"')" ] && ok "and starts no nvim of its own" || fail "started nvim anyway"
rm -rf "$STAND"

step "panel follows the shown space"
do_ space.new --name other --folder "$O"
raise # the tab bar restores the new space's panel choice (none) when it draws
sleep 1.5
do_ layout.select '{"stack":"panel-body","child":"changes"}'
do_ layout.show --id panel
do_ gitview.key:r
shot 07-panel-other-space # shows other.txt, the other repo's change
do_ space.select --index 1
raise
sleep 1.5
do_ gitview.key:r
shot 07b-panel-back
ok "the panel follows the shown space (see 07-panel-other-space.png, 07b-panel-back.png)"

# --- gitview as a pane / a tab (native) ---------------------------------------------------
step "gitview as a sidebar (mod.open:gitview-sidebar, cmd-shift-g)"
do_ layout.hide --id panel
do_ pane.focus --terminal "$SHELL_T"
do_ mod.open --name gitview-sidebar
expect 'mod_panes | grep -q " gitview$"' "a full-height gitview pane opens in this tab"
shot_settled 08-native-sidebar
do_ mod.open --name gitview-sidebar
expect '! mod_panes | grep -q " gitview$"' "pressed again, it closes"

step "gitview in its own tab (mod.open:gitview-tab, cmd-g)"
do_ mod.open --name gitview-tab
expect '[ "$(tab_active)" = gitview ]' "a gitview tab opens"
shot_settled 09-native-tab
do_ tab.select --index 1
do_ mod.open --name gitview-tab
expect '[ "$(tab_active)" = gitview ]' "from another tab it jumps to it"
do_ mod.open --name gitview-tab
expect '[ "$(tab_active)" = 1 ]' "from inside it closes, back to where you were"
[ -n "$(terms | awk '$1=="'"$SHELL_T"'"')" ] && ok "the shell's tab is untouched" || fail "closed the wrong tab"

# --- the full TUI as panes --------------------------------------------------------------
step "TUI sidebar (mod.open:gitview-tui-sidebar, cmd-alt-shift-g)"
do_ pane.focus --terminal "$SHELL_T"
do_ mod.open --name gitview-tui-sidebar
expect '[ $(terms | awk '"'"'$2=="env"'"'"' | wc -l) -ge 2 ]' "preview + list panes start"
VIEW=$(ls "$XDG_STATE_HOME"/herdr-gitview/app/views/*.json | grep -v tab.json | head -1)
PREV=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['preview_pane'])" "$VIEW")
LIST=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['list_pane'])" "$VIEW")
expect 'screen $LIST | grep -q "▾"' "the list shows the change sections"
shot 10-tui-sidebar
hc keys "$LIST" g >/dev/null
for _ in 1 2 3 4 5 6; do
  screen "$PREV" | head -1 | grep -q "main.rs" && break
  hc keys "$LIST" j >/dev/null
  sleep 0.5
done
expect 'screen $PREV | head -1 | grep -q "main.rs"' "j/k drive the diff"
hc keys "$LIST" enter >/dev/null
expect 'screen $PREV | grep -q "let x = 42"' "enter opens nvim in the diff pane"
expect '"$APP" status | grep -q focused_pane' "focus moved"
shot_settled 11-tui-nvim
hc keys "$PREV" esc shift+z shift+q >/dev/null
expect '[ "$(fg_name $PREV)" != nvim ] && screen $PREV | head -1 | grep -q "main.rs"' "ZQ brings the diff back"
hc keys "$LIST" x >/dev/null
expect 'overlay_open layout && terms | grep -q " $LIST$"' "x asks in a dialog (a popup terminal in an overlay)"
ASK=$(terms | awk -v p="$LIST" '$3==p{print $1}' | tail -1)
shot 12-tui-confirm
hc keys "$ASK" n >/dev/null
expect '! terms | grep -q "^$ASK "' "n answers and the dialog closes"
git -C "$S" status --short | grep -q "src/main.rs" && ok "n kept the change" || fail "n discarded"
hc keys "$PREV" v j a >/dev/null
hc send "$PREV" "from the TUI" >/dev/null
hc keys "$PREV" p >/dev/null
expect '[ -n "$(terms | awk -v p="$PREV" '"'"'$3==p'"'"')" ]' "p opens the agent picker"
PICK=$(terms | awk -v p="$PREV" '$3==p{print $1}' | tail -1)
shot 13-tui-pick-agent
hc keys "$PICK" enter >/dev/null
expect 'agent_input | grep -q "from the TUI"' "the TUI places notes in the agent too"
hc keys "$LIST" q >/dev/null
expect '! terms | grep -q "^$LIST "' "q closes the sidebar"
expect '[ -z "$(ls "$XDG_STATE_HOME"/herdr-gitview/app/views/ 2>/dev/null)" ]' "and its state"

step "TUI in its own tab (mod.open:gitview-tui-tab, cmd-alt-g)"
do_ mod.open --name gitview-tui-tab
expect '[ "$(tab_active)" = gitview ]' "a gitview TUI tab opens"
shot 14-tui-tab
do_ tab.select --index 1
do_ mod.open --name gitview-tui-tab
expect '[ "$(tab_active)" = gitview ]' "from another tab it jumps to it"
do_ mod.open --name gitview-tui-tab
expect '[ "$(tab_active)" = 1 ]' "from inside it closes, back to where you were"

# --- uninstall -------------------------------------------------------------------------
step "uninstall restores the stock Changes panel"
"$REPO/herdr-app/install.sh" --uninstall >>"$LOG" 2>&1 || fail "uninstall"
"$APP" status | grep -q '"layout": "main"' && ok "back on the shipped main layout" || fail "layout"
[ ! -e "$HERDR_APP_HOME/layouts/main-mods.json" ] && ok "clone removed" || fail "clone left"
ls "$HERDR_APP_HOME/mods" | grep -q gitview && fail "mods left" || ok "mods removed"
grep -q gitview "$HERDR_APP_HOME/keys.json" && fail "keys left" || ok "keys removed"
do_ layout.select '{"stack":"panel-body","child":"changes"}'
do_ layout.show --id panel
shot 15-uninstalled-stock-changes
step "all passed"
