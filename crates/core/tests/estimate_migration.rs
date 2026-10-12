use aieyes_core::{Engine, estimates::Estimate, store::Store};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn legacy(db: &Connection, table: &str, payload: &Value) {
    db.execute_batch(&format!("CREATE TABLE {table}(id TEXT PRIMARY KEY,account_key TEXT NOT NULL,status TEXT NOT NULL,payload TEXT NOT NULL)" )).unwrap();
    db.execute(
        &format!("INSERT INTO {table} VALUES(?1,?2,?3,?4)"),
        params![
            payload["id"].as_str(),
            payload["accountKey"].as_str(),
            payload["status"].as_str(),
            payload.to_string()
        ],
    )
    .unwrap();
}

#[test]
fn migration_preserves_both_types_same_id_and_original_payloads() {
    let root = tempfile::tempdir().unwrap();
    let quota = json!({"id":"same","kind":"","accountKey":"custom:a","status":"pending","startedAt":100,"baselinePercent":17,"prices":[{"id":"frozen","input":0.2}]});
    let credit = json!({"id":"same","accountKey":"custom:a","status":"pending","startedAt":100,"baselineBalance":"1234567890.123456789","prices":[{"id":"frozen-credit","input":0.3}]});
    let db = Connection::open(root.path().join("aieyes.sqlite")).unwrap();
    legacy(&db, "quota_estimates", &quota);
    legacy(&db, "credit_estimates", &credit);
    drop(db);
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.estimates().unwrap().len(), 2);
    assert_eq!(
        store
            .estimate_for("same", false)
            .unwrap()
            .quota()
            .unwrap()
            .baseline_percent,
        17.0
    );
    assert_eq!(
        store
            .estimate_for("same", true)
            .unwrap()
            .credit()
            .unwrap()
            .baseline_balance
            .as_deref(),
        Some("1234567890.123456789")
    );
    for (kind, expected) in [("quota", quota), ("credits", credit)] {
        let raw: String = store
            .db
            .query_row("SELECT payload FROM estimates WHERE kind=?1", [kind], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), expected);
    }
    let old:i64=store.db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('quota_estimates','credit_estimates')",[],|r|r.get(0)).unwrap();
    assert_eq!(old, 0);
    drop(store);
    let mut engine = Engine::open(root.path()).unwrap();
    assert_eq!(
        engine
            .call("quotaEstimates.get", json!({"id":"same"}))
            .unwrap()["kind"],
        "quota"
    );
    assert_eq!(
        engine
            .call("creditEstimates.get", json!({"id":"same"}))
            .unwrap()["kind"],
        "credits"
    );
    assert_eq!(engine.store.estimates().unwrap().len(), 2);
    engine
        .call("quotaEstimates.stop", json!({"id":"same"}))
        .unwrap();
    assert_eq!(
        engine.store.estimate_for("same", false).unwrap().status,
        "completed"
    );
    assert_eq!(
        engine.store.estimate_for("same", true).unwrap().status,
        "pending"
    );
    engine
        .call("creditEstimates.stop", json!({"id":"same"}))
        .unwrap();
    assert_eq!(
        engine.store.estimate_for("same", true).unwrap().status,
        "completed"
    );
}

#[test]
fn migration_conflict_rolls_back_both_legacy_tables() {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open(root.path().join("aieyes.sqlite")).unwrap();
    let payload = json!({"id":"same","accountKey":"custom:a","status":"completed"});
    legacy(&db, "quota_estimates", &payload);
    legacy(&db, "credit_estimates", &payload);
    db.execute_batch("CREATE TABLE estimates(kind TEXT NOT NULL,id TEXT NOT NULL,account_key TEXT NOT NULL,status TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(kind,id)); INSERT INTO estimates VALUES('credits','same','custom:a','completed','{}')").unwrap();
    assert!(Store::open(root.path()).is_err());
    for table in ["quota_estimates", "credit_estimates", "estimates"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn types_have_disjoint_fields_and_reject_invalid_stored_kind() {
    for (kind, present, absent) in [
        ("quota", "baselinePercent", "baselineBalance"),
        ("credits", "baselineBalance", "baselinePercent"),
    ] {
        let estimate: Estimate = serde_json::from_value(json!({"kind":kind})).unwrap();
        let value = serde_json::to_value(estimate).unwrap();
        assert!(value.get(present).is_some());
        assert!(value.get(absent).is_none());
    }
    assert!(serde_json::from_value::<Estimate>(json!({"kind":"unknown"})).is_err());
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    store
        .db
        .execute(
            "INSERT INTO estimates VALUES('quota','bad','custom:a','completed',?1)",
            [json!({"kind":"credits"}).to_string()],
        )
        .unwrap();
    assert!(store.estimate_for("bad", false).is_err());
    assert!(store.estimates().is_err());
    store
        .db
        .execute("UPDATE estimates SET payload='[]' WHERE id='bad'", [])
        .unwrap();
    assert!(store.estimate_for("bad", false).is_err());
}
