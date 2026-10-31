//! Synthetic Gemini/OMP account lifecycle evidence. No real credentials,
//! provider hosts, client launches, OAuth refreshes or model calls.
use super::*;
use crate::native_quota_transport::Response;

const TOKEN: &str = "PRIVATE-GEMINI-FIXTURE-ONE";
const OTHER: &str = "PRIVATE-GEMINI-FIXTURE-OTHER";
const REFRESH: &str = "PRIVATE-GEMINI-FIXTURE-REFRESH";
const SUBJECT: &str = "123456789012345678901";

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/gemini-cli-usage.json")).unwrap()
}

fn entry(token: &str, project: &str) -> Value {
    json!({"type":"oauth","access":token,"refresh":REFRESH,"expires":4102444800000_i64,"projectId":project})
}

fn document(root: &Path, name: &str, value: Value) -> Document {
    Document {
        root: root.to_string_lossy().into(),
        path: root.join(name).to_string_lossy().into(),
        bytes: serde_json::to_vec(&value).unwrap(),
    }
}

fn database(root: &Path) -> rusqlite::Connection {
    std::fs::create_dir_all(root).unwrap();
    let db = rusqlite::Connection::open(root.join("agent.db")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    db.execute_batch(include_str!("../../tests/fixtures/omp-auth.sql"))
        .unwrap();
    db.execute("DELETE FROM auth_credentials", []).unwrap();
    db
}

fn put(db: &rusqlite::Connection, id: i64, token: &str, project: &str) {
    db.execute(
        "INSERT OR REPLACE INTO auth_credentials VALUES(?1,'google-gemini-cli','oauth',?2,NULL,NULL,1,1)",
        rusqlite::params![id, entry(token, project).to_string()],
    ).unwrap();
}

fn database_rows(root: &Path, machine: &str) -> Vec<Candidate> {
    let doc = read_sqlite(&root.to_string_lossy(), &root.join("agent.db")).unwrap();
    extract("omp", machine, &doc).unwrap()
}

fn identity(candidate: &Candidate) -> crate::upstream::Identity {
    let auth = candidate.upstream_auth.as_ref().unwrap();
    crate::upstream::gemini_identity_using(auth, |request| {
        let data = if request.url.ends_with("/userinfo") {
            let mut user = fixture()["userinfo"].clone();
            if candidate.secret == OTHER {
                user["id"] = json!("223456789012345678901");
            }
            user
        } else {
            let mut load = fixture()["paidAssist"].clone();
            load["cloudaicompanionProject"] = auth["projectId"].clone();
            load
        };
        Ok(Response { status: 200, data })
    })
    .unwrap()
}

fn bind(candidate: &mut Candidate) {
    let identity = identity(candidate);
    candidate.connection.identity_key = Some(crate::upstream::client_identity(
        &candidate.connection.provider,
        &candidate.connection.selector,
        "gemini",
        &identity.subject,
    ));
    candidate.connection.upstream_identity = Some(identity.upstream);
    candidate.connection.email = identity.email;
}

fn seed_probe(candidate: &Candidate, failed: bool) {
    let key = hash(&format!(
        "{}\0{}",
        candidate.connection.id, candidate.fingerprint
    ));
    let answer = if failed {
        Err(now())
    } else {
        Ok(identity(candidate))
    };
    PROBED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(key, answer);
}

fn settings_for(candidates: &mut [Candidate]) -> Settings {
    let mut settings = Settings {
        proxy: ProxyConfig {
            mode: "custom".into(),
            url: "http://127.0.0.1:9".into(),
        },
        agents: ["pi", "omp", "antigravity"]
            .into_iter()
            .map(|provider| AgentConfiguration {
                provider: provider.into(),
                enabled: true,
                machine_ids: vec!["local".into()],
            })
            .collect(),
        ..Default::default()
    };
    for candidate in candidates {
        let row = &mut candidate.connection;
        let index = settings
            .accounts
            .iter()
            .position(|a| a.provider == row.provider && a.identity_key == row.identity_key)
            .unwrap_or_else(|| {
                settings.accounts.push(Account {
                    id: format!("fixture-{}", settings.accounts.len()),
                    provider: row.provider.clone(),
                    name: "Gemini fixture".into(),
                    identity_key: row.identity_key.clone(),
                    upstream_identity: row.upstream_identity.clone(),
                    quota_enabled: true,
                    ..Default::default()
                });
                settings.accounts.len() - 1
            });
        row.account_key = Some(accounts::key(&settings.accounts[index]));
        row.machine_fingerprint = hash(&host_signature(&settings, &row.machine_id).to_string());
        settings.native_connections.push(row.clone());
    }
    settings
}

#[test]
fn gemini_legacy_oauth_projection_preserves_project_expiry_and_product_boundary() {
    let root = Path::new("/synthetic-gemini");
    for provider in ["pi", "omp"] {
        for factor in [1, 1000] {
            for offset in [300, -300, -STALE_AFTER - 1] {
                let at = now() + offset;
                let mut oauth = entry(TOKEN, "project-one");
                oauth["expires"] = json!(at * factor);
                let doc = document(root, "auth.json", json!({"google-gemini-cli":oauth}));
                let row = extract(provider, "local", &doc).unwrap().remove(0);
                assert_eq!(upstream_kind(&row.connection), Some("gemini"));
                assert_eq!(row.connection.token_expired_at, (offset <= 0).then_some(at));
                assert_eq!(
                    row.connection.status,
                    if offset <= -STALE_AFTER {
                        "stale"
                    } else {
                        "observed"
                    }
                );
                assert_eq!(
                    row.upstream_auth.as_ref().unwrap()["projectId"],
                    "project-one"
                );
                assert!(
                    !row.upstream_auth
                        .as_ref()
                        .unwrap()
                        .to_string()
                        .contains(REFRESH)
                );
                assert!(
                    !serde_json::to_string(&row.connection)
                        .unwrap()
                        .contains("PRIVATE")
                );
            }
        }
        let doc = document(
            root,
            "auth.json",
            json!({
                "google-gemini-cli":{"type":"api","key":TOKEN},
                "google":entry(TOKEN, "project-one"),
                "google-antigravity":entry(TOKEN, "project-one")
            }),
        );
        assert!(
            extract(provider, "local", &doc)
                .unwrap()
                .iter()
                .all(|r| upstream_kind(&r.connection).is_none())
        );
    }
    let doc = document(
        root,
        "auth.json",
        json!({"google-gemini-cli":entry(TOKEN, "project-one")}),
    );
    assert!(
        extract("opencode", "local", &doc)
            .unwrap()
            .iter()
            .all(|r| upstream_kind(&r.connection).is_none())
    );
}

#[test]
fn gemini_pool_owner_prefers_usable_database_then_legacy_without_summing_clients() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = database(root);
    put(&db, 1, TOKEN, "project-one");
    put(&db, 2, TOKEN, "project-two");
    let legacy = document(
        root,
        "auth.json",
        json!({"google-gemini-cli":entry(TOKEN, "project-one")}),
    );
    let mut rows = database_rows(root, "local");
    rows.extend(extract("omp", "local", &legacy).unwrap());
    rows.extend(extract("pi", "local", &legacy).unwrap());
    for row in &mut rows {
        bind(row);
    }
    rows[0].connection.id = "z-database".into();
    rows[2].connection.id = "a-json".into();
    let mut settings = settings_for(&mut rows);
    let mut account = settings
        .accounts
        .iter()
        .find(|a| a.provider == "omp" && a.identity_key == rows[0].connection.identity_key)
        .unwrap()
        .clone();
    account.quota_source_id = Some("native-a-json".into());
    settings
        .accounts
        .iter_mut()
        .find(|a| a.id == account.id)
        .unwrap()
        .quota_source_id = Some("native-a-json".into());
    assert_eq!(
        quota_sources(&settings, &account)[0].id,
        "native-z-database"
    );
    let pi = settings
        .accounts
        .iter()
        .find(|a| a.provider == "pi")
        .unwrap();
    assert!(quota_sources(&settings, pi).is_empty());
    assert_eq!(
        settings
            .accounts
            .iter()
            .map(|a| quota_sources(&settings, a).len())
            .sum::<usize>(),
        2
    );
    assert_ne!(
        rows[0].connection.upstream_identity,
        rows[1].connection.upstream_identity
    );
    assert_ne!(
        rows[0].connection.upstream_identity.as_deref(),
        Some(format!("google:{}", hash(SUBJECT)).as_str())
    );

    // A one-second-old token stays observed during the grace interval, but
    // must not steal a quota target from a usable legacy reference.
    let database = settings
        .native_connections
        .iter_mut()
        .find(|r| r.id == "z-database")
        .unwrap();
    database.token_expired_at = Some(now() - 1);
    assert_eq!(quota_sources(&settings, &account)[0].id, "native-a-json");
    let database = settings
        .native_connections
        .iter_mut()
        .find(|r| r.id == "z-database")
        .unwrap();
    database.token_expired_at = None;
    settings
        .native_connections
        .iter_mut()
        .find(|r| r.id == "a-json")
        .unwrap()
        .token_expired_at = Some(now() - 1);
    assert_eq!(
        quota_sources(&settings, &account)[0].id,
        "native-z-database"
    );

    // Duplicate enabled DB rows have the same eligibility rule.
    let mut duplicate = settings
        .native_connections
        .iter()
        .find(|r| r.id == "z-database")
        .unwrap()
        .clone();
    duplicate.id = "zz-database-usable".into();
    duplicate.selector = "/google-gemini-cli#3".into();
    settings.native_connections.push(duplicate);
    settings
        .native_connections
        .iter_mut()
        .find(|r| r.id == "z-database")
        .unwrap()
        .token_expired_at = Some(now() - 1);
    assert_eq!(
        quota_sources(&settings, &account)[0].id,
        "native-zz-database-usable"
    );
}

