use super::*;

fn window(used: f64, reset: Option<i64>) -> QuotaWindow {
    QuotaWindow {
        id: "primary".into(),
        name: "5h".into(),
        window_minutes: Some(300),
        used_percent: used,
        resets_at: reset,
        ..Default::default()
    }
}
#[test]
fn classification_is_conservative_and_allows_usage_after_reset() {
    let old = window(80.0, Some(10_000));
    for (used, reset, stamp, expected) in [
        (5.0, Some(20_000), 5_000, Some(Kind::Unexpected)),
        (0.0, Some(10_000), 5_000, Some(Kind::Unexpected)),
        (1.0, None, 5_000, Some(Kind::Unexpected)),
        (5.0, Some(10_000), 5_000, None),
        (79.5, Some(20_000), 5_000, None),
        (5.0, Some(10_060), 5_000, None),
        (0.0, Some(20_000), 9_939, Some(Kind::Unexpected)),
        (0.0, Some(20_000), 9_940, Some(Kind::Natural)),
        (90.0, Some(30_000), 20_000, Some(Kind::Natural)),
        (80.0, Some(10_000), 10_000, None),
        (80.0, Some(30_000), 20_000, Some(Kind::Natural)),
    ] {
        assert_eq!(
            classify(&old, &window(used, reset), stamp),
            expected,
            "{used} / {reset:?} / {stamp}"
        );
    }
    assert_eq!(
        classify(
            &window(0.0, Some(10_000)),
            &window(0.0, Some(30_000)),
            20_000
        ),
        None
    );
    assert_eq!(
        classify(
            &window(0.5, Some(10_000)),
            &window(0.0, Some(30_000)),
            20_000
        ),
        None
    );
    assert_eq!(
        classify(&window(80.0, None), &window(0.0, Some(30_000)), 20_000),
        None
    );
    let mut changed = window(0.0, Some(30_000));
    changed.window_minutes = Some(10080);
    assert_eq!(classify(&old, &changed, 5_000), None);
}
fn fixture() -> (tempfile::TempDir, Store, Settings, QuotaSnapshot) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let settings = Settings {
        sources: vec![Source {
            id: "s".into(),
            path: root.path().to_string_lossy().into_owned(),
            account_id: "a".into(),
            ..Default::default()
        }],
        accounts: vec![Account {
            id: "a".into(),
            name: "Test".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let settings = store.settings().unwrap();
    let q = QuotaSnapshot {
        source_id: "s".into(),
        account_id: "a".into(),
        provider: "codex".into(),
        origin: "live".into(),
        updated_at: 100,
        windows: vec![window(80.0, Some(10_000))],
        ..Default::default()
    };
    (root, store, settings, q)
}
fn observe(store: &Store, s: &Settings, q: &QuotaSnapshot) {
    let tx = store.write_transaction().unwrap();
    store.quota(q).unwrap();
    store
        .observe_quota_reset(
            q,
            s,
            &crate::quota_queries::fingerprint(s, &s.accounts[0]).unwrap(),
        )
        .unwrap();
    tx.commit().unwrap();
}
#[test]
fn first_snapshot_idle_repeats_and_two_resets_at_the_same_deadline() {
    let (_root, store, s, mut q) = fixture();
    observe(&store, &s, &q);
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    observe(&store, &s, &q);
    assert_eq!(store.take_reset_notifications(0).unwrap().len(), 1);
    for _ in 0..3 {
        q.updated_at += 1;
        q.windows[0].resets_at = Some(q.windows[0].resets_at.unwrap() + 1000);
        observe(&store, &s, &q);
    }
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    q.updated_at += 1;
    q.windows[0].used_percent = 20.0;
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    assert_eq!(store.take_reset_notifications(0).unwrap().len(), 1);
}
#[test]
fn invalid_log_error_and_out_of_order_samples_do_not_replace_baseline() {
    let (_root, store, s, mut q) = fixture();
    observe(&store, &s, &q);
    for mut ignored in [q.clone(), q.clone(), q.clone(), q.clone()] {
        ignored.updated_at = 90;
        ignored.windows[0].used_percent = 0.0;
        observe(&store, &s, &ignored);
    }
    for origin in ["log", ""] {
        let mut ignored = q.clone();
        ignored.updated_at = 101;
        ignored.origin = origin.into();
        ignored.windows[0].used_percent = 0.0;
        observe(&store, &s, &ignored);
    }
    let mut failed = q.clone();
    failed.updated_at = 102;
    failed.error = Some("offline".into());
    observe(&store, &s, &failed);
    let mut invalid = q.clone();
    invalid.updated_at = 103;
    invalid.windows[0].used_percent = 101.0;
    observe(&store, &s, &invalid);
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    q.updated_at = 104;
    q.windows[0].used_percent = 5.0;
    q.windows[0].resets_at = Some(20_000);
    observe(&store, &s, &q);
    assert_eq!(store.take_reset_notifications(0).unwrap().len(), 1);
}
#[test]
fn preferences_inherit_and_disabled_events_do_not_accumulate() {
    let (_root, store, mut s, mut q) = fixture();
    assert_eq!(
        serde_json::from_str::<Settings>("{}")
            .unwrap()
            .quota_reset_notifications,
        Preferences::default()
    );
    assert_eq!(
        Overrides {
            unexpected: Some(false),
            natural: Some(true)
        }
        .resolve(&Preferences::default()),
        Preferences {
            unexpected: false,
            natural: true
        }
    );
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    s.quota_reset_notifications.unexpected = false;
    store.save_settings(&s).unwrap();
    s.quota_reset_notifications.unexpected = true;
    store.save_settings(&s).unwrap();
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    s.accounts[0].quota_reset_notifications.natural = Some(true);
    store.save_settings(&s).unwrap();
    q.updated_at += 1;
    q.windows[0].used_percent = 40.0;
    observe(&store, &s, &q);
    q.updated_at = 10_001;
    q.windows[0].used_percent = 45.0;
    q.windows[0].resets_at = Some(30_000);
    observe(&store, &s, &q);
    let events = store.take_reset_notifications(0).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, Kind::Natural);
}
#[test]
fn restart_delivery_concurrency_and_atomic_rollback() {
    let (root, store, s, mut q) = fixture();
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    {
        let _tx = store.write_transaction().unwrap();
        store
            .observe_quota_reset(
                &q,
                &s,
                &crate::quota_queries::fingerprint(&s, &s.accounts[0]).unwrap(),
            )
            .unwrap();
    }
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    observe(&store, &s, &q);
    drop(store);
    let path = root.path();
    let count = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            Store::open(path)
                .unwrap()
                .take_reset_notifications(0)
                .unwrap()
                .len()
        });
        let b = scope.spawn(|| {
            Store::open(path)
                .unwrap()
                .take_reset_notifications(0)
                .unwrap()
                .len()
        });
        a.join().unwrap() + b.join().unwrap()
    });
    assert_eq!(count, 1);
    let store = Store::open(path).unwrap();
    observe(&store, &s, &q);
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
    q.updated_at += 1;
    q.windows[0].used_percent = 30.0;
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    assert!(
        store
            .take_reset_notifications(q.updated_at + 1)
            .unwrap()
            .is_empty()
    );
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
}
#[test]
fn configuration_plan_identity_and_new_windows_rebaseline() {
    for change in 0..4 {
        let (_root, store, mut s, mut q) = fixture();
        observe(&store, &s, &q);
        q.updated_at += 1;
        q.windows[0].used_percent = 0.0;
        match change {
            0 => q.source_id = "other".into(),
            1 => q.plan = Some("pro".into()),
            2 => {
                q.identity = Some(AccountIdentity {
                    key: "new".into(),
                    ..Default::default()
                })
            }
            _ => q.windows[0].id = "new-window".into(),
        }
        observe(&store, &s, &q);
        assert!(store.take_reset_notifications(0).unwrap().is_empty());
        s.accounts[0].archived = true;
        store.save_settings(&s).unwrap();
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM quota_reset_baselines", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn leaving_a_subscription_discards_its_baseline() {
    let (_root, store, s, mut q) = fixture();
    q.plan = Some("pro".into());
    observe(&store, &s, &q);
    let windows = q.windows.clone();
    q.updated_at += 1;
    q.plan = Some("free".into());
    q.windows.clear();
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.plan = Some("pro".into());
    q.windows = windows;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
}
#[test]
fn multiple_windows_are_grouped_and_account_disabled_before_delivery() {
    let (_root, store, mut s, mut q) = fixture();
    let mut weekly = window(50.0, Some(20_000));
    weekly.id = "week".into();
    weekly.name = "7d".into();
    weekly.window_minutes = Some(10080);
    q.windows.push(weekly);
    observe(&store, &s, &q);
    q.updated_at += 1;
    for w in &mut q.windows {
        w.used_percent = 0.0;
    }
    observe(&store, &s, &q);
    let events = store.take_reset_notifications(0).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].windows.len(), 2);
    q.updated_at += 1;
    q.windows[0].used_percent = 80.0;
    observe(&store, &s, &q);
    q.updated_at += 1;
    q.windows[0].used_percent = 0.0;
    observe(&store, &s, &q);
    s.accounts[0].quota_enabled = false;
    store.save_settings(&s).unwrap();
    assert!(store.take_reset_notifications(0).unwrap().is_empty());
}

