use aieyes_core::{
    Engine, import,
    models::*,
    sessions::{Phase, SessionMonitor},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, SystemTime},
};

fn setup(root: &Path, provider: &str) -> (Engine, Source, Settings) {
    let data = root.join(provider);
    fs::create_dir_all(&data).unwrap();
    let engine = Engine::open(&root.join("db")).unwrap();
    let source = Source {
        id: format!("{provider}-src"),
        account_id: format!("{provider}-acc"),
        name: provider.into(),
        provider: provider.into(),
        path: data.to_string_lossy().into(),
        enabled: true,
        ..Default::default()
    };
    let settings = Settings {
        accounts: vec![Account {
            id: source.account_id.clone(),
            name: provider.into(),
            provider: provider.into(),
            ..Default::default()
        }],
        sources: vec![source.clone()],
        ..Default::default()
    };
    engine.store.save_settings(&settings).unwrap();
    (engine, source, settings)
}

fn rfc3339(epoch_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(epoch_ms)
        .unwrap()
        .to_rfc3339()
}

fn write_lines(path: &Path, lines: &[String]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::File::create(path).unwrap();
    for line in lines {
        file.write_all(line.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
    }
}

#[test]
fn codex_oversized_response_item_does_not_block_subsequent_usage_or_timing() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "codex");
    let t0 = now() * 1000 - 60_000;
    let big_output = "A".repeat(17 * 1024 * 1024);
    let big_line = format!(
        "{{\"timestamp\":\"{}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"c1\",\"output\":\"{}\",\"encrypted_content\":\"{}\"}}}}",
        rfc3339(t0 + 10_000),
        big_output,
        "E".repeat(1024)
    );
    assert!(big_line.len() > 16 * 1024 * 1024);

    let file = Path::new(&source.path)
        .join("sessions/2026/11/07/rollout-2026-11-07T14-40-21-01a1013b-940b-7973-ad67-eb128b853dfc_01a1b4bc-9a01-70f0-8d48-09df07abec6f.jsonl");
    write_lines(
        &file,
        &[
            json!({"timestamp": rfc3339(t0), "type": "session_meta", "payload": {"id": "01a1013b-940b-7973-ad67-eb128b853dfc", "model_provider": "openai", "originator": "codex_cli_rs"}}).to_string(),
            json!({"timestamp": rfc3339(t0 + 1_000), "type": "turn_context", "payload": {"turn_id": "turn-1", "model": "gpt-5.4"}}).to_string(),
            json!({"timestamp": rfc3339(t0 + 2_000), "type": "event_msg", "payload": {"type": "task_started", "turn_id": "turn-1"}}).to_string(),
            big_line,
            json!({"timestamp": rfc3339(t0 + 14_000), "type": "response_item", "payload": {"type": "message", "role": "assistant"}}).to_string(),
            json!({
                "timestamp": rfc3339(t0 + 15_000),
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {"input_tokens": 1000, "cached_input_tokens": 200, "output_tokens": 300},
                        "last_token_usage": {"input_tokens": 1000, "cached_input_tokens": 200, "output_tokens": 300}
                    }
                }
            }).to_string(),
        ],
    );

    let first = import::scan(&engine.store, &settings, &source).unwrap();
    assert!(first["partial"].is_null());
    assert!(first["issues"].is_null());
    assert_eq!(first["newEvents"], 1);
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        1300
    );

    // Cursor reached EOF; subsequent incremental scan reads 0 bytes and stays healthy.
    let second = import::scan(&engine.store, &settings, &source).unwrap();
    assert!(second["partial"].is_null());
    assert_eq!(second["readBytes"], 0);
    assert_eq!(second["newEvents"], 0);

    // Full rescan is idempotent and error-free.
    let rescanned = import::rescan(&engine.store, &settings, &source).unwrap();
    assert!(rescanned["partial"].is_null());
    assert_eq!(rescanned["newEvents"], 0);
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        1300
    );
}

