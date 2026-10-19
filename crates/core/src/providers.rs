//! Antigravity is the provider; agy is its CLI and a legacy wire identifier.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PROVIDERS: &[&str] = crate::contracts::PROVIDER_IDS;

pub fn known(provider: &str) -> bool {
    PROVIDERS.contains(&canonical(provider))
}
pub fn requires_path(provider: &str) -> bool {
    crate::contracts::provider(canonical(provider)).is_some_and(|p| p.requires_path)
}
/// Subscription wakeups require an implemented, verifiable CLI protocol.
pub fn supports_wakeup(provider: &str) -> bool {
    crate::contracts::provider(canonical(provider)).is_some_and(|p| p.wakeup)
}

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
    let lower = brand.to_ascii_lowercase();
    let Some(provider) = crate::contracts::PROVIDERS
        .iter()
        .find(|p| p.id == canonical(&lower) || p.name.to_lowercase() == lower)
    else {
        return name.into();
    };
    let normalized = provider.name;

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
/// Old payloads omitted importIdentity, so hashes cannot be migrated until a
/// matching record is read again. Keep that compatibility at this boundary.
pub(crate) fn usage_ids(
    provider: &str,
    source: &str,
    account: &str,
    identity: &str,
    aliases: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut ids = vec![crate::usage::event_id(provider, source, account, identity)];
    if !antigravity(provider) {
        return ids;
    }
    let key = format!("antigravity:{account}");
    let mut accounts: BTreeSet<&str> = aliases
        .iter()
        .filter(|(_, next)| **next == key)
        .filter_map(|(old, _)| old.strip_prefix("agy:"))
        .collect();
    // Before account aliases existed, both names shared the same account ID.
    accounts.insert(account);
    for account in accounts {
        let namespace = if account.is_empty() {
            format!("source:{source}")
        } else {
            account.into()
        };
        ids.push(hash(&format!("agy:{namespace}:{identity}")));
    }
    ids
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
                    map.insert(
                        "path".into(),
                        json!(
                            crate::contracts::provider("antigravity")
                                .unwrap()
                                .default_path
                        ),
                    );
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
                map.insert(
                    "path".into(),
                    json!(
                        crate::contracts::provider("antigravity")
                            .unwrap()
                            .default_path
                    ),
                );
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
    // Diagnostics are read-only on the wire and intentionally skipped on input.
    settings.migrate();
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
        let tx = self.write_transaction()?;
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
                UNION SELECT account_key FROM estimates
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
            .map(|e| (e.storage_kind(), e.id.clone()))
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
        for table in ["quotas", "quota_history", "estimates"] {
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
            .filter(|e| preserved_samples.contains(&(e.storage_kind(), e.id.clone())))
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
        tx.execute("DELETE FROM kv WHERE key='usage.ids.canonical.v1'", [])?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
        Ok(())
    }

    /// Re-key legacy hashes atomically, merging copied events and their sources.
    pub(crate) fn migrate_usage_ids(&self) -> Result<()> {
        let tx = self.write_transaction()?;
        let done: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv WHERE key='usage.ids.canonical.v1')",
            [],
            |r| r.get(0),
        )?;
        if done {
            tx.commit()?;
            return Ok(());
        }
        let settings = self.settings()?;
        let scopes = tx
            .prepare("SELECT source_id,provider,account_id FROM source_identities")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (source, provider, account) in scopes {
            if provider != "agy" {
                continue;
            }
            let key = account_key(&format!("agy:{account}"), &settings.account_aliases);
            tx.execute(
                "INSERT OR IGNORE INTO source_identities VALUES(?1,'antigravity',?2)",
                params![source, key.strip_prefix("antigravity:").unwrap_or(&account)],
            )?;
            tx.execute("DELETE FROM source_identities WHERE source_id=?1 AND provider='agy' AND account_id=?2", params![source, account])?;
        }
        let mut after: Option<String> = None;
        loop {
            let rows = tx
            .prepare("SELECT id,payload,price FROM events WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT 512")?
            .query_map([&after], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            after = rows.last().map(|row| row.0.clone());
            for (old_id, raw, price) in rows {
                let mut event: UsageEvent = serde_json::from_str(&raw)?;
                // Old imports without a stable identity cannot safely be guessed.
                let Some(identity) = &event.import_identity else {
                    continue;
                };
                event.id = crate::usage::event_id(
                    &event.provider,
                    &event.source_id,
                    &event.account_id,
                    identity,
                );
                if event.id == old_id {
                    continue;
                }
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1)",
                    [&event.id],
                    |r| r.get(0),
                )?;
                if exists {
                    let prices: Vec<ModelPrice> = price
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?
                        .into_iter()
                        .collect();
                    self.merge_usage_event(event.clone(), &event.source_id, &prices, &settings)?;
                } else {
                    // A pure rename preserves the frozen price and cost exactly.
                    tx.execute("INSERT INTO events SELECT ?1,provider,account_id,model,stamp,?2,cost,priced_tokens,price FROM events WHERE id=?3",
                    params![event.id, serde_json::to_string(&event)?, old_id])?;
                }
                if event.provider == "codebuddy" {
                    crate::native_credits::rekey(self, &old_id, &event.id)?;
                }
                tx.execute("INSERT OR IGNORE INTO event_sources SELECT ?1,source_id FROM event_sources WHERE event_id=?2", params![event.id, old_id])?;
                tx.execute("DELETE FROM event_sources WHERE event_id=?1", [&old_id])?;
                tx.execute("DELETE FROM events WHERE id=?1", [&old_id])?;
            }
        }
        tx.execute("INSERT INTO kv VALUES('usage.ids.canonical.v1','true')", [])?;
        tx.commit()?;
        Ok(())
    }
}
