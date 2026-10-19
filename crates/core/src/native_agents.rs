//! Read-only statistics from native client logs. Never retain message content.
use crate::{models::*, usage};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub fn is_dsh_log(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("session.v4.jsonl" | "session.v4.jsonl.zstd" | "session.jsonl" | "session.jsonl.zstd")
    )
}

/// Zstd uses concatenated frames. Read complete lines with a bounded buffer;
/// a torn frame is an error and can be retried on the next scan.
pub fn read_dsh(path: &Path, mut consume: impl FnMut(Value) -> Result<()>) -> Result<()> {
    let file = std::fs::File::open(path)?;
    read_dsh_from(path, file, 0, 4, 0, &mut consume).map(|_| ())
}

pub(crate) fn read_dsh_from(
    path: &Path,
    file: impl Read + 'static,
    offset: u64,
    mut version: u64,
    mut legacy_seed: u64,
    mut consume: impl FnMut(Value) -> Result<()>,
) -> Result<(u64, u64, u64)> {
    let compressed = path.extension().is_some_and(|e| e == "zstd");
    let input: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zstd") {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(input);
    let mut first = offset == 0;
    let mut complete = 0;
    loop {
        crate::operations::check()?;
        let mut line = Vec::new();
        let n = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        ensure!(n <= 8 * 1024 * 1024, "Harness 单条记录过大");
        // An actively appended plaintext tail is not a completed record.
        if line.last() != Some(&b'\n') {
            ensure!(!compressed, "Harness 压缩流末尾记录尚未完成");
            break;
        }
        complete += n as u64;
        let mut row: Value = serde_json::from_slice(&line).context("Harness 记录格式无效")?;
        ensure!(row.is_object(), "Harness 记录应为对象");
        if first {
            version = row["version"].as_u64().context("Harness 会话版本无效")?;
            ensure!(
                row["type"] == "session" && matches!(version, 0 | 4),
                "不支持此 Harness 会话版本"
            );
            ensure!(
                row["id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 4096),
                "Harness 会话身份无效"
            );
            ensure!(
                crate::harness_history::generation(path) == Some(version),
                "Harness 文件名与会话版本不一致"
            );
            if version == 0 {
                legacy_seed = if row.get("seedLength").is_some() {
                    row["seedLength"].as_u64().context("Harness 继承边界无效")?
                } else {
                    0
                };
                row["isSeeded"] = json!(false);
            }
            first = false;
        } else {
            ensure!(row["type"] != "session", "Harness 流中出现重复会话头");
            if version == 0 && row["seq"].as_u64().is_some_and(|seq| seq < legacy_seed) {
                continue;
            }
        }
        row["harnessVersion"] = json!(version);
        consume(row)?;
    }
    ensure!(!first, "Harness 会话尚未写入");
    Ok((version, legacy_seed, complete))
}

pub fn scan_dsh(
    store: &crate::store::Store,
    settings: &Settings,
    source: &Source,
    full: bool,
) -> Result<Value> {
    let root = crate::store::expand(&source.path);
    ensure!(root.exists(), "数据目录不存在");
    let files = crate::harness_history::paths(&root)?;
    let prices = store.prices()?;
    let mut inserted = 0;
    let mut read_bytes = 0;
    for path in &files {
        let (events, bytes) = scan_dsh_file(store, settings, source, path, &prices, full)?;
        inserted += events;
        read_bytes += bytes;
    }
    let result = json!({"updatedAt":now(),"newEvents":inserted,"files":files.len(),"readBytes":read_bytes,"format":"harness-native"});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    Ok(result)
}

fn scan_dsh_file(
    store: &crate::store::Store,
    settings: &Settings,
    source: &Source,
    path: &Path,
    prices: &[ModelPrice],
    full: bool,
) -> Result<(u64, u64)> {
    let key = path.to_string_lossy();
    let before = crate::sessions::database_fingerprint(path);
    let prior = if full {
        None
    } else {
        store.cursor(&source.id, &key)?
    };
    if prior.as_ref().is_some_and(|(_, _, state)| {
        state.parser_version == usage::parser_version(&source.provider)
            && state
                .harness_checkpoint
                .as_ref()
                .is_some_and(|c| c.fingerprint == before)
    }) {
        return Ok((0, 0));
    }
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let size = meta.len();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created());
    let signature = region_digest(&mut file, 0, size.min(128))?;
    let (offset, mut state) = match prior {
        Some((prefix, offset, state))
            if prefix == signature
                && offset <= size
                && state.parser_version == usage::parser_version(&source.provider)
                && state.harness_checkpoint.as_ref().is_some_and(|c| {
                    c.identity == identity
                        && size > c.size
                        && region_digest(&mut file, offset.saturating_sub(128), offset.min(128))
                            .is_ok_and(|anchor| anchor == c.anchor)
                }) =>
        {
            (offset, state)
        }
        _ => (
            0,
            ParseState {
                parser_version: usage::parser_version(&source.provider),
                ..Default::default()
            },
        ),
    };
    let checkpoint = state.harness_checkpoint.clone().unwrap_or_default();
    let anchor = region_digest(&mut file, size.saturating_sub(128), size.min(128))?;
    file.seek(SeekFrom::Start(offset))?;
    // Decode and validate outside SQLite's writer lock. Only narrow metering
    // metadata is spooled; memory stays bounded even on the first large import.
    let mut spool = tempfile::spooled_tempfile(512 * 1024);
    let (version, seed, plain_bytes) = read_dsh_from(
        path,
        file.take(size - offset),
        offset,
        checkpoint.version,
        checkpoint.seed,
        |row| {
            let projection = match row["type"].as_str() {
                Some("session") => {
                    json!({"type":"session","id":row["id"],"harnessVersion":row["harnessVersion"],"isSeeded":row["isSeeded"]})
                }
                Some("session/end-seed") => json!({"type":"session/end-seed"}),
                Some("assistant/message") => {
                    json!({"type":"assistant/message","harnessVersion":row["harnessVersion"],"seq":row["seq"],"time":row["time"],"data":{"turn":row["data"]["turn"],"step":row["data"]["step"],"message":{"source":{"model":row["data"]["message"]["source"]["model"]}},"usage":{"inputTokens":row["data"]["usage"]["inputTokens"],"outputTokens":row["data"]["usage"]["outputTokens"],"cacheReadTokens":row["data"]["usage"]["cacheReadTokens"],"cacheWriteTokens":row["data"]["usage"]["cacheWriteTokens"],"reasoningTokens":row["data"]["usage"]["reasoningTokens"]}}})
                }
                _ => return Ok(()),
            };
            let encoded = serde_json::to_vec(&projection)?;
            ensure!(encoded.len() <= 65536, "Harness 统计元数据过大");
            spool.write_all(&encoded)?;
            spool.write_all(b"\n")?;
            Ok(())
        },
    )?;
    ensure!(
        before == crate::sessions::database_fingerprint(path),
        "Harness 记录读取期间发生变化，将重试"
    );
    let end = if path.extension().is_some_and(|e| e == "zstd") {
        size
    } else {
        offset + plain_bytes
    };
    state.harness_checkpoint = Some(HarnessCheckpoint {
        fingerprint: before,
        identity,
        anchor: if end == size {
            anchor
        } else {
            let mut file = std::fs::File::open(path)?;
            region_digest(&mut file, end.saturating_sub(128), end.min(128))?
        },
        size,
        version,
        seed,
    });
    spool.seek(SeekFrom::Start(0))?;
    let mut rows = BufReader::new(spool).lines();
    let mut inserted = 0;
    loop {
        let batch = rows
            .by_ref()
            .take(256)
            .map(|line| Ok(serde_json::from_str::<Value>(&line?)?))
            .collect::<Result<Vec<_>>>()?;
        if batch.is_empty() {
            break;
        }
        let tx = store.write_transaction()?;
        crate::import::current_settings(store, settings, source)?;
        for row in batch {
            crate::harness_history::prepare(store, source, &row)?;
            if let Some(event) = parse_dsh(source, &mut state, &row)
                && crate::harness_history::put(store, &event, &row, prices, settings)?
            {
                inserted += 1;
            }
        }
        tx.commit()?;
    }
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.set_cursor(&source.id, &key, &signature, end, &state)?;
    tx.commit()?;
    Ok((inserted, size - offset))
}

