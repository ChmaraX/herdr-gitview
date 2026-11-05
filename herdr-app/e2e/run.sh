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
# git, nvim, python3 with pyobjc (Quartz) and Pillow, node, cargo (or
# GITVIEW_BIN).
set -euo pipefail

# The mods' unit tests first (under a second, no app): a decision they
# break fails here, not minutes into the run.
node --test "$(dirname "$0")"/../test/*.test.mjs >/dev/null 2>&1 || {
  node --test "$(dirname "$0")"/../test/*.test.mjs 2>&1 | grep -E "^✖|Error|expected|actual" | head -20
  echo "the unit tests (herdr-app/test) fail" >&2
  exit 1
}

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
# A diff pane in one of the first space's tabs (a hide closes those; the
# panes in other spaces' tabs stay, for when the person goes back there).
space_diff() {
  local tabs
  tabs=$("$APP" ls | python3 -c 'import json,sys; print(" ".join(t["id"] for s in json.load(sys.stdin)["spaces"] if s["id"] == sys.argv[1] for t in s["tabs"]))' "$SPACE")
  mod_panes | awk -v tabs=" $tabs " '$3 == "gitview-diff" && index(tabs, " " $1 " ") {found=1} END {exit !found}'
}
# The tab has its diff pane, on that file: the pane takes its commands then.
# (A space switch can hide the panel a moment, as the tab bar restores the
# space's own panel; its diff panes close with it and open again.)
pane_on() { has_diff $1 && [ "$(view_of "$1" path)" = "$2" ]; }

MADE=$("$APP" do space.new --name scratch --folder "$S")
TAB=$(echo "$MADE" | tab_of)
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
python3 - "$HERDR_APP_HOME/keys.json" <<'EOF' && ok "keys bound: cmd-g, cmd-shift-g (and no cmd-alt-g / cmd-alt-shift-g)" || fail "keys"
import json, sys
g = json.load(open(sys.argv[1]))["global"]
assert g["cmd-g"] == "mod.open:gitview-tab" and g["cmd-shift-g"] == "mod.open:gitview-sidebar"
assert "cmd-alt-g" not in g and "cmd-alt-shift-g" not in g
EOF
[ ! -s "$HERDR_APP_HOME/keys.error" ] && [ ! -s "$HERDR_APP_HOME/layout.error" ] && ok "no keys.error / layout.error" || fail "app file errors"
for m in gitview gitview-diff gitview-agents gitview-tab gitview-sidebar; do
  grep -q '"focusable": true' "$HERDR_APP_HOME/mods/$m/mod.json" 2>/dev/null && echo "focusable $m" >>"$LOG"
done

# --- the panel and its diff pane ---------------------------------------------------------
step "Changes button → gitview's file list, with its first file already in the diff pane left of it"
do_ pane.focus --terminal "$SHELL_T"
expect panel_up "the panel is up (it takes gitview.* commands)" 20
expect '[ "$(panel_state)" = "open changes" ]' "the panel shows Changes"
# The list notices it's shown from a timer too (a covered window doesn't draw).
expect 'has_diff $TAB' "showing the panel opens the diff pane in this tab"
expect '[ -n "$(view_of $TAB path)" ]' "on the list's first file"
shot 01-panel-list
do_ gitview.click:src/main.rs
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

step "with the diff pane open, a file an agent writes shows in the list at once (not on the 10 s timer)"
# The list's own folder watch (the pane has none: the list writes the view
# again when the file's rev moves).
printf 'written by an agent\n' >"$S/agent.txt"
expect '"$APP" do gitview.file:agent.txt >/dev/null && [ "$(view_of $TAB path)" = agent.txt ]' "agent.txt is in the list within 2 s" 2
rm "$S/agent.txt"
expect '"$APP" do gitview.file:agent.txt >/dev/null && [ "$(view_of $TAB path)" != agent.txt ]' "and gone from it within 2 s once deleted" 2
do_ gitview.click:config.toml
expect '[ "$(view_of $TAB path)" = config.toml ]' "(config.toml in the pane again)"

step "keys: the footer's ? (or the key ?) opens every key; ? or esc closes it"
do_ gitview.key:?
expect 'overlay_open gitview-keys' "? in the list opens the keys card"
expect '[ "$(shared "s.get(\"gitview.keys\") is not None")" = true ]' "(it says so in shared gitview.keys)"
do_ gitview.key:?
expect '! overlay_open gitview-keys' "? again closes it"
do_ gitview-diff.key:?
expect 'overlay_open gitview-keys' "? in the diff pane opens its keys card"
do_ gitview-keys.key:escape
expect '! overlay_open gitview-keys' "esc closes it"
expect '[ "$(shared "s.get(\"gitview.keys\")")" = null ]' "and gitview.keys is cleared"
shot 02b-keys 0.5

step "J/K in the diff pane move through the list (the list follows)"
do_ gitview-diff.key:J
expect '[ "$(view_of $TAB path)" != config.toml ]' "J: the next file (the pane asks the list, which shows it)"
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
do_ mod.open --name gitview-sidebar # cmd-shift-g while it asks
sleep 1.5
[ "$(panel_state)" = "open changes" ] && overlay_open confirm && ok "cmd-shift-g waits while the confirm is open (as v1's modal took every key)" || fail "cmd-shift-g acted under the confirm: $(panel_state)"
shot 04-confirm
confirm_discard gitview # what the dialog's Confirm button runs
expect '[ ! -e "$S/notes.md" ]' "confirmed discard removes the untracked file"
printf 'scratch\n' >"$S/tmp.txt"
do_ gitview.key:r
do_ gitview.click:tmp.txt
expect '[ "$(view_of $TAB path)" = tmp.txt ]' "tmp.txt in the pane"
do_ gitview-diff.key:x
expect 'overlay_open confirm' "pane x asks first"
confirm_discard gitview-diff
expect '[ ! -e "$S/tmp.txt" ]' "pane x then confirm discards it"
expect '[ "$(view_of $TAB path)" != tmp.txt ]' "and the pane moves on to another file"
# Shared state outlives a list copy (app restart, layout switch, the mods
# installed again): a new copy must not do the pane's last ask again.
printf 'scratch again\n' >"$S/tmp.txt"
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh again"
expect panel_up "the mods installed again (new list copies)" 20
do_ gitview.key:r
sleep 2
[ -e "$S/tmp.txt" ] && ok "a new list copy doesn't discard tmp.txt again (the pane's answered ask)" || fail "a new list copy did the pane's old discard again"
rm -f "$S/tmp.txt"
do_ gitview.button:unstage:section:staged
expect '[ -z "$(staged)" ]' "the staged section's − unstages all"
do_ gitview.button:stage:section:changes
expect '[ -n "$(staged)" ] && [ -z "$(git -C "$S" diff --name-only)" ]' "the changes section's + stages all"
do_ gitview.button:unstage:section:staged

step "s then J in the pane: on to the next file"
do_ gitview.click:src/app/a.rs
expect '[ "$(view_of $TAB path)" = src/app/a.rs ]' "src/app/a.rs in the pane"
do_ gitview-diff.key:s
expect '[ "$(view_of $TAB section)" = staged ]' "pane s: it shows where the file went (staged)"
do_ gitview-diff.key:J
expect '[ "$(view_of $TAB path)" != src/app/a.rs ] && [ -n "$(view_of $TAB path)" ]' "J goes on to the next file ($(view_of $TAB path)), not \"last file\""
git -C "$S" reset -q -- src/app/a.rs

step "discard from changes keeps a partly staged file's staged part"
printf 'pub fn b() { 2 }\n// staged\n' >"$S/src/app/b.rs" && git -C "$S" add src/app/b.rs
printf 'pub fn b() { 2 }\n// staged\n// not staged\n' >"$S/src/app/b.rs"
do_ gitview.key:r
do_ gitview.button:discard:in:changes:src/app/b.rs
expect 'overlay_open confirm' "↺ on its changes row asks first"
confirm_discard gitview
expect '[ -z "$(git -C "$S" diff -- src/app/b.rs)" ] && git -C "$S" diff --cached -- src/app/b.rs | grep -q "^+// staged"' "the unstaged part is gone, the staged part stays"
git -C "$S" reset -q -- src/app/b.rs

step "a cancelled discard in one copy never runs from another copy's confirm"
printf 'keep me\n' >"$S/keep.txt"
printf 'drop me\n' >"$S/drop.txt"
do_ gitview.key:r
do_ gitview.button:discard:keep.txt
expect 'overlay_open confirm' "the panel asks about keep.txt"
CANCELLED=$(last_confirm gitview)
do_ overlay.close # Esc: cancelled
LAST_CONFIRM=$CANCELLED
do_ gitview.discard # an old, token-less confirm
do_ "gitview.discard:nottheone"
sleep 1
[ -e "$S/keep.txt" ] && ok "a confirm without the right token discards nothing" || fail "keep.txt discarded"
do_ mod.open --name gitview-tab
expect '[ "$(tab_active)" = gitview ]' "cmd-g's tab: a second gitview list"
expect '"$APP" do gitview.key:r' "its list is up" 20
expect '[ "$(panel_state)" = "hidden changes" ]' "cmd-g's tab shows one list: the panel steps aside there"
OWN0=$(tab_id)
expect '[ -n "$(view_of $OWN0 path)" ]' "and its first file shows in its diff pane at once"
# Two lists watch the repo now (the panel's, hidden, and this tab's), and
# only the newest watch wakes: the stamp (gitview.changed) carries it over.
printf 'w\n' >"$S/w.txt"
expect '"$APP" do gitview.file:w.txt >/dev/null && [ "$(view_of $OWN0 path)" = w.txt ]' "a file written now shows in cmd-g's list within 2 s" 2
# Written while another tab shows: cmd-g's list (the only watch that
# wakes) sees it once its tab shows again, not at its next poll.
do_ tab.select --index 1
expect '[ "$(tab_active)" = 1 ]' "(in tab 1 for a moment)"
printf 'w3\n' >"$S/w3.txt"
sleep 1
do_ tab.select --id "$OWN0"
expect '"$APP" do gitview.file:w3.txt >/dev/null && [ "$(view_of $OWN0 path)" = w3.txt ]' "a file written while another tab showed is in cmd-g's list within 2 s of coming back" 2
do_ gitview.button:discard:drop.txt
expect 'overlay_open confirm' "the tab's list asks about drop.txt"
confirm_discard gitview # its Confirm reaches every list on screen
expect '[ ! -e "$S/drop.txt" ]' "the confirmed one (drop.txt) is discarded"
sleep 1
[ -e "$S/keep.txt" ] && ok "the cancelled one (keep.txt) is not" || fail "keep.txt discarded by another copy's confirm"
do_ mod.open --name gitview-tab # closes it, back to tab 1
expect '[ "$(tab_active)" = 1 ]' "back in tab 1"
expect '[ "$(panel_state)" = "open changes" ]' "the panel is back in tab 1"
printf 'w2\n' >"$S/w2.txt"
expect '"$APP" do gitview.file:w2.txt >/dev/null && [ "$(view_of $TAB path)" = w2.txt ]' "after cmd-g's tab closed, a file written shows in the panel's list within 2 s" 2
rm -f "$S/w.txt" "$S/w2.txt" "$S/w3.txt"
rm -f "$S/keep.txt"
do_ gitview.key:r

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
# A message of more than one line (Shift-Enter in the box), as v1's nvim.
do_ "gitview.commit:$(printf 'e2e: commit the staged config\n\nwith a body line')"
expect 'git -C "$S" log -1 --format=%s | grep -qx "e2e: commit the staged config"' "the commit row commits the staged changes"
[ "$(git -C "$S" log -1 --format=%b)" = "with a body line" ] && ok "with the message's body" || fail "the body: $(git -C "$S" log -1 --format=%b)"
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
KEY0=":$(cd "$S" && pwd -P)"
N0=$(shared "len(s.get('gitview.notes',{}).get('$KEY0',[]))")
do_ "gitview-diff.note:a note on a commit"
sleep 0.5
[ "$(shared "len(s.get('gitview.notes',{}).get('$KEY0',[]))")" = "$N0" ] && ok "no notes on a commit's diff (as the TUI)" || fail "a note on a commit"
do_ gitview.key:escape
do_ gitview.key:escape
expect '[ "$(view_of $TAB commit)" = null ] && [ "$(view_of $TAB scope)" = uncommitted ]' "back to the uncommitted list: the pane leaves the commit's file for the list's selected change"
SELP=$(view_of $TAB path)
git -C "$S" status --porcelain | grep -q " $SELP$\|?? $SELP$" && ok "that file has uncommitted changes" || fail "the pane shows $SELP, not a changed file"

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

step "placing multi-line notes into a program without bracketed paste: an error, nothing typed"
PL="$BASE/plain"
mini_repo "$PL" p.txt 'one\n' 'one\ntwo\n'
# Its own space, so its agent is the dialog's first (this space's agents first).
"$APP" do space.new --name plain --folder "$PL" --argv "[\"/usr/bin/env\",\"DUMMY_PLAIN=1\",\"DUMMY_LOG=$BASE/plain.log\",\"python3\",\"-u\",\"$E2E/dummy-agent.py\"]" >>"$LOG"
expect '[ -e "$BASE/plain.log.json" ]' "a dummy agent without bracketed paste, in a space of its own"
expect panel_up "the panel is up in the plain space" 20
expect '"$APP" do gitview.file:p.txt >/dev/null && pane_on "$(tab_id)" p.txt' "the panel shows its repo; p.txt in the pane"
PKEY=":$(cd "$PL" && pwd -P)"
pnotes() { shared "len(s.get('gitview.notes',{}).get('$PKEY',[]))"; }
do_ "gitview-diff.note:the message has several lines"
expect '[ "$(pnotes)" = 1 ]' "a note to send"
do_ gitview.key:p
expect 'overlay_open gitview-agents' "the send dialog opens"
do_ gitview-agents.key:enter
sleep 2
overlay_open gitview-agents && ok "enter: the dialog stays open with the error" || fail "the dialog closed"
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if not d["submitted"] and not d["input"] else 1)' "$BASE/plain.log.json" && ok "nothing typed, nothing submitted in the program" || fail "the plain program got: $(cat "$BASE/plain.log.json")"
[ "$(pnotes)" = 1 ] && ok "the note is kept" || fail "the note went"
shot 12b-send-refused 1
do_ overlay.close
do_ space.select --index 1

step "enter → nvim (no herdr-nvim): a new nvim right of the diff pane"
do_ gitview.click:src/main.rs
expect 'pane_on $TAB src/main.rs' "src/main.rs in the diff pane"
do_ gitview-diff.key:G
do_ gitview-diff.key:k
do_ gitview-diff.key:enter
expect '[ -n "$(terms | awk '"'"'$2=="nvim"'"'"')" ]' "nvim runs in a new pane"
NVIM=$(terms | awk '$2=="nvim"{print $1}')
hc get "$NVIM" --json | grep -q '"+5"' && ok "at the cursor's line (+5)" || fail "nvim line: $(hc get "$NVIM" --json | head -c 300)"
has_diff $TAB && ok "the diff pane stays beside it" || fail "the diff pane went"
shot 13-nvim 2
# Browsing with it open (v1): unsaved changes keep it, and Enter on another
# file goes to that same nvim; clean, a move quits it and the diff shows.
ed_sock() { shared "s.get('gitview.editors',{}).get('$TAB',{}).get('sock') or ''"; }
ed_nv() { nvim --headless --server "$(ed_sock)" "$@" 2>/dev/null; }
nvim_up() { [ -n "$(terms | awk '$2=="nvim"')" ]; }
diff_count() { mod_panes | grep -c "^$1 .* gitview-diff$"; }
left() { cat "$TRD"/*.jsonl | grep -c "\"leave\":\"nvim\",\"state\":\"$1\""; }
expect '[ "$(ed_nv --remote-expr 1)" = 1 ]' "gitview can ask it (its socket answers)" 15
ed_nv --remote-send '<esc>ggOgitview-unsaved<esc>' >/dev/null
expect '[ "$(ed_nv --remote-expr "&modified")" = 1 ]' "an unsaved change in nvim"
do_ gitview.click:src/app/a.rs
expect '[ "$(left dirty)" -ge 1 ]' "unsaved changes, a click on another file: nvim is asked and stays (a toast says so)"
expect '[ "$(view_of $TAB path)" = src/app/a.rs ]' "the list moves, and the diff pane beside nvim follows"
[ "$(terms | awk '$2=="nvim"{print $1}')" = "$NVIM" ] && [ "$(diff_count $TAB)" = 1 ] && ok "the same nvim, no second diff pane" || fail "nvim $(terms | awk '$2=="nvim"{print $1}'), $(diff_count $TAB) diff panes"
do_ gitview.key:enter
expect 'nvim_buf $NVIM | grep -q "src/app/a.rs$"' "the list's enter on another file goes to that same nvim"
do_ gitview.dblclick:src/app/b.rs
expect 'nvim_buf $NVIM | grep -q "src/app/b.rs$"' "a double-click on a file opens it there too"
[ "$(terms | awk '$2=="nvim"' | wc -l | tr -d ' ')" = 1 ] && ok "no second nvim" || fail "second nvim"
ed_nv --remote-send '<esc>:silent! bufdo e!<cr>' >/dev/null
expect '[ "$(ed_nv --remote-expr "len(filter(getbufinfo({'"'"'bufloaded'"'"': 1}), '"'"'v:val.changed'"'"'))")" = 0 ]' "(the change dropped: nvim is clean)"
P0=$(view_of $TAB path) # (a.rs: the double-click's command opens b.rs without selecting it)
do_ gitview-diff.key:J
expect '! nvim_up' "clean, J in the diff pane: nvim quits (v1)" 15
expect 'has_diff $TAB && [ "$(view_of $TAB path)" != "$P0" ] && [ -n "$(view_of $TAB path)" ]' "and the diff shows the next file after $P0"
[ "$(diff_count $TAB)" = 1 ] && [ "$(left clean)" -ge 1 ] && ok "one diff pane, asked first" || fail "$(diff_count $TAB) diff panes, $(left clean) clean asks"

step "the editor is part of gitview: hiding the panel quits a clean nvim; unsaved changes ask first"
GVCONF="$XDG_CONFIG_HOME/herdr/plugins/config/chmarax.gitview"
mkdir -p "$GVCONF" && printf 'editor = ["nvim"]\n' >"$GVCONF/config.toml"
editor_open() { # Enter in the diff on src/main.rs: nvim, which gitview keeps with the tab
  do_ gitview.click:src/main.rs
  expect 'pane_on $TAB src/main.rs' "src/main.rs in the diff pane"
  do_ gitview-diff.key:enter
  expect 'nvim_up && [ -n "$(ed_sock)" ] && [ "$(ed_nv --remote-expr 1)" = 1 ]' "enter: nvim, and gitview can ask it (its socket answers)" 15
}
editor_open
do_ mod.open --name gitview-sidebar # cmd-shift-g: hide
expect '[ "$(panel_state)" = "hidden changes" ] && ! nvim_up' "hiding the panel quits the clean nvim with it" 15
do_ mod.open --name gitview-sidebar
expect 'pane_on $TAB src/main.rs' "shown again: the diff of src/main.rs" 15
editor_open
ed_nv --remote-send '<esc>ggOgitview-unsaved<esc>' >/dev/null
expect '[ "$(ed_nv --remote-expr "&modified")" = 1 ]' "an unsaved change in nvim"
do_ mod.open --name gitview-sidebar
expect 'overlay_open gitview-ask' "hiding the panel asks first (Save, Discard, Cancel)"
shot 13b-unsaved-ask 0.5
do_ gitview-ask.choose:Cancel
expect '[ "$(panel_state)" = "open changes" ] && nvim_up && [ "$(ed_nv --remote-expr "&modified")" = 1 ]' "Cancel: the panel is back and nvim stays, with its change" 15
expect 'pane_on $TAB src/main.rs' "and the diff with it" 15
do_ mod.open --name gitview-sidebar
expect 'overlay_open gitview-ask' "hiding again asks again"
do_ gitview-ask.choose:Discard
expect '! nvim_up' "Discard: nvim quits without saving" 15
git -C "$S" diff -- src/main.rs | grep -q gitview-unsaved && fail "the discarded change was saved" || ok "and the file has no trace of it"
do_ mod.open --name gitview-sidebar
expect 'pane_on $TAB src/main.rs' "shown again: the diff" 15
rm -f "$GVCONF/config.toml"

step "enter → herdr-nvim (its CLI, as a stand-in): it takes the diff pane's place"
STAND="$HERDR_APP_HOME/mods/herdr-nvim"
mkdir -p "$STAND/bin" && echo '{ "place": "none" }' >"$STAND/mod.json"
printf 'import { View, div } from "gpui-kit";\nexport default class S extends View { render() { return div(); } }\n' >"$STAND/main.js"
printf '#!/bin/sh\necho "$@" >>"%s/herdr-nvim.log"\necho "{\\"terminal\\":\\"t99\\"}"\n' "$BASE" >"$STAND/bin/herdr-nvim" && chmod +x "$STAND/bin/herdr-nvim"
do_ gitview.click:src/main.rs
do_ gitview-diff.key:enter
expect 'grep -q "^open --tab $TAB --space $SPACE .*/src/main.rs:2$" "$BASE/herdr-nvim.log"' "enter asks herdr-nvim: open --tab <tab> --space <space> <file>:<line>"
[ -z "$(terms | awk '$2=="nvim"')" ] && ok "and starts no nvim of its own" || fail "started nvim anyway"
expect '! has_diff $TAB' "the diff pane closes (herdr-nvim's nvim sits there)"
do_ gitview.click:src/app/a.rs
expect 'has_diff $TAB' "the next file click opens the diff pane again"
rm -rf "$STAND"

