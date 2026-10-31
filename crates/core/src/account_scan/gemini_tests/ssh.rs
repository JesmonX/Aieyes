use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[test]
fn gemini_native_ssh_scan_quota_and_lifecycle_fixture() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let ssh = root.join("ssh");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/native-quota-ssh.py"),
        &ssh,
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "account_scan::gemini_tests::ssh::gemini_native_ssh_worker",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &root)
        .env("AIEYES_AUTO_SCAN", "off")
        .env("AIEYES_NATIVE_QUOTA_FIXTURE", &root)
        .env("AIEYES_GEMINI_SSH_WORKER", "1")
        .env("AIEYES_NATIVE_QUOTA_ARGV_LOG", root.join("argv.jsonl"))
        .env("XDG_CONFIG_HOME", root.join(".config"))
        .env("XDG_DATA_HOME", root.join(".local/share"))
        .env("PI_CONFIG_DIR", ".omp")
        .env("OMP_PROFILE", "default")
        .env_remove("PI_PROFILE")
        .env_remove("PI_CODING_AGENT_DIR")
        .env_remove("AIEYES_NATIVE_QUOTA_REMOTE_ENV")
        .env_remove("COPILOT_GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let args = std::fs::read_to_string(root.join("argv.jsonl")).unwrap();
    assert!(!args.is_empty());
    for private in [TOKEN, OTHER, REFRESH] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(private));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(private));
        assert!(!args.contains(private));
    }
}

#[test]
fn gemini_native_ssh_worker() {
    if std::env::var("AIEYES_GEMINI_SSH_WORKER").as_deref() != Ok("1") {
        return;
    }
    let root = PathBuf::from(std::env::var_os("AIEYES_NATIVE_QUOTA_FIXTURE").unwrap());
    worker(&root);
}

#[derive(Clone)]
struct Seen {
    method: String,
    path: String,
    auth: String,
    body: Value,
}

#[derive(Default)]
struct State {
    seen: Vec<Seen>,
    quota_status: Option<u16>,
    change_on_quota: Option<PathBuf>,
}

fn scan(engine: &mut Engine) -> Value {
    let result = engine
        .scan_accounts(json!({"targets":[["omp","quota-fixture"],["pi","quota-fixture"]]}))
        .unwrap();
    let public = result.to_string();
    for private in [TOKEN, OTHER, REFRESH] {
        assert!(!public.contains(private));
    }
    result
}

fn selected(settings: &Settings, project: &str) -> (Account, Source) {
    let key = format!(
        "google-gemini-cli:{}",
        hash(&json!([SUBJECT, project]).to_string())
    );
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == "omp" && a.upstream_identity.as_deref() == Some(&key))
        .unwrap()
        .clone();
    let source = quota_sources(settings, &account).remove(0);
    (account, source)
}

