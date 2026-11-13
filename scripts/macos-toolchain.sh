#!/bin/sh
# Source from native build entry points so debug and release use the same SDK generation.
set -eu
AIEYES_XCODE_VERSION=$(xcodebuild -version | awk '$1 == "Xcode" { print $2 }')
case "$AIEYES_XCODE_VERSION" in
  26.*) ;;
  *) echo "Aieyes requires Xcode 26.x; selected: ${AIEYES_XCODE_VERSION:-unavailable}. Select it with DEVELOPER_DIR or xcode-select." >&2; exit 1 ;;
esac
AIEYES_MACOS_SDK_VERSION=$(xcrun --sdk macosx --show-sdk-version)
case "$AIEYES_MACOS_SDK_VERSION" in
  26.*) ;;
  *) echo "Aieyes requires macOS SDK 26.x; selected: $AIEYES_MACOS_SDK_VERSION." >&2; exit 1 ;;
esac
# Override stale SDKROOT values inherited from another development environment.
SDKROOT=$(xcrun --sdk macosx --show-sdk-path)
export SDKROOT
export MACOSX_DEPLOYMENT_TARGET=14.0
echo "Aieyes toolchain: Xcode $AIEYES_XCODE_VERSION, macOS SDK $AIEYES_MACOS_SDK_VERSION, deployment target $MACOSX_DEPLOYMENT_TARGET"