step "enter → the editor the engine picks: config.toml's editor (a vim), started with +line file"
ED="$BASE/edrepo"
mini_repo "$ED" e.txt 'one\ntwo\nthree\n' 'one\ntwo\nthree, changed\n'
FAKE="$BASE/fakebin" && mkdir -p "$FAKE"
printf '#!/bin/sh\necho "$@" >>"%s/vim.args"\n' "$BASE" >"$FAKE/vim" && chmod +x "$FAKE/vim"
GVCONF="$XDG_CONFIG_HOME/herdr/plugins/config/chmarax.gitview"
mkdir -p "$GVCONF" && printf 'editor = ["%s/vim"]\n' "$FAKE" >"$GVCONF/config.toml"
ETAB=$("$APP" do space.new --name ed --folder "$ED" | tab_of)
expect panel_up "the panel shows the editor repo" 20
expect '"$APP" do gitview.file:e.txt >/dev/null && pane_on $ETAB e.txt' "e.txt in the diff pane" 15
do_ gitview-diff.key:enter
expect '[ -s "$BASE/vim.args" ]' "enter starts the config's editor"
grep -qx "+3 $(cd "$ED" && pwd -P)/e.txt\|+3 $ED/e.txt" "$BASE/vim.args" && ok "with +3 and the file (a vim kind gets the line)" || fail "the editor got: $(cat "$BASE/vim.args")"
rm -f "$GVCONF/config.toml"
do_ space.select --index 1
expect '"$APP" do gitview.click:src/app/a.rs >/dev/null && pane_on $TAB src/app/a.rs' "back in the first space, a click opens its diff pane again" 20

