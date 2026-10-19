//! Native SQLite requests are appended, then updated while in flight. Follow
//! rowids and revisit unfinished requests through an indexed local queue.
//! Each refresh reads new rows plus at most 256 pending rows, regardless of
//! completed history size. Explicit rescan replays settled historical edits.
use crate::{models::*, store::Store, usage};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) struct Projection {
    pub table: &'static str,
    pub columns: String,
}
pub(crate) enum Record {
    Skip,
    Pending,
    Event(Box<UsageEvent>),
}

pub(crate) fn scan(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    full: bool,
    project: impl FnOnce(&Connection) -> Result<Projection>,
    parse: impl Fn(&Source, &str, &Value) -> Result<Record>,
) -> Result<Value> {
    let key = path.to_string_lossy();
    let fingerprint = crate::sessions::database_fingerprint(path);
    let scope = hash(
        &json!([
            "native-sql-v1",
            source.provider,
            source.account_id,
            source.path
        ])
        .to_string(),
    );
    let previous = if full {
        None
    } else {
        store
            .cursor(&source.id, &key)?
            .filter(|(signature, _, _)| *signature == scope)
    };
    let version = usage::parser_version(&source.provider);
    if let Some((_, _, state)) = &previous
        && state.parser_version == version
        && let Some(saved) = state
            .database_checkpoint
            .as_ref()
            .filter(|c| c.fingerprint == fingerprint)
    {
        return finish(store, settings, source, 0, 0, saved.malformed);
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    db.execute_batch("PRAGMA query_only=ON;BEGIN")?;
    let projection = project(&db)?;
    let metadata = std::fs::metadata(path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created());
    let (mut offset, mut state) = previous
        .filter(|(_, _, s)| s.parser_version == version)
        .map(|(_, o, s)| (o, s))
        .unwrap_or_default();
    let mut checkpoint = state.database_checkpoint.clone().unwrap_or_default();
    let anchor: Option<String> = db
        .query_row(
            &format!("SELECT id FROM {} WHERE rowid=?1", projection.table),
            [offset],
            |r| r.get(0),
        )
        .optional()?;
    if full
        || checkpoint.identity != identity
        || checkpoint.table != projection.table
        || (offset > 0 && anchor.as_deref() != Some(checkpoint.anchor.as_str()))
    {
        offset = 0;
        checkpoint = DatabaseCheckpoint {
            identity,
            table: projection.table.into(),
            ..Default::default()
        };
        let tx = store.write_transaction()?;
        crate::import::current_settings(store, settings, source)?;
        store.db.execute(
            "DELETE FROM native_pending WHERE source_id=?1 AND path=?2",
            params![source.id, key],
        )?;
        tx.commit()?;
    }
    checkpoint.fingerprint.clear();
    if checkpoint.pending_fingerprint.is_empty() {
        checkpoint.pending_fingerprint = fingerprint.clone();
    }
    state.parser_version = version;
    let prices = store.prices()?;
    let query = format!(
        "SELECT rowid,id,{} FROM {}",
        projection.columns, projection.table
    );
    let pending = |after: i64| -> Result<Vec<i64>> {
        Ok(store.db.prepare("SELECT row_id FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id>?3 ORDER BY row_id LIMIT 256")?
            .query_map(params![source.id,key,after],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    };
    let mut revisit = pending(checkpoint.pending_after)?;
    if revisit.is_empty() {
        revisit = pending(0)?;
    }
    let mut inserted = 0;
    let mut scanned = 0;
    let mut apply = |rows: Vec<(i64, String, String)>,
                     gone: &[i64],
                     advance: bool,
                     offset: &mut u64,
                     state: &mut ParseState,
                     checkpoint: &mut DatabaseCheckpoint|
     -> Result<()> {
        // Native reads, JSON decoding and validation stay outside the writer lock.
        let rows = rows
            .into_iter()
            .map(|(rowid, id, raw)| {
                let record = serde_json::from_str::<Value>(&raw)
                    .map_err(anyhow::Error::from)
                    .and_then(|v| parse(source, &id, &v));
                (rowid, id, record)
            })
            .collect::<Vec<_>>();
        scanned += rows.len() as u64;
        let tx = store.write_transaction()?;
        let current = crate::import::current_settings(store, settings, source)?;
        for rowid in gone {
            let invalid:Option<bool>=store.db.query_row("DELETE FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3 RETURNING invalid",params![source.id,key,rowid],|r|r.get(0)).optional()?;
            checkpoint.malformed = checkpoint
                .malformed
                .saturating_sub(u64::from(invalid == Some(true)));
        }
        for (rowid, id, record) in rows {
            let invalid = record.is_err();
            let pending = matches!(record, Ok(Record::Pending)) || invalid;
            if let Ok(Record::Event(event)) = record
                && store.put_event(&event, &prices, &current)?
            {
                inserted += 1;
            }
            let was_invalid:Option<bool>=store.db.query_row("SELECT invalid FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3",params![source.id,key,rowid],|r|r.get(0)).optional()?;
            checkpoint.malformed = checkpoint
                .malformed
                .saturating_sub(u64::from(was_invalid == Some(true)))
                + u64::from(invalid);
            if pending {
                store.db.execute("INSERT INTO native_pending(source_id,path,row_id,invalid) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,path,row_id) DO UPDATE SET invalid=excluded.invalid",params![source.id,key,rowid,invalid])?;
            } else {
                store.db.execute(
                    "DELETE FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3",
                    params![source.id, key, rowid],
                )?;
            }
            if advance {
                *offset = rowid as u64;
                checkpoint.anchor = id;
            }
        }
        state.database_checkpoint = Some(checkpoint.clone());
        store.set_cursor(&source.id, &key, &scope, *offset, state)?;
        tx.commit()?;
        Ok(())
    };
    if !revisit.is_empty() {
        let list = revisit
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let rows = db
            .prepare(&format!("{query} WHERE rowid IN ({list}) ORDER BY rowid"))?
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let gone = revisit
            .iter()
            .copied()
            .filter(|id| !rows.iter().any(|row| row.0 == *id))
            .collect::<Vec<_>>();
        checkpoint.pending_after = *revisit.last().unwrap();
        apply(rows, &gone, false, &mut offset, &mut state, &mut checkpoint)?;
    }
    loop {
        crate::operations::check()?;
        let rows = db
            .prepare(&format!("{query} WHERE rowid>?1 ORDER BY rowid LIMIT 256"))?
            .query_map([offset], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        apply(rows, &[], true, &mut offset, &mut state, &mut checkpoint)?;
    }
    let more_pending = !revisit.is_empty() && store.db.query_row("SELECT EXISTS(SELECT 1 FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id>?3)",params![source.id,key,checkpoint.pending_after],|r|r.get::<_,bool>(0))?;
    if !more_pending
        && checkpoint.pending_fingerprint == fingerprint
        && fingerprint == crate::sessions::database_fingerprint(path)
    {
        checkpoint.fingerprint = fingerprint;
    }
    if !more_pending {
        checkpoint.pending_after = 0;
        checkpoint.pending_fingerprint.clear();
    }
    let malformed = checkpoint.malformed;
    state.database_checkpoint = Some(checkpoint);
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.set_cursor(&source.id, &key, &scope, offset, &state)?;
    tx.commit()?;
    finish(store, settings, source, inserted, scanned, malformed)
}

fn finish(
    store: &Store,
    settings: &Settings,
    source: &Source,
    inserted: u64,
    scanned: u64,
    malformed: u64,
) -> Result<Value> {
    let result = json!({"updatedAt":now(),"newEvents":inserted,"files":1,"scannedRecords":scanned,"malformedRecords":malformed,"partial":malformed>0,"format":format!("{}-native",source.provider),"issues":if malformed>0 {vec![json!({"code":"invalidRecord","message":"部分原生统计格式无效，将在记录变化后重试；已保留其他计量。"})]} else {vec![]}});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    Ok(result)
}
