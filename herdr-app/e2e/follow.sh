#!/usr/bin/env bash
# The person's case (round 13): j/k through the list at a person's pace, and
# the diff pane follows every move. In a private Herdr app, as the person
# runs it (no debug mode), in the background; checked from the mods' trace.
#
# 40 moves with pauses of 0.1–3 s (a fixed seed: the same run each time),
# several long ones, a burst of 10 with no pause, and a few clicks. Then,
# from the list's view writes and the pane's diff execs and draw lines:
# - every settled move (no other within 300 ms) starts its file's diff
#   within 150 ms;
# - the burst runs at most one diff at a time and no more than it has
#   moves, and its last file wins;
# - the pane ends on the list's file, drawn (not blank, not the old one);
# - the body is never blank once it showed a diff.
# Writes follow.json (each move: its file, when its diff started) and
# follow.md into the artifacts.
#
# Usage: herdr-app/e2e/follow.sh [ARTIFACTS_DIR]
set -euo pipefail
. "$(cd "$(dirname "$0")" && pwd)/lib.sh" "${1:-/tmp/gitview-follow}"

S="$BASE/follow"
mkdir -p "$S"
mini_repo "$S" a.txt '1\n' '2\n'
for i in $(seq -w 1 15); do printf 'file %s\n%s\n' "$i" "$(seq 1 "$((i * 3))")" >"$S/f$i.txt"; done
TAB=$("$APP" do space.new --name follow --folder "$S" | tab_of)
step "install, the panel, the first file in the diff pane"
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1
expect panel_up "the Changes panel" 20
do_ gitview.click:f05.txt
expect 'has_diff $TAB && [ "$(view_of $TAB path)" = f05.txt ]' "f05.txt in the diff pane" 15
sleep 1.5

step "40 moves at a person's pace, a burst of 10, a few clicks"
RANDOM=13
row=5 # f05; the rows are a.txt, f01..f15 (a.txt first or last: either way 1..15 stay inside)
move() { # j or k, kept inside f02..f14 so every move is to another file
  local k=j
  if [ $row -ge 14 ]; then k=k; elif [ $row -le 2 ]; then k=j; elif [ $((RANDOM % 3)) = 0 ]; then k=k; fi
  [ $k = j ] && row=$((row + 1)) || row=$((row - 1))
  "$APP" do gitview.key:$k >/dev/null
}
pause() { python3 -c "import time; time.sleep($1)"; }
T0=$(now_ms)
for n in $(seq 1 15); do
  move
  case $n in 4|9|13) pause 2.6 ;; *) pause "$(python3 -c "import random; random.seed($n); print(round(random.uniform(0.1, 1.4), 2))")" ;; esac
done
BURST0=$(now_ms)
for n in $(seq 1 10); do move; done
BURST1=$(now_ms)
pause 2
for n in $(seq 16 30); do
  move
  case $n in 18|24|29) pause 3 ;; *) pause "$(python3 -c "import random; random.seed($n); print(round(random.uniform(0.1, 1.2), 2))")" ;; esac
done
for f in f03.txt f12.txt f07.txt; do do_ gitview.click:$f; pause 1.1; done
sleep 1.5 # the copies write their lines once a second

python3 - "$TRD" "$TAB" "$T0" "$BURST0" "$BURST1" "$(view_of "$TAB" path)" "$ART" <<'PY' >>"$LOG" 2>&1 || true
import glob, json, os, re, sys
trd, tab, t0, b0, b1, final, art = sys.argv[1], sys.argv[2], *map(int, sys.argv[3:6]), sys.argv[6], sys.argv[7]
L = []
for f in glob.glob(os.path.join(trd, "*.jsonl")):
    for l in open(f, errors="ignore"):
        try:
            L.append(json.loads(l))
        except ValueError:
            pass
L.sort(key=lambda d: d["t"])
writes = [d for d in L if d["mod"] == "list" and d.get("tab") == tab and "view" in d and d["t"] >= t0]
execs = []
for d in L:
    if d["mod"] == "pane" and d["kind"] == "exec" and d.get("op") == "diff" and d["t"] >= t0:
        m = re.search(r"--path (\S+)", d.get("args", ""))
        execs.append({"start": d["t"] - d["ms"], "end": d["t"], "path": m and m.group(1)})