# Keys need the window drawn: macOS doesn't draw a covered window, and a mod
# takes the keyboard only when drawn. Covered (your windows in front; it's
# never raised), the keys' checks are skipped and their commands run
# instead, checked as that.
KEYS=0
if keys_reach; then KEYS=1; else echo "note: key presses reach the private app only while it is the frontmost app (never made so here): skipped, their commands run instead" | tee -a "$LOG"; fi
# press <keys.py spec> <key name> <the mod the key opens> <check> <what it does>
press() {
  if [ "$KEYS" = 1 ]; then
    keys "$1"
    expect "$4" "$2 $5"
  else
    skip "$2 (keys reach only a frontmost app)"
    do_ mod.open --name "$3"
    expect "$4" "mod.open:$3 (what $2 runs) $5"
  fi
}

step "focus: pane.go reaches the diff pane, which takes the keyboard (keys.py, private window only)"
if [ "$KEYS" = 1 ]; then
  do_ pane.focus --terminal "$SHELL_T"
  do_ pane.go --direction right
  sleep 0.6
  L0=$(shared "s['gitview.cursor']['$TAB']['line']")
  keys 38 # j
  keys 38 # j again: a deleted row and its added row can share a line number
  expect '[ "$(shared "s[\"gitview.cursor\"][\"$TAB\"][\"line\"]")" != "$L0" ]' "pane.go then j: the diff pane has the keyboard (the cursor's line moved)"
