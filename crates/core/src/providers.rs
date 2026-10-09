//! Antigravity is the provider; agy is its CLI and a legacy wire identifier.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn canonical(provider: &str) -> &str {
    if provider == "agy" {
        "antigravity"
    } else {
        provider
    }
}
pub fn antigravity(provider: &str) -> bool {
    canonical(provider) == "antigravity"
}
pub fn display_name(name: &str) -> String {
    let trimmed = name.trim();
    let (brand, suffix) = trimmed.split_once(" · ").unwrap_or((trimmed, ""));
    let normalized = match brand.to_ascii_lowercase().as_str() {
        "agy" | "antigravity" => "Antigravity",
        "deepseek" => "DeepSeek",
        "codex" => "Codex",
        "claude code" => "Claude Code",
        _ => return name.into(),
    };
    if suffix.is_empty() {
        normalized.into()
    } else {
        format!("{normalized} · {suffix}")
    }
}
pub fn account_key(key: &str, aliases: &BTreeMap<String, String>) -> String {
    aliases.get(key).cloned().unwrap_or_else(|| {
        key.strip_prefix("agy:")
            .map(|id| format!("antigravity:{id}"))
            .unwrap_or_else(|| key.into())
    })
}
pub fn normalize_value(value: &mut Value, aliases: &BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            if map.get("provider").and_then(Value::as_str) == Some("agy") {
                let id_field = if map.contains_key("accountId") {
                    "accountId"
                } else if map.contains_key("quotaEnabled") {
                    "id"
                } else {
                    ""
                };
                if let Some(id) = map.get(id_field).and_then(Value::as_str) {
                    let key = account_key(&format!("agy:{id}"), aliases);
                    map.insert(
                        id_field.into(),
                        json!(key.strip_prefix("antigravity:").unwrap_or(id)),
                    );
                }
                map.insert("provider".into(), json!("antigravity"));
                if let Some(path) = map
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|s| s.trim().is_empty())
                {
                    let _ = path;
                    map.insert("path".into(), json!("~/.gemini/antigravity-cli"));
                }
                if let Some(name) = map.get("name").and_then(Value::as_str) {
                    map.insert("name".into(), json!(display_name(name)));
                }
            }
            if map.get("provider").and_then(Value::as_str) == Some("antigravity")
                && map
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| path.trim().is_empty())
            {
                map.insert("path".into(), json!("~/.gemini/antigravity-cli"));
            }
            if let Some(name) = map.get("name").and_then(Value::as_str)
                && map.contains_key("provider")
                && !map.contains_key("accountKey")
            {
                map.insert("name".into(), json!(display_name(name)));
            }
            if let Some(key) = map.get("accountKey").and_then(Value::as_str) {
                map.insert("accountKey".into(), json!(account_key(key, aliases)));
            }
            for (key, child) in map.iter_mut() {
                if key != "accountAliases" {
                    normalize_value(child, aliases);
                }
            }
        }
        Value::Array(rows) => {
            for row in rows {
                normalize_value(row, aliases);
            }
        }
        _ => {}
    }
}
pub fn normalize_settings(settings: &mut Settings) {
    settings.migrate();
    let mut value = serde_json::to_value(&*settings).expect("serializable settings");
    normalize_value(&mut value, &settings.account_aliases);
    *settings = serde_json::from_value(value).expect("normalized settings");
}

