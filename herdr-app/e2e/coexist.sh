#!/usr/bin/env bash
# gitview and herdr-nvim in one private Herdr app, installed in either order:
# both edit the person's one personal layout (gitview the `changes` slot,
# herdr-nvim the `files` slot, the 300 px panel and its floating sync
# controller); neither touches the other's edits or keys, and each uninstall
# leaves the layout and keys.json byte-identical to before its install. No
# layout is deleted or switched. And Enter in gitview opens the file in
# herdr-nvim's nvim through its CLI (`herdr-nvim open --tab <tab> --space
# <space> <file>:<line>`), whose pane takes the diff pane's place.
#
#   herdr-app/e2e/coexist.sh [artifacts dir]
#
# HERDR_NVIM_SRC: a herdr-nvim checkout (default ~/projects/herdr-nvim). Its
# committed HEAD is copied to a scratch folder and installed from there, so
# the checkout itself is never built in or changed.
set -euo pipefail
HNV_SRC="${HERDR_NVIM_SRC:-$HOME/projects/herdr-nvim}"
[ -x "$HNV_SRC/herdr-app/install.sh" ] || { echo "no herdr-nvim checkout at $HNV_SRC (set HERDR_NVIM_SRC)" >&2; exit 2; }

. "$(dirname "$0")/lib.sh"

HNV_SHA=$(git -C "$HNV_SRC" rev-parse --short HEAD)
HNV="${GITVIEW_BUILD_DIR:-/tmp/gitview-e2e-build}/herdr-nvim-$HNV_SHA"
if [ ! -d "$HNV" ]; then
  mkdir -p "$HNV" && git -C "$HNV_SRC" archive HEAD | tar -x -C "$HNV"
fi
echo "herdr-nvim $HNV_SHA from $HNV" >>"$LOG"

LAYOUT() { echo "$HERDR_APP_HOME/layouts/$(app_status layout).json"; }
slots() { # "<layout> files=<mod> changes=<mod>"
  python3 - "$(LAYOUT)" <<'PY'
import json, os, sys
doc = json.load(open(sys.argv[1]))
def find(n, i):
    if n.get("id") == i: return n
    for c in n.get("children", []):
        r = find(c, i)
        if r: return r
print(os.path.basename(sys.argv[1])[:-5], *(f"{s}={find(doc, s)['content']['mod']}" for s in ("files", "changes")))
PY
}
# herdr-nvim's own edits, as they read in the layout: the panel's width and
# its floating sync controller.
nvim_bits() {
  python3 - "$(LAYOUT)" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
def walk(n):
    yield n
    for c in n.get("children", []): yield from walk(c)
nodes = list(walk(doc))
panel = next(n for n in nodes if n.get("id") == "panel")
sync = [f for n in nodes for f in n.get("floating", []) if f.get("content", {}).get("mod") == "herdr-nvim"]
print("panel", panel.get("size"), "sync", json.dumps(sync, sort_keys=True))
PY
}
nvim_keys() { python3 -c 'import json,sys; g=json.load(open(sys.argv[1]))["global"]; print({k: g.get(k) for k in ("cmd-e","cmd-o","cmd-shift-e")})' "$HERDR_APP_HOME/keys.json"; }
# Snapshots, kept in the artifacts too (the run's folder goes at the end).
snap() {
  cp "$(LAYOUT)" "$BASE/$1.layout.json"; cp "$HERDR_APP_HOME/keys.json" "$BASE/$1.keys.json"
  mkdir -p "$ART/snapshots" && cp "$BASE/$1.layout.json" "$BASE/$1.keys.json" "$ART/snapshots/"
}
same_as() { # byte-identical layout and keys.json to snapshot $1
  cmp -s "$(LAYOUT)" "$BASE/$1.layout.json" && cmp -s "$HERDR_APP_HOME/keys.json" "$BASE/$1.keys.json"
}
nvim_up() { (cd "$HNV" && herdr-app/install.sh --app-bin "$APP") >>"$LOG" 2>&1; }
nvim_down() { (cd "$HNV" && herdr-app/install.sh --uninstall --app-bin "$APP") >>"$LOG" 2>&1; }
gv_up() { "$REPO/herdr-app/install.sh" >>"$LOG" 2>&1; }
gv_down() { "$REPO/herdr-app/install.sh" --uninstall >>"$LOG" 2>&1; }
want() { local got; got="$(slots)"; [ "$got" = "$1" ] && ok "$2: $got" || fail "$2: got $got, want $1"; }

S="$BASE/scratch"
mkdir -p "$S/src"
git -C "$S" init -q -b main
printf 'fn main() {\n    println!("hello");\n}\n' >"$S/src/main.rs"
git -C "$S" -c user.email=e2e@test -c user.name=e2e add -A
git -C "$S" -c user.email=e2e@test -c user.name=e2e commit -qm init
printf 'fn main() {\n    println!("hello, nvim");\n}\n' >"$S/src/main.rs"
MADE=$("$APP" do space.new --name scratch --folder "$S")
TAB=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tab"])')
SPACE=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["space"])')

