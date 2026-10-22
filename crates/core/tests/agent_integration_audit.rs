use aieyes_core::{
    Engine, import,
    models::*,
    opencode,
    session_events::{self, Activity},
    sessions::{Phase, SessionMonitor},
    usage,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, time::SystemTime};

fn setup(root: &Path, provider: &str) -> (Engine, Source, Settings) {
    let engine = Engine::open(&root.join("app")).unwrap();
    let source = Source {
        id: provider.into(),
        provider: provider.into(),
        path: root.join("client").to_string_lossy().into(),
        ..Default::default()
    };
    fs::create_dir_all(&source.path).unwrap();
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
fn write(path: &Path, rows: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        rows.iter()
            .map(|r| r.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
}
fn events(engine: &Engine) -> Vec<UsageEvent> {
    engine
        .store
        .db
        .prepare("SELECT payload FROM events ORDER BY model")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
        .collect()
}
fn stamp(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap()
        .to_rfc3339()
}

#[test]
fn harness_unsupported_or_corrupt_sessions_do_not_hide_healthy_siblings() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "deepseek-harness");
    let client = Path::new(&source.path);
    let fixture = include_bytes!("fixtures/native-agents/harness.jsonl");
    for name in ["good", "future", "conflict", "broken"] {
        fs::create_dir_all(client.join(name)).unwrap();
    }
    fs::write(client.join("good/session.v4.jsonl"), fixture).unwrap();
    fs::write(client.join("future/session.v5.jsonl"), b"{}\n").unwrap();
    fs::write(client.join("conflict/session.v4.jsonl"), fixture).unwrap();
    fs::write(client.join("conflict/session.v4.jsonl.zstd"), b"broken").unwrap();
    fs::write(client.join("broken/session.v4.jsonl.zstd"), b"broken").unwrap();
    let first = import::scan(&engine.store, &settings, &source).unwrap();
    assert_eq!(first["partial"], true);
    assert_eq!(first["failedFiles"], 3);
    assert!(first["newEvents"].as_u64().unwrap() > 0);
    let before = events(&engine);
    fs::remove_file(client.join("future/session.v5.jsonl")).unwrap();
    fs::remove_file(client.join("conflict/session.v4.jsonl.zstd")).unwrap();
    fs::write(
        client.join("broken/session.v4.jsonl.zstd"),
        zstd::stream::encode_all(fixture.as_slice(), 1).unwrap(),
    )
    .unwrap();
    let repaired = import::scan(&engine.store, &settings, &source).unwrap();
    assert_eq!(repaired["partial"], false);
    assert_eq!(repaired["newEvents"], 0);
    assert_eq!(events(&engine).len(), before.len());
}

#[test]
fn invalid_usage_is_reported_and_repaired_without_losing_other_records() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "codebuddy");
    let path = Path::new(&source.path).join("bad.jsonl");
    let mut row = json!({"type":"model-usage","id":"bad","sessionId":"s","timestamp":1800000000000u64,"providerData":{"model":"fixture","usage":{"inputTokens":20,"outputTokens":4,"inputTokensDetails":["PRIVATE"]}}});
    write(&path, &[row.clone()]);
    let good = json!({"type":"model-usage","id":"good","sessionId":"s","timestamp":1800000000001u64,"providerData":{"model":"fixture","usage":{"inputTokens":8,"outputTokens":2}}});
    write(&Path::new(&source.path).join("good.jsonl"), &[good]);
    let first = import::scan(&engine.store, &settings, &source).unwrap();
    assert_eq!(first["partial"], true);
    assert_eq!(first["newEvents"], 1);
    assert!(!first.to_string().contains("PRIVATE"));
    row["providerData"]["usage"] =
        json!({"inputTokens":20,"outputTokens":4,"inputTokensDetails":{"cached_tokens":5}});
    write(&path, &[row]);
    let fixed = import::scan(&engine.store, &settings, &source).unwrap();
    assert_ne!(fixed["partial"], true);
    assert_eq!(fixed["newEvents"], 1);
    assert_eq!(events(&engine).len(), 2);
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
}

