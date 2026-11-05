#!/usr/bin/env bash
# Installing over a running app, as the person does: the mods hot-reload in
# place. Every reload keeps the old build's code in the app's JS memory for
# its life (herdr-app 0.1.13, app bug #4), so an install must reload each
# changed mod once, and an unchanged one not at all; a day of installs that
# reloaded file by file filled the person's app (every reload then failed
# with Null"null").
#
# In a private app, with the panel and a diff pane on a file:
# - an older build ($RELOAD_FROM, default 5f067b0: round 12) → HEAD: no
#   errors in `herdr-app status`, no failed reload attempt, the list and
#   the pane each rebuilt once and live (new copies, writing their trace);
# - HEAD → HEAD: nothing reloads;
# - HEAD → HEAD with one file changed in gitview: gitview reloads once,
#   gitview-diff doesn't.
#
# Usage: herdr-app/e2e/reload.sh [ARTIFACTS_DIR]
set -euo pipefail
. "$(cd "$(dirname "$0")" && pwd)/lib.sh" "${1:-/tmp/gitview-reload}"

S="$BASE/reload"
mini_repo "$S" a.txt '1\n' '2\n'
printf 'b\n' >"$S/b.txt"
TAB=$("$APP" do space.new --name reload --folder "$S" | tab_of)
FROM="${RELOAD_FROM:-5f067b0}"
OLD="$BASE/from-$FROM"
mkdir -p "$OLD"
git -C "$REPO" archive "$FROM" herdr-app | tar -x -C "$OLD"
# The archive has the repo's links as links: as a checkout would.

errors() { "$APP" status | python3 -c 'import json,sys; print("; ".join(json.load(sys.stdin)["errors"]))'; }
copies() { ls "$TRD" 2>/dev/null | grep -c -- "-$1-" || true; } # trace segments of that kind: one per copy started (a few per long-lived copy)
started() { ls "$TRD" 2>/dev/null | sed -n "s/^\([0-9]*\)-$1-.*/\1/p" | sort -n | tail -1; }
failed() { grep -ac "mod reload failed" "$BASE/app.log" || true; }
# A copy rebuilt by a reload starts a new Tracer: a segment newer than `at`.
rebuilt() { local s; s=$(started "$1"); [ -n "$s" ] && [ "$s" -gt "$2" ]; }

step "install $FROM's mods, the panel, a diff pane"
"$OLD/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "$FROM's install.sh"
expect panel_up "the Changes panel ($FROM)" 20
do_ gitview.click:a.txt
expect 'has_diff $TAB' "a diff pane" 15
sleep 1.5
[ -z "$(errors)" ] || fail "errors before: $(errors)"

step "$FROM → HEAD over the running app: each mod reloads once, live, no errors"
F0=$(failed); T0=$(now_ms)
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh"
grep -a "mods: " "$LOG" | tail -1 | tee -a "$LOG"
expect 'rebuilt list $T0 && rebuilt pane $T0' "the list and the diff pane rebuilt (new copies, writing their trace)" 10
sleep 2
[ -z "$(errors)" ] && ok "herdr-app status: no errors" || fail "herdr-app status: $(errors)"
[ "$(failed)" = "$F0" ] && ok "no reload attempt failed (none saw a half-copied mod)" || fail "$(( $(failed) - F0 )) reload attempts failed: $(grep -a 'mod reload failed' "$BASE/app.log" | tail -3)"
do_ gitview.click:b.txt
expect '[ "$(view_of $TAB path)" = b.txt ] && cat "$TRD"/*.jsonl | grep -q "\"view\":[0-9]*,\"path\":\"b.txt\""' "the new list writes the view (a line only HEAD's list writes)" 10

step "HEAD → HEAD: nothing changed, nothing reloads"
L0=$(started list); P0=$(started pane); T0=$(now_ms)
"$REPO/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh"
grep -a "mods: " "$LOG" | tail -1 | grep -q "none changed" && ok "install.sh: no mod changed" || fail "install.sh changed: $(grep -a 'mods: ' "$LOG" | tail -1)"
sleep 3
[ "$(started list)" = "$L0" ] && [ "$(started pane)" = "$P0" ] && ok "no copy rebuilt" || fail "copies rebuilt: list $L0 → $(started list), pane $P0 → $(started pane)"

step "HEAD with one changed file in gitview: gitview reloads, gitview-diff doesn't"
CH="$BASE/changed"
mkdir -p "$CH"
cp -R "$REPO/herdr-app" "$CH/" # this checkout as it is (links kept)
printf '\n// changed by reload.sh\n' >>"$CH/herdr-app/mods/gitview/main.js"
L0=$(started list); P0=$(started pane); T0=$(now_ms)
"$CH/herdr-app/install.sh" >>"$LOG" 2>&1 || fail "install.sh (changed)"
grep -a "mods: " "$LOG" | tail -1 | grep -qx ".*mods: gitview" && ok "install.sh: only gitview changed" || fail "install.sh: $(grep -a 'mods: ' "$LOG" | tail -1)"
expect 'rebuilt list $T0' "the list rebuilt" 10
sleep 2
[ "$(started pane)" = "$P0" ] && ok "the diff pane kept its copy" || fail "the diff pane rebuilt too"
[ -z "$(errors)" ] && ok "herdr-app status: no errors" || fail "herdr-app status: $(errors)"
passed