# --- order 1 -------------------------------------------------------------------------
step "herdr-nvim first (it clones main as main-copy), then gitview"
nvim_up || fail "herdr-nvim install"
want "main-copy files=herdr-nvim changes=changes" "herdr-nvim's files slot in the personal copy"
snap nvim
NBITS=$(nvim_bits); NKEYS=$(nvim_keys)
echo "herdr-nvim: $NBITS $NKEYS" >>"$LOG"
echo "$NBITS" | grep -Eq "^panel \{'px': 300(\.0)?\}" && ok "herdr-nvim's panel is 300 px" || fail "herdr-nvim's panel isn't 300 px: $NBITS"
gv_up || fail "gitview install"
want "main-copy files=herdr-nvim changes=gitview" "gitview edits the same personal layout, its changes slot only"
[ "$(nvim_bits)" = "$NBITS" ] && ok "herdr-nvim's panel width and sync controller untouched" || fail "herdr-nvim's bits changed: $(nvim_bits)"
[ "$(nvim_keys)" = "$NKEYS" ] && ok "herdr-nvim's keys untouched: $NKEYS" || fail "herdr-nvim's keys changed: $(nvim_keys)"
python3 - "$BASE/nvim.layout.json" "$(LAYOUT)" <<'PY' && ok "the only layout change is the changes slot" || fail "gitview changed more than its slot"
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:])
def drop(n):
    if n.get("id") == "changes": n = {**n, "content": None}
    return {**n, "children": [drop(c) for c in n.get("children", [])]} if "children" in n else n
assert drop(a) == drop(b)
PY

step "Enter in gitview opens the file in herdr-nvim's nvim; its pane takes the diff pane's place"
do_ pane.focus --terminal "$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["terminal"])')"
expect panel_up "the gitview panel is up" 20
expect '[ "$(panel_state)" = "open changes" ]' "the panel shows Changes (gitview), herdr-nvim's sync controller leaves it"
do_ gitview.click:src/main.rs
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$"' "a click opens gitview's diff pane"
shot coexist-01-diff-pane 1.5
do_ gitview-diff.key:enter
DEBUG="$GV_LOG"
expect 'grep -q "herdr-nvim open .*/src/main.rs:2 -> exit status: 0" "$DEBUG"' "gitview ran herdr-nvim open --tab <tab> --space <space> <file>:2, and it succeeded" 40
NV=$(grep "herdr-nvim open .*/src/main.rs:2 -> exit status: 0" "$DEBUG" | tail -1 | sed 's/.*"terminal":"\([^"]*\)".*/\1/')
expect 'nvim_buf $NV | grep -q "src/main.rs$"' "herdr-nvim's nvim ($NV) shows the file" 30
expect '! mod_panes | grep -q gitview-diff' "gitview closed its diff pane (herdr-nvim's nvim sits there now)"
sleep 1
echo "after herdr-nvim open, the side panel shows: $(panel_state)" | tee -a "$LOG"
shot coexist-02-enter-herdr-nvim 2
# herdr-nvim's nvim of this tab (its terminal carries herdr-nvim.sidebar=<tab>),
# its daemon's socket (the --server its remote UI attaches to), and asking it.
tab_nvim() {
  hc ls --json --label "herdr-nvim.sidebar=$TAB" | python3 -c 'import json,sys; print(" ".join(t["address"] for t in json.load(sys.stdin)["terminals"] if t["state"] == "live"))'
}
nv_sock() { hc get "$1" --json | python3 -c 'import json,sys; a=json.load(sys.stdin)["foreground"]["argv"]; print(a[a.index("--server")+1] if "--server" in a else "")'; }
nv() { nvim --headless --server "$SOCK" "$@" 2>/dev/null; }
diff_pane() { mod_panes | awk -v t="$TAB" '$1==t && $3=="gitview-diff" {print $2}'; }
diff_has_keys() { [ -n "$(diff_pane)" ] && [ "$(app_status focused_pane)" = "$(diff_pane)" ]; }

