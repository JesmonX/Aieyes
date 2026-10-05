#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
VERSION=2.9.6
ARCHIVE=".build/Sparkle-$VERSION.tar.xz"
mkdir -p .build/sparkle-tools
if [ ! -f "$ARCHIVE" ]; then
  curl --fail --location --silent --show-error "https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz" -o "$ARCHIVE"
fi
python3 - "$ARCHIVE" <<'PY'
import hashlib, pathlib, sys
assert hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest() == '52bf9e88cdd972fc0c81501377a880e90d47031bd8ca5462488f843e2609e192', 'Sparkle tools checksum mismatch'
PY
tar -xf "$ARCHIVE" -C .build/sparkle-tools
