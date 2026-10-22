use aieyes_core::{import, models::*, store::Store};
use serde_json::json;
use std::fs;

fn fixture(provider: &str) -> (tempfile::TempDir, Store, Settings, Source) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("data")).unwrap();
    let source = Source {
        id: "history".into(),
        provider: provider.into(),
        path: dir.path().join("logs").to_string_lossy().into(),
        account_id: "account".into(),
        ..Default::default()
    };
    fs::create_dir_all(&source.path).unwrap();
    let settings = Settings {
        sources: vec![source.clone()],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    (dir, store, settings, source)
}
fn write(path: &std::path::Path, rows: &[serde_json::Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
}
fn total(store: &Store) -> u64 {
    store.dashboard(&Filter::default()).unwrap().summary.total
}

#[test]
fn global_default_migrates_once_and_legacy_input_still_round_trips() {
    assert!(Settings::default().precise_counting);
    assert!(
        !serde_json::from_value::<Settings>(json!({"codexPreciseCounting":false}))
            .unwrap()
            .precise_counting
    );
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
            [
                json!({"version":11,"codexPreciseCounting":false,"menuMetric":"tokens"})
                    .to_string(),
            ],
        )
        .unwrap();
    store
        .db
        .execute("DELETE FROM kv WHERE key='counting.global-default.v1'", [])
        .unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    let mut settings = store.settings().unwrap();
    assert!(settings.precise_counting);
    assert_eq!(settings.menu_metric, "tokens");
    settings.precise_counting = false;
    store.save_settings(&settings).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert!(!store.settings().unwrap().precise_counting);
    let raw: String = store
        .db
        .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!raw.contains("codexPreciseCounting"));
}

#[test]
fn pi_and_omp_copies_switch_both_directions_without_losing_prices_or_new_requests() {
    for provider in ["pi", "omp"] {
        let (_dir, store, mut settings, source) = fixture(provider);
        let stamp = chrono::Utc::now().to_rfc3339();
        let usage = |id: &str, input: u64| json!({"type":"message","id":id,"timestamp":stamp,"message":{"role":"assistant","model":"fixture","usage":{"input":input,"output":0}}});
        write(
            &std::path::Path::new(&source.path).join("parent.jsonl"),
            &[
                json!({"type":"session","id":"parent","version":3}),
                usage("original", 100),
            ],
        );
        write(
            &std::path::Path::new(&source.path).join("child.jsonl"),
            &[
                json!({"type":"session","id":"child","version":3,"parentSession":"parent.jsonl"}),
                usage("original", 100),
                usage("new", 20),
            ],
        );
        store
            .save_prices(&[ModelPrice {
                id: "fixture".into(),
                input: Some(0.01),
                ..Default::default()
            }])
            .unwrap();
        for (mode, expected) in [
            (true, 120),
            (false, 220),
            (true, 120),
            (false, 220),
            (true, 120),
        ] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected, "{provider} precise={mode}");
            let revision: i64 = store
                .db
                .query_row(
                    "SELECT COALESCE(SUM(revision),0) FROM usage_day_revision",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            import::rescan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected);
            assert_eq!(
                store
                    .db
                    .query_row::<i64, _, _>(
                        "SELECT COALESCE(SUM(revision),0) FROM usage_day_revision",
                        [],
                        |r| r.get(0)
                    )
                    .unwrap(),
                revision,
                "replay must not rewrite the projection"
            );
            assert!(
                (store.dashboard(&Filter::default()).unwrap().summary.cost
                    - expected as f64 * 0.01)
                    .abs()
                    < 1e-8
            );
            store
                .save_prices(&[ModelPrice {
                    id: "fixture".into(),
                    input: Some(9.0),
                    ..Default::default()
                }])
                .unwrap();
        }
    }
}

