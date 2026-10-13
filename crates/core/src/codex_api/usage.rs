use super::*;
use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct BillingCache {
    revision: i64,
    rows: HashMap<String, Value>,
}
pub(crate) fn retain_ledger(store: &Store, source: &Source) -> Result<()> {
    store.db.execute(
        "INSERT OR IGNORE INTO kv VALUES(?1,'-1')",
        [format!(
            "api-ledger-cursor:{}:{}",
            source.id,
            hash(&source.path)
        )],
    )?;
    Ok(())
}

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS api_requests(source_id TEXT NOT NULL,id TEXT NOT NULL,stamp INTEGER NOT NULL,session_id TEXT,turn_id TEXT,status TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(source_id,id));
        CREATE INDEX IF NOT EXISTS api_request_turn ON api_requests(source_id,session_id,turn_id);
        CREATE INDEX IF NOT EXISTS api_request_stamp ON api_requests(stamp);
        CREATE INDEX IF NOT EXISTS api_request_identity ON api_requests(
            source_id,json_extract(payload,'$.connectionId'),COALESCE(NULLIF(json_extract(payload,'$.responseId'),''),id),
            CASE WHEN status='completed' THEN 3 WHEN json_type(payload,'$.usage')='object' THEN 2 ELSE 1 END DESC,stamp,id);
        CREATE TABLE IF NOT EXISTS billing_revision(id INTEGER PRIMARY KEY,revision INTEGER NOT NULL);
        INSERT OR IGNORE INTO billing_revision VALUES(1,0);")?;
    // A retry can replay the same response. Prefer complete usage over an interrupted copy.
    db.execute_batch("CREATE VIEW IF NOT EXISTS api_request_evidence AS
        SELECT source_id,id,stamp,session_id,turn_id,status,payload FROM (
            SELECT *,ROW_NUMBER() OVER (
                PARTITION BY source_id,json_extract(payload,'$.connectionId'),COALESCE(NULLIF(json_extract(payload,'$.responseId'),''),id)
                ORDER BY CASE WHEN status='completed' THEN 3 WHEN json_type(payload,'$.usage')='object' THEN 2 ELSE 1 END DESC,stamp,id
            ) AS preferred FROM api_requests
        ) WHERE preferred=1;")?;
    for table in ["events", "event_sources", "api_requests"] {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS billing_{table}_{action} AFTER {action} ON {table} BEGIN UPDATE billing_revision SET revision=revision+1 WHERE id=1; END;"))?;
        }
    }
    Ok(())
}

