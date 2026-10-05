fn main() {
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
