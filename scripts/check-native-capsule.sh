#!/bin/sh
# Type-check the real Windows capsule against Windows bindings without a C linker.
# AppHandle/desktop glue is stubbed; this does not replace Windows application tests.
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
python3 - <<'PY'
from pathlib import Path
import json
import re
project=Path.cwd()
root=project/'.build/capsule-typecheck'
(root/'src').mkdir(parents=True,exist_ok=True)
manifest=(project/'apps/desktop/src-tauri/Cargo.toml').read_text()
windows=re.search(r'windows = \{ version = "([^"]+)", features = (\[.*?\]) \}',manifest)
assert windows, 'Expected inline Windows dependency declaration'
serde=re.search(r'^serde_json = "([^"]+)"',manifest,re.M)
(root/'Cargo.toml').write_text('[package]\nname="aieyes-capsule-typecheck"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nserde_json='+json.dumps(serde.group(1))+'\nwindows={version='+json.dumps(windows.group(1))+',features='+windows.group(2)+'}\n[workspace]\n')
(root/'src/lib.rs').write_text('''extern crate self as tauri;
#[derive(Clone)] pub struct AppHandle;
pub trait Emitter { fn emit<S>(&self, _: &str, _: &S) -> Result<(), String> { Ok(()) } }
impl Emitter for AppHandle {}
pub trait Manager { fn try_state<T>(&self) -> Option<T> { None } }
impl Manager for AppHandle {}
pub mod desktop {
    pub fn action(_: &crate::AppHandle, _: &str) -> Result<(), String> { Ok(()) }
    pub fn native_interaction(_: &crate::AppHandle, _: bool) {}
    pub fn capsule_moved(_: &crate::AppHandle, _: i32, _: i32) {}
}
#[path = '''+json.dumps(str(project/'apps/desktop/src-tauri/src/capsule.rs'))+''']
pub mod capsule;
''')
PY
CARGO_TARGET_DIR="$PROJECT_DIR/.build/windows-capsule-typecheck" cargo check --manifest-path .build/capsule-typecheck/Cargo.toml --target x86_64-pc-windows-gnu --offline
