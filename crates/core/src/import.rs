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

/// Recheck after a source-lock wait and inside each committing transaction.
/// Pausing or rebinding a device must not let an old reader commit another batch.
pub(crate) fn current_settings(
    store: &Store,
    expected: &Settings,
    source: &Source,
) -> Result<Settings> {
    crate::operations::check()?;
    let settings = store.settings()?;
    let current = settings
        .sources
        .iter()
        .find(|s| s.id == source.id && settings.source_enabled(s))
        .context("来源已暂停、移除或机器未绑定，同步已停止")?;
    anyhow::ensure!(
        current.provider == source.provider
            && current.path == source.path
            && current.account_id == source.account_id
            && current.codex_home_id == source.codex_home_id
            && current.host_id == source.host_id
            && current.machine_binding == source.machine_binding,
        "来源配置已改变，同步已停止"
    );
    if let Some(id) = &source.host_id {
        let old = expected
            .hosts
            .iter()
            .find(|h| &h.id == id)
            .context("原机器配置缺失")?;
        let new = settings
            .hosts
            .iter()
            .find(|h| &h.id == id)
            .context("机器已移除")?;
        anyhow::ensure!(
            (
                &old.target,
                old.port,
                &old.username,
                &old.shell,
                &old.pre_command,
                &old.identity_file
            ) == (
                &new.target,
                new.port,
                &new.username,
                &new.shell,
                &new.pre_command,
                &new.identity_file
            ),
            "机器配置已改变，同步已停止"
        );
    }
    Ok(settings)
}