fn profile_roots(root: &Path) {
    let data = root.join("profile-data/omp/profiles/work");
    let config = root.join(".profiles-omp/profiles/work/agent");
    let db = database(&data);
    put(&db, 1, TOKEN, "project-one");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("auth.json"),
        json!({"google-gemini-cli":entry(TOKEN, "project-one")}).to_string(),
    )
    .unwrap();
    let saved: Vec<_> = ["PI_CONFIG_DIR", "OMP_PROFILE", "XDG_DATA_HOME"]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect();
    unsafe {
        std::env::set_var("PI_CONFIG_DIR", ".profiles-omp");
        std::env::set_var("OMP_PROFILE", "work");
        std::env::set_var("XDG_DATA_HOME", root.join("profile-data"));
    }
    let loaded = load("omp", "local", &Settings::default()).unwrap();
    assert!(loaded.errors.is_empty());
    assert!(
        loaded
            .documents
            .iter()
            .any(|d| d.path == data.join("agent.db").to_string_lossy())
    );
    assert!(
        loaded
            .documents
            .iter()
            .any(|d| d.path == config.join("auth.json").to_string_lossy())
    );
    let watched = watched_files("omp", &Settings::default());
    assert!(watched.contains(&data.join("agent.db")));
    assert!(watched.contains(&config.join("auth.json")));
    for (key, value) in saved {
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn worker(root: &Path) {
    profile_roots(root);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // A dedicated subprocess owns these test-only variables.
    unsafe {
        std::env::set_var("AIEYES_NATIVE_QUOTA_HTTP", &base);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(State::default()));
    let server_stop = stop.clone();
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let parts: Vec<_> = first.split_whitespace().collect();
            let (method, path) = (parts[0].to_owned(), parts[1].to_owned());
            let mut headers = std::collections::BTreeMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                let (key, value) = line.split_once(':').unwrap();
                headers.insert(key.to_ascii_lowercase(), value.trim().to_owned());
            }
            let mut raw = vec![
                0;
                headers
                    .get("content-length")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            ];
            reader.read_exact(&mut raw).unwrap();
            let body = serde_json::from_slice::<Value>(&raw).unwrap_or(Value::Null);
            let auth = headers.get("authorization").cloned().unwrap_or_default();
            let mut status = 200;
            let mut data = match path.as_str() {
                "/oauth2/v2/userinfo" => {
                    let mut user = fixture()["userinfo"].clone();
                    if auth == format!("Bearer {OTHER}") {
                        user["id"] = json!("223456789012345678901");
                    }
                    user
                }
                "/v1internal:loadCodeAssist" => {
                    let mut load = fixture()["paidAssist"].clone();
                    load["cloudaicompanionProject"] = body["cloudaicompanionProject"]
                        .as_str()
                        .unwrap_or("project-one")
                        .into();
                    load
                }
                "/v1internal:retrieveUserQuota" => fixture()["quota"].clone(),
                _ => {
                    status = 404;
                    json!({})
                }
            };
            {
                let mut state = server_state.lock().unwrap();
                state.seen.push(Seen {
                    method,
                    path: path.clone(),
                    auth,
                    body,
                });
                if path == "/v1internal:retrieveUserQuota" {
                    status = state.quota_status.unwrap_or(200);
                    if status != 200 {
                        data = json!({"privateError":REFRESH});
                    }
                    if let Some(path) = state.change_on_quota.take() {
                        let db = rusqlite::Connection::open(path).unwrap();
                        put(&db, 1, OTHER, "project-one");
                    }
                }
            }
            let body = data.to_string();
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });

    let native = root.join(".omp/agent");
    let db = database(&native);
    put(&db, 1, TOKEN, "project-one");
    put(&db, 2, TOKEN, "project-two");
    // The real remote scanner must skip malformed numeric rows without
    // poisoning the JSON document that carries the two valid pool members.
    for (id, number) in [(100, "1e400".to_owned()), (101, "9".repeat(400))] {
        let raw = format!("{{\"access\":\"PRIVATE-BAD-NUMBER\",\"expires\":{number}}}");
        db.execute(
            "INSERT INTO auth_credentials VALUES(?1,'google-gemini-cli','oauth',?2,NULL,NULL,1,1)",
            rusqlite::params![id, raw],
        )
        .unwrap();
    }
    let legacy = json!({"google-gemini-cli":entry(TOKEN, "project-one")});
    std::fs::write(native.join("auth.json"), legacy.to_string()).unwrap();
    let pi = root.join(".pi/agent");
    std::fs::create_dir_all(&pi).unwrap();
    std::fs::write(pi.join("auth.json"), legacy.to_string()).unwrap();
    let mut engine = Engine::open(&root.join("store")).unwrap();
    let settings = Settings {
        local_device_enabled: false,
        proxy: ProxyConfig {
            mode: "custom".into(),
            url: "http://127.0.0.1:9".into(),
        },
        hosts: vec![Host {
            id: "quota-fixture".into(),
            target: "quota-fixture.invalid".into(),
            ..Default::default()
        }],
        agents: ["pi", "omp"]
            .into_iter()
            .map(|provider| AgentConfiguration {
                provider: provider.into(),
                enabled: true,
                machine_ids: vec!["quota-fixture".into()],
            })
            .collect(),
        sources: [("pi", &pi), ("omp", &native)]
            .into_iter()
            .map(|(provider, path)| Source {
                id: format!("{provider}-history"),
                provider: provider.into(),
                path: path.to_string_lossy().into(),
                host_id: Some("quota-fixture".into()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    engine.store.save_auth_settings(&settings).unwrap();
    let initial = scan(&mut engine);
    assert_eq!(initial["addedAccounts"], 3);
    let settings = engine.store.settings().unwrap();
    assert_eq!(settings.native_connections.len(), 4);
    let (account, source) = selected(&settings, "project-one");
    let row = settings
        .native_connections
        .iter()
        .find(|r| source.id == format!("native-{}", r.id))
        .unwrap();
    assert!(row.config_path.ends_with("agent.db"));
    assert_eq!(row.selector, "/google-gemini-cli#1");
    let row_id = row.id.clone();
    assert_eq!(
        settings
            .accounts
            .iter()
            .map(|a| quota_sources(&settings, a).len())
            .sum::<usize>(),
        2
    );
    let start = state.lock().unwrap().seen.len();
    let snapshot = read_native_quota(&engine.store, &settings, &account, &source).unwrap();
    assert_eq!(snapshot.provider, "omp");
    assert_eq!(snapshot.windows.len(), 3);
    assert_eq!(snapshot.meters[0].remaining, Some(1234.));
    assert_eq!(snapshot.meters[0].used_known, Some(false));
    assert_eq!(
        snapshot.identity.as_ref().map(|i| &i.key),
        account.upstream_identity.as_ref()
    );
    let requests = state.lock().unwrap().seen[start..].to_vec();
    assert_eq!(
        requests.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        [
            "/oauth2/v2/userinfo",
            "/v1internal:loadCodeAssist",
            "/oauth2/v2/userinfo",
            "/v1internal:loadCodeAssist",
            "/v1internal:retrieveUserQuota"
        ]
    );
    for request in &requests {
        assert_eq!(request.auth, format!("Bearer {TOKEN}"));
        if request.path.ends_with("loadCodeAssist") {
            assert_eq!(request.method, "POST");
            assert_eq!(request.body["cloudaicompanionProject"], "project-one");
        }
    }
    assert_eq!(requests.last().unwrap().body["project"], "project-one");
    let mut local_source = source.clone();
    local_source.host_id = None;
    let local = crate::subscriptions::gemini::read_using(
        &local_source,
        &crate::upstream::auth("gemini", TOKEN, &entry(TOKEN, "project-one")),
        account.upstream_identity.as_deref(),
        |mut request| {
            let url = reqwest::Url::parse(&request.url).unwrap();
            request.url = format!("{base}{}", url.path());
            request.send(&ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            })
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&local.windows).unwrap(),
        serde_json::to_value(&snapshot.windows).unwrap()
    );
    assert_eq!(local.meters, snapshot.meters);
    assert_eq!(local.plan, snapshot.plan);
    for value in [&snapshot, &local] {
        assert!(!serde_json::to_string(value).unwrap().contains("PRIVATE"));
    }
    engine
        .store
        .db
        .execute(
            "INSERT INTO quotas VALUES(?1,?2)",
            rusqlite::params![
                accounts::key(&account),
                serde_json::to_string(&snapshot).unwrap()
            ],
        )
        .unwrap();

    state.lock().unwrap().quota_status = Some(503);
    let partial = read_native_quota(&engine.store, &settings, &account, &source).unwrap();
    assert!(partial.windows.is_empty() && partial.metadata_error.is_some());
    assert_eq!(partial.meters[0].remaining, Some(1234.));
    state.lock().unwrap().quota_status = Some(401);
    assert!(read_native_quota(&engine.store, &settings, &account, &source).is_err());
    state.lock().unwrap().quota_status = None;
    state.lock().unwrap().change_on_quota = Some(native.join("agent.db"));
    // Credit-only fallback cannot conceal the transport's after-read pin guard.
    assert!(read_native_quota(&engine.store, &settings, &account, &source).is_err());
    assert_eq!(
        last_snapshot(&engine.store, &account).unwrap().snapshot_at,
        snapshot.snapshot_at
    );

    // The same native row id now authenticates another Google subject.
    let before = state
        .lock()
        .unwrap()
        .seen
        .iter()
        .filter(|r| r.path.ends_with("retrieveUserQuota"))
        .count();
    assert_eq!(
        read_native_quota(&engine.store, &settings, &account, &source)
            .unwrap_err()
            .to_string(),
        "原生登录账户已变化，正在重新识别"
    );
    assert_eq!(
        state
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|r| r.path.ends_with("retrieveUserQuota"))
            .count(),
        before
    );
    let result = scan(&mut engine);
    assert_eq!(result["addedAccounts"], 1);
    let changed = engine.store.settings().unwrap();
    let replaced = changed
        .native_connections
        .iter()
        .find(|r| r.id == row_id)
        .unwrap();
    assert_ne!(
        replaced.account_key.as_deref(),
        Some(accounts::key(&account).as_str())
    );
    assert!(
        changed
            .native_connections
            .iter()
            .any(|r| r.config_path == row.config_path
                && r.selector == row.selector
                && r.status == "remembered")
    );
    assert_eq!(
        last_snapshot(&engine.store, &account).unwrap().snapshot_at,
        snapshot.snapshot_at
    );

    // A WAL-only new row is rediscovered; disabling another row keeps its
    // historical account and snapshot but removes it as a query source.
    engine.changed_remote_targets(&changed).unwrap();
    let (second, second_source) = selected(&changed, "project-two");
    let old = QuotaSnapshot {
        provider: "omp".into(),
        account_id: second.id.clone(),
        snapshot_at: Some(888),
        meters: snapshot.meters.clone(),
        ..Default::default()
    };
    engine
        .store
        .db
        .execute(
            "INSERT INTO quotas VALUES(?1,?2)",
            rusqlite::params![accounts::key(&second), serde_json::to_string(&old).unwrap()],
        )
        .unwrap();
    let second_row = changed
        .native_connections
        .iter()
        .find(|r| second_source.id == format!("native-{}", r.id))
        .unwrap();
    let verified_fingerprint = stored_fingerprint(&engine.store, &second_row.id).unwrap();
    for (token, offset) in [(TOKEN, -300), (OTHER, -300), (OTHER, -STALE_AFTER - 1)] {
        let mut expired = entry(token, "project-two");
        expired["expires"] = json!(now() + offset);
        db.execute(
            "UPDATE auth_credentials SET data=?1 WHERE id=2",
            [expired.to_string()],
        )
        .unwrap();
        let calls = state.lock().unwrap().seen.len();
        assert_eq!(scan(&mut engine)["addedAccounts"], 0);
        let observed = engine.store.settings().unwrap();
        let retained = observed
            .native_connections
            .iter()
            .find(|r| r.id == second_row.id)
            .unwrap();
        assert_eq!(retained.identity_key, second_row.identity_key);
        assert_eq!(retained.account_key, second_row.account_key);
        assert_eq!(
            retained.status,
            if offset <= -STALE_AFTER {
                "stale"
            } else {
                "observed"
            }
        );
        assert_eq!(
            stored_fingerprint(&engine.store, &retained.id).as_deref(),
            Some(verified_fingerprint.as_str())
        );
        let actual = database_rows(&native, "quota-fixture")
            .into_iter()
            .find(|c| c.connection.id == retained.id)
            .unwrap();
        assert_ne!(actual.fingerprint, verified_fingerprint);
        assert!(read_native_quota(&engine.store, &observed, &second, &second_source).is_err());
        assert_eq!(
            state.lock().unwrap().seen.len(),
            calls,
            "expired Gemini credentials never start an HTTP request"
        );
        assert_eq!(
            last_snapshot(&engine.store, &second).unwrap().snapshot_at,
            Some(888)
        );
    }
    put(&db, 2, TOKEN, "project-two");
    assert_eq!(scan(&mut engine)["addedAccounts"], 0);
    put(&db, 3, TOKEN, "project-three");
    db.execute(
        "UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=2",
        [],
    )
    .unwrap();
    // Advance the existing five-minute metadata poll schedule without sleeping.
    engine
        .store
        .db
        .execute(
            "DELETE FROM kv WHERE key='native-remote-poll:quota-fixture'",
            [],
        )
        .unwrap();
    assert!(
        engine
            .changed_remote_targets(&changed)
            .unwrap()
            .contains(&("omp".into(), "quota-fixture".into()))
    );
    let result = scan(&mut engine);
    assert_eq!(result["addedAccounts"], 1);
    let current = engine.store.settings().unwrap();
    assert!(quota_sources(&current, &second).is_empty());
    assert!(current.accounts.iter().any(|a| a.id == second.id));
    assert_eq!(
        last_snapshot(&engine.store, &second).unwrap().snapshot_at,
        Some(888)
    );
    assert!(
        current
            .native_connections
            .iter()
            .any(|r| r.selector == "/google-gemini-cli#2" && r.status == "remembered")
    );

    // Whole-database replacement reusing id=3 cannot feed the previous pool.
    let (third, third_source) = selected(&current, "project-three");
    drop(db);
    let replacement = root.join("replacement");
    let db = database(&replacement);
    put(&db, 3, OTHER, "replacement-project");
    drop(db);
    std::fs::rename(replacement.join("agent.db"), native.join("agent.db")).unwrap();
    assert!(read_native_quota(&engine.store, &current, &third, &third_source).is_err());
    assert!(
        !serde_json::to_string(&engine.store.settings().unwrap())
            .unwrap()
            .contains("PRIVATE")
    );
    assert!(state.lock().unwrap().seen.iter().all(|r| matches!(
        r.path.as_str(),
        "/oauth2/v2/userinfo" | "/v1internal:loadCodeAssist" | "/v1internal:retrieveUserQuota"
    )));
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
}
