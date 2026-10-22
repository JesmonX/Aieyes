use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}
impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
    pub fn all_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
    pub fn add(&mut self, other: &Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.reasoning += other.reasoning;
    }
    pub fn checked_delta(&self, prev: &Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(prev.input)?,
            output: self.output.checked_sub(prev.output)?,
            cache_read: self.cache_read.checked_sub(prev.cache_read)?,
            cache_write: self.cache_write.checked_sub(prev.cache_write)?,
            reasoning: self.reasoning.checked_sub(prev.reasoning)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    #[serde(default)]
    pub turn_id: String,
    pub id: String,
    // Persist stable identities for safe re-keying; old records may omit them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_identity: Option<String>,
    pub source_id: String,
    pub account_id: String,
    pub provider: String,
    pub session_id: String,
    pub model: String,
    pub timestamp: i64,
    pub tokens: Tokens,
    pub attribution: String,
    #[serde(default)]
    pub billing: BillingEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_credits: Option<NativeCredits>,
    /// Qoder first-party counters include cached input. Keep the original so a
    /// parser upgrade can replace legacy, double-counted rows safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_tokens: Option<Tokens>,
    #[serde(default)]
    pub interval_start: Option<i64>,
    #[serde(default)]
    pub interval_evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BillingEvidence {
    pub category: String,
    pub evidence: String,
}

/// Native request counters, independent of token prices and account balance.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeCredits {
    pub credits: f64,
    pub original_credits: Option<f64>,
    pub billable: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirrored_by: Option<String>,
}
impl NativeCredits {
    pub fn valid(&self) -> bool {
        self.credits.is_finite()
            && self.credits >= 0.0
            && self
                .original_credits
                .is_none_or(|n| n.is_finite() && n >= 0.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NativeCreditsTotal {
    pub credits: f64,
    pub original_credits: f64,
    pub original_credits_requests: u64,
    pub billable: f64,
    pub billable_requests: u64,
    pub non_billable: f64,
    pub non_billable_requests: u64,
    pub unknown_billability: f64,
    pub unknown_billability_requests: u64,
    pub requests: u64,
}
impl NativeCreditsTotal {
    pub fn add(&mut self, other: &Self) {
        self.credits += other.credits;
        self.original_credits += other.original_credits;
        self.original_credits_requests += other.original_credits_requests;
        self.billable += other.billable;
        self.billable_requests += other.billable_requests;
        self.non_billable += other.non_billable;
        self.non_billable_requests += other.non_billable_requests;
        self.unknown_billability += other.unknown_billability;
        self.unknown_billability_requests += other.unknown_billability_requests;
        self.requests += other.requests;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyConfig {
    pub mode: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineBinding {
    #[default]
    Bound,
    Unbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<crate::subscriptions::SubscriptionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_valuation: Option<crate::automatic::AutoValuationConfig>,
    pub codex_home_id: Option<String>,
    pub id: String,
    pub name: String,
    pub provider: String,
    pub account_id: String,
    pub path: String,
    pub host_id: Option<String>,
    pub machine_binding: MachineBinding,
    pub last_machine_name: Option<String>,
    pub enabled: bool,
    pub quota_command: String,
    pub quota_pre_command: String,
    pub codex_binary: String,
    pub agy_binary: String,
    pub proxy: Option<ProxyConfig>,
}
impl Default for Source {
    fn default() -> Self {
        Self {
            subscription: None,
            codex_home_id: None,
            auto_valuation: None,
            id: String::new(),
            name: String::new(),
            provider: "codex".into(),
            account_id: String::new(),
            path: String::new(),
            host_id: None,
            machine_binding: MachineBinding::Bound,
            last_machine_name: None,
            enabled: true,
            quota_command: String::new(),
            quota_pre_command: String::new(),
            codex_binary: crate::contracts::provider("codex").unwrap().binary.into(),
            agy_binary: crate::contracts::provider("antigravity")
                .unwrap()
                .binary
                .into(),
            proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub quota_reset_notifications: crate::quota_notifications::Overrides,
    pub pending_name: bool,
    pub quota_refresh_seconds: Option<u64>,
    pub device_settings: Vec<AccountDeviceSettings>,
    pub identity_key: Option<String>,
    pub connections: Vec<AccountConnection>,
    pub quota_profile_id: Option<String>,
    pub id: String,
    pub name: String,
    pub provider: String,
    pub quota_enabled: bool,
    pub quota_source_id: Option<String>,
    pub archived: bool,
}
impl Default for Account {
    fn default() -> Self {
        Self {
            quota_reset_notifications: Default::default(),
            pending_name: false,
            quota_refresh_seconds: None,
            device_settings: vec![],
            identity_key: None,
            connections: vec![],
            quota_profile_id: None,
            id: String::new(),
            name: String::new(),
            provider: "codex".into(),
            quota_enabled: true,
            quota_source_id: None,
            archived: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountConnection {
    pub source_id: String,
    pub profile_id: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountDeviceSettings {
    pub machine_id: String,
    pub pre_command: String,
    pub pre_command_reset_required: bool,
}

impl Account {
    pub fn profile_refs(&self) -> Vec<&str> {
        let mut refs = Vec::new();
        for r in self.quota_profile_id.iter().chain(
            self.connections
                .iter()
                .filter_map(|c| c.profile_id.as_ref()),
        ) {
            if !refs.contains(&r.as_str()) {
                refs.push(r.as_str());
            }
        }
        refs
    }
    pub fn has_profile(&self, reference: &str) -> bool {
        self.profile_refs().contains(&reference)
    }
    pub fn uses_source(&self, source: &Source) -> bool {
        self.provider == source.provider
            && (source.account_id == self.id
                || source.codex_home_id.as_ref().is_some_and(|home| {
                    self.profile_refs()
                        .iter()
                        .any(|r| r.starts_with(&format!("{home}:")))
                }))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentConfiguration {
    pub provider: String,
    pub enabled: bool,
    pub machine_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub target: String,
    pub port: Option<u16>,
    pub identity_file: String,
    pub auth_mode: String,
    pub username: String,
    pub password_ref: String,
    pub shell: String,
    pub pre_command: String,
    pub enabled: bool,
    pub monitoring_enabled: bool,
    pub metrics: Vec<String>,
    pub devices: Vec<String>,
    pub details: Vec<String>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            target: String::new(),
            port: None,
            identity_file: String::new(),
            auth_mode: "ssh".into(),
            username: String::new(),
            password_ref: String::new(),
            shell: crate::ssh::DEFAULT_SHELL.into(),
            pre_command: String::new(),
            enabled: true,
            monitoring_enabled: true,
            metrics: ["cpu", "memory", "gpu", "filesystems", "disk", "network"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            devices: vec![],
            details: [
                "uptime",
                "cpuTimes",
                "memoryCache",
                "swap",
                "fsAvailable",
                "fsType",
                "inodes",
                "diskIops",
                "diskBusy",
                "networkTotals",
                "networkErrors",
                "gpuMemory",
                "gpuThermals",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalMonitor {
    pub enabled: bool,
    pub metrics: Vec<String>,
    pub devices: Vec<String>,
    pub details: Vec<String>,
}
impl Default for LocalMonitor {
    fn default() -> Self {
        let host = Host::default();
        Self {
            enabled: true,
            metrics: host.metrics,
            devices: host.devices,
            details: host.details,
        }
    }
}
impl LocalMonitor {
    pub fn host(&self) -> Host {
        Host {
            id: "local".into(),
            name: "本机".into(),
            target: "本机".into(),
            enabled: self.enabled,
            metrics: self.metrics.clone(),
            devices: self.devices.clone(),
            details: self.details.clone(),
            ..Host::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub native_connections: Vec<crate::account_scan::NativeConnection>,
    pub quota_reset_notifications: crate::quota_notifications::Preferences,
    pub codex_apis: Vec<crate::codex_api::Connection>,
    #[serde(skip_deserializing)]
    pub configuration_issues: Vec<String>,
    pub deleted_accounts: Vec<Account>,
    pub history_show_deleted_accounts: bool,
    pub codex_precise_counting: bool,
    pub agents: Vec<AgentConfiguration>,
    pub account_aliases: std::collections::BTreeMap<String, String>,
    pub appearance: Appearance,
    pub version: u32,
    pub sources: Vec<Source>,
    pub accounts: Vec<Account>,
    pub hosts: Vec<Host>,
    pub proxy: ProxyConfig,
    pub refresh_seconds: u64,
    pub history_refresh_seconds: u64,
    pub server_refresh_seconds: u64,
    pub server_foreground_refresh_seconds: u64,
    pub local_monitor: LocalMonitor,
    pub local_device_enabled: bool,
    pub menu_metric: String,
    pub model_mappings: std::collections::BTreeMap<String, String>,
    pub proxy_test_urls: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            native_connections: vec![],
            quota_reset_notifications: Default::default(),
            codex_apis: vec![],
            configuration_issues: vec![],
            deleted_accounts: vec![],
            history_show_deleted_accounts: false,
            codex_precise_counting: false,
            agents: vec![],
            appearance: Appearance::default(),
            account_aliases: Default::default(),
            version: 1,
            sources: vec![],
            accounts: vec![],
            hosts: vec![],
            proxy: ProxyConfig {
                mode: "system".into(),
                url: String::new(),
            },
            refresh_seconds: 300,
            history_refresh_seconds: 300,
            server_refresh_seconds: 10,
            server_foreground_refresh_seconds: 2,
            local_monitor: LocalMonitor::default(),
            local_device_enabled: true,
            menu_metric: "icon".into(),
            model_mappings: Default::default(),
            proxy_test_urls: crate::network::default_test_urls(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub language: String,
    pub theme: String,
    pub accent: String,
    pub chart_palette: String,
    pub chart_contrast: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            theme: "system".into(),
            accent: "indigo".into(),
            chart_palette: "classic".into(),
            chart_contrast: "high".into(),
        }
    }
}

impl Settings {
    pub fn migrate(&mut self) {
        self.appearance.language = crate::localization::normalize(&self.appearance.language).into();
        if self.version < 2 {
            for source in &self.sources {
                if !source.account_id.is_empty()
                    && !self
                        .accounts
                        .iter()
                        .any(|a| a.id == source.account_id && a.provider == source.provider)
                {
                    self.accounts.push(Account {
                        id: source.account_id.clone(),
                        name: if source.name.is_empty() {
                            source.account_id.clone()
                        } else {
                            source.name.clone()
                        },
                        provider: source.provider.clone(),
                        quota_enabled: crate::providers::known(&source.provider)
                            || !source.quota_command.is_empty(),
                        ..Default::default()
                    });
                }
            }
            self.version = 2;
        }
        for agent in &mut self.agents {
            agent.machine_ids.sort();
            agent.machine_ids.dedup();
        }
        let mut unique = Vec::new();
        self.agents.retain(|a| {
            if unique.contains(a) {
                false
            } else {
                unique.push(a.clone());
                true
            }
        });
        for &provider in crate::providers::PROVIDERS {
            if !self.agents.iter().any(|a| a.provider == provider) {
                let sources: Vec<_> = self
                    .sources
                    .iter()
                    .filter(|s| crate::providers::canonical(&s.provider) == provider && s.enabled)
                    .collect();
                let mut machine_ids: Vec<String> = sources
                    .iter()
                    .map(|s| s.host_id.clone().unwrap_or_else(|| "local".into()))
                    .collect();
                machine_ids.sort();
                machine_ids.dedup();
                self.agents.push(AgentConfiguration {
                    provider: provider.into(),
                    enabled: !sources.is_empty(),
                    machine_ids,
                });
            }
        }
        for account in &mut self.accounts {
            account.connections.retain(|c| c.profile_id.is_some());
            for source in &self.sources {
                if source.provider == account.provider
                    && !source.account_id.is_empty()
                    && source.account_id == account.id
                {
                    account.connections.push(AccountConnection {
                        source_id: source.id.clone(),
                        profile_id: None,
                    });
                }
                if self.version < 7
                    && let Some(reference) = &account.quota_profile_id
                    && source
                        .codex_home_id
                        .as_ref()
                        .is_some_and(|h| reference.starts_with(&format!("{h}:")))
                    && !account
                        .connections
                        .iter()
                        .any(|c| c.profile_id.as_ref() == Some(reference))
                {
                    account.connections.push(AccountConnection {
                        source_id: source.id.clone(),
                        profile_id: Some(reference.clone()),
                    });
                }
            }
            let mut seen = Vec::new();
            account.connections.retain(|c| {
                if seen.contains(c) {
                    false
                } else {
                    seen.push(c.clone());
                    true
                }
            });
            account.quota_profile_id = account
                .connections
                .iter()
                .find_map(|c| c.profile_id.clone());
        }
        {
            // New associations use the same account × machine setting as migrated ones.
            // Once present (including an explicit empty value), directory values never win.
            let mut clear_sources = std::collections::BTreeSet::new();
            for account in &mut self.accounts {
                let mut commands: std::collections::BTreeMap<
                    String,
                    std::collections::BTreeSet<String>,
                > = Default::default();
                for source in self.sources.iter().filter(|s| account.uses_source(s)) {
                    if let Some(machine) = &source.host_id {
                        let legacy = if source.quota_pre_command.trim().is_empty() {
                            self.hosts
                                .iter()
                                .find(|h| &h.id == machine)
                                .map(|h| h.pre_command.clone())
                                .unwrap_or_default()
                        } else {
                            source.quota_pre_command.clone()
                        };
                        commands.entry(machine.clone()).or_default().insert(legacy);
                    }
                }
                for (machine_id, values) in commands {
                    if !account
                        .device_settings
                        .iter()
                        .any(|d| d.machine_id == machine_id)
                    {
                        let conflict = values.len() > 1;
                        if conflict {
                            for source in self.sources.iter().filter(|s| {
                                account.uses_source(s) && s.host_id.as_ref() == Some(&machine_id)
                            }) {
                                clear_sources.insert(source.id.clone());
                            }
                        }
                        account.device_settings.push(AccountDeviceSettings {
                            machine_id,
                            pre_command: if conflict {
                                String::new()
                            } else {
                                values.into_iter().next().unwrap_or_default()
                            },
                            pre_command_reset_required: conflict,
                        });
                    }
                }
            }
            for source in &mut self.sources {
                if clear_sources.contains(&source.id) {
                    source.quota_pre_command.clear();
                }
            }
        }
        if self.version < 5 {
            self.history_refresh_seconds = self.refresh_seconds.max(10);
        }
        // Version 6 drops the unused githubRepository key during deserialization.
        if self.version < 9 {
            // Preserve the effective resource-monitoring state before separating the switches.
            self.local_monitor.enabled &= self.local_device_enabled;
            for host in &mut self.hosts {
                host.monitoring_enabled &= host.enabled;
            }
        }
        for connection in &mut self.native_connections {
            if connection.account_key.as_ref().is_some_and(|key| {
                !self
                    .accounts
                    .iter()
                    .any(|a| crate::accounts::key(a) == *key)
            }) {
                connection.account_key = None;
                connection.status = "pending".into();
                connection.note = "账户关联已解除，可重新扫描或在原生客户端管理".into();
            }
        }
        self.version = self.version.max(11);
        self.configuration_issues.clear();
        let mut providers = std::collections::HashSet::new();
        for agent in &self.agents {
            if !providers.insert(&agent.provider) {
                self.configuration_issues.push(format!(
                    "{} 有冲突的重复配置，请在接入与账户中重新选择机器并保存",
                    agent.provider
                ));
            }
        }
        if let Err(error) = crate::codex_auth::validate_settings(self) {
            self.configuration_issues.push(error.to_string());
        }
    }
    pub fn monitored_hosts(&self) -> Vec<Host> {
        std::iter::once(self.local_monitor.host())
            .chain(self.hosts.iter().cloned().map(|mut host| {
                host.enabled = host.monitoring_enabled;
                host
            }))
            .collect()
    }
    pub fn source_enabled(&self, source: &Source) -> bool {
        let mut agents = self.agents.iter().filter(|a| a.provider == source.provider);
        let enabled = agents.next().is_none_or(|a| {
            a.enabled
                && a.machine_ids
                    .iter()
                    .any(|id| id == source.host_id.as_deref().unwrap_or("local"))
        });
        source.enabled
            && source.machine_binding == MachineBinding::Bound
            && (source.host_id.is_some() || self.local_device_enabled)
            && enabled
            && agents.next().is_none()
            && source
                .host_id
                .as_ref()
                .is_none_or(|id| self.hosts.iter().any(|h| &h.id == id && h.enabled))
    }
    pub fn quota_enabled(&self, source: &Source) -> bool {
        !source.account_id.is_empty()
            && self.accounts.iter().any(|a| {
                a.id == source.account_id
                    && a.provider == source.provider
                    && a.quota_enabled
                    && !a.archived
            })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParseState {
    /// First main-chain Qoder model, including requests whose counters are hidden.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub qoder_main_model: String,
    pub turn_id: String,
    pub parser_version: u32,
    // Absent in legacy cursors, which must be replayed once with the saved mode.
    pub codex_precise_counting: Option<bool>,
    pub task_started_at: Option<i64>,
    pub session_id: String,
    pub model: String,
    pub last_cumulative: Option<Tokens>,
    pub cumulative_segment: u64,
    pub last_timestamp: i64,
    pub model_provider: String,
    pub session_started_at: Option<i64>,
    pub inherited_history: Option<crate::codex_history::InheritedHistory>,
    pub first_inherited_count: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_checkpoint: Option<HarnessCheckpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_checkpoint: Option<DatabaseCheckpoint>,
    #[serde(skip)]
    pub counter_correction: Option<crate::codex_history::CounterCorrection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HarnessCheckpoint {
    pub fingerprint: String,
    pub identity: String,
    pub anchor: String,
    pub size: u64,
    pub version: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DatabaseCheckpoint {
    pub fingerprint: String,
    pub identity: String,
    pub table: String,
    pub anchor: String,
    pub pending_after: i64,
    pub pending_fingerprint: String,
    pub malformed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaWindow {
    pub id: String,
    pub group_id: String,
    pub group_name: String,
    pub name: String,
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Balance {
    pub currency: String,
    pub total: String,
    pub granted: String,
    pub topped_up: String,
}

/// Monetary wallet observations in major currency units, kept as exact decimal
/// strings. This is separate from subscription windows and native request Credits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtraUsageWallet {
    pub currency: String,
    pub total: String,
    pub balance: Option<String>,
    pub monthly_limit_enabled: Option<bool>,
    pub monthly_limit: Option<String>,
    pub monthly_used: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CreditsSnapshot {
    #[serde(alias = "has_credits")]
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountIdentity {
    pub key: String,
    pub email: String,
    pub subscription: Option<String>,
    pub checked_at: i64,
    pub subscription_checked_at: Option<i64>,
    pub stale: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaSnapshot {
    pub identity: Option<AccountIdentity>,
    pub metadata_error: Option<String>,
    pub credits: Option<CreditsSnapshot>,
    pub credits_updated_at: Option<i64>,
    pub credits_origin: Option<String>,
    pub source_id: String,
    pub account_id: String,
    pub provider: String,
    pub name: String,
    pub updated_at: i64,
    pub origin: String,
    pub windows: Vec<QuotaWindow>,
    pub balances: Vec<Balance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_usage: Option<ExtraUsageWallet>,
    pub is_available: Option<bool>,
    pub bank_reset: Option<Value>,
    pub bank_updated_at: Option<i64>,
    pub plan: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPrice {
    pub id: String,
    pub name: String,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Aggregate {
    pub key: String,
    pub tokens: Tokens,
    pub total: u64,
    pub cost: f64,
    pub priced_tokens: u64,
    pub events: u64,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub native_credits: std::collections::BTreeMap<String, NativeCreditsTotal>,
}
impl Aggregate {
    pub fn add_native_credits(&mut self, event: &UsageEvent) {
        if let Some(c) = event
            .native_credits
            .as_ref()
            .filter(|c| c.valid() && c.mirrored_by.is_none())
        {
            let row = self
                .native_credits
                .entry(event.provider.clone())
                .or_default();
            row.credits += c.credits;
            row.requests += 1;
            if let Some(original) = c.original_credits {
                row.original_credits += original;
                row.original_credits_requests += 1;
            }
            match c.billable {
                Some(true) => {
                    row.billable += c.credits;
                    row.billable_requests += 1;
                }
                Some(false) => {
                    row.non_billable += c.credits;
                    row.non_billable_requests += 1;
                }
                None => {
                    row.unknown_billability += c.credits;
                    row.unknown_billability_requests += 1;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayModel {
    pub day: String,
    pub model: String,
    pub usage: Aggregate,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PricingGap {
    pub model: String,
    pub price_id: Option<String>,
    pub tokens: Tokens,
    pub unpriced_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dashboard {
    #[serde(default)]
    pub billing_summary: Value,
    pub generated_at: i64,
    pub summary: Aggregate,
    #[serde(skip_serializing)]
    pub days: Vec<Aggregate>,
    pub heatmap: Vec<Aggregate>,
    pub models: Vec<Aggregate>,
    /// Model choices honor the other filters but not the selected model itself.
    pub model_options: Vec<String>,
    /// Display labels only; filters, pricing and persisted events keep their model keys.
    pub model_display_names: BTreeMap<String, String>,
    pub trend_days: Vec<Aggregate>,
    pub day_models: Vec<DayModel>,
    pub pricing_gaps: Vec<PricingGap>,
    pub quotas: Vec<QuotaSnapshot>,
    pub quota_order: Vec<String>,
    pub quota_estimates: Vec<crate::estimates::Estimate>,
    pub credit_estimates: Vec<crate::estimates::Estimate>,
    pub automatic_estimate_summaries: Vec<crate::rolling_estimates::Summary>,
    pub sources: Vec<Value>,
    #[serde(skip_serializing)]
    pub price_updated_at: Option<i64>,
}

impl Dashboard {
    /// Trim display-only responses without changing persisted event/parser tokens.
    pub fn wire_value(&self) -> serde_json::Result<Value> {
        fn trim(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::Object(tokens)) = map.get_mut("tokens") {
                        tokens.remove("reasoning");
                    }
                    for child in map.values_mut() {
                        trim(child);
                    }
                }
                Value::Array(rows) => rows.iter_mut().for_each(trim),
                _ => {}
            }
        }
        let mut value = serde_json::to_value(self)?;
        trim(&mut value);
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DateRange {
    All,
    Custom { start: String, end: String },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub estimate_history: Option<String>,
    pub provider: Option<String>,
    pub source_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub days: Option<u32>,
    pub date_range: Option<DateRange>,
}

impl Filter {
    pub(crate) fn provider_alias(&self) -> Option<&str> {
        // Account keys remain adapter-specific, including when legacy accounts
        // happen to share an ID. Only the agent-wide view combines adapters.
        if self.account_id.as_ref().is_some_and(|id| !id.is_empty()) {
            return None;
        }
        let selected = crate::contracts::provider(self.provider.as_deref()?)?;
        crate::contracts::PROVIDERS
            .iter()
            .find(|row| row.id != selected.id && row.agent_id == selected.agent_id)
            .map(|row| row.id)
    }

    pub(crate) fn includes_provider(&self, provider: &str) -> bool {
        self.provider
            .as_ref()
            .is_none_or(|selected| selected == provider)
            || self.provider_alias() == Some(provider)
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn timestamp(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp())
    })
}
