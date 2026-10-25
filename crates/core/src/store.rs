use crate::{models::*, pricing};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct DeletedAccountsCache {
    revisions: (i64, i64),
    accounts: Vec<Account>,
}

pub struct Store {
    pub root: PathBuf,
    pub db: Connection,
    pub(crate) usage_memory: std::cell::RefCell<crate::usage_cache::Memory>,
    pub(crate) automatic_summary_memory: std::cell::RefCell<crate::rolling_estimates::Memory>,
    pub(crate) estimate_display: std::cell::RefCell<Option<(i64, Vec<crate::estimates::Estimate>)>>,
    settings_memory: std::cell::RefCell<Option<(i64, Settings)>>,
    deleted_accounts_memory: std::cell::RefCell<Option<DeletedAccountsCache>>,
    prices_memory: std::cell::RefCell<Option<(i64, std::sync::Arc<Vec<ModelPrice>>)>>,
    #[cfg(test)]
    pub(crate) billing_memory: std::cell::RefCell<crate::codex_api::usage::BillingCache>,
}
/// Keep a short committed/rolled-back write visible to the updater until SQLite
/// releases its lock. Network and file reads happen outside this guard.
pub(crate) struct WriteTransaction<'a> {
    transaction: rusqlite::Transaction<'a>,
    _critical: crate::operations::Critical,
    signal: std::sync::Arc<crate::runtime::Signal>,
}
impl std::ops::Deref for WriteTransaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.transaction
    }
}
impl WriteTransaction<'_> {
    pub fn commit(self) -> rusqlite::Result<()> {
        self.transaction.commit()?;
        self.signal.wake();
        Ok(())
    }
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let db = Connection::open(root.join("aieyes.sqlite"))?;
        db.busy_timeout(std::time::Duration::from_millis(5000))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY,provider TEXT NOT NULL,account_id TEXT NOT NULL,model TEXT NOT NULL,stamp INTEGER NOT NULL,payload TEXT NOT NULL,cost REAL NOT NULL,priced_tokens INTEGER NOT NULL,price TEXT);
        CREATE TABLE IF NOT EXISTS native_pending(source_id TEXT NOT NULL,path TEXT NOT NULL,row_id INTEGER NOT NULL,invalid INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(source_id,path,row_id));
        CREATE INDEX IF NOT EXISTS events_stamp ON events(stamp);
        CREATE INDEX IF NOT EXISTS events_model ON events(model);
        CREATE INDEX IF NOT EXISTS events_account ON events(provider,account_id);
        CREATE INDEX IF NOT EXISTS events_account_stamp ON events(provider,account_id,stamp);
        CREATE TABLE IF NOT EXISTS event_sources(event_id TEXT NOT NULL REFERENCES events(id),source_id TEXT NOT NULL,PRIMARY KEY(event_id,source_id));
        CREATE INDEX IF NOT EXISTS event_sources_source ON event_sources(source_id,event_id);
        CREATE TABLE IF NOT EXISTS source_identities(source_id TEXT NOT NULL,provider TEXT NOT NULL,account_id TEXT NOT NULL,PRIMARY KEY(source_id,provider,account_id));
        CREATE TABLE IF NOT EXISTS files(source_id TEXT NOT NULL,path TEXT NOT NULL,signature TEXT NOT NULL,offset INTEGER NOT NULL,state TEXT NOT NULL,PRIMARY KEY(source_id,path));
        CREATE TABLE IF NOT EXISTS quotas(account_key TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS source_status(source_id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS price_history(id INTEGER PRIMARY KEY,model_id TEXT NOT NULL,fetched_at INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS price_model ON price_history(model_id,fetched_at);
        CREATE TABLE IF NOT EXISTS quota_history(id INTEGER PRIMARY KEY,account_key TEXT NOT NULL,stamp INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS quota_history_account ON quota_history(account_key,stamp);
        CREATE TABLE IF NOT EXISTS estimates(kind TEXT NOT NULL CHECK(kind IN ('quota','credits')),id TEXT NOT NULL,account_key TEXT NOT NULL,status TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(kind,id));
        DROP INDEX IF EXISTS estimates_active;
        CREATE UNIQUE INDEX IF NOT EXISTS estimates_manual_active ON estimates(kind,account_key) WHERE status IN ('active','pending') AND COALESCE(json_extract(payload,'$.samplingMode'),'')!='automatic';
        CREATE UNIQUE INDEX IF NOT EXISTS estimates_automatic_active ON estimates(kind,account_key,json_extract(payload,'$.windowId')) WHERE status IN ('active','pending') AND json_extract(payload,'$.samplingMode')='automatic';
        CREATE TABLE IF NOT EXISTS capacity_samples(account_key TEXT NOT NULL,stamp INTEGER NOT NULL,five REAL NOT NULL,week REAL NOT NULL,PRIMARY KEY(account_key,stamp));
        ")?;
        crate::usage_cache::setup(&db)?;
        crate::native_credits::setup(&db)?;
        crate::quota_notifications::setup(&db)?;
        crate::remote_history::setup(&db)?;
        crate::qoder_state::setup(&db)?;
        crate::codex_api::usage::setup(&db)?;
        crate::history_corrections::setup(&db)?;
        crate::codex_routes::setup(&db)?;
        crate::codex_attribution::setup(&db)?;
        crate::runtime::setup(&db)?;
        crate::rolling_estimates::setup(&db)?;
        let store = Self {
            root: root.to_path_buf(),
            db,
            usage_memory: Default::default(),
            estimate_display: Default::default(),
            automatic_summary_memory: Default::default(),
            settings_memory: Default::default(),
            deleted_accounts_memory: Default::default(),
            prices_memory: Default::default(),
            #[cfg(test)]
            billing_memory: Default::default(),
        };
        crate::history_counting::setup(&store)?;
        store.migrate_estimate_tables()?;
        store.migrate_antigravity()?;
        store.migrate_usage_ids()?;
        store.bootstrap_capacity()?;
        store.repair_history_estimates()?;
        store.migrate_account_commands()?;
        Ok(store)
    }
    /// Acquire the writer before reading a snapshot; never upgrade a stale WAL reader.
    pub(crate) fn write_transaction(&self) -> Result<WriteTransaction<'_>> {
        let start = std::time::Instant::now();
        loop {
            crate::operations::check()?;
            match rusqlite::Transaction::new_unchecked(
                &self.db,
                rusqlite::TransactionBehavior::Immediate,
            ) {
                Ok(transaction) => {
                    return Ok(WriteTransaction {
                        transaction,
                        _critical: crate::operations::critical()?,
                        signal: crate::runtime::signal(&self.root),
                    });
                }
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if matches!(
                        e.code,
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                    ) && start.elapsed() < std::time::Duration::from_secs(30) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => return Err(e).context("数据库写入未完成，已提交进度保留，请重试"),
            }
        }
    }
    pub fn settings(&self) -> Result<Settings> {
        // Keep the configuration and its revision in one snapshot. An external
        // commit between those reads must not label old settings as current.
        let transaction = if self.db.is_autocommit() {
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        // Read the revision in the caller's SQLite snapshot. Never publish values
        // from a write transaction: a rollback can reuse the same revision later.
        let committed =
            self.db.transaction_state(None::<&str>)? != rusqlite::TransactionState::Write;
        let revision = crate::runtime::revision(&self.db, "settings")?;
        let cached = self
            .settings_memory
            .borrow()
            .as_ref()
            .filter(|(known, _)| committed && *known == revision)
            .map(|(_, settings)| settings.clone());
        let mut settings = if let Some(settings) = cached {
            settings
        } else {
            let mut settings = self.read_settings()?;
            settings.deleted_accounts.clear();
            if self.db.transaction_state(None::<&str>)? != rusqlite::TransactionState::Write {
                *self.settings_memory.borrow_mut() = Some((revision, settings.clone()));
            }
            settings
        };
        // Deleted accounts are a usage-dependent projection, not configuration.
        let committed =
            self.db.transaction_state(None::<&str>)? != rusqlite::TransactionState::Write;
        let version = (
            crate::runtime::revision(&self.db, "settings")?,
            crate::runtime::revision(&self.db, "usage")?,
        );
        let cached = self
            .deleted_accounts_memory
            .borrow()
            .as_ref()
            .filter(|cached| committed && cached.revisions == version)
            .map(|cached| cached.accounts.clone());
        settings.deleted_accounts = if let Some(accounts) = cached {
            accounts
        } else {
            let mut query = self.db.prepare_cached("SELECT value FROM kv WHERE key LIKE 'deleted-account:%' AND EXISTS(SELECT 1 FROM events WHERE provider=json_extract(kv.value,'$.provider') AND account_id=json_extract(kv.value,'$.id')) ORDER BY json_extract(value,'$.deletedAt'),key")?;
            let values = query
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let accounts = values
                .iter()
                .map(|raw| {
                    let mut account: Account = serde_json::from_str(raw)?;
                    account.archived = true;
                    account.quota_enabled = false;
                    Ok(account)
                })
                .collect::<Result<Vec<_>>>()?;
            if committed {
                *self.deleted_accounts_memory.borrow_mut() = Some(DeletedAccountsCache {
                    revisions: version,
                    accounts: accounts.clone(),
                });
            }
            accounts
        };
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(settings)
    }
    fn read_settings(&self) -> Result<Settings> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        match raw {
            Some(s) => {
                let mut settings: Settings = crate::settings::decode(serde_json::from_str(&s)?)?;
                crate::providers::normalize_settings(&mut settings);
                settings.counting_generation = crate::history_counting::generation(self)?;
                Ok(settings)
            }
            None => {
                let mut settings = Settings::default();
                for entry in crate::contracts::PROVIDERS
                    .iter()
                    .filter(|p| p.discover_local)
                {
                    let provider = entry.id;
                    let Ok(path) = crate::provider_paths::default_path(provider, false) else {
                        continue;
                    };
                    let root = expand(&path);
                    if root.is_dir() {
                        settings.sources.push(Source {
                            id: format!("local-{provider}"),
                            name: format!("{} · 本机", entry.name),
                            provider: provider.into(),
                            // Discover history directories only. Explicit account
                            // actions own account creation and identity-based naming.
                            path: root.to_string_lossy().into(),
                            ..Default::default()
                        });
                    }
                }
                crate::providers::normalize_settings(&mut settings);
                // Fresh installations offer every named Agent, without importing logins
                // or opting a newly added SSH machine into collection.
                for agent in &mut settings.agents {
                    if agent.provider != "custom" {
                        agent.enabled = true;
                    }
                }
                self.save_settings(&settings)?;
                settings.counting_generation = crate::history_counting::generation(self)?;
                Ok(settings)
            }
        }
    }
    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.save_settings_checked(s, false)
    }
    pub(crate) fn save_auth_settings(&self, s: &Settings) -> Result<()> {
        self.save_settings_checked(s, true)
    }
    fn save_settings_checked(&self, s: &Settings, verified_auth_change: bool) -> Result<()> {
        anyhow::ensure!(
            ["system", "light", "dark"].contains(&s.appearance.theme.as_str()),
            "无效的外观主题"
        );
        anyhow::ensure!(
            ["indigo", "blue", "teal", "purple"].contains(&s.appearance.accent.as_str()),
            "无效的强调色"
        );
        anyhow::ensure!(
            ["classic", "tableau", "tol", "set2", "nord"]
                .contains(&s.appearance.chart_palette.as_str()),
            "无效的图表配色"
        );
        anyhow::ensure!(
            ["high", "low"].contains(&s.appearance.chart_contrast.as_str()),
            "无效的图表对比度"
        );
        let mut normalized = s.clone();
        crate::providers::normalize_settings(&mut normalized);
        let previous: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let previous = previous
            .map(|raw| serde_json::from_str::<Settings>(&raw))
            .transpose()?;
        // Host removal is a core transaction, including for older clients that
        // used to turn remote sources into local ones by clearing hostId.
        if let Some(old) = &previous {
            let removed: Vec<_> = old
                .hosts
                .iter()
                .filter(|host| !normalized.hosts.iter().any(|h| h.id == host.id))
                .collect();
            for source in &mut normalized.sources {
                if let Some(prior) = old.sources.iter().find(|s| s.id == source.id) {
                    if let Some(host) = removed
                        .iter()
                        .find(|h| prior.host_id.as_ref() == Some(&h.id))
                    {
                        source.host_id = prior.host_id.clone();
                        source.machine_binding = MachineBinding::Unbound;
                        source.last_machine_name = Some(
                            if host.name.is_empty() {
                                &host.target
                            } else {
                                &host.name
                            }
                            .clone(),
                        );
                        source.enabled = false;
                    }
                    anyhow::ensure!(
                        verified_auth_change
                            || prior.machine_binding != MachineBinding::Unbound
                            || (source.machine_binding == MachineBinding::Unbound
                                && source.host_id == prior.host_id),
                        "请使用重新绑定机器操作恢复此目录"
                    );
                }
            }
            for agent in &mut normalized.agents {
                agent
                    .machine_ids
                    .retain(|id| !removed.iter().any(|host| &host.id == id));
            }
        }
        if !verified_auth_change {
            for account in &mut normalized.accounts {
                if previous.as_ref().is_some_and(|old| {
                    old.accounts.iter().any(|a| {
                        a.provider == account.provider
                            && a.id == account.id
                            && a.name != account.name
                    })
                }) {
                    account.pending_name = false;
                }
            }
        }
        anyhow::ensure!(
            normalized.configuration_issues.is_empty(),
            "{}",
            normalized.configuration_issues.join("；")
        );
        crate::codex_auth::validate_settings(&normalized)?;
        let _api_locations =
            crate::codex_api::check_settings(self, previous.as_ref(), &normalized)?;
        self.check_wakeup_settings(&normalized)?;
        let s = &normalized;
        let mut ids = std::collections::HashSet::new();
        for h in &s.hosts {
            anyhow::ensure!(
                ["ssh", "password"].contains(&h.auth_mode.as_str()),
                "请选择登录方式"
            );
            anyhow::ensure!(
                !h.username.starts_with('-') && !h.username.chars().any(char::is_whitespace),
                "用户名格式不正确"
            );
            if h.auth_mode == "password" {
                anyhow::ensure!(!h.password_ref.is_empty(), "请输入服务器密码");
                anyhow::ensure!(
                    !h.username.is_empty() || h.target.contains('@'),
                    "请输入服务器用户名"
                );
            }
            anyhow::ensure!(
                !h.id.is_empty() && h.id != "local" && ids.insert(format!("host:{}", h.id)),
                "主机 ID 重复或为空"
            );
            anyhow::ensure!(
                !h.target.is_empty()
                    && !h.target.starts_with('-')
                    && !h.target.chars().any(char::is_whitespace),
                "请输入 SSH 主机别名或 user@host"
            );
            anyhow::ensure!(
                h.shell.starts_with('/') && !h.shell.chars().any(char::is_whitespace),
                "请输入 shell 的绝对路径"
            );
        }
        for account in &s.accounts {
            anyhow::ensure!(
                account
                    .quota_refresh_seconds
                    .is_none_or(|n| (5..=86400).contains(&n)),
                "账户查询间隔范围为 5–86400 秒"
            );
            anyhow::ensure!(
                !account.id.trim().is_empty()
                    && ids.insert(format!("account:{}:{}", account.provider, account.id)),
                "账户标识重复或为空"
            );
            anyhow::ensure!(!account.name.trim().is_empty(), "请输入账户名称");
            let mut devices = std::collections::HashSet::new();
            for device in &account.device_settings {
                anyhow::ensure!(
                    !device.machine_id.is_empty() && devices.insert(&device.machine_id),
                    "账户设备设置重复或缺少设备"
                );
            }
            if let Some(id) = &account.quota_source_id {
                anyhow::ensure!(
                    s.sources.iter().any(|src| &src.id == id
                        && src.provider == account.provider
                        && account.uses_source(src)),
                    "额度查询位置需关联此账户"
                );
            }
        }
        for source in &s.sources {
            anyhow::ensure!(
                !source.id.is_empty() && ids.insert(format!("source:{}", source.id)),
                "数据源 ID 重复或为空"
            );
            anyhow::ensure!(
                source.account_id.is_empty()
                    || s.accounts
                        .iter()
                        .any(|a| a.id == source.account_id && a.provider == source.provider),
                "请选择同一 Agent 的账户，或选择无账户"
            );
            anyhow::ensure!(
                !crate::providers::requires_path(&source.provider)
                    || source.subscription.as_ref().is_some_and(|config| {
                        crate::subscriptions::native_cli(&source.provider, config)
                            || crate::subscriptions::endpoint(&source.provider, config).is_ok()
                    })
                    || !source.path.trim().is_empty(),
                "请输入数据目录"
            );
            anyhow::ensure!(
                crate::providers::known(&source.provider),
                "请选择 Agent 类型"
            );
            if let Some(id) = &source.host_id {
                anyhow::ensure!(
                    source.machine_binding == MachineBinding::Unbound
                        || s.hosts.iter().any(|h| &h.id == id),
                    "数据源关联的主机不存在"
                );
            }
            anyhow::ensure!(
                source.machine_binding != MachineBinding::Unbound
                    || (!source.enabled && source.host_id.is_some()),
                "未绑定机器的目录必须暂停并保留原机器标识"
            );
        }
        for proxy in std::iter::once(&s.proxy).chain(
            s.sources
                .iter()
                .filter(|s| s.host_id.is_none())
                .filter_map(|s| s.proxy.as_ref()),
        ) {
            anyhow::ensure!(
                ["system", "direct", "custom"].contains(&proxy.mode.as_str()),
                "请选择代理模式"
            );
            if proxy.mode == "custom" {
                let url = reqwest::Url::parse(&proxy.url).context("代理地址格式不正确")?;
                anyhow::ensure!(
                    ["http", "https", "socks5", "socks5h"].contains(&url.scheme()),
                    "代理支持 HTTP、HTTPS 与 SOCKS5"
                );
                anyhow::ensure!(
                    url.host_str().is_some_and(|host| !host.is_empty()),
                    "请输入代理 Host"
                );
                anyhow::ensure!(url.port() != Some(0), "请输入有效代理端口");
                anyhow::ensure!(
                    url.username().is_empty() && url.password().is_none(),
                    "请使用本机代理端口"
                );
            }
        }
        anyhow::ensure!(
            (5..=86400).contains(&s.refresh_seconds),
            "额度查询刷新间隔范围为 5–86400 秒"
        );
        anyhow::ensure!(
            (10..=86400).contains(&s.history_refresh_seconds),
            "Agent 记录刷新间隔范围为 10–86400 秒"
        );
        anyhow::ensure!(
            (2..=86400).contains(&s.server_foreground_refresh_seconds),
            "服务器前台刷新间隔范围为 2–86400 秒"
        );
        crate::network::validate_test_urls(&s.proxy_test_urls)?;
        anyhow::ensure!(
            (2..=86400).contains(&s.server_refresh_seconds),
            "服务器刷新间隔范围为 2–86400 秒"
        );
        let changed_models: Vec<String> = s
            .model_mappings
            .keys()
            .chain(previous.iter().flat_map(|old| old.model_mappings.keys()))
            .filter(|key| {
                previous
                    .as_ref()
                    .and_then(|old| old.model_mappings.get(*key))
                    != s.model_mappings.get(*key)
            })
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let obsolete: Vec<String> = previous
            .iter()
            .flat_map(|old| &old.hosts)
            .filter(|h| {
                !h.password_ref.is_empty()
                    && !s
                        .hosts
                        .iter()
                        .any(|next| next.password_ref == h.password_ref)
            })
            .map(|h| h.password_ref.clone())
            .collect();
        let transaction = if self.db.is_autocommit() {
            Some(self.write_transaction()?)
        } else {
            None
        };
        let tx = &self.db;
        if let Some(ref previous) = previous {
            // Archiving retains the namespace used to deduplicate imported records.
            // Protect history from destructive deletion by older clients as well.
            for account in &previous.accounts {
                if let Some(next) = s
                    .accounts
                    .iter()
                    .find(|a| a.id == account.id && a.provider == account.provider)
                {
                    anyhow::ensure!(
                        verified_auth_change
                            || account.profile_refs().is_empty()
                            || (account.quota_profile_id == next.quota_profile_id
                                && account.profile_refs() == next.profile_refs()),
                        "账户档案关联已改变，请重新读取配置并通过账户管理修改"
                    );
                }
                if !s
                    .accounts
                    .iter()
                    .any(|a| a.id == account.id && a.provider == account.provider)
                {
                    let has_history: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM events WHERE provider=?1 AND account_id=?2)",
                        params![account.provider, account.id],
                        |row| row.get(0),
                    )?;
                    let explicit_removal: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM kv WHERE key=?1)",
                        [format!(
                            "deleted-account:{}:{}",
                            account.provider, account.id
                        )],
                        |r| r.get(0),
                    )?;
                    anyhow::ensure!(
                        !has_history || (verified_auth_change && explicit_removal),
                        "该账户存在历史记录，请归档账户以保留历史"
                    );
                }
            }
            for source in &s.sources {
                if let Some(old) = previous.sources.iter().find(|old| old.id == source.id) {
                    if old.codex_home_id.is_some() {
                        anyhow::ensure!(
                            old.codex_home_id == source.codex_home_id
                                && old.path == source.path
                                && (old.host_id == source.host_id
                                    || (verified_auth_change
                                        && old.machine_binding == MachineBinding::Unbound))
                                && old.provider == source.provider,
                            "受管理 Codex 来源的 home 与路径不能通过普通配置覆盖，请使用账户管理"
                        );
                    }
                    if old.provider != source.provider {
                        tx.execute("DELETE FROM event_sources WHERE source_id=?1", [&source.id])?;
                        tx.execute("DELETE FROM events WHERE NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=events.id)", [])?;
                        tx.execute("DELETE FROM files WHERE source_id=?1", [&source.id])?;
                        tx.execute(
                            "DELETE FROM native_pending WHERE source_id=?1",
                            [&source.id],
                        )?;
                        tx.execute("DELETE FROM qoder_state WHERE source_id=?1", [&source.id])?;
                        tx.execute("DELETE FROM source_status WHERE source_id=?1", [&source.id])?;
                    } else {
                        if old.account_id != source.account_id {
                            // Attribution changes apply to new records only. Remember old
                            // namespaces so full SSH reads and file rescans retain identity.
                            tx.execute(
                                "INSERT OR IGNORE INTO source_identities VALUES(?1,?2,?3)",
                                params![old.id, old.provider, old.account_id],
                            )?;
                        }
                        if old.path != source.path
                            || old.host_id != source.host_id
                            || old.machine_binding != source.machine_binding
                        {
                            tx.execute("DELETE FROM files WHERE source_id=?1", [&source.id])?;
                            tx.execute(
                                "DELETE FROM native_pending WHERE source_id=?1",
                                [&source.id],
                            )?;
                            tx.execute(
                                "DELETE FROM remote_cursors WHERE source_id=?1",
                                [&source.id],
                            )?;
                            // Residual rows describe the old directory only.
                            tx.execute("DELETE FROM event_sources WHERE event_id IN (SELECT id FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual' AND json_extract(payload,'$.sourceId')=?1)", [&source.id])?;
                            tx.execute("DELETE FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual' AND json_extract(payload,'$.sourceId')=?1", [&source.id])?;
                            tx.execute("DELETE FROM qoder_state WHERE source_id=?1", [&source.id])?;
                            tx.execute(
                                "DELETE FROM source_status WHERE source_id=?1",
                                [&source.id],
                            )?;
                            self.pause_shared_source_estimates(&source.id)?;
                            self.invalidate_automatic_sources(std::slice::from_ref(&source.id))?;
                        }
                    }
                }
            }
        }
        tx.execute("DELETE FROM remote_cursors WHERE source_id NOT IN (SELECT json_extract(value,'$.id') FROM json_each(?1))", [serde_json::to_string(&s.sources)?])?;
        tx.execute("DELETE FROM native_pending WHERE source_id NOT IN (SELECT json_extract(value,'$.id') FROM json_each(?1))", [serde_json::to_string(&s.sources)?])?;
        if previous
            .as_ref()
            .is_some_and(|old| old.precise_counting != s.precise_counting)
        {
            tx.execute(
                "UPDATE kv SET value=CAST(value AS INTEGER)+1 WHERE key='counting.generation'",
                [],
            )?;
        }
        tx.execute("INSERT INTO kv VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(s)?])?;
        if !changed_models.is_empty() {
            self.reprice_in_transaction(s, true, Some(&changed_models))?;
        }
        self.mark_wakeup_settings_pending(s)?;
        self.reconcile_estimates(s)?;
        if let Some(previous) = &previous {
            self.reconcile_automatic_scopes(previous, s)?;
        }
        self.reconcile_reset_notifications(s)?;
        let owns_transaction = transaction.is_some();
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        for reference in obsolete.into_iter().filter(|_| owns_transaction) {
            let _ = crate::credentials::delete(&reference);
        }
        Ok(())
    }
    /// The migration only changes Aieyes records; target machines are synchronized explicitly.
    fn migrate_account_commands(&self) -> Result<()> {
        let migrated: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv WHERE key='account-commands.migrated.v10')",
            [],
            |row| row.get(0),
        )?;
        if migrated {
            return Ok(());
        }
        let tx = self.write_transaction()?;
        let raw: Option<String> = tx
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(raw) = raw {
            let mut settings: Settings = serde_json::from_str(&raw)?;
            crate::providers::normalize_settings(&mut settings);
            tx.execute(
                "UPDATE kv SET value=?1 WHERE key='settings'",
                [serde_json::to_string(&settings)?],
            )?;
            self.mark_wakeup_settings_pending(&settings)?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO kv VALUES('account-commands.migrated.v10','true')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    // Mark installed jobs dirty locally; network updates run on a separate lane.
    fn mark_wakeup_settings_pending(&self, s: &Settings) -> Result<()> {
        for mut record in self.wakeups()? {
            if let Some(deployed) = &record.deployment
                && deployed.account_environment_changed(s)
                && !matches!(
                    deployed.state.as_str(),
                    "pending-removal" | "settings-pending"
                )
            {
                record.deployment.as_mut().unwrap().state = "settings-pending".into();
                self.save_wakeup(&record)?;
            }
        }
        Ok(())
    }
    pub fn prices(&self) -> Result<Vec<ModelPrice>> {
        Ok(self.cached_prices()?.as_ref().clone())
    }
    pub(crate) fn cached_prices(&self) -> Result<std::sync::Arc<Vec<ModelPrice>>> {
        let revision = crate::runtime::revision(&self.db, "prices")?;
        let committed =
            self.db.transaction_state(None::<&str>)? != rusqlite::TransactionState::Write;
        if let Some((_, prices)) = self
            .prices_memory
            .borrow()
            .as_ref()
            .filter(|(known, _)| committed && *known == revision)
        {
            return Ok(prices.clone());
        }
        let mut query = self.db.prepare_cached("SELECT payload FROM price_history WHERE id IN (SELECT MAX(id) FROM price_history GROUP BY model_id)")?;
        let raw = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let prices = std::sync::Arc::new(
            raw.iter()
                .map(|s| Ok(serde_json::from_str(s)?))
                .collect::<Result<Vec<_>>>()?,
        );
        if committed && raw.iter().map(String::len).sum::<usize>() <= 4 * 1024 * 1024 {
            *self.prices_memory.borrow_mut() = Some((revision, prices.clone()));
        }
        Ok(prices)
    }
    pub fn save_prices(&self, prices: &[ModelPrice]) -> Result<()> {
        let tx = self.write_transaction()?;
        for p in prices {
            tx.execute(
                "INSERT INTO price_history(model_id,fetched_at,payload) VALUES(?1,?2,?3)",
                params![p.id, p.fetched_at, serde_json::to_string(p)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn put_event(
        &self,
        e: &UsageEvent,
        prices: &[ModelPrice],
        settings: &Settings,
    ) -> Result<bool> {
        self.put_event_inner(e, prices, settings, None)
    }
    pub(crate) fn put_counter_correction(
        &self,
        correction: &crate::codex_history::CounterCorrection,
        prices: &[ModelPrice],
        settings: &Settings,
    ) -> Result<bool> {
        if self.db.is_autocommit() {
            self.prepare_history_corrections()?;
        }
        self.put_event_inner(
            &correction.event,
            prices,
            settings,
            Some(&correction.history),
        )
    }
    fn put_event_inner(
        &self,
        e: &UsageEvent,
        prices: &[ModelPrice],
        settings: &Settings,
        correction: Option<&crate::codex_history::InheritedHistory>,
    ) -> Result<bool> {
        // Imports already own a transaction. Direct writes need the same atomicity
        // when an old identity resolves to more than one stored hash.
        let transaction = if self.db.is_autocommit() {
            Some(self.write_transaction()?)
        } else {
            None
        };
        let mut next = e.clone();
        next.provider = crate::providers::canonical(&e.provider).into();
        if e.provider == "agy" {
            let key = crate::providers::account_key(
                &format!("agy:{}", e.account_id),
                &settings.account_aliases,
            );
            next.account_id = key
                .strip_prefix("antigravity:")
                .unwrap_or(&e.account_id)
                .into();
        }
        if let Some(identity) = &e.import_identity {
            next.id =
                crate::usage::event_id(&next.provider, &next.source_id, &next.account_id, identity);
            let mut query = self.db.prepare("SELECT provider,account_id FROM source_identities WHERE source_id=?1 AND provider=?2 ORDER BY rowid")?;
            let scopes = query
                .query_map(params![e.source_id, next.provider], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Only reuse an event previously observed by this source. Unrelated
            // no-account sources and separate accounts keep independent identities.
            let mut candidates = crate::providers::usage_ids(
                &next.provider,
                &next.source_id,
                &next.account_id,
                identity,
                &settings.account_aliases,
            );
            'scopes: for (provider, account_id) in scopes {
                let ids = crate::providers::usage_ids(
                    &provider,
                    &e.source_id,
                    &account_id,
                    identity,
                    &settings.account_aliases,
                );
                for id in &ids {
                    let observed: bool = self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM events e JOIN event_sources s ON s.event_id=e.id WHERE e.id=?1 AND s.source_id=?2 AND e.provider=?3 AND e.account_id=?4)",
                        params![id, e.source_id, provider, account_id],
                        |row| row.get(0),
                    )?;
                    if observed {
                        next.account_id = account_id;
                        candidates = ids;
                        break 'scopes;
                    }
                }
            }
            // Replaying a stable identity proves that all matching hashes in this
            // namespace describe one event, even if their old payloads lack it.
            // Candidate order prefers the canonical row and its frozen price.
            let mut matched = Vec::new();
            for id in candidates {
                let old: Option<(String, Option<String>)> = self.db.query_row(
                    "SELECT payload,price FROM events WHERE id=?1 AND provider=?2 AND account_id=?3",
                    params![id, next.provider, next.account_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional()?;
                if let Some((payload, price)) = old {
                    matched.push((id, payload, price));
                }
            }
            if let Some((id, _, _)) = matched.first() {
                next.id = id.clone();
                for (duplicate_id, payload, price) in matched.iter().skip(1) {
                    let mut duplicate: UsageEvent = serde_json::from_str(payload)?;
                    duplicate.id = next.id.clone();
                    duplicate.import_identity = Some(identity.clone());
                    let frozen: Option<ModelPrice> = price
                        .as_ref()
                        .map(|raw| serde_json::from_str(raw))
                        .transpose()?;
                    self.merge_usage_event(duplicate, &e.source_id, frozen.as_slice(), settings)?;
                    self.db.execute(
                        "INSERT OR IGNORE INTO event_sources SELECT ?1,source_id FROM event_sources WHERE event_id=?2",
                        params![next.id, duplicate_id],
                    )?;
                    self.db.execute(
                        "DELETE FROM event_sources WHERE event_id=?1",
                        [duplicate_id],
                    )?;
                    self.db
                        .execute("DELETE FROM events WHERE id=?1", [duplicate_id])?;
                }
            }
        }
        if let Some(proof) = correction {
            self.save_counter_correction(&next, proof)?;
        }
        let inserted = self.merge_usage_event(next, &e.source_id, prices, settings)?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(inserted)
    }
    /// Persist a known canonical identity without re-attributing its namespace.
    pub(crate) fn merge_usage_event(
        &self,
        mut next: UsageEvent,
        observed_source: &str,
        prices: &[ModelPrice],
        settings: &Settings,
    ) -> Result<bool> {
        let old: Option<(String, Option<String>)> = self
            .db
            .query_row(
                "SELECT COALESCE(i.payload,e.payload),e.price FROM events e LEFT JOIN usage_inheritance i ON i.event_id=e.id WHERE e.id=?1",
                [&next.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let frozen: Option<ModelPrice> = old
            .as_ref()
            .and_then(|(_, p)| p.as_ref())
            .and_then(|p| serde_json::from_str(p).ok());
        let prior_model = old.as_ref().and_then(|(raw, _)| {
            serde_json::from_str::<UsageEvent>(raw)
                .ok()
                .map(|e| e.model)
        });
        if let Some((old, _)) = &old {
            let prior: UsageEvent = serde_json::from_str(old)?;
            match (&mut next.native_credits, prior.native_credits) {
                (Some(next), Some(prior)) => {
                    next.original_credits = next.original_credits.or(prior.original_credits);
                    next.billable = next.billable.or(prior.billable);
                    next.aliases.extend(prior.aliases);
                    next.aliases.sort();
                    next.aliases.dedup();
                    next.mirror_of = next.mirror_of.take().or(prior.mirror_of);
                }
                (next @ None, prior) => *next = prior,
                _ => {}
            }
            next.import_identity = next.import_identity.or(prior.import_identity);
            if next.provider == "codebuddy" && next.model == "unknown" && prior.model != "unknown" {
                next.model = prior.model.clone();
            }
            if crate::providers::antigravity(&next.provider)
                && crate::antigravity::unknown_model(&next.model)
                && !crate::antigravity::unknown_model(&prior.model)
            {
                next.model = prior.model.clone();
            }
            // A copied record with less metadata must not erase known API billing.
            if (prior.billing.category == "api"
                || next.billing.category.is_empty()
                || next.billing.category == "unknown")
                && !prior.billing.category.is_empty()
            {
                next.billing = prior.billing.clone();
            }
            let verified = next.interval_evidence == "task-request-v2";
            let prior_verified = prior.interval_evidence == "task-request-v2";
            if prior_verified && !verified {
                next.interval_start = prior.interval_start;
                next.interval_evidence = prior.interval_evidence;
            } else if !verified || prior_verified {
                next.interval_start = match (next.interval_start, prior.interval_start) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            next.account_id = prior.account_id;
            next.timestamp = next.timestamp.min(prior.timestamp);
            next.source_id = prior.source_id;
            // Parser 9 splits Qoder's inclusive first-party prompt count into
            // ordinary input plus cached input. A v8 row still has input == raw
            // input, so the normal max would preserve the old double count.
            let qoder_legacy_inclusive = next.provider == "qoder"
                && next.raw_tokens.as_ref().is_some_and(|raw| {
                    prior.raw_tokens.is_none() && prior.tokens.input == raw.input
                });
            if next.usage_version > 0 || prior.usage_version > 0 {
                // Native snapshots are indivisible. A parser upgrade can
                // reclassify cached input or correct a model, including downwards.
                if next.usage_version < prior.usage_version
                    || (next.usage_version == prior.usage_version
                        && next.tokens.total() < prior.tokens.total())
                {
                    next.tokens = prior.tokens;
                    next.model = prior.model;
                    next.usage_version = prior.usage_version;
                }
            } else if !qoder_legacy_inclusive {
                next.tokens.input = next.tokens.input.max(prior.tokens.input);
                next.tokens.output = next.tokens.output.max(prior.tokens.output);
                next.tokens.cache_read = next.tokens.cache_read.max(prior.tokens.cache_read);
                next.tokens.cache_write = next.tokens.cache_write.max(prior.tokens.cache_write);
                next.tokens.reasoning = next.tokens.reasoning.max(prior.tokens.reasoning);
            }
            next.raw_tokens = next.raw_tokens.take().or(prior.raw_tokens.clone());
            next.inheritance = next.inheritance.take().or(prior.inheritance);
        }
        // A proved counter correction is authoritative even when an older parser
        // or a second copy later supplies the original, larger cumulative value.
        if let Some(corrected) = self.corrected_usage(&next.id)? {
            next.tokens = corrected.tokens;
            next.attribution = corrected.attribution;
            next.interval_start = corrected.interval_start;
            next.interval_evidence = corrected.interval_evidence;
            // An inherited cumulative summary is not proof of one request's billing route.
            if next.attribution == "session-summary" {
                next.billing = corrected.billing;
            }
            if next.attribution == "unresolved-inheritance" {
                next.billing = BillingEvidence::default();
            }
        }
        crate::native_credits::resolve(self, &mut next)?;
        let price = frozen
            .as_ref()
            .filter(|price| {
                prior_model.as_deref() == Some(next.model.as_str())
                    || pricing::find_price(
                        &next.model,
                        std::slice::from_ref(*price),
                        &settings.model_mappings,
                    )
                    .is_some()
            })
            .or_else(|| pricing::find_price(&next.model, prices, &settings.model_mappings));
        crate::history_counting::observe(self, &mut next, price, settings)?;
        let (cost, covered) = pricing::estimate(&next.tokens, price);
        self.db.execute("INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET model=excluded.model,stamp=excluded.stamp,payload=excluded.payload,cost=excluded.cost,priced_tokens=excluded.priced_tokens,price=excluded.price",
            params![next.id,next.provider,next.account_id,next.model,next.timestamp,serde_json::to_string(&next)?,cost,covered,price.map(serde_json::to_string).transpose()?])?;
        self.db.execute(
            "INSERT OR IGNORE INTO event_sources VALUES(?1,?2)",
            params![next.id, observed_source],
        )?;
        crate::native_credits::persisted(self, &next)?;
        if let Some(identity) = &next.import_identity
            && next.id
                != crate::usage::event_id(
                    &next.provider,
                    &next.source_id,
                    &next.account_id,
                    identity,
                )
        {
            // Re-import supplied a previously missing identity; re-key on next open.
            self.db
                .execute("DELETE FROM kv WHERE key='usage.ids.canonical.v1'", [])?;
        }
        Ok(old.is_none()
            && (next.inheritance.is_none()
                || next.tokens.total() > 0
                || next
                    .native_credits
                    .as_ref()
                    .is_some_and(|c| c.valid() && c.mirrored_by.is_none())))
    }
    pub fn reprice(&self, settings: &Settings, only_unpriced: bool) -> Result<u64> {
        let tx = self.write_transaction()?;
        let count = self.reprice_in_transaction(settings, only_unpriced, None)?;
        tx.commit()?;
        Ok(count)
    }
    fn reprice_in_transaction(
        &self,
        settings: &Settings,
        only_unpriced: bool,
        models: Option<&[String]>,
    ) -> Result<u64> {
        let prices = self.prices()?;
        let mut query = self.db.prepare("SELECT id,payload,price,priced_tokens FROM events WHERE (?1 IS NULL OR id>?1) AND (?2 IS NULL OR model IN (SELECT value FROM json_each(?2))) ORDER BY id LIMIT 512")?;
        let selected = models.map(serde_json::to_string).transpose()?;
        let mut after: Option<String> = None;
        let mut count = 0;
        loop {
            let rows = query
                .query_map(params![after, selected], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, u64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            after = rows.last().map(|r| r.0.clone());
            for (id, raw, snapshot, covered) in rows {
                let e: UsageEvent = serde_json::from_str(&raw)?;
                if only_unpriced && covered >= e.tokens.total() {
                    continue;
                }
                if let Some(current) =
                    pricing::find_price(&e.model, &prices, &settings.model_mappings)
                {
                    let mut p = current.clone();
                    if only_unpriced && let Some(raw) = snapshot {
                        let old: ModelPrice = serde_json::from_str(&raw)?;
                        p.input = old.input.or(p.input);
                        p.output = old.output.or(p.output);
                        p.cache_read = old.cache_read.or(p.cache_read);
                        p.cache_write = old.cache_write.or(p.cache_write);
                    }
                    let (c, n) = pricing::estimate(&e.tokens, Some(&p));
                    self.db.execute(
                        "UPDATE events SET cost=?1,priced_tokens=?2,price=?3 WHERE id=?4",
                        params![c, n, serde_json::to_string(&p)?, id],
                    )?;
                    count += 1;
                } else if !only_unpriced {
                    self.db.execute(
                        "UPDATE events SET cost=0,priced_tokens=0,price=NULL WHERE id=?1",
                        [&id],
                    )?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }
    pub fn cursor(&self, source: &str, path: &str) -> Result<Option<(String, u64, ParseState)>> {
        let row: Option<(String, u64, String)> = self
            .db
            .query_row(
                "SELECT signature,offset,state FROM files WHERE source_id=?1 AND path=?2",
                params![source, path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(s, o, p)| Ok((s, o, serde_json::from_str(&p)?)))
            .transpose()
    }
    pub fn set_cursor(
        &self,
        source: &str,
        path: &str,
        signature: &str,
        offset: u64,
        state: &ParseState,
    ) -> Result<()> {
        self.db.execute("INSERT INTO files VALUES(?1,?2,?3,?4,?5) ON CONFLICT(source_id,path) DO UPDATE SET signature=excluded.signature,offset=excluded.offset,state=excluded.state",
            params![source,path,signature,offset,serde_json::to_string(state)?])?;
        Ok(())
    }
    pub fn quota(&self, q: &QuotaSnapshot) -> Result<()> {
        if q.account_id.is_empty()
            || (q.error.is_none()
                && q.windows.is_empty()
                && q.balances.is_empty()
                && q.extra_usage.is_none()
                && q.bank_reset.is_none())
        {
            return Ok(());
        }
        let key = format!("{}:{}", q.provider, q.account_id);
        let prior: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM quotas WHERE account_key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        // Shared in-flight reads can be delivered to more than one caller.
        if prior.as_deref() == Some(serde_json::to_string(q)?.as_str()) {
            return Ok(());
        }
        let mut next = q.clone();
        if q.provider == "antigravity" && q.error.is_none() {
            let settings = self.settings()?;
            if let Some(identity) = &q.identity {
                let account = settings
                    .accounts
                    .iter()
                    .find(|a| a.provider == q.provider && a.id == q.account_id);
                anyhow::ensure!(
                    crate::agy_identity::matches(account, identity),
                    "账户身份已变化，额度结果已丢弃"
                );
            }
        }
        if let Some(raw) = prior {
            let old: QuotaSnapshot = serde_json::from_str(&raw)?;
            if q.error.is_none() && q.provider == "antigravity" {
                // A metadata failure must not throw away newly fetched quota windows.
                // Retain a failed subscription query only for a confirmed matching identity.
                if let (Some(new_id), Some(old_id)) = (&mut next.identity, &old.identity)
                    && new_id.key == old_id.key
                    && new_id.stale
                    && new_id.subscription.is_none()
                {
                    new_id.subscription = old_id.subscription.clone();
                    new_id.subscription_checked_at = old_id.subscription_checked_at;
                    next.plan = new_id.subscription.clone();
                }
            }
            if q.error.is_some() {
                next = old;
                next.error = q.error.clone();
            } else if old.updated_at > q.updated_at {
                return Ok(());
            } else if q.origin == "log" {
                if old.bank_reset.is_some() {
                    next.bank_reset = old.bank_reset;
                    next.bank_updated_at = old.bank_updated_at.or(Some(old.updated_at));
                }
                if old.credits.is_some() && old.credits_origin.as_deref() != Some("log") {
                    next.credits = old.credits;
                    next.credits_origin = old.credits_origin;
                    next.credits_updated_at = old.credits_updated_at.or(Some(old.updated_at));
                }
            }
        }
        let observed = if q.provider == "antigravity" {
            &next
        } else {
            q
        };
        self.learn_capacity(observed)?;
        self.observe_estimates(observed)?;
        if q.error.is_none() && q.origin != "log" {
            self.db.execute(
                "INSERT INTO quota_history(account_key,stamp,payload) VALUES(?1,?2,?3)",
                params![key, q.updated_at, serde_json::to_string(observed)?],
            )?;
        }
        self.db.execute("INSERT INTO quotas VALUES(?1,?2) ON CONFLICT(account_key) DO UPDATE SET payload=excluded.payload",params![key,serde_json::to_string(&next)?])?;
        Ok(())
    }
    pub fn source_status(&self, id: &str, status: &Value) -> Result<()> {
        let mut next = status.clone();
        if status.get("error").is_some() {
            let previous: Option<String> = self
                .db
                .query_row(
                    "SELECT payload FROM source_status WHERE source_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                let mut old: Value = serde_json::from_str(&previous)?;
                if let (Some(old), Some(new)) = (old.as_object_mut(), status.as_object()) {
                    old.extend(new.clone());
                    next = Value::Object(old.clone());
                }
            }
        }
        self.db.execute("INSERT INTO source_status VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET payload=excluded.payload",params![id,next.to_string()])?;
        Ok(())
    }
    pub fn dashboard(&self, f: &Filter) -> Result<Dashboard> {
        self.dashboard_mode(f, false)
    }
    pub fn dashboard_summary(&self, f: &Filter) -> Result<Dashboard> {
        self.dashboard_mode(f, true)
    }
    fn dashboard_mode(&self, f: &Filter, summary: bool) -> Result<Dashboard> {
        // Initialize discovery and derived samples before opening a read snapshot.
        self.settings()?;
        self.project_automatic_samples()?;
        let transaction = if self.db.is_autocommit() {
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        let settings = self.settings()?;
        let (mut d, pending) = crate::usage_cache::read(self, f, summary)?;
        self.fill_dashboard_status(&mut d, f, &settings, !summary)?;
        if summary {
            d.days.clear();
            d.heatmap.clear();
            d.trend_days.clear();
            d.day_models.clear();
            d.models.clear();
            d.model_options.clear();
            d.pricing_gaps.clear();
        }
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        let _ = crate::usage_cache::persist(self, pending);
        Ok(d)
    }
    pub fn dashboard_status(&self, f: &Filter) -> Result<Value> {
        self.settings()?;
        self.project_automatic_samples()?;
        let transaction = if self.db.is_autocommit() {
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        let mut filter = f.clone();
        filter.estimate_history = Some("display".into());
        let mut dashboard = Dashboard {
            generated_at: now(),
            ..Default::default()
        };
        self.fill_dashboard_status(&mut dashboard, &filter, &self.settings()?, true)?;
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(
            json!({"generatedAt":dashboard.generated_at,"quotas":dashboard.quotas,"quotaOrder":dashboard.quota_order,
            "quotaEstimates":dashboard.quota_estimates,"creditEstimates":dashboard.credit_estimates,"automaticEstimateSummaries":dashboard.automatic_estimate_summaries,
            "sources":dashboard.sources,"priceUpdatedAt":dashboard.price_updated_at}),
        )
    }
    fn fill_dashboard_status(
        &self,
        d: &mut Dashboard,
        f: &Filter,
        settings: &Settings,
        include_sources: bool,
    ) -> Result<()> {
        for account in settings
            .accounts
            .iter()
            .filter(|a| a.quota_enabled && !a.archived)
        {
            if !f.includes_provider(&account.provider)
                || f.account_id.as_ref().is_some_and(|id| id != &account.id)
            {
                continue;
            }
            let linked: Vec<_> = settings
                .sources
                .iter()
                .filter(|s| s.enabled && account.uses_source(s))
                .collect();
            if linked.is_empty() {
                continue;
            }
            if f.source_id
                .as_ref()
                .is_some_and(|id| !linked.iter().any(|s| &s.id == id))
            {
                continue;
            }
            let raw: Option<String> = self
                .db
                .query_row(
                    "SELECT payload FROM quotas WHERE account_key=?1",
                    [format!("{}:{}", account.provider, account.id)],
                    |r| r.get(0),
                )
                .optional()?;
            let mut quota: QuotaSnapshot = raw
                .map(|r| serde_json::from_str(&r))
                .transpose()?
                .unwrap_or_else(|| QuotaSnapshot {
                    provider: account.provider.clone(),
                    account_id: account.id.clone(),
                    error: Some(
                        if linked.is_empty() {
                            "关联数据源后可读取额度"
                        } else {
                            "尚未读取额度"
                        }
                        .into(),
                    ),
                    ..Default::default()
                });
            quota.name = account.name.clone();
            if quota.error.as_deref() == Some("设置额度查询命令") {
                quota.error = Some("请刷新额度；远程代理可在数据源中设置".into());
            }
            d.quotas.push(quota);
        }
        d.automatic_estimate_summaries = self
            .automatic_estimate_summaries(now())?
            .into_iter()
            .filter(|s| {
                d.quotas.iter().any(|q| {
                    let plan = q
                        .plan
                        .as_deref()
                        .map(str::trim)
                        .map(str::to_ascii_lowercase);
                    s.estimate.account_key == format!("{}:{}", q.provider, q.account_id)
                        && plan.as_deref()
                            == s.estimate
                                .quota()
                                .and_then(|v| v.quota_plan.as_deref())
                                .or_else(|| {
                                    s.estimate.credit().and_then(|v| v.quota_plan.as_deref())
                                })
                })
            })
            .collect();
        d.quota_order = self.quota_order()?;
        let samples = if f.estimate_history.as_deref() == Some("display") {
            self.display_estimates()?
        } else {
            self.estimates()?
        };
        d.credit_estimates = samples
            .iter()
            .filter(|e| matches!(e, crate::estimates::Estimate::Credit(_)))
            .cloned()
            .collect();
        d.quota_estimates = samples
            .into_iter()
            .filter(|e| !matches!(e, crate::estimates::Estimate::Credit(_)))
            .collect();
        d.quotas.sort_by(|a, b| {
            let rank = |q: &QuotaSnapshot| {
                d.quota_order
                    .iter()
                    .position(|k| k == &format!("{}:{}", q.provider, q.account_id))
                    .unwrap_or(usize::MAX)
            };
            rank(a).cmp(&rank(b)).then_with(|| {
                a.windows
                    .is_empty()
                    .cmp(&b.windows.is_empty())
                    .then(a.name.cmp(&b.name))
            })
        });
        if include_sources {
            for source in &settings.sources {
                let status: Option<String> = self
                    .db
                    .query_row(
                        "SELECT payload FROM source_status WHERE source_id=?1",
                        [&source.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut query = self.db.prepare("SELECT account_id FROM usage_source_accounts WHERE source_id=?1 AND provider=?2 AND event_count>0 ORDER BY account_id")?;
                let mut account_ids = query
                    .query_map(params![source.id, source.provider], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if source.codex_home_id.is_some() {
                    account_ids.clear();
                }
                if !account_ids.contains(&source.account_id) {
                    account_ids.push(source.account_id.clone());
                }
                let status = status
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .map(|value| {
                        crate::codex_history::sync_status(value, settings.precise_counting)
                    });
                d.sources.push(json!({"id":source.id,"name":source.name,"provider":source.provider,"enabled":settings.source_enabled(source),"accountId":source.account_id,"accountIds":account_ids,"codexHomeId":source.codex_home_id,"hostId":source.host_id,"status":status,"autoValuation":source.auto_valuation,"autoValuationStatus":self.automatic_status(&source.id)?}));
            }
        }
        d.price_updated_at =
            self.db
                .query_row("SELECT MAX(fetched_at) FROM price_history", [], |r| {
                    r.get(0)
                })?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn dashboard_usage_reference(&self, f: &Filter) -> Result<Dashboard> {
        use chrono::{Duration, Local, TimeZone};
        let crate::usage_cache::DateBounds {
            start,
            end: today,
            trend_start,
            heat_start,
        } = crate::usage_cache::date_bounds(self, f, false)?;
        let until = now().min(crate::usage_cache::boundary(today + Duration::days(1)) - 1);
        let earliest = start.min(heat_start);
        let from = Local
            .from_local_datetime(&earliest.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|v| v.timestamp())
            .unwrap_or(now() - 366 * 86400);
        let settings = self.settings()?;
        let shared = serde_json::to_string(
            &settings
                .sources
                .iter()
                .filter(|s| s.codex_home_id.is_some())
                .map(|s| &s.id)
                .collect::<Vec<_>>(),
        )?;
        let mut query=self.db.prepare("SELECT payload,cost,priced_tokens,price FROM events e WHERE stamp>=?1 AND stamp<=?2 AND (?3 IS NULL OR provider=?3 OR provider=?8) AND (?4 IS NULL OR (?4='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7)))) OR (account_id=?4 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7))))) AND (?5 IS NULL OR model=?5) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?6)) ORDER BY stamp")?;
        let mut rows = query.query(params![
            from,
            until,
            f.provider,
            f.account_id,
            f.model,
            f.source_id,
            shared,
            f.provider_alias()
        ])?;
        let mut d = Dashboard {
            generated_at: now(),
            ..Default::default()
        };
        let options_from = Local
            .from_local_datetime(&trend_start.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|v| v.timestamp())
            .unwrap_or(from);
        let mut options = self.db.prepare(
            "SELECT payload FROM events e WHERE stamp>=?1 AND stamp<=?2
             AND (?3 IS NULL OR provider=?3 OR provider=?7) AND (?4 IS NULL OR (?4='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6)))) OR (account_id=?4 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6)))))
             AND (?5 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?5))
             ORDER BY model",
        )?;
        let option_rows = options
            .query_map(
                params![
                    options_from,
                    until,
                    f.provider,
                    f.account_id,
                    f.source_id,
                    shared,
                    f.provider_alias()
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut choices = std::collections::BTreeSet::new();
        for raw in option_rows {
            let mut event: UsageEvent = serde_json::from_str(&raw)?;
            if crate::native_credits::prepare_view(self, &mut event, f.source_id.as_deref())? {
                choices.insert(event.model);
            }
        }
        d.model_options = choices.into_iter().collect();
        let mut days = BTreeMap::<String, Aggregate>::new();
        let mut heat = BTreeMap::<String, Aggregate>::new();
        let mut models = BTreeMap::<String, Aggregate>::new();
        let mut trend = BTreeMap::<String, Aggregate>::new();
        let mut day_models = BTreeMap::<(String, String), Aggregate>::new();
        let mut gaps = BTreeMap::<String, PricingGap>::new();
        let settings = self.settings()?;
        let prices = self.prices()?;
        while let Some(row) = rows.next()? {
            let mut e: UsageEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
            if !crate::native_credits::prepare_view(self, &mut e, f.source_id.as_deref())? {
                continue;
            }
            let cost: f64 = row.get(1)?;
            let n: u64 = row.get(2)?;
            let Some(date) = Local
                .timestamp_opt(e.timestamp, 0)
                .single()
                .map(|v| v.date_naive())
            else {
                continue;
            };
            let key = date.to_string();
            if date >= heat_start {
                add_aggregate(
                    heat.entry(key.clone()).or_insert_with(|| Aggregate {
                        key: key.clone(),
                        ..Default::default()
                    }),
                    &e,
                    cost,
                    n,
                );
            }
            if date >= trend_start {
                add_aggregate(
                    trend.entry(key.clone()).or_insert_with(|| Aggregate {
                        key: key.clone(),
                        ..Default::default()
                    }),
                    &e,
                    cost,
                    n,
                );
                add_aggregate(
                    day_models
                        .entry((key.clone(), e.model.clone()))
                        .or_insert_with(|| Aggregate {
                            key: e.model.clone(),
                            ..Default::default()
                        }),
                    &e,
                    cost,
                    n,
                );
            }
            if date >= start {
                if n < e.tokens.total() {
                    let snapshot: Option<String> = row.get(3)?;
                    let price: Option<ModelPrice> =
                        snapshot.map(|s| serde_json::from_str(&s)).transpose()?;
                    let gap = gaps.entry(e.model.clone()).or_insert_with(|| PricingGap {
                        model: e.model.clone(),
                        price_id: pricing::find_price(&e.model, &prices, &settings.model_mappings)
                            .map(|p| p.id.clone()),
                        ..Default::default()
                    });
                    gap.tokens
                        .add(&pricing::missing_tokens(&e.tokens, price.as_ref()));
                    gap.unpriced_tokens += e.tokens.total() - n;
                }
                add_aggregate(&mut d.summary, &e, cost, n);
                add_aggregate(
                    days.entry(key.clone()).or_insert_with(|| Aggregate {
                        key,
                        ..Default::default()
                    }),
                    &e,
                    cost,
                    n,
                );
                add_aggregate(
                    models.entry(e.model.clone()).or_insert_with(|| Aggregate {
                        key: e.model.clone(),
                        ..Default::default()
                    }),
                    &e,
                    cost,
                    n,
                );
            }
        }
        d.trend_days = (0..=(today - trend_start).num_days())
            .map(|n| {
                let key = (trend_start + Duration::days(n)).to_string();
                trend.remove(&key).unwrap_or(Aggregate {
                    key,
                    ..Default::default()
                })
            })
            .collect();
        d.day_models = day_models
            .into_iter()
            .map(|((day, model), usage)| DayModel { day, model, usage })
            .collect();
        d.pricing_gaps = gaps.into_values().collect();
        d.days = (0..=(today - start).num_days())
            .map(|n| {
                let key = (start + Duration::days(n)).to_string();
                days.remove(&key).unwrap_or(Aggregate {
                    key,
                    ..Default::default()
                })
            })
            .collect();
        d.heatmap = (0..=(today - heat_start).num_days())
            .map(|n| {
                let key = (heat_start + Duration::days(n)).to_string();
                heat.remove(&key).unwrap_or(Aggregate {
                    key,
                    ..Default::default()
                })
            })
            .collect();
        d.models = models.into_values().collect();
        d.models.sort_by_key(|a| std::cmp::Reverse(a.total));
        Ok(d)
    }
}
#[cfg(test)]
fn add_aggregate(a: &mut Aggregate, e: &UsageEvent, cost: f64, n: u64) {
    a.add_native_credits(e);
    a.tokens.add(&e.tokens);
    a.total = a.tokens.total();
    a.cost += cost;
    a.priced_tokens += n;
    if e.attribution != "qoder-residual" {
        a.events += 1;
    }
}
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        home()
    } else if let Some(p) = path.strip_prefix("~/").or_else(|| {
        if cfg!(windows) {
            path.strip_prefix("~\\")
        } else {
            None
        }
    }) {
        home().join(p)
    } else {
        PathBuf::from(path)
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn home_paths_keep_platform_separators_and_literal_names() {
        assert_eq!(expand("~"), home());
        assert_eq!(
            expand("~/folder name/目录"),
            home().join("folder name/目录")
        );
        #[cfg(windows)]
        assert_eq!(
            expand(r"~\folder name\目录"),
            home().join(r"folder name\目录")
        );
        #[cfg(not(windows))]
        assert_eq!(expand(r"~\folder"), PathBuf::from(r"~\folder"));
        for literal in ["~other/folder", "$HOME/folder", "relative folder", ""] {
            assert_eq!(expand(literal), PathBuf::from(literal));
        }
    }
}
