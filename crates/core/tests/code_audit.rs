use aieyes_core::{Engine, models::*, providers, store::Store, usage};
use rusqlite::params;
use serde_json::json;

#[test]
fn provider_capabilities_and_legacy_custom_settings_are_consistent() {
    assert!(providers::known("agy"));
    assert!(providers::supports_wakeup("agy"));
    assert!(!providers::supports_wakeup("deepseek"));
    assert!(!providers::supports_wakeup("custom"));
    assert!(providers::requires_path("custom"));
    assert!(!providers::requires_path("agy"));
    assert_eq!(
        providers::display_name("claude · 本机"),
        "Claude Code · 本机"
    );
    assert_eq!(providers::display_name("custom"), "自定义");
    let mut settings: Settings = serde_json::from_value(json!({
        "version":1,"githubRepository":"legacy/repository",
        "sources":[{"id":"custom","provider":"custom","accountId":"account"}]
    }))
    .unwrap();
    settings.migrate();
    assert_eq!(settings.version, 7);
    assert!(settings.accounts[0].quota_enabled);
    assert_eq!(settings.agents.len(), providers::PROVIDERS.len());
    assert!(
        serde_json::to_value(settings)
            .unwrap()
            .get("githubRepository")
            .is_none()
    );
}

#[test]
fn canonical_ids_migrate_duplicates_preserving_price_sources_and_account_isolation() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let source = Source {
        id: "first".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        ..Default::default()
    };
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    assert_eq!(
        event.id,
        usage::event_id(
            "agy",
            "first",
            "account",
            event.import_identity.as_ref().unwrap()
        )
    );
    let price = ModelPrice {
        id: "model".into(),
        input: Some(0.01),
        ..Default::default()
    };
    store
        .put_event(&event, std::slice::from_ref(&price), &Settings::default())
        .unwrap();
    let old_id = hash(&format!(
        "agy:account:{}",
        event.import_identity.as_ref().unwrap()
    ));
    let mut legacy = event.clone();
    legacy.id = old_id.clone();
    legacy.source_id = "copy".into();
    legacy.tokens.input = 200;
    store
        .db
        .execute(
            "INSERT INTO events VALUES(?1,'antigravity','account','model',?2,?3,2.0,200,?4)",
            params![
                old_id,
                event.timestamp,
                serde_json::to_string(&legacy).unwrap(),
                serde_json::to_string(&price).unwrap()
            ],
        )
        .unwrap();
    store
        .db
        .execute("INSERT INTO event_sources VALUES(?1,'copy')", [&old_id])
        .unwrap();
    for (id, account) in [("other-account", "other"), ("unassigned", "")] {
        let source = Source {
            id: id.into(),
            account_id: account.into(),
            ..source.clone()
        };
        let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
        store
            .put_event(&event, std::slice::from_ref(&price), &Settings::default())
            .unwrap();
    }
    store
        .db
        .execute("DELETE FROM kv WHERE key='usage.ids.canonical.v1'", [])
        .unwrap();
    drop(store);
    let store = Store::open(temp.path()).unwrap();
    let (cost, tokens): (f64, u64) = store
        .db
        .query_row(
            "SELECT cost,priced_tokens FROM events WHERE id=?1",
            [&event.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((cost, tokens), (2.0, 200));
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM event_sources WHERE event_id=?1",
                [&event.id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT SUM(event_count) FROM usage_source_accounts",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        4
    );
    let copy = Source {
        id: "new-copy".into(),
        ..source
    };
    let event = usage::parse(&copy, &mut ParseState::default(), &row).unwrap();
    assert!(
        !store
            .put_event(&event, &[], &store.settings().unwrap())
            .unwrap()
    );
    drop(store);
    let store = Store::open(temp.path()).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT SUM(cost) FROM events", [], |r| r.get::<_, f64>(0))
            .unwrap(),
        4.0
    );
}

