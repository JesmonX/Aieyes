//! Recover an inherited counter from the immutable rollout prefix referenced by Codex.
use crate::{models::*, usage};
use anyhow::{Result, ensure};
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
    // Kept for wire/cursor compatibility: this identifies a rollout, not necessarily a thread.
    pub parent_thread: String,
    pub end_byte_offset: u64,
    pub tokens: Option<Tokens>,
    pub timestamp: Option<i64>,
    /// Display identity inherited from the bounded parent rollout. Billing
    /// stays in the Codex namespace regardless of this value.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub originator: String,
    pub proof: String,
    pub issue: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub issue_code: String,
}

#[derive(Debug, Clone)]
pub struct CounterCorrection {
    pub event: UsageEvent,
    pub history: InheritedHistory,
}

/// Fork diagnostics belong to counting settings, including statuses saved by older builds.
pub fn sync_status(mut status: Value, precise: bool) -> Value {
    let Some(object) = status.as_object_mut() else {
        return status;
    };
    if let Some(Value::Array(issues)) = object.get_mut("issues") {
        let forks: Vec<_> = issues
            .iter()
            .filter(|i| i["code"] == "missingHistoryBase")
            .cloned()
            .collect();
        issues.retain(|i| i["code"] != "missingHistoryBase");
        let remaining = issues.iter().any(|i| i["code"] != "unknownModel");
        if !forks.is_empty() {
            if !remaining
                && !object.get("error").is_some_and(Value::is_string)
                && [
                    "failedFiles",
                    "invalidRecords",
                    "malformedLines",
                    "malformedRecords",
                ]
                .iter()
                .all(|key| object.get(*key).and_then(Value::as_u64).unwrap_or(0) == 0)
            {
                object.insert("partial".into(), Value::Bool(false));
            }
            object
                .entry("forkHistoryIssues")
                .or_insert(Value::Array(forks));
        }
    }
    if !precise {
        object.remove("forkHistoryIssues");
        object.remove("missingHistoryBases");
    }
    status
}

#[derive(Clone)]
struct Candidate {
    path: PathBuf,
    thread: String,
    canonical: bool,
}

#[derive(Debug)]
struct HistoryError(&'static str, &'static str);
impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}
impl std::error::Error for HistoryError {}
fn failure(code: &'static str, message: &'static str) -> anyhow::Error {
    HistoryError(code, message).into()
}

/// Canonical names encode the stable thread ID and, after revert, a distinct rollout ID.
fn filename_ids(path: &Path) -> Option<(String, String)> {
    fn uuid(s: &str) -> bool {
        s.len() == 36
            && s.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
    }
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zst").unwrap_or(name);
    let name = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    chrono::NaiveDateTime::parse_from_str(name.get(..19)?, "%Y-%m-%dT%H-%M-%S").ok()?;
    let ids = name.get(19..)?.strip_prefix('-')?;
    let (thread, rollout) = ids.split_once('_').unwrap_or((ids, ids));
    (uuid(thread) && uuid(rollout)).then(|| (thread.into(), rollout.into()))
}

fn representation(path: &Path) -> Option<PathBuf> {
    let mut plain = path.to_path_buf();
    if plain.extension().is_some_and(|s| s == "zst") {
        plain.set_extension("");
    }
    if plain.is_file() {
        return Some(plain);
    }
    let mut packed = plain.into_os_string();
    packed.push(".zst");
    let packed = PathBuf::from(packed);
    packed.is_file().then_some(packed)
}

fn reader(path: &Path) -> Result<(BufReader<Box<dyn Read>>, bool)> {
    let path =
        representation(path).ok_or_else(|| failure("parentNotFound", "继承历史记录尚未找到"))?;
    let compressed = path.extension().is_some_and(|e| e == "zst");
    let file = File::open(path).map_err(|_| failure("readError", "无法读取继承历史记录"))?;
    let input: Box<dyn Read> = if compressed {
        let mut decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|_| failure("invalidCompressedHistory", "继承历史压缩记录无效"))?;
        decoder
            .window_log_max(27)
            .map_err(|_| failure("invalidCompressedHistory", "继承历史压缩记录无效"))?;
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    Ok((BufReader::new(input), compressed))
}

