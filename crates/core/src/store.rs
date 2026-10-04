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
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let db = Connection::open(root.join("aieyes.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;
        CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY,provider TEXT NOT NULL,account_id TEXT NOT NULL,model TEXT NOT NULL,stamp INTEGER NOT NULL,payload TEXT NOT NULL,cost REAL NOT NULL,priced_tokens INTEGER NOT NULL,price TEXT);
        CREATE INDEX IF NOT EXISTS events_stamp ON events(stamp);
        CREATE TABLE IF NOT EXISTS event_sources(event_id TEXT NOT NULL REFERENCES events(id),source_id TEXT NOT NULL,PRIMARY KEY(event_id,source_id));
        CREATE INDEX IF NOT EXISTS event_sources_source ON event_sources(source_id,event_id);
        CREATE TABLE IF NOT EXISTS source_identities(source_id TEXT NOT NULL,provider TEXT NOT NULL,account_id TEXT NOT NULL,PRIMARY KEY(source_id,provider,account_id));
        CREATE TABLE IF NOT EXISTS files(source_id TEXT NOT NULL,path TEXT NOT NULL,signature TEXT NOT NULL,offset INTEGER NOT NULL,state TEXT NOT NULL,PRIMARY KEY(source_id,path));
        CREATE TABLE IF NOT EXISTS quotas(account_key TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS source_status(source_id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS price_history(id INTEGER PRIMARY KEY,model_id TEXT NOT NULL,fetched_at INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS price_model ON price_history(model_id,fetched_at);
        PRAGMA user_version=2;")?;
        Ok(Self { db })
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
                settings.migrate();
                Ok(settings)
            }
            None => {
                let mut settings = Settings::default();
                for (provider, name, dir) in [
                    ("codex", "Codex · 本机", ".codex"),
                    ("claude", "Claude Code · 本机", ".claude"),
                    ("agy", "agy · 本机", ".gemini/antigravity-cli"),
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
                settings.migrate();
                self.save_settings(&settings)?;
                Ok(settings)
            }
        }
    }
    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let mut normalized = s.clone();
        normalized.migrate();
        let s = &normalized;
        let mut ids = std::collections::HashSet::new();
        for h in &s.hosts {
            anyhow::ensure!(
                !h.id.is_empty() && ids.insert(format!("host:{}", h.id)),
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
                !account.id.trim().is_empty()
                    && ids.insert(format!("account:{}:{}", account.provider, account.id)),
                "账户标识重复或为空"
            );
            anyhow::ensure!(!account.name.trim().is_empty(), "请输入账户名称");
            if let Some(id) = &account.quota_source_id {
                anyhow::ensure!(
                    s.sources.iter().any(|src| &src.id == id
                        && src.provider == account.provider
                        && src.account_id == account.id),
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
                ["agy", "deepseek"].contains(&source.provider.as_str())
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
            "Agent 刷新间隔范围为 10–86400 秒"
        );
        anyhow::ensure!(
            (2..=86400).contains(&s.server_refresh_seconds),
            "服务器刷新间隔范围为 2–86400 秒"
        );
        let previous: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let tx = self.db.unchecked_transaction()?;
        if let Some(previous) = previous {
            let previous: Settings = serde_json::from_str(&previous)?;
            // Archiving retains the namespace used to deduplicate imported records.
            // Protect history from destructive deletion by older clients as well.
            for account in &previous.accounts {
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
                    anyhow::ensure!(!has_history, "该账户存在历史记录，请归档账户以保留历史");
                }
            }
            for source in &s.sources {
                if let Some(old) = previous.sources.iter().find(|old| old.id == source.id) {
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
        tx.execute("INSERT INTO kv VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(s)?])?;
        self.reprice_in_transaction(s, true)?;
        tx.commit()?;
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
            let mut query = self.db.prepare("SELECT account_id FROM source_identities WHERE source_id=?1 AND provider=?2 ORDER BY rowid")?;
            let scopes = query
                .query_map(params![e.source_id, e.provider], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Only reuse an event previously observed by this source. Unrelated
            // no-account sources and separate accounts keep independent identities.
            for account_id in scopes {
                let id = crate::usage::event_id(&e.provider, &e.source_id, &account_id, identity);
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
        self.db.execute("INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET stamp=excluded.stamp,payload=excluded.payload,cost=excluded.cost,priced_tokens=excluded.priced_tokens,price=excluded.price",
            params![next.id,next.provider,next.account_id,next.model,next.timestamp,serde_json::to_string(&next)?,cost,covered,price.map(serde_json::to_string).transpose()?])?;
        self.db.execute(
            "INSERT OR IGNORE INTO event_sources VALUES(?1,?2)",
            params![next.id, e.source_id],
        )?;
        Ok(old.is_none())
    }
    pub fn reprice(&self, settings: &Settings, only_unpriced: bool) -> Result<u64> {
        let tx = self.db.unchecked_transaction()?;
        let count = self.reprice_in_transaction(settings, only_unpriced)?;
        tx.commit()?;
        Ok(count)
    }
    fn reprice_in_transaction(&self, settings: &Settings, only_unpriced: bool) -> Result<u64> {
        let prices = self.prices()?;
        let mut query = self
            .db
            .prepare("SELECT id,payload,price,priced_tokens FROM events")?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut count = 0;
        for (id, raw, snapshot, covered) in rows {
            let e: UsageEvent = serde_json::from_str(&raw)?;
            if only_unpriced && covered >= e.tokens.total() {
                continue;
            }
            if let Some(current) = pricing::find_price(&e.model, &prices, &settings.model_mappings)
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
        if let Some(raw) = prior {
            let old: QuotaSnapshot = serde_json::from_str(&raw)?;
            if q.error.is_some() {
                next = old;
                next.error = q.error.clone();
            } else if old.updated_at > q.updated_at {
                return Ok(());
            } else if q.origin == "log" && old.bank_reset.is_some() {
                next.bank_reset = old.bank_reset;
                next.bank_updated_at = old.bank_updated_at.or(Some(old.updated_at));
            }
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
        let mut query=self.db.prepare("SELECT payload,cost,priced_tokens,price FROM events e WHERE stamp>=?1 AND stamp<=?2 AND (?3 IS NULL OR provider=?3) AND (?4 IS NULL OR account_id=?4) AND (?5 IS NULL OR model=?5) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?6)) ORDER BY stamp")?;
        let mut rows = query.query(params![
            from,
            now(),
            f.provider,
            f.account_id,
            f.model,
            f.source_id
        ])?;
        let mut d = Dashboard {
            generated_at: now(),
            ..Default::default()
        };
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
                .filter(|s| {
                    s.enabled && s.provider == account.provider && s.account_id == account.id
                })
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
        d.quotas.sort_by(|a, b| {
            a.windows
                .is_empty()
                .cmp(&b.windows.is_empty())
                .then(a.name.cmp(&b.name))
        });
        for source in &settings.sources {
            let status: Option<String> = self
                .db
                .query_row(
                    "SELECT payload FROM source_status WHERE source_id=?1",
                    [&source.id],
                    |r| r.get(0),
                )
                .optional()?;
            let mut query = self.db.prepare("SELECT DISTINCT e.account_id FROM events e JOIN event_sources es ON es.event_id=e.id WHERE es.source_id=?1 AND e.provider=?2")?;
            let mut account_ids = query
                .query_map(params![source.id, source.provider], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !account_ids.contains(&source.account_id) {
                account_ids.push(source.account_id.clone());
            }
            d.sources.push(json!({"id":source.id,"name":source.name,"provider":source.provider,"enabled":source.enabled,"accountId":source.account_id,"accountIds":account_ids,"hostId":source.host_id,"status":status.and_then(|s|serde_json::from_str::<Value>(&s).ok())}));
        }
        d.price_updated_at =
            self.db
                .query_row("SELECT MAX(fetched_at) FROM price_history", [], |r| {
                    r.get(0)
                })?;
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
        "agy" => "agy-local".into(),
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
