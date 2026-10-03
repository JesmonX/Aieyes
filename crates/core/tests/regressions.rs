use aieyes_core::{import, models::*, store::Store, usage};
use serde_json::json;

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
fn changing_account_reimports_without_double_counting() {
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
    source.account_id = "new-account".into();
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
        100
    );
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
    let host = Host {
        target: "example".into(),
        pre_command: "export TEST_PROXY=host".into(),
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
