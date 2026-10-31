//! OpenCode v1/v2 SQLite history. Only settled request statistics are read;
//! message content and provider credentials never enter the Aieyes database.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Context, Result, ensure};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn database(path: &str) -> Option<PathBuf> {
    let root = expand(path);
    let file = if root.is_file() {
        root
    } else {
        // Kilo CLI is an OpenCode fork with the same schema in kilo.db.
        ["opencode.db", "kilo.db"]
            .iter()
            .map(|name| root.join(name))
            .find(|p| p.is_file())?
    };
    (file.is_file() && file.extension().is_some_and(|e| e == "db")).then_some(file)
}

/// Providers whose history is the OpenCode session/message SQLite schema.
pub(crate) fn family(provider: &str) -> bool {
    matches!(provider, "opencode" | "kilo")
}

pub(crate) fn table_exists(db: &Connection, table: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )?)
}

/// Migration can leave independent requests in both tables. Request identities
/// survive migration, whereas each table needs its own append/pending cursor.
pub(crate) fn schemas(db: &Connection) -> Result<Vec<(&'static str, bool)>> {
    let v1 = table_exists(db, "message")?;
    let v2 = table_exists(db, "session_message")?;
    ensure!(
        v1 || v2,
        "不支持此 OpenCode 数据库版本；可使用标准 JSONL 导出"
    );
    Ok([(v1, "message", false), (v2, "session_message", true)]
        .into_iter()
        .filter(|r| r.0)
        .map(|(_, table, v2)| (table, v2))
        .collect())
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
    parsed.attribution = format!("{}-local", source.provider);
    parsed.usage_version = usage::parser_version(&source.provider);
    parsed.billing.evidence = data["provider"].as_str().unwrap_or("").into();
    parsed.inheritance = Some(UsageInheritance {
        origin: format!("message:{id}:{}", data["completed"]),
        inherited: data["inherited"] == true || data["inherited"] == 1,
    });
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
    let db = crate::sqlite_ro::open_ro(path)?;
    let tables = schemas(&db)?;
    drop(db);
    let mut result = json!({"updatedAt":now(),"files":1,"newEvents":0,"scannedRecords":0,"malformedRecords":0,"partial":false,"format":"opencode-native","issues":[]});
    for (table, v2) in tables {
        let scanned = crate::native_db::scan(
            store,
            settings,
            source,
            path,
            full,
            |db| {
                let (role, model, provider) = if v2 {
                    ("type", "$.model.id", "$.model.providerID")
                } else {
                    ("json_extract(data,'$.role')", "$.modelID", "$.providerID")
                };
                let completed = if v2 {
                    "CASE WHEN type='compaction' AND json_extract(data,'$.status') IN ('completed','failed') THEN COALESCE(json_extract(data,'$.time.completed'),json_extract(data,'$.time.created')) ELSE json_extract(data,'$.time.completed') END"
                } else {
                    "json_extract(data,'$.time.completed')"
                };
                let inherited = inheritance_column(db, v2)?;
                Ok(crate::native_db::Projection {
                    table,
                    columns: format!(
                        "CASE WHEN json_valid(data) THEN json_object('session',session_id,'role',{role},'status',json_extract(data,'$.status'),'tokens',json_extract(data,'$.tokens'),'completed',{completed},'model',json_extract(data,'{model}'),'provider',json_extract(data,'{provider}'),'inherited',{inherited}) ELSE '{{}}' END"
                    ),
                })
            },
            record,
        )?;
        for field in [
            "newEvents",
            "scannedRecords",
            "malformedRecords",
            "pendingRecords",
        ] {
            result[field] =
                json!(result[field].as_u64().unwrap_or(0) + scanned[field].as_u64().unwrap_or(0));
        }
        result["issues"]
            .as_array_mut()
            .unwrap()
            .extend(scanned["issues"].as_array().into_iter().flatten().cloned());
    }
    result["partial"] = json!(result["malformedRecords"].as_u64().unwrap_or(0) > 0);
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(result)
}

