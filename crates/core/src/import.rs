use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub fn paths(source: &Source) -> Result<Vec<PathBuf>> {
    let root = expand(&source.path);
    anyhow::ensure!(root.exists(), "数据目录不存在");
    if root.is_file() {
        return Ok(vec![root]);
    }
    let roots = match source.provider.as_str() {
        "codex" if root.join("sessions").exists() => ["sessions", "archived_sessions"]
            .iter()
            .map(|p| root.join(p))
            .filter(|p| p.exists())
            .collect(),
        "claude" if root.join("projects").exists() => vec![root.join("projects")],
        _ => vec![root],
    };
    let mut files = Vec::new();
    for dir in roots {
        for entry in walkdir::WalkDir::new(dir).follow_links(false) {
            let e = entry.context("读取数据目录失败")?;
            if e.file_type().is_file() && e.path().extension().is_some_and(|s| s == "jsonl") {
                files.push(e.path().to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn scan(store: &Store, settings: &Settings, source: &Source) -> Result<Value> {
    let prices = store.prices()?;
    let files = paths(source)?;
    let mut count = 0;
    let mut malformed = 0;
    let mut read_bytes = 0;
    for path in &files {
        let (n, b, m) = scan_file(store, settings, source, path, &prices)?;
        count += n;
        read_bytes += b;
        malformed += m;
    }
    let result = json!({"updatedAt":now(),"newEvents":count,"files":files.len(),"readBytes":read_bytes,"malformedLines":malformed});
    store.source_status(&source.id, &result)?;
    Ok(result)
}

fn scan_file(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    prices: &[ModelPrice],
) -> Result<(u64, u64, u64)> {
    let file = File::open(path).context("打开记录失败")?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    // A fixed prefix is stable as a file grows. Tiny files are rescanned on the next pass.
    let mut prefix = vec![0u8; len.min(128) as usize];
    reader.read_exact(&mut prefix)?;
    let signature = hash(&String::from_utf8_lossy(&prefix));
    let key = path.to_string_lossy();
    let cursor = store.cursor(&source.id, &key)?;
    let (offset, mut state) = match cursor {
        Some((s, o, p)) if s == signature && o <= len => (o, p),
        _ => (
            0,
            ParseState {
                session_id: format!("{}:{}", source.id, hash(&key)),
                ..Default::default()
            },
        ),
    };
    if offset == len {
        return Ok((0, 0, 0));
    }
    reader.seek(SeekFrom::Start(offset))?;
    let tx = store.db.unchecked_transaction()?;
    let mut pos = offset;
    let mut inserted = 0;
    let mut malformed = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        pos += n as u64;
        match serde_json::from_slice::<Value>(&line) {
            Ok(v) => {
                if let Some(e) = usage::parse(source, &mut state, &v)
                    && store.put_event(&e, prices, settings)?
                {
                    inserted += 1;
                }
                if source.provider == "codex"
                    && settings.quota_enabled(source)
                    && v["payload"]["type"] == "token_count"
                    && v["payload"]["rate_limits"].is_object()
                    && let Some(stamp) = timestamp(&v["timestamp"])
                {
                    store.quota(&usage::codex_quota(
                        source,
                        &v["payload"]["rate_limits"],
                        stamp,
                        "log",
                    ))?;
                }
            }
            Err(_) => malformed += 1,
        }
    }
    store.set_cursor(&source.id, &key, &signature, pos, &state)?;
    tx.commit()?;
    Ok((inserted, pos - offset, malformed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn incomplete_lines_and_reimports() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("db")).unwrap();
        let path = temp.path().join("a.jsonl");
        let source = Source {
            id: "s".into(),
            account_id: "a".into(),
            provider: "custom".into(),
            path: path.to_string_lossy().into(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let record=json!({"id":"event-1","sessionId":"session","model":"test","timestamp":now(),"tokens":{"input":100,"output":25}}).to_string();
        std::fs::write(&path, &record).unwrap();
        scan(&store, &settings, &source).unwrap();
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            0
        );
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
        )
        .unwrap();
        scan(&store, &settings, &source).unwrap();
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            125
        );
        scan(&store, &settings, &source).unwrap();
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            125
        );
    }
}
