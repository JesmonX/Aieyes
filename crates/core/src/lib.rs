pub mod account_scan;
mod account_targets;
pub mod accounts;
pub mod agent_api;
pub mod agy_auth;
mod agy_identity;
pub mod antigravity;
pub mod automatic;
mod buddy_auth;
pub mod capacity;
pub mod channels;
pub mod claude_keychain;
mod codebuddy_ide;
pub mod codex_api;
mod codex_attribution;
pub mod codex_auth;
pub mod codex_history;
mod codex_routes;
pub mod contracts;
mod copilot_auth;
pub mod credentials;
pub mod estimates;
pub mod file_lock;
mod harness_history;
mod history_corrections;
mod history_counting;
mod history_scan;
pub mod host_history;
mod host_monitor;
mod host_sampling;
pub mod import;
mod kimi_work;
pub mod localization;
pub mod metrics;
pub mod models;
pub mod monitoring;
pub mod native_agents;
mod native_credits;
mod native_db;
mod native_quota_transport;
mod native_sessions;
pub mod network;
pub mod opencode;
pub mod operations;
pub mod pet_import;
pub mod pets;
mod pi_history;
mod plan_tier;
pub mod pricing;
pub mod process;
mod provider_paths;
pub mod providers;
mod qoder_desktop;
mod qoder_env;
mod qoder_family;
mod qoder_state;
pub mod quota;
pub mod quota_notifications;
mod quota_queries;
pub mod quota_ring;
pub mod quota_schedule;
mod remote_history;
pub mod rolling_estimates;
pub mod runtime;
pub mod session_events;
pub mod session_hooks;
mod session_processes;
pub mod session_sounds;
pub mod session_usage;
pub mod sessions;
pub mod settings;
mod sqlite_ro;
pub mod ssh;
mod ssh_config;
pub mod store;
pub mod subscriptions;
mod task_history;
pub mod update_transport;
mod upstream;
pub mod usage;
mod usage_cache;
pub mod wakeups;
pub mod webhooks;
mod workbuddy;
pub mod zcode;
mod zcode_credentials;

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
    host_monitor: host_monitor::HostMonitor,
    /// Minute history and alert state fed by foreground and background samples.
    host_observer: std::sync::Arc<host_history::HostObserver>,
    /// Alerts raised on monitor threads, queued for webhooks on the next call.
    alert_webhooks: std::sync::mpsc::Receiver<host_history::AlertEvent>,
    sessions: sessions::SessionMonitor,
    /// Last phase and turn per live session, for webhook transitions.
    session_marks: HashMap<String, webhooks::SessionMark>,
    channel_service: Option<channels::Service>,
    progress: Option<Box<dyn FnMut(Value) + Send>>,
    network_cache: HashMap<String, network::NetworkTest>,
    status_observations: Option<std::cell::RefCell<HashMap<String, Value>>>,
    runtime_change: Option<Value>,
    #[cfg(test)]
    status_observation_reads: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    /// Test-only `AIEYES_AUTO_SCAN=off` without mutating the process environment.
    #[cfg(test)]
    native_discovery_off: bool,
}
impl Engine {
    pub fn open(root: &Path) -> Result<Self> {
        let store = store::Store::open(root)?;
        let (host_observer, alert_webhooks) = host_history::HostObserver::new();
        let host_monitor = host_monitor::HostMonitor::default();
        {
            let observer = host_observer.clone();
            host_monitor.set_observer(std::sync::Arc::new(move |host: &Host, sample: &Value| {
                observer.observe(&host.id, &host.name, sample);
            }));
        }
        Ok(Self {
            store,
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
            host_monitor,
            host_observer,
            alert_webhooks,
            sessions: sessions::SessionMonitor::default(),
            session_marks: HashMap::new(),
            channel_service: None,
            progress: None,
            network_cache: HashMap::new(),
            status_observations: None,
            runtime_change: None,
            #[cfg(test)]
            status_observation_reads: None,
            #[cfg(test)]
            native_discovery_off: false,
        })
    }
    /// Detect native accounts once in the background after the engine starts.
    /// Uses its own connection; scans are serialized by per-target file locks.
    /// `AIEYES_AUTO_SCAN=off` disables it so UI fixtures never read the developer's real logins.
    /// Background upkeep every front end starts once; failures retry next launch.
    pub fn spawn_store_maintenance(root: &Path) {
        if std::env::var("AIEYES_AUTO_SCAN").is_ok_and(|v| v == "off") {
            return;
        }
        let root = root.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("aieyes-store-maintenance".into())
            .spawn(move || {
                let result =
                    store::Store::open(&root).and_then(|store| store.maintain_quota_history());
                if let Err(error) = result {
                    eprintln!("Aieyes: 数据库维护失败，将在下次启动时重试：{error:#}");
                }
            });
    }
    pub fn spawn_auto_scan(root: &Path) {
        if std::env::var("AIEYES_AUTO_SCAN").is_ok_and(|v| v == "off") {
            return;
        }
        let root = root.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("aieyes-auto-scan".into())
            .spawn(move || {
                if let Ok(mut engine) = Engine::open(&root) {
                    let _ =
                        engine.call_with_progress("accounts.autoScan", json!({}), Box::new(|_| {}));
                }
            });
    }
    pub fn set_host_event_sink(&mut self, sink: impl Fn(Value) + Send + Sync + 'static) {
        self.host_monitor.set_sink(std::sync::Arc::new(sink));
    }
    pub fn call_with_progress(
        &mut self,
        method: &str,
        params: Value,
        progress: Box<dyn FnMut(Value) + Send>,
    ) -> Result<Value> {
        let progress = std::sync::Arc::new(std::sync::Mutex::new(progress));
        let _query_progress = operations::query_progress(progress.clone());
        self.progress = Some(Box::new(move |value| progress.lock().unwrap()(value)));
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
                    Err(e)
                        if matches!(source.provider.as_str(), "workbuddy" | "codebuddy")
                            && source.subscription.as_ref().is_some_and(|c| {
                                matches!(c.mode.as_str(), "buddy-cn" | "buddy-global")
                            }) =>
                    {
                        last = Some(
                            account_scan::last_snapshot(&self.store, account)
                                .filter(|q| q.source_id == source.id)
                                .map(|mut q| {
                                    q.stale_since = q.stale_since.or(Some(now()));
                                    q.notice = Some(e.to_string());
                                    q
                                })
                                .unwrap_or_else(|| QuotaSnapshot {
                                    source_id: source.id.clone(),
                                    account_id: account.id.clone(),
                                    provider: account.provider.clone(),
                                    name: account.name.clone(),
                                    error: Some(e.to_string()),
                                    ..Default::default()
                                }),
                        );
                    }
                    Err(e) => {
                        last = Some(QuotaSnapshot {
                            source_id: source.id.clone(),
                            account_id: account.id.clone(),
                            provider: account.provider.clone(),
                            name: account.name.clone(),
                            notice: claude_keychain::quota_notice(&e),
                            error: Some(e.to_string()),
                            ..Default::default()
                        })
                    }
                }
            }
            if last.as_ref().is_none_or(|q| q.error.is_some()) {
                for source in account_scan::quota_sources(&settings, account)
                    .into_iter()
                    .filter(|s| {
                        params["sourceId"].as_str().is_none_or(|id| {
                            s.id == id
                                || settings.native_connections.iter().any(|r| {
                                    s.id == format!("native-{}", r.id)
                                        && r.source_id.as_deref() == Some(id)
                                })
                        })
                    })
                {
                    match account_scan::read_native_quota(&self.store, &settings, account, &source)
                    {
                        Ok(q) => {
                            last = Some(q);
                            break;
                        }
                        Err(error) => {
                            // Unreadable or half-written native files fall back to the
                            // last good snapshot, marked stale rather than failed.
                            last = Some(
                                account_scan::last_snapshot(&self.store, account)
                                    .map(|mut q| {
                                        q.stale_since = q.stale_since.or(Some(now()));
                                        // e.g. "在 dsh 中重新登录": a hint, not a failed card.
                                        q.notice = Some(error.to_string());
                                        q
                                    })
                                    .unwrap_or_else(|| QuotaSnapshot {
                                        source_id: source.id,
                                        account_id: account.id.clone(),
                                        provider: account.provider.clone(),
                                        name: account.name.clone(),
                                        notice: claude_keychain::quota_notice(&error),
                                        error: Some(error.to_string()),
                                        ..Default::default()
                                    }),
                            )
                        }
                    }
                }
            }
            // Try all configured locations before presenting an optional local
            // Keychain refusal. Keep real cached values without inventing a
            // remaining-quota estimate from transcript tokens.
            if account.provider == "claude"
                && let Some(q) = last.as_mut()
                && q.error.is_some()
                && let Some(notice) = q.notice.take()
            {
                if let Some(cached) = account_scan::last_snapshot(&self.store, account) {
                    *q = cached;
                    q.stale_since = q.stale_since.or(Some(now()));
                } else {
                    q.error = None;
                    q.updated_at = now();
                    q.origin = "local".into();
                }
                q.notice = Some(notice);
            }
            // Another client's copy of a login read natively elsewhere: say where, not "no data".
            if last.is_none()
                && let Some(notice) = account_scan::shadow_notice(&settings, account)
            {
                last = Some(QuotaSnapshot {
                    account_id: account.id.clone(),
                    provider: account.provider.clone(),
                    name: account.name.clone(),
                    updated_at: now(),
                    origin: "local".into(),
                    notice: Some(notice),
                    ..Default::default()
                });
            }
            // Locally counted monthly credits keep the card visible without a server read.
            if quota::local_credits(&account.provider)
                && let Some(meter) = quota::local_credits_meter(&self.store, &settings, account)?
            {
                let q = last.get_or_insert_with(|| QuotaSnapshot {
                    account_id: account.id.clone(),
                    provider: account.provider.clone(),
                    name: account.name.clone(),
                    updated_at: now(),
                    origin: "local".into(),
                    ..Default::default()
                });
                if q.error.is_some() && q.windows.is_empty() && q.balances.is_empty() {
                    q.notice = q.error.take();
                }
                q.meters.retain(|m| m.id != meter.id);
                q.meters.push(meter);
            }
            if let Some(q) = last {
                result.push(q);
            }
        }
        Ok(result)
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.flush_alert_webhooks();
        let result = self.dispatch(method, params);
        // Quota and identity checks may have run qodercli in this process (any lane);
        // mark those control sessions as Aieyes' own before the next session read.
        let _ = crate::subscriptions::qoder::record_internal(&self.store);
        result
    }
    /// Monitor threads cannot write the database; their alerts are queued here.
    fn flush_alert_webhooks(&mut self) {
        while let Ok(event) = self.alert_webhooks.try_recv() {
            let Ok(settings) = self.store.settings() else {
                break;
            };
            let (title, text, data) =
                webhooks::alert_message(&settings.appearance.language, &event);
            let _ =
                self.store
                    .queue_webhook(&settings, webhooks::Kind::ServerAlert, title, text, data);
        }
    }
    /// Keep the monitor's alert rules and host set in step with the configuration.
    fn sync_host_observer(&self, settings: &Settings) {
        self.host_observer
            .set_interval(settings.server_refresh_seconds);
        self.host_observer.configure(
            &settings.server_alerts,
            settings.webhooks.allows(webhooks::Kind::ServerAlert),
        );
        let monitored: std::collections::HashSet<String> = settings
            .monitored_hosts()
            .into_iter()
            .filter(|host| host.enabled)
            .map(|host| host.id)
            .collect();
        self.host_observer.retain(&monitored);
    }
    /// The version that ran last on this data directory; set to the current one.
    fn launch_state(&self) -> Result<Value> {
        use rusqlite::OptionalExtension;
        let version = env!("CARGO_PKG_VERSION");
        let previous: Option<String> = self
            .store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key='last-run-version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        self.store.db.execute(
            "INSERT INTO kv(key,value) VALUES('last-run-version',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [version],
        )?;
        let updated = previous.as_deref().is_some_and(|p| p != version);
        Ok(json!({"version":version,"previousVersion":previous,"updated":updated}))
    }
    fn dispatch(&mut self, method: &str, mut params: Value) -> Result<Value> {
        match method {
            "dashboard.cancel" => {
                return Ok(operations::cancel_query(
                    params["operationId"].as_str().unwrap_or(""),
                ));
            }
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
                params["nativeConnections"] =
                    serde_json::to_value(self.store.settings()?.native_connections)?;
                params["claudeKeychain"] =
                    serde_json::to_value(self.store.settings()?.claude_keychain)?;
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
            "app.launch" => self.launch_state(),
            "webhooks.dispatch" => self.dispatch_webhooks(),
            "webhooks.test" => self.test_webhook(&params),
            name if name.starts_with("channels.") => self.channel_call(name, params),
            "hosts.history" => {
                let settings = self.store.settings()?;
                self.sync_host_observer(&settings);
                Ok(self.host_observer.history(params["hostId"].as_str()))
            }
            "hostAlerts.take" => {
                let settings = self.store.settings()?;
                self.sync_host_observer(&settings);
                Ok(serde_json::to_value(self.host_observer.take(
                    params["since"].as_i64().context("缺少通知会话起始时间")?,
                ))?)
            }
            "quotaResetNotifications.take" => {
                Ok(serde_json::to_value(self.store.take_reset_notifications(
                    params["since"].as_i64().context("缺少通知会话起始时间")?,
                )?)?)
            }
            "autoEstimates.configure" => self.configure_automatic(params),
            "autoEstimates.tick" => self.tick_automatic(),
            "autoEstimates.rebuild" => {
                let range = match (params["from"].as_i64(), params["to"].as_i64()) {
                    (None, None) => None,
                    (Some(from), Some(to)) => Some((from, to)),
                    _ => anyhow::bail!("历史恢复需要同时指定 from 和 to"),
                };
                self.store.rebuild_automatic_history_range(
                    params["accountKey"].as_str(),
                    params["dryRun"].as_bool().unwrap_or(false),
                    range,
                )
            }
            "accounts.targets" => self.account_targets(&params),
            "accounts.autoScan" => self.auto_scan(params),
            "accounts.native.remove" => self.remove_native_connection(params),
            "accounts.status.refreshBatch" => self.refresh_account_status_batch(&params),
            "subscriptions.openLogin" => subscriptions::open_login(&params),
            "subscriptions.connect" => self.connect_subscription(params),
            "qoderCli.status" => {
                self.qoder_cli_status(params["provider"].as_str().unwrap_or("qoder"))
            }
            "accounts.create"
            | "accounts.status.get"
            | "accounts.status.refresh"
            | "accounts.deletion.preview"
            | "accounts.delete"
            | "accounts.cleanup.list"
            | "accounts.deployments.sync" => self.account_call(method, params),
            name if name.starts_with("claudeAuth.") => self.claude_auth_call(name, params),
            name if name.starts_with("agyAuth.") => self.agy_auth_call(name, params),
            name if name.starts_with("copilotAuth.") => self.copilot_auth_call(name, params),
            name if name.starts_with("buddyAuth.") => self.buddy_auth_call(name, params),
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
                self.sessions.set_internal(self.store.internal_sessions()?);
                let settings = self.store.settings()?;
                let snapshot = self.sessions.read_with_usage(
                    &settings,
                    std::time::SystemTime::now(),
                    &self.store,
                );
                // Channel publication runs on its own worker; the observer hands off identity only.
                let _ = channels::observe(&self.store, &snapshot);
                for (kind, session) in
                    webhooks::session_transitions(&mut self.session_marks, &snapshot.sessions)
                {
                    if session.background || !settings.webhooks.allows(kind) {
                        continue;
                    }
                    let (title, text, data) =
                        webhooks::session_message(&settings.appearance.language, kind, &session);
                    // The session list is the primary result; a queue failure must not hide it.
                    let _ = self.store.queue_webhook(&settings, kind, title, text, data);
                }
                let mut value = serde_json::to_value(&snapshot)?;
                value["pet"] = serde_json::to_value(pets::project(&snapshot))?;
                value["summary"] = json!(snapshot.summary());
                value["phase"] = serde_json::to_value(snapshot.phase())?;
                value["activeCount"] = json!(snapshot.active_count());
                Ok(value)
            }
            "sessions.focusTarget" => Ok(sessions::focus_target(
                params["pid"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .context("缺少会话进程")?,
                params["processStart"].as_u64().unwrap_or(0),
            )),
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
                json!({"name":"Aieyes","version":env!("CARGO_PKG_VERSION"),"protocolVersion":1,"runtimeInvalidations":true,"hostMonitoring":true,"dashboardStatus":true,"chatgptWorkInstalled":provider_paths::chatgpt_work_installed(),"contractHash":contracts::CONTRACT_HASH}),
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
                // Refresh before deriving the cache identity so a forced probe
                // does not mistake its own proxy refresh for a settings change.
                let effective_proxy = network::effective_proxy_with_refresh(
                    &proxy,
                    method == "network.test" && params["force"] == true,
                );
                let key = serde_json::to_string(&(&proxy.mode, effective_proxy, &urls))?;
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
            "agents.manage" => self.manage_agent(params),
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
                let s: Settings = crate::settings::decode(params)?;
                self.store.save_settings(&s)?;
                Ok(json!({"saved":true}))
            }
            "dashboard.status" => self
                .store
                .dashboard_status(&serde_json::from_value(params)?),
            "dashboard" | "dashboard.summary" => {
                let mut filter = serde_json::from_value::<Filter>(params)?;
                // Usage belongs to an Agent/source, never the currently selected login.
                // Keep account filters on quota/status endpoints for compatibility.
                filter.account_id = None;
                Ok((if method == "dashboard.summary" {
                    self.store.dashboard_summary(&filter)?
                } else {
                    self.store.dashboard(&filter)?
                })
                .wire_value()?)
            }
            "sources.scan" | "sources.rescan" => {
                self.scan_history(params, method == "sources.rescan")
            }
            "quotas.schedule" => self.quota_schedule(),
            "quotas.refresh" => self.refresh_scheduled_quotas(params),
            "prices.list" => Ok(serde_json::to_value(self.store.prices()?)?),
            "prices.reference" => self.reference_prices(params),
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
                self.store.db.execute(
                    "INSERT OR REPLACE INTO kv(key,value) VALUES('openrouter-models',?1)",
                    [serde_json::to_string(&prices)?],
                )?;
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
            "hosts.monitor.configure" => {
                let settings = self.store.settings()?;
                self.sync_host_observer(&settings);
                let seconds = params["intervalSeconds"]
                    .as_f64()
                    .unwrap_or(settings.server_refresh_seconds as f64);
                anyhow::ensure!(
                    seconds.is_finite() && (1.0..=86400.0).contains(&seconds),
                    "服务器刷新间隔范围为 1–86400 秒"
                );
                if params["paused"].as_bool().unwrap_or(false) {
                    self.host_observer.pause();
                }
                self.host_observer.set_interval(seconds.ceil() as u64);
                Ok(self.host_monitor.configure(
                    settings.monitored_hosts(),
                    std::time::Duration::from_secs_f64(seconds),
                    params["paused"].as_bool().unwrap_or(false),
                ))
            }
            "hosts.monitor.refresh" => {
                if !self.host_monitor.configured() {
                    let settings = self.store.settings()?;
                    self.host_monitor.configure(
                        settings.monitored_hosts(),
                        std::time::Duration::from_secs(settings.server_refresh_seconds),
                        false,
                    );
                }
                self.host_monitor.refresh(params["hostId"].as_str())
            }
            "hosts.sample" => self.sample_hosts(params),
            _ => anyhow::bail!("未知方法：{method}"),
        }
    }
}