impl Store {
    pub(crate) fn migrate_antigravity(&self) -> Result<()> {
        let done: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv WHERE key='antigravity.migrated.v1')",
            [],
            |r| r.get(0),
        )?;
        if done {
            return Ok(());
        }
        let tx = self.db.unchecked_transaction()?;
        let raw: Option<String> = tx
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let mut settings: Settings = raw
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        settings.migrate();
        let mut used: BTreeSet<String> = settings
            .accounts
            .iter()
            .filter(|a| a.provider == "antigravity")
            .map(|a| a.id.clone())
            .collect();
        let mut old: BTreeSet<String> = settings
            .accounts
            .iter()
            .filter(|a| a.provider == "agy")
            .map(|a| a.id.clone())
            .collect();
        let scopes = tx.prepare(
            "WITH account_keys AS (
                SELECT account_key FROM quotas UNION SELECT account_key FROM quota_history
                UNION SELECT account_key FROM quota_estimates UNION SELECT account_key FROM credit_estimates
                UNION SELECT account_key FROM capacity_samples
            ) SELECT provider,account_id FROM events UNION SELECT provider,account_id FROM source_identities
              UNION SELECT substr(account_key,1,instr(account_key,':')-1),
                           substr(account_key,instr(account_key,':')+1) FROM account_keys"
        )?.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (provider, id) in scopes {
            if provider == "antigravity" {
                used.insert(id);
            } else if provider == "agy" {
                old.insert(id);
            }
        }
        let mut aliases = settings.account_aliases.clone();
        for id in old {
            let mut next = id.clone();
            if used.contains(&next) && !id.is_empty() {
                next = format!("legacy-agy-{}", &hash(&id)[..16]);
                while used.contains(&next) {
                    next.push('_');
                }
            }
            used.insert(next.clone());
            aliases.insert(format!("agy:{id}"), format!("antigravity:{next}"));
        }
        let preserved_samples: BTreeSet<_> = self
            .active_estimates()?
            .into_iter()
            .filter(|e| {
                crate::estimates::source_config(&settings, &e.source_ids, &e.account_key)
                    .is_ok_and(|config| config == e.source_config)
            })
            .map(|e| e.id)
            .collect();
        settings.account_aliases = aliases.clone();
        normalize_settings(&mut settings);
        // Keep old hashing namespaces, including accounts no longer attached to a source.
        tx.execute("INSERT OR IGNORE INTO source_identities SELECT es.source_id,e.provider,e.account_id FROM events e JOIN event_sources es ON es.event_id=e.id WHERE e.provider='agy'", [])?;
        let events = tx
            .prepare("SELECT id,payload FROM events WHERE provider='agy'")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, raw) in events {
            let mut v: Value = serde_json::from_str(&raw)?;
            normalize_value(&mut v, &aliases);
            tx.execute(
                "UPDATE events SET provider='antigravity',account_id=?1,payload=?2 WHERE id=?3",
                params![
                    v["accountId"].as_str().unwrap_or(""),
                    serde_json::to_string(&v)?,
                    id
                ],
            )?;
        }
        for table in [
            "quotas",
            "quota_history",
            "quota_estimates",
            "credit_estimates",
        ] {
            let rows = tx
                .prepare(&format!("SELECT rowid,account_key,payload FROM {table}"))?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (rowid, key, raw) in rows {
                let mut value: Value = serde_json::from_str(&raw)?;
                normalize_value(&mut value, &aliases);
                tx.execute(
                    &format!("UPDATE {table} SET account_key=?1,payload=?2 WHERE rowid=?3"),
                    params![
                        account_key(&key, &aliases),
                        serde_json::to_string(&value)?,
                        rowid
                    ],
                )?;
            }
        }
        let keys = tx
            .prepare(
                "SELECT DISTINCT account_key FROM capacity_samples WHERE account_key LIKE 'agy:%'",
            )?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for key in keys {
            tx.execute(
                "UPDATE capacity_samples SET account_key=?1 WHERE account_key=?2",
                params![account_key(&key, &aliases), key],
            )?;
        }
        let values = tx.prepare("SELECT key,value FROM kv WHERE key='quotaOrder' OR key LIKE 'wakeup:%' OR key LIKE 'capacity:agy:%'")?.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, raw) in values {
            let mut value: Value = serde_json::from_str(&raw)?;
            normalize_value(&mut value, &aliases);
            if key == "quotaOrder"
                && let Some(rows) = value.as_array_mut()
            {
                for row in rows {
                    if let Some(s) = row.as_str() {
                        *row = json!(account_key(s, &aliases));
                    }
                }
            }
            let new_key = key
                .strip_prefix("capacity:")
                .map(|k| format!("capacity:{}", account_key(k, &aliases)))
                .unwrap_or_else(|| key.clone());
            tx.execute("DELETE FROM kv WHERE key=?1", [&key])?;
            tx.execute(
                "INSERT OR REPLACE INTO kv VALUES(?1,?2)",
                params![new_key, serde_json::to_string(&value)?],
            )?;
        }
        if raw.is_some() {
            tx.execute(
                "UPDATE kv SET value=?1 WHERE key='settings'",
                [serde_json::to_string(&settings)?],
            )?;
        }
        for mut estimate in self
            .active_estimates()?
            .into_iter()
            .filter(|e| preserved_samples.contains(&e.id))
        {
            if let Ok(config) = crate::estimates::source_config(
                &settings,
                &estimate.source_ids,
                &estimate.account_key,
            ) {
                estimate.source_config = config;
                self.save_estimate(&estimate)?;
            }
        }
        tx.execute(
            "INSERT INTO kv VALUES('antigravity.migrated.v1','true')",
            [],
        )?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
        Ok(())
    }
}
