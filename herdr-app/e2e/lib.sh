# Shared by the e2e scripts: a private Herdr app + herdr-core (its own app
# home, core session and XDG dirs; nothing touches yours), cleanup on exit,
# and the step/assert/screenshot helpers. Source it with the artifacts dir
# as $1; it starts the app.
#
# The person's own app, core, clipboard, keyboard focus and pointer are never
# touched:
# - every `hc` call runs with HC_SCOPE (and the other HC_/HERDR_ variables)
#   unset, against this run's own core session;
# - the private window is moved on screen once (macOS only draws visible
#   windows; System Events, needs Accessibility) and never raised;
#   screenshots are `herdr-app shot` (the app's last frame, no raise);
# - keys go to the private app's process only (keys.py, CGEventPostToPid);
# - no Cmd+Q: the app is ended with a signal at the end.
#
# The binary: $GITVIEW_BIN, else built once per commit from the repo's
# committed HEAD in a scratch copy ($GITVIEW_BUILD_DIR, default
# /tmp/gitview-e2e-build), so a half-edited tree never breaks a run.
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
E2E="$REPO/herdr-app/e2e"
ART="${1:-/tmp/gitview-e2e-$(date +%H%M%S)}"
mkdir -p "$ART"
ART="$(cd "$ART" && pwd)"
LOG="$ART/run.log"
: >"$LOG"

# --- a private instance --------------------------------------------------------
BASE="$(mktemp -d /tmp/gve.XXXX)"
for v in $(env | grep -oE '^(HERDR_[A-Z_]+|HC_[A-Z_]+)=' | tr -d =); do unset "$v"; done
[ -z "${HERDR_APP_BIN_E2E:-}" ] || HERDR_APP_BIN="$HERDR_APP_BIN_E2E"
export HERDR_APP_HOME="$BASE/home" HC_SESSION="gve$(printf %s "${BASE: -4}" | tr A-Z a-z)$$" XDG_STATE_HOME="$BASE/state"
export XDG_CONFIG_HOME="$BASE/config" XDG_DATA_HOME="$BASE/data" HERDR_APP_NO_SOUND=1 HERDR_TELEMETRY=0 DO_NOT_TRACK=1
export HERDR_RELEASES_URL=http://127.0.0.1:9 # never update the installed app or core
# herdr-nvim (coexist.sh): its daemons, state and agents' session logs here too.
export HERDR_NVIM_RUNTIME_DIR="$BASE/nvrun" HERDR_NVIM_STATE_DIR="$BASE/nvstate"
export PI_CODING_AGENT_DIR="$BASE/pi" CLAUDE_CONFIG_DIR="$BASE/claude"
# herdr-gitview's own debug log (json and app-setup write it): this run's.
export HERDR_PLUGIN_STATE_DIR="$BASE/gvstate"
GV_LOG="$HERDR_PLUGIN_STATE_DIR/debug.log"
export PATH="$HOME/.local/bin:/opt/homebrew/bin:$PATH"
mkdir -p "$HERDR_APP_HOME" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
echo '{"frame.welcomed": true, "layout": "main"}' >"$HERDR_APP_HOME/shared.json"
echo '{"zoom": 1, "theme": "graphite", "appearance": "dark"}' >"$HERDR_APP_HOME/settings.json"
declare -p HERDR_APP_HOME HC_SESSION XDG_STATE_HOME XDG_CONFIG_HOME XDG_DATA_HOME | sed 's/^declare -x/export/' >"$BASE/env.sh"

