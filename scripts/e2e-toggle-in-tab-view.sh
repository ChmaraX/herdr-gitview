#!/usr/bin/env bash
# End-to-end check for issue #10: the sidebar toggle pressed inside the
# dedicated gitview tab must close that view, never nest a sidebar view in it.
# Drives the real orchestrator against a live herdr session, using a
# throwaway repo, a throwaway state dir and a scratch "host" tab — your tabs
# and working tree are not touched (focus will jump while it runs).
#
# Failure modes covered:
#   A. toggle inside the tab view nests a sidebar      → view must close
#   B. open (sidebar) inside the tab view nests too    → must be a no-op
#   C. toggle in a normal tab while a tab view exists  → sidebar opens there,
#      tab view untouched (the fix must not close views from elsewhere)
#   D. toggle-tab routing: from another tab focuses the view, from inside
#      it closes the view
#
# Requires: a running herdr session with this checkout linked as the plugin
# and a release build. Run from inside herdr:  scripts/e2e-toggle-in-tab-view.sh
# Artifact: one log per source revision (commit + hash of uncommitted src
# changes) with herdr tab/state snapshots and gitview's own debug.log, at
# /tmp/gitview-e2e-toggle-in-tab-view-<rev>.log (override with $ARTIFACT).
set -euo pipefail

herdr="${HERDR_BIN_PATH:-herdr}"
root="$(cd "$(dirname "$0")/.." && pwd)"
gitview="${GITVIEW_BIN:-$root/target/release/herdr-gitview}"
ws="${HERDR_WORKSPACE_ID:?run inside a herdr session}"
rev="$(git -C "$root" rev-parse --short HEAD)"
if ! git -C "$root" diff --quiet HEAD -- src; then
  rev="$rev+$(git -C "$root" diff HEAD -- src | shasum | cut -c1-7)"
fi
ARTIFACT="${ARTIFACT:-/tmp/gitview-e2e-toggle-in-tab-view-$rev.log}"
{
  echo "rev: $rev"
  echo "binary: $gitview ($(shasum "$gitview" | cut -c1-12), built $(date -r "$gitview" '+%F %T'))"
  echo "herdr: $("$herdr" --version) workspace: $ws"
} >"$ARTIFACT"

repo="$(mktemp -d)"
export HERDR_PLUGIN_STATE_DIR="$(mktemp -d)"
export GITVIEW_REPO="$repo"
views="$HERDR_PLUGIN_STATE_DIR/views"
host_tab=""

cleanup() {
  "$gitview" close >/dev/null 2>&1 || true
  [ -n "$host_tab" ] && "$herdr" tab close "$host_tab" >/dev/null 2>&1 || true
  # Keep gitview's own decisions ("closing view …") in the artifact.
  { echo "--- gitview debug.log"; cat "$HERDR_PLUGIN_STATE_DIR/debug.log" 2>/dev/null; } >>"$ARTIFACT"
  rm -rf "$repo" "$HERDR_PLUGIN_STATE_DIR"
}
trap cleanup EXIT

git -C "$repo" init -q -b main
git -C "$repo" config user.email e2e@test
git -C "$repo" config user.name e2e
printf 'base\n' >"$repo/base.txt"
git -C "$repo" add . && git -C "$repo" commit -qm init
printf 'change\n' >>"$repo/base.txt"

