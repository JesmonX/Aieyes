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
    .env_remove("AIEYES_AUTO_SCAN")
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
        .env_remove("PI_CONFIG_DIR")
        .env("COPILOT_HOME", root.join("home/.copilot"))
        .env("GH_CONFIG_DIR", root.join("home/.config/gh"))
        .env_remove("COPILOT_GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN");
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

#[cfg(unix)]
#[test]
fn antigravity_ide_only_is_discovered_without_a_cli_directory_and_keeps_credentials_private() {
    use base64::engine::general_purpose::STANDARD;
    use std::os::unix::fs::PermissionsExt;
    let root = setup();
    let p = root.path();
    let fixture = p.join("fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    // Only the isolated helper processes see this Python fixture. No real
    // system credential store is read, including on macOS with a live login.
    std::fs::write(
        fixture.join("sitecustomize.py"),
        r#"import subprocess
original_run = subprocess.run
def fixture_run(command, *args, **kwargs):
    if command[0] in ('/usr/bin/security', 'secret-tool'):
        return subprocess.CompletedProcess(command, 44, b'')
    return original_run(command, *args, **kwargs)
subprocess.run = fixture_run
"#,
    )
    .unwrap();
    let curl = fixture.join("curl");
    std::fs::write(&curl, r#"#!/usr/bin/env python3
import json, os, pathlib, sys
sys.stdin.read()
url = sys.argv[-1]
assert url in ('https://www.googleapis.com/oauth2/v2/userinfo', 'https://aicode.googleapis.com/v1:fetchLicenses', 'https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist', 'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary')
with (pathlib.Path(os.environ['AIEYES_AGY_FIXTURE']) / 'requests').open('a') as out: out.write(url + '\n')
print(json.dumps({'id':'ide-subject','email':'ide@example.test','verified_email':True,'licenses':[{'userTier':'g1-pro-tier'}],'paidTier':{'id':'g1-pro-tier','name':'Google AI Pro'},'groups':[{'displayName':'Gemini','buckets':[{'remainingFraction':0.6,'window':'5h'}]}]}))
"#).unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o700)).unwrap();
    let call = |method: &str, params: Value| {
        let output = command(p, method, &params)
            .env("AIEYES_AUTO_SCAN", "off")
            .env("PYTHONPATH", &fixture)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("AIEYES_AGY_FIXTURE", &fixture)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    fixture.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env_remove("GEMINI_API_KEY")
            .env_remove("AGY_ADC_AUTH")
            .env_remove("AGY_LLM_GATEWAY_API_KEY")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let mut settings = call("settings.get", json!({}));
    for agent in settings["agents"].as_array_mut().unwrap() {
        agent["enabled"] = json!(agent["provider"] == "antigravity");
        agent["machineIds"] = json!([]);
    }
    call("settings.save", settings);
    assert_eq!(call("accounts.autoScan", json!({}))["addedAccounts"], 0);
    assert!(
        !fixture.join("requests").exists(),
        "no store means no HTTP request"
    );
    let base = p.join("home").join(if cfg!(target_os = "macos") {
        "Library/Application Support"
    } else {
        ".config"
    });
    let path = base.join("Antigravity IDE/User/globalStorage/state.vscdb");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let field = |number: u8, value: &[u8]| {
        let mut data = vec![number << 3 | 2];
        let mut len = value.len();
        while len >= 128 {
            data.push((len as u8 & 127) | 128);
            len >>= 7;
        }
        data.push(len as u8);
        data.extend(value);
        data
    };
    // The IDE's native one-time migration writes JSON inside Topic/Row.
    // Current OAuthTokenInfo protobuf is covered independently in Python.
    let token = json!({"accessToken":"private-ide-access","refreshToken":"private-ide-refresh","tokenType":"Bearer","expiryDateSeconds":4102444800_i64}).to_string();
    let entry = [
        field(1, b"oauthTokenInfoSentinelKey"),
        field(2, &field(1, token.as_bytes())),
    ]
    .concat();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO ItemTable VALUES(?1,?2)",
        rusqlite::params![
            "antigravityUnifiedStateSync.oauthToken",
            STANDARD.encode(field(1, &entry))
        ],
    )
    .unwrap();
    drop(db);
    let original = std::fs::read(&path).unwrap();
    assert!(!p.join("home/.gemini").exists());
    let discovered = call("accounts.autoScan", json!({}));
    assert_eq!(discovered["addedAccounts"], 1, "{discovered}");
    assert_eq!(
        discovered["settings"]["accounts"][0]["name"],
        "ide@example.test"
    );
    assert_eq!(
        discovered["settings"]["sources"],
        json!([]),
        "IDE auth storage is not a history source"
    );
    let quota = call("quotas.refresh", json!({}));
    assert!(quota[0]["error"].is_null(), "{quota}");
    assert_eq!(quota[0]["identity"]["email"], "ide@example.test");
    assert_eq!(quota[0]["windows"][0]["usedPercent"], 40.0);
    let again = call("accounts.autoScan", json!({}));
    assert_eq!(again["addedAccounts"], 0);
    for value in [discovered, quota, again] {
        assert!(!value.to_string().contains("private-ide"));
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!p.join("home/.gemini").exists());
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
fn detection_is_automatic_idempotent_private_and_never_pending() {
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
    let result = rpc(p, "accounts.autoScan", json!({}));
    assert_eq!(result["addedAccounts"], 2);
    assert!(
        result["settings"]["nativeConnections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "observed")
    );
    let rows = result["settings"]["nativeConnections"].to_string();
    assert!(!rows.contains("待验证") && !rows.contains("扫描"));
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
    let again = rpc(p, "accounts.autoScan", json!({}));
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
    let preserved = rpc(p, "accounts.autoScan", json!({}));
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
    let r = rpc(p, "accounts.autoScan", json!({}));
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
    // With collection off there is nothing to detect, which is not an error.
    let idle = rpc(p, "accounts.autoScan", json!({}));
    assert_eq!(idle["results"], json!([]));
    assert_eq!(idle["addedAccounts"], 0);
}
#[test]
fn api_refs_do_not_create_fake_accounts_and_removed_credentials_are_remembered() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(
        p,
        "home/.codex/auth.json",
        &json!({"auth_mode":"apikey","OPENAI_API_KEY":"PRIVATE-API-KEY"}),
    );
    let r = rpc(p, "accounts.autoScan", json!({}));
    assert_eq!(r["addedAccounts"], 0);
    assert_eq!(r["addedConnections"], 1);
    assert!(!r.to_string().contains("PRIVATE-API-KEY"));
    std::fs::remove_file(p.join("home/.codex/auth.json")).unwrap();
    let r = rpc(
        p,
        "accounts.autoScan",
        json!({"targets":[["codex","local"]]}),
    );
    assert_eq!(
        r["settings"]["nativeConnections"][0]["status"],
        "remembered"
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
    let local = rpc(p, "accounts.autoScan", json!({}));
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
        "accounts.autoScan",
        &json!({"targets":[["codex","remote"]]}),
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
    let r = rpc(p, "accounts.autoScan", json!({}));
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
    let params = json!({});
    let mut children = vec![];
    for _ in 0..2 {
        children.push(
            command(p, "accounts.autoScan", &params)
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

#[test]
fn auto_scan_without_targets_is_not_an_error() {
    let root = setup();
    let p = root.path();
    let mut first = rpc(p, "settings.get", json!({}));
    first["localDeviceEnabled"] = json!(false);
    rpc(p, "settings.save", first);
    let r = rpc(p, "accounts.autoScan", json!({}));
    assert_eq!(r["results"], json!([]));
    assert_eq!(r["addedAccounts"], 0);
}
#[test]
fn half_written_credentials_keep_the_last_connection() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(p, "home/.codex/auth.json", &auth("user", "personal"));
    let r = rpc(p, "accounts.autoScan", json!({}));
    let before = r["settings"]["nativeConnections"].clone();
    assert_eq!(before.as_array().unwrap().len(), 1);
    std::fs::write(p.join("home/.codex/auth.json"), "{\"tokens\":{\"access_tok").unwrap();
    let r = rpc(p, "accounts.autoScan", json!({}));
    assert!(
        !r["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "failed")
    );
    assert!(
        r["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "retained")
    );
    assert_eq!(r["settings"]["nativeConnections"], before);
    assert_eq!(r["settings"]["accounts"].as_array().unwrap().len(), 1);
    // Still unreadable a day later: shown as stale with a hint, never as an error.
    let db = rusqlite::Connection::open(p.join("data/aieyes.sqlite")).unwrap();
    db.execute(
        "UPDATE kv SET value=?1 WHERE key LIKE 'native-failed-since:%'",
        [(aieyes_core::models::now() - 2 * 86400).to_string()],
    )
    .unwrap();
    drop(db);
    let r = rpc(p, "accounts.autoScan", json!({}));
    let row = &r["settings"]["nativeConnections"][0];
    assert_eq!(row["status"], "stale");
    assert!(row["note"].as_str().unwrap().contains("一次即可刷新"));
}
#[test]
fn credential_changes_are_detected_and_the_previous_account_is_remembered() {
    let root = setup();
    let p = root.path();
    let first = rpc(p, "settings.get", json!({}));
    save(p, first);
    write(p, "home/.codex/auth.json", &auth("first", "personal"));
    let r = rpc(p, "accounts.autoScan", json!({}));
    assert_eq!(r["settings"]["accounts"].as_array().unwrap().len(), 1);
    // A login switch in the native client; the status refresh notices the file change.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut next = auth("second", "personal");
    next["note"] = json!("longer file so the size changes too");
    write(p, "home/.codex/auth.json", &next);
    rpc(p, "accounts.status.refreshBatch", json!({}));
    let settings = rpc(p, "settings.get", json!({}));
    let accounts = settings["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    let rows = settings["nativeConnections"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().filter(|r| r["status"] == "remembered").count(),
        1
    );
    assert_eq!(rows.iter().filter(|r| r["status"] == "observed").count(), 1);
    let keys: std::collections::BTreeSet<_> =
        rows.iter().map(|r| r["accountKey"].to_string()).collect();
    assert_eq!(keys.len(), 2);
    assert!(accounts.iter().all(|a| {
        a["upstreamIdentity"]
            .as_str()
            .unwrap()
            .starts_with("openai:")
    }));
    // Switching back reuses the first account and drops its remembered copy.
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(p, "home/.codex/auth.json", &auth("first", "personal"));
    rpc(p, "accounts.status.refreshBatch", json!({}));
    let settings = rpc(p, "settings.get", json!({}));
    assert_eq!(settings["accounts"].as_array().unwrap().len(), 2);
    let rows = settings["nativeConnections"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|r| r["status"] == "observed").count(), 1);
    let keys: std::collections::BTreeSet<_> =
        rows.iter().map(|r| r["accountKey"].to_string()).collect();
    assert_eq!(keys.len(), 2);
}
#[test]
fn generic_github_token_variables_count_for_copilot_only_where_copilot_has_run() {
    let root = setup();
    let p = root.path();
    let mut settings = rpc(p, "settings.get", json!({}));
    for agent in settings["agents"].as_array_mut().unwrap() {
        let copilot = agent["provider"] == "copilot";
        agent["enabled"] = json!(copilot);
        agent["machineIds"] = json!(if copilot { vec!["local"] } else { vec![] });
    }
    rpc(p, "settings.save", settings);
    // Classic tokens are never sent to GitHub, so this test needs no network.
    let secret = "ghp_PRIVATE_GENERIC_TOKEN";
    let scan = || {
        let mut c = command(
            p,
            "accounts.autoScan",
            &json!({"targets":[["copilot","local"]]}),
        );
        c.env("COPILOT_HOME", p.join("home/.copilot"))
            .env_remove("GH_CONFIG_DIR")
            .env_remove("COPILOT_GITHUB_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env("GH_TOKEN", secret);
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!String::from_utf8_lossy(&out.stdout).contains(secret));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let copilot = |r: &Value| -> Vec<Value> {
        r["settings"]["nativeConnections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["provider"] == "copilot")
            .cloned()
            .collect()
    };
    // A token kept for other GitHub tools is not a Copilot login.
    assert!(copilot(&scan()).is_empty());
    std::fs::create_dir_all(p.join("home/.copilot")).unwrap();
    let rows = copilot(&scan());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["configPath"], "env:GH_TOKEN");
    assert!(
        rows[0]["accountKey"].is_null(),
        "classic tokens never become accounts"
    );
    assert!(rows[0]["note"].as_str().unwrap().contains("ghp_"));
    std::fs::write(
        p.join("home/.copilot/config.json"),
        json!({"copilot_tokens":{"https://github.com:cached-user":"ghp_PRIVATE_DISK"}}).to_string(),
    )
    .unwrap();
    let variables = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];
    for selected in 0..variables.len() {
        let mut c = command(
            p,
            "accounts.autoScan",
            &json!({"targets":[["copilot","local"]]}),
        );
        for name in &variables[selected..] {
            c.env(name, format!("ghp_PRIVATE_{name}"));
        }
        let out = c.output().unwrap();
        assert!(out.status.success());
        let raw = String::from_utf8(out.stdout).unwrap();
        assert!(!raw.contains("ghp_PRIVATE"));
        let result: Value = serde_json::from_str(&raw).unwrap();
        let processed: Vec<_> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["status"] == "added" || row["status"] == "existing")
            .collect();
        assert_eq!(
            processed[0]["configPath"],
            format!("env:{}", variables[selected])
        );
        let observed: Vec<_> = copilot(&result)
            .into_iter()
            .filter(|row| row["status"] == "observed")
            .collect();
        assert_eq!(
            observed.len(),
            2,
            "only the selected override and native file"
        );
        assert!(
            observed.iter().all(|row| row["accountKey"].is_null()),
            "cached logins cannot turn unsupported tokens into accounts"
        );
    }
}