find_app() {
  if [ -n "${HERDR_APP_BIN:-}" ]; then echo "$HERDR_APP_BIN"; return; fi
  local c
  for c in "$(command -v herdr-app 2>/dev/null || true)" /Applications/*.app/Contents/MacOS/Herdr; do
    if [ -x "$c" ] && "$c" --version 2>/dev/null | grep -q '^herdr-app '; then echo "$c"; return; fi
  done
}
APP="$(find_app)"
[ -n "$APP" ] || { echo "no herdr-app" >&2; exit 1; }
export HERDR_APP_BIN="$APP"
APP_REAL=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$APP") # ~/.local/bin/herdr-app is a link into the bundle
version=$(plutil -extract CFBundleShortVersionString raw "${APP_REAL%/MacOS/*}/Info.plist" 2>/dev/null || true)
echo "{\"last_run\": \"$version\", \"presented\": []}" >"$HERDR_APP_HOME/update-notes.json" # no "Updated" toast

PY=
for c in python3 /usr/bin/python3 /opt/homebrew/bin/python3 /opt/homebrew/anaconda3/bin/python3; do
  if "$c" -c 'import Quartz, PIL' 2>/dev/null; then PY=$(command -v "$c"); break; fi
done
[ -n "$PY" ] || { echo "no python3 with pyobjc (Quartz) for key presses and Pillow for pixel probes" >&2; exit 1; }

APP_PID=""
cleanup() {
  set +e
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null && wait "$APP_PID" 2>/dev/null
  for t in $(hc ls --json 2>/dev/null | python3 -c 'import json,sys; print(" ".join(t["address"] for t in json.load(sys.stdin)["terminals"] if "/" not in t["address"]))' 2>/dev/null); do
    hc kill "$t" >/dev/null 2>&1
  done
  hc machine rm fake >/dev/null 2>&1
  HC_SESSION="${HC_SESSION}r" hc server stop >/dev/null 2>&1
  hc server stop >/dev/null 2>&1
  # A stopped core's holder keeps running, and any hc call restarts a core:
  # end this run's own cores and holders (matched by this run's state dir).
  for p in $(pgrep -f '(hc|herdr-core) (serve|hold)'); do
    ps eww -o command= -p "$p" 2>/dev/null | grep -q "XDG_STATE_HOME=$BASE/state " && kill "$p"
  done
  # What the run left, for checking afterwards: the layouts (and their
  # clicks) and keys.json as they ended.
  rm -rf "$ART/final" && mkdir -p "$ART/final"
  cp -R "$HERDR_APP_HOME/layouts" "$HERDR_APP_HOME/keys.json" "$ART/final/" 2>/dev/null
  # herdr-nvim's nvims of this run (their sockets are in this run's folder).
  for s in "$BASE"/nvrun/app/*.sock; do
    [ -S "$s" ] && nvim --headless --server "$s" --remote-send '<cmd>qa!<cr>' >/dev/null 2>&1
  done
  # And any of this run's nvims still running (herdr-nvim's headless
  # daemons): their environment names this run's folder.
  for p in $(pgrep -f nvim); do
    ps eww -o command= -p "$p" 2>/dev/null | grep -qE "=(/private)?${BASE//./\\.}/" && kill "$p" 2>/dev/null
  done
  cp "$GV_LOG" "$ART/gitview-debug.log" 2>/dev/null; cp "$BASE/app.log" "$ART/app.log" 2>/dev/null
  rm -rf "$BASE"
  echo "artifacts: $ART"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# --- helpers ---------------------------------------------------------------------
PASSES=0 SKIPS=0
step() { printf '\n== %s\n' "$*" | tee -a "$LOG"; }
ok() { PASSES=$((PASSES + 1)); printf 'PASS %s\n' "$*" | tee -a "$LOG"; }
skip() { SKIPS=$((SKIPS + 1)); printf 'SKIP %s\n' "$*" | tee -a "$LOG"; }
fail() {
  printf 'FAIL %s\n' "$*" | tee -a "$LOG"
  { echo "--- terminals"; terms; echo "--- status"; "$APP" status; echo "--- mod panes"; mod_panes
    echo "--- shared"; python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(json.dumps({k:v for k,v in d.items() if k.startswith("gitview")}, indent=1)[:4000])' "$HERDR_APP_HOME/shared.json"
    for t in $(terms | awk '{print $1}'); do echo "--- screen $t"; screen "$t" | head -40; done
    echo "--- gitview log"; tail -40 "$GV_LOG"; echo "--- app log"; tail -20 "$BASE/app.log"
  } >>"$LOG" 2>&1 || true
  shot FAIL || true
  [ -z "${E2E_KEEP:-}" ] || { echo "kept: source $BASE/env.sh" | tee -a "$LOG"; trap - EXIT; }
  exit 1
}
passed() { if [ "$SKIPS" -gt 0 ]; then step "all passed: $PASSES checks, $SKIPS skipped"; else step "all passed: $PASSES checks"; fi; }
do_() { "$APP" do "$@" >>"$LOG" 2>&1 || fail "herdr-app do $*"; }
# Poll a condition (a shell snippet) for up to $2 seconds (default 10).
until_() {
  local deadline=$((SECONDS + ${2:-10}))
  until eval "$1" >/dev/null 2>&1; do
    [ $SECONDS -lt $deadline ] || return 1
    sleep 0.2
  done
}
expect() { until_ "$1" "${3:-10}" && ok "$2" || fail "$2"; }
screen() { hc read "$1" --lines 0 2>/dev/null; }
# Whether macOS draws the private window now (some of it isn't covered by
# your windows; drawn.py only reads the window list). Covered, it gets no
# frames: screenshots are stale and a mod can't take the keyboard.
drawn() { "$PY" "$E2E/drawn.py" "$APP_PID"; }
# Keys to the private app's process only (keys.py: keycodes, names, text:).
keys() { "$PY" "$E2E/keys.py" "$APP_PID" "$@"; }
# The dummy agent's input line / submitted messages (its own record).
agent_input() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["input"])' "$BASE/dummy.log.json"; }
agent_submitted() { python3 -c 'import json,sys; print("\n----\n".join(json.load(open(sys.argv[1]))["submitted"]))' "$BASE/dummy.log.json"; }
terms() { hc ls --json | python3 -c 'import json,sys; [print(t["address"], t.get("agent",""), t.get("parent") or "-") for t in json.load(sys.stdin)["terminals"]]'; }
fg_name() { hc get "$1" --json | python3 -c 'import json,sys; print((json.load(sys.stdin).get("foreground") or {}).get("name",""))'; }
overlay_open() { "$APP" status | python3 -c 'import json,sys; sys.exit(0 if sys.argv[1] in json.load(sys.stdin)["overlays"] else 1)' "$1"; }
app_status() { "$APP" status | python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }
# Mod panes (the app's `ls` lists terminals only): "<tab> <pane> <mod>" lines.
mod_panes() {
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"hello","params":{"generation":1}}' \
    '{"jsonrpc":"2.0","id":2,"method":"doc.get","params":{"name":"herdr-app"}}' | hc connect | python3 -c '
import json, sys
doc = [json.loads(l) for l in sys.stdin if l.strip()][-1]["result"]["value"]
def walk(tab, n):
    if n.get("content", {}).get("mod"): print(tab, n["id"], n["content"]["mod"])
    for c in n.get("children", []): walk(tab, c)
for s in doc.get("spaces", []):
    for t in s["tabs"]: walk(t["id"], t["root"])'
}
tab_active() { "$APP" ls | python3 -c "import json,sys; d=json.load(sys.stdin); print([t['name'] for s in d['spaces'] for t in s['tabs'] if t['active'] and s['active']][0])"; }
tab_id() { "$APP" ls | python3 -c "import json,sys; d=json.load(sys.stdin); print([t['id'] for s in d['spaces'] for t in s['tabs'] if t['active'] and s['active']][0])"; }
# gitview's shared state (shared.json, saved in the background): a python
# expression over `s` (the shared dict), e.g. shared 's["gitview.views"]'.
shared() { python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); v=eval(sys.argv[2]); print(v if isinstance(v,str) else json.dumps(v))' "$HERDR_APP_HOME/shared.json" "$1" 2>/dev/null; }
# The diff pane's view of a tab: a field of shared gitview.views[<tab>].
view_of() { shared "s.get('gitview.views',{}).get('$1',{}).get('$2')"; }
# The file an nvim terminal shows, asked of nvim itself over its socket.
nvim_buf() {
  local fg sock pid
  fg=$(hc get "$1" --json)
  sock=$(python3 -c 'import json,sys; a=json.loads(sys.argv[1])["foreground"]["argv"]; print(a[a.index("--server")+1] if "--server" in a else "")' "$fg")
  if [ -z "$sock" ]; then
    pid=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["foreground"]["pid"])' "$fg")
    for p in "$pid" $(pgrep -P "$pid"); do
      sock=$(ls "${TMPDIR:-/tmp/}"nvim."$USER"/*/nvim."$p".0 2>/dev/null | head -1)
      [ -n "$sock" ] && break
    done
  fi
  [ -n "$sock" ] && nvim --server "$sock" --remote-expr 'bufname("%")'
}
status_has() { "$APP" status | grep -q "$1"; }
# $1 file name: the app's last frame (no raise); give it a moment to draw.
# macOS doesn't draw a window that's fully covered: then the frame is the
# last one drawn, and the log says so (the checks never depend on drawing).
LAST_SHOT=
shot() {
  sleep "${2:-0.8}"
  "$APP" shot "$ART/$1.png" >/dev/null || return 0
  local sum
  sum=$(shasum "$ART/$1.png" | cut -c1-40)
  if ! drawn; then
    echo "shot $1.png (stale: the window is covered by another window; not raising it)" | tee -a "$LOG"
  elif [ "$sum" = "$LAST_SHOT" ]; then
    echo "shot $1.png (same frame as the one before)" | tee -a "$LOG"
  else
    echo "shot $1.png" >>"$LOG"
  fi
  LAST_SHOT=$sum
}
# The node of the shown layout by id, as JSON.
layout_node() {
  python3 - "$HERDR_APP_HOME" "$(app_status layout)" "$1" <<'EOF'
import json, sys
home, name, want = sys.argv[1:]
def find(n):
    if n.get("id") == want: return n
    for c in n.get("children", []):
        r = find(c)
        if r: return r
print(json.dumps(find(json.load(open(f"{home}/layouts/{name}.json")))))
EOF
}
panel_slot() { layout_node changes | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["content"], sort_keys=True))'; }
panel_shows() { [ "$(panel_slot)" = '{"mod": "gitview", "props": {"replaces": {"mod": "changes"}}}' ]; }
# The side panel's clicks (shown/hidden, the stack's child), as the app
# keeps them in layouts/.state: "open changes", "hidden files", ...
panel_state() {
  python3 - "$HERDR_APP_HOME" "$(app_status layout)" <<'EOF'
import json, sys
home, name = sys.argv[1:]
try:
    n = json.load(open(f"{home}/layouts/.state/{name}.json")).get("nodes", {})
except FileNotFoundError:
    n = {}
def find(node, i):
    if node.get("id") == i: return node
    for c in node.get("children", []):
        r = find(c, i)
        if r: return r
    return {}
# A click equal to the file's own value isn't kept: the file decides then.
doc = json.load(open(f"{home}/layouts/{name}.json"))
hidden = n.get("panel", {}).get("hidden", find(doc, "panel").get("hidden", False))
active = n.get("panel-body", {}).get("active", find(doc, "panel-body").get("active"))
print("hidden" if hidden else "open", active)
EOF
}
# The panel up on gitview, and gitview answering. Again until it holds: the
# stock tab bar restores a space's own panel choice (none, for a new one)
# when it first draws, which in a covered window can be any time.
panel_up() {
  "$APP" do layout.select '{"stack":"panel-body","child":"changes"}' >>"$LOG" 2>&1 &&
    "$APP" do layout.show --id panel >>"$LOG" 2>&1 && sleep 0.3 && [ "$(panel_state)" = "open changes" ] &&
    "$APP" do gitview.key:r >>"$LOG" 2>&1
}
changes_button() { # exactly what the tab bar's Changes button runs
  do_ layout.select '{"stack":"panel-body","child":"changes"}'
  do_ layout.show --id panel
}

