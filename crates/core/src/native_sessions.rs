//! Bounded native SQLite state. Each coexisting table has an independent cursor;
//! only visible rows and newly appended rows are read after discovery.
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

#[derive(Clone, Default)]
pub(crate) struct State {
    identity: String,
    tables: HashMap<String, TableState>,
    #[cfg(test)]
    scanned: usize,
}
#[derive(Clone, Default)]
struct TableState {
    offset: i64,
    anchor: String,
    rows: HashMap<String, Row>,
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
pub(crate) type Activity = (String, String, String, u64, String);

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
    let tables = if provider == "opencode" {
        crate::opencode::schemas(&db)?
    } else {
        vec![("turn_usage", false)]
    };
    let meta = std::fs::metadata(path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created());
    if state.identity != identity {
        state = State {
            identity,
            ..Default::default()
        };
    }
    state
        .tables
        .retain(|table, _| tables.iter().any(|(name, _)| name == table));
    let mut latest: HashMap<String, (Row, bool)> = HashMap::new();
    for (table, v2) in tables {
        let columns = if provider == "opencode" {
            crate::opencode::state_columns(v2)
        } else {
            "rowid,session_id,turn_id,started_at,COALESCE(NULLIF(completed_at,0),started_at),json_object('status',status)".into()
        };
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
        let saved = state.tables.entry(table.into()).or_default();
        let anchor = db
            .query_row(&format!("{query} WHERE rowid=?1"), [saved.offset], decode)
            .optional()?;
        if saved.offset > 0 && anchor.as_ref().map(Row::anchor).as_deref() != Some(&saved.anchor) {
            *saved = TableState::default();
        }
        if !saved.rows.is_empty() {
            let ids = saved
                .rows
                .values()
                .map(|r| r.rowid.to_string())
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
            saved.rows.clear();
            for row in rows {
                saved.rows.insert(row.session.clone(), row);
            }
        }
        loop {
            crate::operations::check()?;
            let rows = db
                .prepare(&format!("{query} WHERE rowid>?1 ORDER BY rowid LIMIT 256"))?
                .query_map([saved.offset], decode)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            #[cfg(test)]
            {
                state.scanned += rows.len();
            }
            for row in rows {
                saved.offset = row.rowid;
                saved.anchor = row.anchor();
                if now.saturating_sub(row.at / 1000) >= 86400 {
                    continue;
                }
                if saved
                    .rows
                    .get(&row.session)
                    .is_none_or(|old| (row.created, &row.turn) >= (old.created, &old.turn))
                {
                    saved.rows.insert(row.session.clone(), row);
                }
            }
            trim(&mut saved.rows, now);
        }
        trim(&mut saved.rows, now);
        for row in saved.rows.values() {
            if latest.get(&row.session).is_none_or(|(old, old_v2)| {
                (row.created, &row.turn, v2) >= (old.created, &old.turn, *old_v2)
            }) {
                latest.insert(row.session.clone(), (row.clone(), v2));
            }
        }
    }
    let parts = provider == "opencode" && crate::opencode::table_exists(&db, "part")?;
    let parents = if provider == "opencode" && crate::opencode::table_exists(&db, "session")? {
        db.prepare("PRAGMA table_info(session)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|c| c == "parent_id")
    } else {
        false
    };
    let mut result = Vec::new();
    for (row, v2) in latest.into_values() {
        let mut kind = if provider == "zcode" {
            match row.data["status"].as_str() {
                Some("completed") => "complete",
                Some("error") => "failed",
                Some("cancelled") => "interrupted",
                Some("running") => "start",
                _ => "unknown",
            }
        } else {
            crate::opencode::phase(&row.data)
        };
        if parts && !v2 && !["complete", "failed", "interrupted"].contains(&kind) {
            let tools = db.prepare("SELECT json_extract(data,'$.tool') FROM part WHERE message_id=?1 AND json_valid(data) AND json_extract(data,'$.type')='tool' AND json_extract(data,'$.state.status') IN ('pending','running') LIMIT 64")?.query_map([&row.turn],|r|r.get::<_,Option<String>>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if !tools.is_empty() {
                kind = if tools.iter().any(|t| t.as_deref() == Some("question")) {
                    "PermissionRequest"
                } else {
                    "tool"
                };
            }
        }
        let parent = if parents {
            db.query_row(
                "SELECT parent_id FROM session WHERE id=?1",
                [&row.session],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or_default()
        } else {
            String::new()
        };
        result.push((row.session, row.turn, kind.into(), row.at, parent));
    }
    result.sort_by_key(|r| std::cmp::Reverse(r.3));
    result.truncate(256);
    Ok((state, result))
}
fn trim(rows: &mut HashMap<String, Row>, now: u64) {
    rows.retain(|_, r| now.saturating_sub(r.at / 1000) < 86400);
    if rows.len() > 256 {
        let mut sorted = rows.drain().collect::<Vec<_>>();
        sorted.sort_by_key(|(_, r)| std::cmp::Reverse(r.at));
        rows.extend(sorted.into_iter().take(256));
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
