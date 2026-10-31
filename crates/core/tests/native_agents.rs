use aieyes_core::{
    Engine, import,
    models::*,
    native_agents, session_events,
    sessions::{Phase, SessionMonitor},
    usage,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, time::SystemTime};

const KIMI: &str = include_str!("fixtures/native-agents/kimi.jsonl");
const BUDDY: &str = include_str!("fixtures/native-agents/codebuddy.jsonl");
const GROK: &str = include_str!("fixtures/native-agents/grok-updates.jsonl");
const DSH: &str = include_str!("fixtures/native-agents/harness.jsonl");

fn setup(root: &Path, provider: &str) -> (Engine, Source, Settings) {
    let engine = Engine::open(&root.join("app")).unwrap();
    let source = Source {
        id: provider.into(),
        provider: provider.into(),
        path: root.join("client").to_string_lossy().into(),
        ..Default::default()
    };
    engine
        .store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let settings = engine.store.settings().unwrap();
    (engine, source, settings)
}

#[test]
fn real_client_samples_import_correctly_repeat_and_rescan_without_content() {
    for (provider, fixture, relative, input, output) in [
        (
            "pi",
            include_str!("fixtures/native-agents/pi.jsonl"),
            "sessions/project/session.jsonl",
            90,
            37,
        ),
        (
            "omp",
            include_str!("fixtures/native-agents/omp.jsonl"),
            "sessions/project/session.jsonl",
            261,
            32,
        ),
        (
            "kimi",
            KIMI,
            "sessions/session-one/agents/main/wire.jsonl",
            20342,
            48,
        ),
        (
            "codebuddy",
            BUDDY,
            "projects/project/session.jsonl",
            6380,
            48,
        ),
        (
            "grok",
            GROK,
            "sessions/project/session/updates.jsonl",
            11422,
            36,
        ),
        (
            "deepseek-harness",
            DSH,
            "sessions/project/session/session.v4.jsonl.zstd",
            5621,
            108,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(temp.path(), provider);
        let path = temp.path().join("client").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = if provider == "deepseek-harness" {
            zstd::stream::encode_all(fixture.as_bytes(), 1).unwrap()
        } else {
            fixture.as_bytes().to_vec()
        };
        fs::write(&path, &bytes).unwrap();
        if provider == "grok" {
            fs::write(
                path.with_file_name("events.jsonl"),
                include_str!("fixtures/native-agents/grok-events.jsonl"),
            )
            .unwrap();
        }
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            1,
            "{provider}"
        );
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
        assert_eq!(
            import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
        let raw: String = engine
            .store
            .db
            .query_row("SELECT payload FROM events", [], |r| r.get(0))
            .unwrap();
        let event: UsageEvent = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            event.model,
            if provider == "kimi" {
                "qwen-test"
            } else {
                "Qwen3.8-27B"
            }
        );
        assert_eq!((event.tokens.input, event.tokens.output), (input, output));
        assert_eq!(event.billing.category, "unknown");
        assert!(!raw.contains("content"));
        assert_eq!(
            fs::read(path).unwrap(),
            bytes,
            "client file must remain unchanged"
        );
        let snapshot = SessionMonitor::default().read(
            &settings,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(event.timestamp as u64 + 1),
        );
        assert_eq!(snapshot.sessions.len(), 1, "{provider}: {snapshot:?}");
        assert_eq!(snapshot.phase(), Some(Phase::Complete), "{provider}");
        assert!(!snapshot.unavailable, "{provider}: {snapshot:?}");
    }
}

