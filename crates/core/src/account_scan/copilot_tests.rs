//! Copilot's environment overrides and verified server identities must survive
//! account renames without attributing a replacement token to remembered usage.
use super::*;

fn settings() -> Settings {
    Settings {
        local_device_enabled: true,
        proxy: ProxyConfig {
            mode: "custom".into(),
            url: "http://127.0.0.1:9".into(),
        },
        agents: ["copilot", "pi", "omp"]
            .into_iter()
            .map(|provider| AgentConfiguration {
                provider: provider.into(),
                enabled: true,
                machine_ids: vec!["local".into(), "ssh".into()],
            })
            .collect(),
        hosts: vec![Host {
            id: "ssh".into(),
            target: "fixture.invalid".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn login(root: &Path, token: &str) -> Candidate {
    let doc = Document {
        root: root.to_string_lossy().into(),
        path: root.join("apps.json").to_string_lossy().into(),
        bytes: json!({"github.com:fixture":{"user":"cached-name","oauth_token":token}})
            .to_string()
            .into_bytes(),
    };
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(&doc.path, &doc.bytes).unwrap();
    extract("copilot", "local", &doc).unwrap().remove(0)
}

fn record(store: &crate::store::Store, settings: &mut Settings, c: &mut Candidate) -> Account {
    let account = Account {
        id: "kept-account".into(),
        name: "My account name".into(),
        provider: c.connection.provider.clone(),
        identity_key: c.connection.identity_key.clone(),
        upstream_identity: c.connection.upstream_identity.clone(),
        quota_enabled: true,
        ..Default::default()
    };
    c.connection.account_key = Some(accounts::key(&account));
    c.connection.machine_fingerprint =
        hash(&host_signature(settings, &c.connection.machine_id).to_string());
    settings.accounts.push(account.clone());
    settings.native_connections.push(c.connection.clone());
    settings.sources.push(Source {
        id: "history-source".into(),
        provider: c.connection.provider.clone(),
        account_id: account.id.clone(),
        path: c.connection.root.clone(),
        host_id: (c.connection.machine_id != "local").then(|| c.connection.machine_id.clone()),
        ..Default::default()
    });
    store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2)",
            rusqlite::params![
                format!("native-fingerprint:{}", c.connection.id),
                c.fingerprint
            ],
        )
        .unwrap();
    let snapshot = QuotaSnapshot {
        provider: account.provider.clone(),
        account_id: account.id.clone(),
        updated_at: 123,
        snapshot_at: Some(123),
        meters: vec![QuotaMeter {
            id: "premium".into(),
            used: 50.,
            total: Some(300.),
            remaining: Some(250.),
            unit: "requests".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store
        .db
        .execute(
            "INSERT INTO quotas VALUES(?1,?2)",
            rusqlite::params![
                accounts::key(&account),
                serde_json::to_string(&snapshot).unwrap()
            ],
        )
        .unwrap();
    account
}

#[test]
fn copilot_environment_uses_the_first_nonempty_override_and_needs_no_cli_for_its_own_token() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("not-created")
        .to_string_lossy()
        .into_owned();
    let select = |values: [&str; 3]| {
        copilot_environment(&root, |name| {
            COPILOT_ENV
                .iter()
                .position(|value| *value == name)
                .map(|index| values[index].into())
        })
    };
    assert_eq!(
        select([" gho_high ", "gho_middle", "gho_low"]),
        Some(("COPILOT_GITHUB_TOKEN", "gho_high".into()))
    );
    assert!(select(["", "gho_middle", "gho_low"]).is_none());
    std::fs::create_dir(&root).unwrap();
    assert_eq!(
        select([" \n", "gho_middle", "gho_low"]),
        Some(("GH_TOKEN", "gho_middle".into()))
    );
    assert_eq!(
        select(["", "", "gho_low"]),
        Some(("GITHUB_TOKEN", "gho_low".into()))
    );
    assert!(select(["", "", ""]).is_none());
    for invalid in ["gho_ bad", "gho_bad\0value", "gho_bad\u{7f}value"] {
        assert!(select([invalid, "gho_middle", "gho_low"]).is_none());
    }
    assert!(select([&"x".repeat(32769), "gho_middle", "gho_low"]).is_none());
}

#[test]
fn copilot_quota_prefers_environment_over_files_on_each_host_and_keeps_explicit_choice() {
    for machine in ["local", "ssh"] {
        let mut settings = settings();
        let mut account = Account {
            id: "a".into(),
            provider: "copilot".into(),
            identity_key: Some("copilot:7".into()),
            upstream_identity: Some("github:7".into()),
            ..Default::default()
        };
        settings.accounts.push(account.clone());
        for (id, path) in [
            ("disk", "/native/apps.json"),
            ("granted", COPILOT_GRANTED),
            ("github", "env:GITHUB_TOKEN"),
            ("gh", "env:GH_TOKEN"),
            ("copilot", "env:COPILOT_GITHUB_TOKEN"),
        ] {
            settings.native_connections.push(NativeConnection {
                id: id.into(),
                provider: "copilot".into(),
                machine_id: machine.into(),
                machine_fingerprint: hash(&host_signature(&settings, machine).to_string()),
                root: "/native".into(),
                config_path: path.into(),
                kind: "login".into(),
                status: "observed".into(),
                has_credential: true,
                account_key: Some(accounts::key(&account)),
                identity_key: account.identity_key.clone(),
                upstream_identity: account.upstream_identity.clone(),
                ..Default::default()
            });
        }
        for expected in ["copilot", "gh", "github", "disk"] {
            let selected = quota_sources(&settings, &account);
            assert_eq!(selected.len(), 1, "one quota pool per GitHub user");
            assert_eq!(selected[0].id, format!("native-{expected}"));
            assert_eq!(
                selected[0].host_id.as_deref(),
                (machine == "ssh").then_some("ssh")
            );
            if expected == "copilot" {
                account.quota_source_id = Some("native-disk".into());
                assert_eq!(quota_sources(&settings, &account)[0].id, "native-disk");
                account.quota_source_id = None;
            }
            settings.native_connections.retain(|row| row.id != expected);
        }
    }
}

#[test]
fn copilot_numeric_subject_deduplicates_native_and_upstream_but_not_reused_usernames() {
    let mut settings = settings();
    for (provider, id, upstream) in [
        ("copilot", "native", "github:7"),
        ("pi", "upstream", "github:7"),
    ] {
        let account = Account {
            id: id.into(),
            provider: provider.into(),
            quota_enabled: true,
            ..Default::default()
        };
        settings.native_connections.push(NativeConnection {
            id: id.into(),
            provider: provider.into(),
            machine_id: "local".into(),
            machine_fingerprint: hash(&host_signature(&settings, "local").to_string()),
            root: "/native".into(),
            config_path: if provider == "copilot" {
                "env:COPILOT_GITHUB_TOKEN"
            } else {
                "/native/auth.json"
            }
            .into(),
            selector: if provider == "pi" {
                "/github-copilot"
            } else {
                ""
            }
            .into(),
            kind: "login".into(),
            status: "observed".into(),
            has_credential: true,
            email: "same-name".into(),
            account_key: Some(accounts::key(&account)),
            upstream_identity: Some(upstream.into()),
            ..Default::default()
        });
        settings.accounts.push(account);
    }
    assert_eq!(quota_sources(&settings, &settings.accounts[0]).len(), 1);
    assert!(quota_sources(&settings, &settings.accounts[1]).is_empty());
    settings.native_connections[1].upstream_identity = Some("github:8".into());
    assert_eq!(quota_sources(&settings, &settings.accounts[1]).len(), 1);
    settings.native_connections[1].upstream_identity = Some("github:7".into());
    settings.native_connections[0].status = "remembered".into();
    assert_eq!(quota_sources(&settings, &settings.accounts[1]).len(), 1);
}

#[test]
fn copilot_legacy_identity_migrates_only_with_the_same_credential_and_preserves_history_keys() {
    for provider in ["copilot", "pi", "omp"] {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&temp.path().join("store")).unwrap();
        let mut settings = settings();
        let mut old = login(&temp.path().join("native"), "gho_OLD_PRIVATE");
        old.connection.provider = provider.into();
        old.connection.selector = if provider == "copilot" {
            ""
        } else {
            "/github-copilot"
        }
        .into();
        old.connection.identity_key = Some(format!("{provider}:{}", hash("old-login")));
        old.connection.upstream_identity = Some(format!("github:{}", hash("old-login")));
        let account = record(&store, &mut settings, &mut old);
        let mut verified = old.clone();
        verified.connection.identity_key = Some(format!("{provider}:numeric-fixture"));
        verified.connection.upstream_identity = Some("github:7".into());
        verified.connection.email = "renamed-login".into();
        // A matching display name or connection path is insufficient if the
        // token changed: GitHub permits reusing someone else's previous name.
        verified.fingerprint = hash("gho_DIFFERENT_PRIVATE");
        migrate_github_identity(&store, &mut settings, &verified);
        assert_eq!(settings.accounts[0].identity_key, account.identity_key);
        verified.fingerprint = old.fingerprint.clone();
        migrate_github_identity(&store, &mut settings, &verified);
        assert_eq!(settings.accounts.len(), 1);
        assert_eq!(settings.accounts[0].id, account.id);
        assert_eq!(settings.accounts[0].name, "My account name");
        assert_eq!(
            settings.accounts[0].identity_key,
            verified.connection.identity_key
        );
        assert_eq!(
            settings.accounts[0].upstream_identity.as_deref(),
            Some("github:7")
        );
        assert_eq!(
            settings.native_connections[0].identity_key,
            verified.connection.identity_key
        );
        assert_eq!(settings.sources[0].account_id, account.id);
        let snapshot = last_snapshot(&store, &settings.accounts[0]).unwrap();
        assert_eq!(snapshot.meters[0].remaining, Some(250.));
        assert_eq!(snapshot.snapshot_at, Some(123));
        assert!(
            !serde_json::to_string(&settings)
                .unwrap()
                .contains("PRIVATE")
        );
    }
}

#[test]
fn copilot_legacy_fingerprint_requires_a_new_server_id_and_never_guesses_from_a_cached_name() {
    let temp = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(&temp.path().join("store")).unwrap();
    let mut settings = settings();
    let mut old = login(&temp.path().join("native"), "gho_PRIVATE");
    old.connection.identity_key = Some(crate::subscriptions::copilot_identity("cached-name"));
    old.connection.upstream_identity = Some(format!("github:{}", hash("cached-name")));
    record(&store, &mut settings, &mut old);
    let mut fresh = login(&temp.path().join("native"), "gho_PRIVATE");
    assert!(!reuse_verified_identity(&store, &settings, &mut fresh));
    copilot_identities(&store, &settings, std::slice::from_mut(&mut fresh), |_| {
        Ok(crate::upstream::github_identity(&json!({"id":7,"login":"renamed"})).unwrap())
    });
    assert_eq!(fresh.connection.identity_key.as_deref(), Some("copilot:7"));
    let mut unverified = login(&temp.path().join("another"), "gho_UNKNOWN_PRIVATE");
    copilot_identities(
        &store,
        &settings,
        std::slice::from_mut(&mut unverified),
        |_| Err(anyhow::anyhow!("GitHub 未返回有效的账户 ID")),
    );
    assert!(unverified.connection.identity_key.is_none());
    assert!(unverified.connection.upstream_identity.is_none());
}

#[test]
fn copilot_failed_token_replacement_keeps_the_snapshot_but_cannot_read_as_the_old_account() {
    let temp = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(&temp.path().join("store")).unwrap();
    let mut settings = settings();
    let root = temp.path().join("native");
    let mut old = login(&root, "gho_OLD_PRIVATE");
    bind_github_identity(
        &mut old.connection,
        &crate::upstream::github_identity(&json!({"id":7,"login":"old-user"})).unwrap(),
    );
    let account = record(&store, &mut settings, &mut old);
    let mut replacement = login(&root, "gho_NEW_PRIVATE");
    let replacement_fingerprint = replacement.fingerprint.clone();
    copilot_identities(
        &store,
        &settings,
        std::slice::from_mut(&mut replacement),
        |_| Err(anyhow::anyhow!("GitHub 令牌无效或已过期")),
    );
    assert_eq!(
        replacement.connection.identity_key,
        old.connection.identity_key
    );
    assert_eq!(replacement.fingerprint, old.fingerprint);
    assert_ne!(replacement.fingerprint, replacement_fingerprint);
    let source = Source {
        id: format!("native-{}", old.connection.id),
        provider: "copilot".into(),
        path: root.to_string_lossy().into(),
        account_id: account.id.clone(),
        ..Default::default()
    };
    // The mismatching fingerprint stops in the local file/identity check, so
    // no live GitHub request is possible in this regression.
    assert_eq!(
        read_native_quota(&store, &settings, &account, &source)
            .unwrap_err()
            .to_string(),
        "原生登录账户已变化，正在重新识别"
    );
    let snapshot = last_snapshot(&store, &account).unwrap();
    assert_eq!(snapshot.snapshot_at, Some(123));
    assert_eq!(snapshot.meters[0].remaining, Some(250.));
}

#[cfg(unix)]
#[test]
fn copilot_half_written_files_keep_snapshots_and_recover_on_local_and_ssh() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let ssh = root.join("ssh");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/native-quota-ssh.py"),
        &ssh,
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let home = root.join("home");
    std::fs::create_dir(&home).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "account_scan::copilot_tests::copilot_half_written_fixture_worker",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("AIEYES_COPILOT_HALF_WRITTEN_FIXTURE", &root)
        .env("AIEYES_NATIVE_QUOTA_FIXTURE", &root)
        .env("AIEYES_AUTO_SCAN", "off")
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("COPILOT_HOME", home.join(".copilot"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GH_CONFIG_DIR", home.join(".config/gh"))
        .env_remove("COPILOT_GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("AIEYES_NATIVE_QUOTA_REMOTE_ENV")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("gho_HALF_WRITTEN_PRIVATE"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("gho_HALF_WRITTEN_PRIVATE"));
}

#[test]
fn copilot_half_written_fixture_worker() {
    let Some(root) = std::env::var_os("AIEYES_COPILOT_HALF_WRITTEN_FIXTURE") else {
        return;
    };
    let root = Path::new(&root);
    let native = root.join("home/.copilot");
    let configs = [
        native.join("config.json"),
        root.join("home/.config/github-copilot/apps.json"),
    ];
    std::fs::create_dir_all(&native).unwrap();
    for machine in ["local", "ssh"] {
        for (index, path) in configs.iter().enumerate() {
            for other in &configs {
                let _ = std::fs::remove_file(other);
            }
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let value = if index == 0 {
                json!({"copilot_tokens":{"https://github.com:cached-name":"gho_HALF_WRITTEN_PRIVATE"}})
            } else {
                json!({"github.com:fixture":{"user":"cached-name","oauth_token":"gho_HALF_WRITTEN_PRIVATE"}})
            };
            let doc = Document {
                root: native.to_string_lossy().into(),
                path: path.to_string_lossy().into(),
                bytes: value.to_string().into_bytes(),
            };
            std::fs::write(path, &doc.bytes).unwrap();
            let mut row = extract("copilot", machine, &doc).unwrap().remove(0);
            bind_github_identity(
                &mut row.connection,
                &crate::upstream::github_identity(&json!({"id":7,"login":"cached-name"})).unwrap(),
            );
            let mut engine = Engine::open(&root.join(format!("store-{machine}-{index}"))).unwrap();
            let mut settings = settings();
            settings.agents.retain(|agent| agent.provider == "copilot");
            settings.hosts[0].target = "quota-fixture.invalid".into();
            let account = record(&engine.store, &mut settings, &mut row);
            engine.store.save_auth_settings(&settings).unwrap();
            let scan = |engine: &mut Engine| {
                engine
                    .scan_accounts(json!({"targets":[["copilot",machine]]}))
                    .unwrap()
            };
            let initial = scan(&mut engine);
            assert_eq!(
                initial["settings"]["nativeConnections"][0]["status"], "observed",
                "{machine} {index}: {initial}"
            );
            assert_eq!(initial["settings"]["accounts"].as_array().unwrap().len(), 1);
            // JSONC and JSON syntax errors both retain the previous row. A
            // new syntactically valid token-less object is tested separately.
            std::fs::write(
                path,
                if index == 0 {
                    "// native write\n{\"copilot_tokens\":"
                } else {
                    "{\"github.com:fixture\":"
                },
            )
            .unwrap();
            let torn = scan(&mut engine);
            assert!(
                torn["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["status"] == "retained"),
                "{machine} {index}: {torn}"
            );
            assert_eq!(
                torn["settings"]["nativeConnections"],
                initial["settings"]["nativeConnections"]
            );
            assert_eq!(
                torn["settings"]["accounts"],
                initial["settings"]["accounts"]
            );
            engine
                .store
                .db
                .execute(
                    "UPDATE kv SET value=?1 WHERE key=?2",
                    rusqlite::params![
                        (now() - STALE_AFTER - 1).to_string(),
                        format!("native-failed-since:{}", row.connection.id)
                    ],
                )
                .unwrap();
            let stale = scan(&mut engine);
            assert_eq!(stale["settings"]["nativeConnections"][0]["status"], "stale");
            assert_eq!(
                last_snapshot(&engine.store, &account).unwrap().snapshot_at,
                Some(123)
            );
            std::fs::write(path, &doc.bytes).unwrap();
            let restored = scan(&mut engine);
            assert_eq!(
                restored["settings"]["nativeConnections"][0]["status"],
                "observed"
            );
            assert_eq!(restored["settings"]["accounts"][0]["id"], account.id);
            assert_eq!(
                restored["settings"]["accounts"].as_array().unwrap().len(),
                1
            );
            assert_eq!(
                last_snapshot(&engine.store, &account).unwrap().meters[0].remaining,
                Some(250.)
            );
            std::fs::write(path, "{}").unwrap();
            let logged_out = scan(&mut engine);
            assert_eq!(
                logged_out["settings"]["nativeConnections"][0]["status"],
                "remembered"
            );
        }
    }
}
