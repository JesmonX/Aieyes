use crate::{models::*, pricing};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub struct Store {
    pub db: Connection,
    pub(crate) usage_memory: std::cell::RefCell<crate::usage_cache::Memory>,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let db = Connection::open(root.join("aieyes.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;
        CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY,provider TEXT NOT NULL,account_id TEXT NOT NULL,model TEXT NOT NULL,stamp INTEGER NOT NULL,payload TEXT NOT NULL,cost REAL NOT NULL,priced_tokens INTEGER NOT NULL,price TEXT);
        CREATE INDEX IF NOT EXISTS events_stamp ON events(stamp);
        CREATE INDEX IF NOT EXISTS events_model ON events(model);
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
        CREATE TABLE IF NOT EXISTS quota_estimates(id TEXT PRIMARY KEY,account_key TEXT NOT NULL,status TEXT NOT NULL,payload TEXT NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS estimate_active ON quota_estimates(account_key) WHERE status IN ('active','pending');
        CREATE TABLE IF NOT EXISTS credit_estimates(id TEXT PRIMARY KEY,account_key TEXT NOT NULL,status TEXT NOT NULL,payload TEXT NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS credit_estimate_active ON credit_estimates(account_key) WHERE status IN ('active','pending');
        CREATE TABLE IF NOT EXISTS capacity_samples(account_key TEXT NOT NULL,stamp INTEGER NOT NULL,five REAL NOT NULL,week REAL NOT NULL,PRIMARY KEY(account_key,stamp));
        ")?;
        crate::usage_cache::setup(&db)?;
        crate::remote_history::setup(&db)?;
        let store = Self {
            db,
            usage_memory: Default::default(),
        };
        store.migrate_antigravity()?;
        store.bootstrap_capacity()?;
        Ok(store)
    }
    pub fn settings(&self) -> Result<Settings> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        match raw {
            Some(s) => {
                let mut settings: Settings = serde_json::from_str(&s)?;
                crate::providers::normalize_settings(&mut settings);
                let mut stmt = self
                    .db
                    .prepare("SELECT value FROM kv WHERE key LIKE 'deleted-account:%'")?;
                let values = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                settings.deleted_accounts = values
                    .iter()
                    .map(|raw| {
                        let mut account: Account = serde_json::from_str(raw)?;
                        account.archived = true;
                        account.quota_enabled = false;
                        Ok(account)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(settings)
            }
            None => {
                let mut settings = Settings::default();
                for (provider, name, dir) in [
                    ("codex", "Codex · 本机", ".codex"),
                    ("claude", "Claude Code · 本机", ".claude"),
                    (
                        "antigravity",
                        "Antigravity · 本机",
                        ".gemini/antigravity-cli",
                    ),
                ] {
                    let root = home().join(dir);
                    if root.is_dir() {
                        settings.sources.push(Source {
                            id: format!("local-{provider}"),
                            name: name.into(),
                            provider: provider.into(),
                            account_id: local_account_id(provider, &root),
                            path: root.to_string_lossy().into(),
                            ..Default::default()
                        });
                    }
                }
                crate::providers::normalize_settings(&mut settings);
                self.save_settings(&settings)?;
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
        crate::codex_auth::validate_settings(&normalized)?;
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
                    .is_none_or(|n| (30..=86400).contains(&n)),
                "账户查询间隔范围为 30–86400 秒"
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
                    "限额查询位置需关联此账户"
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
                ["antigravity", "agy", "deepseek"].contains(&source.provider.as_str())
                    || !source.path.trim().is_empty(),
                "请输入数据目录"
            );
            anyhow::ensure!(
                [
                    "codex",
                    "claude",
                    "antigravity",
                    "agy",
                    "deepseek",
                    "custom"
                ]
                .contains(&source.provider.as_str()),
                "请选择 Agent 类型"
            );
            if let Some(id) = &source.host_id {
                anyhow::ensure!(
                    s.hosts.iter().any(|h| &h.id == id),
                    "数据源关联的主机不存在"
                );
            }
        }
        for proxy in
            std::iter::once(&s.proxy).chain(s.sources.iter().filter_map(|s| s.proxy.as_ref()))
        {
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
            (10..=86400).contains(&s.refresh_seconds),
            "Agent 限额刷新间隔范围为 10–86400 秒"
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
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        let tx = &self.db;
        if let Some(previous) = previous {
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
                        "账号档案关联已改变，请重新读取配置并通过账号管理修改"
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
                                && old.host_id == source.host_id
                                && old.provider == source.provider,
                            "受管理 Codex 来源的 home 与路径不能通过普通配置覆盖，请使用账号管理"
                        );
                    }
                    if old.provider != source.provider {
                        tx.execute("DELETE FROM event_sources WHERE source_id=?1", [&source.id])?;
                        tx.execute("DELETE FROM events WHERE NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=events.id)", [])?;
                        tx.execute("DELETE FROM files WHERE source_id=?1", [&source.id])?;
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
                        if old.path != source.path || old.host_id != source.host_id {
                            tx.execute("DELETE FROM files WHERE source_id=?1", [&source.id])?;
                        }
                    }
                }
            }
        }
        tx.execute("DELETE FROM remote_cursors WHERE source_id NOT IN (SELECT json_extract(value,'$.id') FROM json_each(?1))", [serde_json::to_string(&s.sources)?])?;
        tx.execute("INSERT INTO kv VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(s)?])?;
        if !changed_models.is_empty() {
            self.reprice_in_transaction(s, true, Some(&changed_models))?;
        }
        // Mark installed jobs dirty locally; network updates run on a separate lane.
        for mut record in self.wakeups()? {
            if let Some(deployed) = &record.deployment
                && let Some(source) = s.sources.iter().find(|v| v.id == deployed.source.id)
            {
                let current_account = s.accounts.iter().find(|a| {
                    a.provider == deployed.source.provider && a.id == deployed.source.account_id
                });
                let previous_command = crate::accounts::pre_command(
                    &Settings {
                        hosts: deployed.host.clone().into_iter().collect(),
                        ..Default::default()
                    },
                    None,
                    &deployed.source,
                );
                if crate::accounts::pre_command(s, current_account, source) != previous_command
                    && deployed.state != "pending-removal"
                {
                    record.deployment.as_mut().unwrap().state = "settings-pending".into();
                    self.save_wakeup(&record)?;
                }
            }
        }
        self.reconcile_estimates(s)?;
        let owns_transaction = transaction.is_some();
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        for reference in obsolete.into_iter().filter(|_| owns_transaction) {
            let _ = crate::credentials::delete(&reference);
        }
        Ok(())
    }
    pub fn prices(&self) -> Result<Vec<ModelPrice>> {
        let mut query=self.db.prepare("SELECT payload FROM price_history WHERE id IN (SELECT MAX(id) FROM price_history GROUP BY model_id)")?;
        let raw = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raw.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn save_prices(&self, prices: &[ModelPrice]) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
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
        let mut next = e.clone();
        if let Some(identity) = &e.import_identity {
            let mut query = self.db.prepare("SELECT provider,account_id FROM source_identities WHERE source_id=?1 AND (provider=?2 OR (?2='antigravity' AND provider='agy')) ORDER BY rowid")?;
            let scopes = query
                .query_map(params![e.source_id, e.provider], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Only reuse an event previously observed by this source. Unrelated
            // no-account sources and separate accounts keep independent identities.
            for (provider, account_id) in scopes {
                let id = crate::usage::event_id(&provider, &e.source_id, &account_id, identity);
                let observed: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM event_sources WHERE event_id=?1 AND source_id=?2)",
                    params![id, e.source_id],
                    |row| row.get(0),
                )?;
                if observed {
                    next.id = id;
                    break;
                }
            }
        }
        // A newly added copy of a migrated account also uses its legacy hash namespace.
        if next.id == e.id
            && crate::providers::antigravity(&e.provider)
            && !e.account_id.is_empty()
            && let Some(identity) = &e.import_identity
        {
            let canonical_key = format!("antigravity:{}", e.account_id);
            for (legacy, canonical) in &settings.account_aliases {
                if canonical != &canonical_key {
                    continue;
                }
                let Some(account) = legacy.strip_prefix("agy:") else {
                    continue;
                };
                let id = crate::usage::event_id("agy", &e.source_id, account, identity);
                let exists: bool = self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1 AND provider='antigravity' AND account_id=?2)",
                        params![id, e.account_id], |r| r.get(0))?;
                if exists {
                    next.id = id;
                    break;
                }
            }
        }
        let old: Option<(String, Option<String>)> = self
            .db
            .query_row(
                "SELECT payload,price FROM events WHERE id=?1",
                [&next.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let frozen: Option<ModelPrice> = old
            .as_ref()
            .and_then(|(_, p)| p.as_ref())
            .and_then(|p| serde_json::from_str(p).ok());
        if let Some((old, _)) = &old {
            let prior: UsageEvent = serde_json::from_str(old)?;
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
            next.tokens.input = next.tokens.input.max(prior.tokens.input);
            next.tokens.output = next.tokens.output.max(prior.tokens.output);
            next.tokens.cache_read = next.tokens.cache_read.max(prior.tokens.cache_read);
            next.tokens.cache_write = next.tokens.cache_write.max(prior.tokens.cache_write);
            next.tokens.reasoning = next.tokens.reasoning.max(prior.tokens.reasoning);
        }
        let price = frozen
            .as_ref()
            .or_else(|| pricing::find_price(&next.model, prices, &settings.model_mappings));
        let (cost, covered) = pricing::estimate(&next.tokens, price);
        self.db.execute("INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET model=excluded.model,stamp=excluded.stamp,payload=excluded.payload,cost=excluded.cost,priced_tokens=excluded.priced_tokens,price=excluded.price",
            params![next.id,next.provider,next.account_id,next.model,next.timestamp,serde_json::to_string(&next)?,cost,covered,price.map(serde_json::to_string).transpose()?])?;
        self.db.execute(
            "INSERT OR IGNORE INTO event_sources VALUES(?1,?2)",
            params![next.id, e.source_id],
        )?;
        Ok(old.is_none())
    }
    pub fn reprice(&self, settings: &Settings, only_unpriced: bool) -> Result<u64> {
        let tx = self.db.unchecked_transaction()?;
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
        // Initialize discovery before opening a read snapshot (settings() may save on first use).
        self.settings()?;
        let transaction = if self.db.is_autocommit() {
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        let settings = self.settings()?;
        let (mut d, pending) = crate::usage_cache::read(self, f, summary)
            .or_else(|_| self.dashboard_usage_reference(f).map(|d| (d, Vec::new())))?;
        for account in settings
            .accounts
            .iter()
            .filter(|a| a.quota_enabled && !a.archived)
        {
            if f.provider.as_ref().is_some_and(|p| p != &account.provider)
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
                            "关联数据源后可读取限额"
                        } else {
                            "尚未读取限额"
                        }
                        .into(),
                    ),
                    ..Default::default()
                });
            quota.name = account.name.clone();
            if quota.error.as_deref() == Some("设置限额查询命令") {
                quota.error = Some("请刷新限额；远程代理可在数据源中设置".into());
            }
            d.quotas.push(quota);
        }
        d.quota_order = self.quota_order()?;
        let samples = self.estimates()?;
        d.credit_estimates = samples.iter().filter(|e| e.is_credit()).cloned().collect();
        d.quota_estimates = samples.into_iter().filter(|e| !e.is_credit()).collect();
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
        if !summary {
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
                d.sources.push(json!({"id":source.id,"name":source.name,"provider":source.provider,"enabled":source.enabled,"accountId":source.account_id,"accountIds":account_ids,"codexHomeId":source.codex_home_id,"hostId":source.host_id,"status":status.and_then(|s|serde_json::from_str::<Value>(&s).ok())}));
            }
        }
        d.price_updated_at =
            self.db
                .query_row("SELECT MAX(fetched_at) FROM price_history", [], |r| {
                    r.get(0)
                })?;
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
    pub(crate) fn dashboard_usage_reference(&self, f: &Filter) -> Result<Dashboard> {
        use chrono::{Duration, Local, TimeZone};
        let today = Local::now().date_naive();
        let start = today - Duration::days(f.days.unwrap_or(1).clamp(1, 3660) as i64 - 1);
        let trend_start = start.min(today - Duration::days(6));
        let heat_start = today - Duration::days(364);
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
        let mut query=self.db.prepare("SELECT payload,cost,priced_tokens,price FROM events e WHERE stamp>=?1 AND stamp<=?2 AND (?3 IS NULL OR provider=?3) AND (?4 IS NULL OR (?4='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7)))) OR (account_id=?4 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7))))) AND (?5 IS NULL OR model=?5) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?6)) ORDER BY stamp")?;
        let mut rows = query.query(params![
            from,
            now(),
            f.provider,
            f.account_id,
            f.model,
            f.source_id,
            shared
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
            "SELECT DISTINCT model FROM events e WHERE stamp>=?1 AND stamp<=?2
             AND (?3 IS NULL OR provider=?3) AND (?4 IS NULL OR (?4='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6)))) OR (account_id=?4 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6)))))
             AND (?5 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?5))
             ORDER BY model",
        )?;
        d.model_options = options
            .query_map(
                params![
                    options_from,
                    d.generated_at,
                    f.provider,
                    f.account_id,
                    f.source_id,
                    shared
                ],
                |row| row.get(0),
            )?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let mut days = BTreeMap::<String, Aggregate>::new();
        let mut heat = BTreeMap::<String, Aggregate>::new();
        let mut models = BTreeMap::<String, Aggregate>::new();
        let mut trend = BTreeMap::<String, Aggregate>::new();
        let mut day_models = BTreeMap::<(String, String), Aggregate>::new();
        let mut gaps = BTreeMap::<String, PricingGap>::new();
        let settings = self.settings()?;
        let prices = self.prices()?;
        while let Some(row) = rows.next()? {
            let e: UsageEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
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
        d.heatmap = (0..365)
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
fn add_aggregate(a: &mut Aggregate, e: &UsageEvent, cost: f64, n: u64) {
    a.tokens.add(&e.tokens);
    a.total = a.tokens.total();
    a.cost += cost;
    a.priced_tokens += n;
    a.events += 1;
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

/// Discovery distinguishes explicit API credentials from subscription logins without storing credentials.
fn local_account_id(provider: &str, root: &Path) -> String {
    let file = root.join(if provider == "codex" {
        "auth.json"
    } else {
        ".credentials.json"
    });
    let auth = std::fs::read(file)
        .ok()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok());
    match provider {
        "codex" => {
            let Some(auth) = auth else {
                return String::new();
            };
            if auth["auth_mode"] == "apikey"
                || (auth["OPENAI_API_KEY"].is_string() && !auth["tokens"].is_object())
            {
                return String::new();
            }
            if let Some(id) = auth["tokens"]["account_id"].as_str() {
                return format!("codex-{}", &hash(id)[..16]);
            }
            if auth["auth_mode"] == "chatgpt" {
                "codex-local".into()
            } else {
                String::new()
            }
        }
        "claude" => {
            if auth
                .as_ref()
                .is_some_and(|v| v["claudeAiOauth"]["accessToken"].is_string())
            {
                return "claude-local".into();
            }
            #[cfg(target_os = "macos")]
            if root == home().join(".claude") {
                // Check existence only: do not ask security to print the stored value.
                if std::process::Command::new("/usr/bin/security")
                    .args(["find-generic-password", "-s", "Claude Code-credentials"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
                {
                    return "claude-local".into();
                }
            }
            String::new()
        }
        "agy" | "antigravity" => "agy-local".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn api_credentials_do_not_create_a_subscription_account() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(local_account_id("codex", root.path()), "");
        std::fs::write(
            root.path().join("auth.json"),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture"}"#,
        )
        .unwrap();
        assert_eq!(local_account_id("codex", root.path()), "");
        std::fs::write(
            root.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"account_id":"fixture-account"}}"#,
        )
        .unwrap();
        assert!(local_account_id("codex", root.path()).starts_with("codex-"));
    }
}
