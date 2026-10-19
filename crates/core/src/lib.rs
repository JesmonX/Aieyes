mod account_targets;
pub mod accounts;
pub mod agent_api;
pub mod agy_auth;
mod agy_identity;
pub mod antigravity;
pub mod automatic;
pub mod capacity;
pub mod codex_api;
pub mod codex_auth;
pub mod codex_history;
mod codex_routes;
pub mod contracts;
pub mod credentials;
pub mod estimates;
pub mod file_lock;
mod harness_history;
mod history_corrections;
mod host_sampling;
pub mod import;
pub mod metrics;
pub mod models;
pub mod monitoring;
pub mod native_agents;
mod native_credits;
mod native_db;
mod native_sessions;
pub mod network;
pub mod opencode;
pub mod operations;
pub mod pet_import;
pub mod pets;
mod pi_history;
pub mod pricing;
pub mod process;
mod provider_paths;
pub mod providers;
pub mod quota;
pub mod quota_notifications;
mod quota_queries;
pub mod quota_schedule;
mod remote_history;
pub mod runtime;
pub mod session_events;
pub mod session_hooks;
pub mod sessions;
pub mod settings;
pub mod ssh;
mod ssh_config;
pub mod store;
pub mod subscriptions;
pub mod update_transport;
pub mod usage;
mod usage_cache;
pub mod wakeups;
pub mod zcode;

use anyhow::{Context, Result};
use models::*;
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

pub struct Engine {
    pub store: store::Store,
    previous_metrics: HashMap<String, Value>,
    host_generation: u64,
    host_session: String,
    local_metrics: monitoring::LocalSampler,
    sessions: sessions::SessionMonitor,
    progress: Option<Box<dyn FnMut(Value) + Send>>,
    network_cache: HashMap<String, network::NetworkTest>,
    status_observations: Option<std::cell::RefCell<HashMap<String, Value>>>,
    runtime_change: Option<Value>,
    #[cfg(test)]
    status_observation_reads: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}
