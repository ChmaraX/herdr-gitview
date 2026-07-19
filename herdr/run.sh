#!/bin/sh
# Launcher for herdr-gitview: prefer the installed binary (bin/, written by
# install.sh from a release download), fall back to a local dev build.
# herdr runs plugin commands with a minimal PATH, so extend it for git/nvim.
PATH="$PATH:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.cargo/bin"
export PATH

root="${HERDR_PLUGIN_ROOT:?HERDR_PLUGIN_ROOT not set}"

if [ -x "$root/bin/herdr-gitview" ]; then
  exec "$root/bin/herdr-gitview" "$@"
fi
if [ -x "$root/target/release/herdr-gitview" ]; then
  exec "$root/target/release/herdr-gitview" "$@"
fi

echo "herdr-gitview binary not found (run herdr/install.sh or cargo build --release)" >&2
exit 1
