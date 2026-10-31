fn main() {
    let manifest = "../../../apps/desktop/src-tauri/windows-app-manifest.xml";
    println!("cargo:rerun-if-changed={manifest}");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Subclass helpers require the same Common Controls activation context
        // as the real desktop binary, rather than the legacy system DLL.
        let path = std::path::Path::new(manifest).canonicalize().unwrap();
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", path.display());
    }
}
