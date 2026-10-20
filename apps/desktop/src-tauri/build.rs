fn main() {
    // Stage platform assets for direct cargo builds as well as the Tauri CLI.
    // The shared source files (including licensed Windows fonts) stay untouched.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        let source = std::path::Path::new("../web");
        let destination =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("linux-web");
        if destination.exists() {
            std::fs::remove_dir_all(&destination).unwrap();
        }
        stage_web(source, &destination);
        let mut config: serde_json::Value = std::env::var("TAURI_CONFIG")
            .ok()
            .map(|s| serde_json::from_str(&s).expect("Invalid TAURI_CONFIG"))
            .unwrap_or_else(|| serde_json::json!({}));
        config["build"]["frontendDist"] = serde_json::json!(destination);
        let config = config.to_string();
        unsafe {
            std::env::set_var("TAURI_CONFIG", &config);
        }
        println!("cargo:rustc-env=TAURI_CONFIG={config}");
        println!("cargo:rerun-if-changed=../web");
    }
    println!("cargo:rerun-if-env-changed=AIEYES_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=AIEYES_REQUIRE_UPDATER");
    if std::env::var("AIEYES_REQUIRE_UPDATER").as_deref() == Ok("1")
        && std::env::var("PROFILE").as_deref() == Ok("release")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
    {
        assert!(
            !std::env::var("AIEYES_UPDATER_PUBLIC_KEY")
                .unwrap_or_default()
                .trim()
                .is_empty(),
            "Official releases require AIEYES_UPDATER_PUBLIC_KEY"
        );
    }
    tauri_build::build()
}

fn stage_web(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "fonts" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        let output = destination.join(&name);
        if entry.file_type().unwrap().is_dir() {
            stage_web(&entry.path(), &output);
        } else if name == "tokens.css" {
            let css = std::fs::read_to_string(entry.path()).unwrap();
            let css = css
                .lines()
                .filter(|line| !line.trim_start().starts_with("@font-face"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(output, css).unwrap();
        } else {
            std::fs::copy(entry.path(), output).unwrap();
        }
    }
}