#[test]
fn codex_inherited_history_resolves_parent_with_oversized_line_plain_and_zstd() {
    for compressed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(root.path(), "codex");
        let t0 = now() * 1000 - 120_000;
        let parent_id = "01a1013b-940b-7973-ad67-eb128b853dfc";
        let child_id = "01a1b4bc-9a01-70f0-8d48-09df07abec6f";
        let parent_prefix = format!(
            "{}\n{{\"timestamp\":\"{}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"output\":\"{}\"}}}}\n{}\n",
            json!({"timestamp": rfc3339(t0), "type": "session_meta", "payload": {"id": parent_id, "originator": "codex_work_desktop"}}),
            rfc3339(t0 + 1_000),
            "B".repeat(17 * 1024 * 1024),
            json!({
                "timestamp": rfc3339(t0 + 2_000),
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {"input_tokens": 500, "cached_input_tokens": 0, "output_tokens": 100},
                        "last_token_usage": {"input_tokens": 500, "cached_input_tokens": 0, "output_tokens": 100}
                    }
                }
            })
        );
        let expected_proof = format!("{:x}", Sha256::digest(parent_prefix.as_bytes()));
        let end_byte_offset = parent_prefix.len() as u64;
        let sessions_dir = Path::new(&source.path).join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        if compressed {
            let packed = zstd::stream::encode_all(parent_prefix.as_bytes(), 1).unwrap();
            fs::write(
                sessions_dir.join(format!("rollout-2026-11-07T14-40-21-{parent_id}.jsonl.zst")),
                packed,
            )
            .unwrap();
        } else {
            fs::write(
                sessions_dir.join(format!("rollout-2026-11-07T14-40-21-{parent_id}.jsonl")),
                parent_prefix.as_bytes(),
            )
            .unwrap();
        }

        let child_path = sessions_dir.join(format!(
            "rollout-2026-11-07T14-45-00-{parent_id}_{child_id}.jsonl"
        ));
        write_lines(
            &child_path,
            &[
                json!({
                    "timestamp": rfc3339(t0 + 10_000),
                    "type": "session_meta",
                    "payload": {
                        "id": parent_id,
                        "history_base": {
                            "thread_id": parent_id,
                            "end_byte_offset": end_byte_offset
                        }
                    }
                })
                .to_string(),
                json!({"timestamp": rfc3339(t0 + 11_000), "type": "turn_context", "payload": {"turn_id": "t2", "model": "gpt-5.4"}}).to_string(),
                json!({
                    "timestamp": rfc3339(t0 + 12_000),
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "total_token_usage": {"input_tokens": 800, "cached_input_tokens": 0, "output_tokens": 250},
                            "last_token_usage": {"input_tokens": 300, "cached_input_tokens": 0, "output_tokens": 150}
                        }
                    }
                })
                .to_string(),
            ],
        );

        let result = import::scan(&engine.store, &settings, &source).unwrap();
        assert!(
            result["partial"].is_null(),
            "compressed={compressed}: {result}"
        );
        assert!(
            result["forkHistoryIssues"].is_null(),
            "compressed={compressed}: {result}"
        );
        let (_, _, state) = engine
            .store
            .cursor(&source.id, &child_path.to_string_lossy())
            .unwrap()
            .unwrap();
        let inherited = state.inherited_history.unwrap();
        assert_eq!(inherited.proof, expected_proof);
        assert_eq!(inherited.originator, "codex_work_desktop");
        assert_eq!(inherited.tokens.unwrap().total(), 600);
        // Plain mode imports both parent (600) and child delta (450) = 1050;
        // zstd-only parent is outside plain jsonl scan, so child delta = 450.
        assert_eq!(
            engine
                .store
                .dashboard(&Filter::default())
                .unwrap()
                .summary
                .total,
            if compressed { 450 } else { 1050 }
        );
    }
}