#[test]
fn omp_database_wal_is_discovered_and_disabled_rows_leave_the_enabled_pool() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = database(root);
    put(&db, 1, TOKEN, "project-one");
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let path = root.join("agent.db");
    let before = std::fs::read(&path).unwrap();
    let signature = file_signature(std::slice::from_ref(&path));
    assert!(spec("omp").1.contains(&"agent.db"));
    put(&db, 2, OTHER, "project-two");
    assert_ne!(signature, file_signature(std::slice::from_ref(&path)));
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert_eq!(database_rows(root, "local").len(), 2);
    db.execute(
        "UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=1",
        [],
    )
    .unwrap();
    let rows = database_rows(root, "local");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].connection.selector, "/google-gemini-cli#2");
}

#[test]
fn gemini_local_reload_requires_fingerprint_and_server_identity_after_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = database(root);
    put(&db, 1, TOKEN, "project-one");
    let store = crate::store::Store::open(&root.join("store")).unwrap();
    let mut first = database_rows(root, "local");
    seed_probe(&first[0], false);
    probe_identities(&store, &Settings::default(), &mut first, false);
    let settings = settings_for(&mut first);
    let row = &first[0].connection;
    store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2)",
            rusqlite::params![
                format!("native-fingerprint:{}", row.id),
                first[0].fingerprint
            ],
        )
        .unwrap();
    let account = &settings.accounts[0];
    let old = QuotaSnapshot {
        account_id: account.id.clone(),
        provider: "omp".into(),
        snapshot_at: Some(777),
        meters: vec![QuotaMeter {
            id: "gemini-credits".into(),
            remaining: Some(1234.),
            used_known: Some(false),
            ..Default::default()
        }],
        ..Default::default()
    };
    store
        .db
        .execute(
            "INSERT INTO quotas VALUES(?1,?2)",
            rusqlite::params![accounts::key(account), serde_json::to_string(&old).unwrap()],
        )
        .unwrap();
    let source = quota_sources(&settings, account).remove(0);
    assert_eq!(
        reload_verified(&store, &settings, row)
            .unwrap()
            .connection
            .identity_key,
        row.identity_key
    );

    put(&db, 1, "PRIVATE-GEMINI-ROTATED", "project-one");
    let changed = database_rows(root, "local").remove(0);
    assert_eq!(changed.connection.id, row.id);
    assert_ne!(changed.fingerprint, first[0].fingerprint);
    seed_probe(&changed, true);
    let mut retained = vec![changed.clone()];
    probe_identities(&store, &settings, &mut retained, true);
    assert_eq!(retained[0].connection.identity_key, row.identity_key);
    assert_eq!(retained[0].fingerprint, first[0].fingerprint);
    assert!(
        reload_verified(&store, &settings, row)
            .unwrap()
            .connection
            .identity_key
            .is_none()
    );
    assert!(read_native_quota(&store, &settings, account, &source).is_err());
    seed_probe(&changed, false);
    assert_eq!(
        reload_verified(&store, &settings, row)
            .unwrap()
            .connection
            .identity_key,
        row.identity_key
    );

    for (token, project) in [(OTHER, "project-one"), (TOKEN, "project-two")] {
        put(&db, 1, token, project);
        let changed = database_rows(root, "local").remove(0);
        seed_probe(&changed, false);
        assert_ne!(
            reload_verified(&store, &settings, row)
                .unwrap()
                .connection
                .identity_key,
            row.identity_key
        );
        assert_eq!(
            read_native_quota(&store, &settings, account, &source)
                .unwrap_err()
                .to_string(),
            "原生登录账户已变化，正在重新识别"
        );
    }
    db.execute(
        "UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=1",
        [],
    )
    .unwrap();
    assert!(reload_candidate(&store, &settings, row).is_err());
    drop(db);
    let replacement = root.join("replacement");
    let db = database(&replacement);
    put(&db, 1, OTHER, "project-one");
    drop(db);
    std::fs::rename(replacement.join("agent.db"), root.join("agent.db")).unwrap();
    let changed = database_rows(root, "local").remove(0);
    seed_probe(&changed, false);
    assert_eq!(changed.connection.id, row.id);
    assert_ne!(changed.fingerprint, first[0].fingerprint);
    assert!(read_native_quota(&store, &settings, account, &source).is_err());
    assert_eq!(
        last_snapshot(&store, account).unwrap().snapshot_at,
        Some(777)
    );
    assert!(
        !serde_json::to_string(&settings)
            .unwrap()
            .contains("PRIVATE")
    );
}