#[test]
fn codebuddy_child_transcript_and_hook_alias_merge_until_native_settlement() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "codebuddy");
    let at = now() * 1000;
    let parent = Path::new(&source.path).join("projects/project/root.jsonl");
    let child = parent
        .with_extension("")
        .join("subagents/agent-worker.jsonl");
    let hook = Path::new(&source.path).join(".aieyes/session-events/worker.jsonl");
    write(
        &parent,
        &[json!({"type":"message","role":"user","id":"prompt","sessionId":"root","timestamp":at})],
    );
    let mut rows = vec![
        json!({"type":"message","role":"user","id":"child-prompt","sessionId":"native-child","timestamp":at+1}),
        json!({"type":"reasoning","id":"think","parentId":"child-prompt","sessionId":"native-child","timestamp":at+5}),
    ];
    write(&child, &rows);
    let mut hooks = vec![
        json!({"kind":"PermissionRequest","session":"root:agent-worker","parentSession":"root","call":"ask","at":at+4}),
    ];
    write(&hook, &hooks);
    write(
        &hook.with_file_name("root.jsonl"),
        &[json!({"kind":"SessionStart","session":"root","at":at})],
    );
    let mut monitor = SessionMonitor::default();
    let waiting = monitor.read(&settings, SystemTime::now());
    assert_eq!(waiting.sessions.len(), 1);
    assert_eq!(waiting.sessions[0].phase, Phase::NeedsInput);
    assert_eq!(waiting.active_count(), 1);
    hooks.push(json!({"kind":"PostToolUse","session":"root:agent-worker","parentSession":"root","call":"ask","at":at+7}));
    write(&hook, &hooks);
    rows.push(json!({"type":"message","role":"assistant","id":"answer","parentId":"think","sessionId":"native-child","timestamp":at+8,"status":"completed","providerData":{"messageId":"model-step","rawUsage":{"input_tokens":10,"output_tokens":2}}}));
    write(&child, &rows);
    write(
        &hook.with_file_name("root.jsonl"),
        &[json!({"kind":"Stop","session":"root","at":at+9,"fullyIdle":true})],
    );
    let completed = monitor.read(&settings, SystemTime::now());
    assert_eq!(completed.sessions.len(), 1);
    assert_eq!(completed.sessions[0].phase, Phase::Complete);
    assert_eq!(completed.active_count(), 0);
    assert_eq!(aieyes_core::pets::project(&completed).behavior, "success");
}

#[test]
fn codebuddy_completed_commentary_does_not_close_the_turn_before_its_tool() {
    let mut activity = Activity::default();
    for row in [
        json!({"type":"message","role":"user","id":"prompt","sessionId":"s","timestamp":1}),
        json!({"type":"message","role":"assistant","status":"completed","id":"commentary","parentId":"prompt","timestamp":2,"providerData":{"messageId":"step"}}),
        json!({"type":"function_call","id":"tool","parentId":"commentary","timestamp":3,"callId":"bash","name":"Bash"}),
    ] {
        activity.apply(session_events::native("codebuddy", &row).unwrap());
    }
    assert_eq!(activity.phase, Some(Phase::Tool));
}

#[test]
fn codebuddy_native_cancellation_beats_generic_stop_and_allows_a_new_prompt() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "codebuddy");
    let at = now() * 1000;
    let log = Path::new(&source.path).join("projects/project/session.jsonl");
    let rows = vec![
        json!({"type":"message","role":"user","id":"prompt","sessionId":"s","timestamp":at}),
        json!({"type":"function_call_result","status":"incomplete","sessionId":"s","parentId":"prompt","timestamp":at+1,"providerData":{"skipRun":true,"error":{"message":"Interrupted by user","isRetryable":false}}}),
    ];
    write(&log, &rows);
    let hook = Path::new(&source.path).join(".aieyes/session-events/s.jsonl");
    let mut hooks = vec![
        json!({"kind":"UserPromptSubmit","session":"s","at":at}),
        json!({"kind":"Stop","session":"s","fullyIdle":true,"at":at+2}),
        json!({"kind":"PermissionRequest","session":"s","call":"late","at":at+3}),
    ];
    write(&hook, &hooks);
    let mut monitor = SessionMonitor::default();
    let stopped = monitor.read(&settings, SystemTime::now());
    assert_eq!(stopped.sessions.len(), 1);
    assert_eq!(stopped.sessions[0].phase, Phase::Interrupted);
    hooks.push(json!({"kind":"UserPromptSubmit","session":"s","at":at+4}));
    write(&hook, &hooks);
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).sessions[0].phase,
        Phase::Working
    );
    let event = session_events::hook(
        "codebuddy",
        "PostToolUseFailure",
        &json!({"session_id":"s","is_interrupt":true,"error":"PRIVATE"}),
        (at + 5) as u64,
    );
    assert_eq!(event.kind, "interrupted");
    assert!(!serde_json::to_string(&event).unwrap().contains("PRIVATE"));
    assert_eq!(session_events::native("codebuddy", &json!({"type":"message","role":"assistant","status":"incomplete","providerData":{"skipRun":true,"error":{"message":"Interrupted by user"}}})).unwrap().kind, "interrupted");
}

