use aieyes_core::{import, models::*, store::Store, usage};
use serde_json::json;

#[test]
fn host_uptime_default_preserves_explicit_legacy_details() {
    assert!(Host::default().details.contains(&"uptime".into()));
    let legacy: Host = serde_json::from_value(json!({"id":"legacy"})).unwrap();
    assert!(legacy.details.contains(&"uptime".into()));
    for details in [json!([]), json!(["gpuMemory", "networkTotals"])] {
        let host: Host =
            serde_json::from_value(json!({"id":"legacy", "details":details, "metrics":[]}))
                .unwrap();
        assert_eq!(serde_json::to_value(&host).unwrap()["details"], details);
        assert!(!host.details.contains(&"uptime".into()));
    }
    let host: Host = serde_json::from_value(json!({"metrics":[], "details":["uptime"]})).unwrap();
    assert_eq!(host.details, ["uptime"]);
    assert!(host.metrics.is_empty());
}

fn source(id: &str) -> Source {
    Source {
        id: id.into(),
        account_id: "account".into(),
        provider: "custom".into(),
        path: "/data".into(),
        ..Default::default()
    }
}
fn event(source: &Source, id: &str, input: u64) -> UsageEvent {
    usage::parse(source,&mut ParseState::default(),&json!({"id":id,"sessionId":"session","model":"model-a","timestamp":now(),"tokens":{"input":input,"output":20}})).unwrap()
}
#[test]
fn model_choices_preserve_siblings_and_honor_other_filters() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let a = source("a");
    let b = Source {
        account_id: "other".into(),
        ..source("b")
    };
    let settings = Settings {
        sources: vec![a.clone(), b.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    for (id, model, src, age) in [
        ("one", "model-a", &a, 0),
        ("two", "model-b", &a, 0),
        ("three", "model-c", &b, 0),
        ("old", "old-model", &a, 40 * 86400),
    ] {
        let mut e = event(src, id, 100);
        e.model = model.into();
        e.timestamp -= age;
        db.put_event(&e, &[], &settings).unwrap();
    }
    let d = db
        .dashboard(&Filter {
            provider: Some("custom".into()),
            account_id: Some("account".into()),
            source_id: Some("a".into()),
            model: Some("model-a".into()),
            days: Some(7),
        })
        .unwrap();
    assert_eq!(d.model_options, ["model-a", "model-b"]);
    assert_eq!(d.summary.total, 120);
    assert_eq!(d.models.len(), 1);
    let d = db
        .dashboard(&Filter {
            account_id: Some("other".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.model_options, ["model-c"]);
    let d = db
        .dashboard(&Filter {
            provider: Some("codex".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(d.model_options.is_empty());
}
#[test]
fn copied_records_have_one_total_and_two_source_views() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let a = source("a");
    let b = source("b");
    let settings = Settings {
        sources: vec![a.clone(), b.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    assert!(
        db.put_event(&event(&a, "one", 100), &[], &settings)
            .unwrap()
    );
    assert!(
        !db.put_event(&event(&b, "one", 100), &[], &settings)
            .unwrap()
    );
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 120);
    for id in ["a", "b"] {
        assert_eq!(
            db.dashboard(&Filter {
                source_id: Some(id.into()),
                ..Default::default()
            })
            .unwrap()
            .summary
            .total,
            120
        );
    }
}
#[test]
fn streaming_updates_replace_usage_and_preserve_price_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let source = source("s");
    let settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    let price = ModelPrice {
        id: "model-a".into(),
        input: Some(0.01),
        output: Some(0.05),
        ..Default::default()
    };
    db.put_event(
        &event(&source, "message", 100),
        std::slice::from_ref(&price),
        &settings,
    )
    .unwrap();
    let changed = ModelPrice {
        input: Some(0.02),
        ..price
    };
    db.save_prices(std::slice::from_ref(&changed)).unwrap();
    db.put_event(&event(&source, "message", 150), &[changed], &settings)
        .unwrap();
    let d = db.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.summary.total, 170);
    assert!((d.summary.cost - 2.5).abs() < 1e-9);
    db.reprice(&settings, false).unwrap();
    assert!((db.dashboard(&Filter::default()).unwrap().summary.cost - 4.0).abs() < 1e-9);
}
#[test]
fn blank_quota_and_failed_refresh_keep_last_data() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let s = Source {
        provider: "codex".into(),
        ..source("s")
    };
    db.save_settings(&Settings {
        sources: vec![s.clone()],
        ..Default::default()
    })
    .unwrap();
    let prior = usage::codex_quota(
        &s,
        &json!({"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300}},"rateLimitResetCredits":{"availableCount":4,"credits":null}}),
        100,
        "live",
    );
    db.quota(&prior).unwrap();
    db.quota(&usage::codex_quota(&s, &json!({}), 110, "log"))
        .unwrap();
    db.quota(&QuotaSnapshot {
        error: Some("offline".into()),
        updated_at: 120,
        ..prior.clone()
    })
    .unwrap();
    let q = db.dashboard(&Filter::default()).unwrap().quotas.remove(0);
    assert_eq!(q.updated_at, 100);
    assert_eq!(q.windows[0].used_percent, 25.0);
    assert_eq!(q.bank_reset.unwrap()["availableCount"], 4);
    db.quota(&usage::codex_quota(
        &s,
        &json!({"primary":{"used_percent":30,"window_minutes":300}}),
        130,
        "log",
    ))
    .unwrap();
    let q = db.dashboard(&Filter::default()).unwrap().quotas.remove(0);
    assert_eq!(q.bank_updated_at, Some(100));
    assert_eq!(q.updated_at, 130);
}
#[test]
fn changing_account_preserves_prior_attribution() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(&temp.path().join("db")).unwrap();
    let path = temp.path().join("usage.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}\n",
            json!({"id":"one","model":"a","timestamp":now(),"tokens":{"input":100}})
        ),
    )
    .unwrap();
    let mut source = Source {
        path: path.to_string_lossy().into(),
        ..source("s")
    };
    let mut settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    import::scan(&db, &settings, &source).unwrap();
    settings = db.settings().unwrap();
    source.account_id = "new-account".into();
    settings.accounts.push(Account {
        id: "new-account".into(),
        provider: source.provider.clone(),
        name: "New account".into(),
        ..Default::default()
    });
    settings.sources = vec![source.clone()];
    db.save_settings(&settings).unwrap();
    import::scan(&db, &settings, &source).unwrap();
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 100);
    assert_eq!(
        db.dashboard(&Filter {
            account_id: Some("new-account".into()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        0
    );
}

#[test]
fn archiving_account_preserves_history_cursors_prices_and_deduplication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let db = Store::open(&root).unwrap();
    let path = temp.path().join("usage.jsonl");
    let log = format!(
        "{}\n",
        json!({"id":"one","model":"model-a","timestamp":now(),"tokens":{"input":100,"output":20}})
    );
    std::fs::write(&path, &log).unwrap();
    let source = Source {
        path: path.to_string_lossy().into(),
        ..source("s")
    };
    let mut settings = Settings {
        version: 2,
        accounts: vec![Account {
            id: source.account_id.clone(),
            name: "Personal".into(),
            provider: source.provider.clone(),
            ..Default::default()
        }],
        sources: vec![source.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    db.save_prices(&[ModelPrice {
        id: "model-a".into(),
        input: Some(0.01),
        output: Some(0.02),
        ..Default::default()
    }])
    .unwrap();
    import::scan(&db, &settings, &source).unwrap();
    let before = db.dashboard(&Filter::default()).unwrap();
    let cursor = db.cursor(&source.id, &source.path).unwrap().unwrap();
    let payload: String = db
        .db
        .query_row("SELECT payload FROM events", [], |r| r.get(0))
        .unwrap();
    let price: String = db
        .db
        .query_row("SELECT price FROM events", [], |r| r.get(0))
        .unwrap();
    std::fs::remove_file(&path).unwrap();

    settings.accounts[0].archived = true;
    db.save_settings(&settings).unwrap();
    let archived = db.settings().unwrap();
    assert!(!archived.quota_enabled(&source));
    let after = db
        .dashboard(&Filter {
            account_id: Some(source.account_id.clone()),
            source_id: Some(source.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(after.summary.total, before.summary.total);
    assert_eq!(after.summary.cost, before.summary.cost);
    assert!(after.quotas.is_empty());
    assert_eq!(
        db.cursor(&source.id, &source.path).unwrap().unwrap().1,
        cursor.1
    );
    assert_eq!(
        db.db
            .query_row::<String, _, _>("SELECT payload FROM events", [], |r| r.get(0))
            .unwrap(),
        payload
    );
    assert_eq!(
        db.db
            .query_row::<String, _, _>("SELECT price FROM events", [], |r| r.get(0))
            .unwrap(),
        price
    );
    let mut engine = aieyes_core::Engine::open(&root).unwrap();
    assert_eq!(engine.call("quotas.refresh", json!({})).unwrap(), json!([]));

    settings.accounts[0].archived = false;
    db.save_settings(&settings).unwrap();
    std::fs::write(&path, log).unwrap();
    import::scan(&db, &settings, &source).unwrap();
    assert_eq!(
        db.dashboard(&Filter::default()).unwrap().summary.total,
        before.summary.total
    );
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.events, 1);
    assert!(db.settings().unwrap().quota_enabled(&source));
}

#[test]
fn deleting_account_with_history_is_rejected_without_changing_settings_or_records() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let source = source("s");
    let settings = Settings {
        version: 2,
        accounts: vec![Account {
            id: source.account_id.clone(),
            name: "Personal".into(),
            provider: source.provider.clone(),
            ..Default::default()
        }],
        sources: vec![source.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    db.put_event(&event(&source, "one", 100), &[], &settings)
        .unwrap();
    let mut draft = settings.clone();
    draft.accounts.clear();
    draft.sources[0].account_id.clear();
    assert!(
        db.save_settings(&draft)
            .unwrap_err()
            .to_string()
            .contains("归档")
    );
    assert_eq!(
        db.settings().unwrap().sources[0].account_id,
        source.account_id
    );
    assert_eq!(db.settings().unwrap().accounts.len(), 1);
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 120);
}

#[test]
fn settings_and_repricing_roll_back_together_if_price_update_fails() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let source = source("s");
    db.save_settings(&Settings {
        sources: vec![source.clone()],
        ..Default::default()
    })
    .unwrap();
    let settings = db.settings().unwrap();
    db.put_event(&event(&source, "one", 100), &[], &settings)
        .unwrap();
    db.save_prices(&[ModelPrice {
        id: "model-a".into(),
        input: Some(0.01),
        output: Some(0.02),
        ..Default::default()
    }])
    .unwrap();
    db.db.execute_batch("CREATE TRIGGER reject_reprice BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
    let mut draft = settings.clone();
    draft.refresh_seconds = 600;
    draft
        .model_mappings
        .insert("model-a".into(), "model-a".into());
    assert!(db.save_settings(&draft).is_err());
    assert_eq!(
        db.settings().unwrap().refresh_seconds,
        settings.refresh_seconds
    );
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.cost, 0.0);
    db.db.execute_batch("DROP TRIGGER reject_reprice;").unwrap();
    db.save_settings(&draft).unwrap();
    assert_eq!(db.settings().unwrap().refresh_seconds, 600);
    assert!(db.dashboard(&Filter::default()).unwrap().summary.cost > 0.0);
}
#[test]
fn source_failures_preserve_success_time() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let s = source("s");
    db.save_settings(&Settings {
        sources: vec![s],
        ..Default::default()
    })
    .unwrap();
    db.source_status("s", &json!({"updatedAt":100,"files":3}))
        .unwrap();
    db.source_status("s", &json!({"error":"offline","failedAt":120}))
        .unwrap();
    let d = db.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.sources[0]["status"]["updatedAt"], 100);
    assert_eq!(d.sources[0]["status"]["files"], 3);
}
#[test]
fn calendar_filter_excludes_the_eighth_day() {
    use chrono::{Duration, Local, TimeZone};
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let s = source("s");
    let settings = Settings {
        sources: vec![s.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    let midnight = Local
        .from_local_datetime(
            &(Local::now().date_naive() - Duration::days(6))
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
        .timestamp();
    let mut included = event(&s, "in", 100);
    included.timestamp = midnight;
    db.put_event(&included, &[], &settings).unwrap();
    let mut excluded = event(&s, "out", 200);
    excluded.timestamp = midnight - 1;
    db.put_event(&excluded, &[], &settings).unwrap();
    let d = db
        .dashboard(&Filter {
            days: Some(7),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.summary.total, 120);
    assert_eq!(d.days.len(), 7);
    assert_eq!(d.heatmap.len(), 365);
}

#[test]
fn legacy_accounts_migrate_and_shared_quota_has_one_identity() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let a = Source {
        provider: "codex".into(),
        name: "Laptop".into(),
        ..source("local")
    };
    let b = Source {
        name: "Server".into(),
        ..a.clone()
    };
    let b = Source {
        id: "remote".into(),
        ..b
    };
    db.save_settings(&Settings {
        sources: vec![a.clone(), b.clone()],
        ..Default::default()
    })
    .unwrap();
    let settings = db.settings().unwrap();
    assert_eq!(settings.version, 2);
    assert_eq!(settings.accounts.len(), 1);
    db.quota(&usage::codex_quota(
        &b,
        &json!({"primary":{"used_percent":25}}),
        now(),
        "log",
    ))
    .unwrap();
    let d = db.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.quotas.len(), 1);
    assert_eq!(d.quotas[0].name, "Laptop");
    assert_eq!(d.quotas[0].source_id, "remote");
    assert_eq!(
        db.dashboard(&Filter {
            source_id: Some(a.id),
            ..Default::default()
        })
        .unwrap()
        .quotas
        .len(),
        1
    );
}

#[test]
fn separate_accounts_and_accountless_sources_keep_usage_separate() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let a = source("a");
    let b = Source {
        account_id: "work".into(),
        ..source("b")
    };
    let api1 = Source {
        account_id: String::new(),
        ..source("api1")
    };
    let api2 = Source {
        account_id: String::new(),
        ..source("api2")
    };
    let settings = Settings {
        sources: vec![a.clone(), b.clone(), api1.clone(), api2.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    for s in [&a, &b, &api1, &api2] {
        assert!(
            db.put_event(&event(s, "same-id", 100), &[], &settings)
                .unwrap()
        );
    }
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 480);
    assert_eq!(
        db.dashboard(&Filter {
            account_id: Some("work".into()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        120
    );
    let d = db
        .dashboard(&Filter {
            account_id: Some(String::new()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.summary.total, 240);
    assert!(d.quotas.is_empty());
}

#[test]
fn today_summary_keeps_seven_day_model_and_cache_breakdown() {
    use chrono::{Duration, Local, TimeZone};
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let s = source("s");
    let settings = Settings {
        sources: vec![s.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    let mut today = event(&s, "today", 100);
    today.tokens.cache_read = 100;
    let mut yesterday = event(&s, "yesterday", 200);
    yesterday.model = "model-b".into();
    yesterday.timestamp = Local
        .from_local_datetime(
            &(Local::now().date_naive() - Duration::days(1))
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
        .timestamp();
    db.put_event(&today, &[], &settings).unwrap();
    db.put_event(&yesterday, &[], &settings).unwrap();
    let d = db.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.days.len(), 1);
    assert_eq!(d.summary.total, 220);
    assert_eq!(d.trend_days.len(), 7);
    assert_eq!(d.day_models.len(), 2);
    assert_eq!(d.trend_days.iter().map(|d| d.total).sum::<u64>(), 440);
    assert_eq!(d.day_models.iter().map(|d| d.usage.total).sum::<u64>(), 440);
    assert_eq!(d.trend_days.last().unwrap().tokens.cache_read, 100);
    assert_eq!(d.pricing_gaps[0].unpriced_tokens, 220);
    assert_eq!(d.pricing_gaps[0].tokens.cache_read, 100);
}

#[test]
fn adding_missing_prices_fills_gaps_without_changing_frozen_rates() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let s = source("s");
    let settings = Settings {
        sources: vec![s.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    let p = ModelPrice {
        id: "model-a".into(),
        input: Some(0.01),
        output: Some(0.02),
        ..Default::default()
    };
    let mut e = event(&s, "one", 100);
    e.tokens.cache_read = 200;
    db.put_event(&e, &[p], &settings).unwrap();
    let gap = db
        .dashboard(&Filter::default())
        .unwrap()
        .pricing_gaps
        .remove(0);
    assert_eq!(gap.tokens.input, 0);
    assert_eq!(gap.tokens.cache_read, 200);
    db.save_prices(&[ModelPrice {
        id: "model-a".into(),
        input: Some(99.0),
        output: Some(99.0),
        cache_read: Some(0.001),
        ..Default::default()
    }])
    .unwrap();
    db.reprice(&settings, true).unwrap();
    let d = db.dashboard(&Filter::default()).unwrap();
    assert!(d.pricing_gaps.is_empty());
    assert_eq!(d.summary.priced_tokens, 320);
    assert!((d.summary.cost - 1.6).abs() < 1e-9);
}

#[cfg(unix)]
#[test]
fn quota_precommand_inherits_or_overrides_and_stops_on_failure() {
    use aieyes_core::{process, quota, ssh};
    use std::process::Command;
    use std::time::Duration;
    // Test the command composition without reading the host's login profiles.
    let temp = tempfile::tempdir().unwrap();
    let test_shell = temp.path().join("test-shell");
    std::fs::write(
        &test_shell,
        "#!/bin/sh\n[ \"$1\" = -lc ] || exit 2\nshift\nexec /bin/sh -c \"$@\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&test_shell, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Host {
        target: "example".into(),
        pre_command: "export TEST_PROXY=host".into(),
        shell: test_shell.to_string_lossy().into(),
        ..Default::default()
    };
    let mut s = source("remote");
    assert_eq!(quota::quota_host(&host, &s).pre_command, host.pre_command);
    s.quota_pre_command = "export TEST_PROXY=source\necho banner".into();
    let query_host = quota::quota_host(&host, &s);
    assert_eq!(host.pre_command, "export TEST_PROXY=host");
    let ssh = ssh::command(&query_host, "printf '%s' \"$TEST_PROXY\"").unwrap();
    let remote = ssh.get_args().last().unwrap().to_str().unwrap();
    let mut shell = Command::new("/bin/sh");
    shell.args(["-c", remote]);
    assert_eq!(
        process::run(shell, vec![], Duration::from_secs(3)).unwrap(),
        b"source"
    );
    s.quota_pre_command = "false".into();
    let ssh = ssh::command(&quota::quota_host(&host, &s), "printf should-not-run").unwrap();
    let mut shell = Command::new("/bin/sh");
    shell.args(["-c", ssh.get_args().last().unwrap().to_str().unwrap()]);
    assert!(process::run(shell, vec![], Duration::from_secs(3)).is_err());
}

#[test]
fn detaching_and_relinking_preserves_local_and_full_replay_history() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("db");
    let path = temp.path().join("usage.jsonl");
    let record = |id: &str, input| {
        json!({"id":id,"sessionId":"session","model":"model-a","timestamp":now(),"tokens":{"input":input,"output":20}}).to_string() + "\n"
    };
    std::fs::write(&path, record("old", 100)).unwrap();
    let mut src = Source {
        path: path.to_string_lossy().into(),
        ..source("s")
    };
    let db = Store::open(&root).unwrap();
    let mut settings = Settings {
        sources: vec![src.clone()],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    settings = db.settings().unwrap();
    db.save_prices(&[ModelPrice {
        id: "model-a".into(),
        input: Some(0.01),
        output: Some(0.01),
        ..Default::default()
    }])
    .unwrap();
    import::scan(&db, &settings, &src).unwrap();
    let cursor = db.cursor(&src.id, &src.path).unwrap();
    let cost = db.dashboard(&Filter::default()).unwrap().summary.cost;
    src.account_id.clear();
    settings.sources[0] = src.clone();
    db.save_settings(&settings).unwrap();
    assert_eq!(
        db.cursor(&src.id, &src.path).unwrap().map(|c| c.1),
        cursor.map(|c| c.1)
    );
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.cost, cost);
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap(),
        "{}",
        record("new", 50).trim()
    )
    .unwrap();
    import::scan(&db, &settings, &src).unwrap();
    // Full replays model SSH, and also exercise legacy events without stored raw identities.
    for (id, input) in [("old", 100), ("new", 50)] {
        db.put_event(&event(&src, id, input), &db.prices().unwrap(), &settings)
            .unwrap();
    }
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 190);
    assert_eq!(
        db.dashboard(&Filter {
            account_id: Some("account".into()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        120
    );
    assert_eq!(
        db.dashboard(&Filter {
            account_id: Some(String::new()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        70
    );
    src.account_id = "account".into();
    settings.sources[0] = src.clone();
    db.save_settings(&settings).unwrap();
    drop(db);
    let db = Store::open(&root).unwrap();
    for (id, input) in [("old", 100), ("new", 50)] {
        db.put_event(&event(&src, id, input), &db.prices().unwrap(), &settings)
            .unwrap();
    }
    db.db.execute("DELETE FROM files", []).unwrap();
    import::scan(&db, &settings, &src).unwrap();
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 190);
    assert_eq!(
        db.dashboard(&Filter {
            source_id: Some(src.id.clone()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        190
    );
    assert_eq!(
        db.dashboard(&Filter {
            account_id: Some(String::new()),
            ..Default::default()
        })
        .unwrap()
        .summary
        .total,
        70
    );
    let metadata = db.dashboard(&Filter::default()).unwrap().sources;
    let ids = metadata.iter().find(|row| row["id"] == src.id).unwrap()["accountIds"]
        .as_array()
        .unwrap();
    assert!(ids.contains(&json!("account")) && ids.contains(&json!("")));
    // An unrelated no-account source with the same event remains independent.
    let other = Source {
        account_id: String::new(),
        ..source("other")
    };
    db.put_event(&event(&other, "old", 100), &db.prices().unwrap(), &settings)
        .unwrap();
    assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 310);
}

#[test]
fn codex_account_toggle_keeps_cumulative_cursor_and_full_replay_identity() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(&temp.path().join("db")).unwrap();
    let path = temp.path().join("codex.jsonl");
    let records = vec![
        json!({"type":"session_meta","payload":{"id":"session"}}),
        json!({"type":"turn_context","payload":{"model":"codex-test"}}),
        json!({"type":"event_msg","timestamp":now()-1,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":30,"output_tokens":40}}}}),
        json!({"type":"event_msg","timestamp":now(),"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":50,"output_tokens":60}}}}),
    ];
    std::fs::write(
        &path,
        records[..3]
            .iter()
            .map(|record| record.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let mut src = Source {
        id: "codex-local".into(),
        provider: "codex".into(),
        account_id: "personal".into(),
        path: path.to_string_lossy().into(),
        ..Default::default()
    };
    let mut settings = Settings {
        sources: vec![src.clone()],
        accounts: vec![Account {
            id: "personal".into(),
            provider: "codex".into(),
            name: "Personal".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    import::scan(&db, &settings, &src).unwrap();
    src.account_id.clear();
    settings.sources[0] = src.clone();
    db.save_settings(&settings).unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap(),
        "{}",
        records[3]
    )
    .unwrap();
    import::scan(&db, &settings, &src).unwrap();
    for account_id in ["", "personal", "", "personal"] {
        src.account_id = account_id.into();
        settings.sources[0] = src.clone();
        db.save_settings(&settings).unwrap();
        let mut state = ParseState::default();
        for record in &records {
            if let Some(event) = usage::parse(&src, &mut state, record) {
                db.put_event(&event, &[], &settings).unwrap();
            }
        }
        assert_eq!(db.dashboard(&Filter::default()).unwrap().summary.total, 220);
        assert_eq!(
            db.dashboard(&Filter {
                account_id: Some("personal".into()),
                ..Default::default()
            })
            .unwrap()
            .summary
            .total,
            140
        );
        assert_eq!(
            db.dashboard(&Filter {
                account_id: Some(String::new()),
                ..Default::default()
            })
            .unwrap()
            .summary
            .total,
            80
        );
    }
}

#[test]
fn proxy_addresses_validate_without_breaking_legacy_default_ports() {
    let temp = tempfile::tempdir().unwrap();
    let db = Store::open(temp.path()).unwrap();
    let mut settings = Settings::default();
    settings.proxy.mode = "custom".into();
    for url in [
        "http://127.0.0.1:7890",
        "https://proxy.example",
        "socks5://localhost",
        "socks5h://[::1]:1080",
    ] {
        settings.proxy.url = url.into();
        db.save_settings(&settings).unwrap();
    }
    for url in [
        "http://127.0.0.1:0",
        "socks5:///path",
        "ftp://proxy.example",
        "http://user:password@localhost:7890",
    ] {
        settings.proxy.url = url.into();
        assert!(db.save_settings(&settings).is_err());
        assert_eq!(db.settings().unwrap().proxy.url, "socks5h://[::1]:1080");
    }
}

#[cfg(unix)]
#[test]
fn explicit_posix_shell_avoids_login_profiles_and_keeps_tools_reachable() {
    use aieyes_core::{process, ssh};
    use std::process::Command;
    use std::time::Duration;
    let host = Host {
        target: "example".into(),
        shell: "/bin/sh".into(),
        ..Default::default()
    };
    assert_eq!(host.shell, "/bin/sh");
    let ssh = ssh::command(&host, "printf '%s' \"$PATH\"").unwrap();
    let remote = ssh.get_args().last().unwrap().to_str().unwrap();
    // A login /bin/sh (dash) exits with status 2 when a profile script contains
    // bash syntax, so explicitly configured POSIX shells must not use login mode.
    assert!(remote.contains("'/bin/sh' -c "), "{remote}");
    assert!(!remote.contains("-lc"), "{remote}");
    assert!(remote.contains("$HOME/.local/bin"), "{remote}");
    // The composed string must survive a POSIX shell parsing it, like sshd does.
    let mut outer = Command::new("/bin/sh");
    outer.args(["-c", remote]);
    let path =
        String::from_utf8(process::run(outer, vec![], Duration::from_secs(3)).unwrap()).unwrap();
    assert!(path.contains(".local/bin"), "{path}");
    // New hosts default to Bash and retain its login semantics.
    let host = Host {
        target: "example".into(),
        ..Default::default()
    };
    assert_eq!(host.shell, "/bin/bash");
    let ssh = ssh::command(&host, "true").unwrap();
    let remote = ssh.get_args().last().unwrap().to_str().unwrap();
    assert!(remote.contains("'/bin/bash' -lc "), "{remote}");
    assert!(!remote.contains("$HOME/.local/bin"), "{remote}");
}