#[test]
fn kimi_inherited_prefix_remains_reversible_and_post_fork_identity_stays_stable() {
    let (_dir, store, mut settings, source) = fixture("kimi");
    let now = now() * 1000;
    let usage = |input: u64| json!({"type":"usage.record","time":now,"model":"fixture","usage":{"inputOther":input,"output":0,"inputCacheRead":0,"inputCacheCreation":0}});
    write(
        &std::path::Path::new(&source.path).join("sessions/parent/agents/main/wire.jsonl"),
        &[usage(100)],
    );
    write(
        &std::path::Path::new(&source.path).join("sessions/child/agents/main/wire.jsonl"),
        &[
            usage(100),
            json!({"type":"forked","agentId":"main"}),
            usage(20),
        ],
    );
    for (mode, expected) in [(true, 120), (false, 220), (true, 120)] {
        settings.precise_counting = mode;
        store.save_settings(&settings).unwrap();
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(total(&store), expected);
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(total(&store), expected);
    }
}

#[test]
fn native_response_copies_deduplicate_but_independent_requests_and_accounts_do_not() {
    for provider in ["claude", "qoder", "codebuddy"] {
        let (_dir, store, mut settings, source) = fixture(provider);
        let stamp = chrono::Utc::now().to_rfc3339();
        let usage = |session: &str, id: &str, input: u64| {
            if provider == "codebuddy" {
                json!({"type":"message","role":"assistant","status":"completed","timestamp":now()*1000,"sessionId":session,"providerData":{"messageId":id,"model":"fixture","rawUsage":{"input_tokens":input,"output_tokens":0}}})
            } else {
                json!({"type":"assistant","timestamp":stamp,"sessionId":session,"requestId":id,"message":{"id":id,"model":"fixture","usage":{"input_tokens":input,"output_tokens":0}}})
            }
        };
        write(
            &std::path::Path::new(&source.path).join("parent.jsonl"),
            &[usage("parent", "original", 100)],
        );
        write(
            &std::path::Path::new(&source.path).join("child.jsonl"),
            &[usage("child", "original", 100), usage("child", "new", 20)],
        );
        for (mode, expected) in [(true, 120), (false, 220), (true, 120)] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected, "{provider}");
        }
        let mut other = source.clone();
        other.id = "other-account".into();
        other.account_id = "independent".into();
        settings.sources.push(other.clone());
        store.save_settings(&settings).unwrap();
        import::scan(&store, &settings, &other).unwrap();
        assert_eq!(total(&store), 240);
    }
}

#[test]
fn a_rapid_round_trip_rejects_a_stale_scan_even_when_the_flag_matches_again() {
    let (_dir, store, _, source) = fixture("custom");
    let old = store.settings().unwrap();
    let mut settings = old.clone();
    settings.precise_counting = false;
    store.save_settings(&settings).unwrap();
    settings.precise_counting = true;
    store.save_settings(&settings).unwrap();
    assert!(import::scan(&store, &old, &source).is_err());
}

#[test]
fn harness_seed_usage_is_reversible_in_plain_and_compressed_history() {
    for compressed in [false, true] {
        let (_dir, store, mut settings, source) = fixture("deepseek-harness");
        let stamp = now() * 1000;
        let usage = |seq, input| json!({"type":"assistant/message","seq":seq,"time":stamp+seq,"data":{"turn":1,"step":seq,"message":{"source":{"model":"fixture"}},"usage":{"inputTokens":input,"outputTokens":0}}});
        for (session, seed, rows) in [
            ("parent", false, vec![usage(1, 100)]),
            (
                "child",
                true,
                vec![
                    usage(1, 100),
                    json!({"type":"session/end-seed"}),
                    usage(2, 20),
                ],
            ),
        ] {
            let path = std::path::Path::new(&source.path)
                .join(session)
                .join(if compressed {
                    "session.v4.jsonl.zstd"
                } else {
                    "session.v4.jsonl"
                });
            let mut data = vec![json!({"type":"session","version":4,"id":session,"isSeeded":seed})];
            data.extend(rows);
            write(&path, &data);
            if compressed {
                fs::write(
                    &path,
                    zstd::stream::encode_all(fs::read(&path).unwrap().as_slice(), 1).unwrap(),
                )
                .unwrap();
            }
        }
        for (mode, expected) in [(true, 120), (false, 220), (true, 120)] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected);
            import::rescan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected);
        }
    }
}