#[test]
fn kimi_usage_model_is_authoritative_and_cache_formats_are_complete() {
    let source = Source {
        id: "s".into(),
        provider: "kimi".into(),
        ..Default::default()
    };
    let mut state = ParseState {
        session_id: "session:main".into(),
        ..Default::default()
    };
    usage::parse(
        &source,
        &mut state,
        &json!({"type":"llm.request","model":"previous"}),
    );
    let event=usage::parse(&source,&mut state,&json!({"type":"usage.record","model":"actual","time":now()*1000,"usage":{"inputOther":10,"output":5,"inputCacheRead":20,"inputCacheCreation":30}})).unwrap();
    assert_eq!(event.model, "actual");
    assert_eq!(event.tokens.total(), 65);
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "codebuddy");
    let normalized = json!({"inputTokens":30,"outputTokens":5,"totalTokens":35,"inputTokensDetails":[{"cached_tokens":20}],"outputTokensDetails":[{"reasoning_tokens":2}]});
    let rows = [
        json!({"type":"model-usage","id":"normalized-raw","sessionId":"s","timestamp":now()*1000,"providerData":{"model":"test","rawUsage":normalized}}),
        json!({"type":"model-usage","id":"normalized","sessionId":"s","timestamp":now()*1000,"providerData":{"model":"test","usage":normalized}}),
        json!({"type":"model-usage","id":"anthropic","sessionId":"s","timestamp":now()*1000,"providerData":{"model":"test","rawUsage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":20,"cache_creation_input_tokens":30}}}),
    ];
    write(
        &Path::new(&source.path).join("projects/project/session.jsonl"),
        &rows,
    );
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        3
    );
    assert_eq!(
        events(&engine)
            .iter()
            .map(|e| e.tokens.total())
            .sum::<u64>(),
        135
    );
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
}

#[test]
fn mixed_opencode_tables_keep_independent_usage_compaction_and_idle() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "opencode");
    let path = Path::new(&source.path).join("opencode.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);").unwrap();
    let at = now() * 1000;
    let data = json!({"role":"assistant","modelID":"test","providerID":"test","model":{"id":"test","providerID":"test"},"time":{"created":at,"completed":at},"finish":"stop","tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}});
    db.execute(
        "INSERT INTO message VALUES('legacy','old-session',?1,?1,?2)",
        params![at, data.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_message VALUES('fresh','new-session','assistant',?1,?1,?2)",
        params![at, data.to_string()],
    )
    .unwrap();
    let compact = json!({"status":"completed","model":{"id":"test","providerID":"test"},"time":{"created":at+1},"tokens":data["tokens"],"summary":"PRIVATE"});
    db.execute(
        "INSERT INTO session_message VALUES('compact','new-session','compaction',?1,?1,?2)",
        params![at + 1, compact.to_string()],
    )
    .unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        3
    );
    db.execute(
        "INSERT INTO session_message VALUES('legacy','old-session','assistant',?1,?1,?2)",
        params![at, data.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_message VALUES('idle','new-session','idle',?1,?1,?2)",
        params![
            at + 2,
            json!({"outcome":"succeeded","time":{"created":at+2}}).to_string()
        ],
    )
    .unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        events(&engine)
            .iter()
            .map(|e| e.tokens.total())
            .sum::<u64>(),
        45
    );
    assert!(
        opencode::sessions(&path)
            .unwrap()
            .iter()
            .all(|r| r.2 == "complete")
    );
    let active = json!({"time":{"created":at+3},"content":[{"type":"tool","id":"call","name":"question","state":{"status":"running","input":{"prompt":"PRIVATE"}}}]});
    db.execute(
        "INSERT INTO session_message VALUES('tool','new-session','assistant',?1,?1,?2)",
        params![at + 3, active.to_string()],
    )
    .unwrap();
    assert_eq!(
        opencode::sessions(&path)
            .unwrap()
            .iter()
            .find(|r| r.0 == "new-session")
            .unwrap()
            .2,
        "PermissionRequest"
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
}

#[test]
fn replaced_file_and_repaired_bad_line_are_retried_without_losing_other_files() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "pi");
    // Use the same native separators as directory discovery and cursor keys.
    let path = Path::new(&source.path)
        .join("sessions")
        .join("project")
        .join("log.jsonl");
    let header = json!({"type":"session","id":"session","padding":"x".repeat(160)});
    let row = |id: &str| json!({"type":"message","id":id,"timestamp":stamp(now()*1000),"message":{"role":"assistant","model":"test","usage":{"input":10,"output":5}}});
    let first = row("first");
    let other = {
        let mut r = first.clone();
        r["id"] = json!("other");
        r
    };
    write(&path, &[header.clone(), first]);
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    let replacement = path.with_extension("new");
    write(&replacement, &[header.clone(), other.clone()]);
    fs::rename(replacement, &path).unwrap();
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    let repaired = row("repaired");
    fs::write(&path, format!("{header}\n{{broken-json}}\n{other}\n")).unwrap();
    write(
        &path.with_file_name("healthy.jsonl"),
        &[header.clone(), row("healthy")],
    );
    let scan = import::scan(&engine.store, &settings, &source).unwrap();
    assert_eq!(scan["newEvents"], 1);
    assert_eq!(scan["partial"], true);
    assert_eq!(
        engine
            .store
            .cursor(&source.id, &path.to_string_lossy())
            .unwrap()
            .expect("the scanner must save the discovered file's cursor")
            .1,
        format!("{header}\n").len() as u64
    );
    write(&path, &[header, repaired, other]);
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
    assert_eq!(events(&engine).len(), 4);
}