pub(crate) fn region_digest(file: &mut std::fs::File, start: u64, len: u64) -> Result<String> {
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; len as usize];
    file.read_exact(&mut bytes)?;
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn parse_dsh(source: &Source, state: &mut ParseState, row: &Value) -> Option<UsageEvent> {
    match row["type"].as_str()? {
        "session" => {
            state.session_id = row["id"].as_str()?.into();
            state.first_inherited_count = row["isSeeded"] == true;
            None
        }
        "session/end-seed" => {
            state.first_inherited_count = false;
            None
        }
        "assistant/message" if !state.first_inherited_count => {
            let seq = row["seq"].as_u64()?;
            let data = &row["data"];
            let route = &data["message"]["source"];
            let u = &data["usage"];
            // Harness input excludes cache, output already includes reasoning.
            let tokens = Tokens {
                input: u["inputTokens"].as_u64()?,
                output: u["outputTokens"].as_u64()?,
                cache_read: optional_counter(u, "cacheReadTokens")?,
                cache_write: optional_counter(u, "cacheWriteTokens")?,
                reasoning: optional_counter(u, "reasoningTokens")?,
            };
            state.turn_id = data["turn"].to_string();
            normalized(
                source,
                state,
                if let (Some(time), Some(turn), Some(step)) = (
                    row["time"].as_u64(),
                    data["turn"].as_u64(),
                    data["step"].as_u64(),
                ) {
                    format!("request:{time}:{turn}:{step}:{}", route["model"].as_str()?)
                } else {
                    format!("seq:{seq}")
                },
                millis(&row["time"])?,
                route["model"].as_str()?,
                tokens,
            )
        }
        _ => None,
    }
}

