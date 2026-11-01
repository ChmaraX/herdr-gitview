#!/usr/bin/env bash
# gitview and herdr-nvim in one private Herdr app: both installers patch the
# same layout (`changes` and `files` in the shared <name>-mods clone), in
# either order, and each uninstall restores only its own slot; the last one
# out removes the clone. And Enter in gitview opens the file in herdr-nvim's
# nvim through its CLI contract (`herdr-nvim open --tab <tab> <file>:<line>`).
#
#   HERDR_NVIM_SRC=/path/to/herdr-nvim herdr-app/e2e/coexist.sh [artifacts dir]
#
# HERDR_NVIM_SRC: a herdr-nvim checkout with herdr-app/install.sh (its
# install builds there and installs into this private instance only).
set -euo pipefail
HNV="${HERDR_NVIM_SRC:?set HERDR_NVIM_SRC to a herdr-nvim checkout}"

. "$(dirname "$0")/lib.sh"

slots() { # "<layout> files=<mod> changes=<mod>" for the shown layout
  python3 - "$HERDR_APP_HOME" "$("$APP" status)" <<'PY'
import json, sys
home, status = sys.argv[1], json.loads(sys.argv[2])
name = status["layout"]
doc = json.load(open(f"{home}/layouts/{name}.json"))
def find(n, i):
    if n.get("id") == i: return n
    for c in n.get("children", []):
        r = find(c, i)
        if r: return r
print(name, *(f"{s}={find(doc, s)['content']['mod']}" for s in ("files", "changes")))
PY
}
nvim_up() { "$HNV/herdr-app/install.sh" >>"$LOG" 2>&1; }
nvim_down() { "$HNV/herdr-app/install.sh" --uninstall >>"$LOG" 2>&1; }
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
do_ space.new --name scratch --folder "$S"

step "herdr-nvim first, then gitview"
nvim_up || fail "herdr-nvim install"
gv_up || fail "gitview install"
want "main-mods files=herdr-nvim changes=gitview" "both slots patched in one clone"

step "Enter in gitview opens the file in herdr-nvim's nvim"
raise
sleep 1.5
do_ layout.select '{"stack":"panel-body","child":"changes"}'
do_ layout.show --id panel
expect '"$APP" do gitview.key:r' "the gitview panel is up"
do_ gitview.file:src/main.rs
do_ gitview.key:enter
DEBUG="$XDG_STATE_HOME/herdr-gitview/app/debug.log"
expect 'grep -q "herdr-nvim open .*/src/main.rs:2 -> exit status: 0" "$DEBUG"' "gitview called herdr-nvim open --tab <tab> <file>:2, and it succeeded" 30
NV=$(grep "herdr-nvim open .*/src/main.rs:2 -> exit status: 0" "$DEBUG" | tail -1 | sed 's/.*"terminal":"\([^"]*\)".*/\1/')
expect 'nvim_buf $NV | grep -q "src/main.rs$"' "herdr-nvim's nvim ($NV) shows the file" 20
shot_settled coexist-01-enter-herdr-nvim

step "uninstall gitview: only changes goes back"
gv_down || fail "gitview uninstall"
want "main-mods files=herdr-nvim changes=changes" "herdr-nvim's slot and the clone stay"
step "reinstall gitview, then uninstall herdr-nvim"
gv_up || fail "gitview reinstall"
want "main-mods files=herdr-nvim changes=gitview" "reinstall patches the same clone"
nvim_down || fail "herdr-nvim uninstall"
want "main-mods files=files changes=gitview" "only files goes back; the clone stays for gitview"
step "gitview last out"
gv_down || fail "gitview uninstall"
want "main files=files changes=changes" "back on the shipped main layout"
[ ! -e "$HERDR_APP_HOME/layouts/main-mods.json" ] && ok "clone removed" || fail "clone left"

step "the other order: gitview first, then herdr-nvim"
gv_up || fail "gitview install"
nvim_up || fail "herdr-nvim install"
want "main-mods files=herdr-nvim changes=gitview" "both slots patched"
nvim_down || fail "herdr-nvim uninstall"
want "main-mods files=files changes=gitview" "herdr-nvim restores its slot only"
gv_down || fail "gitview uninstall"
want "main files=files changes=changes" "last one out removes the clone"
step "all passed"
