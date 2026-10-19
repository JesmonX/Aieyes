//! OpenCode v1/v2 SQLite history. Only completed assistant statistics are read;
//! message content and provider credentials never enter the Aieyes database.
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
    let file = if root.is_file() {
        root
    } else {
        root.join("opencode.db")
    };
    (file.is_file() && file.extension().is_some_and(|e| e == "db")).then_some(file)
}

fn table_exists(db: &Connection, table: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )?)
}

/// 1.19 creates the v2 table before the v1 runtime migrates. An empty future
/// table must not shadow live v1 history. Once v2 has records it owns history;
/// reading both generations would count migrated requests twice.
pub(crate) fn schema(db: &Connection) -> Result<(&'static str, bool)> {
    let v1 = table_exists(db, "message")?;
    let v2 = table_exists(db, "session_message")?;
    ensure!(
        v1 || v2,
        "不支持此 OpenCode 数据库版本；可使用标准 JSONL 导出"
    );
    let active_v2 = v2
        && (!v1
            || db.query_row(
                "SELECT EXISTS(SELECT 1 FROM session_message LIMIT 1)",
                [],
                |r| r.get::<_, bool>(0),
            )?);
    Ok((
        if active_v2 {
            "session_message"
        } else {
            "message"
        },
        active_v2,
    ))
}

pub fn event(source: &Source, id: &str, session: &str, data: &Value) -> Result<UsageEvent> {
    let tokens = &data["tokens"];
    let count = |v: &Value| v.as_u64().context("OpenCode Token 字段缺失、溢出或为负值");
    let input = count(&tokens["input"])?;
    let reasoning = count(&tokens["reasoning"])?;
    // OpenCode output excludes reasoning; Aieyes output includes it.
    let output = count(&tokens["output"])?
        .checked_add(reasoning)
        .context("OpenCode Token 溢出")?;
    let read = count(&tokens["cache"]["read"])?;
    let write = count(&tokens["cache"]["write"])?;
    input
        .checked_add(output)
        .and_then(|n| n.checked_add(read))
        .and_then(|n| n.checked_add(write))
        .context("OpenCode Token 总量溢出")?;
    let stamp = data["completed"]
        .as_i64()
        .filter(|n| *n > 0)
        .context("OpenCode 完成时间无效")?
        / 1000;
    let model = data["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("OpenCode 模型缺失")?;
    let normalized = json!({"id":id,"sessionId":session,"timestamp":stamp,"model":model,
        "tokens":{"input":input,"output":output,"reasoning":reasoning,"cacheRead":read,"cacheWrite":write}});
    let mut parsed = usage::parse(source, &mut ParseState::default(), &normalized)
        .context("OpenCode 用量格式无效")?;
    parsed.attribution = "opencode-local".into();
    parsed.billing.evidence = data["provider"].as_str().unwrap_or("").into();
    // Provider selection alone does not prove API vs subscription billing.
    Ok(parsed)
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
        |db| {
            let (table, v2) = schema(db)?;
            let (role, model, provider) = if v2 {
                ("type", "$.model.id", "$.model.providerID")
            } else {
                ("json_extract(data,'$.role')", "$.modelID", "$.providerID")
            };
            Ok(crate::native_db::Projection {
                table,
                columns: format!(
                    "CASE WHEN json_valid(data) THEN json_object('session',session_id,'role',{role},'tokens',json_extract(data,'$.tokens'),'completed',json_extract(data,'$.time.completed'),'model',json_extract(data,'{model}'),'provider',json_extract(data,'{provider}')) ELSE '{{}}' END"
                ),
            })
        },
        record,
    )
}

pub(crate) fn record(source: &Source, id: &str, data: &Value) -> Result<crate::native_db::Record> {
    use crate::native_db::Record;
    let role = data["role"].as_str().context("OpenCode 消息类型无效")?;
    if role != "assistant" {
        return Ok(Record::Skip);
    }
    if !data["completed"].is_i64() || !data["tokens"].is_object() {
        return Ok(Record::Pending);
    }
    Ok(Record::Event(Box::new(event(
        source,
        id,
        data["session"].as_str().context("OpenCode 会话标识缺失")?,
        data,
    )?)))
}

/// Read metadata only. Message completion by itself can also end a tool-call
/// step, so only an explicit final finish reason means the turn completed.
pub fn sessions(path: &Path) -> Result<Vec<(String, String, String, u64)>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    db.execute_batch("PRAGMA query_only=ON; BEGIN")?;
    let (table, v2) = schema(&db)?;
    let role = if v2 {
        "type"
    } else {
        "json_extract(data,'$.role')"
    };
    let sql = format!(
        "WITH recent AS (SELECT id,session_id,time_updated,data,{role} AS role,ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY time_created DESC,id DESC) AS position FROM {table} WHERE time_updated>?1 AND json_valid(data)) SELECT session_id,id,time_updated,json_object('role',role,'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'error',json_extract(data,'$.error.name')) FROM recent WHERE position=1 ORDER BY time_updated DESC LIMIT 256"
    );
    let rows = db
        .prepare(&sql)?
        .query_map([(now() - 86400) * 1000], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_parts = !v2 && table_exists(&db, "part")?;
    let mut out = Vec::new();
    for (session, message, updated, raw) in rows {
        let data: Value = serde_json::from_str(&raw)?;
        let mut kind = match data["error"].as_str() {
            Some("MessageAbortedError" | "AbortedError") => "interrupted",
            Some(_) => "failed",
            None if data["role"] == "user" => "start",
            None if data["completed"].as_u64().is_some()
                && matches!(data["finish"].as_str(), Some("stop" | "end_turn")) =>
            {
                "complete"
            }
            None if data["completed"].as_u64().is_some() && data["finish"] == "length" => "failed",
            _ => "thinking",
        };
        if has_parts && !matches!(kind, "complete" | "failed" | "interrupted") {
            let mut q=db.prepare("SELECT json_extract(data,'$.tool') FROM part WHERE message_id=?1 AND json_valid(data) AND json_extract(data,'$.type')='tool' AND json_extract(data,'$.state.status') IN ('pending','running') LIMIT 64")?;
            for tool in q.query_map([&message], |r| r.get::<_, Option<String>>(0))? {
                if tool?.as_deref() == Some("question") {
                    kind = "PermissionRequest";
                    break;
                }
                kind = "tool";
            }
        }
        out.push((session, message, kind.into(), updated));
    }
    Ok(out)
}