#[test]
fn large_last_session_record_survives_cold_start_and_append_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "pi");
    let path = Path::new(&source.path).join("sessions/project/log.jsonl");
    let at = now() * 1000;
    write(
        &path,
        &[
            json!({"type":"session","id":"large","timestamp":stamp(at)}),
            json!({"type":"message","id":"u","timestamp":stamp(at),"message":{"role":"user"}}),
            json!({"type":"message","id":"a","timestamp":stamp(at+1),"message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"x".repeat(320*1024)}]}}),
        ],
    );
    let mut monitor = SessionMonitor::default();
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).phase(),
        Some(Phase::Complete)
    );
    let row=json!({"type":"message","id":"next","timestamp":stamp(at+2),"message":{"role":"user","content":"x".repeat(600*1024)}}).to_string();
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    write!(f, "{}", &row[..400 * 1024]).unwrap();
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).phase(),
        Some(Phase::Complete)
    );
    writeln!(f, "{}", &row[400 * 1024..]).unwrap();
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).phase(),
        Some(Phase::Working)
    );
}

#[test]
fn touching_old_native_sessions_never_makes_them_recent() {
    for provider in ["pi", "omp", "kimi", "codebuddy"] {
        let root = tempfile::tempdir().unwrap();
        let (_, source, settings) = setup(root.path(), provider);
        let at = (now() - 172800) * 1000;
        let (path, rows) = match provider {
            "kimi" => (
                "sessions/old/agents/main/wire.jsonl",
                vec![
                    json!({"type":"turn.prompt","turnId":1,"time":at}),
                    json!({"type":"turn.ended","turnId":1,"time":at+1,"reason":"completed"}),
                ],
            ),
            "codebuddy" => (
                "projects/project/session.jsonl",
                vec![
                    json!({"type":"message","role":"user","id":"u","sessionId":"old","timestamp":at}),
                    json!({"type":"message","role":"assistant","id":"a","parentId":"u","sessionId":"old","timestamp":at+1,"status":"completed"}),
                ],
            ),
            _ => (
                "sessions/project/session.jsonl",
                vec![
                    json!({"type":"session","id":"old","timestamp":stamp(at)}),
                    json!({"type":"message","id":"u","timestamp":stamp(at),"message":{"role":"user"}}),
                    json!({"type":"message","id":"a","timestamp":stamp(at+1),"message":{"role":"assistant","stopReason":"stop"}}),
                ],
            ),
        };
        write(&Path::new(&source.path).join(path), &rows);
        assert!(
            SessionMonitor::default()
                .read(&settings, SystemTime::now())
                .sessions
                .is_empty(),
            "{provider}"
        );
    }
}