#[test]
fn pi_family_forks_share_billing_identity_and_title_updates_do_not_replay_history() {
    for (provider, fixture) in [
        ("pi", include_str!("fixtures/native-agents/pi.jsonl")),
        ("omp", include_str!("fixtures/native-agents/omp.jsonl")),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(temp.path(), provider);
        let path = temp.path().join("client/sessions/project/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, fixture).unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            1
        );
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["readBytes"],
            0
        );
        if provider == "omp" {
            let changed = fixture.replacen("PRIVATE_TITLE", "CHANGED_TITLE", 1);
            assert_eq!(changed.len(), fixture.len());
            fs::write(&path, changed).unwrap();
            assert_eq!(
                import::scan(&engine.store, &settings, &source).unwrap()["readBytes"],
                0
            );
        }
        let fork = fixture
            .lines()
            .map(|line| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                if row["type"] == "session" {
                    row["id"] = json!("new-fork-session");
                    row["parentSession"] = json!("PRIVATE_PARENT_PATH");
                }
                row.to_string() + "\n"
            })
            .collect::<String>();
        let child = path.with_file_name("fork.jsonl");
        fs::write(&child, fork).unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
        let usage = json!({"type":if provider=="pi" {"usage"} else {"model_usage"},"id":"new-side-request","timestamp":"2026-10-18T22:00:00.000Z","provider":"different-backend","model":"auxiliary","usage":{"input":11,"output":7,"cacheRead":5,"cacheWrite":3,"reasoning":4}});
        let mut file = fs::OpenOptions::new().append(true).open(&child).unwrap();
        write!(file, "{usage}").unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
        writeln!(file).unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            1
        );
        assert_eq!(
            import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
        let raw: String = engine
            .store
            .db
            .query_row(
                "SELECT payload FROM events WHERE model='auxiliary'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: UsageEvent = serde_json::from_str(&raw).unwrap();
        assert_eq!(event.tokens.total(), 26);
        assert_eq!(event.tokens.reasoning, 4);
        assert_eq!(event.billing.category, "unknown");
        assert!(!raw.contains("PRIVATE_"));
    }
}

#[test]
fn pi_legacy_migration_preserves_prior_account_and_frozen_charge() {
    for provider in ["pi", "omp"] {
        let temp = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(temp.path(), provider);
        engine
            .store
            .save_prices(&[ModelPrice {
                id: "fixture".into(),
                input: Some(1.0),
                output: Some(2.0),
                fetched_at: 1,
                ..Default::default()
            }])
            .unwrap();
        let path = temp.path().join("client/sessions/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let header = json!({"type":"session","id":"legacy","version":1});
        let usage = json!({"type":"message","timestamp":"2026-10-18T22:00:00.001Z","message":{"role":"assistant","model":"fixture","usage":{"input":10,"output":2}}});
        fs::write(
            &path,
            format!("{header}\n{{\"type\":\"model_change\"}}\n{usage}\n"),
        )
        .unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            1
        );
        let frozen: (Option<String>, f64) = engine
            .store
            .db
            .query_row("SELECT price,cost FROM events", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(frozen.1 > 0.0);
        engine
            .store
            .save_prices(&[ModelPrice {
                id: "fixture".into(),
                input: Some(100.0),
                output: Some(200.0),
                fetched_at: 2,
                ..Default::default()
            }])
            .unwrap();
        let mut changed = settings.clone();
        changed.sources[0].account_id = "new-account".into();
        changed.accounts.push(Account {
            id: "new-account".into(),
            name: "Test account".into(),
            provider: provider.into(),
            ..Default::default()
        });
        engine.store.save_settings(&changed).unwrap();
        let changed = engine.store.settings().unwrap();
        let source = changed.sources[0].clone();
        let mut migrated = usage;
        migrated["id"] = json!("native-entry-id");
        fs::write(
            &path,
            format!(
                "{}\n{migrated}\n",
                json!({"type":"session","id":"legacy","version":3})
            ),
        )
        .unwrap();
        assert_eq!(
            import::rescan(&engine.store, &changed, &source).unwrap()["newEvents"],
            0
        );
        let rows: Vec<(String, Option<String>, f64, String)> = engine
            .store
            .db
            .prepare("SELECT account_id,price,cost,payload FROM events")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].0.is_empty(),
            "Historical usage retains its original account"
        );
        assert_eq!((rows[0].1.clone(), rows[0].2), frozen);
        assert!(rows[0].3.contains("entry:message:native-entry-id:"));
        assert_eq!(
            import::rescan(&engine.store, &changed, &source).unwrap()["newEvents"],
            0
        );
    }
}

