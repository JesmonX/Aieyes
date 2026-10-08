#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --disable-sandbox
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"
# Link the production debug objects, excluding only its @main app delegate.
# @testable imports the module emitted by SwiftPM's debug configuration.
python3 - <<'PY'
from pathlib import Path
import subprocess
root = Path.cwd()
debug = root / '.build/swift/debug'
framework = root / '.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64'
objects = sorted(p for p in (debug/'Aieyes.build').glob('*.o') if p.name != 'AieyesApp.swift.o')
assert objects
subprocess.run(['swiftc', '-parse-as-library', '-module-cache-path', '.build/swift-module-cache', '-I', str(debug/'Modules'), '-F', str(framework), '-framework', 'Sparkle', '-Xlinker', '-rpath', '-Xlinker', str(framework), 'scripts/verify-native-performance.swift', *map(str, objects), '-o', '.build/verify-native-performance'], check=True)
PY
AIEYES_CORE_PATH="$PROJECT_DIR/scripts/preview-quota-core.py" AIEYES_UI_SCENARIO=multi .build/verify-native-performance "$PROJECT_DIR/.local/performance-2026-10-08"
