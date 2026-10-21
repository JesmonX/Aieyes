use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
};
fn command(root: &Path, method: &str, params: &Value) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_aieyes-core"));
    c.args([
        "--data-dir",
        root.join("data").to_str().unwrap(),
        "--call",
        method,
        &params.to_string(),
    ])
    .env("HOME", root.join("home"))
    .env("USERPROFILE", root.join("home"))
    .stdin(Stdio::null());
    for (name, path) in [
        ("CODEX_HOME", ".codex"),
        ("CLAUDE_CONFIG_DIR", ".claude"),
        ("KIMI_CODE_HOME", ".kimi-code"),
        ("GROK_HOME", ".grok"),
        ("QODER_CONFIG_DIR", ".qoder"),
        ("CODEBUDDY_CONFIG_DIR", ".codebuddy"),
        ("DSH_HOME", ".dsh"),
        ("PI_CODING_AGENT_DIR", ".pi/agent"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
    ] {
        c.env(name, root.join("home").join(path));
    }
    c.env_remove("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE")
        .env_remove("ZCODE_DATA_BASE_DIR")
        .env_remove("OMP_PROFILE")
        .env_remove("PI_PROFILE")
        .env_remove("PI_CONFIG_DIR");
    c
}
fn rpc(root: &Path, method: &str, params: Value) -> Value {
    let out = command(root, method, &params).output().unwrap();
    assert!(
        out.status.success(),
        "{method}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("home")).unwrap();
    root
}
fn save(root: &Path, mut settings: Value) {
    for agent in settings["agents"].as_array_mut().unwrap() {
        agent["enabled"] = json!(agent["provider"] == "codex");
        agent["machineIds"] = json!([]);
    }
    rpc(root, "settings.save", settings);
}
fn jwt(v: Value) -> String {
    format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(v.to_string()))
}
fn auth(user: &str, workspace: &str) -> Value {
    json!({"tokens":{"account_id":workspace,"access_token":jwt(json!({"sub":user,"exp":4102444800_i64,"https://api.openai.com/auth":{"chatgpt_user_id":user,"chatgpt_account_id":workspace}})),"id_token":jwt(json!({"sub":user,"email":"user@example.test"}))}})
}
fn write(root: &Path, path: &str, v: &Value) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, v.to_string()).unwrap();
}
#[test]
fn defaults_are_fresh_only_and_scanning_is_explicit_idempotent_and_private() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    assert!(
        first["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["provider"] != "custom")
            .all(|a| a["enabled"] == true)
    );
    assert_eq!(first["accounts"], json!([]));
    save(p, first);
    let native = auth("user", "personal");
    write(p, "home/.codex/auth.json", &native);
    write(
        p,
        "home/.codex/.aieyes/accounts/backup/auth.json",
        &auth("other", "work"),
    );
    assert_eq!(rpc(p, "settings.get", json!({}))["accounts"], json!([]));
    let result = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    assert_eq!(result["addedAccounts"], 2);
    assert_eq!(result["addedConnections"], 2);
    let settings = &result["settings"];
    assert_eq!(
        settings["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["provider"] == "codex")
            .unwrap()["machineIds"],
        json!(["local"])
    );
    assert!(
        settings["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["accountId"] == "")
    );
    assert!(
        !settings
            .to_string()
            .contains(native["tokens"]["access_token"].as_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(p.join("home/.codex/auth.json")).unwrap(),
        native.to_string()
    );
    let again = rpc(
        p,
        "accounts.scan",
        json!({"scope":"agent","provider":"codex"}),
    );
    assert_eq!(again["addedAccounts"], 0);
    assert_eq!(again["addedConnections"], 0);
    assert_eq!(
        rpc(p, "accounts.status.get", json!({}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["nativeConnectionId"].is_string())
            .count(),
        2
    );
    let key = again["settings"]["accounts"][0]["id"].as_str().unwrap();
    let mut settings = again["settings"].clone();
    settings["accounts"][0]["name"] = json!("My name");
    settings["accounts"][0]["archived"] = json!(true);
    rpc(p, "settings.save", settings);
    let preserved = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    let a = preserved["settings"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == key)
        .unwrap();
    assert_eq!(a["name"], "My name");
    assert_eq!(a["archived"], true);
    let refs = &preserved["settings"]["nativeConnections"];
    let id = refs[0]["id"].as_str().unwrap();
    let r = rpc(p, "accounts.native.remove", json!({"connectionId":id}));
    assert_eq!(
        r["settings"]["nativeConnections"].as_array().unwrap().len(),
        1
    );
    assert!(p.join("home/.codex/auth.json").exists());
    let remaining = r["settings"]["nativeConnections"][0]["id"].clone();
    let removed = rpc(
        p,
        "accounts.native.remove",
        json!({"connectionIds":[remaining.clone(),remaining,"already-removed"]}),
    );
    assert_eq!(removed["removed"], 1);
    assert_eq!(removed["settings"]["nativeConnections"], json!([]));
    assert_eq!(removed["settings"]["accounts"], r["settings"]["accounts"]);
    assert_eq!(
        std::fs::read_to_string(p.join("home/.codex/auth.json")).unwrap(),
        native.to_string()
    );
    assert!(
        rpc(p, "settings.get", json!({}))["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["provider"] != "codex")
            .all(|a| a["enabled"] == false)
    );
}
#[test]
fn machine_scope_does_not_enable_global_agents_and_partial_failures_keep_successes() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(p, "home/.codex/auth.json", &auth("user", "personal"));
    std::fs::write(p.join("home/.codex/config.toml"), "PRIVATE-SECRET = [").unwrap();
    write(
        p,
        "home/.kimi-code/auth.json",
        &json!({"accessToken":"DO-NOT-IMPORT"}),
    );
    let r = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    assert_eq!(r["addedAccounts"], 1);
    assert!(
        r["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "failed")
    );
    assert!(!r.to_string().contains("PRIVATE-SECRET"));
    assert!(
        !r["settings"]["nativeConnections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["provider"] == "kimi")
    );
    let mut settings = r["settings"].clone();
    settings["localDeviceEnabled"] = json!(false);
    rpc(p, "settings.save", settings);
    assert!(
        !command(
            p,
            "accounts.scan",
            &json!({"scope":"machine","machineId":"local"})
        )
        .output()
        .unwrap()
        .status
        .success()
    );
}
#[test]
fn api_refs_do_not_create_fake_accounts_and_changed_credentials_require_rescan() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(
        p,
        "home/.codex/auth.json",
        &json!({"auth_mode":"apikey","OPENAI_API_KEY":"PRIVATE-API-KEY"}),
    );
    let r = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    assert_eq!(r["addedAccounts"], 0);
    assert_eq!(r["addedConnections"], 1);
    assert!(!r.to_string().contains("PRIVATE-API-KEY"));
    std::fs::remove_file(p.join("home/.codex/auth.json")).unwrap();
    let r = rpc(
        p,
        "accounts.scan",
        json!({"scope":"agent","provider":"codex"}),
    );
    assert_eq!(
        r["settings"]["nativeConnections"][0]["status"],
        "unavailable"
    );
    let mut forged = r["settings"].clone();
    forged["nativeConnections"] = json!([]);
    assert!(
        !command(
            p,
            "settings.patch",
            &json!({"base":r["settings"],"settings":forged})
        )
        .output()
        .unwrap()
        .status
        .success()
    );
}
#[cfg(unix)]
#[test]
fn ssh_uses_remote_home_and_merges_the_same_identity_without_rewriting_files() {
    use std::os::unix::fs::PermissionsExt;
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(p, "home/.codex/auth.json", &auth("user", "personal"));
    let local = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    let bin = p.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\nfor last do :; done\nexec /bin/sh -c \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut settings = local["settings"].clone();
    settings["hosts"] = json!([{"id":"remote","name":"Fixture SSH","target":"fixture.invalid","enabled":true,"shell":"/bin/sh"}]);
    settings["sources"].as_array_mut().unwrap().push(json!({"id":"existing-remote","provider":"codex","hostId":"remote","path":"~/.codex/sessions","accountId":"","enabled":true}));
    rpc(p, "settings.save", settings);
    let out = command(
        p,
        "accounts.scan",
        &json!({"scope":"machine","machineId":"remote"}),
    )
    .env(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    )
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["addedAccounts"], 0);
    assert_eq!(r["addedConnections"], 1);
    let remote_sources: Vec<_> = r["settings"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["hostId"] == "remote")
        .collect();
    assert_eq!(remote_sources.len(), 1);
    assert_eq!(remote_sources[0]["id"], "existing-remote");
    assert_eq!(r["settings"]["accounts"].as_array().unwrap().len(), 1);
    assert!(
        r["settings"]["nativeConnections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["machineId"] == "remote")
    );
}

#[test]
fn existing_managed_profiles_are_bound_without_cli_or_credential_writes() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    let native = auth("saved-user", "saved-workspace");
    let identity = aieyes_core::codex_auth::files::identity(&native).unwrap();
    write(
        p,
        "home/.codex/.aieyes/state/home.json",
        &json!({"homeId":"fixturehome","version":1}),
    );
    write(p, "home/.codex/.aieyes/accounts/backup/auth.json", &native);
    write(
        p,
        "home/.codex/.aieyes/accounts/backup/metadata.json",
        &json!({"id":"backup","name":"Saved","identity":identity,"updatedAt":1}),
    );
    let r = rpc(
        p,
        "accounts.scan",
        json!({"scope":"machine","machineId":"local"}),
    );
    assert_eq!(r["addedAccounts"], 1);
    assert_eq!(
        r["settings"]["accounts"][0]["connections"][0]["profileId"],
        "fixturehome:backup"
    );
    assert_eq!(r["settings"]["sources"][0]["codexHomeId"], "fixturehome");
    assert_eq!(
        std::fs::read_to_string(p.join("home/.codex/.aieyes/accounts/backup/auth.json")).unwrap(),
        native.to_string()
    );
    assert!(!p.join("home/.codex/auth.json").exists());
    let targets = rpc(p, "accounts.targets", json!({}));
    assert!(
        targets
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["accountName"].is_string())
    );
}
#[test]
fn overlapping_processes_do_not_duplicate_accounts_or_references() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(p, "home/.codex/auth.json", &auth("concurrent", "workspace"));
    let params = json!({"scope":"machine","machineId":"local"});
    let mut children = vec![];
    for _ in 0..2 {
        children.push(
            command(p, "accounts.scan", &params)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let settings = rpc(p, "settings.get", json!({}));
    assert_eq!(settings["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(settings["nativeConnections"].as_array().unwrap().len(), 1);
}
