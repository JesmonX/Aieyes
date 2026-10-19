use aieyes_core::{Engine, models::*, opencode};
use rusqlite::{Connection, params};
use serde_json::json;

#[test]
fn tokens_separate_cache_and_include_reasoning_once() {
    let source = Source {
        provider: "opencode".into(),
        id: "local".into(),
        ..Default::default()
    };
    let data = json!({"tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":50,"write":10}},"completed":1700000000000i64,"model":"glm-5","provider":"opencode-go"});
    let event = opencode::event(&source, "message", "session", &data).unwrap();
    assert_eq!(event.tokens.total(), 185);
    assert_eq!(event.tokens.output, 25);
    assert_eq!(event.tokens.reasoning, 5);
    assert_eq!(event.billing.category, "unknown");
    let mut invalid = data.clone();
    invalid["tokens"]["input"] = json!(-1);
    assert!(opencode::event(&source, "message", "session", &invalid).is_err());
    invalid = data;
    invalid["tokens"]["output"] = json!(u64::MAX);
    assert!(opencode::event(&source, "message", "session", &invalid).is_err());
}

#[test]
fn sqlite_v1_v2_are_readonly_idempotent_and_ignore_unfinished_text() {
    for v2 in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let db = Connection::open(&path).unwrap();
        let table = if v2 { "session_message" } else { "message" };
        db.execute_batch(&format!(
            "CREATE TABLE {table}(id TEXT PRIMARY KEY, session_id TEXT, type TEXT, data TEXT)"
        ))
        .unwrap();
        let mut data = json!({"role":"assistant","modelID":"glm-5","providerID":"opencode-go","model":{"id":"glm-5","providerID":"opencode-go"},"time":{"completed":1700000000000i64},"tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":50,"write":10}},"content":"private conversation"});
        db.execute(
            &format!("INSERT INTO {table} VALUES('one','session','assistant',?1)"),
            params![data.to_string()],
        )
        .unwrap();
        data["time"] = json!({});
        db.execute(
            &format!("INSERT INTO {table} VALUES('pending','session','assistant',?1)"),
            params![data.to_string()],
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let engine = Engine::open(&root.path().join("aieyes")).unwrap();
        let source = Source {
            provider: "opencode".into(),
            id: "local-opencode".into(),
            path: path.to_string_lossy().into(),
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
        assert_eq!(
            opencode::scan(&engine.store, &settings, &source, &path).unwrap()["newEvents"],
            1
        );
        assert_eq!(
            opencode::scan(&engine.store, &settings, &source, &path).unwrap()["newEvents"],
            0
        );
        let payload: String = engine
            .store
            .db
            .query_row("SELECT payload FROM events", [], |r| r.get(0))
            .unwrap();
        assert!(!payload.contains("private conversation"));
        assert_eq!(before, std::fs::read(&path).unwrap());
    }
}

#[test]
fn empty_future_table_does_not_hide_real_v1_usage_and_migration_does_not_duplicate_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("opencode.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,data TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,data TEXT);").unwrap();
    let data = json!({"role":"assistant","modelID":"Qwen3.8-27B","providerID":"aieyes-test","time":{"created":1792298994621i64,"completed":1792298996770i64},"finish":"stop","tokens":{"total":2745,"input":2707,"output":38,"reasoning":0,"cache":{"read":0,"write":0}}});
    db.execute(
        "INSERT INTO message VALUES('request','session',?1)",
        [data.to_string()],
    )
    .unwrap();
    let engine = Engine::open(&root.path().join("aieyes")).unwrap();
    let source = Source {
        id: "opencode".into(),
        provider: "opencode".into(),
        path: path.to_string_lossy().into(),
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
    assert_eq!(
        opencode::scan(&engine.store, &settings, &source, &path).unwrap()["newEvents"],
        1
    );
    let mut next = data.clone();
    next["model"] = json!({"id":"Qwen3.8-27B","providerID":"aieyes-test"});
    db.execute(
        "INSERT INTO session_message VALUES('request','session','assistant',?1)",
        [next.to_string()],
    )
    .unwrap();
    assert_eq!(
        opencode::scan(&engine.store, &settings, &source, &path).unwrap()["newEvents"],
        0
    );
    let count: u64 = engine
        .store
        .db
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn native_session_status_distinguishes_tool_steps_from_finished_turns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("opencode.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE part(message_id TEXT,data TEXT);").unwrap();
    let stamp = now() * 1000;
    let mut data = json!({"role":"assistant","time":{"created":stamp,"completed":stamp},"finish":"tool-calls","content":"PRIVATE"});
    db.execute(
        "INSERT INTO message VALUES('one','session',?1,?1,?2)",
        params![stamp, data.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO part VALUES('one',?1)",[json!({"type":"tool","tool":"read","state":{"status":"running","input":{"path":"PRIVATE"}}}).to_string()]).unwrap();
    assert_eq!(opencode::sessions(&path).unwrap()[0].2, "tool");
    db.execute(
        "UPDATE part SET data=?1",
        [json!({"type":"tool","tool":"question","state":{"status":"running"}}).to_string()],
    )
    .unwrap();
    assert_eq!(opencode::sessions(&path).unwrap()[0].2, "PermissionRequest");
    data["finish"] = json!("stop");
    db.execute("UPDATE message SET data=?1", [data.to_string()])
        .unwrap();
    assert_eq!(opencode::sessions(&path).unwrap()[0].2, "complete");
    data["error"] = json!({"name":"MessageAbortedError"});
    db.execute("UPDATE message SET data=?1", [data.to_string()])
        .unwrap();
    assert_eq!(opencode::sessions(&path).unwrap()[0].2, "interrupted");
}

#[test]
fn actual_client_fixture_includes_tool_request_tokens_once() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/native-agents/opencode-messages.json"
    ))
    .unwrap();
    let source = Source {
        id: "open".into(),
        provider: "opencode".into(),
        ..Default::default()
    };
    let mut input = 0;
    let mut output = 0;
    for row in rows.as_array().unwrap() {
        let data = &row["data"];
        let event=opencode::event(&source,row["id"].as_str().unwrap(),row["session"].as_str().unwrap(),&json!({"tokens":data["tokens"],"completed":data["time"]["completed"],"model":data["modelID"],"provider":data["providerID"]})).unwrap();
        input += event.tokens.input;
        output += event.tokens.output;
    }
    assert_eq!((input, output), (8315, 162));
}

