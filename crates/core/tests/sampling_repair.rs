use aieyes_core::{estimates::Estimate, import, models::*, store::Store, usage};
use serde_json::{Value, json};
use std::io::Write;

fn token(stamp: i64, total: u64, last: u64) -> Value {
    json!({"type":"event_msg","timestamp":stamp,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":total,"output_tokens":0},"last_token_usage":{"input_tokens":last,"output_tokens":0}}}})
}
fn source() -> Source {
    Source {
        id: "local".into(),
        path: "/fixture".into(),
        provider: "codex".into(),
        account_id: "a".into(),
        ..Default::default()
    }
}
fn start(s: &Source, p: &mut ParseState, stamp: i64) {
    usage::parse(
        s,
        p,
        &json!({"type":"event_msg","timestamp":stamp,"payload":{"type":"task_started"}}),
    );
}
#[test]
fn verified_task_boundaries_do_not_relocate_accumulated_usage() {
    let s = source();
    let mut state = ParseState::default();
    usage::parse(
        &s,
        &mut state,
        &json!({"type":"session_meta","timestamp":100,"payload":{"id":"s","timestamp":10}}),
    );
    start(&s, &mut state, 101);
    let first = usage::parse(&s, &mut state, &token(105, 100, 100)).unwrap();
    assert_eq!(first.interval_start, Some(101));
    assert_eq!(first.interval_evidence, "task-request-v2");
    start(&s, &mut state, 201);
    let resumed = usage::parse(&s, &mut state, &token(205, 200, 100)).unwrap();
    assert_eq!(resumed.interval_start, Some(201));
    start(&s, &mut state, 301);
    let ambiguous = usage::parse(&s, &mut state, &token(305, 500, 100)).unwrap();
    assert_eq!(ambiguous.interval_start, Some(205));
    assert!(ambiguous.interval_evidence.is_empty());
    usage::parse(
        &s,
        &mut state,
        &json!({"type":"event_msg","timestamp":306,"payload":{"type":"turn_aborted"}}),
    );
    assert!(state.task_started_at.is_none());
}
#[test]
fn parser_upgrade_replays_old_cursors_without_duplicating_events() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let mut s = source();
    let path = root.path().join("session.jsonl");
    s.path = path.to_string_lossy().into();
    let settings = Settings {
        sources: vec![s.clone()],
        accounts: vec![Account {
            id: "a".into(),
            provider: "codex".into(),
            name: "A".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let mut file = std::fs::File::create(&path).unwrap();
    for row in [
        json!({"type":"session_meta","timestamp":10,"payload":{"id":"s"}}),
        json!({"type":"event_msg","timestamp":101,"payload":{"type":"task_started"}}),
        token(105, 100, 100),
    ] {
        writeln!(file, "{row}").unwrap();
    }
    import::scan(&store, &settings, &s).unwrap();
    store
        .db
        .execute(
            "UPDATE files SET state=json_set(state,'$.parserVersion',0)",
            [],
        )
        .unwrap();
    store.db.execute("UPDATE events SET payload=json_set(payload,'$.intervalStart',10,'$.intervalEvidence','')",[]).unwrap();
    let result = import::scan(&store, &settings, &s).unwrap();
    assert_eq!(result["newEvents"], 0);
    assert!(result["readBytes"].as_u64().unwrap() > 0);
    let (count, start): (i64, i64) = store
        .db
        .query_row(
            "SELECT COUNT(*),json_extract(payload,'$.intervalStart') FROM events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, start), (1, 101));
}
#[test]
fn repair_keeps_original_frozen_prices_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let s = source();
    let settings = Settings {
        sources: vec![s.clone()],
        accounts: vec![Account {
            id: "a".into(),
            provider: "codex".into(),
            name: "A".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let price = ModelPrice {
        id: "model".into(),
        input: Some(0.01),
        ..Default::default()
    };
    store.save_prices(std::slice::from_ref(&price)).unwrap();
    let mut state = ParseState {
        session_id: "s".into(),
        model: "model".into(),
        session_started_at: Some(10),
        ..Default::default()
    };
    start(&s, &mut state, 101);
    let verified = usage::parse(&s, &mut state, &token(105, 100, 100)).unwrap();
    let mut legacy = verified.clone();
    legacy.interval_start = Some(10);
    legacy.interval_evidence.clear();
    store
        .put_event(&legacy, std::slice::from_ref(&price), &settings)
        .unwrap();
    let original:Estimate=serde_json::from_value(json!({"id":"sample","kind":"credits","status":"completed","accountKey":"codex:a","sourceIds":["local"],"startedAt":100,"checkpointAt":110,"endedAt":110,"baselineBalance":"30","checkpointBalance":"5","prices":[price],"calculationNote":"边界","capacity":{"ratio":7.0,"samples":10}})).unwrap();
    store
        .db
        .execute(
            "INSERT INTO credit_estimates VALUES('sample','codex:a','completed',?1)",
            [serde_json::to_string(&original).unwrap()],
        )
        .unwrap();
    assert!(store.repair_estimate("sample").is_err());
    store
        .put_event(&verified, std::slice::from_ref(&price), &settings)
        .unwrap();
    store
        .put_event(&legacy, std::slice::from_ref(&price), &settings)
        .unwrap();
    store
        .save_prices(&[ModelPrice {
            input: Some(99.0),
            ..price.clone()
        }])
        .unwrap();
    store.reprice(&settings, false).unwrap();
    let repaired = store.repair_estimate("sample").unwrap();
    assert_eq!(repaired.value_per1000, Some(40.0));
    assert_eq!(repaired.original_estimate_id.as_deref(), Some("sample"));
    assert_eq!(store.repair_estimate("sample").unwrap().id, repaired.id);
    assert!(store.estimate("sample").unwrap().value_per1000.is_none());
    assert_eq!(repaired.capacity.ratio, Some(7.0));
    assert_eq!(repaired.calculation_status, "ready");
    assert_eq!(store.estimates().unwrap().len(), 2);
}