else
  skip "pane.go then j (keys reach only a frontmost app)"
fi

step "cmd-shift-g toggles the Changes panel; cmd-g opens gitview's own tab"
expect 'has_diff $TAB' "(the diff pane is open before cmd-shift-g)"
SEL=$(view_of $TAB path)
press 5:cmd,shift cmd-shift-g gitview-sidebar '[ "$(panel_state)" = "hidden changes" ] && ! space_diff' "hides the panel and its diff pane together"
press 5:cmd,shift cmd-shift-g gitview-sidebar '[ "$(panel_state)" = "open changes" ] && has_diff $TAB && [ "$(view_of $TAB path)" = "$SEL" ]' "shows both again, on the file that was selected ($SEL)"
if [ "$KEYS" = 1 ]; then
  sleep 0.5
  P0=$(view_of $TAB path)
  keys 38 # j
  expect '[ "$(view_of $TAB path)" != "$P0" ]' "and the list has the keyboard: j moves the diff"
else
  skip "the list has the keyboard after cmd-shift-g (keys reach only a frontmost app)"
fi
shot 14-cmd-shift-g 1
press 5:cmd,shift cmd-shift-g gitview-sidebar '[ "$(panel_state)" = "hidden changes" ] && ! space_diff' "hides both again"
changes_button
expect 'has_diff $TAB' "the Changes button shows both too"
do_ layout.hide --id panel
expect '! space_diff' "and hiding the panel (the Changes button again) closes the diff pane"
# The hide was noticed only by a diff pane in the shown tab once (B7).
changes_button
expect 'has_diff $TAB' "shown again in tab 1, with its diff pane"
TAB2=$("$APP" do tab.new --space "$SPACE" --folder "$S" | tab_of)
expect '[ "$(tab_id)" = "$TAB2" ] && ! mod_panes | grep -q "^$TAB2 "' "a second tab, without a diff pane"
do_ layout.hide --id panel
expect '! space_diff' "the Changes button hides the panel from there: tab 1's diff pane closes too"
changes_button
expect 'has_diff $TAB2 && [ -n "$(view_of $TAB2 path)" ]' "and shows it again: its file opens in a diff pane in this tab"
do_ tab.close --id "$TAB2" --confirmed true
expect '[ "$(tab_id)" = "$TAB" ]' "back in tab 1"
press 5:cmd cmd-g gitview-tab '[ "$(tab_active)" = gitview ]' "opens the gitview tab"
OWN=$(tab_id)
expect 'has_diff $OWN && mod_panes | grep -q "^$OWN .* gitview$"' "diff pane and list pane"
do_ gitview.click:src/main.rs
expect '[ "$(view_of $OWN path)" = src/main.rs ]' "a click in its list shows the diff on the left"
shot 15-own-tab 1.5
do_ tab.select --index 1
press 5:cmd cmd-g gitview-tab '[ "$(tab_active)" = gitview ]' "from another tab jumps to it"
# An nvim gitview starts in that tab closes with it (clean: it quits).
printf 'editor = ["nvim"]\n' >"$GVCONF/config.toml"
do_ gitview-diff.key:enter
expect '[ -n "$(shared "s.get(\"gitview.editors\",{}).get(\"$OWN\",{}).get(\"terminal\") or \"\"")" ] && nvim_up' "enter in cmd-g's tab: an nvim gitview keeps with that tab" 15
press 5:cmd cmd-g gitview-tab '[ "$(tab_active)" = 1 ]' "from inside closes it, back to where you were"
expect '! nvim_up' "and its clean nvim quit with it" 15
rm -f "$GVCONF/config.toml"
[ -n "$(terms | awk '$1=="'"$SHELL_T"'"')" ] && ok "the shell's tab is untouched" || fail "closed the wrong tab"

