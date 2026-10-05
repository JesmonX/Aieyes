#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
BUILD_MODE=${1:-debug}
case "$BUILD_MODE" in debug|release) ;; *) echo 'Usage: scripts/build-macos.sh [debug|release]' >&2; exit 2 ;; esac
PACKAGE_MODE=${2:-}
case "$PACKAGE_MODE" in ''|--dmg) ;; *) echo 'Optional second argument: --dmg' >&2; exit 2 ;; esac
if [ "$PACKAGE_MODE" = --dmg ] && [ "$BUILD_MODE" != release ]; then echo 'DMG requires release mode' >&2; exit 2; fi
export MACOSX_DEPLOYMENT_TARGET=14.0
export PATH="$HOME/.cargo/bin:$PATH"
export CLANG_MODULE_CACHE_PATH="$PROJECT_DIR/.build/clang-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$PROJECT_DIR/.build/swift-module-cache"
mkdir -p "$CLANG_MODULE_CACHE_PATH" "$SWIFTPM_MODULECACHE_OVERRIDE"
if [ "${AIEYES_OFFLINE:-0}" = 1 ]; then export CARGO_NET_OFFLINE=true; fi
if [ "$BUILD_MODE" = release ]; then cargo build --locked --release; else cargo build --locked; fi
swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --cache-path "$PROJECT_DIR/.build/spm-cache" --config-path "$PROJECT_DIR/.build/spm-config" --security-path "$PROJECT_DIR/.build/spm-security" --disable-sandbox -c "$BUILD_MODE" -Xlinker -rpath -Xlinker @executable_path/../Frameworks
APP_DIR="$PROJECT_DIR/dist/Aieyes.app"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$PROJECT_DIR/.build/swift/$BUILD_MODE/Aieyes" "$APP_DIR/Contents/MacOS/Aieyes"
cp "$PROJECT_DIR/target/$BUILD_MODE/aieyes-core" "$APP_DIR/Contents/Resources/aieyes-core"
cp "$PROJECT_DIR/apps/macos/Resources/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
cp "$PROJECT_DIR/apps/macos/Resources/Brand.png" "$APP_DIR/Contents/Resources/Brand.png"
cp "$PROJECT_DIR/apps/macos/Resources/BrandTemplate.png" "$APP_DIR/Contents/Resources/BrandTemplate.png"
cp "$PROJECT_DIR/apps/macos/Info.plist" "$APP_DIR/Contents/Info.plist"
if [ "$BUILD_MODE" = debug ]; then
  /usr/libexec/PlistBuddy -c "Add :AieyesDevelopmentDataDirectory string $PROJECT_DIR/.local/app" "$APP_DIR/Contents/Info.plist"
fi
python3 scripts/macos-distribution.py bundle "$APP_DIR" --mode "$BUILD_MODE"
echo "$APP_DIR"
if [ "$PACKAGE_MODE" = --dmg ]; then
  VERSION=$(python3 scripts/release.py check)
  case "$(uname -m)" in arm64) ARCH=arm64 ;; x86_64) ARCH=x64 ;; *) exit 2 ;; esac
  STAGING=$(mktemp -d "$PROJECT_DIR/dist/dmg-stage.XXXXXX")
  trap 'rm -rf "$STAGING"' EXIT HUP INT TERM
  ditto "$APP_DIR" "$STAGING/Aieyes.app"
  ln -s /Applications "$STAGING/Applications"
  hdiutil create -volname Aieyes -srcfolder "$STAGING" -ov -format UDZO "$PROJECT_DIR/dist/Aieyes-$VERSION-macos-$ARCH.dmg"
  python3 scripts/macos-distribution.py dmg "$PROJECT_DIR/dist/Aieyes-$VERSION-macos-$ARCH.dmg"
fi
