#!/usr/bin/env bash
# Install gitview into the Herdr desktop app (herdr-app ≥ 0.1.11).
#
#   herdr-app/install.sh              build, install the mods, add the keys,
#                                     put gitview in the Changes panel
#   herdr-app/install.sh --uninstall  undo all of it (the stock Changes panel
#                                     comes back)
#
# The Herdr app must be running for the install (it may clone its layout and
# checks the mods); the uninstall doesn't need it.
# What it does, in <app home> ($HERDR_APP_HOME, else ~/.config/herdr-app):
#   - builds herdr-gitview (cargo build --release; or uses $GITVIEW_BIN) and
#     copies it to mods/gitview/bin/herdr-gitview
#   - copies every mod folder in herdr-app/mods (the gitview panel, its diff
#     pane, send notes, the full TUI as panes, the popup)
#   - writes mods/gitview/config.json: the binary, herdr-app and hc paths
#   - runs `herdr-app check` on each mod, before anything outside the mod
#     folders changes
#   - `herdr-gitview app-setup install`: adds the keys from keys.json
#     (keys.linux.json on Linux) where they're free, and points the Changes
#     slot of the person's personal layout at gitview (see src/appsetup.rs:
#     on one of Herdr's own layouts, the person's copy of it is used, or it
#     is cloned once as <name>-copy); it also drops what older installs
#     added and nothing uses now (the TUI-in-the-app keys and mods)
# The uninstall runs `app-setup uninstall` (our keys and our layout edits
# only; no layout is deleted or switched) and removes our mod folders.
#
# The herdr TUI plugin (herdr-plugin.toml, herdr/) is separate and untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOME_DIR="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"

say() { printf 'gitview: %s\n' "$*"; }
die() { printf 'gitview: %s\n' "$*" >&2; exit 1; }

[ -d "$HOME_DIR" ] || die "no Herdr app home at $HOME_DIR (start the app once, or set HERDR_APP_HOME)"

case "$(uname -s)" in
  Darwin) KEYS="$ROOT/herdr-app/keys.json" ;;
  *) KEYS="$ROOT/herdr-app/keys.linux.json" ;;
esac

# The mods this checkout ships: every folder in herdr-app/mods.
MODS=()
for d in "$ROOT"/herdr-app/mods/*/; do
  [ -f "$d/mod.json" ] && MODS+=("$(basename "$d")")
done
[ "${#MODS[@]}" -gt 0 ] || die "no mods in $ROOT/herdr-app/mods"

# Only ever replace or remove our own folders.
ours() { [ ! -e "$1" ] || grep -Eq '"title"[[:space:]]*:[[:space:]]*"gitview' "$1/mod.json" 2>/dev/null; }

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

# --- the binary --------------------------------------------------------------
build() {
  if [ -n "${GITVIEW_BIN:-}" ]; then
    BIN_SRC="$GITVIEW_BIN"
  else
    command -v cargo >/dev/null 2>&1 || die "cargo not found (https://rustup.rs), or set GITVIEW_BIN"
    say "building (cargo build --release)…"
    (cd "$ROOT" && cargo build --release --quiet)
    BIN_SRC="$ROOT/target/release/herdr-gitview"
  fi
  [ -x "$BIN_SRC" ] || die "no binary at $BIN_SRC"
}

if [ "${1:-}" = "--uninstall" ]; then
  for m in "${MODS[@]}"; do
    ours "$HOME_DIR/mods/$m" || die "$HOME_DIR/mods/$m isn't gitview's; leaving it"
  done
  # This checkout's build undoes the setup; without cargo, the installed one.
  if [ -z "${GITVIEW_BIN:-}" ] && ! command -v cargo >/dev/null 2>&1 \
    && [ -x "$HOME_DIR/mods/gitview/bin/herdr-gitview" ]; then
    BIN_SRC="$HOME_DIR/mods/gitview/bin/herdr-gitview"
  else
    build
  fi
  "$BIN_SRC" app-setup uninstall --home "$HOME_DIR" --keys "$KEYS"
  # Our folders: the ones this checkout ships, plus gitview mods an older
  # install left (any mods/gitview* whose mod.json says it's gitview's).
  for d in "$HOME_DIR"/mods/gitview "$HOME_DIR"/mods/gitview-*; do
    [ -d "$d" ] || continue
    if ours "$d"; then rm -rf "${d:?}"; else say "left $d (not gitview's)"; fi
  done
  say "uninstalled from $HOME_DIR"
  exit 0
fi

build
find_app_or_die
HC_BIN="$(command -v hc 2>/dev/null || true)"
[ -n "$HC_BIN" ] || HC_BIN="$HOME/.local/bin/hc"
[ -x "$HC_BIN" ] || die "hc (herdr-core) not found"

# --- the mods ----------------------------------------------------------------------
mkdir -p "$HOME_DIR/mods"
for m in "${MODS[@]}"; do
  ours "$HOME_DIR/mods/$m" || die "$HOME_DIR/mods/$m exists and isn't gitview's; not overwriting it"
done
for m in "${MODS[@]}"; do
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

for m in "${MODS[@]}"; do
  "$APP_BIN" check "$HOME_DIR/mods/$m" >/dev/null || die "herdr-app check failed for $m"
done

# --- keys and layout -----------------------------------------------------------------
"$HOME_DIR/mods/gitview/bin/herdr-gitview" app-setup install \
  --home "$HOME_DIR" --app "$APP_BIN" --keys "$KEYS"
say "installed into $HOME_DIR (herdr-app: $APP_BIN)"