#[test]
fn pi_lifecycle_keeps_parallel_tools_active_and_distinguishes_failed_turns() {
    let mut activity = session_events::Activity::default();
    let mut apply = |row: Value| {
        for e in session_events::pi(&row) {
            activity.apply(e);
        }
        activity.phase
    };
    let stamp = "2026-10-18T21:00:00.000Z";
    assert_eq!(
        apply(
            json!({"type":"message","id":"prompt","timestamp":stamp,"message":{"role":"user","content":"PRIVATE"}})
        ),
        Some(Phase::Working)
    );
    assert_eq!(
        apply(
            json!({"type":"message","id":"assistant","parentId":"prompt","timestamp":stamp,"message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"one","name":"bash","arguments":{"secret":"PRIVATE"}},{"type":"toolCall","id":"two","name":"read"}]}})
        ),
        Some(Phase::Tool)
    );
    assert_eq!(
        apply(
            json!({"type":"message","id":"first-result","parentId":"assistant","timestamp":stamp,"message":{"role":"toolResult","toolCallId":"one"}})
        ),
        Some(Phase::Tool)
    );
    apply(
        json!({"type":"message","id":"second-result","parentId":"first-result","timestamp":stamp,"message":{"role":"toolResult","toolCallId":"two"}}),
    );
    assert_eq!(
        apply(
            json!({"type":"message","id":"final","parentId":"second-result","timestamp":stamp,"message":{"role":"assistant","stopReason":"stop"}})
        ),
        Some(Phase::Complete)
    );
    apply(json!({"type":"message","id":"next","timestamp":stamp,"message":{"role":"user"}}));
    assert_eq!(
        apply(
            json!({"type":"message","id":"error","parentId":"next","timestamp":stamp,"message":{"role":"assistant","stopReason":"error"}})
        ),
        Some(Phase::Failed)
    );
}

#[test]
fn harness_compressed_append_reads_only_new_frames_and_retries_torn_frames() {
    for history in [10, 3000] {
        let temp = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(temp.path(), "deepseek-harness");
        let path = temp
            .path()
            .join("client/sessions/project/native/session.v4.jsonl.zstd");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let message = |seq: u64| {
            json!({"type":"assistant/message","seq":seq,"time":1792350000000u64+seq,"data":{"turn":seq,"step":1,"message":{"source":{"kind":"model","model":"fixture"},"content":"PRIVATE_PROMPT"},"usage":{"inputTokens":10,"outputTokens":2}}}).to_string()+"\n"
        };
        let mut content =
            json!({"type":"session","version":4,"id":"native","isSeeded":false}).to_string() + "\n";
        for i in 1..=history {
            content.push_str(&message(i));
        }
        let encoded = zstd::stream::encode_all(content.as_bytes(), 1).unwrap();
        fs::write(&path, &encoded).unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
            history
        );
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["readBytes"],
            0
        );
        let added = zstd::stream::encode_all(message(history + 1).as_bytes(), 1).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&added[..added.len() - 1]).unwrap();
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["partial"],
            true
        );
        file.write_all(&added[added.len() - 1..]).unwrap();
        let delta = import::scan(&engine.store, &settings, &source).unwrap();
        assert_eq!(delta["newEvents"], 1);
        assert_eq!(delta["readBytes"], added.len());
        assert_eq!(
            import::scan(&engine.store, &settings, &source).unwrap()["readBytes"],
            0
        );
        assert_eq!(
            import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
    }
}

#[test]
fn kimi_incremental_tails_fork_inheritance_and_agent_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "kimi");
    let path = temp
        .path()
        .join("client/sessions/child/agents/main/wire.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!("{KIMI}{{\"type\":\"forked\",\"agentId\":\"main\"}}\n"),
    )
    .unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    let row = json!({"type":"usage.record","model":"Qwen3.8-27B","time":1792296701000i64,"usage":{"inputOther":10,"output":3,"inputCacheRead":5,"inputCacheCreation":2}}).to_string();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    write!(file, "{row}").unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    writeln!(file).unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_ne!(
        native_agents::session_id(&path),
        native_agents::session_id(Path::new("sessions/other/agents/main/wire.jsonl"))
    );
    assert_ne!(
        native_agents::session_id(&path),
        native_agents::session_id(Path::new("sessions/child/agents/agent-1/wire.jsonl"))
    );
}

