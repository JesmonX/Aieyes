//! A Qoder quota check runs qodercli in whichever lane process asked for it.
//! The control session it reports must be marked as Aieyes' own there, so the
//! session monitor of another process never counts its transcript.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

fn rpc(root: &Path, method: &str, params: Value) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_aieyes-core"))
        .args([
            "--data-dir",
            root.join("data").to_str().unwrap(),
            "--call",
            method,
            &params.to_string(),
        ])
        .env("HOME", root.join("home"))
        .env_remove("QODER_CONFIG_DIR")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn transcript(dir: &Path, session: &str) {
    fs::create_dir_all(dir).unwrap();
    let record = json!({"type":"user","sessionId":session,"entrypoint":"cli","message":{"role":"user","content":"PRIVATE_PROMPT"}});
    fs::write(dir.join(format!("{session}.jsonl")), format!("{record}\n")).unwrap();
}

/// Live sessions by transcript session id (the wire id is a hash of it).
fn listed(root: &Path) -> Vec<&'static str> {
    let snapshot = rpc(root, "sessions.list", json!({}));
    assert!(!snapshot.to_string().contains("PRIVATE_PROMPT"));
    let ids: Vec<_> = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect();
    ["qoder-control-run", "user-run"]
        .into_iter()
        .filter(|session| {
            ids.contains(&aieyes_core::models::hash(&format!(
                "local:qoder:{session}"
            )))
        })
        .collect()
}

#[test]
fn quota_check_control_session_is_excluded_in_another_process() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home");
    let qoder = home.join(".qoder");
    // The official installer's entry point, reached through PATH_FALLBACK.
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(
        bin.join("qodercli"),
        r#"#!/bin/sh
IFS= read -r init
printf '%s\n' '{"type":"system","subtype":"init","session_id":"qoder-control-run"}'
printf '%s\n' '{"type":"control_response","response":{"request_id":"aieyes-init","subtype":"success","response":{}}}'
IFS= read -r usage
printf '%s\n' '{"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":{"userId":"fixture","userType":"personal_standard","userQuota":{"unit":"credits","total":100,"used":25,"remaining":75}}}}}'
"#,
    )
    .unwrap();
    fs::set_permissions(bin.join("qodercli"), fs::Permissions::from_mode(0o700)).unwrap();
    // Defense in depth: a CLI that ignores --no-session-persistence.
    transcript(&qoder.join("projects/-probe"), "qoder-control-run");
    transcript(&qoder.join("projects/-work"), "user-run");
    let qoder_path = qoder.to_string_lossy();
    rpc(
        root,
        "settings.save",
        json!({
            "accounts":[{"id":"q","provider":"qoder","name":"Qoder","quotaEnabled":true}],
            "sources":[
                {"id":"quota","provider":"qoder","name":"Qoder quota","accountId":"q","path":"","enabled":true,
                 "subscription":{"mode":"cli","nativeConfigDir":qoder_path}},
                {"id":"history","provider":"qoder","name":"Qoder","path":qoder_path,"enabled":true}
            ],
            "hosts":[],
            "agents":[{"provider":"qoder","enabled":true,"machineIds":["local"]}]
        }),
    );
    assert_eq!(listed(root), ["qoder-control-run", "user-run"]);
    // The quota process exits right after its call; marks must already be durable.
    rpc(root, "quotas.refresh", json!({"accountKey":"qoder:q"}));
    assert_eq!(listed(root), ["user-run"]);
    assert!(home.join(".aieyes/qoder-probe").is_dir());
}
