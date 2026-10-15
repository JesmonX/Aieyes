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
    ensure!(
        source.host_id.is_none(),
        "OpenCode 原生 SQLite 采集目前仅支持本机"
    );
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法只读打开 OpenCode 数据库")?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    db.execute_batch("BEGIN")?;
    let exists = |table: &str| -> Result<bool> {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )?)
    };
    let v2 = exists("session_message")?;
    ensure!(
        v2 || exists("message")?,
        "不支持此 OpenCode 数据库版本；可使用标准 JSONL 导出"
    );
    // Prefer v2 as a whole: migrated v1 rows must never be counted twice.
    let (table, role, model, provider) = if v2 {
        (
            "session_message",
            "type='assistant'",
            "$.model.id",
            "$.model.providerID",
        )
    } else {
        (
            "message",
            "json_extract(data,'$.role')='assistant'",
            "$.modelID",
            "$.providerID",
        )
    };
    let sql = format!(
        "SELECT id,session_id,json_object('tokens',json_extract(data,'$.tokens'),'completed',json_extract(data,'$.time.completed'),'model',json_extract(data,'{model}'),'provider',json_extract(data,'{provider}')) FROM {table} WHERE id>?1 AND json_valid(data) AND {role} AND json_type(data,'$.time.completed')='integer' AND json_type(data,'$.tokens')='object' ORDER BY id LIMIT 256"
    );
    let prices = store.prices()?;
    let mut after = String::new();
    let mut inserted = 0;
    let mut malformed = 0;
    let mut scanned = 0;
    loop {
        crate::operations::check()?;
        let rows = db
            .prepare(&sql)?
            .query_map([&after], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        let tx = store.write_transaction()?;
        crate::import::current_settings(store, settings, source)?;
        for (id, session, raw) in rows {
            scanned += 1;
            after = id.clone();
            let data: Value = serde_json::from_str(&raw)?;
            match event(source, &id, &session, &data) {
                Ok(event) => {
                    if store.put_event(&event, &prices, settings)? {
                        inserted += 1;
                    }
                }
                Err(_) => malformed += 1,
            }
        }
        tx.commit()?;
    }
    let result = json!({"updatedAt":now(),"newEvents":inserted,"files":1,"scannedRecords":scanned,"partial":malformed>0,"malformedRecords":malformed,"issues":if malformed>0 { vec![json!({"code":"invalidRecord","message":"部分 OpenCode 统计格式不支持，已跳过；未保存对话正文。"})] } else { vec![] }});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    Ok(result)
}