fn optional_counter(value: &Value, key: &str) -> Option<u64> {
    match value.get(key) {
        None | Some(Value::Null) => Some(0),
        Some(number) => number.as_u64(),
    }
}

/// Kimi agents all use `wire.jsonl`; include both session and agent identity.
pub fn session_id(path: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    if parent
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|p| p == "agents")
    {
        let session = parent.parent().and_then(Path::parent).unwrap_or(parent);
        format!(
            "{}:{}",
            session.file_name().unwrap_or_default().to_string_lossy(),
            parent.file_name().unwrap_or_default().to_string_lossy()
        )
    } else {
        parent
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

/// Forks copy the parent's wire prefix and append a `forked` boundary. Those
/// requests were already billed in the parent; start importing after the last cut.
pub fn kimi_fork_offset(path: &Path) -> Result<u64> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut offset = 0;
    let mut cut = 0;
    loop {
        crate::operations::check()?;
        let mut line = Vec::new();
        let n = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        ensure!(n <= 8 * 1024 * 1024, "Kimi 单条记录过大");
        offset += n as u64;
        if line.last() == Some(&b'\n')
            && serde_json::from_slice::<Value>(&line).is_ok_and(|v| v["type"] == "forked")
        {
            cut = offset;
        }
    }
    Ok(cut)
}

fn millis(value: &Value) -> Option<i64> {
    value.as_i64().filter(|v| *v > 0).map(|v| v / 1000)
}

