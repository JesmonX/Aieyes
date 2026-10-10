pub mod accounts;
pub mod agy_auth;
mod agy_identity;
pub mod antigravity;
pub mod capacity;
pub mod codex_auth;
pub mod credentials;
pub mod estimates;
pub mod file_lock;
mod host_sampling;
pub mod import;
pub mod metrics;
pub mod models;
pub mod monitoring;
pub mod network;
pub mod pricing;
pub mod process;
pub mod providers;
pub mod quota;
mod quota_schedule;
mod remote_history;
pub mod sessions;
pub mod settings;
pub mod ssh;
pub mod store;
pub mod update_transport;
pub mod usage;
mod usage_cache;
pub mod wakeups;

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
        })
    }
    pub fn call_with_progress(
        &mut self,
        method: &str,
        params: Value,
        progress: Box<dyn FnMut(Value) + Send>,
    ) -> Result<Value> {
        self.progress = Some(progress);
        let result = self.call(method, params);
        self.progress = None;
        result
    }
    pub(crate) fn progress(&mut self, stage: &str) {
        if let Some(callback) = &mut self.progress {
            callback(json!({"stage":stage}));
        }
    }
    pub(crate) fn read_quotas(&self, params: &Value) -> Result<Vec<QuotaSnapshot>> {
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
                    s.enabled
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
        if method != "hello" && method != "settings.get" {
            let aliases = self.store.settings()?.account_aliases;
            if method == "settings.save" {
                params["accountAliases"] = serde_json::to_value(&aliases)?;
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
            "accounts.create"
            | "accounts.status.get"
            | "accounts.status.refresh"
            | "accounts.deletion.preview"
            | "accounts.delete"
            | "accounts.cleanup.list"
            | "accounts.deployments.sync" => self.account_call(method, params),
            name if name.starts_with("agyAuth.") => self.agy_auth_call(name, params),
            name if name.starts_with("codexAuth.") => self.codex_auth_call(name, params),
            name if name.starts_with("wakeups.") => self.wakeup_call(name, params),
            name if name.starts_with("quotaEstimates.") || name.starts_with("creditEstimates.") => {
                self.estimate_call(name, params)
            }
            "quotas.order.set" => Ok(serde_json::to_value(
                self.store
                    .set_quota_order(serde_json::from_value(params["keys"].clone())?)?,
            )?),
            "sessions.list" => Ok(serde_json::to_value(self.sessions.read(
                &self.store.settings()?.sources,
                std::time::SystemTime::now(),
            ))?),
            "hello" => {
                Ok(json!({"name":"Aieyes","version":env!("CARGO_PKG_VERSION"),"protocolVersion":1}))
            }
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
            "sources.configure" => self.configure_source(params, false),
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
                Ok(serde_json::to_value(if method == "dashboard.summary" {
                    self.store.dashboard_summary(&filter)?
                } else {
                    self.store.dashboard(&filter)?
                })?)
            }
            "sources.scan" => {
                let settings = self.store.settings()?;
                let mut results = Vec::new();
                let explicit = params["sourceId"].is_string() || params["sourceIds"].is_array();
                let selected: Vec<_> = settings
                    .sources
                    .iter()
                    .filter(|s| {
                        s.enabled
                            && s.provider != "deepseek"
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
                        scope.spawn(move || {
                            loop {
                                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(source) = remote.get(index) else {
                                    break;
                                };
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        remote_history::scan(root, settings, source)
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
                for source in selected {
                    let result = if source.host_id.is_some() {
                        fetched.remove(&source.id).context("远程读取结果缺失")?
                    } else {
                        import::scan(&self.store, &settings, source)
                    };
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
                            let v = json!({"id":source.id,"error":e.to_string(),"failedAt":now()});
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
            "updates.open" => {
                network::open_release_page()?;
                Ok(json!({"opened":true}))
            }
            "updates.check" => {
                let s = self.store.settings()?;
                let repository = "JesmonX/Aieyes";
                let response = network::client(&s.proxy)?
                    .get(format!(
                        "https://api.github.com/repos/{}/releases/latest",
                        repository
                    ))
                    .send()
                    .map_err(|_| anyhow::anyhow!("更新连接失败"))?;
                anyhow::ensure!(
                    response.status().is_success(),
                    "更新查询失败（HTTP {}）",
                    response.status().as_u16()
                );
                let v: Value = response.json()?;
                Ok(
                    json!({"version":v["tag_name"],"url":v["html_url"],"publishedAt":v["published_at"]}),
                )
            }
            _ => anyhow::bail!("未知方法：{method}"),
        }
    }
}
