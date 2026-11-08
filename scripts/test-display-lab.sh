#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
. scripts/native-verifiers.sh
build_native_app
for verifier in dock-presence display-lab; do
  build_native_verifier "$verifier"
  ".build/verify-$verifier"
done
python3 scripts/check-localizations.py