#[test]
fn read_only_settings_reject_edits_but_allow_stale_unchanged_values() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let base = serde_json::to_value(store.settings().unwrap()).unwrap();
    for key in ["deletedAccounts", "accountAliases"] {
        let mut next = base.clone();
        next[key] = if key == "deletedAccounts" {
            json!([{"id":"deleted","provider":"custom"}])
        } else {
            json!({"agy:a":"antigravity:b"})
        };
        assert!(
            store
                .patch_settings(&base, &next, false)
                .unwrap_err()
                .to_string()
                .contains("只读")
        );
        assert_eq!(
            serde_json::to_value(store.settings().unwrap()).unwrap(),
            base
        );
    }
    store
        .db
        .execute(
            "INSERT INTO kv VALUES('deleted-account:custom:deleted',?1)",
            [json!({"id":"deleted","provider":"custom"}).to_string()],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO events VALUES('history','custom','deleted','model',1,'{}',0,0,NULL)",
            [],
        )
        .unwrap();
    let mut next = base.clone();
    next["menuMetric"] = json!("tokens");
    let saved = store.patch_settings(&base, &next, false).unwrap();
    assert_eq!(saved.deleted_accounts.len(), 1);
    assert_eq!(saved.menu_metric, "tokens");
}

#[test]
fn historical_hashes_without_identities_are_preserved_until_reimport() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let source = Source {
        id: "legacy-source".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        ..Default::default()
    };
    let row = json!({"id":"old","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    let old_id = hash(&format!(
        "agy:account:{}",
        event.import_identity.as_ref().unwrap()
    ));
    let mut payload = serde_json::to_value(&event).unwrap();
    payload["id"] = json!(old_id);
    payload.as_object_mut().unwrap().remove("importIdentity");
    store
        .db
        .execute(
            "INSERT INTO events VALUES(?1,'antigravity','account','model',?2,?3,0,0,NULL)",
            params![old_id, event.timestamp, payload.to_string()],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO event_sources VALUES(?1,?2)",
            params![old_id, source.id],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO source_identities VALUES(?1,'antigravity','account')",
            [&source.id],
        )
        .unwrap();
    store
        .db
        .execute("DELETE FROM kv WHERE key='usage.ids.canonical.v1'", [])
        .unwrap();
    drop(store);
    let store = Store::open(temp.path()).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT id FROM events", [], |r| r.get::<_, String>(0))
            .unwrap(),
        old_id
    );
    let copy = Source {
        id: "copy".into(),
        ..source
    };
    let copy_event = usage::parse(&copy, &mut ParseState::default(), &row).unwrap();
    assert!(
        !store
            .put_event(&copy_event, &[], &store.settings().unwrap())
            .unwrap()
    );
    drop(store);
    let store = Store::open(temp.path()).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT id FROM events", [], |r| r.get::<_, String>(0))
            .unwrap(),
        event.id
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM event_sources", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        2
    );
}

fn insert_historical_event(store: &Store, event: &UsageEvent, price: &ModelPrice) {
    let mut old = event.clone();
    old.import_identity = None;
    let (cost, priced) = aieyes_core::pricing::estimate(&old.tokens, Some(price));
    store
        .db
        .execute(
            "INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                old.id,
                old.provider,
                old.account_id,
                old.model,
                old.timestamp,
                serde_json::to_string(&old).unwrap(),
                cost,
                priced,
                serde_json::to_string(price).unwrap()
            ],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO event_sources VALUES(?1,?2)",
            params![old.id, old.source_id],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT OR IGNORE INTO source_identities VALUES(?1,?2,?3)",
            params![old.source_id, old.provider, old.account_id],
        )
        .unwrap();
}

