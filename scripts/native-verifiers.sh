# Sourced by native test scripts after `cd "$PROJECT_DIR"`.
# The debug app is compiled once; each verifier imports it with @testable instead of recompiling app sources.
export CLANG_MODULE_CACHE_PATH="${CLANG_MODULE_CACHE_PATH:-$PROJECT_DIR/.build/clang-cache}"
export SWIFTPM_MODULECACHE_OVERRIDE="${SWIFTPM_MODULECACHE_OVERRIDE:-$PROJECT_DIR/.build/swift-module-cache}"
export MACOSX_DEPLOYMENT_TARGET=14.0
mkdir -p "$CLANG_MODULE_CACHE_PATH" "$SWIFTPM_MODULECACHE_OVERRIDE" .build/swift-module-cache
NATIVE_BUILD_DIR="$PROJECT_DIR/.build/swift/debug"
SPARKLE_FRAMEWORK="$PROJECT_DIR/.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64"

# AIEYES_SKIP_BUILD=1 reuses objects from a caller that already built the app.
build_native_app() {
  [ "${AIEYES_SKIP_BUILD:-0}" = 1 ] && return 0
  swift build --package-path apps/macos --scratch-path "$PROJECT_DIR/.build/swift" --cache-path "$PROJECT_DIR/.build/spm-cache" --config-path "$PROJECT_DIR/.build/spm-config" --security-path "$PROJECT_DIR/.build/spm-security" --disable-sandbox --disable-automatic-resolution -c debug -Xlinker -rpath -Xlinker @executable_path/../Frameworks
}

# build_native_verifier NAME compiles scripts/verify-NAME.swift to .build/verify-NAME.
build_native_verifier() {
  verifier=$1
  set --
  for file in "$NATIVE_BUILD_DIR"/Aieyes.build/*.o; do
    case "$file" in */AieyesApp.swift.o) ;; *) set -- "$@" "$file" ;; esac
  done
  swiftc -parse-as-library -I "$NATIVE_BUILD_DIR/Modules" -module-cache-path .build/swift-module-cache -F "$SPARKLE_FRAMEWORK" -framework Sparkle -Xlinker -rpath -Xlinker "$SPARKLE_FRAMEWORK" "$@" "scripts/verify-$verifier.swift" -o ".build/verify-$verifier"
}
