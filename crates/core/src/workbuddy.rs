//! WorkBuddy's local Credits. WorkBuddy 5.5.6 persists usage per session in
//! `workbuddy.db` `session_usage` (`SqliteConversationUsagePort.persistUsage`):
//! one row per session, rewritten in place, with `credit_json` holding
//! `{requestId: credits}`. Its transcripts carry no usage, so this table is the
//! only Credits record. The database is opened read-only and never checkpointed;
//! prompts, titles and the encrypted login are never read.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

const TABLE: &str = "session_usage";
const PAGE: usize = 256;

/// Requests and their Credits in one `credit_json` value, by WorkBuddy's own
/// rule (`parseCredit`): an object whose finite numeric values are Credits.
pub(crate) fn credits(raw: &str) -> Vec<(String, f64)> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => map
            .into_iter()
            .filter_map(|(request, value)| {
                let n = value.as_f64()?;
                (n.is_finite() && n >= 0.0 && !request.is_empty() && request.len() <= 256)
                    .then_some((request, n))
            })
            .collect(),
        _ => vec![],
    }
}

/// Raw `sessions.user_id` values of a hashed identity (`workbuddy:{hash}`).
fn users(db: &Connection, identity: &str) -> Result<Vec<String>> {
    let Some(hashed) = identity.strip_prefix("workbuddy:") else {
        return Ok(vec![]);
    };
    if !crate::opencode::table_exists(db, "sessions")? {
        return Ok(vec![]);
    }
    let all = db
        .prepare("SELECT DISTINCT user_id FROM sessions WHERE user_id<>'' LIMIT 256")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(all.into_iter().filter(|u| hash(u) == hashed).collect())
}

/// Credits of sessions updated since `since` (ms) in one local database. With an
/// identity, only that user's sessions count, so a remembered earlier login keeps
/// its own total. Returns (credits, requests).
pub(crate) fn month_credits(path: &Path, since: i64, identity: Option<&str>) -> Result<(f64, i64)> {
    let db = crate::sqlite_ro::open_ro(path)?;
    db.execute_batch("BEGIN")?;
    if !crate::opencode::table_exists(&db, TABLE)? {
        return Ok((0.0, 0));
    }
    let users = match identity {
        Some(identity) => {
            let users = users(&db, identity)?;
            if users.is_empty() {
                return Ok((0.0, 0));
            }
            Some(users)
        }
        None => None,
    };
    let filter = if users.is_some() {
        " AND session_id IN (SELECT id FROM sessions WHERE user_id IN (SELECT value FROM json_each(?2)))"
    } else {
        " AND ?2 IS NULL"
    };
    let raw = db
        .prepare(&format!(
            "SELECT credit_json FROM {TABLE} WHERE updated_at>=?1 AND credit_json IS NOT NULL AND credit_json<>''{filter}"
        ))?
        .query_map(
            params![since, users.map(|u| json!(u).to_string())],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut used, mut requests) = (0.0, 0);
    for row in raw {
        for (_, n) in credits(&row) {
            used += n;
            requests += 1;
        }
    }
    Ok((used, requests))
}

