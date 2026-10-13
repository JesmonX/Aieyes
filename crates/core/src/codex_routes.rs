//! Minimal, native inference-route evidence. Never persist request bodies, headers or URL queries.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

const TARGETS: &str = "'codex_http_client::client','codex_api::endpoint::responses_websocket','codex_api::sse::responses','codex_core::client'";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Route {
    pub id: String,
    pub stamp: i64,
    pub nanos: i64,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub process_id: Option<String>,
    pub host: String,
    pub path: String,
    pub category: String,
    pub status: Option<u16>,
}

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS codex_routes(source_id TEXT NOT NULL,id TEXT NOT NULL,stamp INTEGER NOT NULL,session_id TEXT,turn_id TEXT,category TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(source_id,id));
        CREATE INDEX IF NOT EXISTS codex_route_turn ON codex_routes(source_id,session_id,turn_id,stamp);
        CREATE INDEX IF NOT EXISTS codex_route_stamp ON codex_routes(source_id,stamp);")?;
    Ok(())
}

fn fields(body: &str, key: &str) -> Vec<String> {
    let needle = format!("{key}=");
    body.match_indices(&needle)
        .filter_map(|(at, _)| {
            if at > 0 && body.as_bytes()[at - 1].is_ascii_alphanumeric()
                || at > 0 && b"_.".contains(&body.as_bytes()[at - 1])
            {
                return None;
            }
            let tail = &body[at + needle.len()..];
            if tail.starts_with('"') {
                serde_json::Deserializer::from_str(tail)
                    .into_iter::<String>()
                    .next()?
                    .ok()
            } else {
                Some(
                    tail.split(|c: char| c.is_whitespace() || "}]".contains(c))
                        .next()?
                        .to_owned(),
                )
            }
        })
        .collect()
}

fn identity(body: &str, keys: &[&str], fallback: Option<&str>) -> Option<String> {
    let values: BTreeSet<_> = keys
        .iter()
        .flat_map(|key| fields(body, key))
        .chain(fallback.map(str::to_owned))
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .collect();
    (values.len() == 1).then(|| values.into_iter().next().unwrap())
}

pub(crate) fn extract(
    id: i64,
    stamp: i64,
    nanos: i64,
    target: &str,
    body: &str,
    thread: Option<&str>,
    process: Option<&str>,
) -> Vec<Route> {
    if ![
        "codex_http_client::client",
        "codex_api::endpoint::responses_websocket",
        "codex_api::sse::responses",
        "codex_core::client",
    ]
    .contains(&target)
    {
        return Vec::new();
    }
    let mut urls = fields(body, "url");
    urls.extend(fields(body, "endpoint"));
    if urls.is_empty() && target != "codex_http_client::client" {
        urls.extend(
            body.split(|c: char| c.is_whitespace() || c == '"' || c == '<' || c == '>')
                .filter(|s| {
                    ["https://", "http://", "wss://", "ws://"]
                        .iter()
                        .any(|prefix| s.starts_with(prefix))
                })
                .map(str::to_owned),
        );
    }
    let session_id = identity(body, &["thread_id", "thread.id", "conversation.id"], thread);
    let turn_id = identity(body, &["turn_id", "turn.id"], None);
    let status = fields(body, "status")
        .last()
        .and_then(|s| s.parse::<u16>().ok());
    urls.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|raw| {
            let url = reqwest::Url::parse(raw.trim_end_matches([')', ',', '.', ';', '\\'])).ok()?;
            if !["http", "https", "ws", "wss"].contains(&url.scheme()) {
                return None;
            }
            let host = url.host_str()?.to_ascii_lowercase();
            let path = url.path().trim_end_matches('/').to_owned();
            if !["/responses", "/responses/compact", "/chat/completions"]
                .iter()
                .any(|s| path.ends_with(s))
            {
                return None;
            }
            let category = if host == "chatgpt.com" && path.starts_with("/backend-api/codex/") {
                "subscription"
            } else {
                "api"
            };
            Some(Route {
                id: hash(&format!("{id}:{stamp}:{nanos}:{process:?}:{host}:{path}")),
                stamp,
                nanos,
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                process_id: process.map(str::to_owned),
                host,
                path,
                category: category.into(),
                status,
            })
        })
        .collect()
}