# --- conflicts, and the last file ------------------------------------------------------------
step "merge conflicts: + on a conflicts folder stages only the conflict"
C="$BASE/conflict"
mkdir -p "$C/dir" && git -C "$C" init -q -b main
gc() { git -C "$C" -c user.email=e2e@test -c user.name=e2e "$@"; }
printf 'base\n' >"$C/dir/c.txt" && printf 'm\n' >"$C/dir/m.txt" && gc add -A && gc commit -qm base
gc checkout -qb other && printf 'theirs\n' >"$C/dir/c.txt" && gc commit -qam theirs
gc checkout -q main && printf 'ours\n' >"$C/dir/c.txt" && gc commit -qam ours
gc merge -q other >/dev/null 2>&1 || true
printf 'm changed\n' >"$C/dir/m.txt"
git -C "$C" diff --name-only --diff-filter=U | grep -q dir/c.txt || fail "no conflict set up"
"$APP" do space.new --name conflict --folder "$C" >>"$LOG"
expect panel_up "the panel is up in the conflict space" 20
CTAB=$(tab_id)
expect '"$APP" do gitview.file:dir/m.txt >/dev/null && pane_on $CTAB dir/m.txt && [ "$(view_of $CTAB root)" = "$(cd "$C" && pwd -P)" ]' "the panel shows the conflicted repo (dir/m.txt in its diff pane)" 20
CHEAD=$(git -C "$C" rev-parse HEAD) CSTATUS=$(git -C "$C" status --porcelain)
do_ "gitview.commit:e2e: must not commit the conflict"
do_ gitview.key:r # runs once the commit is done (commands wait for loads)
sleep 2
[ "$(git -C "$C" rev-parse HEAD)" = "$CHEAD" ] && [ "$(git -C "$C" status --porcelain)" = "$CSTATUS" ] \
  && ok "Commit all with only conflicts (and a change) is refused: no commit, nothing staged, the conflict stays" || fail "a commit during conflicts: $(git -C "$C" status --short)"
