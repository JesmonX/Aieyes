//! ZCode's durable request statistics. Rollouts contain full API payloads, so
//! prefer the narrow model_usage table and never read message/part/raw JSON.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn database(path: &str) -> Option<PathBuf> {
    let root = expand(path);
    if root.is_file() {
        return root
            .extension()
            .is_some_and(|s| s == "sqlite")
            .then_some(root);
    }
    [root.join("cli/db/db.sqlite"), root.join("db/db.sqlite")]
        .into_iter()
        .find(|p| p.is_file())
}
fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    db.execute_batch("PRAGMA query_only=ON; BEGIN")?;
    Ok(db)
}

pub fn event(source: &Source, row: &Value) -> Result<UsageEvent> {
    let n = |key: &str| row[key].as_u64().context("ZCode Token 字段无效");
    let input = n("input")?;
    let output = n("output")?;
    let read = n("read")?;
    let write = n("write")?;
    let base = input.checked_add(output).context("ZCode Token 溢出")?;
    let sum = base
        .checked_add(read)
        .and_then(|v| v.checked_add(write))
        .context("ZCode Token 溢出")?;
    let total = row["providerTotal"]
        .as_u64()
        .or_else(|| row["computedTotal"].as_u64())
        .context("ZCode Token 总量缺失")?;
    let input = if total == sum {
        input
    } else {
        ensure!(total == base, "ZCode Token 总量与分类不一致");
        input
            .checked_sub(read)
            .and_then(|v| v.checked_sub(write))
            .context("ZCode 缓存计数无效")?
    };
    ensure!(n("reasoning")? <= output, "ZCode 推理计数超出输出");
    let normalized = json!({"id":row["id"],"sessionId":row["session"],"timestamp":row["completed"].as_i64().context("ZCode 完成时间无效")?/1000,
        "model":row["model"],"tokens":{"input":input,"output":output,"cacheRead":read,"cacheWrite":write,"reasoning":n("reasoning")?}});
    let mut event = usage::parse_normalized(source, &mut ParseState::default(), &normalized)
        .context("ZCode 用量格式无效")?;
    event.attribution = "zcode-local".into();
    event.turn_id = row["turn"].as_str().unwrap_or("").into();
    Ok(event)
}

pub fn scan(store: &Store, settings: &Settings, source: &Source, path: &Path) -> Result<Value> {
    scan_mode(store, settings, source, path, false)
}
pub(crate) fn rescan(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
) -> Result<Value> {
    scan_mode(store, settings, source, path, true)
}
fn scan_mode(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    full: bool,
) -> Result<Value> {
    crate::native_db::scan(
        store,
        settings,
        source,
        path,
        full,
        |_| {
            Ok(crate::native_db::Projection {table:"model_usage",columns:"json_object('id',id,'session',session_id,'turn',turn_id,'status',status,'model',model_id,'completed',completed_at,'input',input_tokens,'output',output_tokens,'read',cache_read_input_tokens,'write',cache_creation_input_tokens,'reasoning',reasoning_tokens,'providerTotal',provider_total_tokens,'computedTotal',computed_total_tokens)".into()})
        },
        record,
    )
}

pub(crate) fn record(source: &Source, _: &str, row: &Value) -> Result<crate::native_db::Record> {
    use crate::native_db::Record;
    if !row["status"]
        .as_str()
        .is_some_and(|s| ["completed", "error", "cancelled"].contains(&s))
        || !row["completed"].as_i64().is_some_and(|n| n > 0)
    {
        return Ok(Record::Pending);
    }
    if row["computedTotal"].as_u64() == Some(0) {
        return Ok(Record::Skip);
    }
    Ok(Record::Event(Box::new(event(source, row)?)))
}

/// Last turn per recent session; these tables contain statistics and tool names,
/// independent of message content or credentials.
pub fn sessions(path: &Path) -> Result<Vec<(String, String, String, u64)>> {
    let db = open(path)?;
    let mut query = db.prepare("WITH recent AS (SELECT session_id,turn_id,status,started_at,completed_at,ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY started_at DESC) AS position FROM turn_usage WHERE COALESCE(NULLIF(completed_at,0),started_at)>?1) SELECT session_id,turn_id,status,COALESCE(NULLIF(completed_at,0),started_at),EXISTS(SELECT 1 FROM tool_usage tool WHERE tool.session_id=recent.session_id AND tool.turn_id=recent.turn_id AND tool.status='running'),EXISTS(SELECT 1 FROM model_usage model WHERE model.session_id=recent.session_id AND model.turn_id=recent.turn_id AND model.status='running') FROM recent WHERE position=1 ORDER BY COALESCE(NULLIF(completed_at,0),started_at) DESC LIMIT 256")?;
    let rows = query
        .query_map([(now() - 86400) * 1000], |r| {
            let status: String = r.get(2)?;
            let kind = match status.as_str() {
                "completed" => "complete",
                "error" => "failed",
                "cancelled" => "interrupted",
                "running" if r.get::<_, bool>(4)? => "tool",
                "running" if r.get::<_, bool>(5)? => "thinking",
                "running" => "start",
                _ => "unknown",
            };
            Ok((r.get(0)?, r.get(1)?, kind.into(), r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
