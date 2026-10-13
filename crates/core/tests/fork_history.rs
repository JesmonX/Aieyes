use aieyes_core::{import, models::*, store::Store, usage};
use serde_json::{Value, json};
use std::path::Path;

fn line(v: Value) -> String {
    format!("{v}\n")
}
fn meta(id: &str, stamp: i64, base: Option<(&str, usize)>) -> String {
    let mut payload = json!({"id":id,"timestamp":stamp,"model_provider":"openai"});
    if let Some((parent, limit)) = base {
        payload["history_base"] = json!({"thread_id":parent,"end_byte_offset":limit});
    }
    line(json!({"type":"session_meta","timestamp":stamp,"payload":payload}))
}
fn count(stamp: i64, total: u64, last: u64) -> String {
    line(
        json!({"type":"event_msg","timestamp":stamp,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":total,"output_tokens":0},"last_token_usage":{"input_tokens":last,"output_tokens":0}}}}),
    )
}
fn setup(path: &Path) -> (Store, Settings, Source) {
    let source = Source {
        id: "native".into(),
        provider: "codex".into(),
        path: path.to_string_lossy().into(),
        ..Default::default()
    };
    let settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    let store = Store::open(&path.join("db")).unwrap();
    store.save_settings(&settings).unwrap();
    (store, settings, source)
}
fn usage_for(store: &Store, session: &str) -> u64 {
    let mut query = store
        .db
        .prepare("SELECT payload FROM events WHERE json_extract(payload,'$.sessionId')=?1")
        .unwrap();
    query
        .query_map([session], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| {
            serde_json::from_str::<UsageEvent>(&r.unwrap())
                .unwrap()
                .tokens
                .total()
        })
        .sum()
}
fn legacy(source: &Source, session: &str, stamp: i64, total: u64) -> UsageEvent {
    let tokens = Tokens {
        input: total,
        ..Default::default()
    };
    let identity = format!(
        "{session}:0:{stamp}:{}",
        serde_json::to_string(&tokens).unwrap()
    );
    serde_json::from_value(json!({"id":usage::event_id("codex",&source.id,&source.account_id,&identity),"importIdentity":identity,"sourceId":source.id,"accountId":source.account_id,"provider":"codex","sessionId":session,"model":"unknown","timestamp":stamp,"tokens":tokens,"attribution":"session-summary"})).unwrap()
}

#[test]
fn nested_fork_zero_is_repaired_and_old_parser_cannot_resurrect_it() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 5_910_300, 5_910_300);
    let middle = meta("middle", stamp - 20, Some(("parent", parent.len())));
    let child =
        meta("child", stamp - 10, Some(("middle", middle.len()))) + &count(stamp, 5_910_300, 1_000);
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    std::fs::write(temp.path().join("middle.jsonl"), middle).unwrap();
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let (store, settings, source) = setup(temp.path());
    let old = legacy(&source, "child", stamp, 5_910_300);
    store.put_event(&old, &[], &settings).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 0);
    assert_eq!(usage_for(&store, "parent"), 5_910_300);
    store.put_event(&old, &[], &settings).unwrap();
    import::rescan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 0);
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.events,
        1
    );
    let original: String = store
        .db
        .query_row(
            "SELECT original_payload FROM usage_corrections WHERE event_id=?1",
            [&old.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<UsageEvent>(&original)
            .unwrap()
            .tokens
            .total(),
        5_910_300
    );
    let backup: String = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='history-correction-backup-v5'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Path::new(&backup).is_file());
}

