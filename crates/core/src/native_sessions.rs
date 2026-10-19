//! Bounded live-state projection from append/update native SQLite tables.
//! Cold start enumerates metadata once; later polls read appended rows and the
//! latest row of at most 256 visible sessions. No transcript content is loaded.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

#[derive(Clone, Default)]
pub(crate) struct State {
    identity: String,
    table: String,
    offset: i64,
    anchor: String,
    rows: HashMap<String, Row>,
    #[cfg(test)]
    scanned: usize,
}
#[derive(Clone)]
struct Row {
    rowid: i64,
    session: String,
    turn: String,
    created: u64,
    at: u64,
    data: Value,
}
impl Row {
    fn anchor(&self) -> String {
        format!("{}:{}", self.session, self.turn)
    }
}
pub(crate) type Activity = (String, String, String, u64);

pub(crate) fn refresh(
    provider: &str,
    path: &Path,
    mut state: State,
    now: u64,
) -> Result<(State, Vec<Activity>)> {
    #[cfg(test)]
    {
        state.scanned = 0;
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    db.execute_batch("PRAGMA query_only=ON;BEGIN")?;
    let (table, columns) = if provider == "opencode" {
        let (table, v2) = crate::opencode::schema(&db)?;
        let role = if v2 {
            "type"
        } else {
            "json_extract(data,'$.role')"
        };
        (
            table,
            format!(
                "rowid,session_id,id,time_created,time_updated,CASE WHEN json_valid(data) THEN json_object('role',{role},'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'error',json_extract(data,'$.error.name')) ELSE '{{}}' END"
            ),
        )
    } else {
        ("turn_usage","rowid,session_id,turn_id,started_at,COALESCE(NULLIF(completed_at,0),started_at),json_object('status',status)".into())
    };
    let meta = std::fs::metadata(path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created());
    let query = format!("SELECT {columns} FROM {table}");
    let decode = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
        Ok(Row {
            rowid: r.get(0)?,
            session: r.get(1)?,
            turn: r.get(2)?,
            created: r.get::<_, i64>(3)?.max(0) as u64,
            at: r.get::<_, i64>(4)?.max(0) as u64,
            data: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        })
    };
    let anchor = db
        .query_row(&format!("{query} WHERE rowid=?1"), [state.offset], decode)
        .optional()?;
    if state.identity != identity
        || state.table != table
        || (state.offset > 0 && anchor.as_ref().map(Row::anchor).as_deref() != Some(&state.anchor))
    {
        state = State {
            identity,
            table: table.into(),
            ..Default::default()
        };
    }
    // Refresh rows already visible, including delayed completion of old turns.
    if !state.rows.is_empty() {
        let ids = state
            .rows
            .values()
            .map(|row| row.rowid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let rows = db
            .prepare(&format!("{query} WHERE rowid IN ({ids})"))?
            .query_map([], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        #[cfg(test)]
        {
            state.scanned += rows.len();
        }
        state.rows.clear();
        for row in rows {
            state.rows.insert(row.session.clone(), row);
        }
    }
    loop {
        crate::operations::check()?;
        let rows = db
            .prepare(&format!("{query} WHERE rowid>?1 ORDER BY rowid LIMIT 256"))?
            .query_map([state.offset], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        #[cfg(test)]
        {
            state.scanned += rows.len();
        }
        for row in rows {
            state.offset = row.rowid;
            state.anchor = row.anchor();
            if now.saturating_sub(row.at / 1000) >= 86400 {
                continue;
            }
            if state
                .rows
                .get(&row.session)
                .is_none_or(|old| (row.created, &row.turn) >= (old.created, &old.turn))
            {
                state.rows.insert(row.session.clone(), row);
            }
        }
        trim(&mut state, now);
    }
    trim(&mut state, now);
    let parts = provider == "opencode"
        && table == "message"
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='part')",
            [],
            |r| r.get::<_, bool>(0),
        )?;
    let mut result = Vec::new();
    for row in state.rows.values() {
        let mut kind = if provider == "zcode" {
            match row.data["status"].as_str() {
                Some("completed") => "complete",
                Some("error") => "failed",
                Some("cancelled") => "interrupted",
                Some("running") => "start",
                _ => "unknown",
            }
        } else {
            let error = row.data["error"].as_str().unwrap_or("");
            let finish = row.data["finish"].as_str().unwrap_or("");
            if ["MessageAbortedError", "AbortedError"].contains(&error) {
                "interrupted"
            } else if !error.is_empty() {
                "failed"
            } else if row.data["role"] == "user" {
                "start"
            } else if !row.data["completed"].is_null() && ["stop", "end_turn"].contains(&finish) {
                "complete"
            } else if !row.data["completed"].is_null() && finish == "length" {
                "failed"
            } else {
                "thinking"
            }
        };
        if parts && !["complete", "failed", "interrupted"].contains(&kind) {
            let tools=db.prepare("SELECT json_extract(data,'$.tool') FROM part WHERE message_id=?1 AND json_valid(data) AND json_extract(data,'$.type')='tool' AND json_extract(data,'$.state.status') IN ('pending','running') LIMIT 64")?.query_map([&row.turn],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if !tools.is_empty() {
                kind = if tools.iter().any(|t| t == "question") {
                    "PermissionRequest"
                } else {
                    "tool"
                };
            }
        }
        // ZCode emits exact live tool/model transitions through its observer;
        // the statistics projection supplies durable settlement only.
        result.push((row.session.clone(), row.turn.clone(), kind.into(), row.at));
    }
    result.sort_by_key(|row| std::cmp::Reverse(row.3));
    Ok((state, result))
}
fn trim(state: &mut State, now: u64) {
    state
        .rows
        .retain(|_, row| now.saturating_sub(row.at / 1000) < 86400);
    if state.rows.len() > 256 {
        let mut rows = state.rows.drain().collect::<Vec<_>>();
        rows.sort_by_key(|(_, row)| std::cmp::Reverse(row.at));
        state.rows.extend(rows.into_iter().take(256));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_refresh_reads_only_visible_and_new_rows_at_both_history_sizes() {
        for count in [10, 3000] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("opencode.db");
            let mut db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)").unwrap();
            let now = crate::models::now() as u64;
            let data = json!({"role":"assistant","finish":"stop","time":{"completed":now*1000},"content":"PRIVATE"}).to_string();
            let tx = db.transaction().unwrap();
            for index in 0..count {
                tx.execute(
                    "INSERT INTO message VALUES(?1,'s',?2,?2,?3)",
                    rusqlite::params![index.to_string(), now * 1000 + index, &data],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            let (state, rows) = refresh("opencode", &path, State::default(), now).unwrap();
            assert_eq!(state.scanned, count as usize);
            assert_eq!(rows.len(), 1);
            db.execute(
                "INSERT INTO message VALUES('new','s',?1,?1,?2)",
                rusqlite::params![
                    now * 1000 + count + 1,
                    json!({"role":"assistant"}).to_string()
                ],
            )
            .unwrap();
            let (state, rows) = refresh("opencode", &path, state, now).unwrap();
            assert_eq!(state.scanned, 2);
            assert_eq!((&*rows[0].1, &*rows[0].2), ("new", "thinking"));
            db.execute("UPDATE message SET data=?1 WHERE id='new'", [&data])
                .unwrap();
            let (state, rows) = refresh("opencode", &path, state, now).unwrap();
            assert_eq!(state.scanned, 1);
            assert_eq!(rows[0].2, "complete");
        }
    }
}