draws = [d for d in L if d["mod"] == "pane" and d["kind"] == "draw" and d.get("tab") == tab and d["t"] >= t0]
rows, bad = [], []
for i, w in enumerate(writes):
    nxt = writes[i + 1]["t"] if i + 1 < len(writes) else None
    settled = nxt is None or nxt - w["t"] >= 300
    e = next((e for e in execs if e["path"] == w["path"] and e["start"] >= w["t"] - 5), None)
    lag = e["start"] - w["t"] if e else None
    rows.append({"t": w["t"], "path": w["path"], "settled": settled, "burst": b0 <= w["t"] <= b1 + 300, "diff_started_ms": lag})
    if settled and (lag is None or lag > 150):
        bad.append(f"{w['path']} (seq {w['view']}): its diff {'never started' if lag is None else f'started {lag} ms after the move'}")
burst_w = [r for r in rows if r["burst"]]
burst_e = [e for e in execs if b0 <= e["start"] <= (burst_w[-1]["t"] + 1000 if burst_w else b1)]
overlap = sum(1 for a, b in zip(burst_e, burst_e[1:]) if b["start"] < a["end"] - 1)
last = draws[-1] if draws else {}
first_content = next((i for i, d in enumerate(draws) if d.get("body") == "content"), None)
blanks = [d for d in draws[first_content or 0:] if d.get("body") == "blank"] if first_content is not None else []
res = {
    "moves": len(writes), "settled": sum(r["settled"] for r in rows),
    "late_or_missed": bad,
    "settled_diff_start_ms": sorted(r["diff_started_ms"] for r in rows if r["settled"] and r["diff_started_ms"] is not None),
    "burst": {"moves": len(burst_w), "diffs": len(burst_e), "overlapping": overlap, "last_move": burst_w[-1]["path"] if burst_w else None, "last_diff": burst_e[-1]["path"] if burst_e else None},
    "final": {"list": writes[-1]["path"] if writes else None, "shared": final, "pane_draws": last.get("file"), "body": last.get("body"), "old": last.get("old")},
    "blank_bodies": len(blanks),
    "rows": rows,
}
json.dump(res, open(os.path.join(art, "follow.json"), "w"), indent=1)
s = res["settled_diff_start_ms"]
with open(os.path.join(art, "follow.md"), "w") as f:
    f.write(f"| moves | settled | late/missed | diff start p50 / p95 / max (ms) | burst moves / diffs / overlapping | blank bodies |\n|---|---|---|---|---|---|\n")
    f.write(f"| {res['moves']} | {res['settled']} | {len(bad)} | {s[len(s)//2] if s else '-'} / {s[int(len(s)*0.95)] if s else '-'} / {s[-1] if s else '-'} | {len(burst_w)} / {len(burst_e)} / {overlap} | {len(blanks)} |\n")
    for b in bad:
        f.write(f"- {b}\n")
print(json.dumps({k: v for k, v in res.items() if k != "rows"}))
PY
R="$ART/follow.json"
[ -f "$R" ] || fail "no follow.json (the trace couldn't be read: $(tail -3 "$LOG"))"
j() { python3 -c "import json,sys; r=json.load(open('$R')); print($1)"; }
echo "$(cat "$ART/follow.md")" >>"$LOG"
[ "$(j 'r["moves"] >= 40')" = True ] && ok "the list wrote $(j 'r["moves"]') views ($(j 'r["settled"]') settled)" || fail "the list wrote only $(j 'r["moves"]') views"
[ "$(j 'len(r["late_or_missed"])')" = 0 ] && ok "every settled move's diff started within 150 ms (p50/max $(j 'r["settled_diff_start_ms"][len(r["settled_diff_start_ms"])//2]')/$(j 'r["settled_diff_start_ms"][-1]') ms)" \
  || fail "$(j 'len(r["late_or_missed"])') of $(j 'r["settled"]') settled moves late or missed: $(j '"; ".join(r["late_or_missed"][:6])')"
[ "$(j 'r["burst"]["overlapping"] == 0 and r["burst"]["diffs"] <= r["burst"]["moves"] and r["burst"]["last_diff"] == r["burst"]["last_move"]')" = True ] \
  && ok "the burst: $(j 'r["burst"]["moves"]') moves, $(j 'r["burst"]["diffs"]') diffs one at a time, the last file won" || fail "the burst: $(j 'r["burst"]')"
[ "$(j 'r["final"]["list"] == r["final"]["shared"] and (r["final"]["pane_draws"] or "").endswith(":" + r["final"]["list"]) and r["final"]["body"] == "content" and not r["final"]["old"]')" = True ] \
  && ok "the pane ends on the list's file, drawn ($(j 'r["final"]["list"]'))" || fail "the end: $(j 'r["final"]')"
[ "$(j 'r["blank_bodies"]')" = 0 ] && ok "the body never went blank" || fail "$(j 'r["blank_bodies"]') blank bodies"
passed