pub(crate) fn sync(store: &Store, settings: &Settings, source: &Source) -> Result<()> {
    if source.provider != "codex" {
        return Ok(());
    }
    let key = format!("api-ledger-cursor:{}:{}", source.id, hash(&source.path));
    let cursor: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
        .optional()?;
    if cursor.is_none()
        && !settings.codex_apis.iter().any(|c| c.source_id == source.id)
        && !source
            .auto_valuation
            .as_ref()
            .is_some_and(|v| v.mode == "mixed")
    {
        return Ok(());
    }
    let after = cursor.and_then(|s| s.parse::<i64>().ok()).unwrap_or(-1);
    let result = target(settings, source, "ledger", &json!({"cursor":after}))?;
    let next = result["cursor"].as_i64().context("API 计量游标缺失")?;
    let rows = result["records"]
        .as_array()
        .context("API 计量记录格式无效")?;
    ensure!(next >= after, "API 计量账本已替换，请重新部署并检查原账本");
    let tx = store.write_transaction()?;
    for value in rows {
        let mut value = value.clone();
        // Several sources may share one Codex home and one gateway.
        if value["sourceId"] != source.id {
            continue;
        }
        if value["status"] == "completed" && response_tokens(&value["usage"]).is_none() {
            value["status"] = json!("incomplete");
        }
        let id = value["id"].as_str().context("API 请求标识缺失")?;
        let stamp = value["timestamp"].as_i64().context("API 请求时刻缺失")?;
        let status = value["status"].as_str().unwrap_or("incomplete");
        store.db.execute("INSERT INTO api_requests VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(source_id,id) DO UPDATE SET status=excluded.status,payload=excluded.payload,session_id=excluded.session_id,turn_id=excluded.turn_id WHERE payload!=excluded.payload", rusqlite::params![source.id,id,stamp,value["sessionId"].as_str().filter(|s| !s.is_empty()),value["turnId"].as_str().filter(|s| !s.is_empty()),status,value.to_string()])?;
    }
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, next.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

fn correlated_billing(event: &UsageEvent, states: &[String]) -> String {
    if states.is_empty() {
        return event.billing.category.clone();
    }
    // A turn ID cannot prove that a cumulative catch-up belongs entirely to that turn.
    let complete_delta = matches!(
        event.billing.evidence.as_str(),
        "codex-request-v4" | "codex-response-rate-limits"
    );
    if complete_delta
        && states.iter().all(|s| s == "completed" || s == "rejected")
        && states.iter().any(|s| s == "completed")
    {
        "api".into()
    } else {
        "unknown".into()
    }
}

impl Store {
    /// Resuming a conversation does not update its original session provider.
    pub(crate) fn effective_billing(&self, event: &UsageEvent) -> Result<String> {
        if event.provider != "codex" || event.turn_id.is_empty() {
            return Ok(event.billing.category.clone());
        }
        // Match through the turn and response indexes instead of sorting the full ledger per event.
        let mut query = self.db.prepare("SELECT DISTINCT r.status FROM api_requests r
            WHERE r.source_id IN (SELECT source_id FROM event_sources WHERE event_id=?3)
            AND r.session_id=?1 AND r.turn_id=?2 AND r.id=(
                SELECT p.id FROM api_requests p WHERE p.source_id=r.source_id
                AND json_extract(p.payload,'$.connectionId') IS json_extract(r.payload,'$.connectionId')
                AND COALESCE(NULLIF(json_extract(p.payload,'$.responseId'),''),p.id)=COALESCE(NULLIF(json_extract(r.payload,'$.responseId'),''),r.id)
                ORDER BY CASE WHEN p.status='completed' THEN 3 WHEN json_type(p.payload,'$.usage')='object' THEN 2 ELSE 1 END DESC,p.stamp,p.id LIMIT 1)")?;
        let states = query
            .query_map(
                rusqlite::params![event.session_id, event.turn_id, event.id],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(correlated_billing(event, &states))
    }

    pub(crate) fn api_interval_complete(
        &self,
        ids: &[String],
        start: i64,
        end: i64,
    ) -> Result<bool> {
        let uncertain: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM api_request_evidence WHERE source_id IN (SELECT value FROM json_each(?1)) AND stamp<=?3 AND COALESCE(json_extract(payload,'$.endedAt'),?3+1)>?2 AND (status NOT IN ('completed','rejected') OR (status='completed' AND (session_id IS NULL OR turn_id IS NULL OR COALESCE(json_type(payload,'$.usage.input_tokens'),'null')!='integer' OR COALESCE(json_type(payload,'$.usage.output_tokens'),'null')!='integer'))))", rusqlite::params![serde_json::to_string(ids)?,start,end], |r| r.get(0))?;
        Ok(!uncertain)
    }

    pub(crate) fn billing_summary(&self, filter: &Filter) -> Result<Value> {
        use chrono::{Duration as Days, Local, TimeZone};
        if filter.provider.as_deref().is_some_and(|v| v != "codex") {
            return Ok(json!({}));
        }
        let today = Local::now().date_naive();
        let day = today - Days::days(filter.days.unwrap_or(1).clamp(1, 3660) as i64 - 1);
        let start = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|v| v.timestamp())
            .unwrap_or(0);
        let end = Local
            .from_local_datetime(&(today + Days::days(1)).and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|v| v.timestamp())
            .unwrap_or(i64::MAX);
        let key = json!([
            filter.provider,
            filter.source_id,
            filter.account_id,
            filter.model,
            start,
            end
        ])
        .to_string();
        let revision: i64 = self.db.query_row(
            "SELECT revision FROM billing_revision WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        {
            let mut cache = self.billing_memory.borrow_mut();
            if cache.revision != revision {
                cache.rows.clear();
                cache.revision = revision;
            }
            if let Some(value) = cache.rows.get(&key) {
                return Ok(value.clone());
            }
        }
        // Read correlation once per revision, rather than one SQL query per event.
        let mut states: HashMap<(String, String, String), Vec<String>> = HashMap::new();
        let mut ledger = Vec::new();
        let mut query = self.db.prepare("SELECT source_id,session_id,turn_id,status,payload FROM api_request_evidence WHERE stamp<?2 AND COALESCE(json_extract(payload,'$.endedAt'),stamp+1800)>=?1 ORDER BY CASE WHEN status='completed' THEN 0 ELSE 1 END")?;
        for row in query.query_map(rusqlite::params![start, end], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (source, session, turn, status, payload) = row?;
            if let (Some(session), Some(turn)) = (session, turn) {
                states
                    .entry((source, session, turn))
                    .or_default()
                    .push(status);
            }
            ledger.push(serde_json::from_str::<Value>(&payload)?);
        }
        let mut subscription_cost = 0.0;
        let (
            mut subscription_tokens,
            mut subscription_priced,
            mut unknown,
            mut api_history,
            mut api_unmetered,
        ) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut query = self.db.prepare("SELECT e.payload,e.cost,e.priced_tokens,(SELECT json_group_array(source_id) FROM event_sources WHERE event_id=e.id) FROM events e WHERE provider='codex' AND stamp>=?1 AND stamp<?2 AND (?3 IS NULL OR model=?3) AND (?4 IS NULL OR account_id=?4) AND (?5 IS NULL OR EXISTS(SELECT 1 FROM event_sources s WHERE s.event_id=e.id AND s.source_id=?5))")?;
        for row in query.query_map(
            rusqlite::params![
                start,
                end,
                filter.model,
                filter.account_id,
                filter.source_id
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )? {
            let (raw, cost, priced, sources) = row?;
            let event: UsageEvent = serde_json::from_str(&raw)?;
            let sources: Vec<String> = serde_json::from_str(&sources)?;
            let matched: Vec<String> = sources
                .into_iter()
                .flat_map(|s| {
                    states
                        .get(&(s, event.session_id.clone(), event.turn_id.clone()))
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect();
            match correlated_billing(&event, &matched).as_str() {
                "subscription" => {
                    subscription_cost += cost;
                    subscription_tokens += event.tokens.total();
                    subscription_priced += priced;
                }
                "api" => {
                    api_history += event.tokens.total();
                    if matched.is_empty() {
                        api_unmetered += event.tokens.total();
                    }
                }
                _ => unknown += event.tokens.total(),
            }
        }
        let (mut api_cost, mut api_tokens, mut api_priced, mut incomplete) =
            (0.0, 0u64, 0u64, 0u64);
        let mut seen = HashSet::new();
        // The ledger is a cost breakdown, never a second source of history events.
        if filter.account_id.is_none() {
            for value in ledger {
                if value["timestamp"].as_i64().is_none_or(|v| v < start)
                    || filter
                        .source_id
                        .as_deref()
                        .is_some_and(|s| value["sourceId"] != s)
                    || filter.model.as_deref().is_some_and(|m| value["model"] != m)
                {
                    continue;
                }
                if value["status"] == "rejected" {
                    continue;
                }
                let identity = (
                    value["connectionId"].as_str().unwrap_or("").to_owned(),
                    value["responseId"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| value["id"].as_str())
                        .unwrap_or("")
                        .to_owned(),
                );
                if !seen.insert(identity) {
                    continue;
                }
                let tokens = response_tokens(&value["usage"]);
                if value["status"] != "completed"
                    || tokens.is_none()
                    || ["sessionId", "turnId"]
                        .iter()
                        .any(|key| value[*key].as_str().is_none_or(str::is_empty))
                {
                    incomplete += 1;
                }
                let Some(tokens) = tokens else {
                    continue;
                };
                let prices: Vec<ModelPrice> =
                    serde_json::from_value(value["prices"].clone()).unwrap_or_default();
                let price = crate::pricing::find_price(
                    value["model"].as_str().unwrap_or(""),
                    &prices,
                    &BTreeMap::new(),
                );
                let (cost, priced) = crate::pricing::estimate(&tokens, price);
                api_cost += cost;
                api_tokens += tokens.total();
                api_priced += priced;
            }
        }
        let value = json!({"apiCost":api_cost,"apiTokens":api_tokens,"apiPricedTokens":api_priced,"apiHistoryTokens":api_history,"apiUnmeteredTokens":api_unmetered,"subscriptionEquivalentCost":subscription_cost,"subscriptionTokens":subscription_tokens,"subscriptionPricedTokens":subscription_priced,"unknownTokens":unknown,"incompleteRequests":incomplete,"currency":"USD"});
        let mut cache = self.billing_memory.borrow_mut();
        if cache.rows.len() >= 16 {
            cache.rows.clear();
        }
        cache.rows.insert(key, value.clone());
        Ok(value)
    }
}

pub(crate) fn response_tokens(value: &Value) -> Option<Tokens> {
    let input = value["input_tokens"].as_u64()?;
    let output = value["output_tokens"].as_u64()?;
    let cached = match value["input_tokens_details"].get("cached_tokens") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64()?,
    };
    input.checked_add(output)?;
    Some(Tokens {
        input: input.checked_sub(cached)?,
        output,
        cache_read: cached,
        cache_write: 0,
        reasoning: value["output_tokens_details"]["reasoning_tokens"]
            .as_u64()
            .unwrap_or(0),
    })
}
