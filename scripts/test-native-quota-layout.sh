#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
mkdir -p .build/swift-module-cache .build/clang-cache
export CLANG_MODULE_CACHE_PATH="$PROJECT_DIR/.build/clang-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$PROJECT_DIR/.build/swift-module-cache"
export MACOSX_DEPLOYMENT_TARGET=14.0
swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --cache-path "$PROJECT_DIR/.build/spm-cache" --config-path "$PROJECT_DIR/.build/spm-config" --security-path "$PROJECT_DIR/.build/spm-security" --disable-sandbox --disable-automatic-resolution -c debug -Xlinker -rpath -Xlinker @executable_path/../Frameworks
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"
BUILD_DIR="$PROJECT_DIR/.build/swift/debug"
set --
for file in "$BUILD_DIR"/Aieyes.build/*.o; do
  case "$file" in */AieyesApp.swift.o) ;; *) set -- "$@" "$file" ;; esac
done
swiftc -parse-as-library -I "$BUILD_DIR/Modules" -module-cache-path .build/swift-module-cache -F "$SPARKLE_FRAMEWORK" -framework Sparkle -Xlinker -rpath -Xlinker "$SPARKLE_FRAMEWORK" "$@" scripts/verify-quota-layout.swift -o .build/verify-quota-layout
.build/verify-quota-layout