fn normalized(
    source: &Source,
    state: &ParseState,
    id: String,
    stamp: i64,
    model: &str,
    tokens: Tokens,
) -> Option<UsageEvent> {
    tokens
        .input
        .checked_add(tokens.output)?
        .checked_add(tokens.cache_read)?
        .checked_add(tokens.cache_write)?;
    if tokens.total() == 0 || tokens.reasoning > tokens.output {
        return None;
    }
    let row = json!({"id":id,"sessionId":state.session_id,"timestamp":stamp,"model":model,"tokens":tokens});
    let mut event = usage::parse_normalized(source, &mut state.clone(), &row)?;
    event.attribution = format!("{}-local", source.provider);
    event.turn_id = state.turn_id.clone();
    Some(event)
}

pub fn parse(source: &Source, state: &mut ParseState, row: &Value) -> Option<UsageEvent> {
    if row["tokens"].is_object() {
        return usage::parse_normalized(source, state, row);
    }
    if source.provider == "codebuddy" {
        return codebuddy(source, state, row);
    }
    // Kimi 2.x: each durable usage.record is one request delta, not a total.
    match row["type"].as_str() {
        Some("llm.request") => {
            state.model = row["model"].as_str().unwrap_or("unknown").into();
            state.model_provider = row["provider"].as_str().unwrap_or("").into();
            None
        }
        Some("turn.prompt") => {
            state.turn_id = row["turnId"].to_string();
            None
        }
        Some("usage.record") => {
            state.cumulative_segment += 1;
            let u = &row["usage"];
            let tokens = Tokens {
                input: u["inputOther"].as_u64()?,
                output: u["output"].as_u64()?,
                cache_read: u["inputCacheRead"].as_u64()?,
                cache_write: u["inputCacheCreation"].as_u64()?,
                reasoning: 0,
            };
            let model = if state.model.is_empty() {
                row["model"].as_str()?
            } else {
                &state.model
            };
            normalized(
                source,
                state,
                format!("request:{}", state.cumulative_segment),
                millis(&row["time"])?,
                model,
                tokens,
            )
        }
        _ => {
            // Legacy Python CLI wire logs (KIMI_SHARE_DIR / ~/.kimi).
            let message = &row["message"];
            if message["type"] != "StatusUpdate" {
                return None;
            }
            let payload = &message["payload"];
            let u = &payload["token_usage"];
            let tokens = Tokens {
                input: u["input_other"].as_u64()?,
                output: u["output"].as_u64()?,
                cache_read: u["input_cache_read"].as_u64()?,
                cache_write: u["input_cache_creation"].as_u64()?,
                reasoning: 0,
            };
            let stamp = row["timestamp"]
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.)? as i64;
            normalized(
                source,
                state,
                payload["message_id"].as_str()?.into(),
                stamp,
                "unknown",
                tokens,
            )
        }
    }
}

/// Grok's durable turn completion contains per-model deltas. The session-wide
/// usage.json summary and streamed chunk counters describe the same requests.
pub fn grok_events(source: &Source, row: &Value) -> Vec<UsageEvent> {
    let p = &row["params"];
    let update = &p["update"];
    if row["method"] != "_x.ai/session/update" || update["sessionUpdate"] != "turn_completed" {
        return vec![];
    }
    let Some(models) = update["usage"]["modelUsage"].as_object() else {
        return vec![];
    };
    let Some(session) = p["sessionId"].as_str() else {
        return vec![];
    };
    let Some(id) = p["_meta"]["eventId"].as_str() else {
        return vec![];
    };
    let Some(stamp) = millis(&p["_meta"]["agentTimestampMs"]) else {
        return vec![];
    };
    let state = ParseState {
        session_id: session.into(),
        turn_id: update["prompt_id"].as_str().unwrap_or("").into(),
        ..Default::default()
    };
    models
        .iter()
        .filter_map(|(model, u)| {
            let tokens = Tokens {
                input: u["inputTokens"].as_u64()?,
                output: u["outputTokens"].as_u64()?,
                cache_read: u["cachedReadTokens"].as_u64()?,
                cache_write: u["cacheCreationTokens"].as_u64()?,
                reasoning: optional_counter(u, "reasoningTokens")?,
            };
            // Grok reports mutually exclusive input/cache categories. Refuse
            // inconsistent future counters rather than double-counting cache.
            let total = tokens
                .input
                .checked_add(tokens.output)?
                .checked_add(tokens.cache_read)?
                .checked_add(tokens.cache_write)?;
            if u["totalTokens"].as_u64().is_some_and(|n| n != total) {
                return None;
            }
            normalized(
                source,
                &state,
                format!("{id}:{model}"),
                stamp,
                model,
                tokens,
            )
        })
        .collect()
}

