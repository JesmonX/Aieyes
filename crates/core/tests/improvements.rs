use aieyes_core::{models::*, store::Store, usage};
use serde_json::json;

fn setup() -> (tempfile::TempDir, Store, Source, QuotaSnapshot) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let source = Source {
        id: "source".into(),
        name: "test".into(),
        provider: "custom".into(),
        account_id: "account".into(),
        path: "/test".into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    store
        .save_prices(&[ModelPrice {
            id: "model".into(),
            input: Some(0.01),
            ..Default::default()
        }])
        .unwrap();
    let q = QuotaSnapshot {
        provider: "custom".into(),
        account_id: "account".into(),
        source_id: source.id.clone(),
        origin: "live".into(),
        updated_at: now() - 100,
        windows: vec![
            QuotaWindow {
                id: "short".into(),
                name: "5h".into(),
                window_minutes: Some(300),
                used_percent: 20.0,
                resets_at: Some(now() - 50),
                ..Default::default()
            },
            QuotaWindow {
                id: "week".into(),
                name: "7d".into(),
                window_minutes: Some(10080),
                used_percent: 20.0,
                resets_at: Some(now() + 86400),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (directory, store, source, q)
}
fn add(store: &Store, source: &Source, id: &str, stamp: i64, input: u64, model: &str) {
    let event = usage::parse(
        source,
        &mut ParseState::default(),
        &json!({"id":id,"timestamp":stamp,"model":model,"tokens":{"input":input}}),
    )
    .unwrap();
    store
        .put_event(&event, &store.prices().unwrap(), &store.settings().unwrap())
        .unwrap();
}
#[test]
fn ordinary_settings_skip_history_and_mapping_saves_only_fill_affected_models() {
    let (_dir, store, source, _) = setup();
    let mut settings = store.settings().unwrap();
    for (id, model) in [("direct", "model"), ("mapped", "alias")] {
        let event = usage::parse(
            &source,
            &mut ParseState::default(),
            &json!({"id":id,"timestamp":now(),"model":model,"tokens":{"input":100}}),
        )
        .unwrap();
        store.put_event(&event, &[], &settings).unwrap();
    }
    store.db.execute_batch("CREATE TRIGGER reject_history_write BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT,'ordinary save must not touch history'); END;").unwrap();
    settings.refresh_seconds = 600;
    store.save_settings(&settings).unwrap();
    let unpriced: u64 = store
        .db
        .query_row(
            "SELECT COUNT(*) FROM events WHERE priced_tokens=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unpriced, 2);
    store.db.execute_batch("DROP TRIGGER reject_history_write; CREATE TRIGGER reject_unaffected_model BEFORE UPDATE ON events WHEN NEW.model != 'alias' BEGIN SELECT RAISE(ABORT,'mapping save must only touch the changed model'); END;").unwrap();
    settings
        .model_mappings
        .insert("alias".into(), "model".into());
    store.save_settings(&settings).unwrap();
    let priced: u64 = store
        .db
        .query_row(
            "SELECT priced_tokens FROM events WHERE model='alias'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(priced, 100);
    let unchanged: u64 = store
        .db
        .query_row(
            "SELECT priced_tokens FROM events WHERE model='model'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, 0);
    store
        .db
        .execute_batch("DROP TRIGGER reject_unaffected_model;")
        .unwrap();
    store.reprice(&settings, false).unwrap();
    let full: u64 = store
        .db
        .query_row(
            "SELECT COUNT(*) FROM events WHERE priced_tokens=100",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        full, 2,
        "explicit recalculation must still cover all history"
    );
}
#[test]
fn five_hour_sampling_resumes_normal_resets_and_preserves_three_values() {
    let (dir, store, source, mut q) = setup();
    store.quota(&q).unwrap();
    let estimate = store
        .begin_estimate(&q, "short", vec![source.id.clone()], None)
        .unwrap();
    add(&store, &source, "one", q.updated_at + 10, 100, "model");
    q.updated_at += 20;
    q.windows[0].used_percent = 30.0;
    q.windows[1].used_percent = 22.0;
    store.quota(&q).unwrap();
    let e = store.estimate(&estimate.id).unwrap();
    assert_eq!(e.five_hour_value, Some(10.0));
    assert_eq!(e.weekly_direct_value, Some(50.0));
    assert_eq!(e.weekly_ratio_value, Some(50.0));
    add(
        &store,
        &source,
        "unobserved-gap",
        q.updated_at + 10,
        1000,
        "model",
    );
    q.updated_at += 40;
    q.windows[0].used_percent = 0.0;
    q.windows[0].resets_at = Some(now() + 17900);
    store.quota(&q).unwrap();
    let e = store.estimate(&estimate.id).unwrap();
    assert_eq!(e.status, "active");
    assert_eq!(e.segments.len(), 1);
    assert_eq!(e.total_tokens, 100);
    add(&store, &source, "two", q.updated_at + 10, 200, "model");
    q.updated_at += 20;
    q.windows[0].used_percent = 10.0;
    q.windows[1].used_percent = 24.0;
    store.quota(&q).unwrap();
    let done = store.finish_estimate(&estimate.id).unwrap();
    assert_eq!(done.total_tokens, 300);
    assert_eq!(done.five_hour_value, Some(15.0));
    assert_eq!(done.weekly_direct_value, Some(75.0));
    assert_eq!(done.weekly_ratio_value, Some(75.0));
    drop(store);
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .finish_estimate(&done.id)
            .unwrap()
            .weekly_ratio_value,
        Some(75.0)
    );
}
#[test]
fn early_resets_and_unpriced_usage_do_not_produce_a_five_hour_value() {
    let (_dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "short", vec![source.id.clone()], None)
        .unwrap();
    add(&store, &source, "one", q.updated_at + 1, 100, "missing");
    q.updated_at += 10;
    q.windows[0].used_percent = 30.0;
    store.quota(&q).unwrap();
    assert!(store.estimate(&e.id).unwrap().five_hour_value.is_none());
    q.updated_at += 10;
    q.windows[0].resets_at = Some(now() + 18000);
    q.windows[0].used_percent = 0.0;
    store.quota(&q).unwrap();
    let pending = store.estimate(&e.id).unwrap();
    assert_eq!(pending.status, "pending");
    assert!(pending.reason.contains("提前"));
}
#[test]
fn calibration_favors_recent_samples_and_ignores_duplicate_and_reset_observations() {
    let (_dir, store, _, mut q) = setup();
    store.quota(&q).unwrap();
    q.updated_at += 10;
    q.windows[0].used_percent += 10.0;
    q.windows[1].used_percent += 2.0;
    store.quota(&q).unwrap();
    store.quota(&q).unwrap();
    assert_eq!(store.capacity("custom:account", now()).unwrap().samples, 1);
    q.updated_at += 10;
    q.windows[0].used_percent = 0.0;
    q.windows[0].resets_at = Some(now() + 18000);
    store.quota(&q).unwrap();
    assert_eq!(store.capacity("custom:account", now()).unwrap().samples, 1);
    store
        .db
        .execute("DELETE FROM capacity_samples", [])
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO capacity_samples VALUES('custom:account',?1,100,10)",
            [now() - 28 * 86400],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO capacity_samples VALUES('custom:account',?1,50,10)",
            [now()],
        )
        .unwrap();
    let result = store.capacity("custom:account", now()).unwrap();
    assert!(result.ratio.unwrap() > 5.0 && result.ratio.unwrap() < 5.4);
}
#[test]
fn recalculation_clears_unmatched_snapshots_and_resync_cannot_restore_them() {
    let (_dir, store, source, _) = setup();
    let mut settings = store.settings().unwrap();
    settings
        .model_mappings
        .insert("alias".into(), "model".into());
    store.save_settings(&settings).unwrap();
    add(&store, &source, "one", now(), 100, "alias");
    assert_eq!(
        store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .priced_tokens,
        100
    );
    settings.model_mappings.clear();
    store.save_settings(&settings).unwrap();
    store.reprice(&settings, false).unwrap();
    add(&store, &source, "one", now(), 100, "alias");
    let d = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.summary.priced_tokens, 0);
    assert_eq!(d.pricing_gaps[0].model, "alias");
    add(&store, &source, "exact", now(), 100, "model");
    store.reprice(&settings, false).unwrap();
    assert_eq!(
        store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .priced_tokens,
        100
    );
}

#[test]
fn manual_boundaries_report_progress_without_duplicate_quota_refresh_scans() {
    use std::sync::{Arc, Mutex};
    let directory = tempfile::tempdir().unwrap();
    let logs = directory.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    let mut engine = aieyes_core::Engine::open(directory.path()).unwrap();
    let windows = json!({"windows":[{"id":"short","name":"5h","windowMinutes":300,"usedPercent":20,"resetsAt":now()+18000}]});
    engine
        .store
        .save_settings(&Settings {
            version: 2,
            sources: vec![Source {
                id: "source".into(),
                name: "fixture".into(),
                provider: "custom".into(),
                account_id: "account".into(),
                path: logs.to_string_lossy().into(),
                quota_command: format!("echo '{windows}'"),
                ..Default::default()
            }],
            accounts: vec![Account {
                id: "account".into(),
                provider: "custom".into(),
                name: "fixture".into(),
                quota_enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let stages = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = stages.clone();
    let started = engine.call_with_progress("quotaEstimates.start", json!({"accountKey":"custom:account","windowId":"short","sourceIds":["source"],"confirmed":true}), Box::new(move |p| observed.lock().unwrap().push(p["stage"].as_str().unwrap().into()))).unwrap();
    assert_eq!(
        *stages.lock().unwrap(),
        vec!["同步来源 · fixture", "读取实时限额", "保存采样起点"]
    );
    stages.lock().unwrap().clear();
    let observed = stages.clone();
    let done = engine
        .call_with_progress(
            "quotaEstimates.stop",
            json!({"id":started["id"]}),
            Box::new(move |p| {
                observed
                    .lock()
                    .unwrap()
                    .push(p["stage"].as_str().unwrap().into())
            }),
        )
        .unwrap();
    assert_eq!(done["status"], "completed");
    assert_eq!(
        *stages.lock().unwrap(),
        vec![
            "同步来源 · fixture",
            "读取实时限额",
            "补齐尾部记录",
            "同步来源 · fixture",
            "保存采样结果"
        ]
    );
}

#[test]
fn changing_subscription_plan_discards_old_capacity_and_pauses_the_sample() {
    let (_directory, store, source, mut q) = setup();
    q.plan = Some("first".into());
    store.quota(&q).unwrap();
    let sample = store
        .begin_estimate(&q, "short", vec![source.id.clone()], None)
        .unwrap();
    q.updated_at += 10;
    q.windows[0].used_percent += 10.0;
    q.windows[1].used_percent += 2.0;
    store.quota(&q).unwrap();
    assert_eq!(store.capacity("custom:account", now()).unwrap().samples, 1);
    q.updated_at += 10;
    q.plan = Some("second".into());
    store.quota(&q).unwrap();
    assert!(
        store
            .capacity("custom:account", now())
            .unwrap()
            .ratio
            .is_none()
    );
    let pending = store.estimate(&sample.id).unwrap();
    assert_eq!(pending.status, "pending");
    assert!(pending.reason.contains("订阅方案"));
}