#[test]
fn grok_event_identity_survives_copies_but_equal_new_requests_still_count() {
    let (_dir, store, mut settings, source) = fixture("grok");
    let stamp = now() * 1000;
    let event = |session, id| json!({"method":"_x.ai/session/update","params":{"sessionId":session,"_meta":{"eventId":id,"agentTimestampMs":stamp},"update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"fixture":{"inputTokens":100,"outputTokens":0,"cachedReadTokens":0,"cacheCreationTokens":0,"reasoningTokens":0,"totalTokens":100}}}}}});
    write(
        &std::path::Path::new(&source.path).join("parent.jsonl"),
        &[event("parent", "original")],
    );
    write(
        &std::path::Path::new(&source.path).join("child.jsonl"),
        &[event("child", "original"), event("child", "new")],
    );
    for (mode, expected) in [(true, 200), (false, 300), (true, 200)] {
        settings.precise_counting = mode;
        store.save_settings(&settings).unwrap();
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(total(&store), expected);
    }
}

#[test]
fn opencode_v2_fork_boundaries_do_not_suppress_new_subagent_requests() {
    use rusqlite::{Connection, params};
    for boundary in ["before", "through"] {
        let (_dir, store, mut settings, source) = fixture("opencode");
        let db = Connection::open(std::path::Path::new(&source.path).join("opencode.db")).unwrap();
        db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,parent_id TEXT,fork_session_id TEXT,fork_boundary TEXT); CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,seq INTEGER,time_created INTEGER,data TEXT);").unwrap();
        db.execute(
            "INSERT INTO session_v2 VALUES('child',NULL,'parent',?1)",
            [json!({"type":boundary,"messageID":"boundary"}).to_string()],
        )
        .unwrap();
        // A normal parent relationship denotes delegation, not copied usage.
        db.execute(
            "INSERT INTO session_v2 VALUES('worker','parent',NULL,NULL)",
            [],
        )
        .unwrap();
        let rows = if boundary == "through" {
            vec![
                ("parent", "boundary", 1, 100),
                ("child", "copy-1", 1, 100),
                ("child", "new", 2, 20),
                ("worker", "worker-new", 1, 30),
            ]
        } else {
            vec![
                ("parent", "original", 1, 100),
                ("parent", "boundary", 2, 10),
                ("child", "copy-1", 1, 100),
                ("child", "new", 2, 20),
                ("worker", "worker-new", 1, 30),
            ]
        };
        for (session, id, seq, input) in rows {
            let data = json!({"time":{"created":now()*1000,"completed":now()*1000},"model":{"id":"fixture","providerID":"api"},"tokens":{"input":input,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}});
            db.execute(
                "INSERT INTO session_message VALUES(?1,?2,'assistant',?3,?4,?5)",
                params![id, session, seq, now() * 1000, data.to_string()],
            )
            .unwrap();
        }
        let base = if boundary == "through" { 150 } else { 160 };
        for (mode, expected) in [(true, base), (false, base + 100), (true, base)] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected, "{boundary}");
            import::rescan(&store, &settings, &source).unwrap();
            assert_eq!(total(&store), expected);
        }
    }
}

#[test]
fn standard_exports_keep_request_and_source_dedup_in_both_modes() {
    for provider in [
        "cursor",
        "custom",
        "deepseek",
        "antigravity",
        "zcode",
        "opencode",
    ] {
        let (_dir, store, mut settings, source) = fixture(provider);
        let event = |session, id, proof| json!({"id":id,"sessionId":session,"model":"fixture","timestamp":now(),"tokens":{"input":100},"inheritance":proof});
        let original = event(
            "parent",
            "request",
            json!({"origin":"provider-request-id","inherited":false}),
        );
        let copy = event(
            "child",
            "copy",
            json!({"origin":"provider-request-id","inherited":true}),
        );
        let independent = event("worker", "new", serde_json::Value::Null);
        write(
            &std::path::Path::new(&source.path).join("export.jsonl"),
            &[original.clone(), original, copy, independent],
        );
        let mut mirror = source.clone();
        mirror.id = "mirror".into();
        settings.sources.push(mirror.clone());
        for (mode, expected) in [(true, 200), (false, 300), (true, 200)] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            import::scan(&store, &settings, &mirror).unwrap();
            assert_eq!(total(&store), expected, "{provider}");
            let memberships: i64 = store
                .db
                .query_row("SELECT COUNT(*) FROM event_sources", [], |r| r.get(0))
                .unwrap();
            assert_eq!(memberships, 6);
        }
    }
}