#[test]
fn fork_uses_bounded_parent_prefix_and_preserves_frozen_price() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let prefix = meta("parent", stamp - 100, None) + &count(stamp - 90, 2_358_667, 2_358_667);
    let child = meta("child", stamp - 10, Some(("parent", prefix.len())))
        + &count(stamp, 2_518_780, 160_113);
    std::fs::write(
        temp.path().join("parent.jsonl"),
        prefix + &count(stamp + 1, 9_000_000, 6_641_333),
    )
    .unwrap();
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let (store, settings, source) = setup(temp.path());
    let old = legacy(&source, "child", stamp, 2_518_780);
    let price = ModelPrice {
        id: "unknown".into(),
        input: Some(0.000001),
        ..Default::default()
    };
    store.put_event(&old, &[price], &settings).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 160_113);
    let (cost, price): (f64, String) = store
        .db
        .query_row(
            "SELECT cost,price FROM events WHERE id=?1",
            [&old.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((cost - 0.160113).abs() < 1e-9);
    assert_eq!(
        serde_json::from_str::<ModelPrice>(&price).unwrap().input,
        Some(0.000001)
    );
    store.put_event(&old, &[], &settings).unwrap();
    assert_eq!(usage_for(&store, "child"), 160_113);
}

#[test]
fn missing_parent_excludes_only_uncertain_prefix_and_later_arrival_repairs_it() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 900, 900);
    let child = meta("child", stamp - 10, Some(("parent", parent.len())))
        + &count(stamp, 1000, 100)
        + &count(stamp + 1, 1100, 100);
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let (store, settings, source) = setup(temp.path());
    let result = import::scan(&store, &settings, &source).unwrap();
    assert_eq!(result["missingHistoryBases"], 1);
    assert_eq!(usage_for(&store, "child"), 100);
    assert_eq!(
        import::scan(&store, &settings, &source).unwrap()["readBytes"],
        0
    );
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    let result = import::scan(&store, &settings, &source).unwrap();
    assert!(result.get("missingHistoryBases").is_none());
    assert_eq!(usage_for(&store, "child"), 200);
    import::rescan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
}

#[test]
fn partial_parent_boundary_does_not_guess_a_baseline() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 900, 900);
    let child =
        meta("child", stamp - 10, Some(("parent", parent.len() - 1))) + &count(stamp, 1000, 100);
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let (store, settings, source) = setup(temp.path());
    assert_eq!(
        import::scan(&store, &settings, &source).unwrap()["missingHistoryBases"],
        1
    );
    assert_eq!(usage_for(&store, "child"), 0);
}

