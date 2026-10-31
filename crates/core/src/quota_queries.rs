//! Cross-process coordination for scheduled, manual and valuation quota reads.
use crate::{Engine, file_lock::FileLock, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) fn root(store: &Store) -> Result<&Path> {
    Path::new(store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")
}
pub(crate) fn account_lock(root: &Path, key: &str) -> PathBuf {
    let encoded: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    root.join(format!("quota-{encoded}.lock"))
}
pub(crate) fn try_lock(path: &Path) -> Result<Option<FileLock>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    match FileLock::try_exclusive(file) {
        Ok(lock) => Ok(Some(lock)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
pub(crate) fn finished(store: &Store, key: &str) -> Result<Option<i64>> {
    Ok(store
        .db
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM kv WHERE key=?1",
            [format!("quota-attempt:{key}")],
            |r| r.get(0),
        )
        .optional()?)
}
pub(crate) fn completed(store: &Store, key: &str) -> Result<Option<String>> {
    Ok(store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("quota-result:{key}")],
            |r| r.get(0),
        )
        .optional()?)
}
pub(crate) fn fingerprint(settings: &Settings, account: &Account) -> Result<String> {
    let sources: Vec<_> = settings
        .sources
        .iter()
        .filter(|s| account.uses_source(s) && settings.source_enabled(s))
        .collect();
    let hosts: Vec<_> = settings
        .hosts
        .iter()
        .filter(|h| {
            sources.iter().any(|s| s.host_id.as_ref() == Some(&h.id))
                || settings.native_connections.iter().any(|r| {
                    r.account_key.as_deref() == Some(&crate::accounts::key(account))
                        && r.machine_id == h.id
                })
        })
        .collect();
    Ok(hash(&serde_json::to_string(&json!({
        "account": [account.provider, account.id, account.identity_key, account.quota_enabled,
            account.quota_source_id, account.quota_profile_id, account.connections, account.device_settings, account.archived],
        "sources": sources, "hosts": hosts, "proxy": settings.proxy,
        "nativeConnections": settings.native_connections.iter().filter(|r| r.account_key.as_deref()==Some(&crate::accounts::key(account))).map(|r|json!([r.id,r.provider,r.machine_id,r.machine_fingerprint,r.root,r.config_path,r.selector,r.kind,r.account_key,r.identity_key,r.status,r.source_id,r.source_path,crate::account_scan::available(settings,r)])).collect::<Vec<_>>(),
        "localEnabled": settings.local_device_enabled,
    }))?))
}
fn locations(settings: &Settings, account: &Account, root: &Path) -> Vec<PathBuf> {
    let mut keys: Vec<_> = settings
        .sources
        .iter()
        .filter(|s| settings.source_enabled(s) && account.uses_source(s))
        .map(|source| location(settings, source, root))
        .collect();
    keys.extend(
        crate::account_scan::quota_sources(settings, account)
            .iter()
            .map(|s| location(settings, s, root)),
    );
    keys.sort();
    keys.dedup();
    keys
}
fn location(settings: &Settings, source: &Source, root: &Path) -> PathBuf {
    let home = crate::codex_auth::home_path(&source.path);
    let location = if let Some(id) = &source.host_id {
        let host = settings.hosts.iter().find(|h| &h.id == id).unwrap();
        format!(
            "ssh:{}:{}:{}:{home}",
            host.target,
            host.port.unwrap_or(22),
            host.username
        )
    } else {
        let expanded = crate::store::expand(&home);
        format!(
            "local:{}",
            std::fs::canonicalize(&expanded)
                .unwrap_or(expanded)
                .display()
        )
    };
    root.join(format!("quota-location-{}.lock", hash(&location)))
}
// The legacy raw Codex endpoint retains its response shape while sharing the
// same account, login-directory and global limits as normal quota polling.
pub(crate) fn raw_codex_quota(
    store: &Store,
    settings: &Settings,
    source: &Source,
    params: &Value,
) -> Result<Value> {
    let profile = params["profileId"].as_str().context("请选择账户档案")?;
    let reference = format!(
        "{}:{profile}",
        source.codex_home_id.as_deref().context("home 标识缺失")?
    );
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == "codex" && a.has_profile(&reference));
    let key = account
        .map(crate::accounts::key)
        .unwrap_or_else(|| format!("codex-profile:{reference}"));
    let root = root(store)?;
    let start = Instant::now();
    let _account = loop {
        if let Some(lock) = try_lock(&account_lock(root, &key))? {
            break lock;
        }
        pause(start)?;
    };
    let _location = loop {
        if let Some(lock) = try_lock(&location(settings, source, root))? {
            break lock;
        }
        pause(start)?;
    };
    let _slot = loop {
        let mut acquired = None;
        for slot in 0..4 {
            if let Some(lock) = try_lock(&root.join(format!("quota-slot-{slot}.lock")))? {
                acquired = Some(lock);
                break;
            }
        }
        if let Some(lock) = acquired {
            break lock;
        }
        pause(start)?;
    };
    let signature = |settings: &Settings| -> Result<Value> {
        let source = settings
            .sources
            .iter()
            .find(|s| s.id == source.id && settings.source_enabled(s))
            .context("来源已暂停或移除")?;
        Ok(json!([
            source,
            settings
                .hosts
                .iter()
                .find(|h| Some(&h.id) == source.host_id.as_ref()),
            settings.accounts.iter().find(|a| a.has_profile(&reference)),
            settings.proxy
        ]))
    };
    let expected = signature(settings)?;
    ensure!(signature(&store.settings()?)? == expected, "查询配置已改变");
    let revision = crate::accounts::status_revision(store, source)?;
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![format!("quota-attempt:{key}"), now().to_string()],
    )?;
    let result = crate::codex_auth::backend(source, settings, "quota", params);
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![format!("quota-attempt:{key}"), now().to_string()],
    )?;
    let mut value = result?;
    ensure!(
        signature(&store.settings()?)? == expected
            && crate::accounts::status_revision(store, source)? == revision,
        "查询期间账户或机器配置已改变，结果已丢弃"
    );
    let observations = value
        .as_object_mut()
        .and_then(|object| object.remove("_aieyesIdentityObservations"))
        .unwrap_or(Value::Null);
    store.record_codex_query_observations(settings, source, &observations)?;
    Ok(value)
}
fn pause(start: Instant) -> Result<()> {
    crate::operations::check()?;
    ensure!(
        start.elapsed() < Duration::from_secs(120),
        "额度查询正在使用此账户或目录，请稍后重试"
    );
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

impl Engine {
    /// Sampling boundaries request a fresh read after their preceding log sync.
    pub(crate) fn read_quotas(&mut self, params: &Value) -> Result<Vec<QuotaSnapshot>> {
        self.coordinated_quotas(params, false, true, &Default::default())
    }
    pub(crate) fn coordinated_quotas(
        &mut self,
        params: &Value,
        due_only: bool,
        fresh: bool,
        observed: &std::collections::HashMap<String, Option<String>>,
    ) -> Result<Vec<QuotaSnapshot>> {
        let initial = self.store.settings()?;
        let accounts: Vec<_> = initial
            .accounts
            .iter()
            .filter(|a| {
                a.quota_enabled
                    && !a.archived
                    && params["accountKey"]
                        .as_str()
                        .is_none_or(|key| crate::accounts::key(a) == key)
                    && params["accountId"].as_str().is_none_or(|id| a.id == id)
            })
            .map(crate::accounts::key)
            .collect();
        let root = root(&self.store)?.to_owned();
        let mut all = vec![];
        for key in accounts {
            crate::operations::check()?;
            let before = match observed.get(&key) {
                Some(value) => value.clone(),
                None => completed(&self.store, &key)?,
            };
            let started_waiting = Instant::now();
            let _account = loop {
                if let Some(lock) = try_lock(&account_lock(&root, &key))? {
                    break lock;
                }
                if due_only {
                    return Ok(all);
                }
                pause(started_waiting)?;
            };
            let settings = self.store.settings()?;
            let Some(account) = settings
                .accounts
                .iter()
                .find(|a| crate::accounts::key(a) == key && a.quota_enabled && !a.archived)
            else {
                continue;
            };
            if !settings
                .sources
                .iter()
                .any(|s| account.uses_source(s) && settings.source_enabled(s))
                && !crate::account_scan::has_quota_target(&settings, account)
            {
                continue;
            }
            let signature = fingerprint(&settings, account)?;
            if !fresh
                && let Some(after) = completed(&self.store, &key)?
                && before.as_ref() != Some(&after)
            {
                let cached: Value = serde_json::from_str(&after)?;
                if cached["fingerprint"] == signature && cached["sourceId"] == params["sourceId"] {
                    all.extend(serde_json::from_value::<Vec<QuotaSnapshot>>(
                        cached["rows"].clone(),
                    )?);
                    continue;
                }
            }
            let interval = crate::quota_schedule::interval(account, settings.refresh_seconds);
            if due_only
                && finished(&self.store, &key)?
                    .is_some_and(|last| now() >= last && now() - last < interval)
            {
                continue;
            }
            // Never occupy a global worker slot while waiting for a shared home.
            let paths = locations(&settings, account, &root);
            let _locations = loop {
                let mut locks = vec![];
                for path in &paths {
                    if let Some(lock) = try_lock(path)? {
                        locks.push(lock);
                    } else {
                        break;
                    }
                }
                if locks.len() == paths.len() {
                    break locks;
                }
                drop(locks);
                if due_only {
                    return Ok(all);
                }
                pause(started_waiting)?;
            };
            let _slot = loop {
                let mut acquired = None;
                for slot in 0..4 {
                    if let Some(lock) = try_lock(&root.join(format!("quota-slot-{slot}.lock")))? {
                        acquired = Some(lock);
                        break;
                    }
                }
                if let Some(lock) = acquired {
                    break lock;
                }
                pause(started_waiting)?;
            };
            // A crash leaves the attempt marker, so restart never replays a queue.
            self.store.db.execute(
                "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![format!("quota-attempt:{key}"), now().to_string()],
            )?;
            let mut selected = params.clone();
            selected["accountKey"] = json!(key);
            let result = (|| -> Result<Vec<QuotaSnapshot>> {
                crate::operations::check()?;
                let current = self.store.settings()?;
                let current_account = current
                    .accounts
                    .iter()
                    .find(|a| crate::accounts::key(a) == key)
                    .context("账户已移除")?;
                ensure!(
                    fingerprint(&current, current_account)? == signature,
                    "查询配置已改变，等待下一次刷新"
                );
                let mut sampling_ids: Vec<String> =
                    serde_json::from_value(params["samplingSourceIds"].clone()).unwrap_or_default();
                sampling_ids.extend(
                    self.store
                        .working_estimates(Some(&key))?
                        .iter()
                        .filter(|e| {
                            e.account_key == key
                                && e.status == "active"
                                && e.sampling_mode != "automatic"
                        })
                        .flat_map(|e| e.source_ids.clone()),
                );
                sampling_ids.sort();
                sampling_ids.dedup();
                if !sampling_ids.is_empty() {
                    self.verify_sampling_identities(&sampling_ids, &key)?;
                }
                let revisions = current
                    .sources
                    .iter()
                    .filter(|s| current.source_enabled(s) && current_account.uses_source(s))
                    .map(|s| {
                        Ok((
                            s.id.clone(),
                            crate::accounts::status_revision(&self.store, s)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut rows = self.read_quotas_unlocked(&selected)?;
                // Finish the sampling boundary before Store::quota advances any
                // estimate. Keep the account/home locks through this tail sync.
                if !sampling_ids.is_empty() {
                    self.progress("补齐尾部记录");
                    self.sync_estimate_sources(&sampling_ids)?;
                    if let Err(error) = self.verify_sampling_identities(&sampling_ids, &key) {
                        for id in &sampling_ids {
                            self.store.pause_shared_source_estimates(id)?;
                        }
                        return Err(error);
                    }
                }
                let tx = self.store.write_transaction()?;
                let current = self.store.settings()?;
                let current_account = current
                    .accounts
                    .iter()
                    .find(|a| crate::accounts::key(a) == key)
                    .context("账户已移除")?;
                let mut expected = account.clone();
                // A successful first AGY observation binds its identity itself.
                if expected.provider == "antigravity"
                    && expected.identity_key.is_none()
                    && rows.iter().any(|q| {
                        q.error.is_none()
                            && q.identity.as_ref().is_some_and(|i| {
                                Some(&i.key) == current_account.identity_key.as_ref()
                            })
                    })
                {
                    expected.identity_key = current_account.identity_key.clone();
                }
                let final_signature = fingerprint(&current, current_account)?;
                ensure!(
                    final_signature == fingerprint(&settings, &expected)?,
                    "查询期间账户或机器配置已改变，结果已丢弃"
                );
                for (id, revision) in revisions {
                    let source = current
                        .sources
                        .iter()
                        .find(|s| s.id == id)
                        .context("目录已移除")?;
                    ensure!(
                        crate::accounts::status_revision(&self.store, source)? == revision,
                        "查询期间登录已变化，结果已丢弃"
                    );
                }
                for row in &mut rows {
                    self.store.quota(row)?;
                    let raw: Option<String> = self
                        .store
                        .db
                        .query_row(
                            "SELECT payload FROM quotas WHERE account_key=?1",
                            [format!("{}:{}", row.provider, row.account_id)],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if let Some(raw) = raw {
                        *row = serde_json::from_str(&raw)?;
                    }
                    self.store
                        .observe_quota_reset(row, &current, &final_signature)?;
                }
                self.store.db.execute("INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    rusqlite::params![format!("quota-result:{key}"), serde_json::to_string(&json!({"revision":crate::codex_auth::files::new_id(), "fingerprint":final_signature,"sourceId":params["sourceId"],"rows":rows}))?])?;
                tx.commit()?;
                Ok(rows)
            })();
            // Success and failure both cool down from completion, not start.
            self.store.db.execute(
                "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![format!("quota-attempt:{key}"), now().to_string()],
            )?;
            all.extend(result?);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests;