impl Engine {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            store: store::Store::open(root)?,
            previous_metrics: HashMap::new(),
            host_generation: 0,
            host_session: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            local_metrics: monitoring::LocalSampler::default(),
            sessions: sessions::SessionMonitor::default(),
            progress: None,
            network_cache: HashMap::new(),
            status_observations: None,
            runtime_change: None,
            #[cfg(test)]
            status_observation_reads: None,
        })
    }
    pub fn call_with_progress(
        &mut self,
        method: &str,
        params: Value,
        progress: Box<dyn FnMut(Value) + Send>,
    ) -> Result<Value> {
        self.progress = Some(progress);
        self.runtime_change = None;
        let changes = self.store.db.total_changes();
        let operation = params["operationId"].clone();
        let result = self.call(method, params);
        if (self.store.db.total_changes() != changes
            || contracts::method(method).is_some_and(|m| !m.invalidates.is_empty())
            || method == "accounts.status.refreshBatch")
            && let Ok(revisions) = runtime::revisions(&self.store.db)
        {
            self.runtime_change = Some(json!({"revisions":revisions,"operationId":operation}));
            runtime::signal(&self.store.root).wake();
        }
        self.progress = None;
        result
    }
    pub fn take_runtime_change(&mut self) -> Option<Value> {
        self.runtime_change.take()
    }
    pub(crate) fn progress(&mut self, stage: &str) {
        if let Some(callback) = &mut self.progress {
            callback(json!({"stage":stage}));
        }
    }
    pub(crate) fn read_quotas_unlocked(&self, params: &Value) -> Result<Vec<QuotaSnapshot>> {
        let settings = self.store.settings()?;
        let mut result = Vec::new();
        for account in settings.accounts.iter().filter(|a| {
            a.quota_enabled
                && !a.archived
                && params["accountId"].as_str().is_none_or(|id| a.id == id)
                && params["accountKey"]
                    .as_str()
                    .is_none_or(|key| format!("{}:{}", a.provider, a.id) == key)
        }) {
            if !account.profile_refs().is_empty() {
                let snapshot =
                    codex_auth::read_quota_at(account, &settings, params["sourceId"].as_str())
                        .unwrap_or_else(|e| QuotaSnapshot {
                            account_id: account.id.clone(),
                            provider: account.provider.clone(),
                            name: account.name.clone(),
                            error: Some(e.to_string()),
                            ..Default::default()
                        });
                result.push(snapshot);
                continue;
            }
            let mut sources: Vec<_> = settings
                .sources
                .iter()
                .filter(|s| {
                    settings.source_enabled(s)
                        && s.provider == account.provider
                        && s.account_id == account.id
                        && params["sourceId"].as_str().is_none_or(|id| s.id == id)
                        && s.host_id.as_ref().is_none_or(|id| {
                            settings.hosts.iter().any(|h| &h.id == id && h.enabled)
                        })
                })
                .collect();
            sources.sort_by_key(|s| {
                (
                    account.quota_source_id.as_ref() != Some(&s.id),
                    s.host_id.is_some(),
                )
            });
            let mut last = None;
            for source in sources {
                let auth_revision = accounts::status_revision(&self.store, source)?;
                match quota::read(source, &settings) {
                    Ok(mut q) => {
                        q.name = account.name.clone();
                        if account.provider == "antigravity" {
                            accounts::record_agy_observation(
                                &self.store,
                                &settings,
                                source,
                                &q,
                                &auth_revision,
                            )?;
                            if let Some(a) = self
                                .store
                                .settings()?
                                .accounts
                                .iter()
                                .find(|a| a.provider == account.provider && a.id == account.id)
                            {
                                q.name = a.name.clone();
                            }
                        }
                        last = Some(q);
                        break;
                    }
                    Err(e) => {
                        last = Some(QuotaSnapshot {
                            source_id: source.id.clone(),
                            account_id: account.id.clone(),
                            provider: account.provider.clone(),
                            name: account.name.clone(),
                            error: Some(e.to_string()),
                            ..Default::default()
                        })
                    }
                }
            }
            if let Some(q) = last {
                result.push(q);
            }
        }
        Ok(result)
    }
    pub fn call(&mut self, method: &str, mut params: Value) -> Result<Value> {
        match method {
            "operations.prepareUpdate" => return Ok(operations::prepare()),
            "operations.resume" => return Ok(operations::resume()),
            "operations.status" => return Ok(operations::status()),
            "runtime.revisions" => return runtime::revisions(&self.store.db),
            "quotaResetNotifications.take" => {
                return Ok(serde_json::to_value(self.store.take_reset_notifications(
                    params["since"].as_i64().context("缺少通知会话起始时间")?,
                )?)?);
            }
            _ => {}
        }
        let _operation = operations::enter(method)?;
        if method != "hello" && method != "settings.get" {
            let aliases = self.store.settings()?.account_aliases;
            if method == "settings.save" {
                params["accountAliases"] = serde_json::to_value(&aliases)?;
                params["codexApis"] = serde_json::to_value(self.store.settings()?.codex_apis)?;
            }
            providers::normalize_value(&mut params, &aliases);
            if method == "quotas.order.set"
                && let Some(keys) = params["keys"].as_array_mut()
            {
                for key in keys {
                    if let Some(value) = key.as_str() {
                        *key = json!(providers::account_key(value, &aliases));
                    }
                }
            }
        }
        match method {
            "methods.manifest" => Ok(serde_json::to_value(contracts::METHODS)?),
            "quotaResetNotifications.take" => {
                Ok(serde_json::to_value(self.store.take_reset_notifications(
                    params["since"].as_i64().context("缺少通知会话起始时间")?,
                )?)?)
            }
            "autoEstimates.configure" => self.configure_automatic(params),
            "autoEstimates.tick" => self.tick_automatic(),
            "accounts.targets" => self.account_targets(&params),
            "accounts.status.refreshBatch" => self.refresh_account_status_batch(&params),
            "subscriptions.openLogin" => subscriptions::open_login(&params),
            "subscriptions.connect" => self.connect_subscription(params),
            "accounts.create"
            | "accounts.status.get"
            | "accounts.status.refresh"
            | "accounts.deletion.preview"
            | "accounts.delete"
            | "accounts.cleanup.list"
            | "accounts.deployments.sync" => self.account_call(method, params),
            name if name.starts_with("agyAuth.") => self.agy_auth_call(name, params),
            name if name.starts_with("codexApi.") => self.codex_api_call(name, params),
            name if name.starts_with("agentApi.") => self.agent_api_call(name, params),
            name if name.starts_with("codexAuth.") => self.codex_auth_call(name, params),
            name if name.starts_with("wakeups.") => self.wakeup_call(name, params),
            name if name.starts_with("quotaEstimates.") || name.starts_with("creditEstimates.") => {
                self.estimate_call(name, params)
            }
            "quotas.order.set" => Ok(serde_json::to_value(
                self.store
                    .set_quota_order(serde_json::from_value(params["keys"].clone())?)?,
            )?),
            "sessions.list" => {
                let snapshot = self
                    .sessions
                    .read(&self.store.settings()?, std::time::SystemTime::now());
                let mut value = serde_json::to_value(&snapshot)?;
                value["pet"] = serde_json::to_value(pets::project(&snapshot))?;
                value["summary"] = json!(snapshot.summary());
                value["phase"] = serde_json::to_value(snapshot.phase())?;
                value["activeCount"] = json!(snapshot.active_count());
                Ok(value)
            }
            "pets.current" => pet_import::current(
                &self.store,
                params["includeImage"].as_bool().unwrap_or(false),
            ),
            "pets.list" => pet_import::list(&self.store),
            "pets.select" => pet_import::select(
                &self.store,
                params["libraryId"].as_str().context("请选择库中的桌宠")?,
            ),
            "pets.asset" => pet_import::asset(
                &self.store,
                params["libraryId"].as_str().context("请选择库中的桌宠")?,
            ),
            "sessions.hooks.status" | "sessions.hooks.repair" | "sessions.hooks.remove" => {
                session_hooks::call(&self.store.settings()?, method, &params)
            }
            "pets.import" => {
                let input = params
                    .get("input")
                    .or_else(|| params.get("path"))
                    .context("请输入桌宠链接、npx 命令、npm 包名或选择文件")?
                    .as_str()
                    .context("桌宠导入内容必须是文本")?;
                pet_import::import_input(&self.store, input)?;
                pet_import::current(&self.store, false)
            }
            "hello" => Ok(
                json!({"name":"Aieyes","version":env!("CARGO_PKG_VERSION"),"protocolVersion":1,"runtimeInvalidations":true,"contractHash":contracts::CONTRACT_HASH}),
            ),
            "hosts.credentials.save" => Ok(
                json!({"passwordRef": credentials::save(params["password"].as_str().context("请输入服务器密码")?)?}),
            ),
            "hosts.credentials.delete" => {
                credentials::delete(params["passwordRef"].as_str().context("密码引用缺失")?)?;
                Ok(json!({"deleted":true}))
            }
            "network.test" | "network.status" => {
                let settings = self.store.settings()?;
                let proxy: ProxyConfig = params
                    .get("proxy")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or(settings.proxy);
                let urls: Vec<String> = params
                    .get("urls")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or(settings.proxy_test_urls);
                let key =
                    serde_json::to_string(&(&proxy.mode, network::effective_proxy(&proxy), &urls))?;
                if method == "network.status" {
                    return Ok(serde_json::to_value(self.network_cache.get(&key))?);
                }
                if params["force"] != true
                    && let Some(result) = self
                        .network_cache
                        .get(&key)
                        .filter(|r| now() - r.tested_at < 300)
                {
                    return Ok(serde_json::to_value(result)?);
                }
                let result = network::test(&proxy, &urls)?;
                if params.get("proxy").is_none() && params.get("urls").is_none() {
                    let current = self.store.settings()?;
                    let current_key = serde_json::to_string(&(
                        &current.proxy.mode,
                        network::effective_proxy(&current.proxy),
                        &current.proxy_test_urls,
                    ))?;
                    anyhow::ensure!(current_key == key, "连接设置已变化，请重新测试");
                }
                if self.network_cache.len() > 20 {
                    self.network_cache.clear();
                }
                self.network_cache.insert(key, result.clone());
                Ok(serde_json::to_value(result)?)
            }
            "credentials.save" => {
                use std::io::Write;
                let id = params["sourceId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("数据源 ID 缺失")?;
                let key = params["apiKey"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.contains(['\n', '\r']))
                    .context("请输入有效的 API Key")?;
                let root = Path::new(self.store.db.path().context("凭据目录不可用")?)
                    .parent()
                    .context("凭据目录不可用")?
                    .join("credentials");
                std::fs::create_dir_all(&root)?;
                anyhow::ensure!(
                    !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
                    "凭据目录不可用"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
                }
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos();
                let path = root.join(format!("{}.key", hash(&format!("{id}:{stamp}"))));
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                options.open(&path)?.write_all(key.as_bytes())?;
                Ok(json!({"path":path}))
            }
            "hosts.sshConfig.read" => Ok(serde_json::to_value(ssh_config::read())?),
            "hosts.discover" => {
                if params["hostId"] == "local" || params["host"]["id"] == "local" {
                    return Ok(self.local_metrics.sample(&Host::default().metrics));
                }
                let host: Host = serde_json::from_value(params["host"].clone())?;
                ssh::python(
                    &host,
                    ssh::METRICS_SCRIPT,
                    &[serde_json::to_string(&Host::default().metrics)?],
                )
            }
            "settings.patch" => Ok(serde_json::to_value(self.store.patch_settings(
                &params["base"],
                &params["settings"],
                false,
            )?)?),
            "agents.set" => self.configure_agent(params),
            "sources.defaultPath" => {
                let provider = params["provider"].as_str().context("请选择 Agent")?;
                let remote = params["hostId"].as_str().is_some_and(|id| !id.is_empty());
                Ok(json!({"path":provider_paths::default_path(provider,remote)?}))
            }
            "sources.configure" => self.configure_source(params, false),
            "sources.rebind" => self.rebind_source(params),
            "sources.remove" => self.configure_source(params, true),
            "accounts.connect" => self.connect_account(params),
            "settings.get" => Ok(serde_json::to_value(self.store.settings()?)?),
            "settings.save" => {
                self.progress("保存配置");
                let s: Settings = serde_json::from_value(params)?;
                self.store.save_settings(&s)?;
                Ok(json!({"saved":true}))
            }
            "dashboard" | "dashboard.summary" => {
                let filter = serde_json::from_value::<Filter>(params)?;
                Ok((if method == "dashboard.summary" {
                    self.store.dashboard_summary(&filter)?
                } else {
                    self.store.dashboard(&filter)?
                })
                .wire_value()?)
            }
            "sources.scan" | "sources.rescan" => {
                // Only this explicit RPC bypasses cursors; automatic collection
                // continues to use sources.scan.
                let full = method == "sources.rescan";
                if full {
                    self.progress("全量重扫");
                }
                let settings = self.store.settings()?;
                let mut results = Vec::new();
                let explicit = params["sourceId"].is_string() || params["sourceIds"].is_array();
                let selected: Vec<_> = settings
                    .sources
                    .iter()
                    .filter(|s| {
                        settings.source_enabled(s)
                            && s.provider != "deepseek"
                            && s.subscription.is_none()
                            && params["sourceId"].as_str().is_none_or(|id| s.id == id)
                            && params["sourceIds"]
                                .as_array()
                                .is_none_or(|ids| ids.iter().any(|id| id.as_str() == Some(&s.id)))
                            && (explicit
                                || s.host_id.as_ref().is_none_or(|id| {
                                    settings
                                        .hosts
                                        .iter()
                                        .find(|h| &h.id == id)
                                        .is_none_or(|h| h.enabled)
                                }))
                    })
                    .collect();
                let remote: Vec<_> = selected
                    .iter()
                    .copied()
                    .filter(|s| s.host_id.is_some())
                    .collect();
                let root = std::path::Path::new(self.store.db.path().context("数据目录不可用")?)
                    .parent()
                    .context("数据目录不可用")?;
                let mut fetched = HashMap::new();
                let next = std::sync::atomic::AtomicUsize::new(0);
                std::thread::scope(|scope| {
                    let (tx, rx) = std::sync::mpsc::sync_channel(4);
                    for _ in 0..remote.len().min(2) {
                        let tx = tx.clone();
                        let remote = &remote;
                        let settings = &settings;
                        let next = &next;
                        let operation = operations::context();
                        scope.spawn(move || {
                            let _context = operations::inherit(operation);
                            loop {
                                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(source) = remote.get(index) else {
                                    break;
                                };
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        if full {
                                            remote_history::rescan(root, settings, source)
                                        } else {
                                            remote_history::scan(root, settings, source)
                                        }
                                    }))
                                    .unwrap_or_else(|_| Err(anyhow::anyhow!("远程同步任务失败")));
                                if tx.send((source.id.clone(), result)).is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    drop(tx);
                    for (id, result) in rx {
                        fetched.insert(id, result);
                    }
                });
                operations::check()?;
                for source in selected {
                    operations::check()?;
                    let result = if source.host_id.is_some() {
                        fetched.remove(&source.id).context("远程读取结果缺失")?
                    } else {
                        codex_routes::sync(&self.store, source).and_then(|_| {
                            if full {
                                self.progress(&format!("全量重扫 · {}", source.name));
                                import::rescan(&self.store, &settings, source)
                            } else {
                                import::scan(&self.store, &settings, source)
                            }
                        })
                    };
                    let result = result.and_then(|value| {
                        codex_api::usage::sync(&self.store, &settings, source)?;
                        Ok(value)
                    });
                    match result {
                        Ok(value) => {
                            if value["error"].is_string()
                                || value["failedFiles"].as_u64().unwrap_or(0) > 0
                                || value["invalidRecords"].as_u64().unwrap_or(0) > 0
                                || value["issues"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|i| i["code"] == "invalidRecord")
                            {
                                self.store.pause_source_estimates(&source.id)?;
                            }
                            let mut row = json!({"id":source.id,"name":source.name,"partial":value["partial"],"issues":value["issues"],"result":value});
                            if value["error"].is_string() {
                                row["error"] = value["error"].clone();
                            }
                            results.push(row);
                        }
                        Err(e) => {
                            let v = json!({"id":source.id,"name":source.name,"error":e.to_string(),"failedAt":now()});
                            self.store.source_status(&source.id, &v)?;
                            self.store.pause_source_estimates(&source.id)?;
                            results.push(v);
                        }
                    }
                }
                Ok(json!(results))
            }
            "quotas.schedule" => self.quota_schedule(),
            "quotas.refresh" => self.refresh_scheduled_quotas(params),
            "prices.list" => Ok(serde_json::to_value(self.store.prices()?)?),
            "prices.sync" => {
                let settings = self.store.settings()?;
                let response = network::client(&settings.proxy)?
                    .get("https://openrouter.ai/api/v1/models")
                    .send()
                    .map_err(|_| anyhow::anyhow!("价格连接失败"))?;
                anyhow::ensure!(
                    response.status().is_success(),
                    "价格同步失败（HTTP {}）",
                    response.status().as_u16()
                );
                let prices = pricing::parse_openrouter(&response.json::<Value>()?);
                anyhow::ensure!(!prices.is_empty(), "价格列表为空");
                self.store.save_prices(&prices)?;
                let repriced = self.store.reprice(&settings, true)?;
                Ok(json!({"models":prices.len(),"repriced":repriced,"updatedAt":now()}))
            }
            "prices.save" => {
                let mut prices: Vec<ModelPrice> = serde_json::from_value(params["prices"].clone())?;
                for p in &mut prices {
                    anyhow::ensure!(!p.id.is_empty(), "请输入模型 ID");
                    anyhow::ensure!(
                        [p.input, p.output, p.cache_read, p.cache_write]
                            .into_iter()
                            .flatten()
                            .all(|v| v.is_finite() && v >= 0.0),
                        "价格需要为非负数"
                    );
                    p.fetched_at = now();
                }
                self.store.save_prices(&prices)?;
                let n = self.store.reprice(&self.store.settings()?, true)?;
                Ok(json!({"saved":prices.len(),"repriced":n}))
            }
            "prices.recalculate" => {
                Ok(json!({"repriced":self.store.reprice(&self.store.settings()?,false)?}))
            }
            "hosts.sample" => self.sample_hosts(params),
            _ => anyhow::bail!("未知方法：{method}"),
        }
    }
}
