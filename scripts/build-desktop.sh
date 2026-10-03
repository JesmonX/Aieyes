#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD_MODE=${1:-debug}
case "$BUILD_MODE" in debug|release) ;; *) echo 'Usage: sh scripts/build-desktop.sh [debug|release]' >&2; exit 2 ;; esac
export PATH="$HOME/.cargo/bin:$PATH"
if [ "${AIEYES_OFFLINE:-0}" = 1 ]; then export CARGO_NET_OFFLINE=true; fi
cd "$PROJECT_DIR/apps/desktop"
if [ "$BUILD_MODE" = debug ]; then
  cargo build --manifest-path src-tauri/Cargo.toml --locked --features custom-protocol
  echo "$PROJECT_DIR/apps/desktop/src-tauri/target/debug/aieyes-desktop"
else
  if command -v tauri >/dev/null 2>&1; then set -- tauri; else set -- cargo tauri; fi
  if ! "$@" --version >/dev/null 2>&1; then
    echo 'Install the Tauri v2 CLI: npm install --global @tauri-apps/cli@2 (or cargo install tauri-cli --version "^2" --locked)' >&2
    exit 1
  fi
  case "$(uname -s)" in
    Linux) "$@" build --ci --bundles deb,appimage -- --locked ;;
    *) echo 'Use scripts/build-macos.sh for macOS or scripts/build-desktop.ps1 for Windows.' >&2; exit 2 ;;
  esac
  echo "$PROJECT_DIR/apps/desktop/src-tauri/target/release/bundle"
fi