# --- the binary, from committed HEAD ---------------------------------------------
if [ -z "${GITVIEW_BIN:-}" ]; then
  sha=$(git -C "$REPO" rev-parse --short HEAD)
  build="${GITVIEW_BUILD_DIR:-/tmp/gitview-e2e-build}"
  if [ ! -x "$build/bin-$sha" ]; then
    echo "building herdr-gitview from HEAD ($sha) in $build" | tee -a "$LOG"
    rm -rf "$build/src-tree" && mkdir -p "$build/src-tree"
    git -C "$REPO" archive HEAD | tar -x -C "$build/src-tree"
    (cd "$build/src-tree" && CARGO_TARGET_DIR="$build/target" cargo build --release --quiet) >>"$LOG" 2>&1 || { echo "build failed" >&2; exit 1; }
    cp "$build/target/release/herdr-gitview" "$build/bin-$sha"
  fi
  export GITVIEW_BIN="$build/bin-$sha"
fi

# The discard confirm runs `<mod>.discard:<one-time token>`, and the mod
# logs the token when it asks: run what the dialog's Confirm runs ($1: the
# mod), then close the dialog.
LAST_CONFIRM=
last_confirm() { grep -a -o "$1\.discard:[a-z0-9]*" "$BASE/app.log" | tail -1; }
confirm_discard() {
  until_ '[ -n "$(last_confirm $1)" ] && [ "$(last_confirm $1)" != "$LAST_CONFIRM" ]' 5 || fail "no discard confirm from $1"
  LAST_CONFIRM=$(last_confirm "$1")
  do_ "$LAST_CONFIRM"
  do_ overlay.close
}

# --- start -------------------------------------------------------------------------
step "private app + core (HERDR_APP_HOME=$HERDR_APP_HOME, HC_SESSION=$HC_SESSION, binary $GITVIEW_BIN)"
{ echo "herdr-app: $("$APP" --version 2>&1 | head -1) ($APP)"; echo "hc: $(hc --version 2>&1 | head -1) ($(command -v hc))"
  echo "gitview: $(git -C "$REPO" rev-parse --short HEAD) ($GITVIEW_BIN)"; echo "macOS: $(sw_vers -productVersion 2>/dev/null)"; } | tee -a "$LOG"
hc server start >>"$LOG" 2>&1
"$APP" >"$BASE/app.log" 2>&1 &
APP_PID=$!
expect '"$APP" status' "the private app answers" 30
place_window() {
  osascript -e "tell application \"System Events\" to tell (first process whose unix id is $APP_PID) to set position of window 1 to {40, 60}" \
    -e "tell application \"System Events\" to tell (first process whose unix id is $APP_PID) to set size of window 1 to {1280, 820}"
}
until_ place_window 15 || echo "note: could not move the window (Accessibility); screenshots may be stale" | tee -a "$LOG"