fn codebuddy(source: &Source, state: &mut ParseState, row: &Value) -> Option<UsageEvent> {
    if let Some(session) = row["sessionId"].as_str() {
        state.session_id = session.into();
    }
    let kind = row["type"].as_str()?;
    let p = &row["providerData"];
    state.turn_id = p["conversationRequestId"]
        .as_str()
        .or(p["requestId"].as_str())
        .unwrap_or("")
        .into();
    if state.session_id.is_empty() {
        return None;
    }
    if kind == "credit-usage" {
        let amount = p["credit"].as_f64().filter(|n| n.is_finite() && *n > 0.0)?;
        let stage = p["stageId"].as_str().filter(|id| !id.is_empty())?;
        let mirror = match p["source"].as_str()? {
            "generation_tool" => None,
            "team_member" => Some(stage.strip_prefix("team:")?.to_string()),
            _ => return None,
        };
        let id = format!("credit-stage:{}:{stage}", state.turn_id);
        let data = json!({"id":id,"sessionId":state.session_id,"timestamp":millis(&row["timestamp"])? ,"model":"unknown","tokens":{}});
        let mut event = usage::parse_normalized(source, &mut state.clone(), &data)?;
        event.turn_id = state.turn_id.clone();
        event.attribution = "codebuddy-credit-stage".into();
        event.native_credits = Some(NativeCredits {
            credits: amount,
            mirror_of: mirror,
            ..Default::default()
        });
        return Some(event);
    }
    if !matches!(kind, "function_call" | "model-usage")
        && !(kind == "message" && row["role"] == "assistant" && row["status"] == "completed")
    {
        return None;
    }
    if row["status"]
        .as_str()
        .is_some_and(|status| status != "completed")
    {
        return None;
    }
    let raw = &p["rawUsage"];
    let credits = raw["credit"].as_f64().filter(|n| n.is_finite() && *n > 0.0);
    let tokens = if raw.get("prompt_tokens").is_some() {
        let read = raw["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0);
        Tokens {
            input: raw["prompt_tokens"].as_u64()?.checked_sub(read)?,
            output: raw["completion_tokens"].as_u64()?,
            cache_read: read,
            reasoning: raw["completion_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
            ..Default::default()
        }
    } else if raw.get("input_tokens").is_some() {
        let read = raw["input_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0);
        Tokens {
            input: raw["input_tokens"].as_u64()?.checked_sub(read)?,
            output: raw["output_tokens"].as_u64()?,
            cache_read: read,
            reasoning: raw["output_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
            ..Default::default()
        }
    } else {
        Tokens::default()
    };
    tokens
        .input
        .checked_add(tokens.output)?
        .checked_add(tokens.cache_read)?;
    if (tokens.total() == 0 && credits.is_none()) || tokens.reasoning > tokens.output {
        return None;
    }
    let id = p["messageId"].as_str().or(row["id"].as_str())?;
    let data = json!({"id":id,"sessionId":state.session_id,"timestamp":millis(&row["timestamp"])? ,"model":p["model"].as_str().or(p["requestModelId"].as_str()).unwrap_or("unknown"),"tokens":tokens});
    let mut event = usage::parse_normalized(source, &mut state.clone(), &data)?;
    event.turn_id = state.turn_id.clone();
    event.attribution = "codebuddy-local".into();
    if let Some(credits) = credits {
        event.native_credits = Some(NativeCredits {
            credits,
            aliases: row["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(|id| vec![id.into()])
                .unwrap_or_default(),
            ..Default::default()
        });
    }
    Some(event)
}
