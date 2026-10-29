#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
. scripts/native-verifiers.sh
cargo build --workspace --locked
build_native_app
MODEL_FIXTURE=$(mktemp -d "$PROJECT_DIR/.build/model-fixture.XXXXXX")
trap 'rm -rf "$MODEL_FIXTURE"' EXIT HUP INT TERM
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call settings.save '{"version":6,"sources":[],"accounts":[],"hosts":[]}' > /dev/null
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call settings.get > "$MODEL_FIXTURE/settings.json"
target/debug/aieyes-core --data-dir "$MODEL_FIXTURE" --call dashboard > "$MODEL_FIXTURE/dashboard.json"
build_native_verifier models
.build/verify-models "$MODEL_FIXTURE/dashboard.json" "$MODEL_FIXTURE/settings.json"
for verifier in localization dock-presence palette selection sessions proxy; do
  build_native_verifier "$verifier"
  ".build/verify-$verifier"
done
build_native_verifier prices
AIEYES_CORE_PATH="$PROJECT_DIR/scripts/pricing-test-core.py" AIEYES_PRICING_ISOLATION=1 .build/verify-prices
for verifier in macos-ui configuration wakeups setup-guide updates credit-format reset-format server-format configuration-lane; do
  build_native_verifier "$verifier"
  ".build/verify-$verifier"
done

AIEYES_SKIP_BUILD=1 sh scripts/test-native-pets.sh
python3 scripts/test-dashboard-cache.py

AIEYES_SKIP_BUILD=1 sh scripts/test-native-quota-layout.sh
AIEYES_SKIP_BUILD=1 sh scripts/test-cpu-optimization.sh