#[test]
fn kimi_interactions_and_grok_parallel_tools_keep_waiting_until_answered() {
    let mut activity = Activity::default();
    for row in [
        json!({"type":"turn.prompt","turnId":1,"time":1000}),
        json!({"type":"interaction.request","id":"one","time":1001}),
        json!({"type":"interaction.request","id":"two","time":1002}),
        json!({"type":"interaction.resolved","id":"one","time":1003}),
    ] {
        activity.apply(session_events::native("kimi", &row).unwrap());
    }
    assert_eq!(activity.phase, Some(Phase::NeedsInput));
    activity.apply(
        session_events::native(
            "kimi",
            &json!({"type":"interaction.resolved","id":"two","time":1004}),
        )
        .unwrap(),
    );
    assert_eq!(activity.phase, Some(Phase::Working));
    let mut activity = Activity::default();
    for row in [
        json!({"type":"turn_started","turn_number":1}),
        json!({"type":"tool_started","tool_name":"read"}),
        json!({"type":"tool_started","tool_name":"bash"}),
        json!({"type":"tool_completed","tool_name":"read"}),
    ] {
        activity.apply(session_events::native("grok", &row).unwrap());
    }
    assert_eq!(activity.phase, Some(Phase::Tool));
    activity.apply(
        session_events::native(
            "grok",
            &json!({"type":"permission_requested","request_id":"approval"}),
        )
        .unwrap(),
    );
    assert_eq!(activity.phase, Some(Phase::NeedsInput));
    activity.apply(
        session_events::native(
            "grok",
            &json!({"type":"permission_resolved","request_id":"approval"}),
        )
        .unwrap(),
    );
    assert_eq!(activity.phase, Some(Phase::Tool));
    activity.apply(
        session_events::native(
            "grok",
            &json!({"type":"tool_completed","tool_name":"bash","tool_call_id":"late-id"}),
        )
        .unwrap(),
    );
    assert_eq!(activity.phase, Some(Phase::Working));
}

#[test]
fn kimi_explicit_parent_metadata_merges_waiting_but_independent_sessions_stay_separate() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "kimi");
    let at = now() * 1000;
    let folder = Path::new(&source.path).join("sessions/project/session");
    write(
        &folder.join("agents/main/wire.jsonl"),
        &[json!({"type":"turn.prompt","turnId":1,"time":at})],
    );
    write(
        &folder.join("agents/worker/wire.jsonl"),
        &[
            json!({"type":"forked","agentId":"worker"}),
            json!({"type":"turn.prompt","turnId":1,"time":at+1}),
            json!({"type":"interaction.request","id":"approval","time":at+2}),
        ],
    );
    fs::write(folder.join("state.json"),json!({"id":"session","agents":{"main":{"type":"main"},"worker":{"type":"sub","parentAgentId":"main"}}}).to_string()).unwrap();
    write(
        &Path::new(&source.path).join("sessions/project/unrelated/agents/main/wire.jsonl"),
        &[json!({"type":"turn.prompt","turnId":1,"time":at})],
    );
    let snapshot = SessionMonitor::default().read(&settings, SystemTime::now());
    assert_eq!(snapshot.sessions.len(), 2);
    assert_eq!(snapshot.phase_counts().get("needsInput"), Some(&1));
    assert_eq!(snapshot.active_count(), 2);
}

#[test]
fn large_bad_history_does_not_block_another_file_or_lose_committed_progress() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "pi");
    let folder = Path::new(&source.path).join("sessions/project");
    let at = stamp(now() * 1000);
    let row = |id: &str| json!({"type":"message","id":id,"timestamp":at,"message":{"role":"assistant","model":"test","usage":{"input":10,"output":5}}});
    let path = folder.join("broken.jsonl");
    write(&path, &[row("before")]);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&vec![b'x'; 16 * 1024 * 1024 + 1]).unwrap();
    write(&folder.join("healthy.jsonl"), &[row("healthy")]);
    let result = import::scan(&engine.store, &settings, &source).unwrap();
    assert_eq!(result["partial"], true);
    assert_eq!(result["newEvents"], 2);
    assert!(result["issues"].to_string().contains("16 MiB"));
    write(&path, &[row("before"), row("recovered")]);
    assert_eq!(
        import::scan(&engine.store, &settings, &source).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
        0
    );
}

