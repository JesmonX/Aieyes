# macOS Tauri comparison build

Build the optimized Tauri 2 frontend on the current Mac:

```sh
sh scripts/build-macos-tauri.sh
# Cached dependencies only:
AIEYES_OFFLINE=1 sh scripts/build-macos-tauri.sh
```

Requires Rust, Node.js, Xcode command line tools and the Tauri v2 CLI. The script
also recognizes the CLI installed under `apps/desktop/node_modules` or
`.build/tauri-cli/node_modules`. Outputs are `dist/Aieyes Tauri.app` and
`dist/Aieyes-<version>-macos-tauri-<architecture>.zip`. This is an ad hoc signed
local comparison build; it does not have a notarized distribution or automatic
updates. The SwiftUI build remains available through `scripts/build-macos.sh`.

The macOS Tauri frontend uses the same Rust core and the native release app's
`~/Library/Application Support/Aieyes` data directory by default. Its separate
bundle identifier keeps its WebView preferences separate. To compare the native
debug app's dataset instead, use the supplied launcher:

```sh
sh scripts/run-macos-tauri.sh "$PWD/.local/app"
```

Close the other frontend before opening the comparison build against the same
data directory, so duplicate background polling and account refreshes do not
affect the comparison. Keep the same window size, page, expanded cards, accounts,
machines and refresh settings. Compare scrolling/clicks after the initial load
has settled, then repeat while collecting history or refreshing accounts.

This holds the machine and stored data constant. It can help identify frontend
cost, but does not establish the cause of any difference from Windows: Tauri
uses the system WKWebView on macOS, whereas Windows uses WebView2. Build
optimization, graphics drivers and background work can still affect the result.

The overview packs consecutive single-account provider groups into shared rows
without changing provider order. Folded AGY cards retain every model quota group.
AGY session monitoring uses lifecycle timestamps from step metadata instead of
database file modification times when available. Unconfirmed AGY sessions with
no live-process evidence leave the live summary after three minutes; explicit
waiting states remain subject to the existing lifecycle rules. Older database
schemas without timestamps retain the file-time fallback.