pub struct Resolver {
    paths: HashMap<String, Vec<Candidate>>,
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
            for (_, path) in rows.flatten() {
                // SQLite selects the current thread revision, not the immutable history base.
                resolver.index(Path::new(&path));
            }
        }
        let folders = if root.join("sessions").is_dir() || root.join("archived_sessions").is_dir() {
            vec![root.join("sessions"), root.join("archived_sessions")]
        } else {
            vec![root.to_path_buf()]
        };
        for dir in folders {
            if dir.is_dir() {
                for entry in walkdir::WalkDir::new(dir)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|entry| entry.file_name() != ".aieyes")
                    .flatten()
                {
                    if entry.file_type().is_file()
                        && entry
                            .path()
                            .file_name()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.ends_with(".jsonl") || s.ends_with(".jsonl.zst"))
                    {
                        resolver.index(entry.path());
                    }
                }
            }
        }
        resolver
    }

    fn index(&mut self, path: &Path) {
        let Some(path) = representation(path) else {
            return;
        };
        let canonical = filename_ids(&path);
        let ids = canonical.clone().or_else(|| {
            let (mut reader, _) = reader(&path).ok()?;
            let mut line = Vec::new();
            reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut line)
                .ok()?;
            let value: Value = serde_json::from_slice(&line).ok()?;
            let payload = &value["payload"];
            let id = payload["id"].as_str().or(payload["session_id"].as_str())?;
            (value["type"] == "session_meta").then(|| (id.into(), id.into()))
        });
        if let Some((thread, rollout)) = ids {
            let paths = self.paths.entry(rollout).or_default();
            if !paths
                .iter()
                .any(|c| representation(&c.path).as_ref() == Some(&path))
            {
                paths.push(Candidate {
                    path,
                    thread,
                    canonical: canonical.is_some(),
                });
            }
        }
    }

    pub fn metadata(&mut self, value: &Value) -> Option<InheritedHistory> {
        if value["type"] != "session_meta" {
            return None;
        }
        let base = value["payload"]
            .get("history_base")
            .filter(|b| b.is_object())?;
        let thread = base["thread_id"].as_str().unwrap_or("");
        let limit = base["end_byte_offset"].as_u64();
        Some(self.resolve_reference(thread, limit))
    }

    pub fn refresh(&mut self, history: &InheritedHistory) -> InheritedHistory {
        self.resolve_reference(&history.parent_thread, Some(history.end_byte_offset))
    }

    fn resolve_reference(&mut self, thread: &str, limit: Option<u64>) -> InheritedHistory {
        match limit
            .ok_or_else(|| failure("boundaryInvalid", "继承历史缺少有效字节边界"))
            .and_then(|limit| self.resolve(thread, limit, &mut Vec::new()))
        {
            Ok(value) => value,
            Err(error) => InheritedHistory {
                parent_thread: thread.into(),
                end_byte_offset: limit.unwrap_or(0),
                issue: error.to_string(),
                issue_code: error
                    .downcast_ref::<HistoryError>()
                    .map_or("readError", |e| e.0)
                    .into(),
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
        if thread.is_empty() {
            return Err(failure("identityMismatch", "继承历史引用标识无效"));
        }
        if ancestors.len() >= 64 || ancestors.iter().any(|s| s == thread) {
            return Err(failure("cycle", "继承历史引用形成循环或超过层数限制"));
        }
        let key = (thread.to_owned(), limit);
        if let Some(value) = self.cache.get(&key) {
            return Ok(value.clone());
        }
        let mut candidates = self
            .paths
            .get(thread)
            .cloned()
            .ok_or_else(|| failure("parentNotFound", "继承历史记录尚未找到"))?;
        let canonical = candidates.iter().any(|c| c.canonical);
        candidates.retain(|c| !canonical || c.canonical);
        candidates.sort_by(|a, b| a.path.cmp(&b.path));
        let mut result: Option<InheritedHistory> = None;
        let mut error = None;
        ancestors.push(thread.into());
        for candidate in candidates {
            match self.read_candidate(&candidate, thread, limit, ancestors) {
                Ok(value) => {
                    if result.as_ref().is_some_and(|old| old.proof != value.proof) {
                        error = Some(failure(
                            "ambiguousHistory",
                            "继承历史存在内容不一致的同标识记录",
                        ));
                        result = None;
                        break;
                    }
                    result = Some(value);
                }
                Err(e) => {
                    error.get_or_insert(e);
                }
            }
        }
        ancestors.pop();
        let result = result.ok_or_else(|| {
            error.unwrap_or_else(|| failure("parentNotFound", "继承历史记录尚未找到"))
        })?;
        self.cache.insert(key, result.clone());
        Ok(result)
    }

    fn read_candidate(
        &mut self,
        candidate: &Candidate,
        thread: &str,
        limit: u64,
        ancestors: &mut Vec<String>,
    ) -> Result<InheritedHistory> {
        let (mut reader, compressed) = reader(&candidate.path)?;
        let (mut offset, mut last, mut stamp, mut parent) = (0u64, None, None, None);
        let mut originator = String::new();
        let mut digest = Sha256::new();
        while offset < limit {
            crate::operations::check()?;
            let mut line = Vec::new();
            let n = reader
                .by_ref()
                .take((limit - offset).min(16 * 1024 * 1024) + 1)
                .read_until(b'\n', &mut line)
                .map_err(|_| {
                    failure(
                        if compressed {
                            "invalidCompressedHistory"
                        } else {
                            "readError"
                        },
                        "无法读取完整继承历史",
                    )
                })?;
            ensure!(
                n > 0
                    && n <= 16 * 1024 * 1024
                    && offset + n as u64 <= limit
                    && line.last() == Some(&b'\n'),
                HistoryError(
                    "boundaryInvalid",
                    "继承历史字节边界超出文件或未落在完整记录上"
                )
            );
            let first = offset == 0;
            offset += n as u64;
            digest.update(&line);
            let row: Value = serde_json::from_slice(&line)
                .map_err(|_| failure("invalidRecord", "继承历史包含损坏记录"))?;
            let payload = &row["payload"];
            if first {
                ensure!(
                    row["type"] == "session_meta"
                        && payload["id"].as_str().or(payload["session_id"].as_str())
                            == Some(candidate.thread.as_str()),
                    HistoryError("identityMismatch", "继承历史文件与所属会话标识不一致")
                );
                parent = payload
                    .get("history_base")
                    .filter(|b| b.is_object())
                    .cloned();
                originator = payload["originator"].as_str().unwrap_or("").into();
            } else if row["type"] == "event_msg"
                && payload["type"] == "token_count"
                && payload["info"]["total_token_usage"].is_object()
            {
                let total = &payload["info"]["total_token_usage"];
                ensure!(
                    [
                        "output_tokens",
                        "cached_input_tokens",
                        "cache_write_input_tokens",
                        "reasoning_output_tokens"
                    ]
                    .iter()
                    .all(|key| total.get(*key).is_none_or(|v| v.as_u64().is_some())),
                    HistoryError("invalidCounter", "继承历史累计用量无效")
                );
                last = Some(
                    usage::codex_tokens(&payload["info"]["total_token_usage"])
                        .ok_or_else(|| failure("invalidCounter", "继承历史累计用量无效"))?,
                );
                stamp = timestamp(&row["timestamp"]);
            }
        }
        if compressed {
            // Validate frame checksums, including concatenated frames, without retaining text.
            let mut buffer = [0u8; 64 * 1024];
            loop {
                crate::operations::check()?;
                if reader.read(&mut buffer).map_err(|_| {
                    failure("invalidCompressedHistory", "继承历史压缩记录损坏或不完整")
                })? == 0
                {
                    break;
                }
            }
        }
        if (last.is_none() || originator.is_empty())
            && let Some(parent) = parent
        {
            let inherited = self.resolve(
                parent["thread_id"]
                    .as_str()
                    .ok_or_else(|| failure("identityMismatch", "继承历史标识缺失"))?,
                parent["end_byte_offset"]
                    .as_u64()
                    .ok_or_else(|| failure("boundaryInvalid", "继承历史字节边界缺失"))?,
                ancestors,
            );
            // Missing display metadata must not invalidate a usable counter.
            // A metadata-only fork still needs its parent's counter proof.
            let inherited = if last.is_none() {
                Some(inherited?)
            } else {
                inherited.ok()
            };
            if let Some(inherited) = inherited {
                let prefix = format!("{:x}", digest.finalize());
                digest = Sha256::new();
                digest.update(prefix.as_bytes());
                digest.update(inherited.proof.as_bytes());
                if last.is_none() {
                    last = inherited.tokens;
                    stamp = inherited.timestamp;
                }
                if originator.is_empty() {
                    originator = inherited.originator;
                }
            }
        }
        let result = InheritedHistory {
            parent_thread: thread.into(),
            end_byte_offset: limit,
            tokens: Some(last.unwrap_or_default()),
            timestamp: stamp,
            originator,
            proof: format!("{:x}", digest.finalize()),
            issue: String::new(),
            issue_code: String::new(),
        };
        Ok(result)
    }
}

pub fn seed(state: &mut ParseState, history: InheritedHistory) {
    if state.originator.is_empty() {
        state.originator.clone_from(&history.originator);
    }
    if state.precise_counting != Some(false) {
        state.last_cumulative = history.tokens.clone();
        state.last_timestamp = history.timestamp.unwrap_or(0);
    }
    state.inherited_history = Some(history);
    state.first_inherited_count = true;
}
