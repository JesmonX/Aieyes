#!/bin/sh
# Shared local/CI checks. --rust additionally runs the complete core test suite.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
if command -v python3 >/dev/null 2>&1; then PYTHON=python3; else PYTHON=python; fi
"$PYTHON" scripts/release.py check
"$PYTHON" scripts/test-release.py
"$PYTHON" scripts/test-collectors.py
"$PYTHON" scripts/test-wakeups.py
"$PYTHON" scripts/sync-phase-colors.py --check
"$PYTHON" scripts/sync-chart-palettes.py --check
"$PYTHON" scripts/sync-core-contracts.py --check
for script in apps/desktop/web/*.js; do node --check "$script"; done
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs scripts/test-release-publish.mjs
case "${1:-}" in
  '') ;;
  --rust) cargo test --workspace --locked ;;
  *) echo 'Usage: scripts/ci-checks.sh [--rust]' >&2; exit 2 ;;
esac
