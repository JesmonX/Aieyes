//! Account/device configuration, lightweight observations and explicit account removal.
use crate::{Engine, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};

pub fn default_name(provider: &str) -> String {
    match provider {
        "codex" => "Codex · 待登录",
        "claude" => "Claude Code 账户",
        "antigravity" => "Antigravity 账户",
        "deepseek" => "DeepSeek 账户",
        "zcode" => "ZCode 账户",
        "qoder" => "Qoder 账户",
        "qoder-cn" => "Qoder CN 账户",
        "grok" => "Grok 账户",
        "cursor" => "Cursor 账户",
        "opencode" => "OpenCode 账户",
        "kimi" => "Kimi Code 账户",
        "deepseek-harness" => "DeepSeek Harness 账户",
        "codebuddy" => "CodeBuddy 账户",
        "pi" => "Pi 账户",
        "omp" => "Oh My Pi 账户",
        "chatgpt-work" => "ChatGPT Work 账户",
        "workbuddy" => "WorkBuddy 账户",
        "copilot" => "GitHub Copilot 账户",
        "cline" => "Cline 账户",
        "kilo" => "Kilo Code 账户",
        "roo" => "Roo Code 账户",
        "kiro" => "Kiro 账户",
        "trae" => "Trae 账户",
        "trae-cn" => "Trae CN 账户",
        _ => "自定义 Agent 账户",
    }
    .into()
}

pub fn pre_command(settings: &Settings, account: Option<&Account>, source: &Source) -> String {
    let machine = source.host_id.as_deref().unwrap_or("local");
    if let Some(value) =
        account.and_then(|a| a.device_settings.iter().find(|d| d.machine_id == machine))
    {
        return value.pre_command.clone(); // An explicit empty string suppresses legacy inheritance.
    }
    if !source.quota_pre_command.trim().is_empty() {
        return source.quota_pre_command.clone();
    }
    source
        .host_id
        .as_ref()
        .and_then(|id| settings.hosts.iter().find(|h| &h.id == id))
        .map(|h| h.pre_command.clone())
        .unwrap_or_default()
}
pub(crate) fn key(account: &Account) -> String {
    format!("{}:{}", account.provider, account.id)
}
fn cache_key(account: &Account, source: &Source) -> String {
    format!("account-status:{}:{}", key(account), source.id)
}
pub(crate) fn status_revision(store: &Store, source: &Source) -> Result<Value> {
    Ok(get(store, &format!("account-status-revision:{}", source.id))?.unwrap_or(Value::Null))
}
pub(crate) fn codex_switched(store: &Store, source: &Source, operation: &Value) -> Result<()> {
    if source.provider == "codex"
        && source.codex_home_id.is_some()
        && status_revision(store, source)? != *operation
    {
        store.pause_shared_source_estimates(&source.id)?;
        store.invalidate_automatic_sources(std::slice::from_ref(&source.id))?;
    }
    put(
        store,
        &format!("account-status-revision:{}", source.id),
        operation,
    )
}
fn fingerprint(settings: &Settings, account: &Account, source: &Source) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let host = source
        .host_id
        .as_ref()
        .and_then(|id| settings.hosts.iter().find(|h| &h.id == id));
    let connection = host.map(|h| {
        json!([
            h.id,
            h.target,
            h.port,
            h.username,
            h.identity_file,
            h.auth_mode,
            h.password_ref,
            h.shell,
            h.pre_command,
            h.enabled
        ])
    });
    json!([
        source.id,
        source.provider,
        source.account_id,
        source.path,
        source.host_id,
        source.machine_binding,
        source.codex_home_id,
        source.codex_binary,
        source.agy_binary,
        source.subscription,
        (source.provider != "codex").then_some(&source.proxy),
        (source.provider != "codex" && source.host_id.is_none()).then_some(&settings.proxy),
        account.identity_key,
        account.profile_refs(),
        // Invalidate pre-CLI observations and follow the effective native root.
        (crate::qoder_family::is_qoder(&source.provider)).then(|| (
            1,
            crate::subscriptions::qoder::config_dir(source, source.subscription.as_ref()).ok()
        )),
        pre_command(settings, Some(account), source),
        connection
    ])
    .to_string()
    .hash(&mut hash);
    hash.finish()
}
fn get(store: &Store, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
}
fn put(store: &Store, key: &str, value: &Value) -> Result<()> {
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, value.to_string()],
    )?;
    Ok(())
}

pub(crate) fn agy_status(
    account: Option<&Account>,
    identity: Option<&AccountIdentity>,
    metadata_error: Option<&str>,
) -> Value {
    let current = identity.map(|id| crate::agy_identity::matches(account, id));
    let note = match current {
        Some(true) => "已确认当前登录账户",
        Some(false) => "设备当前登录身份与此账户不一致；请在 agy 中切换后重新检查",
        None => "登录有效；账户身份暂无法确认",
    };
    json!({"authenticated":true,"identityConfirmed":identity.is_some(),"identity":identity,
        "current":current,"credential":current,"checkedAt":now(),"note":note,"metadataError":metadata_error})
}

fn retain_agy_subscription(row: &mut Value, prior: Option<Value>) {
    let Some(old) = prior else {
        return;
    };
    if row["identity"]["key"].is_string()
        && row["identity"]["key"] == old["identity"]["key"]
        && row["identity"]["stale"] == true
        && row["identity"]["subscription"].is_null()
    {
        row["identity"]["subscription"] = old["identity"]["subscription"].clone();
        row["identity"]["subscriptionCheckedAt"] = old["identity"]["subscriptionCheckedAt"].clone();
    }
}

// Call inside a transaction. Bind once; never silently replace an established identity.
fn bind_agy_identity(
    store: &Store,
    settings: &Settings,
    account: &Account,
    identity: &AccountIdentity,
) -> Result<Settings> {
    if !crate::agy_identity::matches(Some(account), identity) {
        return Ok(settings.clone());
    }
    let mut next = settings.clone();
    let a = next
        .accounts
        .iter_mut()
        .find(|a| key(a) == key(account))
        .context("账户已删除")?;
    let mut changed = false;
    if a.identity_key.is_none() {
        a.identity_key = Some(identity.key.clone());
        changed = true;
        if a.pending_name || a.name == default_name("antigravity") || a.name.is_empty() {
            a.name = identity.email.clone();
            a.pending_name = false;
        }
    }
    if changed {
        store.save_auth_settings(&next)?;
    }
    Ok(next)
}