#[test]
fn token_units_reject_inconsistent_values_and_unfinished_codebuddy_messages() {
    let harness = Source {
        provider: "deepseek-harness".into(),
        ..Default::default()
    };
    let original: Value = serde_json::from_str(
        DSH.lines()
            .find(|line| line.contains("assistant/message"))
            .unwrap(),
    )
    .unwrap();
    for key in ["cacheReadTokens", "cacheWriteTokens", "reasoningTokens"] {
        for value in [json!(-1), json!("2"), json!(true)] {
            let mut invalid = original.clone();
            invalid["data"]["usage"][key] = value;
            assert!(
                native_agents::parse_dsh(&harness, &mut ParseState::default(), &invalid).is_none()
            );
        }
    }
    let source = Source {
        provider: "codebuddy".into(),
        ..Default::default()
    };
    let mut row: Value = serde_json::from_str(BUDDY.lines().last().unwrap()).unwrap();
    row["providerData"]["rawUsage"] = json!({"prompt_tokens":100,"completion_tokens":25,"prompt_tokens_details":{"cached_tokens":60},"completion_tokens_details":{"reasoning_tokens":5}});
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.cache_read,
            event.tokens.total(),
            event.tokens.reasoning
        ),
        (40, 60, 125, 5)
    );
    row["status"] = json!("in_progress");
    assert!(usage::parse(&source, &mut ParseState::default(), &row).is_none());
    row["status"] = json!("completed");
    row["providerData"]["rawUsage"]["prompt_tokens"] = json!(50);
    assert!(usage::parse(&source, &mut ParseState::default(), &row).is_none());
    row["providerData"]["rawUsage"]["prompt_tokens"] = json!(u64::MAX);
    assert!(usage::parse(&source, &mut ParseState::default(), &row).is_none());
}

#[test]
fn harness_seed_prefix_compression_errors_and_future_versions_are_explicit() {
    let source = Source {
        provider: "deepseek-harness".into(),
        ..Default::default()
    };
    let rows: Vec<Value> = DSH
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let mut state = ParseState::default();
    let mut header = rows[0].clone();
    header["isSeeded"] = json!(true);
    native_agents::parse_dsh(&source, &mut state, &header);
    let assistant = rows
        .iter()
        .find(|r| r["type"] == "assistant/message")
        .unwrap();
    let inherited = native_agents::parse_dsh(&source, &mut state, assistant).unwrap();
    assert!(inherited.inheritance.unwrap().inherited);
    assert!(inherited.tokens.total() > 0);
    native_agents::parse_dsh(&source, &mut state, &json!({"type":"session/end-seed"}));
    assert!(native_agents::parse_dsh(&source, &mut state, assistant).is_some());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.v4.jsonl.zstd");
    let mut bytes = zstd::stream::encode_all(DSH.as_bytes(), 1).unwrap();
    bytes.truncate(bytes.len() - 10);
    fs::write(&path, bytes).unwrap();
    assert!(native_agents::read_dsh(&path, |_| Ok(())).is_err());
    fs::write(
        &path,
        zstd::stream::encode_all(b"{\"type\":\"session\",\"version\":5}\n".as_slice(), 1).unwrap(),
    )
    .unwrap();
    assert!(native_agents::read_dsh(&path, |_| Ok(())).is_err());
}

#[test]
fn harness_explicit_file_resolves_conflicting_encodings() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, mut source, mut settings) = setup(temp.path(), "deepseek-harness");
    let folder = temp.path().join("client/sessions/project/session");
    fs::create_dir_all(&folder).unwrap();
    let plain = folder.join("session.v4.jsonl");
    fs::write(&plain, DSH).unwrap();
    fs::write(
        folder.join("session.v4.jsonl.zstd"),
        zstd::stream::encode_all(DSH.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["partial"],
        true
    );
    source.path = plain.to_string_lossy().into_owned();
    settings.sources = vec![source.clone()];
    engine.store.save_settings(&settings).unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
}

