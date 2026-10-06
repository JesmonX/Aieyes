#!/bin/sh
# Synthetic five-state native screenshot matrix. No personal config or credentials.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
AIEYES_RENDER_BINARY=${AIEYES_RENDER_BINARY:-"$PROJECT_DIR/.build/swift/debug/Aieyes"}
test -x "$AIEYES_RENDER_BINARY"
export AIEYES_RENDER_ROOT=${AIEYES_RENDER_ROOT:-"$PROJECT_DIR/.local/ui-reform-2026-10-06/macos"}
mkdir -p "$AIEYES_RENDER_ROOT/data"
for scenario in empty single multi long failure; do
  AIEYES_CORE_PATH="$PROJECT_DIR/scripts/preview-quota-core.py" AIEYES_DATA_DIR="$AIEYES_RENDER_ROOT/data" AIEYES_UI_SCENARIO="$scenario" "$AIEYES_RENDER_BINARY" --render "$AIEYES_RENDER_ROOT/$scenario"
done
python3 - <<'PY'
from pathlib import Path
import os
root=Path(os.environ['AIEYES_RENDER_ROOT'])
for scenario in ['empty','single','multi','long','failure']:
    for theme in ['light','dark']:
        for surface in ['dashboard','menubar','low-panel','minimum-detail','increased-contrast','settings-prices','settings-general','servers']:
            assert (root/scenario/(surface+'-'+theme+'.png')).stat().st_size > 1000
print('Native five-state light/dark, settings, minimum size, reduced transparency and increased contrast screenshots passed')
PY
