//! Small durable invalidation counters; never put configuration or credentials on this channel.
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::Duration,
};

#[derive(Default)]
pub struct Signal {
    generation: Mutex<u64>,
    changed: Condvar,
}
impl Signal {
    pub fn generation(&self) -> u64 {
        *self.generation.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn wait(&self, generation: u64, timeout: Duration) {
        let guard = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .changed
            .wait_timeout_while(guard, timeout, |current| *current == generation);
    }
    pub fn wake(&self) {
        *self.generation.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.changed.notify_all();
    }
}
static SIGNALS: OnceLock<Mutex<HashMap<PathBuf, Weak<Signal>>>> = OnceLock::new();
pub fn signal(root: &Path) -> Arc<Signal> {
    let mut all = SIGNALS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    if let Some(value) = all.get(&key).and_then(Weak::upgrade) {
        return value;
    }
    all.retain(|_, value| value.strong_count() > 0);
    let value = Arc::new(Signal::default());
    all.insert(key, Arc::downgrade(&value));
    value
}
pub(crate) fn setup(db: &Connection) -> Result<()> {
    for (domain, table, condition) in [
        (
            "settings",
            "kv",
            "{row}.key='settings' OR {row}.key LIKE 'deleted-account:%'",
        ),
        (
            "pets",
            "kv",
            "{row}.key IN ('pet.current.v1','pet.library.v1')",
        ),
        (
            "accounts",
            "kv",
            "{row}.key LIKE 'account-status:%' OR {row}.key LIKE 'account-status-revision:%'",
        ),
        (
            "quotas",
            "kv",
            "{row}.key LIKE 'quota-attempt:%' OR {row}.key LIKE 'quota-result:%'",
        ),
        ("quotas", "quotas", "1"),
        ("estimates", "estimates", "1"),
        ("prices", "price_history", "1"),
        ("notifications", "quota_reset_notifications", "1"),
        ("sources", "source_status", "1"),
        ("sources", "kv", "{row}.key LIKE 'automatic-status:%'"),
    ] {
        db.execute(
            "INSERT OR IGNORE INTO kv VALUES(?1,'0')",
            [format!("runtime.revision:{domain}")],
        )?;
        for (action, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            let condition = condition.replace("{row}", row);
            let unchanged = if action == "UPDATE" {
                match table {
                    "kv" => " AND OLD.value IS NOT NEW.value",
                    "price_history" => "",
                    _ => " AND OLD.payload IS NOT NEW.payload",
                }
            } else {
                ""
            };
            db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS runtime_{domain}_{table}_{action} AFTER {action} ON {table} WHEN ({condition}){unchanged} BEGIN UPDATE kv SET value=CAST(value AS INTEGER)+1 WHERE key='runtime.revision:{domain}'; END;"))?;
        }
    }
    // Reuse existing event revision triggers instead of adding another write per imported event.
    db.execute_batch("CREATE INDEX IF NOT EXISTS estimates_display_order ON estimates(account_key,kind,json_extract(payload,'$.startedAt') DESC,COALESCE(json_extract(payload,'$.repairedAt'),0) DESC);
        CREATE INDEX IF NOT EXISTS estimates_replaces_text ON estimates(kind,CAST(json_extract(payload,'$.originalEstimateId') AS TEXT));")?;
    Ok(())
}
pub fn revisions(db: &Connection) -> Result<Value> {
    let mut result = serde_json::Map::new();
    let mut query = db.prepare_cached("SELECT substr(key,18),CAST(value AS INTEGER) FROM kv WHERE key IN ('runtime.revision:settings','runtime.revision:pets','runtime.revision:accounts','runtime.revision:quotas','runtime.revision:estimates','runtime.revision:prices','runtime.revision:notifications','runtime.revision:sources')")?;
    for row in query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (key, value) = row?;
        result.insert(key, json!(value));
    }
    let usage: i64 = db.query_row(
        "SELECT COALESCE(SUM(revision),0) FROM usage_day_revision",
        [],
        |r| r.get(0),
    )?;
    result.insert("usage".into(), json!(usage));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_are_transactional_and_unchanged_reads_do_not_write() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let before = revisions(&store.db).unwrap();
        let changes = store.db.total_changes();
        assert_eq!(before, revisions(&store.db).unwrap());
        assert_eq!(changes, store.db.total_changes());
        store
            .db
            .execute_batch("BEGIN;INSERT INTO kv VALUES('pet.current.v1','{}');ROLLBACK;")
            .unwrap();
        assert_eq!(before, revisions(&store.db).unwrap());
        store
            .db
            .execute("INSERT INTO kv VALUES('pet.current.v1','{}')", [])
            .unwrap();
        assert_eq!(
            revisions(&store.db).unwrap()["pets"].as_i64().unwrap(),
            before["pets"].as_i64().unwrap() + 1
        );
        let known = revisions(&store.db).unwrap();
        store
            .db
            .execute("UPDATE kv SET value=value WHERE key='pet.current.v1'", [])
            .unwrap();
        assert_eq!(known, revisions(&store.db).unwrap());
    }
    #[test]
    fn display_is_bounded_and_keeps_values_from_older_canonical_records() {
        let temp = tempfile::tempdir().unwrap();
        let mut engine = crate::Engine::open(temp.path()).unwrap();
        for i in 0..1000 {
            let row = json!({"id":format!("q{i}"),"accountKey":"codex:a","status":"completed","startedAt":i,"weeklyValue":if i==5 {Some(42.0)} else {None},"prices":[{"id":"large-frozen-price"}]});
            engine
                .store
                .db
                .execute(
                    "INSERT INTO estimates VALUES('quota',?1,'codex:a','completed',?2)",
                    rusqlite::params![format!("q{i}"), row.to_string()],
                )
                .unwrap();
        }
        let result = engine
            .call("dashboard.summary", json!({"estimateHistory":"display"}))
            .unwrap();
        let rows = result["quotaEstimates"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r["weeklyValue"] == 42.0));
        assert!(
            rows.iter()
                .all(|r| r["prices"].as_array().unwrap().is_empty())
        );
        assert_eq!(
            engine
                .call("quotaEstimates.list", json!({"accountKey":"codex:other"}))
                .unwrap(),
            json!([])
        );
        assert_eq!(
            engine.call("dashboard.summary", json!({})).unwrap()["quotaEstimates"]
                .as_array()
                .unwrap()
                .len(),
            1000
        );
        engine
            .store
            .db
            .execute(
                "UPDATE estimates SET payload=json_set(payload,'$.weeklyValue',77) WHERE id='q999'",
                [],
            )
            .unwrap();
        assert!(
            engine
                .call("dashboard.summary", json!({"estimateHistory":"display"}))
                .unwrap()["quotaEstimates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["weeklyValue"].as_f64() == Some(77.0))
        );
    }
    #[test]
    fn display_cache_never_retains_rolled_back_samples_or_invalidates_on_unrelated_writes() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        store.db.execute("INSERT INTO estimates VALUES('quota','q','codex:a','completed',?1)", [json!({"id":"q","accountKey":"codex:a","status":"completed","startedAt":1,"weeklyValue":10}).to_string()]).unwrap();
        assert_eq!(
            store.display_estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(10.0)
        );
        store
            .db
            .execute_batch(
                "BEGIN;UPDATE estimates SET payload=json_set(payload,'$.weeklyValue',20); ",
            )
            .unwrap();
        assert_eq!(
            store.display_estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(20.0)
        );
        store
            .db
            .execute_batch(
                "ROLLBACK;BEGIN;UPDATE estimates SET payload=json_set(payload,'$.weeklyValue',30);",
            )
            .unwrap();
        assert_eq!(
            store.display_estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(30.0)
        );
        store.db.execute_batch("COMMIT;").unwrap();
        assert_eq!(
            store.display_estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(30.0)
        );
        let revision = store.estimate_display.borrow().as_ref().unwrap().0;
        store
            .db
            .execute("INSERT INTO kv VALUES('unrelated-cache','1')", [])
            .unwrap();
        assert_eq!(
            store.display_estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(30.0)
        );
        assert_eq!(
            store.estimate_display.borrow().as_ref().unwrap().0,
            revision
        );
    }
    #[test]
    fn replacement_lookup_uses_the_complete_index() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let mut statement=store.db.prepare("EXPLAIN QUERY PLAN SELECT id FROM estimates e WHERE NOT EXISTS(SELECT 1 FROM estimates replacement WHERE replacement.kind=e.kind AND CAST(json_extract(replacement.payload,'$.originalEstimateId') AS TEXT)=e.id)").unwrap();
        let plan = statement
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(";");
        assert!(
            plan.contains("kind=? AND <expr>=?"),
            "Replacement lookup must seek both type and ID, not scan each type: {plan}"
        );
    }
    #[test]
    fn settings_cache_sees_external_writes_and_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let before = store.settings().unwrap();
        let external = Connection::open(temp.path().join("aieyes.sqlite")).unwrap();
        external
            .execute(
                "UPDATE kv SET value=json_set(value,'$.refreshSeconds',600) WHERE key='settings'",
                [],
            )
            .unwrap();
        assert_eq!(store.settings().unwrap().refresh_seconds, 600);
        store.db.execute_batch("BEGIN;UPDATE kv SET value=json_set(value,'$.refreshSeconds',900) WHERE key='settings';").unwrap();
        assert_eq!(store.settings().unwrap().refresh_seconds, 900);
        store.db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(store.settings().unwrap().refresh_seconds, 600);
        let _ = before;
    }
    #[test]
    fn empty_notifications_do_not_contend_with_an_active_writer() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let other = Connection::open(temp.path().join("aieyes.sqlite")).unwrap();
        other.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let started = std::time::Instant::now();
        let changes = store.db.total_changes();
        assert!(store.take_reset_notifications(0).unwrap().is_empty());
        assert!(started.elapsed() < Duration::from_millis(200));
        assert_eq!(changes, store.db.total_changes());
        other.execute_batch("ROLLBACK;").unwrap();
    }
}