#[test]
fn historical_estimate_is_repaired_with_original_prices_and_audit_after_parent_arrives() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 900, 900);
    let mut billed: Value = serde_json::from_str(count(stamp, 1000, 100).trim()).unwrap();
    billed["payload"]["rate_limits"] = json!({"plan_type":"plus","primary":{"used_percent":20}});
    let child = meta("child", stamp - 10, Some(("parent", parent.len())))
        + &line(
            json!({"type":"event_msg","timestamp":stamp-9,"payload":{"type":"task_started","turn_id":"turn"}}),
        )
        + &line(billed);
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let source = Source {
        id: "source".into(),
        provider: "codex".into(),
        account_id: "account".into(),
        path: temp.path().to_string_lossy().into(),
        ..Default::default()
    };
    let mut copy = source.clone();
    copy.id = "copy".into();
    let settings = Settings {
        sources: vec![source.clone(), copy.clone()],
        accounts: vec![Account {
            id: "account".into(),
            name: "Fixture".into(),
            provider: "codex".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let store = Store::open(&temp.path().join("db")).unwrap();
    store.save_settings(&settings).unwrap();
    let price = ModelPrice {
        id: "unknown".into(),
        input: Some(0.01),
        ..Default::default()
    };
    store.save_prices(std::slice::from_ref(&price)).unwrap();
    let mut quota = QuotaSnapshot {
        provider: "codex".into(),
        account_id: "account".into(),
        source_id: source.id.clone(),
        origin: "live".into(),
        updated_at: stamp - 20,
        windows: vec![QuotaWindow {
            id: "weekly".into(),
            window_minutes: Some(10080),
            resets_at: Some(stamp + 10000),
            ..Default::default()
        }],
        ..Default::default()
    };
    let estimate = store
        .begin_estimate(
            &quota,
            "weekly",
            vec![source.id.clone(), copy.id.clone()],
            None,
        )
        .unwrap();
    let mut old = legacy(&source, "child", stamp, 1000);
    old.interval_start = Some(stamp - 10);
    old.billing = BillingEvidence {
        category: "subscription".into(),
        evidence: "codex-response-rate-limits".into(),
    };
    store
        .put_event(&old, std::slice::from_ref(&price), &settings)
        .unwrap();
    let mut duplicate = old.clone();
    duplicate.source_id = copy.id.clone();
    store.put_event(&duplicate, &[price], &settings).unwrap();
    quota.updated_at = stamp + 1;
    quota.windows[0].used_percent = 20.0;
    store.quota(&quota).unwrap();
    let original = store.finish_estimate_for(&estimate.id, false).unwrap();
    assert_eq!(original.quota().unwrap().weekly_value, Some(50.0));
    store
        .save_prices(&[ModelPrice {
            id: "unknown".into(),
            input: Some(9.0),
            ..Default::default()
        }])
        .unwrap();
    import::scan(&store, &settings, &source).unwrap();
    let invalid = store
        .estimate_for(&format!("{}-fork-v5", original.id), false)
        .unwrap();
    assert!(!invalid.has_value());
    assert_eq!(invalid.calculation_status, "unknownBilling");
    assert_eq!(invalid.prices, original.prices);
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    let repaired = store
        .estimate_for(&format!("{}-fork-v5", invalid.id), false)
        .unwrap();
    assert_eq!(repaired.quota().unwrap().weekly_value, Some(5.0));
    assert_eq!(repaired.prices, original.prices);
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM event_sources WHERE event_id=?1",
                [&old.id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
    let audit: String = store
        .db
        .query_row(
            "SELECT original_payload FROM estimate_correction_audit WHERE id=?1",
            [&original.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&audit).unwrap()["weeklyValue"],
        50.0
    );
    store.put_event(&old, &[], &settings).unwrap();
    import::rescan(&store, &settings, &source).unwrap();
    assert_eq!(store.estimates().unwrap().len(), 3);
    assert_eq!(usage_for(&store, "child"), 100);
}

#[test]
fn archived_parent_prefix_changes_are_rechecked_from_a_day_source() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let sessions = temp.path().join("sessions/2026/10/13");
    let archive = temp.path().join("archived_sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&archive).unwrap();
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 900, 900);
    std::fs::write(archive.join("parent.jsonl"), &parent).unwrap();
    std::fs::write(
        sessions.join("child.jsonl"),
        meta("child", stamp - 10, Some(("parent", parent.len()))) + &count(stamp, 1000, 100),
    )
    .unwrap();
    let (store, settings, source) = setup(&sessions);
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 100);
    std::fs::write(archive.join("parent.jsonl"), parent.replace("900", "800")).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
    std::fs::remove_file(archive.join("parent.jsonl")).unwrap();
    assert_eq!(
        import::scan(&store, &settings, &source).unwrap()["missingHistoryBases"],
        1
    );
    assert_eq!(usage_for(&store, "child"), 0);
}

#[test]
fn cyclic_parents_never_charge_the_first_cumulative_counter() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    // A boundary of zero is a valid empty prefix, so use fixed-width real bounds.
    let mut left = meta("left", stamp - 20, Some(("right", 999)));
    let mut right = meta("right", stamp - 20, Some(("left", 999)));
    for _ in 0..3 {
        left = meta("left", stamp - 20, Some(("right", right.len())));
        right = meta("right", stamp - 20, Some(("left", left.len())));
    }
    std::fs::write(temp.path().join("left.jsonl"), &left).unwrap();
    std::fs::write(temp.path().join("right.jsonl"), right).unwrap();
    std::fs::write(
        temp.path().join("child.jsonl"),
        meta("child", stamp - 10, Some(("left", left.len())))
            + &count(stamp, 1000, 100)
            + &count(stamp + 1, 1100, 100),
    )
    .unwrap();
    let (store, settings, source) = setup(temp.path());
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 100);
}
