#!/bin/sh
# Isolated native animation/retained-page soak: default 10 min warm-up + 30 min samples.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
OUTPUT=${1:-.local/runtime-performance}
DURATION=${2:-2400}
WARMUP=${3:-600}
mkdir -p "$OUTPUT"
OUTPUT=$(CDPATH= cd -- "$OUTPUT" && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
export CLANG_MODULE_CACHE_PATH="$PROJECT_DIR/.build/clang-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$PROJECT_DIR/.build/swift-module-cache"
cargo build --workspace --locked
swift build --package-path apps/macos --scratch-path .build/swift --cache-path .build/spm-cache --config-path .build/spm-config --security-path .build/spm-security --disable-sandbox
AIEYES_UI_SCENARIO=long python3 scripts/preview-quota-core.py --fixture > "$OUTPUT/fixture.json"
target/debug/aieyes-core --data-dir "$OUTPUT/pet-data" --call pets.current > "$OUTPUT/pet.json"
python3 - <<'PY'
from pathlib import Path
import subprocess
root = Path.cwd()
debug = root / '.build/swift/debug'
framework = root / '.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64'
objects = sorted(p for p in (debug/'Aieyes.build').glob('*.o') if p.name != 'AieyesApp.swift.o')
assert objects
subprocess.run(['swiftc', '-D', 'AIEYES_PERFORMANCE_MODULE', '-parse-as-library', '-module-cache-path', '.build/swift-module-cache', '-I', str(debug/'Modules'), '-F', str(framework), '-framework', 'Sparkle', '-Xlinker', '-rpath', '-Xlinker', str(framework), 'scripts/verify-runtime-performance.swift', *map(str,objects), '-o', '.build/verify-runtime-performance'], check=True)
PY
AIEYES_CORE_PATH="$PROJECT_DIR/scripts/preview-quota-core.py" AIEYES_UI_SCENARIO=long .build/verify-runtime-performance "$OUTPUT" "$OUTPUT/fixture.json" "$OUTPUT/pet.json" "$DURATION" "$WARMUP"
