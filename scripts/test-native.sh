#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
mkdir -p .build/swift-module-cache
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Palette.swift scripts/verify-palette.swift -o .build/verify-palette
.build/verify-palette
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift scripts/verify-selection.swift -o .build/verify-selection
.build/verify-selection
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-sessions.swift -o .build/verify-sessions
.build/verify-sessions
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift scripts/verify-proxy.swift -o .build/verify-proxy
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-prices.swift -o .build/verify-prices
AIEYES_CORE_PATH="$PROJECT_DIR/scripts/pricing-test-core.py" AIEYES_PRICING_ISOLATION=1 .build/verify-prices
.build/verify-proxy
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-macos-ui.swift -o .build/verify-macos-ui
.build/verify-macos-ui
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"
swiftc -module-cache-path .build/swift-module-cache -F "$SPARKLE_FRAMEWORK" -framework Sparkle -Xlinker -rpath -Xlinker "$SPARKLE_FRAMEWORK" apps/macos/Sources/Aieyes/Typography.swift apps/macos/Sources/Aieyes/UpdateTransport.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/AppUpdater.swift scripts/verify-updates.swift -o .build/verify-updates
.build/verify-updates

swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift scripts/verify-credit-format.swift -o .build/verify-credit-format
.build/verify-credit-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift scripts/verify-reset-format.swift -o .build/verify-reset-format
.build/verify-reset-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift scripts/verify-server-format.swift -o .build/verify-server-format
.build/verify-server-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-configuration-lane.swift -o .build/verify-configuration-lane
.build/verify-configuration-lane
