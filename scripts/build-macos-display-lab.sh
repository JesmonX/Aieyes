#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export AIEYES_DISPLAY_LAB=1
sh scripts/build-macos.sh "${1:-release}"
# Start with an isolated, empty workspace. Users can opt into their real sources
# in the lab; building it must not duplicate observers or saved production tasks.
if [ ! -f "$PROJECT_DIR/.local/display-lab/app/aieyes.sqlite" ]; then
  "$PROJECT_DIR/dist/Aieyes Lab.app/Contents/Resources/aieyes-core" --data-dir "$PROJECT_DIR/.local/display-lab/app" --call settings.save '{"version":6,"sources":[],"accounts":[],"hosts":[]}' > /dev/null
fi