#[test]
fn lifecycle_uses_explicit_failure_wait_and_cancellation_boundaries() {
    for (provider, row, phase) in [
        (
            "kimi",
            json!({"type":"turn.ended","reason":"failed"}),
            Phase::Failed,
        ),
        (
            "kimi",
            json!({"type":"turn.ended","reason":"cancelled"}),
            Phase::Interrupted,
        ),
        (
            "kimi",
            json!({"type":"turn.ended","reason":"blocked"}),
            Phase::NeedsInput,
        ),
        (
            "deepseek-harness",
            json!({"type":"turn/end","data":{"reason":{"kind":"error"}}}),
            Phase::Failed,
        ),
        (
            "deepseek-harness",
            json!({"type":"turn/end","data":{"reason":{"kind":"aborted"}}}),
            Phase::Interrupted,
        ),
        (
            "deepseek-harness",
            json!({"type":"turn/end","data":{"reason":{"kind":"blocked"}}}),
            Phase::NeedsInput,
        ),
    ] {
        let mut activity = session_events::Activity::default();
        activity.apply(session_events::Event {
            kind: "start".into(),
            ..Default::default()
        });
        activity.apply(session_events::native(provider, &row).unwrap());
        assert_eq!(activity.phase, Some(phase), "{provider}: {row}");
    }
}

#[test]
fn zcode_durable_request_statistics_exclude_raw_payloads_and_handle_cache_units() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "zcode");
    let path = temp.path().join("client/cli/db/db.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE model_usage(id TEXT,session_id TEXT,turn_id TEXT,model_id TEXT,completed_at INTEGER,input_tokens INTEGER,output_tokens INTEGER,cache_read_input_tokens INTEGER,cache_creation_input_tokens INTEGER,reasoning_tokens INTEGER,provider_total_tokens INTEGER,computed_total_tokens INTEGER,status TEXT,raw_usage_json TEXT);").unwrap();
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-agents/zcode-usage.json")).unwrap();
    let sql = "INSERT INTO model_usage VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'PRIVATE_RESPONSE')";
    db.execute(
        sql,
        rusqlite::params![
            data["id"].as_str(),
            data["session"].as_str(),
            data["turn"].as_str(),
            data["model"].as_str(),
            data["completed"].as_i64(),
            data["input"].as_i64(),
            data["output"].as_i64(),
            data["read"].as_i64(),
            data["write"].as_i64(),
            data["reasoning"].as_i64(),
            data["providerTotal"].as_i64(),
            data["computedTotal"].as_i64(),
            "completed"
        ],
    )
    .unwrap();
    db.execute("INSERT INTO model_usage SELECT 'pending',session_id,turn_id,model_id,completed_at,input_tokens,output_tokens,cache_read_input_tokens,cache_creation_input_tokens,reasoning_tokens,provider_total_tokens,computed_total_tokens,'running',raw_usage_json FROM model_usage",[]).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(before, fs::read(&path).unwrap());
    let raw: String = engine
        .store
        .db
        .query_row("SELECT payload FROM events", [], |r| r.get(0))
        .unwrap();
    assert!(!raw.contains("PRIVATE_RESPONSE"));
    assert_eq!(
        serde_json::from_str::<UsageEvent>(&raw)
            .unwrap()
            .tokens
            .total(),
        15210
    );
    let mut cached = data;
    cached["input"] = json!(100);
    cached["output"] = json!(25);
    cached["read"] = json!(60);
    cached["write"] = json!(10);
    cached["providerTotal"] = json!(125);
    let event = aieyes_core::zcode::event(&source, &cached).unwrap();
    assert_eq!((event.tokens.input, event.tokens.total()), (30, 125));
    cached["providerTotal"] = json!(195);
    let event = aieyes_core::zcode::event(&source, &cached).unwrap();
    assert_eq!((event.tokens.input, event.tokens.total()), (100, 195));
    cached["providerTotal"] = json!(2);
    assert!(aieyes_core::zcode::event(&source, &cached).is_err());
}

