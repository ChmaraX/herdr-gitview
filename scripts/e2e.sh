#!/usr/bin/env bash
# End-to-end smoke test: drives a live gitview inside a running herdr session
# via `herdr pane send-keys` / `herdr pane read`. Runs against a throwaway
# temp repo — never your working tree. Stages a file and commits it through
# the UI, asserting it moves changes → staged → committed and the tree ends
# clean.
#
# Requires: a running herdr session with the plugin linked, git, and nvim
# (the default commit editor). Run it from inside that herdr session:
#   scripts/e2e.sh
set -euo pipefail

herdr="${HERDR_BIN_PATH:-herdr}"
gitview="${GITVIEW_BIN:-herdr-gitview}"
state_dir="${HERDR_PLUGIN_STATE_DIR:-$HOME/.local/state/herdr-gitview}"

# --- throwaway repo -------------------------------------------------------
repo="$(mktemp -d)"
cleanup() {
  GITVIEW_REPO="$repo" "$gitview" close >/dev/null 2>&1 || true
  rm -rf "$repo"
}
trap cleanup EXIT

git -C "$repo" init -q -b main
git -C "$repo" config user.email e2e@test
git -C "$repo" config user.name e2e
printf 'base\n' >"$repo/base.txt"
git -C "$repo" add . && git -C "$repo" commit -qm init
printf 'hello e2e\n' >"$repo/scratch.txt" # untracked change to review

export GITVIEW_REPO="$repo"

die() {
  echo "FAIL: $*" >&2
  exit 1
}

pane_of() { # $1 = preview_pane | list_pane
  python3 - "$1" "$state_dir"/views/*.json <<'EOF'
import json, sys
print(json.load(open(sys.argv[2]))[sys.argv[1]])
EOF
}

read_pane() { "$herdr" pane read "$1" --lines 60 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g'; }

wait_for() { # $1 pane, $2 substring, $3 tries
  for _ in $(seq 1 "${3:-40}"); do
    read_pane "$1" | grep -qF "$2" && return 0
    sleep 0.3
  done
  return 1
}

echo "== open the view"
"$gitview" open || die "open failed"
sleep 1
list="$(pane_of list_pane)"
preview="$(pane_of preview_pane)"
wait_for "$list" "scratch.txt" || die "scratch file not in the list"
wait_for "$list" "CHANGES" || die "changes section missing"

echo "== stage it — moves to the staged section"
"$herdr" pane send-keys "$list" g >/dev/null # jump to the first entry
"$herdr" pane send-keys "$list" s >/dev/null
wait_for "$list" "STAGED CHANGES" || die "file did not move to the staged section"

echo "== commit through nvim"
"$herdr" pane send-keys "$list" c >/dev/null
wait_for "$preview" "COMMIT_EDITMSG" || die "commit editor did not open"
"$herdr" pane send-keys "$preview" i >/dev/null # insert mode
"$herdr" pane send-text "$preview" "e2e: scripted commit" >/dev/null
"$herdr" pane send-keys "$preview" escape >/dev/null
"$herdr" pane send-text "$preview" ":wq" >/dev/null
"$herdr" pane send-keys "$preview" enter >/dev/null

echo "== verify"
wait_for "$list" "working tree clean" || die "tree not clean after commit"
git -C "$repo" log -1 --format=%s | grep -qxF "e2e: scripted commit" ||
  die "commit missing from log"

echo "PASS"