#[test]
fn claude_kimi_and_harness_preserve_usage_on_oversized_records() {
    // 1. Claude: >16 MiB assistant content BEFORE trailing message.usage on the same line.
    {
        let root = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(root.path(), "claude");
        let ts = rfc3339(now() * 1000 - 30_000);
        let big_assistant = format!(
            "{{\"type\":\"assistant\",\"uuid\":\"u1\",\"sessionId\":\"s1\",\"requestId\":\"r1\",\"timestamp\":\"{ts}\",\"message\":{{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"Write\",\"input\":{{\"content\":\"{}\"}}}}],\"usage\":{{\"input_tokens\":1000,\"output_tokens\":2500,\"cache_read_input_tokens\":500,\"cache_creation_input_tokens\":0}}}}}}",
            "C".repeat(17 * 1024 * 1024)
        );
        let path = Path::new(&source.path).join("projects/proj/session.jsonl");
        write_lines(&path, &[big_assistant]);
        let res = import::scan(&engine.store, &settings, &source).unwrap();
        assert!(res["partial"].is_null(), "{res}");
        assert_eq!(res["newEvents"], 1);
        assert_eq!(
            engine
                .store
                .dashboard(&Filter::default())
                .unwrap()
                .summary
                .total,
            4000
        );
    }

    // 2. Kimi: >9 MiB non-forked line before `forked` cut must not break kimi_fork_offset (>8 MiB).
    {
        let root = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(root.path(), "kimi");
        let t = now() * 1000 - 20_000;
        let big_kimi = format!(
            "{{\"type\":\"context.append_loop_event\",\"time\":{t},\"event\":{{\"type\":\"tool.result\",\"output\":\"{}\"}}}}",
            "K".repeat(9 * 1024 * 1024)
        );
        let path = Path::new(&source.path).join("sessions/proj/s1/agents/main/wire.jsonl");
        write_lines(
            &path,
            &[
                big_kimi,
                json!({"type": "forked"}).to_string(),
                json!({"type": "usage.record", "model": "kimi-k2", "time": t + 1000, "usage": {"inputOther": 100, "output": 50, "inputCacheRead": 20, "inputCacheCreation": 10}}).to_string(),
            ],
        );
        let res = import::scan(&engine.store, &settings, &source).unwrap();
        assert!(res["partial"].is_null(), "{res}");
        assert_eq!(res["newEvents"], 1);
        assert_eq!(
            engine
                .store
                .dashboard(&Filter::default())
                .unwrap()
                .summary
                .total,
            180
        );
    }

    // 3. DeepSeek Harness: >9 MiB assistant/message content (>8 MiB) still records usage.
    {
        let root = tempfile::tempdir().unwrap();
        let (engine, source, settings) = setup(root.path(), "deepseek-harness");
        let t = now() * 1000 - 10_000;
        let big_dsh = format!(
            "{{\"type\":\"assistant/message\",\"seq\":2,\"time\":{t},\"data\":{{\"turn\":1,\"step\":1,\"message\":{{\"content\":\"{}\",\"source\":{{\"model\":\"deepseek-r1\"}}}},\"usage\":{{\"inputTokens\":700,\"outputTokens\":300,\"cacheReadTokens\":100}}}}}}",
            "H".repeat(9 * 1024 * 1024)
        );
        let path = Path::new(&source.path).join("sessions/proj/s1/session.v4.jsonl");
        write_lines(
            &path,
            &[
                json!({"type": "session", "version": 4, "id": "dsh-1"}).to_string(),
                big_dsh,
            ],
        );
        let res = import::scan(&engine.store, &settings, &source).unwrap();
        assert_eq!(res["partial"], false, "{res}");
        assert_eq!(res["newEvents"], 1);
        assert_eq!(
            engine
                .store
                .dashboard(&Filter::default())
                .unwrap()
                .summary
                .total,
            1100
        );
    }
}

#[test]
fn in_flight_oversized_json_waits_cleanly_until_newline_arrives() {
    let root = tempfile::tempdir().unwrap();
    let (engine, source, settings) = setup(root.path(), "codex");
    let t0 = now() * 1000 - 30_000;
    let path = Path::new(&source.path).join("sessions/in_flight.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let prefix = format!(
        "{}\n{{\"timestamp\":\"{}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"output\":\"{}",
        json!({"timestamp": rfc3339(t0), "type": "session_meta", "payload": {"id": "s-flight"}}),
        rfc3339(t0 + 1_000),
        "W".repeat(17 * 1024 * 1024)
    );
    fs::write(&path, &prefix).unwrap();

    // No `\n` yet, valid JSON object prefix >16 MiB -> waits cleanly without reporting an error.
    let pending = import::scan(&engine.store, &settings, &source).unwrap();
    assert!(pending["partial"].is_null(), "{pending}");
    assert!(pending["issues"].is_null(), "{pending}");

    // Finish flushing the line plus a following token_count record.
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, "\"}}}}").unwrap();
    writeln!(
        f,
        "{}",
        json!({
            "timestamp": rfc3339(t0 + 2_000),
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {"input_tokens": 400, "output_tokens": 100},
                    "last_token_usage": {"input_tokens": 400, "output_tokens": 100}
                }
            }
        })
    )
    .unwrap();

    let done = import::scan(&engine.store, &settings, &source).unwrap();
    assert!(done["partial"].is_null(), "{done}");
    assert_eq!(done["newEvents"], 1);
    assert_eq!(
        engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .total,
        500
    );
}