#[test]
fn qoder_real_zero_counters_do_not_become_estimated_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "qoder");
    let path = temp.path().join("client/projects/project/session.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let fixture = include_str!("fixtures/native-agents/qoder.jsonl");
    fs::write(&path, fixture).unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    let dashboard = engine
        .store
        .dashboard(&Filter {
            date_range: Some(DateRange::All),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(dashboard.summary.total, 0);
    assert_eq!(dashboard.summary.cost, 0.0);
    let credits = &dashboard.summary.native_credits["qoder"];
    assert_eq!(credits.requests, 1);
    assert_eq!(credits.credits, 0.031734285714285715);
    assert_eq!(credits.non_billable, credits.credits);
    assert_eq!(credits.non_billable_requests, 1);
    assert_eq!(credits.original_credits_requests, 1);
    assert_eq!(credits.original_credits, credits.credits);
    assert_eq!(credits.billable, 0.0);
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        SessionMonitor::default()
            .read(
                &settings,
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(
                        serde_json::from_str::<Value>(fixture.lines().last().unwrap())
                            .ok()
                            .and_then(|r| timestamp(&r["timestamp"]))
                            .unwrap() as u64
                            + 1
                    )
            )
            .phase(),
        Some(Phase::Complete)
    );
    let mut row: Value = serde_json::from_str(fixture.lines().last().unwrap()).unwrap();
    assert_eq!(row["message"]["usage"]["billable"], false);
    row["message"]["usage"]["input_tokens"] = json!(11);
    row["message"]["usage"]["output_tokens"] = json!(5);
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    assert_eq!(event.tokens.total(), 16);
    let normalized = json!({"id":"export","sessionId":"export-session","timestamp":now(),"model":"fixture","tokens":{"input":4,"output":2}});
    assert_eq!(
        usage::parse(&source, &mut ParseState::default(), &normalized)
            .unwrap()
            .tokens
            .total(),
        6
    );
}

#[test]
fn qoder_parser_upgrade_replaces_inclusive_input_without_losing_cache() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "qoder");
    let row = json!({
        "type":"assistant","timestamp":"2026-10-18T04:29:24.853Z","sessionId":"session",
        "message":{"id":"request","model":"qfmodel","usage":{
            "input_tokens":100,"output_tokens":5,"cache_read_input_tokens":40,
            "cache_creation_input_tokens":0
        }},
    });
    let mut old = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    old.tokens.input += old.tokens.cache_read + old.tokens.cache_write;
    old.raw_tokens = None;
    engine.store.put_event(&old, &[], &settings).unwrap();
    let new = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    engine.store.put_event(&new, &[], &settings).unwrap();
    let raw: String = engine
        .store
        .db
        .query_row("SELECT payload FROM events", [], |r| r.get(0))
        .unwrap();
    let event: UsageEvent = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.cache_read,
            event.tokens.total()
        ),
        (60, 40, 105)
    );
    assert!(event.raw_tokens.is_some());
}

