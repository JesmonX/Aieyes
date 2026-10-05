use aieyes_core::{estimates::credit_units, models::*, pricing, store::Store, usage};
use serde_json::json;
fn setup() -> (tempfile::TempDir, Store, QuotaSnapshot) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .save_settings(&Settings {
            sources: vec![Source {
                id: "s".into(),
                provider: "codex".into(),
                account_id: "a".into(),
                path: "/fixtures".into(),
                ..Default::default()
            }],
            accounts: vec![Account {
                id: "a".into(),
                name: "Test".into(),
                provider: "codex".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let q = QuotaSnapshot {
        provider: "codex".into(),
        account_id: "a".into(),
        source_id: "s".into(),
        origin: "live".into(),
        updated_at: now() - 100,
        credits: Some(CreditsSnapshot {
            has_credits: true,
            balance: Some("1000.125".into()),
            ..Default::default()
        }),
        windows: vec![QuotaWindow {
            id: "weekly".into(),
            name: "7d".into(),
            window_minutes: Some(10080),
            used_percent: 90.0,
            resets_at: Some(now() + 10000),
            ..Default::default()
        }],
        ..Default::default()
    };
    (dir, store, q)
}
fn event(store: &Store, q: &QuotaSnapshot, model: &str, category: &str, id: &str) {
    let event = UsageEvent {
        id: id.into(),
        import_identity: None,
        source_id: "s".into(),
        account_id: "a".into(),
        provider: "codex".into(),
        session_id: "new".into(),
        model: model.into(),
        timestamp: q.updated_at + 5,
        tokens: Tokens {
            input: 100,
            output: 20,
            cache_read: 200,
            cache_write: 30,
            reasoning: 10,
        },
        attribution: "delta".into(),
        billing: BillingEvidence {
            category: category.into(),
            evidence: "fixture".into(),
        },
        interval_start: Some(q.updated_at),
    };
    store
        .put_event(&event, &store.prices().unwrap(), &store.settings().unwrap())
        .unwrap();
}
#[test]
fn cache_prices_are_shared_by_weekly_and_credit_estimates() {
    let (_dir, store, mut q) = setup();
    let p = ModelPrice {
        id: "m".into(),
        input: Some(0.01),
        output: Some(0.02),
        cache_read: Some(0.001),
        cache_write: Some(0.0125),
        ..Default::default()
    };
    store.save_prices(std::slice::from_ref(&p)).unwrap();
    let c = store
        .begin_estimate(&q, "credits", vec!["s".into()], None)
        .unwrap();
    let w = store
        .begin_estimate(&q, "weekly", vec!["s".into()], None)
        .unwrap();
    event(&store, &q, "m", "subscription", "one");
    event(&store, &q, "m", "api", "api");
    q.updated_at += 20;
    q.credits.as_mut().unwrap().balance = Some("990.125".into());
    q.windows[0].used_percent = 100.0;
    store.quota(&q).unwrap();
    let c = store.finish_estimate(&c.id).unwrap();
    let w = store.finish_estimate(&w.id).unwrap();
    assert!((c.cost - 1.975).abs() < 1e-9);
    assert_eq!(c.cost, w.cost);
    assert_eq!(c.total_tokens, 350);
    assert_eq!(c.priced_tokens, 350);
    assert_eq!(c.excluded_api_tokens, 350);
    assert_eq!(c.consumed_credits, 10.0);
    assert!((c.value_per500.unwrap() - 98.75).abs() < 1e-8);
    assert!((c.value_per1000.unwrap() - 197.5).abs() < 1e-8);
    store
        .save_prices(&[ModelPrice {
            input: Some(10.0),
            ..p
        }])
        .unwrap();
    store.reprice(&store.settings().unwrap(), false).unwrap();
    assert!((store.estimate(&c.id).unwrap().cost - c.cost).abs() < 1e-12);
}
#[test]
fn credit_balance_is_exact_and_missing_is_not_zero() {
    assert_eq!(
        credit_units("1000000000.000000001").unwrap()
            - credit_units("1000000000.000000000").unwrap(),
        1_000_000_000
    );
    for invalid in ["NaN", "inf", "-1", "1e3", "", ".5", "1.0000000000000000001"] {
        assert!(credit_units(invalid).is_none());
    }
    let (_dir, store, mut q) = setup();
    q.credits = None;
    assert!(
        store
            .begin_estimate(&q, "credits", vec!["s".into()], None)
            .is_err()
    );
    q.credits = Some(CreditsSnapshot {
        unlimited: true,
        balance: Some("50".into()),
        ..Default::default()
    });
    assert!(
        store
            .begin_estimate(&q, "credits", vec!["s".into()], None)
            .is_err()
    );
}
#[test]
fn credit_sampling_pauses_on_topup_and_preserves_valid_checkpoint() {
    let (dir, store, mut q) = setup();
    let e = store
        .begin_estimate(&q, "credits", vec!["s".into()], None)
        .unwrap();
    q.updated_at += 10;
    q.credits.as_mut().unwrap().balance = Some("990".into());
    store.quota(&q).unwrap();
    let stamp = q.updated_at;
    q.updated_at += 10;
    q.credits.as_mut().unwrap().balance = Some("1200".into());
    store.quota(&q).unwrap();
    let paused = store.estimate(&e.id).unwrap();
    assert_eq!(paused.status, "pending");
    assert_eq!(paused.checkpoint_at, stamp);
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.estimate(&e.id).unwrap().checkpoint_balance.as_deref(),
        Some("990")
    );
    let restarted = store
        .begin_estimate(&q, "credits", vec!["s".into()], Some(&e.id))
        .unwrap();
    assert_eq!(restarted.baseline_balance.as_deref(), Some("1200"));
    q.updated_at += 10;
    q.windows[0].used_percent = 1.0;
    store.quota(&q).unwrap();
    assert_eq!(store.estimate(&restarted.id).unwrap().status, "pending");
}
#[test]
fn small_unpriced_or_boundary_samples_have_no_value() {
    let (_dir, store, mut q) = setup();
    let e = store
        .begin_estimate(&q, "credits", vec!["s".into()], None)
        .unwrap();
    event(&store, &q, "missing", "subscription", "one");
    q.updated_at += 10;
    q.credits.as_mut().unwrap().balance = Some("999".into());
    store.quota(&q).unwrap();
    assert!(store.estimate(&e.id).unwrap().value_per500.is_none());
    q.updated_at += 10;
    q.credits.as_mut().unwrap().balance = Some("900".into());
    store.quota(&q).unwrap();
    assert!(
        store
            .estimate(&e.id)
            .unwrap()
            .calculation_note
            .contains("价格")
    );
    let p = ModelPrice {
        id: "missing".into(),
        input: Some(1.0),
        output: Some(1.0),
        cache_read: Some(1.0),
        cache_write: Some(1.0),
        ..Default::default()
    };
    store.save_prices(&[p]).unwrap();
    store.reprice(&store.settings().unwrap(), true).unwrap();
    q.error = Some("offline".into());
    store.quota(&q).unwrap();
    assert_eq!(store.estimate(&e.id).unwrap().status, "pending");
}
#[test]
fn credits_are_not_reset_counts_or_sums_and_logs_cannot_replace_live_balances() {
    let (_dir, store, _) = setup();
    let source = store.settings().unwrap().sources[0].clone();
    let stamp = now();
    let q = usage::codex_quota(
        &source,
        &json!({"rateLimits":{"credits":{"hasCredits":true,"unlimited":false,"balance":"123.45"}},"rateLimitsByLimitId":{"codex":{"credits":{"hasCredits":true,"unlimited":false,"balance":"123.45"}},"other":{"credits":{"hasCredits":true,"unlimited":false,"balance":"123.45"}}},"rateLimitResetCredits":{"availableCount":2}}),
        stamp,
        "live",
    );
    store.quota(&q).unwrap();
    assert_eq!(q.credits.unwrap().balance.as_deref(), Some("123.45"));
    assert_eq!(q.bank_reset.unwrap()["availableCount"], 2);
    for n in 1..=2 {
        store
            .quota(&usage::codex_quota(
                &source,
                &json!({"credits":{"has_credits":true,"unlimited":false,"balance":"900"}}),
                stamp + n,
                "log",
            ))
            .unwrap();
    }
    let d = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(
        d.quotas[0].credits.as_ref().unwrap().balance.as_deref(),
        Some("123.45")
    );
    assert_eq!(d.quotas[0].credits_updated_at, Some(stamp));
    let t=usage::codex_tokens(&json!({"input_tokens":330,"cached_input_tokens":200,"cache_write_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":10})).unwrap();
    assert_eq!(t.input, 100);
    assert_eq!(t.total(), 350);
    assert_eq!(pricing::estimate(&t, None), (0.0, 0));
}

#[test]
fn normalized_credit_snapshot_preserves_weekly_windows() {
    let (_dir, store, _) = setup();
    let source = store.settings().unwrap().sources[0].clone();
    let q = aieyes_core::quota::normalize(
        &source,
        &json!({
            "windows": [{"id":"weekly", "name":"7d", "windowMinutes":10080,
                "usedPercent":75.0, "resetsAt":now()+1000}],
            "credits":{"hasCredits":true,"unlimited":false,"balance":"100.5"}
        }),
        "live",
    )
    .unwrap();
    assert_eq!(q.windows.len(), 1);
    assert_eq!(q.windows[0].used_percent, 75.0);
    assert_eq!(q.credits.unwrap().balance.as_deref(), Some("100.5"));
    assert_eq!(q.credits_origin.as_deref(), Some("live"));
}
