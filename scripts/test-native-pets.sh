#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
. scripts/native-verifiers.sh
cargo build --workspace --locked
build_native_app
build_native_verifier pets
.build/verify-pets "$@"
