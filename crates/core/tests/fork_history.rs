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
        precise_counting: true,
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

#[test]
fn work_usage_and_nested_forks_are_separate_from_the_shared_codex_billing_pool() {
    for precise in [true, false] {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let stamp = now() - 100;
            let with_originator = |text: String, originator: &str| {
                let mut value: Value = serde_json::from_str(text.trim()).unwrap();
                value["payload"]["originator"] = json!(originator);
                line(value)
            };
            let parent = with_originator(meta("work", stamp - 40, None), "codex_work_desktop")
                + &count(stamp - 30, 100, 100);
            let middle = meta("middle", stamp - 25, Some(("work", parent.len())))
                + &count(stamp - 20, 150, 50);
            let child = meta("child", stamp - 15, Some(("middle", middle.len())))
                + &count(stamp - 10, 175, 25);
            let explicit = with_originator(
                meta("explicit-codex", stamp - 5, Some(("work", parent.len()))),
                "codex_cli_rs",
            ) + &count(stamp, 105, 5);
            if compressed {
                std::fs::write(
                    temp.path().join("parent.jsonl.zst"),
                    packed(parent.as_bytes()),
                )
                .unwrap();
            } else {
                std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
            }
            std::fs::write(temp.path().join("middle.jsonl"), middle).unwrap();
            std::fs::write(temp.path().join("child.jsonl"), &child).unwrap();
            std::fs::write(temp.path().join("explicit.jsonl"), explicit).unwrap();
            let (store, mut settings, source) = setup(temp.path());
            settings.precise_counting = precise;
            store.save_settings(&settings).unwrap();
            import::scan(&store, &settings, &source).unwrap();
            let totals = |provider: Option<&str>| {
                let filter = Filter {
                    provider: provider.map(str::to_owned),
                    ..Default::default()
                };
                let detail = store.dashboard(&filter).unwrap();
                assert_eq!(
                    detail.summary.tokens.total(),
                    store
                        .dashboard_summary(&filter)
                        .unwrap()
                        .summary
                        .tokens
                        .total()
                );
                detail.summary.tokens.total()
            };
            let (work, codex) = if precise {
                (if compressed { 75 } else { 175 }, 5)
            } else {
                (if compressed { 325 } else { 425 }, 105)
            };
            assert_eq!(totals(Some("chatgpt-work")), work);
            assert_eq!(totals(Some("codex")), codex);
            assert_eq!(totals(None), work + codex);
            for id in ["work", "middle", "child", "explicit-codex"]
                .into_iter()
                .filter(|id| !compressed || *id != "work")
            {
                let payload: String = store
                    .db
                    .query_row(
                        "SELECT payload FROM events WHERE json_extract(payload,'$.sessionId')=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .unwrap();
                let mut event: UsageEvent = serde_json::from_str(&payload).unwrap();
                assert_eq!(event.provider, "codex", "billing identity is stable");
                assert_eq!(
                    event.agent,
                    if id == "explicit-codex" {
                        "codex"
                    } else {
                        "chatgpt-work"
                    }
                );
                event.agent.clear();
                store.put_event(&event, &[], &settings).unwrap();
            }
            assert_eq!(
                totals(Some("chatgpt-work")),
                work,
                "a poorer duplicate keeps known originator"
            );
            import::rescan(&store, &settings, &source).unwrap();
            assert_eq!(totals(Some("chatgpt-work")), work);
            assert_eq!(totals(Some("codex")), codex);
            assert_eq!(totals(None), work + codex);
            compare_remote_baseline(&source, &child);
        }
    }
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
            "SELECT value FROM kv WHERE key='history-correction-backup-shared-v1'",
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
fn renamed_parent_uses_metadata_and_repairs_an_existing_missing_baseline() {
    for newline in ["\n", "\r\n"] {
        let temp = tempfile::tempdir().unwrap();
        let stamp = now() - 100;
        let parent_id = "01a12393-1534-7750-8dbc-21bebb00678e";
        let suffix_id = "01a13e39-8119-7f31-a38c-69d61a59ebfb";
        let parent = (meta(parent_id, stamp - 100, None)
            + &line(json!({"type":"response_item","payload":{"text":"父历史 中文"}}))
            + &count(stamp - 90, 900, 900))
            .replace('\n', newline);
        let child = (meta("child", stamp - 10, Some((parent_id, parent.len())))
            + &count(stamp, 1000, 100)
            + &count(stamp + 1, 1100, 100))
            .replace('\n', newline);
        std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
        let (store, settings, source) = setup(temp.path());
        assert_eq!(
            import::scan(&store, &settings, &source).unwrap()["missingHistoryBases"],
            1
        );
        assert_eq!(usage_for(&store, "child"), 100);
        std::fs::write(
            temp.path()
                .join(format!("imported-{parent_id}_{suffix_id}.jsonl")),
            parent,
        )
        .unwrap();
        let result = import::scan(&store, &settings, &source).unwrap();
        assert!(result.get("missingHistoryBases").is_none());
        assert_eq!(usage_for(&store, "child"), 200);
        assert_eq!(usage_for(&store, parent_id), 900);
        import::rescan(&store, &settings, &source).unwrap();
        assert_eq!(usage_for(&store, "child"), 200);
    }
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

const THREAD: &str = "01a12393-1534-7750-8dbc-21bebb00678e";
const REVISION: &str = "01a13e39-8119-7f31-a38c-69d61a59ebfb";
const SECOND_REVISION: &str = "01a13e39-8119-7f31-a38c-69d61a59ebfc";

fn rollout(thread: &str, revision: &str) -> String {
    if thread == revision {
        format!("rollout-2026-10-15T14-21-58-{thread}.jsonl")
    } else {
        format!("rollout-2026-10-15T14-21-58-{thread}_{revision}.jsonl")
    }
}

fn packed(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
    encoder.include_checksum(true).unwrap();
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn compare_remote_baseline(source: &Source, metadata: &str) {
    use aieyes_core::codex_history::Resolver;
    let mut resolver = Resolver::new(source, &import::paths(source).unwrap());
    let meta: Value = serde_json::from_str(metadata.lines().next().unwrap()).unwrap();
    let native = resolver.metadata(&meta).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Exercise the real SSH loader shape: the helpers are bundled, without __file__.
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = std::process::Command::new(python).args(["-c", r#"
import pathlib,json,sys
root=pathlib.Path(sys.argv[1])
scope={'REQUEST':dict(protocol=2,path=sys.argv[2],provider='codex',preciseCounting=True),
       'HARNESS_SCRIPT':(root/'scripts/harness_remote.py').read_text(encoding='utf-8')}
exec(compile((root/'scripts/remote_history.py').read_text(encoding='utf-8'),'<wire-history>','exec'),scope)
"#]).arg(&project).arg(&source.path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("PRIVATE_PARENT"));
    let frames: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let remote = frames
        .iter()
        .filter_map(|f| f["events"].as_array())
        .flatten()
        .find(|e| e["type"] == "session_meta" && e["payload"]["id"] == meta["payload"]["id"])
        .unwrap()["inheritedHistory"]
        .clone();
    assert_eq!(remote, serde_json::to_value(native).unwrap());
}

#[test]
fn same_thread_revert_resolves_physical_rollouts_instead_of_the_selected_db_revision() {
    for newline in ["\n", "\r\n"] {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions/2026/10/15");
        let archive = temp.path().join("archived_sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&archive).unwrap();
        let stamp = now() - 100;
        let parent =
            (meta(THREAD, stamp - 50, None) + &count(stamp - 40, 900, 900)).replace('\n', newline);
        // No forked_from_id or manual fork: revert keeps the logical thread ID stable.
        let first = (meta(THREAD, stamp - 30, Some((THREAD, parent.len())))
            + &count(stamp - 20, 1000, 100))
            .replace('\n', newline);
        let second = (meta(THREAD, stamp - 15, Some((REVISION, first.len())))
            + &count(stamp - 10, 1100, 100))
            .replace('\n', newline);
        let child = (meta("child", stamp - 5, Some((SECOND_REVISION, second.len())))
            + &count(stamp, 1200, 100)
            + &count(stamp + 1, 1300, 100))
            .replace('\n', newline);
        std::fs::write(archive.join(rollout(THREAD, THREAD)), &parent).unwrap();
        std::fs::write(archive.join(rollout(THREAD, REVISION)), &first).unwrap();
        let current = sessions.join(rollout(THREAD, SECOND_REVISION));
        std::fs::write(&current, &second).unwrap();
        std::fs::write(sessions.join("child.jsonl"), &child).unwrap();
        let db = rusqlite::Connection::open(temp.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT,rollout_path TEXT);")
            .unwrap();
        db.execute(
            "INSERT INTO threads VALUES(?1,?2)",
            rusqlite::params![THREAD, current.to_string_lossy()],
        )
        .unwrap();
        drop(db);
        let (store, settings, source) = setup(temp.path());
        let result = import::scan(&store, &settings, &source).unwrap();
        assert!(result.get("missingHistoryBases").is_none(), "{result}");
        assert_eq!(usage_for(&store, THREAD), 1100);
        assert_eq!(usage_for(&store, "child"), 200);
        compare_remote_baseline(&source, &child);
        import::rescan(&store, &settings, &source).unwrap();
        assert_eq!(usage_for(&store, THREAD), 1100);
        assert_eq!(usage_for(&store, "child"), 200);
    }
}

#[test]
fn verified_parent_counter_does_not_require_an_unavailable_ancestor() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent =
        meta("parent", stamp - 20, Some(("unavailable", 123))) + &count(stamp - 10, 900, 100);
    let child = meta("child", stamp - 5, Some(("parent", parent.len())))
        + &count(stamp, 1000, 100)
        + &count(stamp + 1, 1100, 100);
    std::fs::write(temp.path().join("parent.jsonl"), &parent).unwrap();
    std::fs::write(temp.path().join("child.jsonl"), &child).unwrap();
    let wrong = temp.path().join("wrong.jsonl");
    std::fs::write(&wrong, meta("different", stamp - 40, None)).unwrap();
    let db = rusqlite::Connection::open(temp.path().join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT,rollout_path TEXT);")
        .unwrap();
    db.execute(
        "INSERT INTO threads VALUES(?1,?2)",
        rusqlite::params!["parent", wrong.to_string_lossy()],
    )
    .unwrap();
    drop(db);
    let (store, settings, source) = setup(temp.path());
    let result = import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
    assert_eq!(result["missingHistoryBases"], 1);
    assert_eq!(result["forkHistoryIssues"][0]["path"], "parent.jsonl");
    compare_remote_baseline(&source, &child);
}

#[test]
fn metadata_only_revisions_recurse_with_matching_native_and_ssh_proofs() {
    for compressed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let stamp = now() - 100;
        let parent = meta(THREAD, stamp - 30, None) + &count(stamp - 20, 900, 900);
        let middle = meta(THREAD, stamp - 10, Some((THREAD, parent.len())));
        let child =
            meta("child", stamp - 5, Some((REVISION, middle.len()))) + &count(stamp, 1000, 100);
        for (id, data) in [(THREAD, parent), (REVISION, middle)] {
            let name = rollout(THREAD, id);
            if compressed {
                std::fs::write(
                    temp.path().join(format!("{name}.zst")),
                    packed(data.as_bytes()),
                )
                .unwrap();
            } else {
                std::fs::write(temp.path().join(name), data).unwrap();
            }
        }
        std::fs::write(temp.path().join("child.jsonl"), &child).unwrap();
        let (store, settings, source) = setup(temp.path());
        assert!(
            import::scan(&store, &settings, &source)
                .unwrap()
                .get("missingHistoryBases")
                .is_none()
        );
        assert_eq!(usage_for(&store, "child"), 100);
        compare_remote_baseline(&source, &child);
    }
}

#[test]
fn compressed_parent_recovers_and_representation_changes_preserve_the_prefix_proof() {
    for newline in ["\n", "\r\n"] {
        let temp = tempfile::tempdir().unwrap();
        let stamp = now() - 100;
        let parent = (meta(THREAD, stamp - 20, None)
            + &line(json!({"type":"response_item","payload":{"text":"PRIVATE_PARENT 中文"}}))
            + &count(stamp - 10, 900, 900))
            .replace('\n', newline);
        let child = (meta("child", stamp - 5, Some((REVISION, parent.len())))
            + &count(stamp, 1000, 100)
            + &count(stamp + 1, 1100, 100))
            .replace('\n', newline);
        let child_path = temp.path().join("child.jsonl");
        std::fs::write(&child_path, &child).unwrap();
        let (store, settings, source) = setup(temp.path());
        assert_eq!(
            import::scan(&store, &settings, &source).unwrap()["missingHistoryBases"],
            1
        );
        assert_eq!(usage_for(&store, "child"), 100);
        let plain = temp.path().join(rollout(THREAD, REVISION));
        let compressed = temp
            .path()
            .join(format!("{}.zst", rollout(THREAD, REVISION)));
        // Multiple checksummed frames, with a non-inherited tail after the frozen boundary.
        let mut encoded = packed(parent.as_bytes());
        encoded.extend(packed(count(stamp + 2, 9999, 9099).as_bytes()));
        std::fs::write(&compressed, &encoded).unwrap();
        let result = import::scan(&store, &settings, &source).unwrap();
        assert!(result.get("missingHistoryBases").is_none(), "{result}");
        assert_eq!(usage_for(&store, "child"), 200);
        assert_eq!(std::fs::read(&compressed).unwrap(), encoded);
        let proof = store
            .cursor(&source.id, &child_path.to_string_lossy())
            .unwrap()
            .unwrap()
            .2
            .inherited_history
            .unwrap()
            .proof;
        compare_remote_baseline(&source, &child);
        // Existing cursors from the previous parser are repaired without a manual rescan.
        store
            .db
            .execute(
                "UPDATE files SET state=json_set(state,'$.parserVersion',7)",
                [],
            )
            .unwrap();
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(usage_for(&store, "child"), 200);
        std::fs::write(&plain, &parent).unwrap();
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(
            store
                .cursor(&source.id, &child_path.to_string_lossy())
                .unwrap()
                .unwrap()
                .2
                .inherited_history
                .unwrap()
                .proof,
            proof
        );
        std::fs::remove_file(&plain).unwrap();
        assert_eq!(
            import::scan(&store, &settings, &source).unwrap()["readBytes"],
            0
        );
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        std::fs::write(&compressed, &encoded).unwrap();
        let invalid = import::scan(&store, &settings, &source).unwrap();
        assert_eq!(
            invalid["forkHistoryIssues"][0]["reason"],
            "invalidCompressedHistory"
        );
        assert_eq!(usage_for(&store, "child"), 100);
    }
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
    // Keep the initial quota clear of its 120-second freshness boundary while
    // slower hosts initialize the database and prices below.
    let stamp = now() - 30;
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
        precise_counting: true,
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
        .estimates()
        .unwrap()
        .into_iter()
        .find(|e| e.original_estimate_id.as_deref() == Some(&original.id))
        .unwrap();
    assert!(!invalid.has_value());
    assert_eq!(invalid.calculation_status, "unknownBilling");
    assert_eq!(invalid.prices, original.prices);
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    let repaired = store
        .estimates()
        .unwrap()
        .into_iter()
        .find(|e| e.original_estimate_id.as_deref() == Some(&invalid.id))
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
    let mut cumulative = settings.clone();
    cumulative.precise_counting = false;
    store.save_settings(&cumulative).unwrap();
    import::scan(&store, &cumulative, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 1000);
    let records = store.estimates().unwrap();
    assert_eq!(records.len(), 4);
    let approximate = records
        .iter()
        .find(|e| e.original_estimate_id.as_deref() == Some(&repaired.id))
        .unwrap();
    assert!(
        !approximate.has_value(),
        "An inherited cumulative summary cannot prove subscription billing"
    );
    store.save_settings(&settings).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    let records = store.estimates().unwrap();
    assert_eq!(records.len(), 5);
    let precise = records
        .iter()
        .find(|e| e.original_estimate_id.as_deref() == Some(&approximate.id))
        .unwrap();
    assert_eq!(precise.quota().unwrap().weekly_value, Some(5.0));
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(store.estimates().unwrap().len(), 5);
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

#[test]
fn counting_mode_defaults_on_and_replays_both_directions_with_frozen_prices() {
    assert!(
        serde_json::from_value::<Settings>(json!({}))
            .unwrap()
            .precise_counting
    );
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let parent = meta("parent", stamp - 100, None) + &count(stamp - 90, 900, 900);
    let child = meta("child", stamp - 10, Some(("parent", parent.len())))
        + &count(stamp, 1000, 100)
        + &count(stamp + 1, 1100, 100);
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    let (store, mut settings, source) = setup(temp.path());
    settings.precise_counting = false;
    store.save_settings(&settings).unwrap();
    store
        .save_prices(&[ModelPrice {
            id: "unknown".into(),
            input: Some(0.01),
            ..Default::default()
        }])
        .unwrap();
    let result = import::scan(&store, &settings, &source).unwrap();
    assert_ne!(result["partial"], true);
    assert!(result.get("missingHistoryBases").is_none());
    assert_eq!(usage_for(&store, "child"), 1100);
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.cost,
        11.0
    );
    store
        .save_prices(&[ModelPrice {
            id: "unknown".into(),
            input: Some(9.0),
            ..Default::default()
        }])
        .unwrap();
    let mut first = true;
    for (precise, expected) in [(true, 100), (false, 1100), (true, 100), (false, 1100)] {
        let old = settings.clone();
        settings.precise_counting = precise;
        store.save_settings(&settings).unwrap();
        assert!(import::scan(&store, &old, &source).is_err());
        let result = import::scan(&store, &settings, &source).unwrap();
        assert_eq!(usage_for(&store, "child"), expected);
        assert_eq!(result["forkHistoryIssues"].is_array(), precise);
        assert_ne!(result["partial"], true);
        if first {
            assert!(
                result["forkHistoryIssues"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains("尚未找到")
            );
            first = false;
        }
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(usage_for(&store, "child"), expected);
    }
    std::fs::write(temp.path().join("parent.jsonl"), parent).unwrap();
    settings.precise_counting = true;
    store.save_settings(&settings).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
    // Simulate upgrading a cursor from the previous build and a settings file without the option.
    store
        .db
        .execute(
            "UPDATE files SET state=json_remove(state,'$.preciseCounting')",
            [],
        )
        .unwrap();
    store
        .db
        .execute(
            "UPDATE kv SET value=json_remove(value,'$.preciseCounting') WHERE key='settings'",
            [],
        )
        .unwrap();
    let upgraded = store.settings().unwrap();
    assert!(upgraded.precise_counting);
    import::scan(&store, &upgraded, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
    let child_cost: f64 = store
        .db
        .query_row(
            "SELECT SUM(cost) FROM events WHERE json_extract(payload,'$.sessionId')='child'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(child_cost, 2.0);
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE json_extract(payload,'$.sessionId')='child'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn cached_fork_warnings_are_settings_diagnostics_but_real_failures_remain_visible() {
    use aieyes_core::codex_history::sync_status;
    let fork = json!({"code":"missingHistoryBase","message":"父历史缺失"});
    let old = json!({"partial":true,"issues":[fork],"updatedAt":42});
    let precise = sync_status(old.clone(), true);
    assert_eq!(precise["partial"], false);
    assert_eq!(precise["forkHistoryIssues"][0], fork);
    let normal = sync_status(old, false);
    assert_eq!(normal["partial"], false);
    assert!(normal.get("forkHistoryIssues").is_none());
    let mixed = sync_status(
        json!({"partial":true,"issues":[fork,{"code":"fileRead","message":"无法读取"}]}),
        true,
    );
    assert_eq!(mixed["partial"], true);
    assert_eq!(mixed["issues"].as_array().unwrap().len(), 1);
    let failed = sync_status(
        json!({"partial":true,"error":"连接失败","issues":[fork]}),
        false,
    );
    assert_eq!(failed["partial"], true);
    assert_eq!(failed["error"], "连接失败");
}

#[test]
fn chatgpt_work_fork_keeps_codex_metering_and_its_own_agent() {
    let temp = tempfile::tempdir().unwrap();
    let stamp = now() - 100;
    let work = |line: String| {
        line.replace(
            "\"model_provider\"",
            "\"originator\":\"codex_work_desktop\",\"model_provider\"",
        )
    };
    let prefix = work(meta("parent", stamp - 100, None)) + &count(stamp - 90, 1_000, 1_000);
    let child =
        work(meta("child", stamp - 10, Some(("parent", prefix.len())))) + &count(stamp, 1_200, 200);
    std::fs::write(temp.path().join("parent.jsonl"), prefix).unwrap();
    std::fs::write(temp.path().join("child.jsonl"), child).unwrap();
    std::fs::write(
        temp.path().join("plain.jsonl"),
        meta("plain", stamp - 5, None) + &count(stamp, 300, 300),
    )
    .unwrap();
    let (store, settings, source) = setup(temp.path());
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(usage_for(&store, "child"), 200);
    let agents = |session: &str| -> Vec<(String, String)> {
        let mut query = store
            .db
            .prepare("SELECT payload FROM events WHERE json_extract(payload,'$.sessionId')=?1")
            .unwrap();
        query
            .query_map([session], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| {
                let e = serde_json::from_str::<UsageEvent>(&r.unwrap()).unwrap();
                (e.provider, e.agent)
            })
            .collect()
    };
    for session in ["parent", "child"] {
        let rows = agents(session);
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|(p, a)| p == "codex" && a == "chatgpt-work"),
            "{session}: {rows:?}"
        );
    }
    assert_eq!(agents("plain"), vec![("codex".into(), String::new())]);
}
