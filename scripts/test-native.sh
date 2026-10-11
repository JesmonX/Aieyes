#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
mkdir -p .build/swift-module-cache
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace --locked
MODEL_FIXTURE=$(mktemp -d "$PROJECT_DIR/.build/model-fixture.XXXXXX")
trap 'rm -rf "$MODEL_FIXTURE"' EXIT HUP INT TERM
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call settings.save '{"version":6,"sources":[],"accounts":[],"hosts":[]}' > /dev/null
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call settings.get > "$MODEL_FIXTURE/settings.json"
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call dashboard > "$MODEL_FIXTURE/dashboard.json"
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-models.swift -o .build/verify-models
.build/verify-models "$MODEL_FIXTURE/dashboard.json" "$MODEL_FIXTURE/settings.json"
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/DockPresence.swift scripts/verify-dock-presence.swift -o .build/verify-dock-presence
.build/verify-dock-presence
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Palette.swift scripts/verify-palette.swift -o .build/verify-palette
.build/verify-palette
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-selection.swift -o .build/verify-selection
.build/verify-selection
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-sessions.swift -o .build/verify-sessions
.build/verify-sessions
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-proxy.swift -o .build/verify-proxy
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-prices.swift -o .build/verify-prices
AIEYES_CORE_PATH="$PROJECT_DIR/scripts/pricing-test-core.py" AIEYES_PRICING_ISOLATION=1 .build/verify-prices
.build/verify-proxy
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-macos-ui.swift -o .build/verify-macos-ui
.build/verify-macos-ui
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"
swiftc -module-cache-path .build/swift-module-cache -F "$SPARKLE_FRAMEWORK" -framework Sparkle -Xlinker -rpath -Xlinker "$SPARKLE_FRAMEWORK" apps/macos/Sources/Aieyes/Typography.swift apps/macos/Sources/Aieyes/UpdateTransport.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/AppUpdater.swift scripts/verify-updates.swift -o .build/verify-updates
.build/verify-updates

swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-credit-format.swift -o .build/verify-credit-format
.build/verify-credit-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-reset-format.swift -o .build/verify-reset-format
.build/verify-reset-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift scripts/verify-server-format.swift -o .build/verify-server-format
.build/verify-server-format
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/Palette.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-configuration-lane.swift -o .build/verify-configuration-lane
.build/verify-configuration-lane
