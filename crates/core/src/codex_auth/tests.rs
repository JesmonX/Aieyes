use super::{files, local, rpc::Rpc};
use crate::{import, models::*};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{fs, process::Command};

fn auth(user: &str, workspace: &str, refresh: &str) -> Value {
    let token=URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_user_id":user,"chatgpt_account_id":workspace,"chatgpt_plan_type":"plus"}}).to_string());
    json!({"auth_mode":"chatgpt","tokens":{"access_token":format!("header.{token}.signature"),"refresh_token":refresh,"account_id":workspace}})
}
fn root() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = fs::canonicalize(d.path()).unwrap();
    (d, p)
}

#[test]
fn unnamed_accounts_use_login_email_and_preserve_manual_names() {
    let (_d, r) = root();
    let mut engine = crate::Engine::open(&r.join("database")).unwrap();
    engine
        .store
        .save_settings(&Settings {
            sources: vec![Source {
                id: "source".into(),
                codex_home_id: Some("home".into()),
                path: r.to_string_lossy().into(),
                codex_binary: "missing-fixture-cli".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    for (user, rename_before_login) in [("automatic", false), ("custom", true)] {
        let created = engine
            .call("accounts.create", json!({"provider":"codex"}))
            .unwrap();
        let key = created["accountKey"].as_str().unwrap();
        let id = key.strip_prefix("codex:").unwrap();
        let mut settings = engine.store.settings().unwrap();
        let account = settings.accounts.iter_mut().find(|a| a.id == id).unwrap();
        assert_eq!(account.name, "Codex · 待登录");
        assert!(account.pending_name);
        if rename_before_login {
            account.name = "My custom name".into();
            engine.store.save_settings(&settings).unwrap();
        }
        let mut credential = auth(user, "workspace", "FIXTURE");
        let email = format!("{user}@example.test");
        let token = URL_SAFE_NO_PAD.encode(json!({"email":email}).to_string());
        credential["tokens"]["id_token"] = json!(format!("header.{token}.signature"));
        let profile = files::save_profile(&r, user, "", &credential).unwrap();
        assert_eq!(profile.name, email);
        engine
            .call(
                "codexAuth.profiles.bind",
                json!({"sourceId":"source","profileId":user,"accountId":id}),
            )
            .unwrap();
        let mut settings = engine.store.settings().unwrap();
        let account = settings.accounts.iter_mut().find(|a| a.id == id).unwrap();
        assert_eq!(
            account.name,
            if rename_before_login {
                "My custom name"
            } else {
                &email
            }
        );
        assert!(!account.pending_name);
        account.name = "Renamed after login".into();
        engine.store.save_settings(&settings).unwrap();
        engine
            .call(
                "codexAuth.profiles.bind",
                json!({"sourceId":"source","profileId":user,"accountId":id}),
            )
            .unwrap();
        assert_eq!(
            engine
                .store
                .settings()
                .unwrap()
                .accounts
                .iter()
                .find(|a| a.id == id)
                .unwrap()
                .name,
            "Renamed after login"
        );
    }
    let mut credential = auth("direct", "workspace", "FIXTURE");
    let token = URL_SAFE_NO_PAD.encode(json!({"email":"direct@example.test"}).to_string());
    credential["tokens"]["id_token"] = json!(format!("header.{token}.signature"));
    files::save_profile(&r, "direct", "Old profile label", &credential).unwrap();
    let created = engine
        .call(
            "codexAuth.profiles.bind",
            json!({"sourceId":"source","profileId":"direct"}),
        )
        .unwrap();
    assert_eq!(
        engine
            .store
            .settings()
            .unwrap()
            .accounts
            .iter()
            .find(|a| a.id == created["accountId"])
            .unwrap()
            .name,
        "direct@example.test"
    );
    let fallback = files::save_profile(
        &r,
        "no-email",
        "",
        &auth("no-email", "workspace", "FIXTURE"),
    )
    .unwrap();
    assert_eq!(fallback.name, "Codex 账户");
}

#[test]
fn verified_profiles_connect_multiple_homes_without_duplicate_accounts_or_unarchiving() {
    let (_d, r) = root();
    let first = r.join("first");
    let second = r.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    files::save_profile(
        &first,
        "one",
        "Personal",
        &auth("user", "workspace", "FIXTURE"),
    )
    .unwrap();
    files::save_profile(
        &second,
        "two",
        "Remote",
        &auth("user", "workspace", "FIXTURE"),
    )
    .unwrap();
    files::save_profile(
        &second,
        "other",
        "Other workspace",
        &auth("user", "other", "FIXTURE"),
    )
    .unwrap();
    let mut engine = crate::Engine::open(&r.join("database")).unwrap();
    engine
        .store
        .save_settings(&Settings {
            sources: vec![
                Source {
                    id: "first".into(),
                    codex_home_id: Some("home1".into()),
                    path: first.to_string_lossy().into(),
                    codex_binary: "aieyes-missing-fixture-cli".into(),
                    ..Default::default()
                },
                Source {
                    id: "second".into(),
                    codex_home_id: Some("home2".into()),
                    path: second.to_string_lossy().into(),
                    codex_binary: "aieyes-missing-fixture-cli".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
    let a = engine
        .call(
            "codexAuth.profiles.bind",
            json!({"sourceId":"first","profileId":"one"}),
        )
        .unwrap();
    let b = engine
        .call(
            "codexAuth.profiles.bind",
            json!({"sourceId":"second","profileId":"two"}),
        )
        .unwrap();
    assert_eq!(a["accountId"], b["accountId"]);
    let settings = engine.store.settings().unwrap();
    assert_eq!(settings.accounts.len(), 1);
    assert_eq!(settings.accounts[0].connections.len(), 2);
    assert!(
        engine
            .call(
                "codexAuth.profiles.bind",
                json!({"sourceId":"second","profileId":"other","accountId":a["accountId"]})
            )
            .is_err()
    );
    let other = engine
        .call(
            "codexAuth.profiles.bind",
            json!({"sourceId":"second","profileId":"other"}),
        )
        .unwrap();
    assert_ne!(other["accountId"], a["accountId"]);
    let mut settings = engine.store.settings().unwrap();
    settings.accounts[0].archived = true;
    engine.store.save_settings(&settings).unwrap();
    engine
        .call(
            "codexAuth.profiles.bind",
            json!({"sourceId":"first","profileId":"one"}),
        )
        .unwrap();
    let settings = engine.store.settings().unwrap();
    assert!(settings.accounts[0].archived);
    assert_eq!(settings.accounts.len(), 2);
}

#[test]
fn identity_is_user_and_workspace_and_external_omits_refresh() {
    let a = auth("one", "team", "SECRET");
    let b = auth("two", "team", "SECRET");
    let c = auth("one", "other", "SECRET");
    assert_ne!(
        files::identity(&a).unwrap().key,
        files::identity(&b).unwrap().key
    );
    assert_ne!(
        files::identity(&a).unwrap().key,
        files::identity(&c).unwrap().key
    );
    assert!(!files::external(&a).unwrap().to_string().contains("SECRET"));
    assert!(files::identity(&json!({"auth_mode":"apikey"})).is_err());
}
#[test]
fn lock_is_exclusive_and_profiles_reject_identity_changes() {
    let (_d, r) = root();
    let lease = files::lock(&r).unwrap();
    assert!(files::lock(&r).is_err());
    drop(lease);
    assert!(files::lock(&r).is_ok());
    files::save_profile(&r, "first", "First", &auth("one", "team", "SECRET")).unwrap();
    assert!(files::save_profile(&r, "first", "Second", &auth("two", "team", "SECRET")).is_err());
    assert!(files::profile_path(&r, "../outside").is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for folder in [".aieyes", ".aieyes/accounts", ".aieyes/accounts/first"] {
            assert_eq!(
                fs::metadata(r.join(folder)).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::metadata(r.join(".aieyes/accounts/first/auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        std::os::unix::fs::symlink(r.join(".aieyes/accounts/first"), r.join("link")).unwrap();
        assert!(files::read(&r.join("link/auth.json")).is_err());
    }
}
#[test]
fn shared_scan_excludes_managed_histories_and_keeps_plain_imports() {
    let (_d, r) = root();
    fs::write(r.join("direct.jsonl"), "{}\n").unwrap();
    let mut s = Source {
        provider: "codex".into(),
        path: r.to_string_lossy().into(),
        ..Default::default()
    };
    assert_eq!(import::paths(&s).unwrap().len(), 1);
    fs::create_dir_all(r.join("sessions")).unwrap();
    fs::write(r.join("sessions/user.jsonl"), "{}\n").unwrap();
    fs::create_dir_all(r.join(".aieyes/codex/test/managed/sessions")).unwrap();
    fs::write(
        r.join(".aieyes/codex/test/managed/sessions/synthetic.jsonl"),
        "{}\n",
    )
    .unwrap();
    s.codex_home_id = Some("home".into());
    let files = import::paths(&s).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("user.jsonl"));
}
#[test]
fn stale_switch_cannot_replace_changed_credentials() {
    let (_d, r) = root();
    let a = auth("one", "team", "SECRET");
    let b = auth("two", "team", "SECRET");
    files::atomic(&r.join("auth.json"), &a).unwrap();
    files::save_profile(&r, "second", "Second", &b).unwrap();
    let loc = local::Location {
        path: r.to_string_lossy().into(),
        binary: "missing-fixture-codex".into(),
        proxy: ProxyConfig::default(),
    };
    let prepared = local::call(&loc, "switch.prepare", &json!({"profileId":"second"})).unwrap();
    let mut changed = a.clone();
    changed["tokens"]["refresh_token"] = json!("rotated");
    files::atomic(&r.join("auth.json"), &changed).unwrap();
    let error = local::call(
        &loc,
        "switch.commit",
        &json!({"operationId":prepared["operationId"],"closeProcesses":true}),
    )
    .unwrap_err();
    assert!(error.to_string().contains("凭据已更新"));
    assert_eq!(files::read(&r.join("auth.json")).unwrap(), changed);
}
#[test]
fn rpc_handles_server_requests_with_colliding_ids_and_redacts_errors() {
    let script = r#"
import sys,json
for line in sys.stdin:
 v=json.loads(line)
 if v.get('method')=='initialize': print(json.dumps({'id':v['id'],'result':{}}),flush=True)
 elif v.get('method')=='account/login/start': print(json.dumps({'id':v['id'],'result':{}}),flush=True)
 elif v.get('method')=='account/read': print(json.dumps({'id':v['id'],'result':{'account':{'type':'chatgpt'}}}),flush=True)
 elif v.get('method')=='account/rateLimits/read':
  ident=v['id'];print(json.dumps({'id':ident,'method':'account/chatgptAuthTokens/refresh','params':{'previousAccountId':'team'}}),flush=True)
  response=json.loads(sys.stdin.readline());assert 'refresh_token' not in str(response);assert 'result' in response
  print(json.dumps({'id':ident,'result':{'ok':True}}),flush=True)
 elif v.get('method')=='failure': print(json.dumps({'id':v['id'],'error':{'code':401,'message':'unauthorized SECRET_TOKEN'}}),flush=True)
"#;
    let mut command = Command::new("python3");
    command.args(["-u", "-c", script]);
    let mut rpc = Rpc::command(command).unwrap();
    let a = auth("one", "team", "SECRET");
    let mut b = a.clone();
    b["tokens"]["access_token"] = json!(format!(
        "{}.new",
        a["tokens"]["access_token"].as_str().unwrap()
    ));
    rpc.login_external(&a).unwrap();
    assert_eq!(rpc.quota(&a, &mut || Ok(b.clone())).unwrap()["ok"], true);
    let error = rpc
        .call("failure", Value::Null, &mut |_| Ok(None))
        .unwrap_err()
        .to_string();
    assert!(error.contains("认证已失效"));
    assert!(!error.contains("SECRET"));
}

#[test]
fn refresh_journal_recovers_after_response_loss_without_touching_active_account() {
    let (_d, r) = root();
    let _lease = files::lock(&r).unwrap();
    let a = auth("one", "team", "SECRET");
    let b = auth("two", "team", "OLD");
    let c = auth("two", "team", "NEW");
    files::atomic(&r.join("auth.json"), &a).unwrap();
    files::save_profile(&r, "second", "Second", &b).unwrap();
    let stage = r.join(".aieyes/codex/second/managed");
    files::private_dir(&stage).unwrap();
    files::atomic(&stage.join("auth.json"), &c).unwrap();
    let journal = r.join(".aieyes/state/refresh.json");
    files::atomic(
        &journal,
        &json!({"profileId":"second","current":false,"revision":files::revision(&b)}),
    )
    .unwrap();
    local::recover(&r).unwrap();
    assert!(!journal.exists());
    assert_eq!(files::read(&r.join("auth.json")).unwrap(), a);
    assert_eq!(
        files::read(&r.join(".aieyes/accounts/second/auth.json")).unwrap(),
        c
    );
    files::atomic(
        &journal,
        &json!({"profileId":"second","current":false,"revision":"unknown"}),
    )
    .unwrap();
    files::atomic(&r.join(".aieyes/accounts/second/auth.json"), &b).unwrap();
    assert!(local::recover(&r).is_err());
    assert!(journal.exists());
}

#[test]
fn shared_history_totals_survive_and_profile_quotas_stay_visible() {
    use crate::store::Store;
    let (temp, r) = root();
    let store = Store::open(temp.path()).unwrap();
    let mut settings = Settings {
        sources: vec![Source {
            id: "source".into(),
            provider: "codex".into(),
            account_id: "one".into(),
            path: r.to_string_lossy().into(),
            ..Default::default()
        }],
        accounts: vec![Account {
            id: "one".into(),
            provider: "codex".into(),
            name: "One".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let event = UsageEvent {
        id: "event".into(),
        source_id: "source".into(),
        provider: "codex".into(),
        account_id: "one".into(),
        timestamp: now(),
        model: "fixture".into(),
        tokens: Tokens {
            input: 10,
            ..Default::default()
        },
        import_identity: None,
        session_id: "session".into(),
        attribution: "fixture".into(),
        billing: Default::default(),
        interval_start: None,
        interval_evidence: String::new(),
    };
    store.put_event(&event, &[], &settings).unwrap();
    let before = store.dashboard(&Filter::default()).unwrap().summary.total;
    settings.sources[0].account_id.clear();
    settings.sources[0].codex_home_id = Some("home".into());
    settings.accounts[0].quota_profile_id = Some("home:profile".into());
    store.save_settings(&settings).unwrap();
    let dashboard = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(dashboard.summary.total, before);
    assert_eq!(dashboard.quotas.len(), 1);
    let filtered = store
        .dashboard(&Filter {
            account_id: Some("one".into()),
            ..Default::default()
        })
        .unwrap();
    let shared = store
        .dashboard(&Filter {
            account_id: Some(String::new()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(shared.summary.total, before);
    assert_eq!(filtered.summary.total, 0);
    assert_eq!(filtered.quotas.len(), 1);
    assert!(
        dashboard.sources[0]["accountIds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id != "one")
    );
}

#[cfg(unix)]
#[test]
fn quota_uses_active_auth_in_memory_without_refresh_token_copy() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, r) = root();
    let a = auth("one", "team", "PRIVATE_REFRESH");
    files::atomic(&r.join("auth.json"), &a).unwrap();
    files::save_profile(&r, "first", "First", &a).unwrap();
    let executable = r.join("mock-codex");
    fs::write(
        &executable,
        r#"#!/usr/bin/env python3
import sys,json,os,pathlib
if '--version' in sys.argv:
 print('codex-cli 0.161.0');sys.exit(0)
assert not pathlib.Path(os.environ['CODEX_HOME'],'auth.json').exists()
assert 'PRIVATE_REFRESH' not in ' '.join(sys.argv)
for line in sys.stdin:
 assert 'PRIVATE_REFRESH' not in line
 v=json.loads(line);method=v.get('method')
 if method=='initialized': continue
 if method=='account/read': result={'account':{'type':'chatgpt'}}
 elif method=='account/rateLimits/read': result={'fixture':True}
 else: result={}
 print(json.dumps({'id':v['id'],'result':result}),flush=True)
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let loc = local::Location {
        path: r.to_string_lossy().into(),
        binary: executable.to_string_lossy().into(),
        proxy: ProxyConfig::default(),
    };
    assert_eq!(
        local::call(&loc, "quota", &json!({"profileId":"first"})).unwrap()["fixture"],
        true
    );
    assert_eq!(files::read(&r.join("auth.json")).unwrap(), a);
    assert!(!r.join(".aieyes/codex/first/query/auth.json").exists());
    assert!(!r.join(".aieyes/codex/first/managed/auth.json").exists());
}