step "a click on the row while it's in nvim brings the diff back; nvim keeps running with the buffer and an unsaved edit"
SOCK=$(nv_sock "$NV")
[ -n "$SOCK" ] && ok "herdr-nvim's daemon: $SOCK" || fail "no daemon socket for $NV"
nv --remote-send '<esc>ggOgitview-unsaved<esc>'
expect '[ "$(nv --remote-expr "&modified")" = 1 ]' "an unsaved edit in nvim"
# The click is handled at once: a status load in flight when the diff pane
# closed used to hang for 35 s (the app dropped its exec; Loads.revive).
CLICK0=$SECONDS
do_ gitview.click:src/main.rs
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$"' "the click on the row brings the diff pane back" 30
TOOK=$((SECONDS - CLICK0))
[ "$TOOK" -lt 5 ] && ok "and it took under 5 s ($TOOK s)" || fail "the click after herdr-nvim open took $TOOK s"
expect '[ -z "$(tab_nvim)" ]' "nvim's pane is gone (the diff is in its place)"
[ "$(nv --remote-expr 'getline(1)')" = gitview-unsaved ] && [ "$(nv --remote-expr '&modified')" = 1 ] \
  && ok "nvim still runs, with the buffer and its unsaved edit" || fail "nvim lost the edit: $(nv --remote-expr 'getline(1)')"
expect diff_has_keys "the diff pane has the keyboard"
nv --remote-send '<esc>u'
expect '[ "$(nv --remote-expr "&modified")" = 0 ]' "(the edit undone, for :q)"
echo "then: panel $(panel_state)" | tee -a "$LOG"
shot coexist-03-click-again 2

# The list holds the keys with the app's focus (its trace: focused, and
# the app's focus came with it).
list_has_keys() {
  grep -a 'GVTRACE.*"mod":"list"' "$BASE/app.log" | grep -F "\"tab\":\"$TAB\"" | tail -1 | sed 's/.*GVTRACE //' | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("focused") and d.get("viaApp") else 1)'
}

step "ZZ in nvim brings the diff back on the same file, the keys on the list (v1)"
do_ gitview-diff.key:enter
expect '[ -n "$(tab_nvim)" ] && ! mod_panes | grep -q "^$TAB .* gitview-diff$"' "Enter: the file in nvim again, in the diff's place" 30
NV2=$(tab_nvim)
sleep 1
# Quit nvim in its pane with ZZ (save if changed, quit; hc keys can't type
# a colon for :q, shift+; is ;).
hc keys "$NV2" esc shift+z shift+z >>"$LOG" 2>&1
expect '[ -z "$(tab_nvim)" ]' "quitting nvim closes its pane" 15
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$" && [ "$(view_of $TAB path)" = src/main.rs ]' "the diff pane is back on src/main.rs within 2 s" 2
expect list_has_keys "with the keys on the list"
expect focus_check "and one focus shows"
shot coexist-03b-q-back 1.5

step "cmd-shift-g with the file in nvim closes the whole unit, nvim too (its daemon keeps the edit); again: the diff, the keys on the list"
do_ gitview-diff.key:enter
expect '[ -n "$(tab_nvim)" ] && ! mod_panes | grep -q "^$TAB .* gitview-diff$"' "Enter: nvim, in the diff's place" 30
expect '[ "$(nv --remote-expr 1)" = 1 ]' "its daemon answers on the tab's socket" 15
nv --remote-send '<esc>ggOgitview-unsaved-2<esc>'
expect '[ "$(nv --remote-expr "&modified")" = 1 ]' "an unsaved edit in nvim"
do_ mod.open --name gitview-sidebar
# A window started in the background lags its own live data now and then
# (herdr-app 0.1.12): the launcher can read the panel as hidden and show it.
# Once more then, and the log says so.
if ! until_ '[ "$(panel_state)" = "hidden changes" ]' 5; then
  echo "note: cmd-shift-g didn't hide the panel within 5 s (the launcher read stale live data); pressed again" | tee -a "$LOG"
  do_ mod.open --name gitview-sidebar
fi
expect '[ "$(panel_state)" = "hidden changes" ]' "cmd-shift-g: the panel hides" 15
expect '! mod_panes | grep -q "^$TAB .* gitview-diff$"' "the diff pane goes with it" 15
expect '[ -z "$(tab_nvim)" ]' "and nvim's pane too (gitview handed it the file)" 15
[ "$(nv --remote-expr 'getline(1)')" = gitview-unsaved-2 ] && [ "$(nv --remote-expr '&modified')" = 1 ] \
  && ok "nvim's daemon still holds the buffer and its unsaved edit" || fail "the daemon lost the edit: $(nv --remote-expr 'getline(1)')"
nv --remote-send '<esc>u' # undone, for what follows
do_ mod.open --name gitview-sidebar
expect '[ "$(panel_state)" = "open changes" ] && mod_panes | grep -q "^$TAB .* gitview-diff$" && [ "$(view_of $TAB path)" = src/main.rs ]' "cmd-shift-g again: the diff of src/main.rs, not nvim" 15
[ -z "$(tab_nvim)" ] && ok "(nvim's pane stays closed)" || fail "nvim's pane came back"
expect list_has_keys "the keys on the list"

