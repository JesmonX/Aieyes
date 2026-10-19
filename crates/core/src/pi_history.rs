//! Pi and Oh My Pi share the durable entry model, not the wire API format.
//! Entries copied by /fork retain their id and timestamp. Account-wide billing
//! identity therefore uses those fields rather than the new session header.
use crate::{models::*, usage};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};

pub(crate) fn parse(source: &Source, state: &mut ParseState, row: &Value) -> Option<UsageEvent> {
    let kind = row["type"].as_str()?;
    if kind == "session" {
        state.session_id = row["id"].as_str()?.into();
        state.turn_id.clear();
        state.cumulative_segment = 0;
        return None;
    }
    if kind == "message" && row["message"]["role"] == "user" {
        state.turn_id = row["id"].as_str().unwrap_or("").into();
        return None;
    }
    let data = match kind {
        "message" if row["message"]["role"] == "assistant" => &row["message"],
        "usage" | "model_usage" | "compaction" | "branch_summary" => row,
        _ => return None,
    };
    let u = &data["usage"];
    let optional_count = |key: &str| match u.get(key) {
        None => Some(0),
        Some(value) => value.as_u64(),
    };
    let tokens = Tokens {
        input: u["input"].as_u64()?,
        output: u["output"].as_u64()?,
        cache_read: optional_count("cacheRead")?,
        cache_write: optional_count("cacheWrite")?,
        reasoning: optional_count("reasoning")?,
    };
    let total = tokens
        .input
        .checked_add(tokens.output)?
        .checked_add(tokens.cache_read)?
        .checked_add(tokens.cache_write)?;
    if total == 0 || tokens.reasoning > tokens.output {
        return None;
    }
    let stamp = timestamp(&row["timestamp"])?;
    // SSH drops non-metering entries. Count valid usage records only so an
    // unmigrated v1 session has the same identity locally and over SSH.
    state.cumulative_segment = state.cumulative_segment.checked_add(1)?;
    let id = row["id"].as_str().filter(|s| !s.is_empty());
    let identity = match id {
        Some(id) => format!("entry:{kind}:{id}:{}", row["timestamp"].as_str()?),
        // Unmigrated v1 sessions have no entry ids. Keep their accounting scoped
        // to the native session until the client migrates them to v2/v3.
        None => format!(
            "legacy:{}:{}:{stamp}",
            state.session_id, state.cumulative_segment
        ),
    };
    let model = data["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let normalized = json!({"id":identity,"sessionId":state.session_id,"timestamp":stamp,"model":model,"tokens":tokens});
    let mut event = usage::parse_normalized(source, &mut state.clone(), &normalized)?;
    event.id = usage::event_id(&source.provider, &source.id, &source.account_id, &identity);
    event.import_identity = Some(identity);
    event.turn_id = state.turn_id.clone();
    event.attribution = format!("{}-local", source.provider);
    event.billing.evidence = data["provider"].as_str().unwrap_or("").into();
    Some(event)
}

/// v1 -> v2 assigns random entry ids. Resolve the previous metering ordinal by
/// indexed identity lookup so opening a legacy session cannot bill it twice.
pub(crate) fn put(
    store: &crate::store::Store,
    event: &UsageEvent,
    state: &ParseState,
    prices: &[ModelPrice],
    settings: &Settings,
) -> Result<bool> {
    if event
        .import_identity
        .as_deref()
        .is_some_and(|id| id.starts_with("entry:"))
    {
        let legacy = format!(
            "legacy:{}:{}:{}",
            event.session_id, state.cumulative_segment, event.timestamp
        );
        let mut accounts = vec![event.account_id.clone()];
        accounts.extend(
            store
                .db
                .prepare(
                    "SELECT account_id FROM source_identities WHERE source_id=?1 AND provider=?2",
                )?
                .query_map(params![event.source_id, event.provider], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        accounts.sort();
        accounts.dedup();
        for account in accounts {
            let old_id = usage::event_id(&event.provider, &event.source_id, &account, &legacy);
            let old: Option<(String, Option<String>)> = store.db.query_row(
                "SELECT e.payload,e.price FROM events e JOIN event_sources s ON s.event_id=e.id WHERE e.id=?1 AND s.source_id=?2",
                params![old_id,event.source_id], |r|Ok((r.get(0)?,r.get(1)?)),
            ).optional()?;
            if let Some((raw, price)) = old {
                let mut prior: UsageEvent = serde_json::from_str(&raw)?;
                ensure!(
                    prior.session_id == event.session_id
                        && prior.timestamp == event.timestamp
                        && prior.model == event.model
                        && prior.tokens == event.tokens,
                    "Pi 旧请求与迁移后的计量不一致，请保留原始记录"
                );
                prior.import_identity = event.import_identity.clone();
                prior.id = usage::event_id(
                    &prior.provider,
                    &prior.source_id,
                    &prior.account_id,
                    prior.import_identity.as_deref().unwrap(),
                );
                let exists: bool = store.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1)",
                    [&prior.id],
                    |r| r.get(0),
                )?;
                if exists {
                    let frozen: Option<ModelPrice> =
                        price.as_deref().map(serde_json::from_str).transpose()?;
                    store.merge_usage_event(
                        prior.clone(),
                        &event.source_id,
                        frozen.as_slice(),
                        settings,
                    )?;
                } else {
                    store.db.execute("INSERT INTO events SELECT ?1,provider,account_id,model,stamp,?2,cost,priced_tokens,price FROM events WHERE id=?3",
                        params![prior.id,serde_json::to_string(&prior)?,old_id])?;
                }
                store.db.execute("INSERT OR IGNORE INTO event_sources SELECT ?1,source_id FROM event_sources WHERE event_id=?2",params![prior.id,old_id])?;
                store
                    .db
                    .execute("DELETE FROM event_sources WHERE event_id=?1", [&old_id])?;
                store
                    .db
                    .execute("DELETE FROM events WHERE id=?1", [&old_id])?;
            }
        }
    }
    store.put_event(event, prices, settings)
}

/// OMP's mutable 256-byte title slot is not part of the history identity.
/// Checking and hashing a fixed-size header keeps ordinary appends O(delta).
pub(crate) fn signature(reader: &mut (impl Read + Seek), len: u64) -> std::io::Result<String> {
    reader.seek(SeekFrom::Start(0))?;
    let mut slot = vec![0; len.min(256) as usize];
    reader.read_exact(&mut slot)?;
    let title = slot.len() == 256
        && slot.last() == Some(&b'\n')
        && serde_json::from_slice::<Value>(&slot).is_ok_and(|v| v["type"] == "title");
    let start = if title { 256 } else { 0 };
    reader.seek(SeekFrom::Start(start))?;
    let mut prefix = vec![0; len.saturating_sub(start).min(128) as usize];
    reader.read_exact(&mut prefix)?;
    Ok(hash(&String::from_utf8_lossy(&prefix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_identity_survives_ssh_projection_and_invalid_optional_counters_are_rejected() {
        let source = Source {
            id: "pi".into(),
            provider: "pi".into(),
            ..Default::default()
        };
        let header = json!({"type":"session","id":"legacy","version":1});
        let row = json!({"type":"message","timestamp":"2026-10-18T22:00:00.001Z","message":{"role":"assistant","model":"fixture","usage":{"input":10,"output":2}}});
        let mut local = ParseState::default();
        let mut remote = ParseState::default();
        parse(&source, &mut local, &header);
        parse(&source, &mut remote, &header);
        parse(
            &source,
            &mut local,
            &json!({"type":"model_change","modelId":"fixture"}),
        );
        parse(
            &source,
            &mut local,
            &json!({"type":"message","message":{"role":"toolResult","content":"PRIVATE"}}),
        );
        let a = parse(&source, &mut local, &row).unwrap();
        let b = parse(&source, &mut remote, &row).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.tokens.total(), 12);
        for key in ["cacheRead", "cacheWrite", "reasoning"] {
            for value in [json!(-1), json!("3"), json!(true), Value::Null] {
                let mut invalid = row.clone();
                invalid["message"]["usage"][key] = value;
                assert!(parse(&source, &mut local, &invalid).is_none());
            }
        }
        assert_eq!(local.cumulative_segment, 1);
    }
}
