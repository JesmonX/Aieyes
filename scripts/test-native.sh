#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
mkdir -p .build/swift-module-cache
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/MonitorSelection.swift scripts/verify-selection.swift -o .build/verify-selection
.build/verify-selection
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-sessions.swift -o .build/verify-sessions
.build/verify-sessions
