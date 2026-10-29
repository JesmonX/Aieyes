#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
. scripts/native-verifiers.sh
build_native_app
build_native_verifier quota-layout
.build/verify-quota-layout
