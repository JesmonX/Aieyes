#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
BUILD_MODE=${1:-debug}
case "$BUILD_MODE" in debug|release) ;; *) echo 'Usage: scripts/build-macos.sh [debug|release]' >&2; exit 2 ;; esac
export PATH="$HOME/.cargo/bin:$PATH"
export CLANG_MODULE_CACHE_PATH="$PROJECT_DIR/.build/clang-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$PROJECT_DIR/.build/swift-module-cache"
mkdir -p "$CLANG_MODULE_CACHE_PATH" "$SWIFTPM_MODULECACHE_OVERRIDE"
if [ "${AIEYES_OFFLINE:-0}" = 1 ]; then export CARGO_NET_OFFLINE=true; fi
if [ "$BUILD_MODE" = release ]; then cargo build --locked --release; else cargo build --locked; fi
swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --cache-path "$PROJECT_DIR/.build/spm-cache" --config-path "$PROJECT_DIR/.build/spm-config" --security-path "$PROJECT_DIR/.build/spm-security" --disable-sandbox -c "$BUILD_MODE"
APP_DIR="$PROJECT_DIR/dist/Aieyes.app"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$PROJECT_DIR/.build/swift/$BUILD_MODE/Aieyes" "$APP_DIR/Contents/MacOS/Aieyes"
cp "$PROJECT_DIR/target/$BUILD_MODE/aieyes-core" "$APP_DIR/Contents/Resources/aieyes-core"
cp "$PROJECT_DIR/apps/macos/Resources/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
cp "$PROJECT_DIR/apps/macos/Info.plist" "$APP_DIR/Contents/Info.plist"
if [ "$BUILD_MODE" = debug ]; then
  /usr/libexec/PlistBuddy -c "Add :AieyesDevelopmentDataDirectory string $PROJECT_DIR/.local/app" "$APP_DIR/Contents/Info.plist"
fi
codesign --force --sign - "$APP_DIR/Contents/Resources/aieyes-core"
codesign --force --sign - "$APP_DIR"
echo "$APP_DIR"
