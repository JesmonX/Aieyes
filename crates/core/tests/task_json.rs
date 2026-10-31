use aieyes_core::{import, models::*, store::Store};
use serde_json::{Value, json};
use std::path::Path;

fn setup(root: &Path, provider: &str) -> (Store, Settings, Source) {
    let source = Source {
        id: "tasks".into(),
        provider: provider.into(),
        path: root.join("client").to_string_lossy().into(),
        ..Default::default()
    };
    let settings = Settings {
        sources: vec![source.clone()],
        precise_counting: true,
        ..Default::default()
    };
    let store = Store::open(&root.join("app")).unwrap();
    store.save_settings(&settings).unwrap();
    (store, settings, source)
}
fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}
fn events(store: &Store) -> Vec<UsageEvent> {
    store
        .db
        .prepare("SELECT payload FROM events")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
        .collect()
}

#[test]
fn cline_real_metrics_children_copies_and_replay_have_one_charge() {
    let temp = tempfile::tempdir().unwrap();
    let (store, settings, source) = setup(temp.path(), "cline");
    let value: Value =
        serde_json::from_str(include_str!("fixtures/native-agents/cline-current.json")).unwrap();
    let root = temp.path().join("client/sessions/root");
    write(&root.join("root.messages.json"), &value);
    let mut copy = value.clone();
    copy["sessionId"] = json!("copy");
    write(
        &temp.path().join("client/sessions/copy/copy.messages.json"),
        &copy,
    );
    let mut child = value;
    child["sessionId"] = json!("child");
    child["origin"]["parentThreadId"] = json!("root");
    child["messages"][1]["id"] = json!("msg-child");
    child["messages"][1]["metrics"]["inputTokens"] = json!(10);
    child["messages"][1]["metrics"]["outputTokens"] = json!(2);
    write(&root.join("worker__subtask.messages.json"), &child);
    import::scan(&store, &settings, &source).unwrap();
    let totals = || events(&store).iter().map(|e| e.tokens.total()).sum::<u64>();
    assert_eq!(totals(), 4689 + 23 + 10 + 2);
    assert_eq!(
        import::scan(&store, &settings, &source).unwrap()["newEvents"],
        0
    );
    import::rescan(&store, &settings, &source).unwrap();
    assert_eq!(totals(), 4689 + 23 + 10 + 2);
    assert!(
        !serde_json::to_string(&events(&store))
            .unwrap()
            .contains("PRIVATE")
    );
}

#[test]
fn legacy_compaction_keeps_cumulative_usage_and_no_profile_model_is_invented() {
    for provider in ["cline", "kilo", "roo"] {
        let temp = tempfile::tempdir().unwrap();
        let (store, settings, source) = setup(temp.path(), provider);
        let file = temp.path().join("client/tasks/task/ui_messages.json");
        let message = |kind: &str, input: u64, ts: i64| json!({"type":"say","say":kind,"ts":ts,"modelInfo":{"modelId":"verified-model"},"text":json!({"tokensIn":input,"tokensOut":1,"request":"PRIVATE"}).to_string()});
        write(
            &file,
            &json!([
                message("api_req_started", 10, 1000),
                message("api_req_started", 20, 2000)
            ]),
        );
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(events(&store)[0].tokens.input, 30);
        write(
            &file,
            &json!([
                message("deleted_api_reqs", 10, 1000),
                message("api_req_started", 20, 2000)
            ]),
        );
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(events(&store).len(), 1);
        assert_eq!(events(&store)[0].tokens.input, 30);
        assert_eq!(events(&store)[0].model, "unknown");
        write(
            &file.with_file_name("history_item.json"),
            &json!({"id":"task","ts":2000,"task":"PRIVATE","tokensIn":30,"tokensOut":2,"apiConfigName":"profile-name"}),
        );
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(events(&store).len(), 1);
        assert_eq!(events(&store)[0].model, "unknown");
        assert!(
            !serde_json::to_string(&events(&store))
                .unwrap()
                .contains("PRIVATE")
        );
    }
}