impl Store {
    pub(crate) fn put_codex_routes(&self, source: &str, rows: &[Route]) -> Result<()> {
        for row in rows {
            // Remote collectors use the same narrow schema. Reject arbitrary billing labels.
            anyhow::ensure!(
                matches!(row.category.as_str(), "api" | "subscription")
                    && row.host.len() <= 253
                    && row.path.len() <= 4096,
                "推理路由证据无效"
            );
            self.db.execute(
                "INSERT OR IGNORE INTO codex_routes VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    source,
                    row.id,
                    row.stamp,
                    row.session_id,
                    row.turn_id,
                    row.category,
                    serde_json::to_string(row)?
                ],
            )?;
        }
        Ok(())
    }

    /// None means no native route was observed. Some(unknown) must never fall back to a login label.
    pub(crate) fn native_billing(&self, event: &UsageEvent) -> Result<Option<String>> {
        if event.turn_id.is_empty() {
            return Ok(None);
        }
        let mut stmt = self.db.prepare("SELECT DISTINCT category FROM codex_routes WHERE source_id IN (SELECT source_id FROM event_sources WHERE event_id=?1) AND session_id=?2 AND turn_id=?3 AND stamp>=?4 AND stamp<=?5 AND COALESCE(json_extract(payload,'$.status'),200)<400")?;
        let categories = stmt
            .query_map(
                params![
                    event.id,
                    event.session_id,
                    event.turn_id,
                    event.interval_start.unwrap_or(event.timestamp),
                    event.timestamp
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if categories.is_empty() {
            return Ok(None);
        }
        let complete = matches!(
            event.billing.evidence.as_str(),
            "codex-request-v4" | "codex-response-rate-limits"
        );
        Ok(Some(if complete && categories.len() == 1 {
            categories[0].clone()
        } else {
            "unknown".into()
        }))
    }

    pub(crate) fn native_interval_complete(
        &self,
        ids: &[String],
        start: i64,
        end: i64,
    ) -> Result<bool> {
        let uncertain: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM codex_routes r WHERE source_id IN (SELECT value FROM json_each(?1)) AND stamp>?2 AND stamp<=?3 AND COALESCE(json_extract(payload,'$.status'),200)<400 AND (session_id IS NULL OR turn_id IS NULL))", params![serde_json::to_string(ids)?,start,end], |r| r.get(0))?;
        Ok(!uncertain)
    }
}

