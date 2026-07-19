#!/usr/bin/env bash
# E2E smoke test: drives a live gitview inside a running herdr session via
# `herdr pane send-keys` / `herdr pane read`. Run from inside a git repo in a
# herdr workspace. Creates a scratch file, stages and commits it through the
# UI, and asserts the list empties out.
set -euo pipefail

herdr="${HERDR_BIN_PATH:-herdr}"
repo="$(git rev-parse --show-toplevel)"
state_dir="${HERDR_PLUGIN_STATE_DIR:-$HOME/.local/state/herdr/plugins/adamchmara.gitview}"

die() { echo "FAIL: $*" >&2; exit 1; }
pane_of() { # $1 = preview_pane | list_pane
  python3 - "$1" "$state_dir"/views/*.json <<'EOF'
import json, sys
print(json.load(open(sys.argv[2]))[sys.argv[1]])
EOF
}

read_pane() { "$herdr" pane read "$1" --lines 60 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g'; }

wait_for() { # $1 pane, $2 substring, $3 tries
  for _ in $(seq 1 "${3:-30}"); do
    read_pane "$1" | grep -qF "$2" && return 0
    sleep 0.3
  done
  return 1
}

echo "== open view"
scratch="e2e-scratch-$$.txt"
printf 'hello e2e\n' > "$repo/$scratch"
(cd "$repo" && "${GITVIEW_BIN:-herdr-gitview}" open) || die "open failed"
sleep 1
list="$(pane_of list_pane)"; preview="$(pane_of preview_pane)"
wait_for "$list" "$scratch" || die "scratch file not in list"

echo "== stage + commit"
# cursor lands on conflicts/paths alphabetically; find and select our file: just
# stage everything visible by pressing s on each row from the top.
"$herdr" pane send-keys "$list" g >/dev/null    # jump to top
rows="$(read_pane "$list" | grep -c '^\s*[●◐]\?[MADR?U] ' || true)"
for _ in $(seq 1 "${rows:-1}"); do
  "$herdr" pane send-keys "$list" s >/dev/null; sleep 0.2
  "$herdr" pane send-keys "$list" j >/dev/null
done
"$herdr" pane send-keys "$list" c >/dev/null
wait_for "$preview" "COMMIT_EDITMSG" 40 || die "commit editor did not open"
"$herdr" pane send-text "$preview" "e2e: scripted commit" >/dev/null
"$herdr" pane send-keys "$preview" escape >/dev/null
"$herdr" pane send-text "$preview" ":wq" >/dev/null
"$herdr" pane send-keys "$preview" enter >/dev/null

echo "== verify"
wait_for "$list" "working tree clean" 40 || die "list did not empty after commit"
git -C "$repo" log -1 --format=%s | grep -qF "e2e: scripted commit" || die "commit missing from log"

echo "== close view"
(cd "$repo" && "${GITVIEW_BIN:-herdr-gitview}" close)
echo "PASS"