#[test]
fn legacy_subagent_overlap_corrects_only_its_task_and_preserves_child_and_source_namespaces() {
    for own in [0, 10] {
        let temp = tempfile::tempdir().unwrap();
        let (store, mut settings, source) = setup(temp.path(), "cline");
        let path = temp.path().join("client/tasks/lead/ui_messages.json");
        let message = |kind: &str, input: u64| {
            json!({"type":"say","say":kind,"ts":1000,"modelInfo":{"modelId":"legacy-model"},
            "text":json!({"source":"subagents","tokensIn":input,"tokensOut":0}).to_string()})
        };
        let mut messages = vec![message("subagent_usage", 20)];
        if own > 0 {
            messages.push(message("api_req_started", own));
        }
        write(&path, &json!(messages));
        import::scan(&store, &settings, &source).unwrap();
        assert_eq!(events(&store)[0].tokens.input, own + 20);

        let other = Source {
            id: "other-unknown-account".into(),
            path: temp.path().join("other-client").to_string_lossy().into(),
            ..source.clone()
        };
        settings.sources.push(other.clone());
        store.save_settings(&settings).unwrap();
        write(
            &temp.path().join("other-client/tasks/lead/ui_messages.json"),
            &json!([message("api_req_started", 99)]),
        );
        import::scan(&store, &settings, &other).unwrap();

        let mut child: Value =
            serde_json::from_str(include_str!("fixtures/native-agents/cline-current.json"))
                .unwrap();
        child["sessionId"] = json!("child");
        child["origin"]["parentThreadId"] = json!("lead");
        child["messages"][1]["id"] = json!("independent-child-request");
        child["messages"][1]["metrics"] = json!({"inputTokens":20,"outputTokens":0});
        write(
            &temp
                .path()
                .join("client/sessions/tree/worker__job.messages.json"),
            &child,
        );
        let result = import::scan(&store, &settings, &source).unwrap();
        assert_eq!(result["partial"], true);
        assert_eq!(result["issues"][0]["code"], "overlappingSubagentSummary");
        let current = events(&store);
        let parent = current
            .iter()
            .find(|e| e.source_id == source.id && e.session_id == "lead")
            .unwrap();
        assert_eq!(parent.tokens.input, own);
        assert_eq!(parent.usage_version, 3);
        assert!(
            current.iter().any(|e| e.source_id == source.id
                && e.session_id == "child"
                && e.tokens.input == 20)
        );
        assert!(
            current
                .iter()
                .any(|e| e.source_id == other.id && e.session_id == "lead" && e.tokens.input == 99)
        );
        assert_eq!(current.len(), 3);
        assert_eq!(
            import::scan(&store, &settings, &source).unwrap()["partial"],
            true
        );
        import::rescan(&store, &settings, &source).unwrap();
        assert_eq!(
            events(&store).iter().map(|e| e.tokens.input).sum::<u64>(),
            own + 20 + 99
        );
    }
}

#[test]
fn cline_overlap_version_never_applies_to_a_message_or_another_provider() {
    let task = json!({"id":"task:lead","sessionId":"lead","timestamp":1000,"model":"unknown",
        "tokens":{"input":1},"clineTaskUsageVersion":3});
    for (provider, id, expected) in [
        ("cline", "task:lead", 3),
        ("cline", "msg:child", 0),
        ("cline", "task:other", 0),
        ("roo", "task:lead", 0),
        ("kilo", "task:lead", 0),
    ] {
        let source = Source {
            provider: provider.into(),
            ..Default::default()
        };
        let mut row = task.clone();
        row["id"] = json!(id);
        let event = aieyes_core::usage::parse(&source, &mut ParseState::default(), &row).unwrap();
        assert_eq!(event.usage_version, expected);
    }
}

#[test]
fn malformed_task_does_not_block_other_files_and_retries_after_repair() {
    let temp = tempfile::tempdir().unwrap();
    let (store, settings, source) = setup(temp.path(), "roo");
    let good = temp.path().join("client/tasks/good/history_item.json");
    let bad = temp.path().join("client/tasks/bad/history_item.json");
    let item = |id: &str| json!({"id":id,"ts":1000,"tokensIn":1,"tokensOut":2,"task":"PRIVATE"});
    write(&good, &item("good"));
    write(&bad, &item("bad"));
    std::fs::write(&bad, "{").unwrap();
    let status = import::scan(&store, &settings, &source).unwrap();
    assert_eq!(status["partial"], true);
    assert_eq!(events(&store).len(), 1);
    write(&bad, &item("bad"));
    let fixed = import::scan(&store, &settings, &source).unwrap();
    assert_eq!(fixed["partial"], false);
    assert_eq!(events(&store).len(), 2);
}

#[test]
fn cline_database_follows_explicit_transcript_locations_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let (store, settings, source) = setup(temp.path(), "cline");
    let outside = temp.path().join("custom-transcripts/root.messages.json");
    let value: Value =
        serde_json::from_str(include_str!("fixtures/native-agents/cline-current.json")).unwrap();
    write(&outside, &value);
    let dbpath = temp.path().join("client/db/sessions.db");
    std::fs::create_dir_all(dbpath.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    db.execute_batch("CREATE TABLE sessions(session_id TEXT,messages_path TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES('root',?1)",
        [outside.to_string_lossy().as_ref()],
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(&dbpath).unwrap();
    import::scan(&store, &settings, &source).unwrap();
    assert_eq!(events(&store)[0].tokens.input, 4689);
    assert_eq!(before, std::fs::read(&dbpath).unwrap());
}
