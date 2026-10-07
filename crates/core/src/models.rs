use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub id: String,
    // Only the parser needs this key; old persisted events remain readable.
    #[serde(skip)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyConfig {
    pub mode: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub account_id: String,
    pub path: String,
    pub host_id: Option<String>,
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
            id: String::new(),
            name: String::new(),
            provider: "codex".into(),
            account_id: String::new(),
            path: String::new(),
            host_id: None,
            enabled: true,
            quota_command: String::new(),
            quota_pre_command: String::new(),
            codex_binary: "codex".into(),
            agy_binary: "agy".into(),
            proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
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
            id: String::new(),
            name: String::new(),
            provider: "codex".into(),
            quota_enabled: true,
            quota_source_id: None,
            archived: false,
        }
    }
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
            metrics: ["cpu", "memory", "gpu", "filesystems", "disk", "network"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            devices: vec![],
            details: [
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
pub struct Settings {
    pub appearance: Appearance,
    pub version: u32,
    pub sources: Vec<Source>,
    pub accounts: Vec<Account>,
    pub hosts: Vec<Host>,
    pub proxy: ProxyConfig,
    pub refresh_seconds: u64,
    pub server_refresh_seconds: u64,
    pub menu_metric: String,
    pub github_repository: String,
    pub model_mappings: std::collections::BTreeMap<String, String>,
    pub proxy_test_urls: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            version: 1,
            sources: vec![],
            accounts: vec![],
            hosts: vec![],
            proxy: ProxyConfig {
                mode: "system".into(),
                url: String::new(),
            },
            refresh_seconds: 300,
            server_refresh_seconds: 10,
            menu_metric: "icon".into(),
            github_repository: String::new(),
            model_mappings: Default::default(),
            proxy_test_urls: crate::network::default_test_urls(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub theme: String,
    pub accent: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            accent: "indigo".into(),
        }
    }
}

impl Settings {
    pub fn migrate(&mut self) {
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
                        quota_enabled: ["codex", "claude", "agy", "deepseek"]
                            .contains(&source.provider.as_str())
                            || !source.quota_command.is_empty(),
                        ..Default::default()
                    });
                }
            }
            self.version = 2;
        }
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
    pub parser_version: u32,
    pub task_started_at: Option<i64>,
    pub session_id: String,
    pub model: String,
    pub last_cumulative: Option<Tokens>,
    pub cumulative_segment: u64,
    pub last_timestamp: i64,
    pub model_provider: String,
    pub session_started_at: Option<i64>,
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
pub struct QuotaSnapshot {
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
    pub generated_at: i64,
    pub summary: Aggregate,
    pub days: Vec<Aggregate>,
    pub heatmap: Vec<Aggregate>,
    pub models: Vec<Aggregate>,
    /// Model choices honor the other filters but not the selected model itself.
    pub model_options: Vec<String>,
    pub trend_days: Vec<Aggregate>,
    pub day_models: Vec<DayModel>,
    pub pricing_gaps: Vec<PricingGap>,
    pub quotas: Vec<QuotaSnapshot>,
    pub quota_order: Vec<String>,
    pub quota_estimates: Vec<crate::estimates::Estimate>,
    pub credit_estimates: Vec<crate::estimates::Estimate>,
    pub sources: Vec<Value>,
    pub price_updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub provider: Option<String>,
    pub source_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub days: Option<u32>,
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