#[cfg(unix)]
mod ssh;

#[test]
fn gemini_remote_stale_discovery_keeps_verified_history_without_verifying_a_new_token() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = document(
        root,
        "auth.json",
        json!({"google-gemini-cli":entry(TOKEN, "project-one")}),
    );
    let mut known = extract("pi", "quota-fixture", &doc).unwrap().remove(0);
    bind(&mut known);
    let store = crate::store::Store::open(&root.join("store")).unwrap();
    let settings = Settings {
        native_connections: vec![known.connection.clone()],
        ..Default::default()
    };
    store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2)",
            rusqlite::params![
                format!("native-fingerprint:{}", known.connection.id),
                known.fingerprint
            ],
        )
        .unwrap();
    for token in [TOKEN, OTHER] {
        let mut value = entry(token, "project-one");
        value["expires"] = json!(now() - STALE_AFTER - 1);
        let doc = document(root, "auth.json", json!({"google-gemini-cli":value}));
        let mut aged = extract("pi", "quota-fixture", &doc).unwrap();
        let unverified = aged[0].fingerprint.clone();
        remote::probe_scan_identities(&store, &settings, &mut aged);
        assert_eq!(
            aged[0].connection.identity_key,
            known.connection.identity_key
        );
        assert_eq!(aged[0].connection.status, "stale");
        assert_eq!(aged[0].fingerprint, known.fingerprint);
        assert_ne!(aged[0].fingerprint, unverified);
        assert_eq!(
            stored_fingerprint(&store, &known.connection.id),
            Some(known.fingerprint.clone())
        );
    }
}