[ "$(shared "s.get('gitview.tree')")" = true ] || do_ gitview.key:t
do_ gitview.button:stage:in:conflicts:dir/
expect '[ -z "$(git -C "$C" diff --name-only --diff-filter=U)" ]' "the conflict under dir/ is staged (resolved as it is)"
git -C "$C" diff --name-only | grep -q "dir/m.txt" && ok "dir/m.txt (a change, not a conflict) is left unstaged" || fail "+ on the conflicts folder staged dir/m.txt too"
shot 16a-conflicts 1
do_ gitview.key:t

step "discarding the last file leaves the pane empty"
L="$BASE/last"
mini_repo "$L" only.txt 'one\n' 'two\n'
LTAB=$("$APP" do space.new --name last --folder "$L" | tab_of)
expect panel_up "the panel shows the one-file repo" 20
do_ gitview.click:only.txt
expect 'pane_on $LTAB only.txt' "only.txt in the pane"
do_ gitview-diff.key:x
confirm_discard gitview-diff
expect '[ -z "$(git -C "$L" status --short)" ]' "it's discarded"
expect '[ "$(view_of $LTAB empty)" = true ]' "the pane's view is empty (not a broken one)"
shot 16b-last-file-gone 1.5
do_ space.select --index 1

step "Commit all: with nothing staged the commit row commits everything (untracked too, unless they're hidden)"
A="$BASE/all"
mini_repo "$A" tracked.txt 'one\n' 'one\ntwo\n'
printf 'new\n' >"$A/new.txt"
"$APP" do space.new --name all --folder "$A" >>"$LOG"
expect panel_up "the panel shows a repo with nothing staged" 20
do_ "gitview.commit:e2e: commit all"
expect 'git -C "$A" log -1 --format=%s | grep -qx "e2e: commit all"' "the commit row commits"
[ -z "$(git -C "$A" status --porcelain)" ] && git -C "$A" show --name-only --format= HEAD | sort | tr '\n' ' ' | grep -qx "new.txt tracked.txt " \
  && ok "everything went in: the change and the untracked file" || fail "commit all left: $(git -C "$A" status --short); took: $(git -C "$A" show --name-only --format= HEAD)"
# The person's gitview config (this run's XDG_CONFIG_HOME) hides untracked files.
GVCONF="$XDG_CONFIG_HOME/herdr/plugins/config/chmarax.gitview"
mkdir -p "$GVCONF" && printf 'show_untracked = false\n' >"$GVCONF/config.toml"
printf 'three\n' >>"$A/tracked.txt" && printf 'hidden\n' >"$A/hidden.txt"
do_ gitview.key:r
do_ "gitview.commit:e2e: commit all, untracked hidden"
expect 'git -C "$A" log -1 --format=%s | grep -qx "e2e: commit all, untracked hidden"' "with untracked files hidden, the commit row commits"
[ "$(git -C "$A" show --name-only --format= HEAD)" = tracked.txt ] && [ "$(git -C "$A" status --porcelain)" = "?? hidden.txt" ] \
  && ok "the hidden untracked file stays out (add -u)" || fail "took: $(git -C "$A" show --name-only --format= HEAD); left: $(git -C "$A" status --short)"