step "leaving the tab before :q: the diff doesn't come back"
do_ gitview-diff.key:enter
expect '[ -n "$(tab_nvim)" ] && ! mod_panes | grep -q "^$TAB .* gitview-diff$"' "Enter: nvim again" 30
NV3=$(tab_nvim)
sleep 1
OTHER=$("$APP" do tab.new --space "$SPACE" --folder "$S" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tab"])')
expect '[ "$(tab_id)" = "$OTHER" ]' "in another tab"
sleep 1
do_ tab.select --id "$TAB"
expect '[ "$(tab_id)" = "$TAB" ]' "back in the first tab"
hc keys "$NV3" esc shift+z shift+q >>"$LOG" 2>&1
expect '[ -z "$(tab_nvim)" ]' "quitting nvim closes its pane" 15
sleep 3
! mod_panes | grep -q "^$TAB .* gitview-diff$" && ok "no diff came back (the person had left the tab)" || fail "the diff came back after leaving the tab"
do_ tab.close --id "$OTHER" --confirmed true
do_ gitview.click:src/main.rs
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$"' "a click opens the diff again"

step "uninstall gitview: herdr-nvim's layout and keys come back byte for byte"
gv_down || fail "gitview uninstall"
want "main-copy files=herdr-nvim changes=changes" "herdr-nvim's slot stays; no layout switched"
same_as nvim && ok "layout and keys.json byte-identical to before gitview's install" || fail "not byte-identical: $(diff "$BASE/nvim.layout.json" "$(LAYOUT)" | head -20)"
step "reinstall gitview, then uninstall herdr-nvim"
gv_up || fail "gitview reinstall"
nvim_down || fail "herdr-nvim uninstall"
want "main-copy files=files changes=gitview" "herdr-nvim restores its slot only; gitview's stays"
gv_down || fail "gitview uninstall"
want "main-copy files=files changes=changes" "both out: the personal layout stays, as it was"
[ -e "$HERDR_APP_HOME/layouts/main-copy.json" ] && ok "main-copy never deleted" || fail "main-copy deleted"

# --- order 2 -------------------------------------------------------------------------
step "the other order: gitview first, then herdr-nvim"
snap before2
gv_up || fail "gitview install"
want "main-copy files=files changes=gitview" "gitview edits the personal layout in place"
snap gv
nvim_up || fail "herdr-nvim install"
want "main-copy files=herdr-nvim changes=gitview" "herdr-nvim adds its slot beside gitview's"
[ "$(nvim_keys)" = "$NKEYS" ] && ok "herdr-nvim's keys bound as before" || fail "herdr-nvim's keys: $(nvim_keys)"
python3 -c 'import json,sys; g=json.load(open(sys.argv[1]))["global"]; assert g["cmd-g"]=="mod.open:gitview-tab" and g["cmd-shift-g"]=="mod.open:gitview-sidebar"' "$HERDR_APP_HOME/keys.json" && ok "gitview's keys kept" || fail "gitview's keys"
NBITS2=$(nvim_bits)
shot coexist-04-both 1.5
nvim_down || fail "herdr-nvim uninstall"
want "main-copy files=files changes=gitview" "herdr-nvim out: gitview's slot stays"
same_as gv && ok "layout and keys.json byte-identical to gitview alone" || fail "not byte-identical to gitview alone: $(diff "$BASE/gv.layout.json" "$(LAYOUT)" | head -20)"
nvim_up || fail "herdr-nvim install"
snap both
gv_down || fail "gitview uninstall"
want "main-copy files=herdr-nvim changes=changes" "gitview out first: only its slot goes"
[ "$(nvim_bits)" = "$NBITS2" ] && ok "herdr-nvim's panel width and sync controller untouched" || fail "herdr-nvim's bits changed"
[ "$(nvim_keys)" = "$NKEYS" ] && ok "herdr-nvim's keys untouched" || fail "herdr-nvim's keys changed"
python3 - "$BASE/both.layout.json" "$(LAYOUT)" "$BASE/both.keys.json" "$HERDR_APP_HOME/keys.json" <<'PY' && ok "everything else as it was: only the changes slot and gitview's keys went" || fail "gitview's uninstall changed more"
import json, sys
a, b, ka, kb = (json.load(open(p)) for p in sys.argv[1:])
def drop(n):
    if n.get("id") == "changes": n = {**n, "content": None}
    return {**n, "children": [drop(c) for c in n.get("children", [])]} if "children" in n else n
assert drop(a) == drop(b)
assert {k: v for k, v in ka["global"].items() if "gitview" not in v} == kb["global"]
PY
nvim_down || fail "herdr-nvim uninstall"
same_as before2 && ok "both out: layout and keys.json byte-identical to before both" || fail "not byte-identical to before: $(diff "$BASE/before2.layout.json" "$(LAYOUT)" | head -20)"
[ "$(app_status layout)" = main-copy ] && ok "still on main-copy: no layout was switched" || fail "layout switched"
passed
