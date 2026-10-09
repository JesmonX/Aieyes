//! Read only the statistical protobuf fields in Antigravity CLI conversation databases.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy)]
enum Wire<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
fn varint(bytes: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*offset).context("Antigravity 元数据不完整")?;
        *offset += 1;
        ensure!(shift < 63 || byte <= 1, "Antigravity 元数据整数溢出");
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    bail!("Antigravity 元数据整数无效")
}
fn fields(bytes: &[u8]) -> Result<Vec<(u64, Wire<'_>)>> {
    ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "Antigravity 单条元数据过大"
    );
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < bytes.len() {
        let key = varint(bytes, &mut offset)?;
        ensure!(key >> 3 > 0, "Antigravity 元数据字段无效");
        let value = match key & 7 {
            0 => Wire::Number(varint(bytes, &mut offset)?),
            kind @ (1 | 2 | 5) => {
                let length = if kind == 2 {
                    usize::try_from(varint(bytes, &mut offset)?)?
                } else if kind == 1 {
                    8
                } else {
                    4
                };
                let end = offset
                    .checked_add(length)
                    .context("Antigravity 元数据长度溢出")?;
                let data = bytes.get(offset..end).context("Antigravity 元数据不完整")?;
                offset = end;
                Wire::Bytes(data)
            }
            _ => bail!("Antigravity 元数据格式暂不支持"),
        };
        result.push((key >> 3, value));
    }
    Ok(result)
}
fn data<'a>(fields: &[(u64, Wire<'a>)], key: u64) -> Option<&'a [u8]> {
    fields.iter().find_map(|(k, v)| match v {
        Wire::Bytes(b) if *k == key => Some(*b),
        _ => None,
    })
}
fn number(fields: &[(u64, Wire<'_>)], key: u64) -> u64 {
    fields
        .iter()
        .find_map(|(k, v)| match v {
            Wire::Number(n) if *k == key => Some(*n),
            _ => None,
        })
        .unwrap_or(0)
}
fn text(fields: &[(u64, Wire<'_>)], key: u64) -> Option<String> {
    data(fields, key)
        .and_then(|b| std::str::from_utf8(b).ok())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn stamp(fields: &[(u64, Wire<'_>)], key: u64) -> Result<i64> {
    Ok(data(fields, key)
        .map(self::fields)
        .transpose()?
        .map(|f| number(&f, 1))
        .unwrap_or(0)
        .try_into()?)
}

pub fn group_for_model(model: &str) -> Option<&'static str> {
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    if name.starts_with("gemini-") {
        Some("gemini")
    } else if name.starts_with("claude-")
        || name.starts_with("gpt-")
        || ["o1", "o3", "o4"]
            .iter()
            .any(|p| name == *p || name.starts_with(&format!("{p}-")))
    {
        Some("claude-gpt")
    } else {
        None
    }
}
pub fn group_kind(group: &str) -> Option<&'static str> {
    match group.to_ascii_lowercase().as_str() {
        "gemini" | "gemini models" => Some("gemini"),
        "claude-gpt" | "claude and gpt models" => Some("claude-gpt"),
        _ => None,
    }
}
pub fn window_group(window: &QuotaWindow) -> &str {
    if window.group_id.is_empty() {
        &window.group_name
    } else {
        &window.group_id
    }
}
pub fn directory(root: &Path) -> Option<PathBuf> {
    let dir = if root.file_name().is_some_and(|s| s == "conversations") {
        root.to_path_buf()
    } else {
        root.join("conversations")
    };
    dir.is_dir().then_some(dir)
}
fn generators(db: &Connection) -> Result<BTreeMap<u64, String>> {
    let mut models = BTreeMap::new();
    let mut query = db.prepare("SELECT data FROM gen_metadata")?;
    for raw in query.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
        let raw = raw?;
        let f = fields(&raw)?;
        let chat = data(&f, 1).map(fields).transpose()?.unwrap_or_default();
        let config = data(&f, 3).map(fields).transpose()?.unwrap_or_default();
        let Some(model) = text(&chat, 19).or_else(|| text(&config, 28)) else {
            continue;
        };
        for (key, value) in &f {
            if *key != 2 {
                continue;
            }
            match value {
                Wire::Number(index) => {
                    models.insert(*index, model.clone());
                }
                Wire::Bytes(indices) => {
                    let mut i = 0;
                    while i < indices.len() {
                        models.insert(varint(indices, &mut i)?, model.clone());
                    }
                }
            }
        }
    }
    Ok(models)
}
pub fn read_database(path: &Path) -> Result<Vec<Value>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法只读打开 Antigravity 会话数据库")?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    db.execute_batch("BEGIN")?; // One consistent snapshot, including committed WAL records.
    let models = generators(&db)?;
    let session: String =
        db.query_row("SELECT cascade_id FROM trajectory_meta LIMIT 1", [], |r| {
            r.get(0)
        })?;
    ensure!(!session.is_empty(), "Antigravity 会话标识缺失");
    let mut query =
        db.prepare("SELECT idx,status,metadata FROM steps WHERE length(metadata)>0 ORDER BY idx")?;
    let mut events = Vec::new();
    for row in query.query_map([], |r| {
        Ok((
            r.get::<_, u64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Vec<u8>>(2)?,
        ))
    })? {
        let (index, status, raw) = row?;
        let f = fields(&raw)?;
        let Some(stats) = data(&f, 9) else { continue };
        let stats = fields(stats)?;
        let end = stamp(&f, 8)?.max(stamp(&f, 7)?);
        if status != 3 && end == 0 {
            continue;
        }
        let tokens = Tokens {
            input: number(&stats, 2),
            output: number(&stats, 3),
            cache_read: number(&stats, 5),
            cache_write: number(&stats, 4),
            reasoning: number(&stats, 9),
        };
        if tokens.total() == 0 {
            continue;
        }
        ensure!(
            tokens.reasoning <= tokens.output,
            "Antigravity 推理 Token 口径无效"
        );
        let start = stamp(&f, 1)?;
        ensure!(start > 0 && end >= start, "Antigravity 调用时间范围缺失");
        let model = models
            .get(&index)
            .context("Antigravity 调用缺少实际模型；请更新 CLI 后重试")?;
        let id = text(&stats, 7)
            .or_else(|| text(&stats, 11))
            .unwrap_or_else(|| format!("step:{index}"));
        events.push(json!({"id":id,"sessionId":session,"timestamp":end,"intervalStart":start,"intervalEvidence":"antigravity-step-v1","model":model,"tokens":tokens}));
    }
    Ok(events)
}
pub fn scan(store: &Store, settings: &Settings, source: &Source) -> Result<Value> {
    let root = expand(&source.path);
    let dir = directory(&root).context("Antigravity 会话目录不存在")?;
    let mut paths = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_file())
                && e.path().extension().is_some_and(|s| s == "db")
        })
        .map(|e| e.path())
        .collect::<Vec<_>>();
    paths.sort();
    let prices = store.prices()?;
    let mut count = 0;
    let mut read_bytes = 0;
    let tx = store.db.unchecked_transaction()?;
    for path in &paths {
        let signature = [
            path.clone(),
            PathBuf::from(format!("{}-wal", path.display())),
        ]
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .ok()
                .map(|m| {
                    format!(
                        "{}:{}",
                        m.len(),
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|t| t.as_nanos())
                            .unwrap_or(0)
                    )
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(":");
        let key = path.to_string_lossy();
        if store
            .cursor(&source.id, &key)?
            .is_some_and(|(s, _, _)| s == signature)
        {
            continue;
        }
        for row in read_database(path)? {
            read_bytes += serde_json::to_vec(&row)?.len();
            if let Some(event) = usage::parse(source, &mut ParseState::default(), &row)
                && store.put_event(&event, &prices, settings)?
            {
                count += 1;
            }
        }
        store.set_cursor(&source.id, &key, &signature, 0, &ParseState::default())?;
    }
    let status =
        json!({"updatedAt":now(),"newEvents":count,"files":paths.len(),"readBytes":read_bytes});
    store.source_status(&source.id, &status)?;
    tx.commit()?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protobuf_rejects_truncation_and_overflow() {
        for bytes in [
            &[10, 8, 1][..],
            &[8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
            &[0],
            &[15],
        ] {
            assert!(fields(bytes).is_err());
        }
    }
    #[test]
    fn groups_are_explicit() {
        assert_eq!(group_for_model("google/gemini-3.8-flash-n"), Some("gemini"));
        assert_eq!(
            group_for_model("anthropic/claude-sonnet-4"),
            Some("claude-gpt")
        );
        assert_eq!(group_for_model("gpt-5"), Some("claude-gpt"));
        assert_eq!(group_for_model("unknown"), None);
    }
}