rm -f "$GVCONF/config.toml"
do_ space.select --index 1

# --- a remote space ------------------------------------------------------------------------
step "remote space (a fake machine: a second core reached with 'sh -c')"
R="$BASE/remote-repo"
mini_repo "$R" r.txt 'one\n' 'one\ntwo\n'
if perl -e 'alarm 40; exec @ARGV' hc machine add --via 'sh -c' --name fake --session "${HC_SESSION}r" --json >>"$LOG" 2>&1; then
  ok "hc machine add --via 'sh -c' --name fake (a second local core session)"
  # The app connects to the new machine a moment later.
  remote_space() { RM=$("$APP" do space.new "{\"name\":\"remote\",\"folder\":\"$R\",\"machine\":\"fake\"}" 2>>"$LOG"); }
  expect remote_space "a space on machine fake" 30
  RTAB=$(echo "$RM" | tab_of)
  rpanes() { HC_SESSION="${HC_SESSION}r" mod_panes; }
  sleep 1
  expect panel_up "the panel is up in the remote space" 20
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

# --- the preview ---------------------------------------------------------------------------------
step "Preview: markdown and images show rendered by default; m (kept per kind) switches to the diff"
"$PY" -c "
from PIL import Image
Image.new('RGB', (40, 30), (200, 40, 40)).save('$S/pic.png')"
printf '# Doc\n\nfirst\n' >"$S/doc.md"
git -C "$S" add doc.md pic.png && git -C "$S" commit -qm "doc and pic" -- doc.md pic.png
printf '# Doc\n\nfirst, *changed*\n\n- a list\n' >"$S/doc.md"
"$PY" -c "
from PIL import Image
Image.new('RGB', (40, 30), (40, 160, 60)).save('$S/pic.png')"
do_ gitview.key:r
cursor_of() { shared "s.get('gitview.cursor',{}).get('$TAB',{}).get('$1')"; }
expect '"$APP" do gitview.click:doc.md >/dev/null && pane_on $TAB doc.md && [ "$(cursor_of path)" = doc.md ] && [ "$(cursor_of preview)" = true ]' "a markdown file opens in Preview" 15
N0=$(notes_n)
do_ "gitview-diff.note:not in a preview"
sleep 0.5
[ "$(notes_n)" = "$N0" ] && ok "no notes in Preview" || fail "a note in Preview"
shot 22a-preview-md 1
do_ gitview-diff.key:m
expect '[ "$(cursor_of preview)" = false ]' "m: the diff"
expect '"$APP" do gitview.click:pic.png >/dev/null && pane_on $TAB pic.png && [ "$(cursor_of preview)" = true ]' "an image still opens in Preview (the choice is per kind)" 15
CACHE="$HERDR_APP_HOME/mods/gitview-diff/.cache"
AFTER=$(shasum -a 256 "$S/pic.png" | cut -c1-64) BEFORE=$(git -C "$S" show :pic.png | shasum -a 256 | cut -c1-64)
expect '[ -e "$CACHE/$AFTER.png" ] && [ -e "$CACHE/$BEFORE.png" ]' "a changed image shows before (the index) and after (the worktree), byte for byte in the cache"
shot 22b-preview-png 1
expect '"$APP" do gitview.click:doc.md >/dev/null && pane_on $TAB doc.md && [ "$(cursor_of path)" = doc.md ] && [ "$(cursor_of preview)" = false ]' "back on the markdown file: still the diff (kept for markdown)"
do_ gitview-diff.key:m
expect '[ "$(cursor_of preview)" = true ]' "m again: the Preview"
git -C "$S" checkout -q HEAD -- doc.md pic.png

# --- looks: widths and a light theme ---------------------------------------------------------
step "the panel at 280, 300 and 900 px and in a light theme: drawn without errors"
expect panel_up "the panel is up" 20
do_ gitview.click:src/main.rs
panel_px() {
  python3 - "$HERDR_APP_HOME" "$(app_status layout)" <<'PY'
import json, sys
home, name = sys.argv[1:]
print(int(json.load(open(f"{home}/layouts/.state/{name}.json"))["nodes"]["panel"]["size"]["px"]))
PY
}
for w in 280 300 900; do
  do_ layout.resize "{\"id\":\"panel\",\"px\":$w}"
  expect '[ "$(panel_px)" = '$w' ]' "the panel is $w px"
  do_ gitview.key:t
  do_ gitview.key:t
  status_has '"errors": \[\]' && ok "no mod errors at $w px (list and tree)" || fail "mod errors at $w px: $("$APP" status)"
  shot "20-width-$w" 1.2
done
do_ layout.resize '{"id":"panel","px":300}'
do_ gitview.key:t
echo '{"zoom": 1, "theme": "paper", "appearance": "light"}' >"$HERDR_APP_HOME/settings.json"
sleep 1.5
do_ gitview-diff.key:r # the pane reloads its diff in the light syntax set
status_has '"errors": \[\]' && ok "no mod errors in a light theme" || fail "mod errors in light: $("$APP" status)"
shot 21-light-tree 1
echo '{"zoom": 1, "theme": "graphite", "appearance": "dark"}' >"$HERDR_APP_HOME/settings.json"
do_ gitview.key:t

