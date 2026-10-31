#!/bin/sh
# Shared local/CI checks. --rust additionally runs the complete core test suite.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
case "${1:-}" in
  ''|--rust) ;;
  *) echo 'Usage: scripts/ci-checks.sh [--rust]' >&2; exit 2 ;;
esac
if [ -z "${PYTHON:-}" ]; then
  if command -v python3 >/dev/null 2>&1; then PYTHON=python3; else PYTHON=python; fi
fi
"$PYTHON" scripts/release.py check
for script in test-release.py test-collectors.py test-wakeups.py test-incremental-history.py test-qoder-env.py test-codex-estimate-diagnostics.py test-legacy-account-history.py test-agy-identity.py test-native-wp8.py test-task-remote.py test-session-observers.py test-session-processes.py test-harness-remote.py test-native-database-remote.py test-account-scan.py test-remote-native-quota.py test-codebuddy-ide.py test-agent-live-audits.py test-profile-release-interactions.py; do
  "$PYTHON" "scripts/$script"
done
if "$PYTHON" -c 'import os, sys; sys.exit(os.name == "nt")'; then
  "$PYTHON" scripts/test-codex-auth.py
  "$PYTHON" scripts/test-codex-api.py
else
  echo 'Remote Codex POSIX permissions/locking tests require Unix; covered by the Unix check jobs.'
fi
"$PYTHON" scripts/sync-phase-colors.py --check
"$PYTHON" scripts/sync-chart-palettes.py --check
"$PYTHON" scripts/sync-ui-design-tokens.py --check
"$PYTHON" scripts/sync-core-contracts.py --check
"$PYTHON" scripts/generate-session-sounds.py --check
"$PYTHON" scripts/sync-localizations.py --check
"$PYTHON" scripts/check-localizations.py
npm --prefix apps/desktop run lint:css
node scripts/test-css-rules.mjs
for script in apps/desktop/web/*.js; do node --check "$script"; done
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs scripts/test-release-publish.mjs scripts/test-release-ci-baseline.mjs
node scripts/test-reset-display.cjs
case "${1:-}" in
  '') ;;
  --rust) cargo test --workspace --locked ;;
esac
