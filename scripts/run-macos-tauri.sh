#!/bin/sh
# Pass the Swift debug data directory to compare that build's exact dataset.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export AIEYES_DATA_DIR="${1:-${AIEYES_DATA_DIR:-$HOME/Library/Application Support/Aieyes}}"
exec "$PROJECT_DIR/dist/Aieyes Tauri.app/Contents/MacOS/aieyes-desktop"
