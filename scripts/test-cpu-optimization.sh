#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
if [ "${AIEYES_SKIP_BUILD:-0}" != 1 ]; then
  swift build --package-path apps/macos --scratch-path .build/swift --cache-path .build/spm-cache --config-path .build/spm-config --security-path .build/spm-security --disable-sandbox
fi
python3 - <<'PY'
from pathlib import Path
import subprocess
root=Path.cwd();debug=root/'.build/swift/debug';framework=root/'.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64'
objects=sorted(p for p in (debug/'Aieyes.build').glob('*.o') if p.name!='AieyesApp.swift.o')
subprocess.run(['swiftc','-parse-as-library','-module-cache-path','.build/swift-module-cache','-I',str(debug/'Modules'),'-F',str(framework),'-framework','Sparkle','-Xlinker','-rpath','-Xlinker',str(framework),'scripts/verify-cpu-optimization.swift',*map(str,objects),'-o','.build/verify-cpu-optimization'],check=True)
PY
check_domain="app.aieyes.cpuchecks.$$"
check_bundle="$PROJECT_DIR/.build/CPUChecks-$$.app"
trap 'defaults delete "$check_domain" >/dev/null 2>&1 || true; rm -rf "$check_bundle"' EXIT
mkdir -p "$check_bundle/Contents/MacOS"
cp .build/verify-cpu-optimization "$check_bundle/Contents/MacOS/checks"
cat > "$check_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>$check_domain</string><key>CFBundleExecutable</key><string>checks</string><key>LSUIElement</key><true/></dict></plist>
PLIST
"$check_bundle/Contents/MacOS/checks"