#[test]
fn native_poller_respects_intervals_and_confirms_each_expired_deadline_once() {
    let (root, store, s, mut q) = fixture();
    let now = crate::models::now();
    q.updated_at = now;
    q.windows[0].resets_at = Some(now - 1);
    store.quota(&q).unwrap();
    store
        .db
        .execute(
            "INSERT INTO kv VALUES('quota-attempt:codex:a',?1)",
            [now.to_string()],
        )
        .unwrap();
    let engine = crate::Engine::open(root.path()).unwrap();
    let mut poller = crate::quota_schedule::Poller::default();
    assert_eq!(
        poller.next(&engine).unwrap().unwrap()["accountKey"],
        "codex:a"
    );
    assert!(poller.next(&engine).unwrap().is_none());
    store
        .db
        .execute(
            "UPDATE kv SET value=?1 WHERE key='quota-attempt:codex:a'",
            [(now - 301).to_string()],
        )
        .unwrap();
    assert_eq!(poller.next(&engine).unwrap().unwrap()["dueOnly"], true);
    let mut disabled = s;
    disabled.accounts[0].quota_enabled = false;
    store.save_settings(&disabled).unwrap();
    assert!(poller.next(&engine).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn accepted_live_query_commits_a_deliverable_event_with_the_snapshot() {
    let (root, store, mut s, _) = fixture();
    let path = root.path().join("quota.json");
    s.sources[0].quota_command = format!("cat '{}'", path.display());
    store.save_settings(&s).unwrap();
    let now = crate::models::now();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"windows":[window(80.0,Some(now+10000))]})).unwrap(),
    )
    .unwrap();
    let mut engine = crate::Engine::open(root.path()).unwrap();
    let first = engine
        .call(
            "quotas.refresh",
            serde_json::json!({"accountKey":"codex:a"}),
        )
        .unwrap();
    assert!(first[0]["error"].is_null(), "{first}");
    // Simulate the prior poll's time without a wall-clock sleep.
    store
        .db
        .execute(
            "UPDATE quota_reset_baselines SET payload=json_set(payload,'$.quota.updatedAt',?1)",
            [now - 2],
        )
        .unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"windows":[window(5.0,Some(now+20000))]})).unwrap(),
    )
    .unwrap();
    let second = engine
        .call(
            "quotas.refresh",
            serde_json::json!({"accountKey":"codex:a"}),
        )
        .unwrap();
    assert!(second[0]["error"].is_null(), "{second}");
    let events = engine
        .call(
            "quotaResetNotifications.take",
            serde_json::json!({"since":now-10}),
        )
        .unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1, "{events}");
    assert_eq!(events[0]["windows"][0]["usedPercent"], 5.0);
    assert_eq!(events[0]["kind"], "unexpected");
    let quota: String = store
        .db
        .query_row(
            "SELECT payload FROM quotas WHERE account_key='codex:a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<QuotaSnapshot>(&quota)
            .unwrap()
            .windows[0]
            .used_percent,
        5.0
    );
}
