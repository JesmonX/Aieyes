//! Opt-in source-level valuation. Ambiguous intervals never enter either cost pool.
use crate::{
    Engine,
    estimates::{CreditEstimate, Estimate, EstimateCommon, QuotaEstimate},
    models::*,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const CONSENT_VERSION: u32 = 1;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoValuationConfig {
    pub enabled: bool,
    pub consent_version: u32,
    /// Empty in legacy settings: retain the original subscription-only consent.
    pub mode: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoSegment {
    pub started_at: i64,
    pub ended_at: i64,
    pub reset_at: i64,
    pub cost: f64,
    pub tokens: u64,
    pub priced_tokens: u64,
    pub consumed: f64,
    pub weekly_consumed: f64,
    pub weekly_cost: f64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Baseline {
    quota: Option<QuotaSnapshot>,
    config: String,
    source_ids: Vec<String>,
}

fn enabled(s: &Source) -> bool {
    s.auto_valuation
        .as_ref()
        .is_some_and(|a| a.enabled && a.consent_version == CONSENT_VERSION)
}
fn baseline_key(key: &str) -> String {
    format!("automatic-baseline:{key}")
}
fn status_key(id: &str) -> String {
    format!("automatic-status:{id}")
}
fn window_id(w: &QuotaWindow) -> &str {
    if w.id.is_empty() { &w.name } else { &w.id }
}
fn fresh_credits(q: &QuotaSnapshot) -> Option<&CreditsSnapshot> {
    if q.credits_origin.as_deref() == Some("log")
        || q.credits_updated_at.is_some_and(|t| t != q.updated_at)
    {
        return None;
    }
    q.credits.as_ref()
}
fn balance(q: &QuotaSnapshot) -> Option<i128> {
    fresh_credits(q)
        .filter(|c| !c.unlimited)?
        .balance
        .as_deref()
        .and_then(crate::estimates::credit_units)
}
fn valid(q: &QuotaSnapshot) -> bool {
    q.error.is_none()
        && q.origin != "log"
        && q.updated_at > 0
        && !matches!(
            q.plan
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "api" | "free"
        )
        && !q.windows.is_empty()
        && q.windows
            .iter()
            .all(|w| w.used_percent.is_finite() && (0.0..=100.0).contains(&w.used_percent))
}

/// A zero percentage delta is not evidence of credit-only usage: quota must already be exhausted.
fn interval_kind(a: &QuotaSnapshot, b: &QuotaSnapshot) -> Option<&'static str> {
    if !valid(a)
        || !valid(b)
        || b.updated_at <= a.updated_at
        || b.updated_at - a.updated_at > 300
        || a.plan != b.plan
    {
        return None;
    }
    if a.windows.len() != b.windows.len() {
        return None;
    }
    let pairs = a
        .windows
        .iter()
        .map(|old| {
            b.windows
                .iter()
                .find(|w| window_id(w) == window_id(old))
                .map(|new| (old, new))
        })
        .collect::<Option<Vec<_>>>()?;
    if pairs.iter().any(|(old, new)| {
        old.resets_at != new.resets_at
            || old.resets_at.is_none_or(|t| t <= b.updated_at)
            || new.used_percent < old.used_percent
    }) {
        return None;
    }
    let quota_changed = pairs.iter().any(|(a, b)| b.used_percent > a.used_percent);
    let (credit_changed, credit_unchanged) = if a.provider == "codex" {
        match (balance(a), balance(b)) {
            (Some(x), Some(y)) if y <= x => (y < x, y == x),
            _ if fresh_credits(a)
                .is_some_and(|c| !c.has_credits && !c.unlimited && c.balance.is_none())
                && fresh_credits(b)
                    .is_some_and(|c| !c.has_credits && !c.unlimited && c.balance.is_none()) =>
            {
                (false, true)
            }
            _ => return None,
        }
    } else {
        (false, true)
    };
    if quota_changed && credit_unchanged {
        Some("quota")
    } else if credit_changed
        && !quota_changed
        && pairs.iter().any(|(old, new)| {
            old.used_percent == 100.0
                && new.used_percent == 100.0
                && crate::estimates::eligible(b, new)
        })
    {
        Some("credits")
    } else {
        None
    }
}

impl Store {
    fn auto_get<T: for<'de> Deserialize<'de> + Default>(&self, key: &str) -> Result<T> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
            .optional()?;
        Ok(raw
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default())
    }
    fn auto_put(&self, key: &str, value: &impl Serialize) -> Result<()> {
        self.db.execute(
            "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn automatic_status(&self, id: &str) -> Result<Value> {
        self.auto_get(&status_key(id))
    }
    pub(crate) fn automatic_status_set(
        &self,
        ids: &[String],
        state: &str,
        note: &str,
        account: &str,
    ) -> Result<()> {
        for id in ids {
            let previous = self.automatic_status(id)?;
            if previous["state"] == state
                && previous["note"] == note
                && previous["accountKey"] == account
            {
                continue;
            }
            self.auto_put(
                &status_key(id),
                &json!({"state":state,"note":note,"accountKey":account,"updatedAt":now()}),
            )?;
        }
        Ok(())
    }
    pub(crate) fn invalidate_automatic_sources(&self, ids: &[String]) -> Result<()> {
        let tx = if self.db.is_autocommit() {
            Some(self.write_transaction()?)
        } else {
            None
        };
        for mut e in self.estimates()?.into_iter().filter(|e| {
            e.sampling_mode == "automatic"
                && e.status != "completed"
                && e.source_ids.iter().any(|id| ids.contains(id))
        }) {
            e.status = "completed".into();
            e.ended_at = Some(e.checkpoint_at);
            self.save_estimate(&e)?;
            self.auto_put(&baseline_key(&e.account_key), &Baseline::default())?;
        }
        // A baseline can exist before the first priced interval produces a record.
        let mut query = self
            .db
            .prepare("SELECT key,value FROM kv WHERE key LIKE 'automatic-baseline:%'")?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(query);
        for (key, raw) in rows {
            let b: Baseline = serde_json::from_str(&raw)?;
            if b.source_ids.iter().any(|id| ids.contains(id)) {
                self.auto_put(&key, &Baseline::default())?;
            }
        }
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(())
    }

    fn automatic_fingerprint(
        &self,
        settings: &Settings,
        ids: &[String],
        key: &str,
    ) -> Result<String> {
        let config = crate::estimates::source_config(settings, ids, key)?;
        let revisions = ids
            .iter()
            .map(|id| {
                crate::accounts::status_revision(
                    self,
                    settings.sources.iter().find(|s| &s.id == id).unwrap(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(hash(&serde_json::to_string(&json!([
            config,
            revisions,
            settings.model_mappings
        ]))?))
    }

    /// Persist one authoritative observation. Used by the background coordinator and regression tests.
    pub fn advance_automatic(&self, q: &QuotaSnapshot, ids: &[String], config: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        let settings = self.settings()?;
        ensure!(
            !ids.is_empty()
                && ids.iter().all(|id| settings
                    .sources
                    .iter()
                    .any(|s| &s.id == id && enabled(s) && settings.source_enabled(s))),
            "自动估值来源已变化"
        );
        let key = format!("{}:{}", q.provider, q.account_id);
        ensure!(
            self.automatic_fingerprint(&settings, ids, &key)? == config,
            "采样过程中登录或来源设置变化"
        );
        let previous: Baseline = self.auto_get(&baseline_key(&key))?;
        if self.estimates()?.iter().any(|e| {
            e.sampling_mode != "automatic"
                && e.status != "completed"
                && e.source_ids.iter().any(|id| ids.contains(id))
        }) {
            self.auto_put(&baseline_key(&key), &Baseline::default())?;
            self.automatic_status_set(ids, "waitingManual", "等待手动采样结束", &key)?;
            tx.commit()?;
            return Ok(());
        }
        if !valid(q) {
            self.invalidate_automatic_sources(ids)?;
            self.automatic_status_set(ids, "waiting", "等待有效的实时订阅限额", &key)?;
            tx.commit()?;
            return Ok(());
        }
        let old = previous.quota.as_ref().filter(|old| {
            previous.config == config
                && previous.source_ids == ids
                && old.account_id == q.account_id
                && old.provider == q.provider
        });
        if old.is_none() {
            self.invalidate_automatic_sources(ids)?;
            self.auto_put(
                &baseline_key(&key),
                &Baseline {
                    quota: Some(q.clone()),
                    config: config.into(),
                    source_ids: ids.to_vec(),
                },
            )?;
            self.automatic_status_set(ids, "collecting", "已建立基线，正在积累样本", &key)?;
            tx.commit()?;
            return Ok(());
        }
        let old = old.unwrap();
        if q.updated_at <= old.updated_at {
            tx.commit()?;
            return Ok(());
        }
        if old.plan != q.plan {
            self.invalidate_automatic_sources(ids)?;
        }
        let kind = interval_kind(old, q);
        let mut accepted = false;
        if let Some(kind) = kind {
            let windows = q
                .windows
                .iter()
                .filter(|w| crate::estimates::eligible(q, w))
                .filter(|w| {
                    w.window_minutes == Some(crate::contracts::FIVE_HOUR_MINUTES)
                        || !q.windows.iter().any(|v| {
                            crate::estimates::eligible(q, v)
                                && v.group_id == w.group_id
                                && v.window_minutes == Some(crate::contracts::FIVE_HOUR_MINUTES)
                        })
                })
                .collect::<Vec<_>>();
            let targets: Vec<Option<&QuotaWindow>> = if kind == "credits" {
                vec![None]
            } else {
                windows.into_iter().map(Some).collect()
            };
            for window in targets {
                let consumed = if let Some(w) = window {
                    w.used_percent
                        - old
                            .windows
                            .iter()
                            .find(|v| window_id(v) == window_id(w))
                            .map(|v| v.used_percent)
                            .unwrap_or(w.used_percent)
                } else {
                    (balance(old).unwrap() - balance(q).unwrap()) as f64 / 1e18
                };
                if consumed <= 0.0 {
                    continue;
                }
                let group = window
                    .filter(|_| crate::providers::antigravity(&q.provider))
                    .map(crate::antigravity::window_group);
                let mut common = EstimateCommon {
                    account_key: key.clone(),
                    source_ids: ids.to_vec(),
                    shared_source_ids: ids
                        .iter()
                        .filter(|id| {
                            settings
                                .sources
                                .iter()
                                .any(|s| &s.id == *id && s.codex_home_id.is_some())
                        })
                        .cloned()
                        .collect(),
                    started_at: old.updated_at,
                    checkpoint_at: q.updated_at,
                    ..Default::default()
                };
                self.calculate_usage(&mut common, group)?;
                if !common.cost.is_finite()
                    || common.cost < 0.0
                    || common.calculation_status != "ready"
                    || common.priced_tokens != common.total_tokens
                {
                    continue;
                }
                let win = window.map(window_id).unwrap_or("credits");
                let mut record = self.estimates()?.into_iter().find(|e| {
                    e.sampling_mode == "automatic"
                        && e.status == "active"
                        && e.account_key == key
                        && e.window_id == win
                });
                if record.is_none() {
                    common.id = format!(
                        "auto-{}-{}",
                        kind,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_nanos()
                    );
                    common.sampling_mode = "automatic".into();
                    common.status = "active".into();
                    common.window_id = win.into();
                    common.window_name = window.map(|w| w.name.clone()).unwrap_or("Credits".into());
                    common.source_config = crate::estimates::source_config(&settings, ids, &key)?;
                    common.source_names = ids
                        .iter()
                        .filter_map(|id| {
                            settings
                                .sources
                                .iter()
                                .find(|s| &s.id == id)
                                .map(|s| s.name.clone())
                        })
                        .collect();
                    common.cost = 0.0;
                    common.total_tokens = 0;
                    common.priced_tokens = 0;
                    common.prices.clear();
                    record = Some(if let Some(window) = window {
                        Estimate::Quota(QuotaEstimate {
                            common,
                            group_id: group.map(str::to_owned),
                            valuation_mode: if window.window_minutes
                                == Some(crate::contracts::FIVE_HOUR_MINUTES)
                            {
                                "fiveHour".into()
                            } else {
                                String::new()
                            },
                            quota_plan: q.plan.clone(),
                            ..Default::default()
                        })
                    } else {
                        Estimate::Credit(CreditEstimate {
                            common,
                            ..Default::default()
                        })
                    });
                }
                let mut e = record.unwrap();
                let mut sample = EstimateCommon {
                    account_key: key.clone(),
                    source_ids: ids.to_vec(),
                    shared_source_ids: e.shared_source_ids.clone(),
                    started_at: old.updated_at,
                    checkpoint_at: q.updated_at,
                    ..Default::default()
                };
                self.calculate_usage(&mut sample, group)?;
                let weekly = window
                    .filter(|w| w.window_minutes == Some(crate::contracts::FIVE_HOUR_MINUTES))
                    .and_then(|_| {
                        crate::capacity::window(
                            q,
                            crate::contracts::WEEK_MINUTES,
                            group.unwrap_or(""),
                        )
                    })
                    .and_then(|w| {
                        old.windows
                            .iter()
                            .find(|v| window_id(v) == window_id(w))
                            .map(|v| w.used_percent - v.used_percent)
                    })
                    .unwrap_or(0.0);
                let segment = AutoSegment {
                    started_at: old.updated_at,
                    ended_at: q.updated_at,
                    reset_at: window.and_then(|w| w.resets_at).unwrap_or(0),
                    cost: sample.cost,
                    tokens: sample.total_tokens,
                    priced_tokens: sample.priced_tokens,
                    consumed,
                    weekly_consumed: weekly,
                    weekly_cost: if weekly > 0.0 { sample.cost } else { 0.0 },
                };
                if let Some(last) = e
                    .automatic_segments
                    .last_mut()
                    .filter(|s| s.ended_at == segment.started_at && s.reset_at == segment.reset_at)
                {
                    last.ended_at = segment.ended_at;
                    last.cost += segment.cost;
                    last.tokens += segment.tokens;
                    last.priced_tokens += segment.priced_tokens;
                    last.consumed += segment.consumed;
                    last.weekly_consumed += segment.weekly_consumed;
                    last.weekly_cost += segment.weekly_cost;
                } else {
                    e.automatic_segments.push(segment);
                }
                e.cost += sample.cost;
                e.total_tokens += sample.total_tokens;
                e.priced_tokens += sample.priced_tokens;
                e.checkpoint_at = q.updated_at;
                for p in sample.prices {
                    if !e.prices.contains(&p) {
                        e.prices.push(p);
                    }
                }
                let used = e.automatic_segments.iter().map(|s| s.consumed).sum::<f64>();
                let ready = used + 0.000001 >= 5.0;
                e.calculation_status = if ready { "ready" } else { "insufficientUsage" }.into();
                e.calculation_note = if ready {
                    "自动估值 · 仅计入可确认归属的有效区间"
                } else {
                    "自动估值 · 正在积累样本，至少消耗 5 个百分点或 5 credits"
                }
                .into();
                match &mut e {
                    Estimate::Quota(v) => {
                        v.consumed_percent = used;
                        if ready {
                            if v.valuation_mode == "fiveHour" {
                                v.five_hour_value = Some(v.cost * 100.0 / used);
                                let week = v
                                    .automatic_segments
                                    .iter()
                                    .map(|s| s.weekly_consumed)
                                    .sum::<f64>();
                                let cost = v
                                    .automatic_segments
                                    .iter()
                                    .map(|s| s.weekly_cost)
                                    .sum::<f64>();
                                if week >= 5.0 {
                                    v.weekly_value = Some(cost * 100.0 / week);
                                    v.weekly_direct_value = v.weekly_value;
                                }
                                v.capacity =
                                    self.capacity_for_group(&key, group.unwrap_or(""), now())?;
                                v.weekly_ratio_value =
                                    v.five_hour_value.zip(v.capacity.ratio).map(|(a, b)| a * b);
                            } else {
                                v.weekly_value = Some(v.cost * 100.0 / used);
                            }
                        }
                    }
                    Estimate::Credit(v) => {
                        v.consumed_credits = used;
                        if ready {
                            v.value_per1000 = Some(v.cost * 1000.0 / used);
                            v.value_per500 = Some(v.cost * 500.0 / used);
                        }
                    }
                }
                self.save_estimate(&e)?;
                accepted = true;
            }
        }
        self.auto_put(
            &baseline_key(&key),
            &Baseline {
                quota: Some(q.clone()),
                config: config.into(),
                source_ids: ids.to_vec(),
            },
        )?;
        self.automatic_status_set(
            ids,
            if accepted { "sampling" } else { "waiting" },
            if accepted {
                "自动更新有效样本"
            } else {
                "等待可确认归属且价格完整的样本；已保留有效估值"
            },
            &key,
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl Engine {
    pub(crate) fn configure_automatic(&mut self, p: Value) -> Result<Value> {
        let tx = self.store.write_transaction()?;
        let base = self.store.settings()?;
        let mut next = base.clone();
        let id = p["sourceId"].as_str().context("请选择数据源")?;
        let source = next
            .sources
            .iter_mut()
            .find(|s| s.id == id)
            .context("数据源已删除")?;
        let enable = p["enabled"].as_bool().context("请选择开启或关闭")?;
        let mode = p["mode"].as_str().unwrap_or("subscription");
        ensure!(matches!(mode, "subscription" | "mixed"), "自动估值模式无效");
        ensure!(
            mode != "mixed" || source.provider == "codex",
            "混用估值仅适用于 Codex"
        );
        ensure!(
            !enable || (p["confirmed"] == true && source.provider != "deepseek"),
            "请确认所选计费方式，并纳入影响订阅额度的所有设备用量"
        );
        if let Some(expected) = p.get("baseConfig") {
            let expected: Option<AutoValuationConfig> = serde_json::from_value(expected.clone())?;
            let normalize = |mut config: Option<AutoValuationConfig>| {
                if let Some(c) = &mut config
                    && c.mode.is_empty()
                {
                    c.mode = "subscription".into();
                }
                serde_json::to_value(config)
            };
            ensure!(
                normalize(expected)? == normalize(source.auto_valuation.clone())?,
                "自动估值设置已变化，请刷新后重试"
            );
        }
        source.auto_valuation = enable.then_some(AutoValuationConfig {
            enabled: true,
            consent_version: CONSENT_VERSION,
            mode: mode.into(),
        });
        // Consent changes do not change the token scope of an ongoing manual sample.
        // Keep its original checkpoint and status; the automatic worker waits for it.
        let manuals = self
            .store
            .estimates()?
            .into_iter()
            .filter(|e| {
                e.sampling_mode != "automatic"
                    && e.status == "active"
                    && e.source_ids.iter().any(|source| source == id)
                    && crate::estimates::source_config(&base, &e.source_ids, &e.account_key)
                        .ok()
                        .as_ref()
                        == Some(&e.source_config)
            })
            .collect::<Vec<_>>();
        self.store.save_settings(&next)?;
        let saved = self.store.settings()?;
        for mut manual in manuals {
            manual.source_config =
                crate::estimates::source_config(&saved, &manual.source_ids, &manual.account_key)?;
            self.store.save_estimate(&manual)?;
        }
        self.store.invalidate_automatic_sources(&[id.into()])?;
        self.store.automatic_status_set(
            &[id.into()],
            if enable { "collecting" } else { "disabled" },
            if enable {
                "等待订阅登录与首个实时快照"
            } else {
                "自动估值已关闭，结果保留"
            },
            "",
        )?;
        tx.commit()?;
        Ok(json!({"settings":self.store.settings()?}))
    }

    pub(crate) fn sampling_identity(&self, settings: &Settings, source: &Source) -> Result<String> {
        if source.provider == "codex" && source.codex_home_id.is_some() {
            let info = crate::codex_auth::backend(source, settings, "profiles.list", &json!({}))?;
            let identity = info["currentIdentity"]
                .as_str()
                .context("当前订阅登录身份无法确认")?;
            let profile = info["profiles"]
                .as_array()
                .and_then(|v| {
                    v.iter()
                        .find(|p| p["current"] == true && p["identity"]["key"] == identity)
                })
                .and_then(|p| p["id"].as_str())
                .context("请在账户页读取当前订阅登录")?;
            let reference = format!("{}:{profile}", source.codex_home_id.as_deref().unwrap());
            let account = settings
                .accounts
                .iter()
                .find(|a| {
                    a.provider == source.provider
                        && a.has_profile(&reference)
                        && !a.archived
                        && a.quota_enabled
                })
                .context("当前登录未连接启用的订阅账户")?;
            Ok(crate::accounts::key(account))
        } else {
            let account = settings
                .accounts
                .iter()
                .find(|a| {
                    a.provider == source.provider
                        && a.id == source.account_id
                        && !a.archived
                        && a.quota_enabled
                })
                .context("数据源未连接启用的订阅账户")?;
            Ok(crate::accounts::key(account))
        }
    }
    pub(crate) fn verify_sampling_identities(&self, ids: &[String], key: &str) -> Result<()> {
        let settings = self.store.settings()?;
        for id in ids {
            let s = settings
                .sources
                .iter()
                .find(|s| &s.id == id)
                .context("来源已删除")?;
            ensure!(
                self.sampling_identity(&settings, s)? == key,
                "共享目录的当前登录与采样目标不一致"
            );
        }
        Ok(())
    }
    pub(crate) fn tick_automatic(&mut self) -> Result<Value> {
        let root = Path::new(self.store.db.path().context("数据目录不存在")?)
            .parent()
            .context("数据目录不存在")?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("automatic-valuation.lock"))?;
        let _lock = match crate::file_lock::FileLock::try_exclusive(file) {
            Ok(v) => v,
            Err(std::fs::TryLockError::WouldBlock) => return Ok(json!({"changed":false})),
            Err(e) => return Err(e.into()),
        };
        let settings = self.store.settings()?;
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        let mut changed = false;
        for s in settings.sources.iter().filter(|s| enabled(s)) {
            if !settings.source_enabled(s) {
                self.store
                    .invalidate_automatic_sources(std::slice::from_ref(&s.id))?;
                self.store.automatic_status_set(
                    std::slice::from_ref(&s.id),
                    "waiting",
                    "来源或机器已暂停",
                    "",
                )?;
                continue;
            }
            match self.sampling_identity(&settings, s) {
                Ok(key) => {
                    let prior = self.store.automatic_status(&s.id)?;
                    if prior["accountKey"]
                        .as_str()
                        .is_some_and(|old| !old.is_empty() && old != key)
                    {
                        self.store
                            .invalidate_automatic_sources(std::slice::from_ref(&s.id))?;
                    }
                    groups.entry(key).or_default().push(s.id.clone());
                }
                Err(_) => {
                    let prior = self.store.automatic_status(&s.id)?;
                    self.store
                        .invalidate_automatic_sources(std::slice::from_ref(&s.id))?;
                    self.store.automatic_status_set(
                        std::slice::from_ref(&s.id),
                        "waitingIdentity",
                        "等待可确认的订阅登录",
                        "",
                    )?;
                    changed |= prior["state"] != "waitingIdentity";
                }
            }
        }
        for (key, mut ids) in groups {
            ids.sort();
            let attempt_key = format!("automatic-attempt:{key}");
            let attempted: i64 = self.store.auto_get(&attempt_key)?;
            if (0..30).contains(&(now() - attempted)) {
                continue;
            }
            if self.store.estimates()?.iter().any(|e| {
                e.sampling_mode != "automatic"
                    && e.status != "completed"
                    && e.source_ids.iter().any(|id| ids.contains(id))
            }) {
                self.store
                    .auto_put(&baseline_key(&key), &Baseline::default())?;
                self.store
                    .automatic_status_set(&ids, "waitingManual", "等待手动采样结束", &key)?;
                self.store.auto_put(&attempt_key, &now())?;
                changed = true;
                continue;
            }
            let last: Baseline = self.store.auto_get(&baseline_key(&key))?;
            if last
                .quota
                .as_ref()
                .is_some_and(|q| now() - q.updated_at < 30 && now() >= q.updated_at)
                && last.source_ids == ids
            {
                continue;
            }
            let config = crate::estimates::source_config(&settings, &ids, &key)?;
            let revisions = ids
                .iter()
                .map(|id| {
                    crate::accounts::status_revision(
                        &self.store,
                        settings.sources.iter().find(|s| &s.id == id).unwrap(),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let fingerprint = self.store.automatic_fingerprint(&settings, &ids, &key)?;
            self.store.auto_put(&attempt_key, &now())?;
            let outcome = (|| -> Result<()> {
                self.verify_sampling_identities(&ids, &key)?;
                self.sync_estimate_sources(&ids)?;
                let rows = self.read_quotas(&json!({"accountKey":key}))?;
                let q = rows
                    .iter()
                    .find(|q| format!("{}:{}", q.provider, q.account_id) == key)
                    .context("未读到账户额度")?;
                ensure!(
                    valid(q) && (now() - q.updated_at).abs() <= 120,
                    "等待有效订阅快照"
                );
                self.sync_estimate_sources(&ids)?;
                self.verify_sampling_identities(&ids, &key)?;
                let latest = self.store.settings()?;
                ensure!(
                    crate::estimates::source_config(&latest, &ids, &key)? == config,
                    "采样过程中来源设置变化"
                );
                for (id, rev) in ids.iter().zip(revisions.iter()) {
                    ensure!(
                        crate::accounts::status_revision(
                            &self.store,
                            latest.sources.iter().find(|s| &s.id == id).unwrap()
                        )? == *rev,
                        "采样过程中登录已变化"
                    );
                }
                self.store.quota(q)?;
                self.store.advance_automatic(q, &ids, &fingerprint)?;
                Ok(())
            })();
            if outcome.is_err() {
                self.store.invalidate_automatic_sources(&ids)?;
                self.store.automatic_status_set(
                    &ids,
                    "waiting",
                    "等待完整用量和有效订阅快照；结果保留",
                    &key,
                )?;
            }
            changed = true;
        }
        Ok(json!({"changed":changed}))
    }
}

/// One worker per application; the on-disk lock also coordinates multiple engine processes.
pub struct Background {
    stopped: Arc<AtomicBool>,
}
impl Background {
    pub fn start(root: &Path, on_change: impl Fn() + Send + 'static) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let root = root.to_owned();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok(_scope) = crate::operations::enter("autoEstimates.tick")
                    && let Ok(mut engine) = Engine::open(&root)
                    && let Ok(value) = engine.tick_automatic()
                    && value["changed"] == true
                {
                    on_change();
                }
                for _ in 0..5 {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });
        Self { stopped }
    }
}
impl Drop for Background {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(shared: bool) -> (tempfile::TempDir, Engine, QuotaSnapshot) {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path()).unwrap();
        let s = Source {
            id: "s".into(),
            name: "本机".into(),
            path: "/fixture".into(),
            provider: "codex".into(),
            account_id: if shared { String::new() } else { "a".into() },
            codex_home_id: shared.then(|| "home".into()),
            auto_valuation: Some(AutoValuationConfig {
                enabled: true,
                consent_version: CONSENT_VERSION,
                mode: String::new(),
            }),
            ..Default::default()
        };
        let a = Account {
            id: "a".into(),
            name: "A".into(),
            provider: "codex".into(),
            quota_profile_id: shared.then(|| "home:one".into()),
            ..Default::default()
        };
        engine
            .store
            .save_settings(&Settings {
                sources: vec![s],
                accounts: vec![a],
                ..Default::default()
            })
            .unwrap();
        engine
            .store
            .save_prices(&[ModelPrice {
                id: "model".into(),
                input: Some(0.01),
                ..Default::default()
            }])
            .unwrap();
        let q = QuotaSnapshot {
            provider: "codex".into(),
            account_id: "a".into(),
            source_id: "s".into(),
            plan: Some("plus".into()),
            origin: "live".into(),
            updated_at: now() - 200,
            credits: Some(CreditsSnapshot {
                has_credits: true,
                balance: Some("1000".into()),
                ..Default::default()
            }),
            windows: vec![QuotaWindow {
                id: "week".into(),
                name: "7d".into(),
                used_percent: 10.0,
                window_minutes: Some(crate::contracts::WEEK_MINUTES),
                resets_at: Some(now() + 10000),
                ..Default::default()
            }],
            ..Default::default()
        };
        (root, engine, q)
    }
    fn event(e: &Engine, q: &QuotaSnapshot, id: &str, tokens: u64, api: bool) {
        let settings = e.store.settings().unwrap();
        let source = &settings.sources[0];
        e.store
            .put_event(
                &UsageEvent {
                    turn_id: String::new(),
                    id: id.into(),
                    source_id: source.id.clone(),
                    account_id: source.account_id.clone(),
                    provider: "codex".into(),
                    timestamp: q.updated_at + 5,
                    model: "model".into(),
                    tokens: Tokens {
                        input: tokens,
                        ..Default::default()
                    },
                    billing: BillingEvidence {
                        category: if api { "api" } else { "unknown" }.into(),
                        evidence: "fixture".into(),
                    },
                    interval_start: Some(q.updated_at),
                    import_identity: None,
                    session_id: "session".into(),
                    attribution: "delta".into(),
                    interval_evidence: String::new(),
                },
                &e.store.prices().unwrap(),
                &settings,
            )
            .unwrap();
    }
    fn observe(e: &Engine, q: &QuotaSnapshot) {
        e.store
            .advance_automatic(
                q,
                &["s".into()],
                &e.store
                    .automatic_fingerprint(
                        &e.store.settings().unwrap(),
                        &["s".into()],
                        &format!("{}:{}", q.provider, q.account_id),
                    )
                    .unwrap(),
            )
            .unwrap();
    }
    #[test]
    fn consent_is_source_scoped_default_off_and_concurrent_edits_are_rejected() {
        let (_dir, mut e, _q) = setup(false);
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":false}),
        )
        .unwrap();
        assert!(
            e.call(
                "autoEstimates.configure",
                json!({"sourceId":"s","enabled":true})
            )
            .is_err()
        );
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":true,"confirmed":true,"baseConfig":null}),
        )
        .unwrap();
        assert!(
            e.call(
                "autoEstimates.configure",
                json!({"sourceId":"s","enabled":false,"baseConfig":null})
            )
            .is_err()
        );
        assert!(
            e.store.settings().unwrap().sources[0]
                .auto_valuation
                .as_ref()
                .unwrap()
                .enabled
        );
        let old: Source = serde_json::from_value(json!({"id":"old","provider":"codex"})).unwrap();
        assert!(old.auto_valuation.is_none());
    }
    #[test]
    fn automatic_subscription_sampling_keeps_shared_history_unattributed() {
        let (_dir, e, mut q) = setup(true);
        observe(&e, &q);
        event(&e, &q, "good", 100, false);
        event(&e, &q, "api", 100, true);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        let records = e.store.estimates().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.sampling_mode, "automatic");
        assert_eq!(record.cost, 1.0);
        assert_eq!(record.total_tokens, 100);
        assert_eq!(record.quota().unwrap().weekly_value, Some(10.0));
        assert_eq!(
            e.store
                .dashboard(&Filter {
                    account_id: Some("a".into()),
                    ..Default::default()
                })
                .unwrap()
                .summary
                .total,
            0
        );
        assert_eq!(
            e.store.dashboard(&Filter::default()).unwrap().summary.total,
            200
        );
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.0);
    }
    #[test]
    fn mixed_and_rounded_quota_intervals_never_enter_credit_costs() {
        let (_dir, e, mut q) = setup(false);
        observe(&e, &q);
        event(&e, &q, "subscription", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        event(&e, &q, "mixed", 200, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        q.credits.as_mut().unwrap().balance = Some("990".into());
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.0);
        event(&e, &q, "rounded", 200, false);
        q.updated_at += 30;
        q.credits.as_mut().unwrap().balance = Some("980".into());
        observe(&e, &q);
        assert_eq!(
            e.store.estimates().unwrap().len(),
            1,
            "unchanged but nonexhausted quota cannot prove credits-only billing"
        );
        q.updated_at += 30;
        q.windows[0].used_percent = 100.0;
        observe(&e, &q);
        event(&e, &q, "credits", 300, false);
        q.updated_at += 30;
        q.credits.as_mut().unwrap().balance = Some("970".into());
        observe(&e, &q);
        let all = e.store.estimates().unwrap();
        let credit = all.iter().find_map(|v| v.credit()).unwrap();
        assert_eq!(credit.cost, 3.0);
        assert_eq!(credit.consumed_credits, 10.0);
        assert_eq!(credit.value_per1000, Some(300.0));
        assert_eq!(all.iter().find_map(|v| v.quota()).unwrap().cost, 1.0);
    }
    #[test]
    fn normal_resets_topups_and_missing_prices_rebaseline_without_user_action() {
        let (_dir, e, mut q) = setup(false);
        observe(&e, &q);
        event(&e, &q, "before", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        q.updated_at += 30;
        q.windows[0].used_percent = 0.0;
        q.windows[0].resets_at = Some(now() + 20000);
        observe(&e, &q);
        event(&e, &q, "after", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent = 10.0;
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 2.0);
        event(&e, &q, "topup", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent = 20.0;
        q.credits.as_mut().unwrap().balance = Some("2000".into());
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 2.0);
        e.store
            .save_prices(&[ModelPrice {
                id: "model".into(),
                ..Default::default()
            }])
            .unwrap();
        event(&e, &q, "unpriced", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent = 30.0;
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 2.0);
    }
    #[test]
    fn switch_seals_old_account_and_follows_new_account_without_reconsent() {
        let (_dir, e, mut q) = setup(true);
        let mut s = e.store.settings().unwrap();
        s.accounts.push(Account {
            id: "b".into(),
            name: "B".into(),
            provider: "codex".into(),
            connections: vec![AccountConnection {
                source_id: "s".into(),
                profile_id: Some("home:two".into()),
            }],
            ..Default::default()
        });
        e.store.save_settings(&s).unwrap();
        observe(&e, &q);
        event(&e, &q, "old", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        crate::accounts::codex_switched(&e.store, &s.sources[0], &json!("switch-1")).unwrap();
        assert_eq!(e.store.estimates().unwrap()[0].status, "completed");
        assert!(enabled(&e.store.settings().unwrap().sources[0]));
        q.account_id = "b".into();
        q.updated_at += 30;
        observe(&e, &q);
        event(&e, &q, "new", 200, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        crate::accounts::codex_switched(&e.store, &s.sources[0], &json!("switch-1")).unwrap();
        let all = e.store.estimates().unwrap();
        assert_eq!(all.len(), 2);
        let new = all.iter().find(|v| v.account_key == "codex:b").unwrap();
        assert_eq!(new.status, "active");
        assert_eq!(new.cost, 2.0);
    }
    #[test]
    fn disabling_persists_frozen_values_and_reopening_does_not_duplicate() {
        let (dir, mut e, mut q) = setup(false);
        observe(&e, &q);
        event(&e, &q, "one", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        drop(e);
        e = Engine::open(dir.path()).unwrap();
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap().len(), 1);
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":false}),
        )
        .unwrap();
        let all = e.store.estimates().unwrap();
        assert_eq!(all[0].status, "completed");
        assert_eq!(all[0].cost, 1.0);
        assert!(
            e.store
                .advance_automatic(&q, &["s".into()], "scope")
                .is_err()
        );
    }
    #[test]
    fn fingerprints_reject_switched_credentials_and_source_changes_atomically() {
        let (_dir, e, q) = setup(true);
        let ids = vec!["s".into()];
        let config = e
            .store
            .automatic_fingerprint(&e.store.settings().unwrap(), &ids, "codex:a")
            .unwrap();
        crate::accounts::codex_switched(
            &e.store,
            &e.store.settings().unwrap().sources[0],
            &json!("new-login"),
        )
        .unwrap();
        assert!(e.store.advance_automatic(&q, &ids, &config).is_err());
        assert!(e.store.estimates().unwrap().is_empty());
        observe(&e, &q);
        let config = e
            .store
            .automatic_fingerprint(&e.store.settings().unwrap(), &ids, "codex:a")
            .unwrap();
        let mut settings = e.store.settings().unwrap();
        settings.sources[0].enabled = false;
        e.store.save_settings(&settings).unwrap();
        assert!(e.store.advance_automatic(&q, &ids, &config).is_err());
    }
    #[test]
    fn source_memberships_do_not_multiply_cost_and_cross_boundary_usage_is_skipped() {
        let (_dir, e, mut q) = setup(true);
        let mut settings = e.store.settings().unwrap();
        let mut copy = settings.sources[0].clone();
        copy.id = "copy".into();
        copy.path = "/fixture-copy".into();
        copy.codex_home_id = None;
        copy.account_id = "a".into();
        settings.sources.push(copy);
        e.store.save_settings(&settings).unwrap();
        let ids = vec!["copy".into(), "s".into()];
        let fingerprint = e
            .store
            .automatic_fingerprint(&e.store.settings().unwrap(), &ids, "codex:a")
            .unwrap();
        e.store.advance_automatic(&q, &ids, &fingerprint).unwrap();
        event(&e, &q, "shared", 100, false);
        e.store
            .db
            .execute("INSERT INTO event_sources VALUES('shared','copy')", [])
            .unwrap();
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        e.store.advance_automatic(&q, &ids, &fingerprint).unwrap();
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.0);
        event(&e, &q, "cross-boundary", 100, false);
        e.store.db.execute("UPDATE events SET payload=json_set(payload,'$.intervalStart',?1) WHERE id='cross-boundary'", [q.updated_at - 1]).unwrap();
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        e.store.advance_automatic(&q, &ids, &fingerprint).unwrap();
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.0);
    }
    #[test]
    fn exhausted_unmapped_subpool_does_not_prove_credit_only_usage() {
        let (_dir, _e, mut a) = setup(false);
        a.windows.push(QuotaWindow {
            id: "unmapped".into(),
            window_minutes: Some(60),
            used_percent: 100.0,
            ..a.windows[0].clone()
        });
        let mut b = a.clone();
        b.updated_at += 30;
        b.credits.as_mut().unwrap().balance = Some("990".into());
        assert!(interval_kind(&a, &b).is_none());
        a.windows[0].used_percent = 100.0;
        b.windows[0].used_percent = 100.0;
        assert_eq!(interval_kind(&a, &b), Some("credits"));
        a.credits.as_mut().unwrap().has_credits = false;
        b.credits.as_mut().unwrap().has_credits = false;
        b.credits_updated_at = Some(a.updated_at);
        b.windows[0].used_percent = 100.0;
        assert!(interval_kind(&a, &b).is_none());
    }
    #[test]
    fn weekly_rounded_intervals_do_not_pollute_direct_weekly_value() {
        let (_dir, e, mut q) = setup(false);
        let mut five = q.windows[0].clone();
        five.id = "five".into();
        five.window_minutes = Some(crate::contracts::FIVE_HOUR_MINUTES);
        q.windows.push(five);
        observe(&e, &q);
        event(&e, &q, "week-rounded", 100, false);
        q.updated_at += 30;
        q.windows[1].used_percent += 5.0;
        observe(&e, &q);
        event(&e, &q, "week-advances", 200, false);
        q.updated_at += 30;
        q.windows[1].used_percent += 10.0;
        q.windows[0].used_percent += 5.0;
        observe(&e, &q);
        let rows = e.store.estimates().unwrap();
        let value = rows[0].quota().unwrap();
        assert_eq!(value.cost, 3.0);
        assert_eq!(value.five_hour_value, Some(20.0));
        assert_eq!(value.weekly_direct_value, Some(40.0));
        let mut stale = q.clone();
        stale.updated_at += 30;
        stale.credits_updated_at = Some(q.updated_at);
        stale.windows[1].used_percent += 5.0;
        assert!(interval_kind(&q, &stale).is_none());
    }
    #[test]
    fn shared_manual_sampling_uses_only_the_interval_and_switch_keeps_valid_values() {
        let (_dir, e, mut q) = setup(true);
        q.updated_at = now() - 100;
        let manual = e
            .store
            .begin_estimate(&q, "week", vec!["s".into()], None)
            .unwrap();
        event(&e, &q, "manual-shared", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        e.store.quota(&q).unwrap();
        let calculated = e.store.estimate_for(&manual.id, false).unwrap();
        assert_eq!(calculated.quota().unwrap().weekly_value, Some(10.0));
        crate::accounts::codex_switched(
            &e.store,
            &e.store.settings().unwrap().sources[0],
            &json!("manual-switch"),
        )
        .unwrap();
        assert_eq!(
            e.store.estimate_for(&manual.id, false).unwrap().status,
            "pending"
        );
        event(&e, &q, "after-switch", 500, false);
        let result = e.store.finish_estimate_for(&manual.id, false).unwrap();
        assert_eq!(result.status, "completed");
        assert_eq!(result.cost, 1.0);
        assert_eq!(result.quota().unwrap().weekly_value, Some(10.0));
    }
    #[test]
    fn competing_background_processes_do_not_collect_at_the_same_time() {
        let (root, mut e, _q) = setup(false);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.path().join("automatic-valuation.lock"))
            .unwrap();
        let _held = crate::file_lock::FileLock::try_exclusive(file).unwrap();
        assert_eq!(
            e.call("autoEstimates.tick", json!({})).unwrap()["changed"],
            false
        );
        assert_eq!(e.store.automatic_status("s").unwrap(), Value::Null);
    }

    #[test]
    fn opting_in_during_manual_sampling_does_not_pause_or_rebase_the_manual_record() {
        let (_dir, mut e, mut q) = setup(false);
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":false}),
        )
        .unwrap();
        q.updated_at = now() - 60;
        let manual = e
            .store
            .begin_estimate(&q, "week", vec!["s".into()], None)
            .unwrap();
        event(&e, &q, "manual-first", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        e.store.quota(&q).unwrap();
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":true,"confirmed":true}),
        )
        .unwrap();
        let preserved = e.store.estimate_for(&manual.id, false).unwrap();
        assert_eq!(preserved.status, "active");
        assert_eq!(preserved.cost, 1.0);
        assert_eq!(preserved.started_at, manual.started_at);
        assert_eq!(preserved.checkpoint_at, q.updated_at);
        observe(&e, &q);
        assert_eq!(
            e.store.automatic_status("s").unwrap()["state"],
            "waitingManual"
        );
        e.call(
            "autoEstimates.configure",
            json!({"sourceId":"s","enabled":false}),
        )
        .unwrap();
        assert_eq!(
            e.store.estimate_for(&manual.id, false).unwrap().status,
            "active"
        );
        event(&e, &q, "manual-next", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 10.0;
        e.store.quota(&q).unwrap();
        let finished = e.store.finish_estimate_for(&manual.id, false).unwrap();
        assert_eq!(finished.cost, 2.0);
        assert_eq!(finished.quota().unwrap().weekly_value, Some(10.0));
    }
    #[test]
    fn manual_sampling_has_priority_and_long_gaps_do_not_invent_costs() {
        let (_dir, e, mut q) = setup(false);
        q.updated_at = now();
        let manual = e
            .store
            .begin_estimate(&q, "week", vec!["s".into()], None)
            .unwrap();
        observe(&e, &q);
        assert_eq!(
            e.store.automatic_status("s").unwrap()["state"],
            "waitingManual"
        );
        e.store.finish_estimate_for(&manual.id, false).unwrap();
        observe(&e, &q);
        event(&e, &q, "long-gap", 100, false);
        q.updated_at += 301;
        q.windows[0].used_percent += 10.0;
        observe(&e, &q);
        assert!(
            !e.store
                .estimates()
                .unwrap()
                .iter()
                .any(|v| v.sampling_mode == "automatic")
        );
    }
}