#[test]
fn incremental_scans_follow_new_rows_and_delayed_completion_without_old_history_reads() {
    for history in [10, 3000] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,data TEXT)")
            .unwrap();
        let done=json!({"role":"assistant","modelID":"fixture","time":{"completed":now()*1000},"tokens":{"input":10,"output":2,"reasoning":0,"cache":{"read":0,"write":0}}}).to_string();
        db.execute(
            "INSERT INTO message VALUES('pending-old','session',?1)",
            [json!({"role":"assistant","modelID":"fixture"}).to_string()],
        )
        .unwrap();
        db.execute_batch("BEGIN").unwrap();
        for i in 0..history {
            db.execute(
                "INSERT INTO message VALUES(?1,'session',?2)",
                params![format!("historical-{i}"), done],
            )
            .unwrap();
        }
        db.execute_batch("COMMIT").unwrap();
        let engine = Engine::open(&root.path().join("app")).unwrap();
        let source = Source {
            id: "open".into(),
            provider: "opencode".into(),
            path: path.to_string_lossy().into(),
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
        assert_eq!(
            opencode::scan(&engine.store, &settings, &source, &path).unwrap()["newEvents"],
            history
        );
        assert_eq!(
            opencode::scan(&engine.store, &settings, &source, &path).unwrap()["scannedRecords"],
            0
        );
        db.execute("UPDATE message SET data=?1 WHERE id='pending-old'", [&done])
            .unwrap();
        db.execute(
            "INSERT INTO message VALUES('a-new-lexically-earlier-id','session',?1)",
            [&done],
        )
        .unwrap();
        let delta = opencode::scan(&engine.store, &settings, &source, &path).unwrap();
        assert_eq!(delta["newEvents"], 2);
        assert_eq!(delta["scannedRecords"], 2);
        assert_eq!(
            opencode::scan(&engine.store, &settings, &source, &path).unwrap()["scannedRecords"],
            0
        );
        assert_eq!(
            aieyes_core::import::rescan(&engine.store, &settings, &source).unwrap()["newEvents"],
            0
        );
    }
}

#[test]
fn pending_queue_finishes_a_bounded_sweep_even_after_native_writes_stop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("opencode.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,data TEXT);BEGIN")
        .unwrap();
    for i in 0..700 {
        db.execute(
            "INSERT INTO message VALUES(?1,'s',?2)",
            params![i.to_string(), json!({"role":"assistant"}).to_string()],
        )
        .unwrap();
    }
    db.execute_batch("COMMIT").unwrap();
    let engine = Engine::open(&root.path().join("app")).unwrap();
    let source = Source {
        id: "open".into(),
        provider: "opencode".into(),
        path: path.to_string_lossy().into(),
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
    opencode::scan(&engine.store, &settings, &source, &path).unwrap();
    db.execute("UPDATE message SET data=?1 WHERE id='693'",[json!({"role":"assistant","modelID":"fixture","time":{"completed":now()*1000},"tokens":{"input":10,"output":2,"reasoning":0,"cache":{"read":0,"write":0}}}).to_string()]).unwrap();
    let mut added = 0;
    for _ in 0..3 {
        let result = opencode::scan(&engine.store, &settings, &source, &path).unwrap();
        assert!(result["scannedRecords"].as_u64().unwrap() <= 256);
        added += result["newEvents"].as_u64().unwrap();
    }
    assert_eq!(added, 1);
    assert_eq!(
        opencode::scan(&engine.store, &settings, &source, &path).unwrap()["scannedRecords"],
        0
    );
}
