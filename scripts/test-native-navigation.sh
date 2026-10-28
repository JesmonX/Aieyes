#!/bin/sh
# Requires a macOS GUI session. Uses only an isolated core directory and volatile UI preferences.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
if [ "${AIEYES_SKIP_BUILD:-}" != 1 ]; then
  swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --disable-sandbox --disable-automatic-resolution -c debug
fi
TEST_DATA=$(mktemp -d "$PROJECT_DIR/.build/navigation-fixture.XXXXXX")
trap 'rm -rf "$TEST_DATA"' EXIT HUP INT TERM
export AIEYES_DATA_DIR="$TEST_DATA" AIEYES_CORE_PATH="$PROJECT_DIR/scripts/preview-quota-core.py"
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"
BUILD_DIR="$PROJECT_DIR/.build/swift/debug"
set --
for file in "$BUILD_DIR"/Aieyes.build/*.o; do
  case "$file" in */AieyesApp.swift.o) ;; *) set -- "$@" "$file" ;; esac
done
swiftc -parse-as-library -I "$BUILD_DIR/Modules" -module-cache-path .build/swift-module-cache -F "$SPARKLE_FRAMEWORK" -framework Sparkle -Xlinker -rpath -Xlinker "$SPARKLE_FRAMEWORK" "$@" scripts/verify-native-navigation.swift -o .build/verify-native-navigation
.build/verify-native-navigation