/// v2 forks retain sequence numbers and an explicit before/through boundary,
/// even though copied message IDs change. A subagent's parent_id is not a fork.
fn inheritance_column(db: &Connection, v2: bool) -> Result<&'static str> {
    if !v2 || !table_exists(db, "session_v2")? {
        return Ok("0");
    }
    let columns = db
        .prepare("PRAGMA table_info(session_v2)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !["fork_session_id", "fork_boundary"]
        .iter()
        .all(|name| columns.iter().any(|c| c == name))
    {
        return Ok("0");
    }
    Ok(
        "EXISTS(SELECT 1 FROM session_v2 f JOIN session_message b ON b.session_id=f.fork_session_id WHERE f.id=session_message.session_id AND CASE WHEN json_valid(f.fork_boundary) THEN b.id=json_extract(f.fork_boundary,'$.messageID') AND ((json_extract(f.fork_boundary,'$.type')='before' AND session_message.seq<b.seq) OR (json_extract(f.fork_boundary,'$.type')='through' AND session_message.seq<=b.seq)) ELSE 0 END)",
    )
}

pub(crate) fn record(source: &Source, id: &str, data: &Value) -> Result<crate::native_db::Record> {
    use crate::native_db::Record;
    let role = data["role"].as_str().context("OpenCode 消息类型无效")?;
    if !matches!(role, "assistant" | "compaction") {
        return Ok(Record::Skip);
    }
    if !data["completed"].is_i64() || !data["tokens"].is_object() {
        return Ok(Record::Pending);
    }
    let mut data = data.clone();
    if role == "compaction" && data["model"].as_str().is_none_or(str::is_empty) {
        data["model"] = json!("opencode-unknown-compaction");
    }
    Ok(Record::Event(Box::new(event(
        source,
        id,
        data["session"].as_str().context("OpenCode 会话标识缺失")?,
        &data,
    )?)))
}

/// Shared with the live monitor so standalone and persistent polling agree.
pub fn sessions(path: &Path) -> Result<Vec<(String, String, String, u64)>> {
    let (_, rows) =
        crate::native_sessions::refresh("opencode", path, Default::default(), now() as u64)?;
    Ok(rows
        .into_iter()
        .map(|(session, turn, kind, at, _)| (session, turn, kind, at))
        .collect())
}

pub(crate) fn state_columns(v2: bool) -> String {
    let role = if v2 {
        "type"
    } else {
        "json_extract(data,'$.role')"
    };
    let tool = if v2 {
        "CASE WHEN EXISTS(SELECT 1 FROM json_each(data,'$.content') WHERE json_extract(value,'$.type')='tool' AND json_extract(value,'$.state.status') IN ('pending','streaming','running') AND json_extract(value,'$.name') IN ('question','ask_question')) THEN 'question' WHEN EXISTS(SELECT 1 FROM json_each(data,'$.content') WHERE json_extract(value,'$.type')='tool' AND json_extract(value,'$.state.status') IN ('pending','streaming','running')) THEN 'tool' END"
    } else {
        "NULL"
    };
    format!(
        "rowid,session_id,id,COALESCE(CASE WHEN json_valid(data) THEN json_extract(data,'$.time.created') END,time_created),COALESCE(CASE WHEN json_valid(data) THEN COALESCE(json_extract(data,'$.time.completed'),json_extract(data,'$.time.created')) END,time_created),CASE WHEN json_valid(data) THEN json_object('role',{role},'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'error',COALESCE(json_extract(data,'$.error.name'),json_extract(data,'$.error.type'),json_extract(data,'$.error._tag')),'outcome',json_extract(data,'$.outcome'),'tool',{tool}) ELSE '{{}}' END"
    )
}

pub(crate) fn phase(data: &Value) -> &'static str {
    if data["role"] == "idle" {
        return match data["outcome"].as_str() {
            Some("succeeded") => "complete",
            Some("failed") => "failed",
            Some("interrupted") => "interrupted",
            _ => "unknown",
        };
    }
    let error = data["error"].as_str().unwrap_or("");
    let finish = data["finish"].as_str().unwrap_or("");
    if ["MessageAbortedError", "AbortedError"].contains(&error) {
        "interrupted"
    } else if !error.is_empty() {
        "failed"
    } else if data["role"] == "user" {
        "start"
    } else if !data["completed"].is_null() && ["stop", "end_turn"].contains(&finish) {
        "complete"
    } else if !data["completed"].is_null() && finish == "length" {
        "failed"
    } else if data["tool"] == "question" {
        "PermissionRequest"
    } else if data["tool"] == "tool" {
        "tool"
    } else {
        "thinking"
    }
}
