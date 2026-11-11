#!/bin/sh
# Requires a macOS GUI session; uses the production chart in an isolated test bundle.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
. "$PROJECT_DIR/scripts/native-verifiers.sh"
build_native_app
build_native_verifier model-legend
check_domain="app.aieyes.legendchecks.$$"
check_root=$(mktemp -d "$PROJECT_DIR/.build/legend-fixture.XXXXXX")
check_bundle="$check_root/LegendChecks.app"
trap 'defaults delete "$check_domain" >/dev/null 2>&1 || true; rm -rf "$check_root"' EXIT HUP INT TERM
mkdir -p "$check_bundle/Contents/MacOS"
cp .build/verify-model-legend "$check_bundle/Contents/MacOS/checks"
cat > "$check_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>$check_domain</string><key>CFBundleExecutable</key><string>checks</string><key>LSUIElement</key><true/></dict></plist>
PLIST
AIEYES_DATA_DIR="$check_bundle/data" "$check_bundle/Contents/MacOS/checks"