step "Catppuccin Frappé: a comment on an added line reads on its green tint (contrast ≥ 3.5)"
# The regression 1799186 fixed: One Dark's comment color on the theme's own
# insert tint. Measured in the app's frame when the window is drawn; the
# color the pane's render code gives the span is checked either way
# (colors.mjs runs the installed diffview.js on the engine's diff).
CT="$ART/contrast" && mkdir -p "$CT" "$HERDR_APP_HOME/themes"
cp "$E2E/catppuccin-frappe.json" "$HERDR_APP_HOME/themes/catppuccin-frappe.json"
printf '// MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM\n' >"$S/probe.rs"
for _ in 1 2 3 4 5 6; do printf '// MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM a comment\n' >>"$S/probe.rs"; done
echo '{"zoom": 1, "theme": "catppuccin-frappe", "appearance": "dark"}' >"$HERDR_APP_HOME/settings.json"
sleep 1.5
do_ gitview.key:r
expect '"$APP" do gitview.click:probe.rs >/dev/null && [ "$(view_of $TAB path)" = probe.rs ]' "probe.rs (only comments, all added) in the pane"
python3 -c 'import json,sys; th=json.load(open(sys.argv[1])); print(json.dumps({"background":th["background"],"foreground":th["foreground"],"selection":th["selection"],**{f"ansi{i}":c for i,c in enumerate(th["ansi"])},**th.get("colors",{})}))' \
  "$E2E/catppuccin-frappe.json" >"$CT/theme-colors.json"
env GITVIEW_HOST=app "$HERDR_APP_HOME/mods/gitview/bin/herdr-gitview" json diff --repo "$S" --path probe.rs --kind "$(view_of $TAB kind)" --theme dark >"$CT/diff.json" || fail "json diff probe.rs"
node "$E2E/colors.mjs" "$HERDR_APP_HOME/mods/gitview-diff/diffview.js" "$CT/diff.json" "$CT/theme-colors.json" >"$CT/colors.jsonl" || fail "colors.mjs"
COMPUTED=$("$PY" "$E2E/contrast.py" computed "$CT/colors.jsonl" "$E2E/catppuccin-frappe.json") && ok "the pane gives the comment a color that reads: $COMPUTED" || fail "the comment's color doesn't read: $COMPUTED"
echo "$COMPUTED" | python3 -c 'import json,sys; sys.exit(0 if all(o["engine_contrast"] < 3.5 for o in json.load(sys.stdin)) else 1)' \
  && ok "(the engine's own comment color wouldn't: the lift is what the check sees)" || fail "the engine's comment color already reads here: the check proves nothing"
# Pixels need frames: only a window started in front (E2E_FOREGROUND=1)
# draws; a background-started one never renders on its own.
if [ -n "${E2E_FOREGROUND:-}" ] && until_ drawn 5; then
  PIXELS=
  for i in 1 2 3; do
    shot "23-frappe-probe-$i" 1.5
    PIXELS=$("$PY" "$E2E/contrast.py" pixels "$ART/23-frappe-probe-$i.png" "$E2E/catppuccin-frappe.json") && break
  done
  echo "$PIXELS" >"$CT/pixels.json"
  "$PY" "$E2E/contrast.py" pixels "$ART/23-frappe-probe-$i.png" "$E2E/catppuccin-frappe.json" >/dev/null \
    && ok "drawn: the comment's pixels on the tint reach 3.5: $PIXELS" || fail "drawn comment pixels: $PIXELS"
else
  skip "the drawn pixels of the comment (no frames: the window started in the background; E2E_FOREGROUND=1)"
fi
echo '{"zoom": 1, "theme": "graphite", "appearance": "dark"}' >"$HERDR_APP_HOME/settings.json"
rm -f "$S/probe.rs"

# --- the trace ---------------------------------------------------------------------------
step "the trace: every copy's and the engine's lines in one folder, and trace.py reads it"
do_ mod.open --name gitview-mark
expect 'grep -qh "\"mod\":\"gitview-mark\".*\"kind\":\"mark\"" "$TRD"/*.jsonl && grep -qh "\"mod\":\"list\".*\"kind\":\"mark\"" "$TRD"/*.jsonl' "the mark command: a mark with its snapshot, and the list's own part (within its 1 s flush)" 10
sleep 1.5 # the other copies' last lines
python3 "$REPO/herdr-app/tools/trace.py" "$TRD" >"$ART/trace.txt" 2>&1 || fail "trace.py: $(tail -3 "$ART/trace.txt")"
python3 "$REPO/herdr-app/tools/trace.py" "$TRD" --around mark >"$ART/trace-mark.txt" 2>&1
grep -q '"panes"' "$ART/trace.txt" && grep -q 'list/.* mark ' "$ART/trace-mark.txt" && ok "trace.py --around mark: the mark and the copies' snapshots" || fail "no mark in trace.py's output"
grep -Eq '^[0-9:.]+ (list|pane) +(key:|click)' <(sed -n '/== interactions/,$p' "$ART/trace.txt") && ok "the inputs, each with what it led to" || fail "no inputs in trace.py's interactions"
grep -Eq '^status +n=[0-9]+ .* · +[0-9.]+ +[0-9.]+' "$ART/trace.txt" && ok "the execs: the mods' ms and the engine's ($(grep -E '^status +n=' "$ART/trace.txt"))" || fail "status execs without the engine's ms"
grep -q ' exec .*sidebar ' "$ART/trace.txt" && grep -q 'cmd=pane.close' "$ART/trace.txt" && grep -q 'edge=' "$ART/trace.txt" \
  && ok "a diff pane's split (sidebar), a pane.close and the panel's edges" || fail "the split, close or edges are missing"
BYTES=$(cat "$TRD"/*.jsonl | wc -c | tr -d ' ') # (grep -q on a pipe: cat's SIGPIPE fails it under pipefail)
echo "trace: $BYTES bytes in $(ls "$TRD" | wc -l | tr -d ' ') files after ${SECONDS}s ($(( BYTES * 60 / (SECONDS > 0 ? SECONDS : 1) / 1024 )) KB/min)" | tee -a "$LOG" >"$ART/trace-size.txt"

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
