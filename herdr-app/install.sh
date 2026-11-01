#!/usr/bin/env bash
# Install gitview into the Herdr desktop app (herdr-app ≥ 0.1.11).
#
#   herdr-app/install.sh              build, install the mods, add the keys,
#                                     put gitview in the Changes panel
#   herdr-app/install.sh --uninstall  undo all of it (the stock Changes panel
#                                     comes back)
#
# The Herdr app must be running (it clones its layout and checks the mods).
# What it does, in <app home> ($HERDR_APP_HOME, else ~/.config/herdr-app):
#   - builds herdr-gitview (cargo build --release; or uses $GITVIEW_BIN) and
#     copies it to mods/gitview/bin/herdr-gitview
#   - copies the mods: gitview (the panel), gitview-agents (send notes),
#     gitview-sidebar / gitview-tab (the full TUI as panes), gitview-popup
#   - writes mods/gitview/config.json: the binary, herdr-app and hc paths
#   - adds the keys from keys.json (keys.linux.json on Linux) to keys.json's "global" section,
#     never replacing a key that's already bound to something else
#   - points the shown layout's `changes` block at the gitview mod, keeping
#     what it showed in props.replaces (layout_slot.py, the same mechanism as
#     herdr-nvim's: a shipped layout is cloned once as <name>-mods)
#   - runs `herdr-app check` on each mod
#
# The herdr TUI plugin (herdr-plugin.toml, herdr/) is separate and untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOME_DIR="${HERDR_APP_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}/herdr-app}"
MODS=(gitview gitview-agents gitview-sidebar gitview-tab gitview-tui-sidebar gitview-tui-tab gitview-popup)

say() { printf 'gitview: %s\n' "$*"; }
die() { printf 'gitview: %s\n' "$*" >&2; exit 1; }

[ -d "$HOME_DIR" ] || die "no Herdr app home at $HOME_DIR (start the app once, or set HERDR_APP_HOME)"
[ -f "$HOME_DIR/keys.json" ] || die "$HOME_DIR/keys.json is missing (start the app once first)"

case "$(uname -s)" in
  Darwin) KEYS="$ROOT/herdr-app/keys.json" ;;
  *) KEYS="$ROOT/herdr-app/keys.linux.json" ;;
esac

# Add (or with --uninstall remove) our keys in keys.json's "global" section.
merge_keys() { # $1 = add | remove
  python3 - "$1" "$KEYS" "$HOME_DIR/keys.json" <<'EOF'
import json, os, sys
mode, ours_path, path = sys.argv[1:]
ours = json.load(open(ours_path))["global"]
keys = json.load(open(path))
glob = keys.setdefault("global", {})
changed = False
for key, action in ours.items():
    have = glob.get(key)
    if mode == "remove":
        if have == action:
            del glob[key]; changed = True
            print(f"gitview: removed key {key}")
    elif have is None:
        glob[key] = action; changed = True
        print(f"gitview: bound {key} -> {action}")
    elif have != action:
        print(f"gitview: {key} is already bound to {have!r}; not replacing it. "
              f"Bind {action!r} to a free key in {path} yourself.")
if changed:
    tmp = path + ".gitview-tmp"
    with open(tmp, "w") as f:
        json.dump(keys, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp, path)
EOF
}

# Only ever replace or remove our own folders.
ours() { [ ! -e "$1" ] || grep -q '"title": "gitview' "$1/mod.json" 2>/dev/null; }

# --- herdr-app and hc ----------------------------------------------------------
is_herdr_app() { "$1" --version 2>/dev/null | grep -q '^herdr-app '; }
find_app() {
  if [ -n "${HERDR_APP_BIN:-}" ]; then echo "$HERDR_APP_BIN"; return; fi
  local c
  c="$(command -v herdr-app 2>/dev/null || true)"
  if [ -n "$c" ] && is_herdr_app "$c"; then echo "$c"; return; fi
  # macOS bundles: several apps are called Herdr; ask each which it is.
  for c in /Applications/*.app/Contents/MacOS/Herdr "$HOME"/Applications/*.app/Contents/MacOS/Herdr; do
    if [ -x "$c" ] && is_herdr_app "$c"; then echo "$c"; return; fi
  done
}
find_app_or_die() {
  APP_BIN="$(find_app)"
  [ -n "$APP_BIN" ] || die "herdr-app not found; set HERDR_APP_BIN=/path/to/herdr-app"
  HERDR_APP_HOME="$HOME_DIR" "$APP_BIN" status >/dev/null 2>&1 || die "start the Herdr app first (HERDR_APP_HOME=$HOME_DIR)"
}

if [ "${1:-}" = "--uninstall" ]; then
  find_app_or_die
  python3 "$ROOT/herdr-app/layout_slot.py" uninstall "$HOME_DIR" changes gitview "$APP_BIN"
  for m in "${MODS[@]}"; do
    ours "$HOME_DIR/mods/$m" || die "$HOME_DIR/mods/$m isn't gitview's; leaving it"
    rm -rf "${HOME_DIR:?}/mods/$m"
  done
  merge_keys remove
  say "uninstalled from $HOME_DIR"
  exit 0
fi

# --- the binary --------------------------------------------------------------
if [ -n "${GITVIEW_BIN:-}" ]; then
  BIN_SRC="$GITVIEW_BIN"
else
  command -v cargo >/dev/null 2>&1 || die "cargo not found (https://rustup.rs), or set GITVIEW_BIN"
  say "building (cargo build --release)…"
  (cd "$ROOT" && cargo build --release --quiet)
  BIN_SRC="$ROOT/target/release/herdr-gitview"
fi
[ -x "$BIN_SRC" ] || die "no binary at $BIN_SRC"

# --- herdr-app and hc ------------------------------------------------------------
find_app_or_die
HC_BIN="$(command -v hc 2>/dev/null || true)"
[ -n "$HC_BIN" ] || HC_BIN="$HOME/.local/bin/hc"
[ -x "$HC_BIN" ] || die "hc (herdr-core) not found"

# --- the mods ----------------------------------------------------------------------
mkdir -p "$HOME_DIR/mods"
for m in "${MODS[@]}"; do
  ours "$HOME_DIR/mods/$m" || die "$HOME_DIR/mods/$m exists and isn't gitview's; not overwriting it"
  rm -rf "${HOME_DIR:?}/mods/$m"
  cp -RL "$ROOT/herdr-app/mods/$m" "$HOME_DIR/mods/$m" # -L: shared files are links in the repo
done
mkdir -p "$HOME_DIR/mods/gitview/bin"
install -m 0755 "$BIN_SRC" "$HOME_DIR/mods/gitview/bin/herdr-gitview"
python3 - "$HOME_DIR/mods/gitview/config.json" "$HOME_DIR/mods/gitview/bin/herdr-gitview" "$APP_BIN" "$HC_BIN" <<'EOF'
import json, sys
path, bin_, app, hc = sys.argv[1:]
json.dump({"bin": bin_, "herdr_app": app, "hc": hc}, open(path, "w"), indent=2)
EOF

merge_keys add
python3 "$ROOT/herdr-app/layout_slot.py" install "$HOME_DIR" changes gitview "$APP_BIN"

for m in "${MODS[@]}"; do
  "$APP_BIN" check "$HOME_DIR/mods/$m" >/dev/null || die "herdr-app check failed for $m"
done
say "installed into $HOME_DIR (herdr-app: $APP_BIN)"
