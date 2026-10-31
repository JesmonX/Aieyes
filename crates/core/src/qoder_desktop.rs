//! Native desktop transcripts stay in the owner's database. Persist only usage.
use crate::{
    models::*,
    store::{Store, expand},
};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) fn database(path: &str) -> Option<PathBuf> {
    let p = expand(path);
    let p = if p.is_dir() { p.join("main.sqlite") } else { p };
    (p.file_name().is_some_and(|n| n == "main.sqlite") && p.is_file()).then_some(p)
}
fn has_table(db: &Connection, table: &str) -> bool {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}
fn tables(db: &Connection) -> Vec<(&'static str, &'static str)> {
    // UI message payloads contain display parts, not billing counters. Desktop
    // requests normally live in the native projects JSONL and state.json; only
    // daemon transcript entries can supply the same usage schema here.
    [("qoder_daemon_session_store_entries", "entry_json")]
        .into_iter()
        .filter(|(t, _)| has_table(db, t))
        .collect()
}
pub(crate) fn scan(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    full: bool,
) -> Result<Value> {
    let db = crate::sqlite_ro::open_ro(path)?;
    db.execute_batch("BEGIN")?;
    let tables = tables(&db);
    anyhow::ensure!(
        !tables.is_empty() || has_table(&db, "chat_session_messages"),
        "桌面会话库版本暂不支持"
    );
    let metadata = std::fs::metadata(path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created());
    let generation = hash(
        &json!([
            identity,
            source.account_id,
            settings.precise_counting,
            settings.counting_generation
        ])
        .to_string(),
    );
    let fingerprint = hash(
        &json!([
            crate::sessions::database_fingerprint(path),
            source.account_id,
            settings.precise_counting,
            settings.counting_generation
        ])
        .to_string(),
    );
    let prices = store.prices()?;
    let mut added = 0;
    let mut scanned = 0;
    let mut invalid = 0;
    for (table, column) in tables {
        let key = format!("{}#{table}", path.display());
        let previous = store.cursor(&source.id, &key)?;
        if !full
            && previous.as_ref().is_some_and(|(sig, _, state)| {
                sig == &fingerprint
                    && state.parser_version == crate::usage::parser_version(&source.provider)
            })
        {
            continue;
        }
        let max: u64 = db.query_row(
            &format!("SELECT COALESCE(MAX(rowid),0) FROM {table}"),
            [],
            |r| r.get(0),
        )?;
        let mut offset = if full {
            0
        } else {
            previous
                .filter(|(_, _, state)| {
                    state.parser_version == crate::usage::parser_version(&source.provider)
                        && state
                            .database_checkpoint
                            .as_ref()
                            .is_some_and(|c| c.identity == generation)
                })
                .and_then(|(_, offset, state)| {
                    let anchor: Option<String> = db
                        .query_row(
                            &format!(
                                "SELECT session_id || ':' || {column} FROM {table} WHERE rowid=?1"
                            ),
                            [offset],
                            |r| r.get(0),
                        )
                        .optional()
                        .ok()
                        .flatten();
                    (anchor.map(|s| hash(&s)).as_deref()
                        == state
                            .database_checkpoint
                            .as_ref()
                            .map(|c| c.anchor.as_str()))
                    .then_some(offset)
                })
                .filter(|o| *o <= max)
                .unwrap_or(0)
        };
        // Daemon transcript entries are appended. An anchor detects replacement
        // or truncation; counting/account changes replay all entries.
        while offset < max {
            crate::operations::check()?;
            let rows = db.prepare(&format!("SELECT rowid,session_id,{column} FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT 512"))?.query_map([offset], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            let mut events = Vec::new();
            let mut anchor = String::new();
            for (id, session, raw) in rows {
                offset = id;
                anchor = hash(&format!("{session}:{raw}"));
                scanned += 1;
                if raw.len() > 8 * 1024 * 1024 {
                    invalid += 1;
                    continue;
                }
                let Ok(mut v) = serde_json::from_str::<Value>(&raw) else {
                    invalid += 1;
                    continue;
                };
                if !v.is_object() {
                    invalid += 1;
                    continue;
                }
                if v["sessionId"].is_null() {
                    v["sessionId"] = json!(session);
                }
                let mut state = ParseState {
                    session_id: session,
                    ..Default::default()
                };
                if let Some(event) = crate::usage::parse(source, &mut state, &v) {
                    events.push(event);
                }
            }
            let tx = store.write_transaction()?;
            let current = crate::import::current_settings(store, settings, source)?;
            for event in events {
                added += u64::from(store.put_event(&event, &prices, &current)?);
            }
            let state = ParseState {
                parser_version: crate::usage::parser_version(&source.provider),
                database_checkpoint: Some(DatabaseCheckpoint {
                    identity: generation.clone(),
                    anchor,
                    ..Default::default()
                }),
                ..Default::default()
            };
            store.set_cursor(
                &source.id,
                &key,
                if offset >= max { &fingerprint } else { "" },
                offset,
                &state,
            )?;
            tx.commit()?;
        }
    }
    let result = json!({"updatedAt":now(),"newEvents":added,"records":scanned,"malformedLines":invalid,"partial":invalid>0});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    Ok(result)
}
pub(crate) fn activity(path: &Path, now: u64) -> Result<Vec<crate::native_sessions::Activity>> {
    let db = crate::sqlite_ro::open_ro(path)?;
    db.execute_batch("BEGIN")?;
    if !has_table(&db, "chat_sessions") || !has_table(&db, "chat_session_turn_states") {
        return Ok(vec![]);
    }
    // Current apps store status on messages and leave turn_states empty. Older
    // snapshots can still provide a turn state. Correlate both joins by session.
    let messages = has_table(&db, "chat_session_messages");
    let query = if messages {
        "SELECT s.session_id,COALESCE(m.turn_id,t.turn_id,''),CASE WHEN m.role='user' THEN 'running' ELSE COALESCE(m.status,t.status,'') END,COALESCE(m.updated_at,s.updated_at),COALESCE(s.owner_session_id,'') FROM chat_sessions s LEFT JOIN chat_session_turn_states t ON t.session_id=s.session_id AND t.revision=(SELECT MAX(t2.revision) FROM chat_session_turn_states t2 WHERE t2.session_id=s.session_id) LEFT JOIN chat_session_messages m ON m.rowid=(SELECT m2.rowid FROM chat_session_messages m2 WHERE m2.session_id=s.session_id AND m2.role IN ('user','assistant') ORDER BY m2.sequence DESC LIMIT 1) WHERE s.deleted_at IS NULL ORDER BY s.updated_at DESC LIMIT 512"
    } else {
        "SELECT s.session_id,COALESCE(t.turn_id,''),COALESCE(t.status,''),s.updated_at,COALESCE(s.owner_session_id,'') FROM chat_sessions s LEFT JOIN chat_session_turn_states t ON t.session_id=s.session_id AND t.revision=(SELECT MAX(t2.revision) FROM chat_session_turn_states t2 WHERE t2.session_id=s.session_id) WHERE s.deleted_at IS NULL ORDER BY s.updated_at DESC LIMIT 512"
    };
    let rows = db
        .prepare(query)?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, rusqlite::types::Value>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, turn, status, time, parent)| {
            let at = match time {
                rusqlite::types::Value::Integer(n) => {
                    if n > 10_000_000_000 {
                        n as u64
                    } else {
                        n.max(0) as u64 * 1000
                    }
                }
                rusqlite::types::Value::Text(s) => chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()?
                    .timestamp_millis()
                    .max(0) as u64,
                _ => return None,
            };
            if now.saturating_sub(at / 1000) > 86400 {
                return None;
            }
            let kind = match status.as_str() {
                "running" | "pending" | "streaming" | "in_progress" => "activity",
                "waiting_for_approval" | "waiting_for_input" => "waiting",
                "completed" | "done" | "failed" | "cancelled" | "interrupted" => "finish",
                _ => return None,
            };
            Some((id, turn, kind.into(), at, parent))
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wal_sessions_are_visible_and_foreign_database_is_read_only() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("main.sqlite");
        let writer = Connection::open(&p).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE chat_sessions(session_id TEXT,updated_at INTEGER,owner_session_id TEXT,deleted_at INTEGER); CREATE TABLE chat_session_turn_states(revision INTEGER,session_id TEXT,turn_id TEXT,status TEXT); INSERT INTO chat_sessions VALUES('s',1700000000000,NULL,NULL); INSERT INTO chat_session_turn_states VALUES(1,'s','t','completed');").unwrap();
        let rows = activity(&p, 1700000000).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "finish");
        assert!(
            crate::sqlite_ro::open_ro(&p)
                .unwrap()
                .execute("DELETE FROM chat_sessions", [])
                .is_err()
        );
    }
    #[test]
    fn current_apps_use_message_status_without_turn_states() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE chat_sessions(session_id TEXT,updated_at INTEGER,owner_session_id TEXT,deleted_at INTEGER);
          CREATE TABLE chat_session_turn_states(revision INTEGER,session_id TEXT,turn_id TEXT,status TEXT);
          CREATE TABLE chat_session_messages(session_id TEXT,turn_id TEXT,sequence INTEGER,role TEXT,status TEXT,updated_at INTEGER);
          INSERT INTO chat_sessions VALUES('done',1700000000000,NULL,NULL),('active',1700000000000,NULL,NULL),('old',1600000000000,NULL,NULL);
          INSERT INTO chat_session_messages VALUES('done','t1',1,'assistant','completed',1700000000000),('active','t2',1,'user','completed',1700000000000),('old','t0',1,'assistant','completed',1600000000000);").unwrap();
        let rows = activity(&path, 1700000000).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.0 == "done" && r.2 == "finish"));
        assert!(rows.iter().any(|r| r.0 == "active" && r.2 == "activity"));
    }
    #[test]
    fn desktop_usage_replays_and_duplicate_entries_do_not_double_count() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app");
        std::fs::create_dir(&app).unwrap();
        let path = app.join("main.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE qoder_daemon_session_store_entries(session_id TEXT,entry_json TEXT);CREATE TABLE chat_session_messages(session_id TEXT,payload_json TEXT);").unwrap();
        let row = json!({"type":"assistant","sessionId":"session","uuid":"response","timestamp":"2026-10-31T00:00:00Z","message":{"id":"m","model":"qfmodel","usage":{"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":30,"credits":0.1}}});
        for table in ["qoder_daemon_session_store_entries"; 2] {
            writer
                .execute(
                    &format!("INSERT INTO {table} VALUES(?1,?2)"),
                    rusqlite::params!["session", row.to_string()],
                )
                .unwrap();
        }
        let store = Store::open(&tmp.path().join("aieyes")).unwrap();
        let source = Source {
            id: "cn-desktop".into(),
            provider: "qoder-cn".into(),
            path: app.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let settings = store.settings().unwrap();
        scan(&store, &settings, &source, &path, false).unwrap();
        scan(&store, &settings, &source, &path, true).unwrap();
        let count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE provider='qoder-cn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let payload: String = store
            .db
            .query_row(
                "SELECT payload FROM events WHERE provider='qoder-cn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let v: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["tokens"]["input"], 70);
        assert!(!payload.contains("message"));
    }
}