#[test]
fn full_log_replay_repairs_two_historical_hashes_without_identities() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("store");
    let store = Store::open(&database).unwrap();
    let log = root.path().join("event.jsonl");
    let source = Source {
        id: "first".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        path: log.to_string_lossy().into(),
        ..Default::default()
    };
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    std::fs::write(&log, format!("{row}\n")).unwrap();
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    let mut legacy = event.clone();
    legacy.id = hash(&format!(
        "agy:account:{}",
        event.import_identity.as_ref().unwrap()
    ));
    let price = ModelPrice {
        id: "model".into(),
        input: Some(0.01),
        ..Default::default()
    };
    for old in [&event, &legacy] {
        insert_historical_event(&store, old, &price);
    }
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.total,
        200
    );
    // Real imports already own a transaction, so repair must support that path.
    assert_eq!(
        aieyes_core::import::scan(&store, &Settings::default(), &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.total,
        100
    );
    let copy = Source {
        id: "copy".into(),
        ..source.clone()
    };
    assert_eq!(
        aieyes_core::import::scan(&store, &Settings::default(), &copy).unwrap()["newEvents"],
        0
    );
    drop(store);
    let store = Store::open(&database).unwrap();
    let dashboard = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(
        (dashboard.summary.total, dashboard.summary.cost),
        (100, 1.0)
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM event_sources WHERE event_id=?1",
                [&event.id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn replay_merges_alias_hashes_preserving_frozen_prices_metadata_and_isolation() {
    for (account, legacy_account, replay_source) in [
        ("account", "account", "first"),
        ("account", "account", "new-copy"),
        ("legacy-account", "account", "new-copy"),
        ("", "", "first"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "first".into(),
            provider: "antigravity".into(),
            account_id: account.into(),
            ..Default::default()
        };
        let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
        let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
        let namespace = if legacy_account.is_empty() {
            "source:first"
        } else {
            legacy_account
        };
        let mut legacy = event.clone();
        legacy.id = hash(&format!(
            "agy:{namespace}:{}",
            event.import_identity.as_ref().unwrap()
        ));
        legacy.source_id = "legacy-copy".into();
        legacy.tokens.output = 20;
        legacy.tokens.reasoning = 17;
        legacy.billing.category = "api".into();
        let frozen = ModelPrice {
            id: "model".into(),
            input: Some(0.01),
            output: Some(0.02),
            ..Default::default()
        };
        let changed = ModelPrice {
            input: Some(0.9),
            output: Some(0.9),
            ..frozen.clone()
        };
        insert_historical_event(&store, &event, &frozen);
        insert_historical_event(&store, &legacy, &changed);
        // The same identity in another account or unassigned source stays separate.
        let other_source = Source {
            id: "other".into(),
            account_id: if account == "legacy-account" {
                "account"
            } else {
                "other-account"
            }
            .into(),
            ..source.clone()
        };
        let other = usage::parse(&other_source, &mut ParseState::default(), &row).unwrap();
        insert_historical_event(&store, &other, &frozen);
        let unassigned = Source {
            id: "unassigned".into(),
            account_id: "".into(),
            ..source.clone()
        };
        let separate = usage::parse(&unassigned, &mut ParseState::default(), &row).unwrap();
        insert_historical_event(&store, &separate, &frozen);
        let mut settings = Settings::default();
        if account == "legacy-account" {
            settings
                .account_aliases
                .insert("agy:account".into(), "antigravity:legacy-account".into());
        }
        let replay = usage::parse(
            &Source {
                id: replay_source.into(),
                ..source
            },
            &mut ParseState::default(),
            &row,
        )
        .unwrap();
        assert!(!store.put_event(&replay, &[changed], &settings).unwrap());
        let (raw, price, cost): (String, String, f64) = store
            .db
            .query_row(
                "SELECT payload,price,cost FROM events WHERE id=?1",
                [&event.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let merged: UsageEvent = serde_json::from_str(&raw).unwrap();
        assert_eq!((merged.tokens.total(), merged.tokens.reasoning), (120, 17));
        assert_eq!(merged.billing.category, "api");
        assert_eq!(merged.import_identity, event.import_identity);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&price).unwrap(),
            serde_json::to_value(frozen).unwrap()
        );
        assert!((cost - 1.4).abs() < 1e-10);
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT COUNT(*) FROM event_sources WHERE event_id=?1",
                    [&event.id],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            if replay_source == "first" { 2 } else { 3 }
        );
        for kept in [&other.id, &separate.id] {
            assert!(store.db.query_row("SELECT json_extract(payload,'$.importIdentity') IS NULL FROM events WHERE id=?1", [kept], |r| r.get::<_, bool>(0)).unwrap());
        }
    }
}

#[test]
fn failed_historical_merge_rolls_back_tokens_memberships_and_cache_revisions() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let source = Source {
        id: "first".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        ..Default::default()
    };
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    let mut legacy = event.clone();
    legacy.id = hash(&format!(
        "agy:account:{}",
        event.import_identity.as_ref().unwrap()
    ));
    legacy.tokens.input = 200;
    let price = ModelPrice {
        id: "model".into(),
        input: Some(0.01),
        ..Default::default()
    };
    for old in [&event, &legacy] {
        insert_historical_event(&store, old, &price);
    }
    let before = store.dashboard(&Filter::default()).unwrap().summary;
    store.db.execute_batch("CREATE TRIGGER reject_event_removal BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'fixture merge failure'); END;").unwrap();
    assert!(
        store
            .put_event(&event, &[price], &Settings::default())
            .is_err()
    );
    let after = store.dashboard(&Filter::default()).unwrap().summary;
    assert_eq!((after.total, after.cost), (before.total, before.cost));
    assert_eq!(store.db.query_row("SELECT COUNT(*) FROM events WHERE json_extract(payload,'$.importIdentity') IS NULL", [], |r| r.get::<_, u64>(0)).unwrap(), 2);
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM event_sources", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert!(store.db.is_autocommit());
}

#[test]
fn only_manual_full_rescan_replays_finished_logs_and_repairs_old_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("store");
    let log = root.path().join("event.jsonl");
    let mut engine = Engine::open(&database).unwrap();
    let source = Source {
        id: "first".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        path: log.to_string_lossy().into(),
        enabled: true,
        ..Default::default()
    };
    let settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    engine.store.save_settings(&settings).unwrap();
    let price = ModelPrice {
        id: "model".into(),
        input: Some(0.01),
        ..Default::default()
    };
    engine
        .store
        .save_prices(std::slice::from_ref(&price))
        .unwrap();
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    std::fs::write(&log, format!("{row}\n")).unwrap();
    engine.call("sources.scan", json!({})).unwrap();
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    engine
        .store
        .db
        .execute(
            "UPDATE events SET payload=json_remove(payload,'$.importIdentity')",
            [],
        )
        .unwrap();
    let mut legacy = event.clone();
    legacy.id = hash(&format!(
        "agy:account:{}",
        event.import_identity.as_ref().unwrap()
    ));
    insert_historical_event(&engine.store, &legacy, &price);
    drop(engine);
    let mut engine = Engine::open(&database).unwrap();
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        200
    );
    let incremental = engine.call("sources.scan", json!({"full":true})).unwrap();
    assert_eq!(incremental[0]["result"]["readBytes"], 0);
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        200
    );
    for _ in 0..2 {
        let replay = engine.call("sources.rescan", json!({})).unwrap();
        assert!(replay[0]["error"].is_null());
        assert!(replay[0]["result"]["readBytes"].as_u64().unwrap() > 0);
        assert_eq!(replay[0]["result"]["newEvents"], 0);
        let summary = engine.store.dashboard(&Filter::default()).unwrap().summary;
        assert_eq!((summary.total, summary.cost), (100, 1.0));
    }
    let incremental = engine.call("sources.scan", json!({})).unwrap();
    assert_eq!(incremental[0]["result"]["readBytes"], 0);
}

