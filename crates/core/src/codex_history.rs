//! Recover a fork's counter from exactly the parent prefix referenced by Codex.
use crate::{models::*, usage};
use anyhow::{Context, Result, ensure};
use rusqlite::OpenFlags;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InheritedHistory {
    pub parent_thread: String,
    pub end_byte_offset: u64,
    pub tokens: Option<Tokens>,
    pub timestamp: Option<i64>,
    pub proof: String,
    pub issue: String,
}

#[derive(Debug, Clone)]
pub struct CounterCorrection {
    pub event: UsageEvent,
    pub history: InheritedHistory,
}

pub struct Resolver {
    paths: HashMap<String, PathBuf>,
    cache: HashMap<(String, u64), InheritedHistory>,
}

impl Resolver {
    pub fn new(source: &Source, paths: &[PathBuf]) -> Self {
        let mut resolver = Self {
            paths: HashMap::new(),
            cache: HashMap::new(),
        };
        for path in paths {
            resolver.index(path);
        }
        let root = crate::store::expand(&crate::codex_auth::home_path(&source.path));
        let root = if root.is_file() {
            root.parent().unwrap_or(&root)
        } else {
            &root
        };
        // The native index also finds archived parents outside the selected sessions folder.
        if let Ok(db) = rusqlite::Connection::open_with_flags(
            root.join("state_5.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) && let Ok(mut query) = db.prepare("SELECT id,rollout_path FROM threads")
            && let Ok(rows) =
                query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (id, path) in rows.flatten() {
                let path = PathBuf::from(path);
                if path.is_file() {
                    resolver.paths.insert(id, path);
                }
            }
        }
        for folder in ["sessions", "archived_sessions"] {
            let dir = root.join(folder);
            if dir.is_dir() {
                for entry in walkdir::WalkDir::new(dir)
                    .follow_links(false)
                    .into_iter()
                    .flatten()
                {
                    if entry.file_type().is_file()
                        && entry.path().extension().is_some_and(|s| s == "jsonl")
                    {
                        resolver.index(entry.path());
                    }
                }
            }
        }
        resolver
    }

    fn index(&mut self, path: &Path) {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return;
        };
        if let Some(id) = stem.get(stem.len().saturating_sub(36)..)
            && id.len() == 36
            && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            self.paths.entry(id.into()).or_insert_with(|| path.into());
            return;
        }
        if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            if reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut line)
                .is_ok()
                && let Ok(value) = serde_json::from_slice::<Value>(&line)
                && value["type"] == "session_meta"
                && let Some(id) = value["payload"]["id"]
                    .as_str()
                    .or(value["payload"]["session_id"].as_str())
            {
                self.paths.entry(id.into()).or_insert_with(|| path.into());
            }
        }
    }

    pub fn metadata(&mut self, value: &Value) -> Option<InheritedHistory> {
        if value["type"] != "session_meta" {
            return None;
        }
        let base = value["payload"].get("history_base")?;
        let thread = base["thread_id"].as_str().unwrap_or("");
        let limit = base["end_byte_offset"].as_u64();
        Some(self.resolve_reference(thread, limit))
    }

    pub fn refresh(&mut self, history: &InheritedHistory) -> InheritedHistory {
        self.resolve_reference(&history.parent_thread, Some(history.end_byte_offset))
    }

    fn resolve_reference(&mut self, thread: &str, limit: Option<u64>) -> InheritedHistory {
        match limit
            .context("父会话缺少有效历史边界")
            .and_then(|limit| self.resolve(thread, limit, &mut Vec::new()))
        {
            Ok(value) => value,
            Err(error) => InheritedHistory {
                parent_thread: thread.into(),
                end_byte_offset: limit.unwrap_or(0),
                issue: error.to_string(),
                ..Default::default()
            },
        }
    }

    fn resolve(
        &mut self,
        thread: &str,
        limit: u64,
        ancestors: &mut Vec<String>,
    ) -> Result<InheritedHistory> {
        ensure!(
            !thread.is_empty() && ancestors.len() < 64 && !ancestors.iter().any(|s| s == thread),
            "父会话引用无效或形成循环"
        );
        let key = (thread.to_owned(), limit);
        if let Some(value) = self.cache.get(&key) {
            return Ok(value.clone());
        }
        let path = self
            .paths
            .get(thread)
            .context("父会话记录尚未找到")?
            .clone();
        let file = File::open(&path).context("无法读取父会话记录")?;
        ensure!(
            limit <= file.metadata()?.len(),
            "父会话历史边界超出文件长度"
        );
        let mut reader = BufReader::new(file);
        let (mut offset, mut last, mut stamp, mut parent, mut actual_id) =
            (0u64, None, None, None, None);
        let mut digest = Sha256::new();
        while offset < limit {
            let mut line = Vec::new();
            let n = reader
                .by_ref()
                .take((limit - offset).min(16 * 1024 * 1024) + 1)
                .read_until(b'\n', &mut line)?;
            ensure!(
                n > 0
                    && n <= 16 * 1024 * 1024
                    && offset + n as u64 <= limit
                    && line.last() == Some(&b'\n'),
                "父会话历史边界未落在完整记录上"
            );
            offset += n as u64;
            digest.update(&line);
            let row: Value = serde_json::from_slice(&line).context("父会话历史包含损坏记录")?;
            let payload = &row["payload"];
            if row["type"] == "session_meta" {
                actual_id = payload["id"]
                    .as_str()
                    .or(payload["session_id"].as_str())
                    .map(str::to_owned);
                parent = payload.get("history_base").cloned();
            } else if row["type"] == "event_msg"
                && payload["type"] == "token_count"
                && payload["info"]["total_token_usage"].is_object()
            {
                last = Some(
                    usage::codex_tokens(&payload["info"]["total_token_usage"])
                        .context("父会话累计用量无效")?,
                );
                stamp = timestamp(&row["timestamp"]);
            }
        }
        ensure!(
            actual_id.as_deref().is_none_or(|id| id == thread),
            "父会话标识与引用不一致"
        );
        if let Some(parent) = parent {
            ancestors.push(thread.into());
            let inherited = self.resolve(
                parent["thread_id"].as_str().context("父会话标识缺失")?,
                parent["end_byte_offset"]
                    .as_u64()
                    .context("父会话历史边界缺失")?,
                ancestors,
            );
            ancestors.pop();
            let inherited = inherited?;
            digest.update(inherited.proof.as_bytes());
            if last.is_none() {
                last = inherited.tokens;
                stamp = inherited.timestamp;
            }
        }
        let result = InheritedHistory {
            parent_thread: thread.into(),
            end_byte_offset: limit,
            tokens: Some(last.unwrap_or_default()),
            timestamp: stamp,
            proof: format!("{:x}", digest.finalize()),
            issue: String::new(),
        };
        self.cache.insert(key, result.clone());
        Ok(result)
    }
}

pub fn seed(state: &mut ParseState, history: InheritedHistory) {
    state.last_cumulative = history.tokens.clone();
    state.last_timestamp = history.timestamp.unwrap_or(0);
    state.inherited_history = Some(history);
    state.first_inherited_count = true;
}
