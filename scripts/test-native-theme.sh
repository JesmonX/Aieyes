#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
# Uses the production objects from scripts/build-macos.sh debug.
python3 - <<'PY'
from pathlib import Path
import subprocess
root = Path.cwd()
debug = root / '.build/swift/debug'
framework = root / '.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64'
objects = sorted(p for p in (debug/'Aieyes.build').glob('*.o') if p.name != 'AieyesApp.swift.o')
assert objects
subprocess.run(['swiftc', '-parse-as-library', '-module-cache-path', '.build/swift-module-cache', '-I', str(debug/'Modules'), '-F', str(framework), '-framework', 'Sparkle', '-Xlinker', '-rpath', '-Xlinker', str(framework), 'scripts/verify-native-theme.swift', *map(str, objects), '-o', '.build/verify-native-theme'], check=True)
PY
.build/verify-native-theme "$PROJECT_DIR/.local/theme-sync/native"
