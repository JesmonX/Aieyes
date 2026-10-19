#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
export CLANG_MODULE_CACHE_PATH="${CLANG_MODULE_CACHE_PATH:-$PROJECT_DIR/.build/clang-cache}"
export SWIFTPM_MODULECACHE_OVERRIDE="${SWIFTPM_MODULECACHE_OVERRIDE:-$PROJECT_DIR/.build/swift-module-cache}"
mkdir -p .build/swift-module-cache
cargo build --workspace --locked
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/ProviderPresentation.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/QuotaNotifications.swift apps/macos/Sources/Aieyes/ConfigurationActions.swift apps/macos/Sources/Aieyes/DesignTokens.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/Typography.swift apps/macos/Sources/Aieyes/SessionActivity.swift apps/macos/Sources/Aieyes/CopyNotice.swift apps/macos/Sources/Aieyes/ContextMenu.swift apps/macos/Sources/Aieyes/PetWindow.swift scripts/verify-pets.swift -o .build/verify-pets
.build/verify-pets "$@"
