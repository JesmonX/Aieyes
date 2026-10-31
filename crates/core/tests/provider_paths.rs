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
    ("copilot", "COPILOT_HOME"),
    ("cline", "CLINE_DATA_DIR"),
    ("kiro", "KIRO_HOME"),
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
fn discovery_never_creates_accounts_from_local_credentials() {
    for auth in [
        None,
        Some(json!({"auth_mode":"apikey","OPENAI_API_KEY":"fixture"})),
        Some(json!({"auth_mode":"chatgpt","tokens":{"account_id":"fixture-account"}})),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        roots(root);
        let codex = root.join("home/.codex");
        let agy = root.join("home/.gemini/antigravity-cli");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&agy).unwrap();
        let claude = root.join("native/claude/.credentials.json");
        let claude_auth = json!({"claudeAiOauth":{"accessToken":"fixture"}}).to_string();
        std::fs::write(&claude, &claude_auth).unwrap();
        if let Some(auth) = &auth {
            std::fs::write(codex.join("auth.json"), auth.to_string()).unwrap();
        }

        // Every RPC starts a new process, covering both initialization and restart.
        for _ in 0..2 {
            let settings = rpc(root, "settings.get", json!({}));
            assert_eq!(settings["accounts"], json!([]));
            for provider in ["codex", "claude", "antigravity"] {
                let source = settings["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["provider"] == provider)
                    .unwrap();
                assert_eq!(source["accountId"], "", "{provider}");
            }
            assert_eq!(rpc(root, "dashboard", json!({}))["quotas"], json!([]));
            assert_eq!(rpc(root, "accounts.status.get", json!({})), json!([]));
        }
        assert_eq!(std::fs::read_to_string(&claude).unwrap(), claude_auth);
        if let Some(auth) = &auth {
            assert_eq!(
                std::fs::read_to_string(codex.join("auth.json")).unwrap(),
                auth.to_string()
            );
        }

        // Clearing only Aieyes data must not re-import the native logins.
        std::fs::remove_dir_all(root.join("data")).unwrap();
        assert_eq!(rpc(root, "settings.get", json!({}))["accounts"], json!([]));

        // Explicit account creation and its saved name still survive a restart.
        let created = rpc(
            root,
            "accounts.connect",
            json!({"provider":"antigravity","name":"My account","sourceIds":["local-antigravity"]}),
        );
        let settings = rpc(root, "settings.get", json!({}));
        assert_eq!(settings["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(settings["accounts"][0]["id"], created["accountId"]);
        assert_eq!(settings["accounts"][0]["name"], "My account");
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
        if !aieyes_core::contracts::provider(provider)
            .unwrap()
            .discover_local
        {
            // Registered ahead of their history adapters: no discovered source yet.
            assert!(
                !settings["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["provider"] == *provider),
                "{provider}"
            );
            assert_eq!(
                rpc(root, "sources.defaultPath", json!({"provider":provider}))["path"],
                expected.to_str().unwrap()
            );
            continue;
        }
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
    assert_eq!(
        source["path"],
        root.join("native").join("kimi").to_str().unwrap()
    );
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