pub(crate) fn record_agy_observation(
    store: &Store,
    settings: &Settings,
    source: &Source,
    q: &QuotaSnapshot,
    revision: &Value,
) -> Result<()> {
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == "antigravity" && a.id == source.account_id);
    let row = agy_status(account, q.identity.as_ref(), q.metadata_error.as_deref());
    record_agy_status(store, settings, source, row, revision, None)?;
    Ok(())
}

/// Persist the same public observation returned by login, without another CLI query.
pub(crate) fn record_agy_status(
    store: &Store,
    settings: &Settings,
    source: &Source,
    mut row: Value,
    revision: &Value,
    next_revision: Option<&Value>,
) -> Result<Value> {
    let Some(account) = settings
        .accounts
        .iter()
        .find(|a| a.provider == "antigravity" && a.id == source.account_id)
    else {
        return Ok(Value::Null);
    };
    let tx = store.write_transaction()?;
    ensure!(
        status_revision(store, source)? == *revision,
        "登录状态已变化，请重新检查"
    );
    let latest = store.settings()?;
    let pair = latest
        .accounts
        .iter()
        .find(|a| key(a) == key(account))
        .zip(latest.sources.iter().find(|s| s.id == source.id));
    ensure!(
        pair.is_some_and(|(a, s)| {
            let mut expected = account.clone();
            // A parallel check may have bound the same identity while login was running.
            if expected.identity_key.is_none()
                && a.identity_key
                    .as_deref()
                    .is_some_and(|key| row["identity"]["key"] == key)
            {
                expected.identity_key = a.identity_key.clone();
            }
            fingerprint(&latest, a, s) == fingerprint(settings, &expected, source)
        }),
        "设备配置已变化，请重新检查"
    );
    let current_account = pair.unwrap().0;
    let next =
        if let Ok(identity) = serde_json::from_value::<AccountIdentity>(row["identity"].clone()) {
            bind_agy_identity(store, &latest, current_account, &identity)?
        } else {
            latest
        };
    let a = next
        .accounts
        .iter()
        .find(|a| key(a) == key(account))
        .unwrap();
    retain_agy_subscription(&mut row, get(store, &cache_key(a, source))?);
    if let Some(next_revision) = next_revision {
        // Invalidate checks started during authorization in the same transaction as success.
        codex_switched(store, source, next_revision)?;
    }
    row["fingerprint"] = json!(fingerprint(&next, a, source));
    row["authRevision"] = status_revision(store, source)?;
    row["attemptedAt"] = json!(now());
    put(store, &cache_key(a, source), &row)?;
    tx.commit()?;
    row["accountKey"] = json!(key(a));
    row["sourceId"] = json!(source.id);
    row["machineId"] = json!(source.host_id.as_deref().unwrap_or("local"));
    row.as_object_mut().unwrap().remove("fingerprint");
    row.as_object_mut().unwrap().remove("authRevision");
    Ok(row)
}
pub fn task_uses(account: &Account, settings: &Settings, record: &crate::wakeups::Record) -> bool {
    let task_matches = |task: &crate::wakeups::Task| {
        task.codex_profile_id
            .as_deref()
            .is_some_and(|p| account.has_profile(p))
            || (task.codex_profile_id.is_none()
                && settings.sources.iter().any(|s| {
                    s.id == task.source_id
                        && s.provider == account.provider
                        && s.account_id == account.id
                }))
    };
    task_matches(&record.task)
        || record.deployment.as_ref().is_some_and(|d| {
            task_matches(&d.task)
                || (d.source.provider == account.provider && d.source.account_id == account.id)
        })
}
pub fn lock_tasks(store: &Store) -> Result<crate::file_lock::FileLock> {
    lock_task_ownership(store, false)
}
pub(crate) fn lock_task_ownership(
    store: &Store,
    shared: bool,
) -> Result<crate::file_lock::FileLock> {
    let root = std::path::Path::new(store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("account-tasks.lock"))?;
    let lock = if shared {
        crate::file_lock::FileLock::try_shared(file)
    } else {
        crate::file_lock::FileLock::try_exclusive(file)
    };
    lock.map_err(|_| anyhow::anyhow!("有账户任务正在部署或删除，请稍后重试"))
}
pub fn ensure_not_deleting(store: &Store, account: &Account) -> Result<()> {
    ensure!(
        get(store, &format!("account-deleting:{}", key(account)))?.is_none(),
        "此账户正在删除，请等待清理完成"
    );
    Ok(())
}
fn public_task(record: &crate::wakeups::Record) -> Value {
    json!({"id":record.task.id,"name":record.task.name,"deployed":record.deployment.is_some(),
        "machine":record.deployment.as_ref().and_then(|d| d.host.as_ref()).map(|h| if h.name.is_empty() { &h.target } else { &h.name }).map(String::as_str).unwrap_or("本机"),
        "root":record.deployment.as_ref().map(|d| &d.root)})
}
impl Engine {
    fn status_observation_key(
        &self,
        settings: &Settings,
        account: &Account,
        source: &Source,
    ) -> Result<String> {
        // A native quota connection and a history directory can point at the
        // same Qoder login. Query that root once, then project it onto accounts.
        let location = if crate::qoder_family::is_qoder(&source.provider)
            && source
                .subscription
                .as_ref()
                .is_none_or(|c| crate::subscriptions::native_cli(&source.provider, c))
        {
            crate::subscriptions::qoder::config_dir(source, source.subscription.as_ref())
                .map(Value::String)
                .unwrap_or_else(|_| json!([source.path, source.subscription]))
        } else {
            json!([source.path, source.subscription])
        };
        Ok(crate::models::hash(
            &json!([
                source.provider,
                location,
                source.codex_binary,
                pre_command(settings, Some(account), source),
                source.proxy,
                settings.proxy,
                source
                    .host_id
                    .as_ref()
                    .and_then(|id| settings.hosts.iter().find(|h| &h.id == id)),
                status_revision(&self.store, source)?
            ])
            .to_string(),
        ))
    }
    pub(crate) fn refresh_account_status_batch(&mut self, params: &Value) -> Result<Value> {
        use std::{
            collections::{BTreeMap, VecDeque},
            sync::{Mutex, mpsc},
        };
        let settings = self.store.settings()?;
        let mut groups = BTreeMap::<String, Vec<(String, String)>>::new();
        for source in settings.sources.iter().filter(|s| {
            settings.source_enabled(s)
                && params["sourceIds"]
                    .as_array()
                    .is_none_or(|ids| ids.iter().any(|id| id == &s.id))
        }) {
            for account in settings.accounts.iter().filter(|a| {
                !a.archived
                    && a.provider == source.provider
                    && params["accountKeys"]
                        .as_array()
                        .is_none_or(|keys| keys.iter().any(|k| k == &key(a)))
            }) {
                if params["dueOnly"] == true
                    && let Some(cached) = get(&self.store, &cache_key(account, source))?
                    && cached["fingerprint"] == fingerprint(&settings, account, source)
                    && cached["authRevision"] == status_revision(&self.store, source)?
                    && let Some(at) = cached["attemptedAt"]
                        .as_i64()
                        .or_else(|| cached["checkedAt"].as_i64())
                    && now() >= at
                    && now() - at
                        < account
                            .quota_refresh_seconds
                            .unwrap_or(settings.refresh_seconds)
                            .max(300)
                            .max(crate::quota_schedule::floor(&account.provider))
                            as i64
                {
                    continue;
                }
                groups
                    .entry(self.status_observation_key(&settings, account, source)?)
                    .or_default()
                    .push((key(account), source.id.clone()));
            }
        }
        let jobs = Mutex::new(VecDeque::from_iter(groups));
        let count = jobs.lock().unwrap().len().min(3);
        let root = self.store.root.clone();
        let context = crate::operations::context();
        let (tx, rx) = mpsc::channel();
        let mut all = Vec::new();
        let mut errors = Vec::new();
        #[cfg(test)]
        let reads = &self.status_observation_reads;
        std::thread::scope(|scope| {
            for _ in 0..count {
                let (jobs, tx, root, context) = (&jobs, tx.clone(), &root, context.clone());
                scope.spawn(move || {
                    let _operation = crate::operations::inherit(context);
                    let mut worker = match Engine::open(root) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    worker.status_observations = Some(Default::default());
                    #[cfg(test)]
                    {
                        worker.status_observation_reads = reads.clone();
                    }
                    loop {
                        let job = { jobs.lock().unwrap().pop_front() };
                        let Some((observation, pairs)) = job else {
                            break;
                        };
                        if crate::operations::check().is_err() {
                            break;
                        }
                        let path = root.join(format!("account-observation-{observation}.lock"));
                        let started = std::time::Instant::now();
                        let lock = loop {
                            match crate::quota_queries::try_lock(&path) {
                                Ok(Some(lock)) => break Some(lock),
                                Ok(None) if params["dueOnly"] == true => break None,
                                Ok(None) => {
                                    if crate::operations::check().is_err()
                                        || started.elapsed().as_secs() >= 120
                                    {
                                        let _ =
                                            tx.send(Err("设备状态检查仍在进行，请稍后重试".into()));
                                        break None;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                                Err(error) => {
                                    let _ = tx.send(Err(error.to_string()));
                                    break None;
                                }
                            }
                        };
                        let Some(lock) = lock else {
                            continue;
                        };
                        for (account, source) in pairs {
                            // Recheck due state after the cross-process observation lock.
                            if params["dueOnly"] == true {
                                let fresh = worker.store.settings().ok();
                                if let Some(s) = fresh.as_ref()
                                    && let Some(a) = s.accounts.iter().find(|a| key(a) == account)
                                    && let Some(src) = s.sources.iter().find(|s| s.id == source)
                                    && let Ok(Some(row)) = get(&worker.store, &cache_key(a, src))
                                    && row["fingerprint"] == fingerprint(s, a, src)
                                    && row["authRevision"]
                                        == status_revision(&worker.store, src)
                                            .unwrap_or(Value::Null)
                                    && let Some(at) = row["attemptedAt"]
                                        .as_i64()
                                        .or_else(|| row["checkedAt"].as_i64())
                                    && now() >= at
                                    && now() - at
                                        < a.quota_refresh_seconds
                                            .unwrap_or(s.refresh_seconds)
                                            .max(300)
                                            .max(crate::quota_schedule::floor(&a.provider))
                                            as i64
                                {
                                    continue;
                                }
                            }
                            let result = worker
                                .account_call(
                                    "accounts.status.refresh",
                                    json!({"accountKey":account,"sourceId":source}),
                                )
                                .map_err(|e| e.to_string());
                            if tx.send(result).is_err() {
                                return;
                            }
                        }
                        drop(lock);
                        worker
                            .status_observations
                            .as_ref()
                            .unwrap()
                            .borrow_mut()
                            .clear();
                    }
                });
            }
            drop(tx);
            for result in rx {
                match result {
                    Ok(row) => {
                        if let Some(progress) = &mut self.progress {
                            progress(json!({"kind":"accounts.status.refresh","row":row}));
                        }
                        all.push(row);
                    }
                    Err(error) => {
                        errors.push(error);
                    }
                }
            }
        });
        crate::operations::check()?;
        self.refresh_native_connections(params)?;
        let latest = self.store.settings()?;
        all.extend(
            crate::account_scan::status_rows(&latest)
                .into_iter()
                .filter(|row| {
                    params["accountKeys"]
                        .as_array()
                        .is_none_or(|keys| keys.contains(&row["accountKey"]))
                        && params["sourceIds"]
                            .as_array()
                            .is_none_or(|ids| ids.contains(&row["sourceId"]))
                }),
        );
        if all.is_empty() && !errors.is_empty() {
            anyhow::bail!(
                "设备状态检查未完成：{}",
                errors.into_iter().take(3).collect::<Vec<_>>().join("；")
            );
        }
        Ok(json!(all))
    }
    pub(crate) fn account_call(&mut self, method: &str, params: Value) -> Result<Value> {
        if method == "accounts.cleanup.list" {
            let mut stmt = self
                .store
                .db
                .prepare("SELECT value FROM kv WHERE key LIKE 'account-cleanup:%' ORDER BY key")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(json!(
                rows.iter()
                    .map(|r| serde_json::from_str::<Value>(r))
                    .collect::<std::result::Result<Vec<_>, _>>()?
            ));
        }
        let settings = self.store.settings()?;
        if method == "accounts.create" {
            let provider = params["provider"].as_str().context("请选择 Agent")?;
            ensure!(
                settings
                    .agents
                    .iter()
                    .any(|a| a.provider == provider && a.enabled),
                "请先启用此 Agent"
            );
            let name = params["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let mut next = settings.clone();
            let id = crate::codex_auth::files::new_id();
            next.accounts.push(Account {
                id: id.clone(),
                name: name
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_name(provider)),
                pending_name: matches!(provider, "codex" | "antigravity") && name.is_none(),
                provider: provider.into(),
                ..Default::default()
            });
            let saved = self
                .store
                .patch_settings(&json!(settings), &json!(next), false)?;
            return Ok(json!({"settings":saved,"accountKey":format!("{provider}:{id}")}));
        }
        if method == "accounts.status.get" {
            let mut rows = vec![];
            for account in &settings.accounts {
                for source in settings
                    .sources
                    .iter()
                    .filter(|s| s.provider == account.provider)
                {
                    let mut row = get(&self.store, &cache_key(account, source))?.unwrap_or_else(
                        || json!({"current":null,"credential":null,"checkedAt":null}),
                    );
                    if row["fingerprint"] != json!(fingerprint(&settings, account, source))
                        || row["authRevision"] != status_revision(&self.store, source)?
                    {
                        row = json!({"current":null,"credential":null,"checkedAt":null});
                    }
                    row["accountKey"] = json!(key(account));
                    row["sourceId"] = json!(source.id);
                    row["machineId"] = json!(source.host_id.as_deref().unwrap_or("local"));
                    row.as_object_mut().unwrap().remove("fingerprint");
                    row.as_object_mut().unwrap().remove("authRevision");
                    rows.push(row);
                }
            }
            rows.extend(crate::account_scan::status_rows(&settings));
            return Ok(json!(rows));
        }
        if method == "accounts.deployments.sync" {
            return self.sync_account_deployments(&params);
        }
        let account_key = params["accountKey"].as_str().context("请选择账户")?;
        if method == "accounts.delete"
            && !settings.accounts.iter().any(|a| key(a) == account_key)
            && get(&self.store, &format!("deleted-account:{account_key}"))?.is_some()
        {
            return Ok(json!({"deleted":true,"settings":settings}));
        }
        let account = settings
            .accounts
            .iter()
            .find(|a| key(a) == account_key)
            .context("账户已删除，请刷新")?
            .clone();
        if method == "accounts.status.refresh" {
            let source = settings
                .sources
                .iter()
                .find(|s| params["sourceId"] == s.id && s.provider == account.provider)
                .context("设备目录不存在")?;
            ensure!(settings.source_enabled(source), "设备已暂停");
            let stamp = fingerprint(&settings, &account, source);
            let auth_revision = status_revision(&self.store, source)?;
            let result = self.inspect_account_device(&settings, &account, source);
            // Observations made with old settings must never overwrite a new device's state.
            let transaction = self.store.write_transaction()?;
            let latest = self.store.settings()?;
            let unchanged = latest
                .accounts
                .iter()
                .find(|a| key(a) == account_key)
                .zip(latest.sources.iter().find(|s| s.id == source.id))
                .is_some_and(|(a, s)| {
                    latest.source_enabled(s) && fingerprint(&latest, a, s) == stamp
                });
            ensure!(unchanged, "设备配置已变化，请重新检查");
            // Keep the generation check and cache write in the same transaction, so an
            // observation started before a switch cannot overwrite the new account state.
            ensure!(
                status_revision(&self.store, source)? == auth_revision,
                "账户已切换，请重新检查设备状态"
            );
            let mut row = match result {
                Ok(v) => v,
                Err(e) => {
                    let mut old = get(&self.store, &cache_key(&account, source))?
                        .filter(|v| {
                            v["fingerprint"] == json!(stamp) && v["authRevision"] == auth_revision
                        })
                        .unwrap_or_else(
                            || json!({"current":null,"credential":null,"checkedAt":null}),
                        );
                    old["error"] = json!(e.to_string());
                    old
                }
            };
            let mut observed_settings = latest;
            if account.provider == "antigravity"
                && row["current"] == true
                && let Ok(identity) =
                    serde_json::from_value::<AccountIdentity>(row["identity"].clone())
            {
                let latest_account = observed_settings
                    .accounts
                    .iter()
                    .find(|a| key(a) == account_key)
                    .unwrap();
                observed_settings =
                    bind_agy_identity(&self.store, &observed_settings, latest_account, &identity)?;
            }
            let observed_account = observed_settings
                .accounts
                .iter()
                .find(|a| key(a) == account_key)
                .unwrap();
            if account.provider == "antigravity" {
                retain_agy_subscription(&mut row, get(&self.store, &cache_key(&account, source))?);
            }
            row["attemptedAt"] = json!(now());
            row["fingerprint"] = json!(fingerprint(&observed_settings, observed_account, source));
            row["authRevision"] = auth_revision;
            put(&self.store, &cache_key(&account, source), &row)?;
            transaction.commit()?;
            row["accountKey"] = json!(account_key);
            row["sourceId"] = json!(source.id);
            row["machineId"] = json!(source.host_id.as_deref().unwrap_or("local"));
            row.as_object_mut().unwrap().remove("fingerprint");
            row.as_object_mut().unwrap().remove("authRevision");
            return Ok(row);
        }
        let tasks = self
            .store
            .wakeups()?
            .into_iter()
            .filter(|r| task_uses(&account, &settings, r))
            .collect::<Vec<_>>();
        if method == "accounts.deletion.preview" {
            return Ok(
                json!({"accountKey":account_key,"name":account.name,"tasks":tasks.iter().map(public_task).collect::<Vec<_>>() }),
            );
        }
        ensure!(method == "accounts.delete", "未知账户操作");
        ensure!(account.archived, "请先归档账户再删除");
        // Serialize deletion against new deployments, without holding a SQL lock over SSH.
        let _task_lock = lock_tasks(&self.store)?;
        let marker = format!("account-deleting:{account_key}");
        put(&self.store, &marker, &json!({"startedAt":now()}))?;
        let result =
            self.delete_archived_account(&account, params["force"].as_bool().unwrap_or(false));
        self.store
            .db
            .execute("DELETE FROM kv WHERE key=?1", [marker])?;
        result
    }

    fn inspect_account_device(
        &self,
        settings: &Settings,
        account: &Account,
        source: &Source,
    ) -> Result<Value> {
        if source.provider == "antigravity" && source.account_id == account.id {
            return crate::agy_auth::inspect(source, settings);
        }
        if source.provider == "codex" {
            let observation = self.status_observation_key(settings, account, source)?;
            let cached = self
                .status_observations
                .as_ref()
                .and_then(|cache| cache.borrow().get(&observation).cloned());
            let info = if let Some(value) = cached {
                value
            } else {
                #[cfg(test)]
                if let Some(reads) = &self.status_observation_reads {
                    reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                let value = crate::codex_auth::backend(
                    source,
                    settings,
                    "status",
                    &json!({"accountId":account.id}),
                )
                .unwrap_or_else(|e| json!({"_statusError":format!("{e:#}")}));
                if let Some(cache) = &self.status_observations {
                    cache.borrow_mut().insert(observation, value.clone());
                }
                value
            };
            if let Some(error) = info["_statusError"].as_str() {
                anyhow::bail!("{error}");
            }
            self.store.record_codex_observation(
                settings,
                source,
                info["currentIdentity"].as_str().unwrap_or(""),
                info["observedAt"].as_i64().unwrap_or_else(now),
            )?;
            if info["storageMode"] != "file" {
                return Ok(
                    json!({"current":null,"credential":null,"checkedAt":now(),"note":"认证存储不支持状态识别"}),
                );
            }
            let profiles = info["profiles"].as_array().cloned().unwrap_or_default();
            let matching = profiles
                .iter()
                .filter(|p| {
                    account
                        .identity_key
                        .as_deref()
                        .is_some_and(|k| p["identityKey"] == k)
                        || source.codex_home_id.as_ref().is_some_and(|h| {
                            p["id"]
                                .as_str()
                                .is_some_and(|id| account.has_profile(&format!("{h}:{id}")))
                        })
                })
                .collect::<Vec<_>>();
            let identity = account.identity_key.clone().or_else(|| {
                matching
                    .first()
                    .and_then(|p| p["identityKey"].as_str().map(str::to_owned))
            });
            let current = identity.as_deref().map(|id| info["currentIdentity"] == id);
            let credential = identity.as_deref().map(|id| {
                (info["currentIdentity"] == id && info["currentCredential"] == true)
                    || matching.iter().any(|p| p["credential"] == true)
            });
            let profile_credentials: serde_json::Map<String, Value> = matching
                .iter()
                .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p["credential"].clone())))
                .collect();
            return Ok(
                json!({"current":current,"credential":credential,"profileCredentials":profile_credentials,"checkedAt":now(),"note": if identity.is_none() { "尚未验证账户身份，请读取当前登录或授权" } else { "" }}),
            );
        }
        if crate::qoder_family::is_qoder(&source.provider)
            && source
                .subscription
                .as_ref()
                .is_none_or(|c| crate::subscriptions::native_cli(&source.provider, c))
        {
            if source.host_id.is_none()
                && source.subscription.is_none()
                && crate::qoder_desktop::database(&source.path).is_some()
            {
                return Ok(
                    json!({"current":null,"credential":null,"checkedAt":now(),"note":"账户身份待确认"}),
                );
            }
            let observation = self.status_observation_key(settings, account, source)?;
            let cached = self
                .status_observations
                .as_ref()
                .and_then(|cache| cache.borrow().get(&observation).cloned());
            let info = if let Some(value) = cached {
                value
            } else {
                #[cfg(test)]
                if let Some(reads) = &self.status_observation_reads {
                    reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                let value = crate::subscriptions::qoder::inspect(
                    source,
                    settings,
                    &pre_command(settings, Some(account), source),
                )
                .map(|identity| json!(identity))
                .unwrap_or_else(|e| json!({"_statusError":e.to_string()}));
                if let Some(cache) = &self.status_observations {
                    cache.borrow_mut().insert(observation, value.clone());
                }
                value
            };
            if let Some(error) = info["_statusError"].as_str() {
                anyhow::bail!("{error}");
            }
            let identity: AccountIdentity = serde_json::from_value(info)?;
            let current = account
                .identity_key
                .as_ref()
                .map(|key| key == &identity.key);
            let note = match current {
                Some(true) => "已确认 Qoder CLI 当前登录账户",
                Some(false) => "此机器当前登录另一 Qoder 账户",
                None => "已读取 Qoder CLI 当前登录；此账户尚未绑定身份",
            };
            return Ok(
                json!({"current":current,"credential":current,"authenticated":true,
                "identityConfirmed":true,"identity":identity,"checkedAt":now(),"note":note,
                "plan":identity.subscription}),
            );
        }
        if source.subscription.is_some() && account.uses_source(source) {
            let quota = crate::subscriptions::read(source, settings)?;
            return Ok(
                json!({"current":null,"credential":true,"authenticated":true,"identityConfirmed":false,"checkedAt":now(),"note":"已验证查询凭据；客户端当前账户待确认","plan":quota.plan}),
            );
        }
        // Other providers cannot safely infer the identity of a machine's current login.
        let credential = if !account.uses_source(source) {
            Some(false)
        } else if source.provider == "deepseek" && source.host_id.is_none() {
            Some(crate::store::expand(&source.path).is_file())
        } else if source.provider == "claude"
            && source.host_id.is_none()
            && crate::store::expand(&source.path)
                .join(".credentials.json")
                .is_file()
        {
            Some(true)
        } else {
            None
        };
        Ok(
            json!({"current":null,"credential":credential,"checkedAt":now(),"note":"已连接目录；当前账户需在 Agent 中确认"}),
        )
    }

    fn delete_archived_account(&mut self, account: &Account, force: bool) -> Result<Value> {
        let settings = self.store.settings()?;
        let tasks = self
            .store
            .wakeups()?
            .into_iter()
            .filter(|r| task_uses(account, &settings, r))
            .collect::<Vec<_>>();
        let mut failed = vec![];
        let mut abandoned = vec![];
        let mut forced_ids = vec![];
        for record in tasks {
            if record.deployment.is_some() {
                if force {
                    abandoned.push(public_task(&record));
                } else {
                    self.progress(&format!("正在停止并移除任务：{}", record.task.name));
                    if let Err(e) =
                        self.wakeup_call_unlocked("wakeups.remove", json!({"id":record.task.id}))
                    {
                        let mut failure = public_task(&record);
                        failure["error"] = json!(e.to_string());
                        failed.push(failure);
                        continue;
                    }
                }
            }
            if force {
                forced_ids.push(record.task.id);
            } else {
                self.wakeup_call_unlocked("wakeups.delete", json!({"id":record.task.id}))?;
            }
        }
        if !failed.is_empty() {
            return Ok(json!({"deleted":false,"failedTasks":failed,"canForce":true}));
        }
        let tx = self.store.write_transaction()?;
        let base = self.store.settings()?;
        ensure!(
            base.accounts
                .iter()
                .any(|a| key(a) == key(account) && a.archived),
            "账户状态已变化，请重新确认删除"
        );
        for id in forced_ids {
            self.store
                .db
                .execute("DELETE FROM kv WHERE key=?1", [format!("wakeup:{id}")])?;
        }
        ensure!(
            !self
                .store
                .wakeups()?
                .iter()
                .any(|r| task_uses(account, &base, r)),
            "出现新的关联任务，请重试清理"
        );
        let mut next = base.clone();
        next.accounts.retain(|a| key(a) != key(account));
        for source in &mut next.sources {
            if source.provider == account.provider && source.account_id == account.id {
                source.account_id.clear();
            }
        }
        put(
            &self.store,
            &format!("deleted-account:{}", key(account)),
            &json!({"id":account.id,"provider":account.provider,"name":account.name,"deletedAt":now()}),
        )?;
        if !abandoned.is_empty() {
            put(
                &self.store,
                &format!("account-cleanup:{}", key(account)),
                &json!({"accountKey":key(account),"name":account.name,"deletedAt":now(),"tasks":abandoned,"warning":"已解除本地管理；这些设备上的远端任务可能仍在运行"}),
            )?;
        }
        self.store.save_auth_settings(&next)?;
        let saved = self.store.settings()?;
        tx.commit()?;
        Ok(json!({"deleted":true,"settings":saved,"abandonedTasks":abandoned}))
    }

    fn sync_account_deployments(&mut self, params: &Value) -> Result<Value> {
        let mut results = vec![];
        let mut synced = vec![];
        for record in self
            .store
            .wakeups()?
            .into_iter()
            .filter(|r| params["taskId"].as_str().is_none_or(|id| r.task.id == id))
        {
            let Some(deployment) = &record.deployment else {
                continue;
            };
            if deployment.state != "settings-pending" {
                continue;
            }
            let id = &record.task.id;
            let result = (|| -> Result<Option<Value>> {
                let _lock = crate::wakeups::lock_task(&self.store, id)?;
                self.sync_wakeup_settings_unlocked(id)
            })();
            match result {
                Err(e) => {
                    let mut failure = public_task(&record);
                    failure["error"] = json!(e.to_string());
                    results.push(failure);
                }
                Ok(Some(value)) => synced.push(value),
                _ => {}
            }
        }
        Ok(json!({"failedTasks":results,"syncedTasks":synced}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wakeups::{Deployment, Record, Task};
    fn fixture() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            account_id: "a".into(),
            path: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        engine
            .store
            .save_settings(&Settings {
                sources: vec![source],
                accounts: vec![Account {
                    id: "a".into(),
                    name: "Preserved account".into(),
                    archived: true,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        (dir, engine)
    }
    #[test]
    fn agy_login_commit_rejects_older_checks_and_accepts_parallel_same_identity_binding() {
        let (_dir, mut e) = fixture();
        let mut settings = e.store.settings().unwrap();
        settings.sources[0].provider = "antigravity".into();
        settings.accounts[0].provider = "antigravity".into();
        settings.accounts[0].archived = false;
        e.store.save_auth_settings(&settings).unwrap();
        let source = settings.sources[0].clone();
        let identity = AccountIdentity {
            key: format!("google:{}", "a".repeat(64)),
            email: "fixture@example.test".into(),
            ..Default::default()
        };
        let revision = json!({"session":"authorizing"});
        codex_switched(&e.store, &source, &revision).unwrap();
        let q = QuotaSnapshot {
            identity: Some(identity.clone()),
            ..Default::default()
        };
        // Another observation binds the same identity before the login is published.
        record_agy_observation(&e.store, &settings, &source, &q, &revision).unwrap();
        let new_revision = json!({"session":"confirmed"});
        let row = record_agy_status(
            &e.store,
            &settings,
            &source,
            agy_status(Some(&settings.accounts[0]), Some(&identity), None),
            &revision,
            Some(&new_revision),
        )
        .unwrap();
        assert_eq!(row["current"], true);
        assert_eq!(status_revision(&e.store, &source).unwrap(), new_revision);
        let latest = e.store.settings().unwrap();
        // Even with unchanged identity/configuration, a check started before success is stale.
        assert!(
            record_agy_observation(
                &e.store,
                &latest,
                &source,
                &QuotaSnapshot::default(),
                &revision
            )
            .is_err()
        );
        assert_eq!(
            e.account_call("accounts.status.get", json!({})).unwrap()[0]["identity"]["key"],
            identity.key
        );
    }
    #[test]
    fn agy_observations_reject_changed_configuration_and_authentication_revision() {
        let (_dir, e) = fixture();
        let mut settings = e.store.settings().unwrap();
        settings.sources[0].provider = "antigravity".into();
        settings.accounts[0].provider = "antigravity".into();
        settings.accounts[0].archived = false;
        e.store.save_auth_settings(&settings).unwrap();
        let source = settings.sources[0].clone();
        let q = QuotaSnapshot {
            identity: Some(AccountIdentity {
                key: format!("google:{}", "a".repeat(64)),
                email: "fixture@example.test".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let revision = status_revision(&e.store, &source).unwrap();
        let mut changed = settings.clone();
        changed.proxy.mode = "direct".into();
        e.store.save_settings(&changed).unwrap();
        assert!(record_agy_observation(&e.store, &settings, &source, &q, &revision).is_err());
        assert!(
            e.store.settings().unwrap().accounts[0]
                .identity_key
                .is_none()
        );
        e.store.save_settings(&settings).unwrap();
        codex_switched(&e.store, &source, &json!({"session":"new"})).unwrap();
        assert!(record_agy_observation(&e.store, &settings, &source, &q, &revision).is_err());
        assert!(
            e.store.settings().unwrap().accounts[0]
                .identity_key
                .is_none()
        );
    }
    #[test]
    fn empty_deleted_accounts_stay_deleted_without_becoming_history_filters() {
        let (dir, mut e) = fixture();
        let result = e
            .account_call("accounts.delete", json!({"accountKey":"codex:a"}))
            .unwrap();
        assert_eq!(result["deleted"], true);
        assert!(e.store.settings().unwrap().deleted_accounts.is_empty());
        assert_eq!(
            e.account_call("accounts.delete", json!({"accountKey":"codex:a"}))
                .unwrap()["deleted"],
            true
        );
        drop(e);
        let e = Engine::open(dir.path()).unwrap();
        let s = e.store.settings().unwrap();
        assert!(s.accounts.is_empty() && s.deleted_accounts.is_empty());
        assert!(!s.history_show_deleted_accounts);
        assert!(get(&e.store, "deleted-account:codex:a").unwrap().is_some());
    }
    #[test]
    fn delete_preserves_history_credentials_and_deduplication_namespace() {
        let (dir, mut e) = fixture();
        let credential = dir.path().join("auth.json");
        std::fs::write(&credential, "FAKE-SECRET").unwrap();
        e.store
            .db
            .execute(
                "INSERT INTO events VALUES('event','codex','a','m',1,'{}',2,3,NULL)",
                [],
            )
            .unwrap();
        let base = e.store.settings().unwrap();
        let mut removed = base.clone();
        removed.accounts.clear();
        removed.sources[0].account_id.clear();
        assert!(e.store.save_settings(&removed).is_err());
        let result = e
            .account_call("accounts.delete", json!({"accountKey":"codex:a"}))
            .unwrap();
        assert_eq!(result["deleted"], true);
        let settings = e.store.settings().unwrap();
        assert!(settings.accounts.is_empty());
        assert_eq!(settings.deleted_accounts[0].name, "Preserved account");
        assert_eq!(settings.sources[0].account_id, "");
        assert_eq!(std::fs::read_to_string(credential).unwrap(), "FAKE-SECRET");
        assert_eq!(
            e.store
                .db
                .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            e.store
                .db
                .query_row(
                    "SELECT account_id FROM source_identities WHERE source_id='s'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "a"
        );
        // A stale client's edit cannot resurrect the deleted account.
        let mut stale = base.clone();
        stale.accounts[0].name = "changed".into();
        assert!(
            e.store
                .patch_settings(&json!(base), &json!(stale), false)
                .is_err()
        );
    }
    #[test]
    fn normal_deletion_removes_drafts_and_force_records_unreachable_deployments() {
        let (_dir, mut e) = fixture();
        let settings = e.store.settings().unwrap();
        let task = Task {
            id: "job".into(),
            name: "abandoned job".into(),
            source_id: "s".into(),
            ..Default::default()
        };
        let draft = Task {
            id: "draft".into(),
            ..task.clone()
        };
        e.store
            .save_wakeup(&Record {
                task: draft,
                deployment: None,
            })
            .unwrap();
        let record = Record {
            task: task.clone(),
            deployment: Some(Deployment {
                task,
                source: settings.sources[0].clone(),
                host: Some(Host {
                    id: "gone".into(),
                    name: "Abandoned machine".into(),
                    target: "".into(),
                    ..Default::default()
                }),
                root: "~/fixture/jobs".into(),
                deployed_at: 1,
                enabled: true,
                state: "deployed".into(),
                timezone: "UTC".into(),
            }),
        };
        e.store.save_wakeup(&record).unwrap();
        let result = e
            .account_call("accounts.delete", json!({"accountKey":"codex:a"}))
            .unwrap();
        assert_eq!(result["deleted"], false);
        assert_eq!(result["failedTasks"].as_array().unwrap().len(), 1);
        assert_eq!(e.store.wakeups().unwrap().len(), 1);
        assert_eq!(e.store.settings().unwrap().accounts.len(), 1);
        let result = e
            .account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true}),
            )
            .unwrap();
        assert_eq!(result["deleted"], true);
        assert!(e.store.wakeups().unwrap().is_empty());
        let audit = e.account_call("accounts.cleanup.list", json!({})).unwrap();
        assert_eq!(audit[0]["tasks"][0]["root"], "~/fixture/jobs");
        assert!(!audit.to_string().contains("passwordRef"));
    }
    #[test]
    fn active_account_cannot_be_deleted_and_deploy_lock_prevents_races() {
        let (_dir, mut e) = fixture();
        let lock = lock_tasks(&e.store).unwrap();
        assert!(
            e.account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true})
            )
            .is_err()
        );
        drop(lock);
        let mut s = e.store.settings().unwrap();
        s.accounts[0].archived = false;
        e.store.save_settings(&s).unwrap();
        assert!(
            e.account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true})
            )
            .is_err()
        );
    }
    #[test]
    fn account_device_commands_are_independent_and_empty_is_explicit() {
        let mut s = Settings {
            version: 3,
            hosts: vec![Host {
                id: "remote".into(),
                pre_command: "legacy host".into(),
                ..Default::default()
            }],
            sources: vec![Source {
                id: "s".into(),
                account_id: "a".into(),
                host_id: Some("remote".into()),
                quota_pre_command: "legacy source".into(),
                ..Default::default()
            }],
            accounts: vec![Account {
                id: "a".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        s.migrate();
        assert_eq!(
            s.accounts[0].device_settings[0].pre_command,
            "legacy source"
        );
        s.accounts[0].device_settings[0].pre_command.clear();
        assert_eq!(pre_command(&s, Some(&s.accounts[0]), &s.sources[0]), "");
        let other = Account {
            device_settings: vec![AccountDeviceSettings {
                machine_id: "remote".into(),
                pre_command: "other".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(pre_command(&s, Some(&other), &s.sources[0]), "other");
        let host = Host {
            target: "fixture.invalid".into(),
            pre_command: "NEVER-SYNC".into(),
            ..Default::default()
        };
        let command = crate::ssh::command(&host, "history").unwrap();
        assert!(
            !command
                .get_args()
                .last()
                .unwrap()
                .to_string_lossy()
                .contains("NEVER-SYNC")
        );
        let command = crate::ssh::account_command(&host, "quota").unwrap();
        assert!(
            command
                .get_args()
                .last()
                .unwrap()
                .to_string_lossy()
                .contains("NEVER-SYNC")
        );
    }
    #[test]
    fn conflicting_legacy_directories_clear_commands_and_request_one_device_setting() {
        let mut s = Settings {
            version: 3,
            accounts: vec![Account {
                id: "a".into(),
                ..Default::default()
            }],
            sources: vec![
                Source {
                    id: "s1".into(),
                    account_id: "a".into(),
                    host_id: Some("server".into()),
                    quota_pre_command: "first".into(),
                    ..Default::default()
                },
                Source {
                    id: "s2".into(),
                    account_id: "a".into(),
                    host_id: Some("server".into()),
                    quota_pre_command: "second".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.migrate();
        assert_eq!(s.accounts[0].device_settings.len(), 1);
        let device = &s.accounts[0].device_settings[0];
        assert_eq!(device.machine_id, "server");
        assert!(device.pre_command_reset_required && device.pre_command.is_empty());
        for source in &s.sources {
            assert!(source.quota_pre_command.is_empty());
            assert_eq!(pre_command(&s, Some(&s.accounts[0]), source), "");
        }
        let migrated = json!(s);
        s.migrate();
        assert_eq!(json!(s), migrated);
    }
    #[test]
    fn legacy_remote_commands_migrate_for_current_settings_and_new_directories_inherit() {
        let mut settings = Settings {
            version: 9,
            hosts: vec![Host {
                id: "remote".into(),
                pre_command: "host initialization".into(),
                ..Default::default()
            }],
            accounts: vec![Account {
                id: "a".into(),
                ..Default::default()
            }],
            sources: vec![Source {
                id: "s".into(),
                account_id: "a".into(),
                host_id: Some("remote".into()),
                quota_pre_command: "account initialization".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        settings.migrate();
        assert_eq!(
            settings.accounts[0].device_settings[0].pre_command,
            "account initialization"
        );
        assert!(!settings.accounts[0].device_settings[0].pre_command_reset_required);
        let next = Source {
            id: "next".into(),
            account_id: "a".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        assert_eq!(
            pre_command(&settings, Some(&settings.accounts[0]), &next),
            "account initialization"
        );
        settings.sources.push(next);
        settings.migrate();
        assert_eq!(settings.accounts[0].device_settings.len(), 1);
    }
    #[test]
    fn qoder_desktop_history_does_not_probe_a_cli_login() {
        let (dir, engine) = fixture();
        std::fs::write(dir.path().join("main.sqlite"), []).unwrap();
        for provider in ["qoder", "qoder-cn"] {
            let row = engine
                .inspect_account_device(
                    &Settings::default(),
                    &Account {
                        provider: provider.into(),
                        ..Default::default()
                    },
                    &Source {
                        provider: provider.into(),
                        path: dir.path().to_string_lossy().into_owned(),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(row["current"].is_null());
            assert!(row["credential"].is_null());
            assert!(row["identity"].is_null());
        }
    }
    #[test]
    fn batch_projects_one_context_keeps_identity_and_only_rechecks_due_or_changed_devices() {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (dir, mut e) = fixture();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let mut settings = e.store.settings().unwrap();
        settings.accounts=(0..10).map(|i| {
            let token=URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_user_id":format!("user{i}"),"chatgpt_account_id":"workspace"}}).to_string());
            let auth=json!({"auth_mode":"chatgpt","tokens":{"access_token":format!("head.{token}.tail"),"refresh_token":"FIXTURE","account_id":"workspace"}});
            crate::codex_auth::files::save_profile(&canonical,&format!("p{i}"),"Fixture",&auth).unwrap();
            if i==0 { crate::codex_auth::files::atomic(&canonical.join("auth.json"),&auth).unwrap(); }
            Account{id:format!("a{i}"),name:format!("Account {i}"),identity_key:Some(crate::codex_auth::files::identity(&auth).unwrap().key),quota_profile_id:Some(format!("home0:p{i}")),quota_refresh_seconds:Some(if i==0 {900}else{60}),..Default::default()}
        }).collect();
        settings.sources = (0..30)
            .map(|i| Source {
                id: format!("s{i}"),
                path: canonical.to_string_lossy().into(),
                codex_home_id: Some(format!("home{i}")),
                ..Default::default()
            })
            .collect();
        e.store.save_auth_settings(&settings).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        e.status_observation_reads = Some(reads.clone());
        // The batch projects these explicit accounts/directories. Automatic native
        // discovery may add references, which have their own observation tests.
        let selection = |due_only| {
            json!({
                "accountKeys": (0..10).map(|i| format!("codex:a{i}")).collect::<Vec<_>>(),
                "sourceIds": (0..30).map(|i| format!("s{i}")).collect::<Vec<_>>(),
                "dueOnly": due_only,
            })
        };
        let rows = e.refresh_account_status_batch(&selection(false)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 300);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 1);
        assert_eq!(
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|r| r["current"] == true)
                .count(),
            30
        );
        assert!(
            rows.as_array()
                .unwrap()
                .iter()
                .all(|r| r["credential"] == true)
        );
        assert!(!rows.to_string().contains("FIXTURE"));
        assert_eq!(
            e.refresh_account_status_batch(&selection(true)).unwrap(),
            json!([])
        );
        settings.menu_metric = "cost".into();
        e.store.save_settings(&settings).unwrap();
        assert_eq!(
            e.refresh_account_status_batch(&selection(true)).unwrap(),
            json!([])
        );
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        e.store.db.execute("UPDATE kv SET value=json_set(value,'$.attemptedAt',?1) WHERE key LIKE 'account-status:%'",[now()-500]).unwrap();
        let due = e.refresh_account_status_batch(&selection(true)).unwrap();
        assert_eq!(due.as_array().unwrap().len(), 270);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 1);
        settings.sources[1].quota_pre_command = "export CHANGED_CONTEXT=1".into();
        e.store.save_settings(&settings).unwrap();
        let changed = e.refresh_account_status_batch(&selection(true)).unwrap();
        assert_eq!(changed.as_array().unwrap().len(), 10);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 1);
        let a = &settings.accounts[0];
        let one = e
            .status_observation_key(&settings, a, &settings.sources[0])
            .unwrap();
        let mut alternate = a.clone();
        alternate.device_settings.push(AccountDeviceSettings {
            machine_id: "local".into(),
            pre_command: "export DIFFERENT_CONTEXT=1".into(),
            ..Default::default()
        });
        assert_ne!(
            one,
            e.status_observation_key(&settings, &alternate, &settings.sources[0])
                .unwrap(),
            "Different precommands must never share an observation"
        );
    }
    #[test]
    fn lightweight_status_distinguishes_current_from_stored_and_invalidates_old_config() {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let (dir, mut e) = fixture();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let auth = |user: &str| {
            let token=URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_user_id":user,"chatgpt_account_id":"workspace"}}).to_string());
            json!({"auth_mode":"chatgpt","tokens":{"access_token":format!("head.{token}.tail"),"refresh_token":"FIXTURE","account_id":"workspace"}})
        };
        let one = auth("one");
        let two = auth("two");
        crate::codex_auth::files::save_profile(&canonical, "one", "One", &one).unwrap();
        crate::codex_auth::files::save_profile(&canonical, "two", "Two", &two).unwrap();
        crate::codex_auth::files::atomic(&canonical.join("auth.json"), &one).unwrap();
        let mut settings = e.store.settings().unwrap();
        settings.sources[0].path = canonical.to_string_lossy().into();
        settings.sources[0].codex_home_id = Some("home".into());
        settings.sources[0].account_id.clear();
        settings.sources[0].codex_binary = "missing-fixture-cli".into();
        settings.accounts[0].identity_key =
            Some(crate::codex_auth::files::identity(&two).unwrap().key);
        settings.accounts[0].quota_profile_id = Some("home:two".into());
        e.store.save_auth_settings(&settings).unwrap();
        let result = e
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"codex:a","sourceId":"s"}),
            )
            .unwrap();
        assert_eq!(result["current"], false);
        assert_eq!(result["credential"], true);
        assert!(!result.to_string().contains("FIXTURE"));
        let mut settings = e.store.settings().unwrap();
        settings.accounts[0]
            .device_settings
            .push(AccountDeviceSettings {
                machine_id: "local".into(),
                pre_command: "changed".into(),
                ..Default::default()
            });
        e.store.save_settings(&settings).unwrap();
        let cached = e.account_call("accounts.status.get", json!({})).unwrap();
        assert!(cached[0]["current"].is_null());
    }
}
