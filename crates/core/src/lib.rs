pub mod import;
pub mod metrics;
pub mod models;
pub mod network;
pub mod pricing;
pub mod process;
pub mod quota;
pub mod sessions;
pub mod ssh;
pub mod store;
pub mod usage;

use anyhow::{Context, Result};
use models::*;
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

pub struct Engine {
    pub store: store::Store,
    previous_metrics: HashMap<String, Value>,
    sessions: sessions::SessionMonitor,
}
impl Engine {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            store: store::Store::open(root)?,
            previous_metrics: HashMap::new(),
            sessions: sessions::SessionMonitor::default(),
        })
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "sessions.list" => Ok(serde_json::to_value(self.sessions.read(
                &self.store.settings()?.sources,
                std::time::SystemTime::now(),
            ))?),
            "hello" => {
                Ok(json!({"name":"Aieyes","version":env!("CARGO_PKG_VERSION"),"protocolVersion":1}))
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
                let host: Host = serde_json::from_value(params["host"].clone())?;
                ssh::python(
                    &host,
                    ssh::METRICS_SCRIPT,
                    &[serde_json::to_string(&Host::default().metrics)?],
                )
            }
            "settings.get" => Ok(serde_json::to_value(self.store.settings()?)?),
            "settings.save" => {
                let s: Settings = serde_json::from_value(params)?;
                self.store.save_settings(&s)?;
                self.store.reprice(&self.store.settings()?, true)?;
                Ok(json!({"saved":true}))
            }
            "dashboard" => Ok(serde_json::to_value(
                self.store
                    .dashboard(&serde_json::from_value::<Filter>(params)?)?,
            )?),
            "sources.scan" => {
                let settings = self.store.settings()?;
                let mut results = Vec::new();
                for source in settings.sources.iter().filter(|s| {
                    s.enabled
                        && !["agy", "deepseek"].contains(&s.provider.as_str())
                        && params["sourceId"].as_str().is_none_or(|id| s.id == id)
                }) {
                    let result = if let Some(id) = &source.host_id {
                        (|| -> Result<Value> {
                            let host = settings
                                .hosts
                                .iter()
                                .find(|h| &h.id == id)
                                .context("主机不存在")?;
                            let data = ssh::python(
                                host,
                                ssh::HISTORY_SCRIPT,
                                &[source.path.clone(), source.provider.clone()],
                            )?;
                            let prices = self.store.prices()?;
                            let tx = self.store.db.unchecked_transaction()?;
                            let mut count = 0;
                            let files = data["files"].as_array().context("远程记录格式不正确")?;
                            for file in files {
                                let mut state = ParseState {
                                    session_id: format!(
                                        "{}:{}",
                                        source.id,
                                        file["path"].as_str().unwrap_or("remote")
                                    ),
                                    ..Default::default()
                                };
                                for v in file["events"].as_array().context("远程事件格式不正确")?
                                {
                                    if let Some(e) = usage::parse(source, &mut state, v)
                                        && self.store.put_event(&e, &prices, &settings)?
                                    {
                                        count += 1;
                                    }
                                    if source.provider == "codex"
                                        && settings.quota_enabled(source)
                                        && v["payload"]["rate_limits"].is_object()
                                        && let Some(t) = timestamp(&v["timestamp"])
                                    {
                                        self.store.quota(&usage::codex_quota(
                                            source,
                                            &v["payload"]["rate_limits"],
                                            t,
                                            "log",
                                        ))?;
                                    }
                                }
                            }
                            tx.commit()?;
                            let status =
                                json!({"updatedAt":now(),"newEvents":count,"files":files.len()});
                            self.store.source_status(&source.id, &status)?;
                            Ok(status)
                        })()
                    } else {
                        import::scan(&self.store, &settings, source)
                    };
                    match result {
                        Ok(value) => results.push(json!({"id":source.id,"result":value})),
                        Err(e) => {
                            let v = json!({"id":source.id,"error":e.to_string(),"failedAt":now()});
                            self.store.source_status(&source.id, &v)?;
                            results.push(v);
                        }
                    }
                }
                Ok(json!(results))
            }
            "quotas.refresh" => {
                let settings = self.store.settings()?;
                let mut result = Vec::new();
                for account in settings.accounts.iter().filter(|a| {
                    a.quota_enabled && params["accountId"].as_str().is_none_or(|id| a.id == id)
                }) {
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
                        match quota::read(source, &settings) {
                            Ok(mut q) => {
                                q.name = account.name.clone();
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
                        self.store.quota(&q)?;
                        result.push(q);
                    }
                }
                Ok(serde_json::to_value(result)?)
            }
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
            "hosts.sample" => {
                let settings = self.store.settings()?;
                let mut results = Vec::new();
                let selected: Vec<_> = settings
                    .hosts
                    .iter()
                    .filter(|h| h.enabled && params["hostId"].as_str().is_none_or(|id| h.id == id))
                    .collect();
                let samples = std::thread::scope(|scope| {
                    let tasks: Vec<_> = selected
                        .iter()
                        .map(|host| {
                            let warm = !self.previous_metrics.contains_key(&host.id);
                            scope.spawn(move || {
                                ssh::python(
                                    host,
                                    ssh::METRICS_SCRIPT,
                                    &[
                                        serde_json::to_string(&host.metrics).unwrap(),
                                        warm.to_string(),
                                    ],
                                )
                            })
                        })
                        .collect();
                    tasks
                        .into_iter()
                        .map(|t| {
                            t.join()
                                .unwrap_or_else(|_| Err(anyhow::anyhow!("采样任务失败")))
                        })
                        .collect::<Vec<_>>()
                });
                for (host, sample) in selected.into_iter().zip(samples) {
                    match sample {
                        Ok(raw) => {
                            let mut display = metrics::rates(
                                &raw,
                                self.previous_metrics
                                    .get(&host.id)
                                    .or_else(|| raw.get("previous")),
                            );
                            self.previous_metrics.insert(host.id.clone(), raw);
                            if !host.devices.is_empty() {
                                for group in ["cpu", "gpu", "filesystems", "disk", "network"] {
                                    if host
                                        .devices
                                        .iter()
                                        .any(|d| d.starts_with(&format!("{group}:")))
                                        && let Some(rows) = display[group].as_array_mut()
                                    {
                                        rows.retain(|r| {
                                            r["id"] == "cpu"
                                                || host.devices.iter().any(|d| {
                                                    d == &format!(
                                                        "{}:{}",
                                                        group,
                                                        r["id"].as_str().unwrap_or("")
                                                    )
                                                })
                                        });
                                    }
                                }
                            }
                            results.push(json!({"id":host.id,"name":host.name,"sample":display}));
                        }
                        Err(e) => {
                            self.previous_metrics.remove(&host.id);
                            results
                                .push(json!({"id":host.id,"name":host.name,"error":e.to_string()}));
                        }
                    }
                }
                Ok(json!(results))
            }
            "updates.check" => {
                let s = self.store.settings()?;
                let parts: Vec<_> = s.github_repository.split('/').collect();
                anyhow::ensure!(
                    parts.len() == 2
                        && parts.iter().all(|p| !p.is_empty()
                            && p.chars()
                                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))),
                    "填写 GitHub 仓库 owner/repo"
                );
                let response = network::client(&s.proxy)?
                    .get(format!(
                        "https://api.github.com/repos/{}/releases/latest",
                        s.github_repository
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
