#!/usr/bin/env bash
# How gitview looks while it works: a realistic repo (a clone of this one with
# a dozen assorted changes) driven through everyday actions by the mods'
# commands, in a private Herdr app. For each action it records the mods'
# render trace (GITVIEW_DEBUG: what each copy draws, a line per change) and
# every `herdr-gitview json` run (the installed binary is wrapped to log
# them), then measures with jank.py: blank diff bodies, "Loading…" and "No
# file selected" flashes, Diff/Preview flips, spinner blinks, header
# changes, a stale file at the end, the execs. With E2E_FOREGROUND=1 it also
# records the distinct frames (`herdr-app shot` in a loop) as contact
# sheets. Writes jank.md (a table) and jank.json into the artifacts.
#
#   herdr-app/e2e/jank.sh [artifacts dir]
#   JANK_ASSERT=0  record only, no budgets (a baseline)
#   HERDR_NVIM_SRC=<herdr-nvim checkout>  also Enter → herdr-nvim → quit
#   E2E_FOREGROUND=1  start the private app in the foreground (it comes to
#     the front once): a background-started window doesn't draw on its own,
#     so without it only the execs are measured and the frame budgets skip
#
# The budgets (JANK_ASSERT=1, the default): no blank diff body on j/k, s, u,
# discard, commit or an agent's write, and none back from nvim but the new
# pane's first file (a new mod copy has nothing to show before it); at most one
# status and one diff per s/u/discard; no diff for an agent's write to
# another file; no blank body on `t`; back from history the pane shows the
# worktree's file.
set -euo pipefail
export GITVIEW_DEBUG=1 # the mods' render trace (GVTRACE lines in the app's log)
HNV_SRC="${HERDR_NVIM_SRC:-}" # read before lib.sh clears HERDR_* variables
JANK_ASSERT="${JANK_ASSERT:-1}"
. "$(dirname "$0")/lib.sh"

if [ -n "$HNV_SRC" ]; then
  HNV="${GITVIEW_BUILD_DIR:-/tmp/gitview-e2e-build}/herdr-nvim-$(git -C "$HNV_SRC" rev-parse --short HEAD)"
  [ -d "$HNV" ] || { mkdir -p "$HNV" && git -C "$HNV_SRC" archive HEAD | tar -x -C "$HNV"; }
  (cd "$HNV" && herdr-app/install.sh --app-bin "$APP") >>"$LOG" 2>&1 || fail "herdr-nvim install"
fi

# --- the repo -------------------------------------------------------------------
R="$BASE/proj"
git clone -q --no-hardlinks "$REPO" "$R"
git -C "$R" config user.email e2e@test && git -C "$R" config user.name e2e
(
  cd "$R"
  printf '\n// a tail comment\n' >>src/lib.rs
  python3 - <<'PY'
p = "src/appjson.rs"; s = open(p).read(); open(p, "w").write(s.replace("fn ", "// touched\nfn ", 3))
PY
  printf '# Notes\n\nSome **bold** text and a list:\n\n- one\n- two\n' >NOTES.md
  mkdir -p assets
  "$PY" -c "
from PIL import Image, ImageDraw
im = Image.new('RGB', (320, 200), (40, 120, 200)); ImageDraw.Draw(im).ellipse((80, 40, 240, 160), fill=(250, 250, 250)); im.save('assets/logo.png')"
  printf 'staged line\n' >>README.md && git add README.md && printf 'unstaged too\n' >>README.md
  git rm -q --cached Cargo.lock
  git mv -f herdr-app/RECIPE.md RECIPE-moved.md
  git rm -q herdr-app/keys.linux.json
  for i in 1 2 3; do printf 'scratch %s\n' "$i" >"untracked-$i.txt"; done
)
MADE=$("$APP" do space.new --name proj --folder "$R")
TAB=$(echo "$MADE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tab"])')