#[test]
fn qoder_request_credits_keep_discount_metadata_and_reject_errors_or_invalid_counters() {
    let source = Source {
        provider: "qoder".into(),
        ..Default::default()
    };
    let mut row: Value = serde_json::from_str(
        include_str!("fixtures/native-agents/qoder.jsonl")
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    row["message"]["usage"]["credits"] = json!(0.25);
    row["message"]["usage"]["original_credits"] = json!(0.5);
    let event = usage::parse(&source, &mut ParseState::default(), &row).unwrap();
    let credits = event.native_credits.unwrap();
    assert_eq!(
        (credits.credits, credits.original_credits, credits.billable),
        (0.25, Some(0.5), Some(false))
    );
    assert_eq!(event.billing.category, "unknown");
    for invalid in [json!(-1), json!("0.25"), Value::Null] {
        row["message"]["usage"]["credits"] = invalid;
        assert!(usage::parse(&source, &mut ParseState::default(), &row).is_none());
    }
    row["message"]["usage"]["credits"] = json!(0);
    assert!(usage::parse(&source, &mut ParseState::default(), &row).is_some());
    row["isApiErrorMessage"] = json!(true);
    assert!(usage::parse(&source, &mut ParseState::default(), &row).is_none());
    row["isApiErrorMessage"] = json!(false);
    assert!(
        usage::parse(
            &Source {
                provider: "claude".into(),
                ..Default::default()
            },
            &mut ParseState::default(),
            &row
        )
        .is_none()
    );
    let normalized = json!({"id":"export","sessionId":"s","model":"qfmodel","timestamp":now(),"tokens":{},"nativeCredits":{"credits":0.25,"originalCredits":0.5,"billable":false}});
    assert_eq!(
        usage::parse(&source, &mut ParseState::default(), &normalized)
            .unwrap()
            .native_credits
            .unwrap()
            .credits,
        0.25
    );
}

#[test]
fn qoder_native_api_errors_end_the_session_without_retaining_error_details() {
    let temp = tempfile::tempdir().unwrap();
    let (_, _, settings) = setup(temp.path(), "qoder");
    let path = temp.path().join("client/projects/project/session.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let row = json!({"type":"assistant","uuid":"failed-response","sessionId":"failed-session","timestamp":chrono::Utc::now().to_rfc3339(),"isApiErrorMessage":true,"error":"PRIVATE_ERROR","errorDetails":"PRIVATE_STACK","message":{"role":"assistant","stop_reason":"stop_sequence","content":[{"type":"text","text":"PRIVATE_BODY"}]}});
    fs::write(&path, format!("{row}\n")).unwrap();
    let snapshot = SessionMonitor::default().read(&settings, SystemTime::now());
    assert_eq!(snapshot.phase(), Some(Phase::Failed));
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains("PRIVATE_")
    );
    let mut ordinary = row;
    ordinary["isApiErrorMessage"] = json!(false);
    assert!(session_events::claude(&ordinary).is_empty());
}

#[test]
fn codebuddy_tool_request_usage_is_counted_once_for_parallel_tool_calls() {
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "codebuddy");
    let fixture = include_str!("fixtures/native-agents/codebuddy-tool.jsonl");
    let mut rows: Vec<Value> = fixture
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let mut parallel = rows
        .iter()
        .find(|v| v["type"] == "function_call")
        .unwrap()
        .clone();
    parallel["id"] = json!("parallel-row");
    parallel["callId"] = json!("parallel-call");
    rows.push(parallel);
    let path = temp.path().join("client/projects/project/session.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        rows.iter()
            .map(|r| r.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        2
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    let events = engine
        .store
        .db
        .prepare("SELECT payload FROM events")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| serde_json::from_str::<UsageEvent>(&r.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.iter().map(|e| e.tokens.input).sum::<u64>(), 14458);
    assert_eq!(events.iter().map(|e| e.tokens.output).sum::<u64>(), 165);
    let result = rows
        .iter()
        .find(|v| v["type"] == "function_call_result")
        .unwrap();
    let event = session_events::native("codebuddy", result).unwrap();
    assert_eq!(event.kind, "output");
    assert!(!event.call.is_empty());
}

#[test]
fn copilot_cli_session_state_imports_usage_and_prompt_mode_is_headless() {
    // Recorded with Copilot CLI 1.0.105 `copilot -p` against a BYOK endpoint; text trimmed.
    let fixture = include_str!("fixtures/native-agents/copilot-events.jsonl");
    let temp = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(temp.path(), "copilot");
    let path = temp
        .path()
        .join("client/session-state/9b009a56-0bf5-4830-8d65-8da8fe5fc299/events.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, fixture).unwrap();
    // Sibling state that is not a transcript is never imported.
    fs::write(path.with_file_name("workspace.yaml"), "id: fixture\n").unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    let raw: String = engine
        .store
        .db
        .query_row("SELECT payload FROM events", [], |r| r.get(0))
        .unwrap();
    let event: UsageEvent = serde_json::from_str(&raw).unwrap();
    assert_eq!(event.model, "Qwen3.8-27B");
    assert_eq!((event.tokens.input, event.tokens.output), (13654, 32));
    assert!(!raw.contains("trimmed"));

    let rows: Vec<Value> = fixture
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let events: Vec<_> = rows
        .iter()
        .filter_map(|r| session_events::native("copilot", r))
        .collect();
    let kinds: Vec<_> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["identity", "start", "identity", "thinking", "complete"]
    );
    assert_eq!(events[0].session, "9b009a56-0bf5-4830-8d65-8da8fe5fc299");
    assert_eq!(events[2].classification, session_events::HEADLESS);
    let tool = session_events::native(
        "copilot",
        &json!({"type":"tool.execution_start","timestamp":"2026-10-29T22:56:40.000Z","data":{"toolCallId":"call-1","toolName":"bash","turnId":"0"}}),
    )
    .unwrap();
    assert_eq!(
        (tool.kind.as_str(), tool.call.as_str(), tool.tool.as_str()),
        ("tool", "call-1", "bash")
    );
    let pending = session_events::native(
        "copilot",
        &json!({"type":"assistant.message","data":{"toolRequests":[{"toolCallId":"call-1"}]}}),
    );
    assert!(
        pending.is_none(),
        "a reply that requests tools does not end the turn"
    );
}