pub(crate) fn sync(store: &Store, source: &Source) -> Result<()> {
    if source.provider != "codex" || source.host_id.is_some() {
        return Ok(());
    }
    let home = crate::store::expand(&crate::codex_auth::home_path(&source.path));
    let path = home.join("logs_2.sqlite");
    if !path.is_file() {
        return Ok(());
    }
    let db = rusqlite::Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let key = format!("codex-route-cursor:{}:{}", source.id, fingerprint(&path)?);
    let cursor: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
        .optional()?;
    let maximum: i64 = db.query_row("SELECT COALESCE(MAX(id),0) FROM logs", [], |r| r.get(0))?;
    let mut after = cursor.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    if maximum < after {
        after = 0;
    }
    let mut query = db.prepare(&format!("SELECT id,ts,ts_nanos,target,feedback_log_body,thread_id,process_uuid FROM logs WHERE id>?1 AND id<=?2 AND target IN ({TARGETS}) ORDER BY id"))?;
    let mut rows = query.query(params![after, maximum])?;
    let mut batch = Vec::new();
    while let Some(row) = rows.next()? {
        crate::operations::check()?;
        let body: Option<String> = row.get(4)?;
        batch.extend(extract(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            &row.get::<_, String>(3)?,
            body.as_deref().unwrap_or(""),
            row.get::<_, Option<String>>(5)?.as_deref(),
            row.get::<_, Option<String>>(6)?.as_deref(),
        ));
        if batch.len() >= 256 {
            let tx = store.write_transaction()?;
            store.put_codex_routes(&source.id, &batch)?;
            tx.commit()?;
            batch.clear();
        }
    }
    let tx = store.write_transaction()?;
    store.put_codex_routes(&source.id, &batch)?;
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, maximum.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

fn fingerprint(path: &Path) -> Result<String> {
    let metadata = path.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(hash(&format!(
            "{}:{}:{}",
            path.display(),
            metadata.dev(),
            metadata.ino()
        )))
    }
    #[cfg(not(unix))]
    {
        Ok(hash(&format!(
            "{}:{:?}",
            path.display(),
            metadata.created().ok()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn extracts_only_inference_and_never_private_fields() {
        let body = r#"thread_id="session" turn_id="turn" url="https://secret:password@example.com/v1/responses?api_key=private" Authorization="Bearer secret" status=200"#;
        let rows = extract(
            1,
            2,
            3,
            "codex_http_client::client",
            body,
            Some("session"),
            Some("process"),
        );
        assert_eq!(rows[0].category, "api");
        assert_eq!(rows[0].turn_id.as_deref(), Some("turn"));
        let saved = serde_json::to_string(&rows).unwrap();
        for private in ["secret", "password", "private", "Authorization"] {
            assert!(!saved.contains(private));
        }
        assert!(
            extract(
                1,
                2,
                0,
                "codex_http_client::client",
                r#"url="https://api.openai.com/v1/models""#,
                None,
                None
            )
            .is_empty()
        );
        assert!(extract(1, 2, 0, "analytics", body, None, None).is_empty());
        let rows = extract(
            1,
            2,
            0,
            "codex_core::client",
            r#"thread_id="one" thread_id="two" turn_id="turn" https://chatgpt.com/backend-api/codex/responses"#,
            Some("one"),
            None,
        );
        assert_eq!(rows[0].category, "subscription");
        assert!(rows[0].session_id.is_none());
    }

    fn event(id: &str, turn: &str, evidence: &str) -> UsageEvent {
        serde_json::from_value(json!({"id":id,"sourceId":"source","accountId":"account","provider":"codex","sessionId":"session","turnId":turn,
            "timestamp":110,"intervalStart":100,"model":"fixture","tokens":{"input":100},"attribution":"event",
            "billing":{"category":"subscription","evidence":evidence}})).unwrap()
    }

    #[test]
    fn actual_route_overrides_login_labels_but_needs_a_complete_matching_delta() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let settings = Settings::default();
        let e = event("event", "turn", "codex-response-rate-limits");
        store.put_event(&e, &[], &settings).unwrap();
        let rows = extract(
            1,
            105,
            0,
            "codex_http_client::client",
            r#"url="https://api.openai.com/v1/responses" turn_id="turn" status=200"#,
            Some("session"),
            None,
        );
        store.put_codex_routes("unrelated-source", &rows).unwrap();
        assert_eq!(store.effective_billing(&e).unwrap(), "subscription");
        store.put_codex_routes("source", &rows).unwrap();
        assert_eq!(store.effective_billing(&e).unwrap(), "api");
        let different = event("different", "other-turn", "codex-response-rate-limits");
        store.put_event(&different, &[], &settings).unwrap();
        assert_eq!(store.effective_billing(&different).unwrap(), "subscription");
        let catchup = event("catchup", "turn", "openai");
        store.put_event(&catchup, &[], &settings).unwrap();
        assert_eq!(store.effective_billing(&catchup).unwrap(), "unknown");
        let subscribed = extract(
            2,
            107,
            0,
            "codex_http_client::client",
            r#"url="https://chatgpt.com/backend-api/codex/responses" turn_id="turn" status=200"#,
            Some("session"),
            None,
        );
        store.put_codex_routes("source", &subscribed).unwrap();
        assert_eq!(store.effective_billing(&e).unwrap(), "unknown");
    }

    #[test]
    fn sqlite_route_sync_is_incremental_and_orphan_success_blocks_sampling() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("db")).unwrap();
        let native = rusqlite::Connection::open(temp.path().join("logs_2.sqlite")).unwrap();
        native.execute_batch("CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,ts_nanos INTEGER,target TEXT,feedback_log_body TEXT,thread_id TEXT,process_uuid TEXT)").unwrap();
        let insert = |id, stamp, body: &str| {
            native.execute("INSERT INTO logs VALUES(?1,?2,0,'codex_http_client::client',?3,'session','process')",params![id,stamp,body]).unwrap()
        };
        insert(
            1,
            105,
            r#"url="https://secret:password@api.openai.com/v1/responses?api_key=private" turn_id="turn" status=200"#,
        );
        insert(
            2,
            106,
            r#"url="https://api.openai.com/v1/models" status=200"#,
        );
        let source = Source {
            id: "source".into(),
            provider: "codex".into(),
            path: temp
                .path()
                .join("sessions/2026/10/13")
                .to_string_lossy()
                .into(),
            ..Default::default()
        };
        sync(&store, &source).unwrap();
        sync(&store, &source).unwrap();
        let rows: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM codex_routes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let saved: String = store
            .db
            .query_row("SELECT payload FROM codex_routes", [], |r| r.get(0))
            .unwrap();
        for private in ["secret", "password", "private", "api_key"] {
            assert!(!saved.contains(private));
        }
        assert!(
            store
                .native_interval_complete(std::slice::from_ref(&source.id), 100, 110)
                .unwrap()
        );
        insert(
            3,
            107,
            r#"url="https://api.openai.com/v1/responses" status=429"#,
        );
        sync(&store, &source).unwrap();
        assert!(
            store
                .native_interval_complete(std::slice::from_ref(&source.id), 100, 110)
                .unwrap()
        );
        insert(
            4,
            108,
            r#"url="https://api.openai.com/v1/responses" status=200"#,
        );
        sync(&store, &source).unwrap();
        assert!(
            !store
                .native_interval_complete(&[source.id], 100, 110)
                .unwrap()
        );
        let rows: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM codex_routes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 3);
    }
}
