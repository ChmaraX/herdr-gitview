#!/usr/bin/env bash
# The install a person runs: the root install.sh with a release's app
# bundle (herdr-app/bundle.sh, as the release workflow packs it), into a
# private app, with no clone and no build. What can go wrong:
# - a bundle that doesn't match its SHA-256 gets installed anyway;
# - the bundle misses a file the mods need (a linked one, an icon), so a mod
#   fails `herdr-app check` or doesn't load;
# - the install builds, or runs the clone's binary, instead of the bundle's;
# - running it again reloads mods that didn't change;
# - the uninstall leaves a mod, a key or a layout edit behind.
#
# Usage: herdr-app/e2e/package.sh [ARTIFACTS_DIR]
set -euo pipefail
. "$(cd "$(dirname "$0")" && pwd)/lib.sh" "${1:-/tmp/gitview-package}"

errors() { "$APP" status | python3 -c 'import json,sys; print("; ".join(json.load(sys.stdin)["errors"]))'; }
ours() { ls -d "$HERDR_APP_HOME"/mods/gitview* 2>/dev/null | wc -l | tr -d ' '; }
in_layouts() { grep -l '"gitview' "$HERDR_APP_HOME"/layouts/*.json 2>/dev/null | wc -l | tr -d ' '; }
# Not the clone: the installer must use what the bundle carries.
root_install() { (cd "$BASE" && env -u GITVIEW_BIN sh "$REPO/install.sh" --app "$@") >>"$LOG" 2>&1; }

step "the bundle, packed as the release packs it"
B="$BASE/dist"
"$REPO/herdr-app/bundle.sh" "$GITVIEW_BIN" test-target "$B" >>"$LOG" 2>&1 || fail "bundle.sh"
TGZ="$B/herdr-gitview-app-test-target.tar.gz"
[ -f "$TGZ" ] && [ -f "$TGZ.sha256" ] && ok "bundle and its .sha256 ($(du -h "$TGZ" | cut -f1 | tr -d ' '))" || fail "no bundle in $B"
L=$(tar -tzf "$TGZ")
echo "$L" | grep -q '/bin/herdr-gitview$' && echo "$L" | grep -q '/herdr-app/install.sh$' && echo "$L" | grep -q '/herdr-app/mods/gitview-diff/main.js$' \
  && ! echo "$L" | grep -qE '/(e2e|test|tools)/|jsconfig.json|\.d\.ts$' && ok "it holds the binary, the installer and the mods; no tests, tools or editor files" || fail "bundle contents: $(echo "$L" | head -20)"
cp "$HERDR_APP_HOME/keys.json" "$BASE/keys.before.json" 2>/dev/null || : >"$BASE/keys.before.json"

step "a bundle that doesn't match its SHA-256 is refused"
mkdir -p "$BASE/bad"
cp "$TGZ" "$BASE/bad/x.tar.gz" && printf 'x' >>"$BASE/bad/x.tar.gz" && cp "$TGZ.sha256" "$BASE/bad/x.tar.gz.sha256"
if root_install --bundle "$BASE/bad/x.tar.gz"; then fail "the tampered bundle installed"; fi
[ "$(ours)" = 0 ] && ok "refused, nothing installed" || fail "$(ours) gitview mods after the refusal"

step "install from the bundle"
root_install --bundle "$TGZ" || fail "install.sh --app --bundle"
expect panel_up "the Changes panel shows gitview" 20
[ -z "$(errors)" ] && ok "herdr-app status: no errors" || fail "herdr-app status: $(errors)"
X=$(mktemp -d) && tar -xzf "$TGZ" -C "$X"
cmp -s "$X"/herdr-gitview-app-*/bin/herdr-gitview "$HERDR_APP_HOME/mods/gitview/bin/herdr-gitview" && ok "the installed binary is the bundle's (no build)" || fail "the installed binary isn't the bundle's"
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$HERDR_APP_HOME/mods/gitview/config.json" && ok "config.json is valid JSON" || fail "config.json: $(cat "$HERDR_APP_HOME/mods/gitview/config.json")"
S="$BASE/pkg"
mini_repo "$S" a.txt '1\n' '2\n'
TAB=$("$APP" do space.new --name pkg --folder "$S" | tab_of)
do_ gitview.click:a.txt
expect 'has_diff $TAB && [ "$(view_of $TAB path)" = a.txt ]' "a click shows the diff (the bundle's mods and binary work)" 15

step "again: an upgrade with nothing new reloads nothing"
root_install --bundle "$TGZ" || fail "install.sh again"
grep -a "mods: " "$LOG" | tail -1 | grep -q "none changed" && ok "no mod changed" || fail "$(grep -a 'mods: ' "$LOG" | tail -1)"

step "uninstall"
root_install --bundle "$TGZ" --uninstall || fail "install.sh --uninstall"
[ "$(ours)" = 0 ] && ok "no gitview mod left" || fail "left: $(ls -d "$HERDR_APP_HOME"/mods/gitview*)"
cmp -s "$HERDR_APP_HOME/keys.json" "$BASE/keys.before.json" && ok "keys.json byte-identical to before" || fail "keys.json differs: $(diff "$BASE/keys.before.json" "$HERDR_APP_HOME/keys.json" | head)"
[ "$(in_layouts)" = 0 ] && ok "no layout names a gitview mod" || fail "layouts naming gitview: $(grep -l '"gitview' "$HERDR_APP_HOME"/layouts/*.json)"
passed
