#!/usr/bin/env bash
# Install gitview into the Herdr desktop app (herdr-app ≥ 0.1.11).
#
#   herdr-app/install.sh              build, install the mods, add the keys,
#                                     put gitview in the Changes panel
#   herdr-app/install.sh --uninstall  undo all of it (the stock Changes panel
#                                     comes back)
#   --binary PATH                     a prebuilt herdr-gitview instead of a
#                                     build (the release bundle's; the root
#                                     install.sh passes it)
#
# Running it again upgrades in place.
# The Herdr app must be running for the install (it may clone its layout and
# checks the mods); the uninstall doesn't need it.
# What it does, in <app home> ($HERDR_APP_HOME, else ~/.config/herdr-app):
#   - builds herdr-gitview (cargo build --release; or uses --binary, else
#     $GITVIEW_BIN) and
#     copies it to mods/gitview/bin/herdr-gitview
#   - copies every mod folder in herdr-app/mods: gitview (the file list, in
#     the Changes panel and in cmd-g's tab), gitview-diff (the diff pane),
#     gitview-agents (send notes to an agent), gitview-sidebar (cmd-shift-g:
#     show or hide the Changes panel with gitview, its diff pane with it),
#     gitview-tab (cmd-g: gitview in a tab of its own), gitview-ask (the
#     editor's "Save them?" dialog), gitview-keys (the keys card, ?),
#     gitview-mark and gitview-tracing (the palette's "gitview: mark
#     (something's wrong)" and "gitview: tracing on/off": the trace,
#     telemetry.js)
#   - writes mods/gitview/config.json: the binary, herdr-app and hc paths
#     (herdr-app: $HERDR_APP_BIN, else found on PATH or in /Applications)
#   - runs `herdr-app check` on each mod, before anything outside the mod
#     folders changes
#   - `herdr-gitview app-setup install`: adds the keys from keys.json
#     (keys.linux.json on Linux) where they're free, and points the Changes
#     slot of the person's personal layout at gitview (see src/appsetup.rs:
#     on one of Herdr's own layouts, the person's copy of it is used, or it
#     is cloned once as <name>-copy); it also drops what older installs
#     added and nothing uses now (the cmd-alt-g / cmd-alt-shift-g keys and
#     the gitview-popup, gitview-tui-sidebar, gitview-tui-tab mods). It
#     chooses the layout first: when that refuses (several copies), keys.json
#     and the layouts are left as they were
# The uninstall runs `app-setup uninstall` (our keys and our layout edits
# only; no layout is deleted or switched) and removes our mod folders.
#
# The herdr TUI plugin (herdr-plugin.toml, herdr/) is separate and untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOME_DIR="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"

say() { printf 'gitview: %s\n' "$*"; }
die() { printf 'gitview: %s\n' "$*" >&2; exit 1; }

UNINSTALL=
while [ $# -gt 0 ]; do
  case $1 in
    --uninstall) UNINSTALL=1 ;;
    --binary) [ $# -ge 2 ] || die "--binary needs a path"; GITVIEW_BIN=$2; shift ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) die "unknown argument $1" ;;
  esac
  shift
done

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

if [ -n "$UNINSTALL" ]; then
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

command -v git >/dev/null 2>&1 || die "git not found: gitview needs git (on macOS: xcode-select --install)"
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
# Each mod is put together in a hidden folder (the app doesn't watch those),
# checked, and swapped in with one rename, only when it differs from the
# installed one: the app hot-reloads a mod on every file it sees change, and
# every reload keeps the old build's code for the app's life (herdr-app
# 0.1.13, app bug #4), so a file-by-file copy (a dozen reloads, most of a
# half-copied mod) filled a long-running app's JS memory in a day of
# installs: then every reload failed with Null"null". An unchanged mod
# isn't touched (no reload at all).
STAGE="$HOME_DIR/mods/.gitview-install"
rm -rf "$STAGE" && mkdir -p "$STAGE"
for m in "${MODS[@]}"; do
  cp -RL "$ROOT/herdr-app/mods/$m" "$STAGE/$m" # -L: shared files are links in the repo
done
mkdir -p "$STAGE/gitview/bin"
install -m 0755 "$BIN_SRC" "$STAGE/gitview/bin/herdr-gitview"
json_str() { printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"; }
printf '{\n  "bin": %s,\n  "herdr_app": %s,\n  "hc": %s\n}\n' \
  "$(json_str "$HOME_DIR/mods/gitview/bin/herdr-gitview")" "$(json_str "$APP_BIN")" "$(json_str "$HC_BIN")" >"$STAGE/gitview/config.json"
for m in "${MODS[@]}"; do
  "$APP_BIN" check "$STAGE/$m" >/dev/null || die "herdr-app check failed for $m"
done
CHANGED=()
for m in "${MODS[@]}"; do
  # What the app writes into a mod folder (jsconfig.json, .d.ts, hidden
  # caches) isn't ours to compare.
  if [ -d "$HOME_DIR/mods/$m" ] && diff -rq -x jsconfig.json -x '*.d.ts' -x '.*' "$STAGE/$m" "$HOME_DIR/mods/$m" >/dev/null 2>&1; then continue; fi
  CHANGED+=("$m")
  if [ -d "$HOME_DIR/mods/$m" ]; then mv "$HOME_DIR/mods/$m" "$STAGE/.old-$m"; fi
  mv "$STAGE/$m" "$HOME_DIR/mods/$m"
done
rm -rf "$STAGE"
say "mods: ${CHANGED[*]:-none changed}"

# --- keys and layout -----------------------------------------------------------------
"$HOME_DIR/mods/gitview/bin/herdr-gitview" app-setup install \
  --home "$HOME_DIR" --app "$APP_BIN" --keys "$KEYS"
say "installed into $HOME_DIR (herdr-app: $APP_BIN)"
