#!/usr/bin/env bash
# Build step for `herdr plugin install adamchmara/herdr-gitview`:
# download the release binary for this platform (with checksum verification),
# falling back to a local cargo build when no asset matches.
set -euo pipefail

REPO="adamchmara/herdr-gitview"
ROOT="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/herdr-plugin.toml" | head -1)"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  *)             TARGET="" ;;
esac

fallback_build() {
  if command -v cargo >/dev/null 2>&1; then
    echo "herdr-gitview: building from source (cargo build --release)…"
    (cd "$ROOT" && cargo build --release)
    mkdir -p "$ROOT/bin"
    install -m 0755 "$ROOT/target/release/herdr-gitview" "$ROOT/bin/herdr-gitview"
    echo "herdr-gitview: installed from source build"
    exit 0
  fi
  echo "herdr-gitview: no release asset for $(uname -s)/$(uname -m) and cargo is not installed." >&2
  echo "Install rust (https://rustup.rs) and re-run, or download a binary manually." >&2
  exit 1
}

[ -n "$TARGET" ] || fallback_build

ASSET="herdr-gitview-v${VERSION}-${TARGET}.tar.gz"
BASE="https://github.com/${REPO}/releases/download/v${VERSION}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch() {
  # 5 retries with backoff.
  curl -fsSL --retry 5 --retry-delay 2 -o "$2" "$1"
}

echo "herdr-gitview: downloading ${ASSET}…"
if ! fetch "${BASE}/${ASSET}" "$TMP/$ASSET" || ! fetch "${BASE}/${ASSET}.sha256" "$TMP/$ASSET.sha256"; then
  echo "herdr-gitview: download failed — falling back to source build"
  fallback_build
fi

(cd "$TMP" && shasum -a 256 -c "$ASSET.sha256")
tar -xzf "$TMP/$ASSET" -C "$TMP"

mkdir -p "$ROOT/bin"
install -m 0755 "$TMP/herdr-gitview" "$ROOT/bin/herdr-gitview"
echo "herdr-gitview: installed v${VERSION} (${TARGET})"