#[test]
fn full_rescan_preserves_missing_history_and_reports_partial_or_failed_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(&root.path().join("store")).unwrap();
    let log = root.path().join("event.jsonl");
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"model","tokens":{"input":100}});
    std::fs::write(&log, format!("{row}\n")).unwrap();
    let source = Source {
        id: "active".into(),
        provider: "custom".into(),
        account_id: "account".into(),
        path: log.to_string_lossy().into(),
        enabled: true,
        ..Default::default()
    };
    let disabled = Source {
        id: "disabled".into(),
        enabled: false,
        account_id: "other".into(),
        ..source.clone()
    };
    engine
        .store
        .save_settings(&Settings {
            sources: vec![source.clone(), disabled.clone()],
            ..Default::default()
        })
        .unwrap();
    engine.call("sources.scan", json!({})).unwrap();
    let other = usage::parse(&disabled, &mut ParseState::default(), &row).unwrap();
    engine
        .store
        .put_event(&other, &[], &Settings::default())
        .unwrap();
    let cursor_before = engine
        .store
        .cursor(&source.id, &source.path)
        .unwrap()
        .unwrap();
    std::fs::remove_file(&log).unwrap();
    let failed = engine.call("sources.rescan", json!({})).unwrap();
    assert_eq!(failed.as_array().unwrap().len(), 1);
    assert_eq!(failed[0]["id"], source.id);
    assert!(failed[0]["error"].is_string());
    assert_eq!(
        engine
            .store
            .cursor(&source.id, &source.path)
            .unwrap()
            .unwrap()
            .1,
        cursor_before.1
    );
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        200
    );
    std::fs::write(&log, format!("{row}\ninvalid json\n")).unwrap();
    let partial = engine.call("sources.rescan", json!({})).unwrap();
    assert_eq!(partial[0]["partial"], true);
    assert_eq!(partial[0]["issues"][0]["code"], "invalidRecord");
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        200
    );
}