step "install, and the binary wrapped to log every json run"
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh"
BIN="$HERDR_APP_HOME/mods/gitview/bin/herdr-gitview"
EXECS="$BASE/execs.log"
: >"$EXECS"
mv "$BIN" "$BIN.real"
cat >"$BIN" <<SH
#!/bin/sh
s=\$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
"$BIN.real" "\$@"; c=\$?
printf '%s %s %s %s\n' "\$s" "\$(perl -MTime::HiRes=time -e 'printf "%.3f", time')" "\$c" "\$*" >>"$EXECS"
exit \$c
SH
chmod +x "$BIN"
ok "installed; json runs go to $EXECS"

# --- recording --------------------------------------------------------------------
J="$ART/jank" && mkdir -p "$J"
RESULTS="$J/results.jsonl" && : >"$RESULTS"
recorder() { # dir seconds: distinct frames, named by ms since the start
  local out=$1 secs=$2 t0 now last="" sum f
  t0=$(perl -MTime::HiRes=time -e 'printf "%d", time*1000')
  while :; do
    now=$(perl -MTime::HiRes=time -e 'printf "%d", time*1000')
    [ $((now - t0)) -lt $((secs * 1000)) ] || break
    f="$out/f-$(printf %05d $((now - t0))).png"
    "$APP" shot "$f" >/dev/null 2>&1 || continue
    sum=$(md5 -q "$f")
    if [ "$sum" = "$last" ]; then rm -f "$f"; else last=$sum; fi
  done
}
# act NAME SECONDS CMD...: record while CMD runs and for SECONDS.
act() {
  local name=$1 secs=$2; shift 2
  local d="$J/$name" mark tmark rec=""
  rm -rf "$d" && mkdir -p "$d"
  mark=$(wc -l <"$EXECS")
  tmark=$(wc -l <"$BASE/app.log")
  if [ "$FRAMES" = 1 ]; then recorder "$d" "$secs" & rec=$!; fi
  sleep 0.3
  eval "$@" >>"$LOG" 2>&1 || true
  if [ -n "$rec" ]; then wait "$rec"; else sleep "$secs"; fi
  tail -n +"$((mark + 1))" "$EXECS" >"$d/execs.log"
  tail -n +"$((tmark + 1))" "$BASE/app.log" | grep -a GVTRACE >"$d/trace.log" || true
  local m
  m=$("$PY" "$E2E/jank.py" "$d")
  echo "{\"action\": \"$name\", \"m\": $m}" >>"$RESULTS"
  echo "$name: $m" | tee -a "$LOG"
  "$PY" - "$d" "$J/$name.png" <<'PY' || true
import os, sys
from PIL import Image, ImageDraw
d, out = sys.argv[1:]
fs = sorted(f for f in os.listdir(d) if f.endswith(".png"))
if not fs: sys.exit()
ims = [Image.open(os.path.join(d, f)).convert("RGB") for f in fs]
w, h = ims[0].size
tw, th = w // 3, h // 3
cols = 4
sheet = Image.new("RGB", (cols * tw, ((len(ims) + cols - 1) // cols) * (th + 16)), (255, 255, 255))
dr = ImageDraw.Draw(sheet)
for i, (f, im) in enumerate(zip(fs, ims)):
    x, y = (i % cols) * tw, (i // cols) * (th + 16)
    sheet.paste(im.resize((tw, th)), (x, y + 16)); dr.text((x + 3, y + 2), f, fill=(200, 0, 0))
sheet.save(out)
PY
}
m_of() { python3 -c 'import json,sys; [print(json.dumps(r["m"])) for r in map(json.loads, open(sys.argv[1])) if r["action"] == sys.argv[2]]' "$RESULTS" "$1" | tail -1; }
field() { m_of "$1" | python3 -c "import json,sys; m=json.load(sys.stdin); print(eval(sys.argv[1]))" "$2"; }
budget() { # name python-expression-over-m what
  if [ "$JANK_ASSERT" = 1 ]; then
    [ "$(field "$1" "$2")" = True ] && ok "$1: $3" || fail "$1: $3 ($(m_of "$1"))"
  fi
}

# --- the actions -------------------------------------------------------------------
# Frames only with E2E_FOREGROUND: a background-started window doesn't draw.
FRAMES=0
[ -n "${E2E_FOREGROUND:-}" ] && FRAMES=1

step "actions"
expect panel_up "the panel is up" 20
act panel 3 'do_ gitview.key:r'
expect 'mod_panes | grep -q "^$TAB .* gitview-diff$"' "the diff pane is open"
act click 3 'do_ gitview.click:src/appjson.rs'
act j3 3 'do_ gitview.key:j; sleep 0.15; do_ gitview.key:j; sleep 0.15; do_ gitview.key:j'
do_ gitview.click:src/lib.rs
sleep 1
act stage 3 'do_ gitview.key:s'
act unstage 3 'do_ gitview.key:u'
act discard 3 'do_ gitview.button:discard:untracked-3.txt; sleep 0.6; confirm_discard gitview'
do_ gitview.click:src/lib.rs
sleep 1
act commit 4 'do_ gitview.button:stage:NOTES.md; sleep 0.8; do_ "gitview.commit:jank: commit notes"'
do_ gitview.click:src/lib.rs
sleep 1
act write-shown 3 'printf "// an agent line\n" >>"$R/src/lib.rs"'
act write-other 3 'printf "more\n" >>"$R/untracked-1.txt"'
act history 5 'do_ gitview.key:l; sleep 0.8; do_ gitview.key:enter; sleep 0.8; do_ gitview.key:escape; sleep 0.5; do_ gitview.key:escape'
HCOMMIT=$(view_of "$TAB" commit)
act tree 3 'do_ gitview.key:t'
do_ gitview.key:t
if [ -n "$HNV_SRC" ]; then
  do_ gitview.click:src/lib.rs
  sleep 1
  act nvim-open 4 'do_ gitview-diff.key:enter'
  NVT=$(hc ls --json --label "herdr-nvim.sidebar=$TAB" | python3 -c 'import json,sys; t=[t["address"] for t in json.load(sys.stdin)["terminals"] if t["state"] == "live"]; print(t[0] if t else "")')
  if [ -n "$NVT" ]; then
    act nvim-quit 4 "hc keys $NVT esc shift+z shift+q"
  else
    echo "note: no herdr-nvim pane after Enter" | tee -a "$LOG"
  fi
fi

# --- the table ---------------------------------------------------------------------
python3 - "$RESULTS" "$J/jank.md" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
out = ["| action | blank body | Loading… | No file selected | mode flips | spinner blinks | header changes | stale at end | execs | frames |",
       "|---|---|---|---|---|---|---|---|---|---|"]
for r in rows:
    m = r["m"]
    ex = ", ".join(f"{k} {v}" for k, v in sorted(m["execs"].items())) or "-"
    fr = f"{m['frames']} (blank {m['blank']}, cards {m['cards']}, widths {' → '.join(map(str, m['terminal_widths']))})" if m["frames"] else "-"
    out.append(f"| {r['action']} | {m['trace_blank']} | {m['loading']} | {m['no_file']} | {m['mode_flips']} | {m['spinner_blinks']} | {m['trace_header_changes']} | {'yes' if m['stale_end'] else ''} | {ex} | {fr} |")
open(sys.argv[2], "w").write("\n".join(out) + "\n")
print("\n".join(out))
PY
cp "$RESULTS" "$J/jank.json"

# --- budgets -----------------------------------------------------------------------
if [ "$JANK_ASSERT" = 1 ]; then
  step "budgets"
  for a in j3 stage unstage discard commit write-shown write-other; do budget "$a" 'm["trace_blank"] == 0 and m["blank"] == 0' "no blank diff body"; done
  for a in stage unstage discard; do
    budget "$a" 'm["execs"].get("status", 0) <= 1 and m["execs"].get("diff", 0) <= 1' "at most one status and one diff"
  done
  budget write-other 'm["execs"].get("diff", 0) == 0' "no diff for a write to another file"
  budget tree 'm["trace_blank"] == 0 and m["no_file"] == 0' "no blank body, no \"No file selected\""
  [ "$HCOMMIT" = null ] && ok "history: back to files, the pane shows the worktree's file" || fail "history: the pane still shows commit $HCOMMIT"
  if [ -n "$HNV_SRC" ] && [ -n "${NVT:-}" ]; then budget nvim-quit 'm["trace_blank"] <= 1' "coming back from nvim, no blank body after the new pane's first file (a new mod copy starts empty: nothing crosses copies)"; fi
fi
passed