log() { echo "$*" | tee -a "$ARTIFACT"; }
die() {
  log "FAIL: $*"
  log "artifact: $ARTIFACT"
  exit 1
}
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
alive() { "$herdr" pane get "$1" >/dev/null 2>&1; }
tab_count() { "$herdr" tab list --workspace "$ws" | json "len(d['result']['tabs'])"; }
tab_state() { json "d['$1']" <"$views"/*.tab.json; }
sidebar_states() { find "$views" -name '*.json' ! -name '*.tab.json' ! -name '*.recover.json' | wc -l | tr -d ' '; }
snapshot() {
  {
    echo "--- $1"
    "$herdr" tab list --workspace "$ws" | json "[t['label'] for t in d['result']['tabs']]"
    ls "$views" 2>/dev/null | grep -v '\.sock$' || true
  } >>"$ARTIFACT"
}
# Run an action as herdr would when invoked from <tab> with <pane> focused.
invoke() { # $1 action, $2 tab, $3 pane
  HERDR_PLUGIN_CONTEXT_JSON="{\"workspace_id\":\"$ws\",\"tab_id\":\"$2\",\"focused_pane_id\":\"$3\"}" \
    "$gitview" "$1" >>"$ARTIFACT" 2>&1
}

log "== setup: scratch host tab in $ws"
# Focused: herdr places new view tabs in the focused workspace.
reply="$("$herdr" tab create --workspace "$ws" --cwd "$repo" --label e2e-host --focus)"
host_tab="$(echo "$reply" | json "d['result']['tab']['tab_id']")"
host_pane="$(echo "$reply" | json "d['result']['root_pane']['pane_id']")"
log "host tab $host_tab, host pane $host_pane"
baseline="$(tab_count)"
snapshot "baseline"

open_tab_view() {
  invoke toggle-tab "$host_tab" "$host_pane" || die "toggle-tab failed"
  view_tab="$(tab_state tab_id)" || die "toggle-tab wrote no tab state"
  list="$(tab_state list_pane)"
  preview="$(tab_state preview_pane)"
  log "tab view $view_tab: list $list, preview $preview"
  alive "$list" || die "tab view list pane not alive"
}

log "== A: toggle inside the tab view closes it"
open_tab_view
snapshot "A: tab view open"
invoke toggle "$view_tab" "$list" || die "toggle inside tab view errored"
sleep 0.5
snapshot "A: after toggle inside tab view"
alive "$list" && die "A: tab view still open (list pane $list alive)"
alive "$preview" && die "A: tab view still open (preview pane $preview alive)"
[ "$(sidebar_states)" = 0 ] || die "A: a sidebar view was nested into the tab view"
[ "$(tab_count)" = "$baseline" ] || die "A: tab count $(tab_count) != baseline $baseline (parking/view tab left behind)"

log "== B: open (sidebar) inside the tab view is a no-op"
open_tab_view
invoke open "$view_tab" "$list" || die "open inside tab view errored"
sleep 0.5
snapshot "B: after open inside tab view"
[ "$(sidebar_states)" = 0 ] || die "B: open nested a sidebar view into the tab view"
alive "$list" || die "B: open closed the tab view"
[ "$(tab_count)" = "$((baseline + 1))" ] || die "B: tab count $(tab_count) != baseline+1 (parking tab left behind?)"

log "== C: toggle in a normal tab still opens a sidebar, tab view untouched"
invoke toggle "$host_tab" "$host_pane" || die "toggle in host tab errored"
sleep 0.5
snapshot "C: after toggle in host tab"
[ "$(sidebar_states)" = 1 ] || die "C: no sidebar view opened in the host tab"
alive "$list" || die "C: toggle in another tab closed the tab view"
alive "$preview" || die "C: toggle in another tab closed the tab view preview"

log "== D: toggle-tab from another tab focuses the view, from inside closes it"
invoke toggle-tab "$host_tab" "$host_pane" || die "toggle-tab from host tab errored"
sleep 0.5
snapshot "D: after toggle-tab from host tab"
alive "$list" || die "D: toggle-tab from another tab closed the view instead of focusing it"
invoke toggle-tab "$view_tab" "$list" || die "toggle-tab inside tab view errored"
sleep 0.5
snapshot "D: after toggle-tab inside tab view"
alive "$list" && die "D: toggle-tab inside the tab view did not close it"
alive "$preview" && die "D: toggle-tab inside the tab view left the preview open"

log "PASS"
log "artifact: $ARTIFACT"
