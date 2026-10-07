#!/bin/sh
# Synthetic six-state native screenshot matrix. No personal config or credentials.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
AIEYES_RENDER_BINARY=${AIEYES_RENDER_BINARY:-"$PROJECT_DIR/.build/swift/debug/Aieyes"}
test -x "$AIEYES_RENDER_BINARY"
export AIEYES_RENDER_ROOT=${AIEYES_RENDER_ROOT:-"$PROJECT_DIR/.local/ui-reform-2026-10-06/macos"}
mkdir -p "$AIEYES_RENDER_ROOT/data"
for scenario in empty single multi long failure capacity; do
  AIEYES_CORE_PATH="$PROJECT_DIR/scripts/preview-quota-core.py" AIEYES_DATA_DIR="$AIEYES_RENDER_ROOT/data" AIEYES_UI_SCENARIO="$scenario" "$AIEYES_RENDER_BINARY" --render "$AIEYES_RENDER_ROOT/$scenario"
done
python3 - <<'PY'
from pathlib import Path
import os
root=Path(os.environ['AIEYES_RENDER_ROOT'])
for scenario in ['empty','single','multi','long','failure','capacity']:
    for surface in ['panel-accounts', 'filter-failed', 'filter-restored']:
        assert (root/scenario/(surface+'.png')).stat().st_size > 1000
    if scenario != 'empty':
        assert (root/scenario/'quota-folded-summary.png').stat().st_size > 1000
        assert (root/scenario/'quota-folded-summary-dark.png').stat().st_size > 1000
        assert (root/scenario/'quota-dormant-reset.png').stat().st_size > 1000
    for theme in ['light','dark']:
        for symbol in ['circle.dotted','waveform','sparkles','gearshape.2','checkmark','pause.fill','questionmark']:
            assert (root/scenario/('menu-symbol-'+symbol+'-'+theme+'.png')).stat().st_size > 1000
        for surface in ['dashboard','menubar','low-panel','minimum-detail','increased-contrast','settings-prices','settings-general','servers']:
            assert (root/scenario/(surface+'-'+theme+'.png')).stat().st_size > 1000
print('Native six-state light/dark, settings, minimum size, reduced transparency and increased contrast screenshots passed')
PY
swift -module-cache-path .build/swift-module-cache scripts/verify-native-contrast.swift "$AIEYES_RENDER_ROOT"
for scenario in empty single multi long failure capacity; do
  swift -module-cache-path .build/swift-module-cache scripts/verify-panel-transparency.swift "$AIEYES_RENDER_ROOT/$scenario"
done
