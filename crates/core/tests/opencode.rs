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
