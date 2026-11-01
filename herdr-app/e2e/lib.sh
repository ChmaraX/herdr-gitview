# Shared by the e2e scripts: a private Herdr app + herdr-core (its own app
# home, core session and XDG dirs; nothing touches yours), cleanup on exit,
# and the step/assert/screenshot helpers. Source it with the artifacts dir
# as $1; it starts the app.
#
# The window only draws while it's visible, so each screenshot raises it for
# a moment (that takes the keyboard from whatever you're typing in).
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ART="${1:-/tmp/gitview-e2e-$(date +%H%M%S)}"
mkdir -p "$ART"
ART="$(cd "$ART" && pwd)"
LOG="$ART/run.log"
: >"$LOG"

# --- a private instance --------------------------------------------------------
BASE="$(mktemp -d /tmp/gve.XXXX)"
unset HC_SOCKET HC_TERMINAL_ID HC_TERMINAL_SOCKET HC_SCOPE
unset HERDR_ENV HERDR_TAB_ID HERDR_SOCKET_PATH HERDR_BIN_PATH HERDR_WORKSPACE_ID HERDR_PANE_ID
export HERDR_APP_HOME="$BASE/home" HC_SESSION="gve$$" XDG_STATE_HOME="$BASE/state"
export XDG_CONFIG_HOME="$BASE/config" XDG_DATA_HOME="$BASE/data" HERDR_APP_NO_SOUND=1 HERDR_TELEMETRY=0
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
export HERDR_APP_BIN="$APP"
[ -n "$APP" ] || { echo "no herdr-app" >&2; exit 1; }

APP_PID=""
cleanup() {
  set +e
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null && wait "$APP_PID" 2>/dev/null
  for t in $(hc ls --json 2>/dev/null | python3 -c 'import json,sys; print(" ".join(t["address"] for t in json.load(sys.stdin)["terminals"]))' 2>/dev/null); do
    hc kill "$t" >/dev/null 2>&1
  done
  hc server stop >/dev/null 2>&1
  rm -rf "$BASE"
  echo "artifacts: $ART"
}
trap cleanup EXIT

# --- helpers ---------------------------------------------------------------------
step() { printf '\n== %s\n' "$*" | tee -a "$LOG"; }
ok() { printf 'PASS %s\n' "$*" | tee -a "$LOG"; }
fail() {
  printf 'FAIL %s\n' "$*" | tee -a "$LOG"
  { echo "--- terminals"; terms; echo "--- status"; "$APP" status
    for t in $(terms | awk '{print $1}'); do echo "--- screen $t"; screen "$t" | head -40; done
    echo "--- gitview log"; tail -40 "$XDG_STATE_HOME/herdr-gitview/app/debug.log"; echo "--- app log"; tail -20 "$BASE/app.log"
  } >>"$LOG" 2>&1 || true
  shot FAIL || true
  [ -z "${E2E_KEEP:-}" ] || { echo "kept: source $BASE/env.sh" | tee -a "$LOG"; trap - EXIT; }
  exit 1
}
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
# The dummy agent's input line / submitted messages (its own record).
agent_input() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["input"])' "$BASE/dummy.log.json"; }
agent_submitted() { python3 -c 'import json,sys; print("\n----\n".join(json.load(open(sys.argv[1]))["submitted"]))' "$BASE/dummy.log.json"; }
terms() { hc ls --json | python3 -c 'import json,sys; [print(t["address"], t.get("agent",""), t.get("parent") or "-") for t in json.load(sys.stdin)["terminals"]]'; }
fg_name() { hc get "$1" --json | python3 -c 'import json,sys; print((json.load(sys.stdin).get("foreground") or {}).get("name",""))'; }
overlay_open() { "$APP" status | python3 -c 'import json,sys; sys.exit(0 if sys.argv[1] in json.load(sys.stdin)["overlays"] else 1)' "$1"; }
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
# The file an nvim terminal shows, asked of nvim itself (its screen in a
# hidden window can lag), over its server socket.
nvim_buf() {
  local fg sock pid
  fg=$(hc get "$1" --json)
  # A remote UI (herdr-nvim) names its server; a plain nvim's is its pid's.
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
raise() {
  python3 -c "import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall(b'{\"raise\": true}\n'); s.recv(64)" \
    "$HERDR_APP_HOME/herdr-app.sock"
}
shot() { # $1 file name; raise, let a frame (and any resize) land, save
  raise
  sleep 1.5
  "$APP" shot "$ART/$1.png" >/dev/null && echo "shot $1.png" >>"$LOG"
}
# For a program that just started: its terminal gets its real size only once
# the window lays it out, then it redraws.
shot_settled() {
  raise
  sleep 2
  shot "$1"
}
panel_shows() { # the gitview panel is the shown child of the open panel
  "$APP" status >/dev/null && python3 - "$HERDR_APP_HOME" "$("$APP" status)" <<'EOF'
import json, sys
home = sys.argv[1]
name = json.loads(sys.argv[2])["layout"]
def find(n, i):
    if n.get("id") == i: return n
    for c in n.get("children", []):
        r = find(c, i)
        if r: return r
doc = json.load(open(f"{home}/layouts/{name}.json"))
assert find(doc, "changes")["content"] == {"mod": "gitview", "props": {"replaces": {"mod": "changes"}}}, "changes slot"
EOF
}


# --- start -------------------------------------------------------------------------
step "private app + core (HERDR_APP_HOME=$HERDR_APP_HOME, HC_SESSION=$HC_SESSION)"
hc server start >>"$LOG" 2>&1
"$APP" >"$BASE/app.log" 2>&1 &
APP_PID=$!
expect '"$APP" status' "the private app answers" 30