#[test]
fn local_session_monitor_handles_oversized_codex_and_claude_records() {
    // Codex: cold-tail reset crosses a >16 MiB function_call, then incremental >16 MiB output completes it.
    {
        let root = tempfile::tempdir().unwrap();
        let (_, source, settings) = setup(root.path(), "codex");
        let at_ms = now() * 1000;
        let path = Path::new(&source.path).join("sessions/live.jsonl");
        let big_call = format!(
            "{{\"timestamp\":\"{}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"exec_command\",\"arguments\":\"{}\"}}}}",
            rfc3339(at_ms - 2_000),
            "A".repeat(17 * 1024 * 1024)
        );
        write_lines(
            &path,
            &[
                json!({"timestamp": rfc3339(at_ms - 5_000), "type": "session_meta", "payload": {"id": "codex-live", "source": "cli"}}).to_string(),
                json!({"timestamp": rfc3339(at_ms - 4_000), "type": "event_msg", "payload": {"type": "task_started", "turn_id": "t1"}}).to_string(),
                big_call,
            ],
        );
        let mut monitor = SessionMonitor::default();
        let snap1 = monitor.read(&settings, SystemTime::now());
        assert_eq!(snap1.sources[0].state, "healthy");
        assert_eq!(snap1.phase(), Some(Phase::Tool));

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            "{{\"timestamp\":\"{}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"c1\",\"output\":\"{}\"}}}}",
            rfc3339(at_ms - 1_000),
            "O".repeat(17 * 1024 * 1024)
        )
        .unwrap();
        let snap2 = monitor.read(&settings, SystemTime::now() + Duration::from_secs(1));
        assert_eq!(snap2.sources[0].state, "healthy");
        assert_eq!(snap2.phase(), Some(Phase::Working));
    }

    // Claude: >16 MiB tool_use input and >16 MiB tool_result content keep live state healthy.
    {
        let root = tempfile::tempdir().unwrap();
        let (_, source, settings) = setup(root.path(), "claude");
        let at_ms = now() * 1000;
        let path = Path::new(&source.path).join("projects/p/live.jsonl");
        let big_tool = format!(
            "{{\"type\":\"assistant\",\"sessionId\":\"claude-live\",\"timestamp\":\"{}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"tu1\",\"name\":\"Write\",\"input\":{{\"content\":\"{}\"}}}}]}}}}",
            rfc3339(at_ms - 2_000),
            "W".repeat(17 * 1024 * 1024)
        );
        write_lines(
            &path,
            &[
                json!({"type": "user", "sessionId": "claude-live", "uuid": "u1", "timestamp": rfc3339(at_ms - 3_000), "message": {"content": "write file"}}).to_string(),
                big_tool,
            ],
        );
        let mut monitor = SessionMonitor::default();
        let snap1 = monitor.read(&settings, SystemTime::now());
        assert_eq!(snap1.sources[0].state, "healthy");
        assert_eq!(snap1.phase(), Some(Phase::Tool));

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            "{{\"type\":\"user\",\"sessionId\":\"claude-live\",\"timestamp\":\"{}\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"tu1\",\"content\":\"{}\"}}]}}}}\n{}",
            rfc3339(at_ms - 1_000),
            "R".repeat(17 * 1024 * 1024),
            json!({"type": "assistant", "sessionId": "claude-live", "timestamp": rfc3339(at_ms), "message": {"stop_reason": "end_turn", "content": [{"type": "text", "text": "done"}]}})
        )
        .unwrap();
        // First poll consumes the >16 MiB tool_result (exceeding the 512 KiB per-poll batch cap),
        // transitioning Tool -> Working; the next poll reads the trailing end_turn -> Complete.
        let snap2 = monitor.read(&settings, SystemTime::now() + Duration::from_secs(1));
        assert_eq!(snap2.sources[0].state, "healthy");
        assert_eq!(snap2.phase(), Some(Phase::Working));
        let snap3 = monitor.read(&settings, SystemTime::now() + Duration::from_secs(2));
        assert_eq!(snap3.sources[0].state, "healthy");
        assert_eq!(snap3.phase(), Some(Phase::Complete));
    }
}