#[test]
fn hook_and_log_new_turns_use_milliseconds_in_the_same_second() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "codebuddy");
    let at = now() * 1000;
    let log = Path::new(&source.path).join("projects/project/session.jsonl");
    write(
        &log,
        &[
            json!({"type":"message","id":"old","sessionId":"session","role":"assistant","status":"completed","timestamp":at+1}),
        ],
    );
    let hook = Path::new(&source.path).join(".aieyes/session-events/session.jsonl");
    write(
        &hook,
        &[json!({"kind":"UserPromptSubmit","session":"session","turn":"new","at":at+2})],
    );
    let mut monitor = SessionMonitor::default();
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).phase(),
        Some(Phase::Working)
    );
    let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
    writeln!(file,"{}",json!({"type":"message","id":"new-answer","sessionId":"session","role":"assistant","status":"completed","timestamp":at+3,"providerData":{"conversationRequestId":"new"}})).unwrap();
    assert_eq!(
        monitor.read(&settings, SystemTime::now()).phase(),
        Some(Phase::Complete)
    );
}

#[test]
fn codex_child_waiting_is_merged_and_child_failure_does_not_fail_parent() {
    let root = tempfile::tempdir().unwrap();
    let (_, source, settings) = setup(root.path(), "codex");
    let at = now() * 1000;
    let main = Path::new(&source.path).join("sessions/main.jsonl");
    let child = main.with_file_name("child.jsonl");
    write(
        &main,
        &[
            json!({"type":"session_meta","payload":{"id":"parent","source":"cli"}}),
            json!({"type":"event_msg","timestamp":stamp(at),"payload":{"type":"task_started","turn_id":"main-turn"}}),
        ],
    );
    write(
        &child,
        &[
            json!({"type":"session_meta","payload":{"id":"child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}),
            json!({"type":"event_msg","timestamp":stamp(at+1),"payload":{"type":"task_started","turn_id":"child-turn"}}),
            json!({"type":"response_item","timestamp":stamp(at+2),"payload":{"type":"function_call","name":"request_user_input","call_id":"approval"}}),
        ],
    );
    let mut monitor = SessionMonitor::default();
    let snapshot = monitor.read(&settings, SystemTime::now());
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.phase(), Some(Phase::NeedsInput));
    assert_eq!(snapshot.active_count(), 1);
    let mut f = fs::OpenOptions::new().append(true).open(&child).unwrap();
    writeln!(f,"{}",json!({"type":"event_msg","timestamp":stamp(at+3),"payload":{"type":"error","turn_id":"child-turn"}})).unwrap();
    let snapshot = monitor.read(&settings, SystemTime::now());
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.phase(), Some(Phase::Working));
}

#[test]
fn parser_correction_replaces_categories_and_reprices_only_when_model_changes() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "codebuddy");
    let old_price = ModelPrice {
        id: "old".into(),
        input: Some(10.0),
        output: Some(10.0),
        cache_read: Some(10.0),
        ..Default::default()
    };
    let new_price = ModelPrice {
        id: "actual".into(),
        input: Some(2.0),
        output: Some(2.0),
        cache_read: Some(2.0),
        ..Default::default()
    };
    let mut event=usage::parse(&source,&mut ParseState::default(),&json!({"id":"r","sessionId":"s","timestamp":now(),"model":"old","tokens":{"input":30,"output":5,"cacheRead":20}})).unwrap();
    engine
        .store
        .put_event(&event, &[old_price], &settings)
        .unwrap();
    event.model = "actual".into();
    event.tokens.input = 10;
    event.usage_version = 10;
    engine
        .store
        .put_event(&event, std::slice::from_ref(&new_price), &settings)
        .unwrap();
    let corrected = events(&engine).remove(0);
    assert_eq!(corrected.tokens.total(), 35);
    assert_eq!(corrected.model, "actual");
    let frozen: String = engine
        .store
        .db
        .query_row("SELECT price FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<ModelPrice>(&frozen).unwrap().id,
        "actual"
    );
    event.tokens.input = 30;
    event.model = "old".into();
    event.usage_version = 0;
    engine.store.put_event(&event, &[], &settings).unwrap();
    assert_eq!(events(&engine)[0].model, "actual");
    assert_eq!(events(&engine)[0].tokens.total(), 35);
    event.model = "unknown-model".into();
    event.usage_version = 11;
    engine
        .store
        .put_event(&event, &[new_price], &settings)
        .unwrap();
    assert!(
        engine
            .store
            .db
            .query_row(
                "SELECT price IS NULL AND priced_tokens=0 FROM events",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}