pub fn paths(source: &Source) -> Result<Vec<PathBuf>> {
    let root = expand(&source.path);
    anyhow::ensure!(root.exists(), "数据目录不存在");
    if root.is_file() {
        return Ok(vec![root]);
    }
    let roots = match source.provider.as_str() {
        "codex"
            if (source.codex_home_id.is_some()
                || root.file_name().is_some_and(|p| p == ".codex")
                || [
                    "sessions",
                    "archived_sessions",
                    "auth.json",
                    "config.toml",
                    ".aieyes",
                ]
                .iter()
                .any(|p| root.join(p).exists()))
                && !root
                    .file_name()
                    .is_some_and(|p| p == "sessions" || p == "archived_sessions") =>
        {
            ["sessions", "archived_sessions"]
                .iter()
                .map(|p| root.join(p))
                .filter(|p| p.exists())
                .collect()
        }
        "claude" if root.join("projects").exists() => vec![root.join("projects")],
        _ => vec![root],
    };
    let mut files = Vec::new();
    for dir in roots {
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".aieyes")
        {
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
    scan_mode(store, settings, source, false)
}

/// Explicitly replay available history without deleting stored usage or prices.
pub fn rescan(store: &Store, settings: &Settings, source: &Source) -> Result<Value> {
    scan_mode(store, settings, source, true)
}

fn scan_mode(store: &Store, settings: &Settings, source: &Source, full: bool) -> Result<Value> {
    let root = Path::new(store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")?;
    let _lock = crate::file_lock::history(root, &source.id)?;
    let current = current_settings(store, settings, source)?;
    let settings = &current;
    if crate::providers::antigravity(&source.provider)
        && crate::antigravity::directory(&expand(&source.path)).is_some()
    {
        return if full {
            crate::antigravity::rescan(store, settings, source)
        } else {
            crate::antigravity::scan(store, settings, source)
        };
    }
    let prices = store.prices()?;
    let files = paths(source)?;
    let mut history = if source.provider == "codex" {
        store.prepare_history_corrections()?;
        Some(crate::codex_history::Resolver::new(source, &files))
    } else {
        None
    };
    let mut count = 0;
    let mut malformed = 0;
    let mut read_bytes = 0;
    let mut missing_history = 0;
    for path in &files {
        let (n, b, m, missing) = scan_file(
            store,
            settings,
            source,
            path,
            &prices,
            full,
            history.as_mut(),
        )?;
        count += n;
        read_bytes += b;
        missing_history += missing;
        malformed += m;
    }
    let mut result = json!({"updatedAt":now(),"newEvents":count,"files":files.len(),"readBytes":read_bytes,"malformedLines":malformed});
    if full && malformed > 0 {
        result["partial"] = json!(true);
        result["issues"] = json!([{"code":"invalidRecord","message":format!("{malformed} 行记录格式无效，已保留其他可读取的历史。") }]);
    }
    if missing_history > 0 {
        result["partial"] = json!(true);
        result["missingHistoryBases"] = json!(missing_history);
        if !result["issues"].is_array() {
            result["issues"] = json!([]);
        }
        result["issues"].as_array_mut().unwrap().push(json!({"code":"missingHistoryBase","message":format!("{missing_history} 个会话缺少可验证的父历史，首段累计暂不计入；父记录恢复后自动补全。") }));
    }
    let tx = store.write_transaction()?;
    current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(result)
}

fn scan_file(
    store: &Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    prices: &[ModelPrice],
    full: bool,
    mut history: Option<&mut crate::codex_history::Resolver>,
) -> Result<(u64, u64, u64, u64)> {
    let file = File::open(path).context("打开记录失败")?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    // A fixed prefix is stable as a file grows. Tiny files are rescanned on the next pass.
    let mut prefix = vec![0u8; len.min(128) as usize];
    reader.read_exact(&mut prefix)?;
    let signature = hash(&String::from_utf8_lossy(&prefix));
    let key = path.to_string_lossy();
    let cursor = if full {
        None
    } else {
        store.cursor(&source.id, &key)?
    };
    let parent_changed = cursor
        .as_ref()
        .and_then(|(_, _, state)| state.inherited_history.as_ref())
        .is_some_and(|old| {
            history.as_deref_mut().is_some_and(|r| {
                let fresh = r.refresh(old);
                fresh.proof != old.proof || fresh.tokens != old.tokens
            })
        });
    let (offset, mut state) = match cursor {
        Some((s, o, p))
            if s == signature
                && o <= len
                && (source.provider != "codex" || p.parser_version == usage::PARSER_VERSION)
                && !parent_changed =>
        {
            (o, p)
        }
        _ => (
            0,
            ParseState {
                session_id: format!("{}:{}", source.id, hash(&key)),
                ..Default::default()
            },
        ),
    };
    if offset == len {
        return Ok((
            0,
            0,
            0,
            u64::from(
                state
                    .inherited_history
                    .as_ref()
                    .is_some_and(|h| h.tokens.is_none()),
            ),
        ));
    }
    reader.seek(SeekFrom::Start(offset))?;
    let mut pos = offset;
    let mut inserted = 0;
    let mut malformed = 0;
    let mut eof = false;
    while !eof {
        crate::operations::check()?;
        // Read a bounded batch before taking SQLite's writer lock.
        let mut rows = Vec::new();
        let mut bytes = 0u64;
        while rows.len() < 256 && bytes < 1024 * 1024 {
            let mut line = Vec::new();
            let n = (&mut reader)
                .take(16 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line)?;
            anyhow::ensure!(
                n <= 16 * 1024 * 1024,
                "记录行超过 16 MiB，已提交进度保留：{}",
                path.display()
            );
            if n == 0 || line.last() != Some(&b'\n') {
                eof = true;
                break;
            }
            bytes += n as u64;
            rows.push(line);
        }
        if rows.is_empty() {
            break;
        }
        let tx = store.write_transaction()?;
        let current = current_settings(store, settings, source)?;
        let settings = &current;
        for line in rows {
            match serde_json::from_slice::<Value>(&line) {
                Ok(v) => {
                    let baseline = history.as_deref_mut().and_then(|r| r.metadata(&v));
                    let parsed = usage::parse(source, &mut state, &v);
                    if state.first_inherited_count
                        && let Some(baseline) = baseline
                    {
                        crate::codex_history::seed(&mut state, baseline);
                    }
                    let changed = if let Some(correction) = state.counter_correction.take() {
                        store.put_counter_correction(&correction, prices, settings)?
                            && correction.event.tokens.total() > 0
                    } else if let Some(e) = parsed {
                        store.put_event(&e, prices, settings)?
                    } else {
                        false
                    };
                    if changed {
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
        pos += bytes;
        store.set_cursor(&source.id, &key, &signature, pos, &state)?;
        tx.commit()?;
    }
    Ok((
        inserted,
        pos - offset,
        malformed,
        u64::from(
            state
                .inherited_history
                .as_ref()
                .is_some_and(|h| h.tokens.is_none()),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn failed_later_batch_keeps_committed_cursor_and_full_replay_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("db")).unwrap();
        let path = root.path().join("history.jsonl");
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
        let lines: Vec<_> = (0..600).map(|i| format!("{}\n", json!({"id":format!("event-{i}"),"sessionId":"session","timestamp":now(),"model":"fixture","tokens":{"input":10}}))).collect();
        std::fs::write(&path, lines.concat()).unwrap();
        store.db.execute_batch("CREATE TRIGGER fail_later BEFORE UPDATE ON files BEGIN SELECT RAISE(ABORT,'fixture batch failure'); END;").unwrap();
        assert!(rescan(&store, &settings, &source).is_err());
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            2560
        );
        let cursor = store
            .cursor(&source.id, &path.to_string_lossy())
            .unwrap()
            .unwrap();
        assert_eq!(
            cursor.1,
            lines[..256].iter().map(|s| s.len() as u64).sum::<u64>()
        );
        store.db.execute_batch("DROP TRIGGER fail_later").unwrap();
        for _ in 0..2 {
            rescan(&store, &settings, &source).unwrap();
        }
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            6000
        );
        assert_eq!(
            store
                .cursor(&source.id, &path.to_string_lossy())
                .unwrap()
                .unwrap()
                .1,
            lines.concat().len() as u64
        );
    }
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
