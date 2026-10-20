#!/bin/sh
# Optimized macOS WebView build for comparison with the SwiftUI application.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test "$(uname -s)" = Darwin || { echo 'This script requires macOS.' >&2; exit 2; }
export PATH="$HOME/.cargo/bin:$PATH"
if [ "${AIEYES_OFFLINE:-0}" = 1 ]; then export CARGO_NET_OFFLINE=true; fi
cd "$PROJECT_DIR/apps/desktop"
if command -v tauri >/dev/null 2>&1; then
  set -- tauri
elif [ -x node_modules/.bin/tauri ]; then
  set -- node_modules/.bin/tauri
elif [ -x "$PROJECT_DIR/.build/tauri-cli/node_modules/.bin/tauri" ]; then
  set -- "$PROJECT_DIR/.build/tauri-cli/node_modules/.bin/tauri"
else
  set -- cargo tauri
fi
"$@" build --ci --bundles app -- --locked
APP="$PROJECT_DIR/apps/desktop/src-tauri/target/release/bundle/macos/Aieyes Tauri.app"
test -x "$APP/Contents/MacOS/aieyes-desktop"
codesign --verify --deep --strict "$APP"
mkdir -p "$PROJECT_DIR/dist"
ditto "$APP" "$PROJECT_DIR/dist/Aieyes Tauri.app"
VERSION=$(node -p 'require("./src-tauri/tauri.conf.json").version')
ARCH=$(uname -m)
ditto -c -k --sequesterRsrc --keepParent "$PROJECT_DIR/dist/Aieyes Tauri.app" "$PROJECT_DIR/dist/Aieyes-$VERSION-macos-tauri-$ARCH.zip"
echo "$PROJECT_DIR/dist/Aieyes Tauri.app"
echo "$PROJECT_DIR/dist/Aieyes-$VERSION-macos-tauri-$ARCH.zip"
