#![cfg(unix)]
use aieyes_core::{Engine, models::*};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};

fn poll(engine: &mut Engine, source: &str, id: &str, phase: &str) -> Value {
    let start = Instant::now();
    loop {
        let result = engine
            .call("agyAuth.status", json!({"sourceId":source,"id":id}))
            .unwrap();
        if result["phase"] == phase {
            return result;
        }
        assert!(start.elapsed() < Duration::from_secs(12), "{result}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
fn login_uses_owned_pty_verifies_reuses_and_cancels_without_credentials_in_responses() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("agy-fixture");
    std::fs::write(
        &binary,
        r##"#!/usr/bin/env python3
import json, pathlib, sys, time
marker=pathlib.Path(__file__).with_suffix('.authenticated')
if '--print' in sys.argv:
    if not marker.exists(): sys.exit(1)
    print(json.dumps({'command':{'name':'usage','data':{'groups':[]}}}))
else:
    print('Open https://accounts.google.com/o/oauth2/auth?state=fixture',flush=True)
    code=input()
    if code=='fake-authorization-code': marker.touch()
    while True: time.sleep(.1)
"##,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut engine = Engine::open(&root.path().join("data")).unwrap();
    let source = Source {
        id: "agy-auth-test".into(),
        provider: "antigravity".into(),
        agy_binary: binary.to_string_lossy().into(),
        quota_pre_command: "export GEMINI_API_KEY=fixture".into(),
        path: root.path().to_string_lossy().into(),
        ..Default::default()
    };
    let settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    engine.store.save_settings(&settings).unwrap();
    let start = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let id = start["id"].as_str().unwrap();
    poll(&mut engine, &source.id, id, "authorizing");
    let result = engine
        .call(
            "agyAuth.submitCode",
            json!({"sourceId":source.id,"id":id,"code":"fake-authorization-code"}),
        )
        .unwrap();
    assert!(!result.to_string().contains("fake-authorization-code"));
    std::thread::sleep(Duration::from_millis(100));
    engine
        .call("agyAuth.verify", json!({"sourceId":source.id,"id":id}))
        .unwrap();
    let result = poll(&mut engine, &source.id, id, "authenticated");
    assert_eq!(result["identityConfirmed"], false);
    let existing = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    poll(
        &mut engine,
        &source.id,
        existing["id"].as_str().unwrap(),
        "authenticated",
    );
    std::fs::remove_file(binary.with_extension("authenticated")).unwrap();
    let pending = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let id = pending["id"].as_str().unwrap();
    poll(&mut engine, &source.id, id, "authorizing");
    let result = engine
        .call("agyAuth.cancel", json!({"sourceId":source.id,"id":id}))
        .unwrap();
    assert_eq!(result["phase"], "cancelled");
    assert!(result["authUrl"].is_null());
}

fn identity_fixture() -> (tempfile::TempDir, Engine, Source) {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    std::fs::create_dir(dir.join(".gemini")).unwrap();
    let claims = URL_SAFE_NO_PAD.encode(json!({"iss":"https://accounts.google.com","sub":"fixture-subject","email":"account@example.test","email_verified":true}).to_string());
    std::fs::write(dir.join(".gemini/antigravity-oauth-token"), json!({"auth_method":"consumer","id_token":format!("h.{claims}.s"),"token":{"access_token":"fake-private-access","refresh_token":"fake-private-refresh"}}).to_string()).unwrap();
    let binary = dir.join("agy-fixture");
    std::fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' '{\"status\":\"SUCCESS\",\"command\":{\"name\":\"usage\",\"data\":{\"groups\":[{\"name\":\"Gemini\",\"buckets\":[{\"remaining_fraction\":0.7,\"window\":\"5h\"}]}]}}}'\n").unwrap();
    let curl = dir.join("curl");
    std::fs::write(&curl, "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"licenses\":[{\"tierDisplayName\":\"Google AI Pro\"}]}'\n").unwrap();
    for path in [&binary, &curl] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let source = Source {
        id: format!(
            "agy-identity-{}",
            dir.file_name().unwrap().to_string_lossy()
        ),
        provider: "antigravity".into(),
        account_id: "existing-id".into(),
        agy_binary: binary.to_string_lossy().into(),
        path: dir.join("history-is-not-auth").to_string_lossy().into(),
        quota_pre_command: format!(
            "export HOME={}\nexport SSH_CONNECTION=fixture\nexport PATH={}:$PATH\nunset GEMINI_API_KEY AGY_ADC_AUTH AGY_LLM_GATEWAY_API_KEY",
            aieyes_core::process::quote(&dir.to_string_lossy()),
            aieyes_core::process::quote(&dir.to_string_lossy())
        ),
        ..Default::default()
    };
    let engine = Engine::open(&dir.join("data")).unwrap();
    engine
        .store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: "existing-id".into(),
                provider: "antigravity".into(),
                name: "Antigravity 账户".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    (root, engine, source)
}

#[test]
fn login_persists_identity_before_success_and_retains_it_on_later_query_failure() {
    let (root, mut engine, source) = identity_fixture();
    let start = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let id = start["id"].as_str().unwrap();
    let session = poll(&mut engine, &source.id, id, "authenticated");
    let row = &session["accountStatus"];
    assert_eq!(row["accountKey"], "antigravity:existing-id");
    assert_eq!(row["sourceId"], source.id);
    assert_eq!(row["current"], true);
    assert_eq!(row["identity"]["email"], "account@example.test");
    assert_eq!(session["workspaceConfirmationRequired"], false);
    let settings = engine.store.settings().unwrap();
    assert!(settings.accounts[0].identity_key.is_some());
    assert_eq!(settings.accounts[0].name, "account@example.test");
    let mut reader = Engine::open(&root.path().join("data")).unwrap();
    let cached = reader.call("accounts.status.get", json!({})).unwrap();
    assert_eq!(
        cached[0], *row,
        "another connection sees the committed login"
    );

    // Status polling must reuse the verified observation instead of running the CLI again.
    std::fs::write(root.path().join("agy-fixture"), "#!/bin/sh\nexit 1\n").unwrap();
    assert_eq!(
        engine
            .call("agyAuth.status", json!({"sourceId":source.id,"id":id}))
            .unwrap()["accountStatus"],
        *row
    );
    let failed = reader
        .call(
            "accounts.status.refresh",
            json!({"accountKey":"antigravity:existing-id","sourceId":source.id}),
        )
        .unwrap();
    assert!(failed["error"].is_string());
    assert_eq!(failed["current"], true);
    assert_eq!(failed["identity"], row["identity"]);
    reader
        .call(
            "quotas.refresh",
            json!({"accountKey":"antigravity:existing-id"}),
        )
        .unwrap();
    assert_eq!(
        reader.call("accounts.status.get", json!({})).unwrap()[0]["identity"],
        row["identity"]
    );
}

#[test]
fn superseded_login_cannot_publish_or_cache_its_observation() {
    let (root, mut engine, source) = identity_fixture();
    let start = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let id = start["id"].as_str().unwrap();
    engine
        .store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![
                format!("account-status-revision:{}", source.id),
                json!({"session":"newer-login"}).to_string()
            ],
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        match engine.call("agyAuth.status", json!({"sourceId":source.id,"id":id})) {
            Err(error) => {
                assert!(error.to_string().contains("登录状态已变化"));
                break;
            }
            Ok(value) => {
                assert_ne!(value["authenticated"], true);
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut reader = Engine::open(&root.path().join("data")).unwrap();
    assert!(reader.call("accounts.status.get", json!({})).unwrap()[0]["identity"].is_null());
    assert!(
        reader.store.settings().unwrap().accounts[0]
            .identity_key
            .is_none()
    );
}

#[test]
fn workspace_confirmation_is_consumed_once_despite_split_escapes_and_redraws() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("agy-trust-fixture");
    std::fs::write(
        &binary,
        r#"#!/usr/bin/env python3
import json,pathlib,sys,time
marker=pathlib.Path(__file__).with_suffix('.authenticated')
if '--print' in sys.argv:
    if not marker.exists():sys.exit(1)
    print(json.dumps({'command':{'name':'usage','data':{'groups':[]}}}))
else:
    for text in ['\x1b[','32mYes, I tru','st this folder\x1b[0m\n']:
        sys.stdout.write(text);sys.stdout.flush();time.sleep(.08)
    assert input()==''
    # Repainting the old trust screen must not request another confirmation.
    print('Yes, I trust this folder',flush=True)
    print('Open https://accounts.google.com/o/oauth2/auth?state=trust-fixture',flush=True)
    if input()=='fake-code':marker.touch()
    while True:time.sleep(.1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = Source {
        id: format!(
            "agy-trust-{}",
            root.path().file_name().unwrap().to_string_lossy()
        ),
        provider: "antigravity".into(),
        account_id: "a".into(),
        agy_binary: binary.to_string_lossy().into(),
        quota_pre_command: "export GEMINI_API_KEY=fixture".into(),
        ..Default::default()
    };
    let mut engine = Engine::open(&root.path().join("data")).unwrap();
    engine
        .store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: "a".into(),
                name: "Trust fixture".into(),
                provider: "antigravity".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let start = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let id = start["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let status = engine
            .call("agyAuth.status", json!({"sourceId":source.id,"id":id}))
            .unwrap();
        if status["workspaceConfirmationRequired"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        engine
            .call(
                "agyAuth.submitCode",
                json!({"sourceId":source.id,"id":id,"code":"should-not-be-submitted"})
            )
            .is_err()
    );
    let confirmed = engine
        .call(
            "agyAuth.confirmWorkspace",
            json!({"sourceId":source.id,"id":id}),
        )
        .unwrap();
    assert_eq!(confirmed["workspaceConfirmationRequired"], false);
    loop {
        let status = engine
            .call("agyAuth.status", json!({"sourceId":source.id,"id":id}))
            .unwrap();
        assert_eq!(status["workspaceConfirmationRequired"], false, "{status}");
        if status["authUrl"]
            .as_str()
            .is_some_and(|s| s.ends_with("state=trust-fixture"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        engine
            .call(
                "agyAuth.confirmWorkspace",
                json!({"sourceId":source.id,"id":id})
            )
            .is_err()
    );
    engine
        .call(
            "agyAuth.submitCode",
            json!({"sourceId":source.id,"id":id,"code":"fake-code"}),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    engine
        .call("agyAuth.verify", json!({"sourceId":source.id,"id":id}))
        .unwrap();
    let result = poll(&mut engine, &source.id, id, "authenticated");
    assert_eq!(result["accountStatus"]["authenticated"], true);
    assert_eq!(result["accountStatus"]["identityConfirmed"], false);
    assert_eq!(result["accountStatus"]["current"], Value::Null);
}

#[test]
fn known_identity_binds_once_keeps_ids_and_reports_mismatch_without_secrets() {
    let (root, mut engine, source) = identity_fixture();
    let params = json!({"accountKey":"antigravity:existing-id","sourceId":source.id});
    let row = engine
        .call("accounts.status.refresh", params.clone())
        .unwrap();
    assert_eq!(row["identityConfirmed"], true);
    assert_eq!(row["current"], true);
    assert_eq!(row["identity"]["subscription"], "Google AI Pro");
    let settings = engine.store.settings().unwrap();
    assert_eq!(settings.accounts[0].id, "existing-id");
    assert_eq!(settings.accounts[0].name, "account@example.test");
    assert!(
        settings.accounts[0]
            .identity_key
            .as_ref()
            .unwrap()
            .starts_with("google:")
    );
    let cached = engine.call("accounts.status.get", json!({})).unwrap();
    assert_eq!(
        cached[0]["current"], true,
        "binding must not invalidate its own observation"
    );
    let start = engine
        .call("agyAuth.start", json!({"sourceId":source.id}))
        .unwrap();
    let session = poll(
        &mut engine,
        &source.id,
        start["id"].as_str().unwrap(),
        "authenticated",
    );
    assert_eq!(session["identityConfirmed"], true);
    assert_eq!(session["identity"]["email"], "account@example.test");
    let quota = engine
        .call(
            "quotas.refresh",
            json!({"accountKey":"antigravity:existing-id"}),
        )
        .unwrap();
    assert_eq!(quota[0]["plan"], "Google AI Pro", "{quota}");
    assert_eq!(quota[0]["identity"]["email"], "account@example.test");
    let mut settings = engine.store.settings().unwrap();
    settings.accounts[0].name = "Manual name".into();
    engine.store.save_settings(&settings).unwrap();
    let file = root.path().join(".gemini/antigravity-oauth-token");
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let claims=URL_SAFE_NO_PAD.encode(json!({"iss":"accounts.google.com","sub":"different-subject","email":"other@example.test","email_verified":true}).to_string());
    let mut credential: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    credential["id_token"] = json!(format!("h.{claims}.s"));
    std::fs::write(file, credential.to_string()).unwrap();
    let row = engine
        .call("accounts.status.refresh", params.clone())
        .unwrap();
    assert_eq!(row["current"], false);
    assert_eq!(row["credential"], false);
    assert_eq!(row["identity"]["email"], "other@example.test");
    assert_eq!(
        engine.store.settings().unwrap().accounts[0].name,
        "Manual name"
    );
    assert_eq!(
        engine.store.settings().unwrap().accounts[0].identity_key,
        settings.accounts[0].identity_key
    );
    let failed = engine
        .call(
            "quotas.refresh",
            json!({"accountKey":"antigravity:existing-id"}),
        )
        .unwrap();
    assert!(failed[0]["error"].as_str().unwrap().contains("身份不一致"));
    let stored: String = engine
        .store
        .db
        .query_row("SELECT payload FROM quotas", [], |r| r.get(0))
        .unwrap();
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["identity"]["email"], "account@example.test");
    for result in [row, cached, session, quota, failed, stored] {
        assert!(!result.to_string().contains("fake-private"));
    }
    let values:Vec<String>=engine.store.db.prepare("SELECT value FROM kv UNION ALL SELECT payload FROM quotas UNION ALL SELECT payload FROM quota_history").unwrap().query_map([],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    assert!(values.iter().all(|s| !s.contains("fake-private")
        && !s.contains("access_token")
        && !s.contains("refresh_token")));
    // A device configuration edit invalidates the old public identity as well.
    let mut settings = engine.store.settings().unwrap();
    settings.sources[0]
        .quota_pre_command
        .push_str("\nexport EXTRA=changed");
    engine.store.save_settings(&settings).unwrap();
    let cached = engine.call("accounts.status.get", json!({})).unwrap();
    assert!(cached[0]["identity"].is_null());
}

#[test]
fn metadata_failure_preserves_new_quota_and_only_same_identity_subscription() {
    let (root, mut engine, source) = identity_fixture();
    let mut settings = engine.store.settings().unwrap();
    settings.accounts[0].name = "Custom account".into();
    engine.store.save_settings(&settings).unwrap();
    engine.call("quotas.refresh", json!({})).unwrap();
    assert_eq!(
        engine.store.settings().unwrap().accounts[0].name,
        "Custom account"
    );
    std::fs::write(root.path().join("curl"), "#!/bin/sh\nexit 22\n").unwrap();
    let binary = root.path().join("agy-fixture");
    let usage = std::fs::read_to_string(&binary)
        .unwrap()
        .replace("0.7", "0.4");
    std::fs::write(binary, usage).unwrap();

    let rows = engine.call("quotas.refresh", json!({})).unwrap();
    assert!(rows[0]["error"].is_null(), "{rows}");
    assert!(rows[0]["metadataError"].is_string());
    let raw: String = engine
        .store
        .db
        .query_row("SELECT payload FROM quotas", [], |r| r.get(0))
        .unwrap();
    let mut saved: QuotaSnapshot = serde_json::from_str(&raw).unwrap();
    assert_eq!(saved.plan.as_deref(), Some("Google AI Pro"));
    assert!(saved.identity.as_ref().unwrap().stale);
    assert_eq!(saved.windows.len(), 1);
    assert!((saved.windows[0].used_percent - 60.0).abs() < 0.0001);
    let cached = engine.call("accounts.status.get", json!({})).unwrap();
    assert_eq!(cached[0]["identity"]["subscription"], "Google AI Pro");
    assert_eq!(cached[0]["identity"]["stale"], true);
    let history: String = engine
        .store
        .db
        .query_row(
            "SELECT payload FROM quota_history ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&history).unwrap()["plan"],
        "Google AI Pro"
    );

    saved.identity = None;
    saved.plan = None;
    saved.updated_at += 1;
    engine.store.quota(&saved).unwrap();
    let raw: String = engine
        .store
        .db
        .query_row("SELECT payload FROM quotas", [], |r| r.get(0))
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&raw).unwrap()["identity"].is_null(),
        "unidentified refresh must not reuse another login's metadata"
    );
    assert_eq!(engine.store.settings().unwrap().sources[0].id, source.id);
}
