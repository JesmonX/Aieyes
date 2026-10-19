use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
};

const ROOTS: &[(&str, &str)] = &[
    ("claude", "CLAUDE_CONFIG_DIR"),
    ("kimi", "KIMI_CODE_HOME"),
    ("codebuddy", "CODEBUDDY_CONFIG_DIR"),
    ("deepseek-harness", "DSH_HOME"),
    ("grok", "GROK_HOME"),
    ("qoder", "QODER_CONFIG_DIR"),
    ("opencode", "XDG_DATA_HOME"),
];

fn call(root: &Path, method: &str, params: Value, invalid_kimi: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aieyes-core"));
    command
        .args([
            "--data-dir",
            root.join("data").to_str().unwrap(),
            "--call",
            method,
            &params.to_string(),
        ])
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .stdin(Stdio::null());
    for (provider, key) in ROOTS {
        command.env(key, root.join("native").join(provider));
    }
    if invalid_kimi {
        command.env("KIMI_CODE_HOME", "relative-profile");
    }
    command.output().unwrap()
}
fn rpc(root: &Path, method: &str, params: Value) -> Value {
    let output = call(root, method, params, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn roots(root: &Path) {
    std::fs::create_dir_all(root.join("home")).unwrap();
    for (provider, _) in ROOTS {
        let path = root.join("native").join(provider);
        std::fs::create_dir_all(if *provider == "opencode" {
            path.join("opencode")
        } else {
            path
        })
        .unwrap();
    }
}

#[test]
fn discovery_and_directory_defaults_use_each_clients_effective_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    roots(root);
    let settings = rpc(root, "settings.get", json!({}));
    for (provider, _) in ROOTS {
        let expected = root.join("native").join(provider);
        let expected = if *provider == "opencode" {
            expected.join("opencode")
        } else {
            expected
        };
        let source = settings["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["provider"] == *provider)
            .unwrap();
        assert_eq!(source["path"], expected.to_str().unwrap());
        assert_eq!(
            rpc(root, "sources.defaultPath", json!({"provider":provider}))["path"],
            source["path"]
        );
        assert_eq!(
            rpc(
                root,
                "sources.defaultPath",
                json!({"provider":provider,"hostId":"ssh"})
            )["path"],
            aieyes_core::contracts::provider(provider)
                .unwrap()
                .default_path
        );
    }
}

#[test]
fn new_local_sources_follow_environment_but_explicit_sources_and_ssh_do_not_move() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    roots(root);
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["sources"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["provider"] != "kimi");
    rpc(root, "settings.save", settings);
    let mut settings = rpc(
        root,
        "agents.set",
        json!({"provider":"kimi","enabled":true,"machineIds":["local"]}),
    );
    let source = settings["sources"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s["provider"] == "kimi")
        .unwrap();
    assert_eq!(source["path"], root.join("native/kimi").to_str().unwrap());
    let explicit = root.join("explicit-history");
    source["path"] = json!(explicit);
    settings["hosts"] =
        json!([{"id":"ssh","name":"Uncontacted test host","target":"test.invalid","enabled":true}]);
    rpc(root, "settings.save", settings);
    let settings = rpc(
        root,
        "agents.set",
        json!({"provider":"kimi","enabled":true,"machineIds":["local","ssh"]}),
    );
    let sources = settings["sources"].as_array().unwrap();
    assert_eq!(
        sources
            .iter()
            .find(|s| s["provider"] == "kimi" && s["hostId"].is_null())
            .unwrap()["path"],
        json!(explicit)
    );
    assert_eq!(
        sources
            .iter()
            .find(|s| s["provider"] == "kimi" && s["hostId"] == "ssh")
            .unwrap()["path"],
        "~/.kimi-code"
    );
}

#[test]
fn invalid_local_override_never_discovers_the_legacy_default_or_saves_a_wrong_profile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    roots(root);
    std::fs::create_dir_all(root.join("home/.kimi-code")).unwrap();
    let output = call(root, "settings.get", json!({}), true);
    assert!(output.status.success());
    let before: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !before["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["provider"] == "kimi")
    );
    let output = call(
        root,
        "agents.set",
        json!({"provider":"kimi","enabled":true,"machineIds":["local"]}),
        true,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("KIMI_CODE_HOME"));
    let after = call(root, "settings.get", json!({}), true);
    assert_eq!(
        before,
        serde_json::from_slice::<Value>(&after.stdout).unwrap()
    );
}
