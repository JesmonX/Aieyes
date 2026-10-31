//! Claude Code on macOS keeps its login in the Keychain. The account is identified
//! from `~/.claude.json` without any secret, and nothing here reads the real Keychain:
//! only `dismiss`, consent-gated paths and the fixture switch are exercised.
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
};

fn rpc(root: &Path, method: &str, params: Value) -> Value {
    rpc_with(root, method, params, &[])
}

fn rpc_with(root: &Path, method: &str, params: Value, env: &[(&str, &str)]) -> Value {
    let home = root.join("home");
    let out = Command::new(env!("CARGO_BIN_EXE_aieyes-core"))
        .args([
            "--data-dir",
            root.join("data").to_str().unwrap(),
            "--call",
            method,
            &params.to_string(),
        ])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("AIEYES_AUTO_SCAN")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{method}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn write(root: &Path, path: &str, v: &Value) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, v.to_string()).unwrap();
}

fn claude_rows(settings: &Value) -> Vec<Value> {
    settings["nativeConnections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["provider"] == "claude")
        .cloned()
        .collect()
}

#[test]
fn keychain_login_is_identified_without_secrets_and_never_read_without_consent() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    std::fs::create_dir_all(p.join("home/.claude")).unwrap();
    write(
        p,
        "home/.claude.json",
        &json!({"numStartups":3,"oauthAccount":{"accountUuid":"uuid-1","emailAddress":"claude@example.test","organizationUuid":"org-1"}}),
    );
    // UI fixtures (`AIEYES_AUTO_SCAN=off`) never trigger the launch-time Keychain read.
    let startup = rpc_with(
        p,
        "claudeAuth.keychain",
        json!({"startup":true}),
        &[("AIEYES_AUTO_SCAN", "off")],
    );
    assert_eq!(startup["asked"], false);
    assert!(
        rpc(p, "settings.get", json!({}))
            .get("claudeKeychain")
            .is_none()
    );
    let scan = rpc(
        p,
        "accounts.autoScan",
        json!({"targets":[["claude","local"]]}),
    );
    let settings = rpc(p, "settings.get", json!({}));
    let rows = claude_rows(&settings);
    if !cfg!(target_os = "macos") {
        // Elsewhere the login is `.credentials.json`; an account file alone is no login.
        assert!(rows.is_empty());
        assert_eq!(scan["results"][0]["message"], "未登录或使用 API key");
        let status = rpc(p, "claudeAuth.keychain", json!({"dismiss":true}));
        assert_eq!(status["supported"], false);
        assert_eq!(status["asked"], false);
        return;
    }
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["kind"], "login");
    assert_eq!(row["status"], "observed");
    assert_eq!(row["hasCredential"], false);
    assert_eq!(row["email"], "claude@example.test");
    assert!(
        row["configPath"]
            .as_str()
            .unwrap()
            .ends_with(".claude.json")
    );
    let account = settings["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["provider"] == "claude")
        .unwrap()
        .clone();
    assert_eq!(account["name"], "claude@example.test");
    let key = format!("claude:{}", account["id"].as_str().unwrap());
    assert_eq!(row["accountKey"], key.as_str());

    // Never asked: the quota query explains instead of opening a Keychain dialog.
    let quotas = rpc(p, "quotas.refresh", json!({"accountKey":key}));
    assert!(quotas[0]["error"].is_null());
    assert_eq!(quotas[0]["notice"], "未授权读取钥匙串，仍可查看本地用量");
    assert_eq!(quotas[0]["windows"], json!([]));
    assert!(quotas[0].get("meters").is_none());

    // A refusal is recorded without reading, survives settings saves and stops startup asks.
    let status = rpc(p, "claudeAuth.keychain", json!({"dismiss":true}));
    assert_eq!(status["supported"], true);
    assert_eq!(status["asked"], true);
    assert_eq!(status["granted"], false);
    assert_eq!(status["result"], "denied");
    let mut forged = rpc(p, "settings.get", json!({}));
    forged["claudeKeychain"] = json!({"askedAt":1,"granted":true,"result":"granted"});
    rpc(p, "settings.save", forged);
    assert_eq!(
        rpc(p, "settings.get", json!({}))["claudeKeychain"]["result"],
        "denied"
    );
    let startup = rpc(p, "claudeAuth.keychain", json!({"startup":true}));
    assert_eq!(startup["result"], "denied");
    let quotas = rpc(p, "quotas.refresh", json!({"accountKey":key}));
    assert!(quotas[0]["error"].is_null());
    assert_eq!(quotas[0]["notice"], "已拒绝钥匙串授权，仍可查看本地用量");

    // Once Claude writes a credentials file, that login takes over the same account.
    write(
        p,
        "home/.claude/.credentials.json",
        &json!({"claudeAiOauth":{"accessToken":"PRIVATE-NEVER-SERIALIZE"}}),
    );
    rpc(
        p,
        "accounts.autoScan",
        json!({"targets":[["claude","local"]]}),
    );
    let settings = rpc(p, "settings.get", json!({}));
    assert!(!settings.to_string().contains("PRIVATE-NEVER-SERIALIZE"));
    let rows = claude_rows(&settings);
    let live: Vec<_> = rows.iter().filter(|r| r["status"] == "observed").collect();
    assert_eq!(live.len(), 1, "{rows:?}");
    assert!(
        live[0]["configPath"]
            .as_str()
            .unwrap()
            .ends_with(".credentials.json")
    );
    assert_eq!(live[0]["hasCredential"], true);
    assert_eq!(live[0]["accountKey"], key.as_str());
    assert!(rows.iter().all(|r| r["accountKey"] == key.as_str()));
    assert_eq!(
        settings["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["provider"] == "claude")
            .count(),
        1
    );
}

#[test]
fn wallet_only_snapshot_survives_a_torn_native_credential_with_its_original_age() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    write(
        p,
        "home/.claude.json",
        &json!({"oauthAccount":{"accountUuid":"wallet-user","emailAddress":"wallet@example.test"}}),
    );
    write(
        p,
        "home/.claude/.credentials.json",
        &json!({"claudeAiOauth":{"accessToken":"PRIVATE-NOT-SENT"}}),
    );
    rpc(
        p,
        "accounts.autoScan",
        json!({"targets":[["claude","local"]]}),
    );
    let settings = rpc(p, "settings.get", json!({}));
    let account = settings["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["provider"] == "claude")
        .unwrap();
    let key = format!("claude:{}", account["id"].as_str().unwrap());
    let observed = aieyes_core::models::now() - 2 * 86400;
    let wallet = json!({"currency":"USD","total":"50.00","balance":"37.66","monthlyLimitEnabled":true,"monthlyLimit":"50.00","monthlyUsed":"12.34"});
    let snapshot = json!({"provider":"claude","accountId":account["id"],"updatedAt":observed,"snapshotAt":observed,"origin":"live","windows":[],"balances":[],"extraUsage":wallet});
    let db = rusqlite::Connection::open(p.join("data/aieyes.sqlite")).unwrap();
    db.execute(
        "INSERT INTO quotas(account_key,payload) VALUES(?1,?2) ON CONFLICT(account_key) DO UPDATE SET payload=excluded.payload",
        rusqlite::params![key, snapshot.to_string()],
    )
    .unwrap();
    // There is no usable token from this point, so a request cannot reach a server.
    std::fs::write(p.join("home/.claude/.credentials.json"), "{").unwrap();
    let refreshed = rpc(p, "quotas.refresh", json!({"accountKey":key}));
    for row in [
        refreshed[0].clone(),
        rpc(p, "dashboard", json!({}))["quotas"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["accountId"] == account["id"])
            .unwrap()
            .clone(),
    ] {
        assert_eq!(row["extraUsage"], wallet);
        assert!(row["error"].is_null(), "{row}");
        assert!(row["notice"].is_string(), "{row}");
        assert!(row["staleSince"].as_i64().is_some_and(|t| t > observed));
        assert_eq!(row["updatedAt"], observed);
        assert_eq!(row["snapshotAt"], observed);
    }
    // Older versions persisted a read error alongside the still-valid wallet.
    // Upgrading must recover that cached value into the same stale notice path.
    let mut legacy = snapshot;
    legacy["error"] = json!("Previous native read failed");
    db.execute(
        "UPDATE quotas SET payload=?2 WHERE account_key=?1",
        rusqlite::params![key, legacy.to_string()],
    )
    .unwrap();
    let recovered = rpc(p, "quotas.refresh", json!({"accountKey":key}));
    assert_eq!(recovered[0]["extraUsage"], wallet);
    assert!(recovered[0]["error"].is_null());
    assert!(recovered[0]["notice"].is_string());
    assert_eq!(recovered[0]["snapshotAt"], observed);
}