/// Imports each request's Credits as a Credits-only usage event. Rows are read
/// in `updated_at` order from the last high-water mark; rows at the mark are read
/// again, which is idempotent. A request keeps the time it was first seen (the
/// row's `updated_at` then); later reads only update its Credits.
pub(crate) fn scan(
    store: &Store,
    settings: &Settings,
    source: &Source,
    full: bool,
) -> Result<Value> {
    let Some(path) = crate::sessions::workbuddy_database(&source.path) else {
        ensure!(expand(&source.path).exists(), "数据目录不存在");
        return finish(store, settings, source, 0, 0, 0);
    };
    let key = path.to_string_lossy().into_owned();
    let scope = hash(&json!(["workbuddy-credits-v2", source.account_id, source.path]).to_string());
    let fingerprint = crate::sessions::database_fingerprint(&path);
    let version = usage::parser_version(&source.provider);
    let saved = if full {
        None
    } else {
        store
            .cursor(&source.id, &key)?
            .filter(|(signature, _, state)| {
                *signature == scope
                    && state.parser_version == version
                    && state.precise_counting == Some(settings.precise_counting)
                    && state.counting_generation == settings.counting_generation
            })
    };
    let metadata = std::fs::metadata(&path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created());
    let (mut mark, mut checkpoint) = saved
        .map(|(_, offset, state)| (offset as i64, state.database_checkpoint.unwrap_or_default()))
        .unwrap_or_default();
    if checkpoint.identity != identity || checkpoint.table != TABLE {
        // A replaced database starts over; already imported requests keep their identity.
        mark = 0;
        checkpoint = DatabaseCheckpoint {
            identity,
            table: TABLE.into(),
            ..Default::default()
        };
    }
    if !checkpoint.fingerprint.is_empty() && checkpoint.fingerprint == fingerprint {
        return finish(store, settings, source, 0, 0, 1);
    }
    checkpoint.fingerprint.clear();
    let mut state = ParseState {
        parser_version: version,
        precise_counting: Some(settings.precise_counting),
        counting_generation: settings.counting_generation,
        ..Default::default()
    };
    let db = crate::sqlite_ro::open_ro(&path)?;
    db.execute_batch("BEGIN")?;
    let (mut inserted, mut scanned) = (0, 0);
    if crate::opencode::table_exists(&db, TABLE)? {
        let query = format!(
            "SELECT session_id,updated_at,credit_json FROM {TABLE} WHERE updated_at>?1 OR (updated_at=?1 AND session_id>?2) ORDER BY updated_at,session_id LIMIT {PAGE}"
        );
        let prices = store.prices()?;
        let mut after = (mark, String::new());
        loop {
            crate::operations::check()?;
            let rows = db
                .prepare(&query)?
                .query_map(params![after.0, after.1], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let Some(last) = rows.last() else {
                break;
            };
            after = (last.1, last.0.clone());
            mark = mark.max(last.1);
            scanned += rows.len() as u64;
            // Native reads and parsing stay outside the writer lock.
            let events: Vec<_> = rows
                .iter()
                .flat_map(|(session, at, raw)| {
                    raw.as_deref()
                        .map(credits)
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(move |(request, credits)| {
                            event(source, session, *at, &request, credits)
                        })
                })
                .collect();
            let tx = store.write_transaction()?;
            let current = crate::import::current_settings(store, settings, source)?;
            for event in &events {
                if store.put_event(event, &prices, &current)? {
                    inserted += 1;
                }
            }
            state.database_checkpoint = Some(checkpoint.clone());
            store.set_cursor(&source.id, &key, &scope, mark.max(0) as u64, &state)?;
            tx.commit()?;
            if rows.len() < PAGE {
                break;
            }
        }
    }
    drop(db);
    if fingerprint == crate::sessions::database_fingerprint(&path) {
        checkpoint.fingerprint = fingerprint;
    }
    state.database_checkpoint = Some(checkpoint);
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.set_cursor(&source.id, &key, &scope, mark.max(0) as u64, &state)?;
    tx.commit()?;
    finish(store, settings, source, inserted, scanned, 1)
}

fn event(
    source: &Source,
    session: &str,
    at: i64,
    request: &str,
    credits: f64,
) -> Option<UsageEvent> {
    // A session's current model is not evidence of earlier requests' models.
    // Stable native request IDs make replay idempotent within the account/source.
    let origin = format!(
        "workbuddy-credit:{}",
        hash(&json!([session, request]).to_string())
    );
    let row = json!({"id":request,"sessionId":session,"timestamp":at.max(0) / 1000,"model":"unknown",
        "tokens":{},"nativeCredits":{"credits":credits},"inheritance":{"origin":origin,"inherited":false}});
    usage::parse_normalized(source, &mut ParseState::default(), &row)
}

fn finish(
    store: &Store,
    settings: &Settings,
    source: &Source,
    inserted: u64,
    scanned: u64,
    files: u64,
) -> Result<Value> {
    let result = json!({"updatedAt":now(),"newEvents":inserted,"files":files,"scannedRecords":scanned,"format":"workbuddy-native"});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/workbuddy/workbuddy.sql");

    fn database(dir: &Path) -> Connection {
        let db = Connection::open(dir.join("workbuddy.db")).unwrap();
        db.execute_batch(FIXTURE).unwrap();
        db
    }

    #[test]
    fn credit_json_follows_workbuddy_parse_credit() {
        let mut rows = credits(r#"{"req-a":0.5,"req-b":1,"bad":"2","neg":-1,"nan":null}"#);
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows, [("req-a".into(), 0.5), ("req-b".into(), 1.0)]);
        for raw in ["", "not json", "1.5", "[1]", r#""x""#, "{}"] {
            assert!(credits(raw).is_empty(), "{raw}");
        }
    }

    #[test]
    fn month_credits_count_only_this_identity_and_period() {
        let dir = tempfile::tempdir().unwrap();
        database(dir.path());
        let path = dir.path().join("workbuddy.db");
        let month = chrono::Utc::now().timestamp_millis() - 86_400_000;
        let (all, requests) = month_credits(&path, month, None).unwrap();
        // Every session of this period counts, background automations included:
        // they consume the same Credits pool.
        assert!((all - 4.375).abs() < 1e-9, "{all}");
        assert_eq!(requests, 5);
        let mine = format!("workbuddy:{}", hash("fixture-user"));
        let (own, requests) = month_credits(&path, month, Some(&mine)).unwrap();
        assert!((own - 4.25).abs() < 1e-9, "{own}");
        assert_eq!(requests, 4);
        let other = format!("workbuddy:{}", hash("fixture-earlier-user"));
        assert_eq!(month_credits(&path, month, Some(&other)).unwrap().1, 1);
        let unknown = format!("workbuddy:{}", hash("someone-else"));
        assert_eq!(
            month_credits(&path, month, Some(&unknown)).unwrap(),
            (0.0, 0)
        );
    }

    #[test]
    fn history_imports_each_request_once_and_keeps_its_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let store = Store::open(&data).unwrap();
        let home = dir.path().join("wb");
        std::fs::create_dir(&home).unwrap();
        let db = database(&home);
        let source = Source {
            id: "wb".into(),
            provider: "workbuddy".into(),
            path: home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let settings = store.settings().unwrap();
        let first = scan(&store, &settings, &source, false).unwrap();
        assert_eq!(first["newEvents"], 6);
        let credits = |store: &Store| -> (f64, i64) {
            store
                .db
                .query_row(
                    "SELECT SUM(json_extract(payload,'$.nativeCredits.credits')),COUNT(*) FROM events WHERE provider='workbuddy'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };
        let (sum, count) = credits(&store);
        assert!((sum - 104.375).abs() < 1e-9, "{sum}");
        assert_eq!(count, 6);
        let stamp = |store: &Store| -> i64 {
            store
                .db
                .query_row(
                    "SELECT stamp FROM events WHERE json_extract(payload,'$.sessionId')='user-chat' ORDER BY stamp LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let before = stamp(&store);
        // An unchanged database is not opened again.
        assert_eq!(
            scan(&store, &settings, &source, false).unwrap()["scannedRecords"],
            0
        );
        // A new request of an existing session, written later: one new event; the
        // earlier request keeps the time it was first seen.
        db.execute(
            "UPDATE session_usage SET credit_json=json_set(credit_json,'$.reqnew',0.5),updated_at=updated_at+60000 WHERE session_id='user-chat'",
            [],
        )
        .unwrap();
        let next = scan(&store, &settings, &source, false).unwrap();
        assert_eq!(next["newEvents"], 1);
        assert_eq!(stamp(&store), before);
        assert_eq!(credits(&store).1, 7);
        // A full replay only confirms what is stored.
        assert_eq!(
            scan(&store, &settings, &source, true).unwrap()["newEvents"],
            0
        );
        assert_eq!(credits(&store).1, 7);
        let model: String = store
            .db
            .query_row(
                "SELECT model FROM events WHERE json_extract(payload,'$.sessionId')='user-chat' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model, "unknown");
        db.execute("UPDATE sessions SET model='a-different-current-model'", [])
            .unwrap();
        assert_eq!(
            scan(&store, &settings, &source, true).unwrap()["newEvents"],
            0
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE model<>'unknown'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let payloads: Vec<String> = store
            .db
            .prepare("SELECT payload FROM events")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(payloads.iter().all(|p| !p.contains("PRIVATE")));

        // Consume the bundled SSH projection through the same normalized parser.
        // Reading this source through either transport must not charge twice.
        let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
        let request = json!({"protocol":2,"path":home,"provider":"workbuddy","cursors":{}});
        let output = std::process::Command::new("python3")
            .args(["-c", "import json,pathlib,sys;p=pathlib.Path(sys.argv[1]);exec(compile((p/'remote_history.py').read_text(encoding='utf-8'),'<embedded-history>','exec'),{'__name__':'__main__','REQUEST':json.loads(sys.argv[2]),'NATIVE_DATABASE_SCRIPT':(p/'native_database_remote.py').read_text(encoding='utf-8')})"])
            .arg(scripts).arg(request.to_string()).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let frames: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(frames.last().unwrap()["partial"], false);
        let before = credits(&store);
        let original_stamp = stamp(&store);
        let prices = store.prices().unwrap();
        let mut parsed = 0;
        for row in frames
            .iter()
            .filter_map(|frame| frame["events"].as_array())
            .flatten()
        {
            let event = usage::parse(&source, &mut ParseState::default(), row).unwrap();
            assert_eq!(event.model, "unknown");
            assert!(event.tokens.total() == 0 && event.native_credits.is_some());
            store.put_event(&event, &prices, &settings).unwrap();
            parsed += 1;
        }
        assert_eq!(parsed, 7);
        assert_eq!(credits(&store), before);
        assert_eq!(stamp(&store), original_stamp);
    }
}
