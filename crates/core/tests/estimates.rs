use aieyes_core::{estimates, models::*, store::Store, usage};
use serde_json::json;

fn setup() -> (tempfile::TempDir, Store, Source, QuotaSnapshot) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let source = Source {
        id: "source".into(),
        name: "本机".into(),
        provider: "custom".into(),
        account_id: "account".into(),
        path: "/data".into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: "account".into(),
                name: "账户".into(),
                provider: "custom".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    store
        .save_prices(&[ModelPrice {
            id: "model".into(),
            input: Some(0.01),
            output: Some(0.02),
            cache_read: Some(0.005),
            cache_write: Some(0.01),
            ..Default::default()
        }])
        .unwrap();
    let quota = QuotaSnapshot {
        provider: "custom".into(),
        account_id: "account".into(),
        source_id: "source".into(),
        origin: "live".into(),
        updated_at: now() - 100,
        windows: vec![QuotaWindow {
            id: "weekly".into(),
            name: "7d".into(),
            window_minutes: Some(10080),
            used_percent: 20.0,
            resets_at: Some(now() + 10000),
            ..Default::default()
        }],
        ..Default::default()
    };
    (dir, store, source, quota)
}
fn add(store: &Store, source: &Source, id: &str, stamp: i64, billing: &str) {
    let e=usage::parse(source,&mut ParseState::default(),&json!({"id":id,"timestamp":stamp,"model":"model","tokens":{"input":100},"billing":{"category":billing}})).unwrap();
    store
        .put_event(&e, &store.prices().unwrap(), &store.settings().unwrap())
        .unwrap();
}
#[test]
fn valuation_excludes_api_deduplicates_sources_and_freezes_prices() {
    let (_dir, store, source, mut q) = setup();
    let mut settings = store.settings().unwrap();
    let mut copy = source.clone();
    copy.id = "copy".into();
    settings.sources.push(copy.clone());
    store.save_settings(&settings).unwrap();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id.clone(), copy.id.clone()], None)
        .unwrap();
    add(&store, &source, "usage", q.updated_at + 10, "subscription");
    add(&store, &copy, "usage", q.updated_at + 10, "subscription");
    add(&store, &source, "api", q.updated_at + 20, "api");
    add(&store, &copy, "api", q.updated_at + 20, "unknown");
    add(&store, &source, "before", q.updated_at - 1, "subscription");
    q.updated_at += 50;
    q.windows[0].used_percent = 30.0;
    store.quota(&q).unwrap();
    let done = store.finish_estimate(&e.id).unwrap();
    assert_eq!(done.weekly_value, Some(10.0));
    assert_eq!(done.total_tokens, 100);
    assert_eq!(done.excluded_api_tokens, 100);
    store
        .save_prices(&[ModelPrice {
            id: "model".into(),
            input: Some(8.0),
            ..Default::default()
        }])
        .unwrap();
    store.reprice(&settings, false).unwrap();
    assert_eq!(
        store.finish_estimate(&e.id).unwrap().weekly_value,
        Some(10.0)
    );
    assert_eq!(done.prices.len(), 1);
}
#[test]
fn reset_pauses_at_last_valid_checkpoint_and_requires_explicit_restart() {
    let (dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id.clone()], None)
        .unwrap();
    add(&store, &source, "good", q.updated_at + 1, "unknown");
    q.updated_at += 20;
    q.windows[0].used_percent = 30.0;
    store.quota(&q).unwrap();
    let good = q.updated_at;
    q.updated_at += 20;
    q.windows[0].used_percent = 1.0;
    q.windows[0].resets_at = Some(now() + 20000);
    store.quota(&q).unwrap();
    let pending = store.estimate(&e.id).unwrap();
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.checkpoint_at, good);
    assert_eq!(pending.weekly_value, Some(10.0));
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().status, "pending");
    assert!(
        store
            .begin_estimate(&q, "weekly", vec![source.id.clone()], None)
            .is_err()
    );
    let next = store
        .begin_estimate(&q, "weekly", vec![source.id], Some(&e.id))
        .unwrap();
    assert_ne!(next.id, e.id);
    assert_eq!(next.baseline_percent, 1.0);
    assert_eq!(store.estimate(&e.id).unwrap().status, "completed");
}
#[test]
fn low_sample_missing_price_and_ambiguous_boundary_never_produce_a_number() {
    let (_dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id.clone()], None)
        .unwrap();
    add(&store, &source, "one", q.updated_at + 1, "unknown");
    q.updated_at += 10;
    q.windows[0].used_percent = 22.0;
    store.quota(&q).unwrap();
    assert!(store.estimate(&e.id).unwrap().weekly_value.is_none());
    let mut event = usage::parse(
        &source,
        &mut ParseState::default(),
        &json!({"id":"missing","timestamp":q.updated_at+1,"model":"unpriced","tokens":{"input":1}}),
    )
    .unwrap();
    store
        .put_event(&event, &[], &store.settings().unwrap())
        .unwrap();
    q.updated_at += 10;
    q.windows[0].used_percent = 40.0;
    store.quota(&q).unwrap();
    assert!(
        store
            .estimate(&e.id)
            .unwrap()
            .calculation_note
            .contains("价格")
    );
    event.id = "boundary".into();
    event.attribution = "session-summary".into();
    event.interval_start = None;
    store
        .put_event(&event, &[], &store.settings().unwrap())
        .unwrap();
    q.updated_at += 1;
    store.quota(&q).unwrap();
    assert!(
        store
            .estimate(&e.id)
            .unwrap()
            .calculation_note
            .contains("边界")
    );
}
#[test]
fn log_snapshots_and_older_live_snapshots_cannot_reset_or_advance_sampling() {
    let (_dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id], None)
        .unwrap();
    q.origin = "log".into();
    q.updated_at += 10;
    q.windows[0].used_percent = 0.0;
    store.quota(&q).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().checkpoint_at, e.started_at);
    q.origin = "live".into();
    q.updated_at = e.started_at - 1;
    store.observe_estimates(&q).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().status, "active");
}
#[test]
fn read_failure_retains_previous_checkpoint_and_source_changes_are_not_silent() {
    let (_dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id.clone()], None)
        .unwrap();
    q.error = Some("offline".into());
    store.quota(&q).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().status, "pending");
    assert_eq!(store.estimate(&e.id).unwrap().checkpoint_at, e.started_at);
    q.error = None;
    let next = store
        .begin_estimate(&q, "weekly", vec![source.id], Some(&e.id))
        .unwrap();
    let mut settings = store.settings().unwrap();
    settings.sources[0].path = "/other".into();
    store.save_settings(&settings).unwrap();
    q.updated_at += 10;
    store.quota(&q).unwrap();
    assert_eq!(store.estimate(&next.id).unwrap().status, "pending");
}
#[test]
fn quota_order_survives_drafts_filtering_new_accounts_and_reopen() {
    let (dir, store, source, _q) = setup();
    let mut settings = store.settings().unwrap();
    settings.accounts.push(Account {
        id: "second".into(),
        name: "A".into(),
        provider: source.provider.clone(),
        ..Default::default()
    });
    store.save_settings(&settings).unwrap();
    store
        .set_quota_order(vec!["custom:account".into(), "custom:second".into()])
        .unwrap();
    // A settings draft saved later does not contain/overwrite the independent order.
    settings.refresh_seconds = 123;
    store.save_settings(&settings).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.quota_order().unwrap(),
        vec!["custom:account", "custom:second"]
    );
    settings.accounts.push(Account {
        id: "new".into(),
        name: "0".into(),
        provider: "custom".into(),
        ..Default::default()
    });
    store.save_settings(&settings).unwrap();
    assert_eq!(store.quota_order().unwrap().last().unwrap(), "custom:new");
    assert!(
        store
            .set_quota_order(vec!["custom:account".into(), "custom:account".into()])
            .is_err()
    );
}
#[test]
fn overlapping_pools_require_model_mapping_and_old_event_payloads_remain_unknown() {
    let (_dir, _store, source, mut q) = setup();
    q.windows.push(QuotaWindow {
        id: "other".into(),
        ..q.windows[0].clone()
    });
    assert!(!estimates::eligible(&q, &q.windows[0]));
    let event = usage::parse(
        &source,
        &mut ParseState::default(),
        &json!({"id":"old","timestamp":now(),"model":"model","tokens":{"input":1}}),
    )
    .unwrap();
    let mut value = serde_json::to_value(&event).unwrap();
    value.as_object_mut().unwrap().remove("billing");
    value.as_object_mut().unwrap().remove("intervalStart");
    let old: UsageEvent = serde_json::from_value(value).unwrap();
    assert!(old.billing.category.is_empty());
    assert!(old.interval_start.is_none());
}
#[test]
fn new_codex_session_has_a_known_start_but_current_provider_is_only_evidence() {
    let source = Source {
        provider: "codex".into(),
        ..Default::default()
    };
    let mut state = ParseState::default();
    let start = now();
    usage::parse(
        &source,
        &mut state,
        &json!({"type":"session_meta","timestamp":start,"payload":{"id":"new","model_provider":"openai"}}),
    );
    let e=usage::parse(&source,&mut state,&json!({"type":"event_msg","timestamp":start+10,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100}}}})).unwrap();
    assert_eq!(e.interval_start, Some(start));
    assert_eq!(e.billing.category, "unknown");
    assert_eq!(e.billing.evidence, "openai");
}

#[test]
fn pending_record_can_end_after_source_removal_without_recomputing_frozen_segment() {
    let (dir, store, source, mut q) = setup();
    let e = store
        .begin_estimate(&q, "weekly", vec![source.id.clone()], None)
        .unwrap();
    add(&store, &source, "good", q.updated_at + 1, "unknown");
    q.updated_at += 10;
    q.windows[0].used_percent = 30.0;
    store.quota(&q).unwrap();
    let mut settings = store.settings().unwrap();
    settings.sources.clear();
    store.save_settings(&settings).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().status, "pending");
    drop(store);
    let mut engine = aieyes_core::Engine::open(dir.path()).unwrap();
    let done = engine
        .call("quotaEstimates.stop", json!({"id":e.id}))
        .unwrap();
    assert_eq!(done["status"], "completed");
    assert_eq!(done["weeklyValue"], 10.0);
    assert!(
        engine
            .call(
                "quotaEstimates.start",
                json!({"accountKey":"custom:account"})
            )
            .unwrap_err()
            .to_string()
            .contains("确认")
    );
}
