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
const BASELINE_VERSION: u32 = 3;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoValuationConfig {
    pub enabled: bool,
    pub consent_version: u32,
    /// Read old consent modes for migration; classification now always follows evidence.
    #[serde(skip_serializing)]
    pub mode: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoSegment {
    /// Weekly windows keep their own delayed counter and usage interval.
    pub pool: String,
    pub prices: Vec<Value>,
    #[serde(default = "segment_valid")]
    pub valid: bool,
    pub note: String,
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
fn segment_valid() -> bool {
    true
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Baseline {
    version: u32,
    quota: Option<QuotaSnapshot>,
    config: String,
    source_ids: Vec<String>,
    pending: BTreeMap<String, QuotaSnapshot>,
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
        !same_cycle(old.resets_at, new.resets_at, b.updated_at)
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
    } else if !quota_changed && credit_unchanged {
        Some("idle")
    } else {
        None
    }
}

pub(crate) fn same_cycle(anchor: Option<i64>, observed: Option<i64>, stamp: i64) -> bool {
    anchor
        .zip(observed)
        .is_some_and(|(a, b)| a > stamp && b > stamp && a.abs_diff(b) <= 10)
}

fn anchored(old: &QuotaSnapshot, next: &QuotaSnapshot) -> QuotaSnapshot {
    let mut value = next.clone();
    for w in &mut value.windows {
        if let Some(previous) = old.windows.iter().find(|v| window_id(v) == window_id(w))
            && same_cycle(previous.resets_at, w.resets_at, next.updated_at)
            && w.used_percent >= previous.used_percent
        {
            w.resets_at = previous.resets_at;
        }
    }
    value
}

pub(crate) fn recalculate(e: &mut Estimate) {
    let accepted: Vec<_> = e
        .automatic_segments
        .iter()
        .filter(|s| s.valid)
        .cloned()
        .collect();
    e.cost = accepted
        .iter()
        .filter(|s| s.pool != "weekly")
        .map(|s| s.cost)
        .sum();
    e.total_tokens = accepted
        .iter()
        .filter(|s| s.pool != "weekly")
        .map(|s| s.tokens)
        .sum();
    e.priced_tokens = accepted
        .iter()
        .filter(|s| s.pool != "weekly")
        .map(|s| s.priced_tokens)
        .sum();
    let used: f64 = accepted
        .iter()
        .filter(|s| s.pool != "weekly")
        .map(|s| s.consumed)
        .sum();
    let weekly: f64 = accepted
        .iter()
        .map(|s| {
            if s.pool == "weekly" {
                s.consumed
            } else {
                s.weekly_consumed
            }
        })
        .sum();
    let weekly_cost: f64 = accepted
        .iter()
        .map(|s| {
            if s.pool == "weekly" {
                s.cost
            } else {
                s.weekly_cost
            }
        })
        .sum();
    let main = crate::estimates::finite_value(e.cost, used, 100.0)
        .filter(|_| e.total_tokens > 0 && e.priced_tokens == e.total_tokens);
    let week = crate::estimates::finite_value(weekly_cost, weekly, 100.0).filter(|_| {
        accepted
            .iter()
            .filter(|s| s.pool == "weekly" || s.weekly_consumed > 0.0)
            .all(|s| s.tokens > 0 && s.tokens == s.priced_tokens)
    });
    match e {
        Estimate::Quota(v) => {
            v.consumed_percent = used;
            if v.valuation_mode == "fiveHour" {
                v.five_hour_value = main;
                v.weekly_direct_value = week;
                v.weekly_value = week;
                v.weekly_ratio_value = main
                    .zip(v.capacity.ratio)
                    .map(|(a, b)| a * b)
                    .filter(|v| v.is_finite());
            } else {
                v.weekly_value = main;
            }
        }
        Estimate::Credit(v) => {
            v.consumed_credits = used;
            v.value_per500 = main.and_then(|_| crate::estimates::finite_value(v.cost, used, 500.0));
            v.value_per1000 =
                main.and_then(|_| crate::estimates::finite_value(v.cost, used, 1000.0));
        }
    }
    e.calculation_status = if e.has_value() { "ready" } else { "noUsage" }.into();
    e.calculation_note =
        if e.has_value() && (used > 0.0 && used < 5.0 || weekly > 0.0 && weekly < 5.0) {
            "自动估值 · 小样本，数值会随用量积累更新"
        } else if e.has_value() {
            "自动估值 · 仅计入可确认归属的有效区间"
        } else {
            "等待可确认归属且价格完整的样本"
        }
        .into();
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

    /// Persist one authoritative observation and independently pair each pool's rounded counter.
    pub fn advance_automatic(
        &self,
        snapshot: &QuotaSnapshot,
        ids: &[String],
        config: &str,
    ) -> Result<()> {
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
        let key = format!("{}:{}", snapshot.provider, snapshot.account_id);
        ensure!(
            self.automatic_fingerprint(&settings, ids, &key)? == config,
            "采样过程中登录或来源设置变化"
        );
        let mut baseline: Baseline = self.auto_get(&baseline_key(&key))?;
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
        if !valid(snapshot) {
            self.invalidate_automatic_sources(ids)?;
            self.automatic_status_set(ids, "waiting", "等待有效的实时订阅限额", &key)?;
            tx.commit()?;
            return Ok(());
        }
        let previous = baseline.quota.clone().filter(|q| {
            baseline.version == BASELINE_VERSION
                && baseline.config == config
                && baseline.source_ids == ids
                && q.account_id == snapshot.account_id
                && q.provider == snapshot.provider
        });
        let Some(old) = previous else {
            self.invalidate_automatic_sources(ids)?;
            self.auto_put(
                &baseline_key(&key),
                &Baseline {
                    version: BASELINE_VERSION,
                    quota: Some(snapshot.clone()),
                    config: config.into(),
                    source_ids: ids.to_vec(),
                    pending: BTreeMap::new(),
                },
            )?;
            self.automatic_status_set(ids, "collecting", "已建立基线，等待首个可靠样本", &key)?;
            tx.commit()?;
            return Ok(());
        };
        if snapshot.updated_at <= old.updated_at {
            tx.commit()?;
            return Ok(());
        }
        let q = anchored(&old, snapshot);
        if old.plan != q.plan {
            self.invalidate_automatic_sources(ids)?;
        }
        let kind = interval_kind(&old, &q);
        let mut accepted = false;
        if let Some(kind) = kind {
            let windows: Vec<_> = q
                .windows
                .iter()
                .filter(|w| crate::estimates::eligible(&q, w))
                .collect();
            // (pending counter, containing record, independent weekly segment)
            let mut targets: Vec<(Option<&QuotaWindow>, Option<&QuotaWindow>, bool)> = Vec::new();
            if kind != "credits" {
                for window in &windows {
                    let five = (window.window_minutes == Some(crate::contracts::WEEK_MINUTES))
                        .then(|| {
                            windows
                                .iter()
                                .find(|v| {
                                    v.group_id == window.group_id
                                        && v.window_minutes
                                            == Some(crate::contracts::FIVE_HOUR_MINUTES)
                                })
                                .copied()
                        })
                        .flatten();
                    targets.push((Some(window), Some(five.unwrap_or(window)), five.is_some()));
                }
            }
            let exhausted = q
                .windows
                .iter()
                .any(|w| crate::estimates::eligible(&q, w) && w.used_percent == 100.0);
            if kind == "credits" || kind == "idle" && exhausted {
                targets.push((None, None, false));
            }
            // Quota usage cannot later be charged to credits, or vice versa.
            if kind == "quota" || !exhausted {
                baseline.pending.remove("credits");
            }
            if kind == "credits" {
                baseline.pending.retain(|id, _| id == "credits");
            }
            for (window, record_window, weekly) in targets {
                let pool = window.map(window_id).unwrap_or("credits").to_owned();
                let from = baseline
                    .pending
                    .get(&pool)
                    .cloned()
                    .unwrap_or_else(|| old.clone());
                let consumed = if let Some(w) = window {
                    from.windows
                        .iter()
                        .find(|v| window_id(v) == window_id(w))
                        .map(|v| w.used_percent - v.used_percent)
                        .unwrap_or(0.0)
                } else {
                    balance(&from)
                        .zip(balance(&q))
                        .map(|(a, b)| (a - b) as f64 / 1e18)
                        .unwrap_or(0.0)
                };
                let group = window
                    .filter(|_| crate::providers::antigravity(&q.provider))
                    .map(crate::antigravity::window_group);
                let mut sample = EstimateCommon {
                    sampling_mode: "automatic".into(),
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
                    started_at: from.updated_at,
                    checkpoint_at: q.updated_at,
                    ..Default::default()
                };
                self.calculate_usage(&mut sample, group)?;
                let reliable = sample.calculation_status == "ready"
                    && sample.cost.is_finite()
                    && sample.cost >= 0.0
                    && sample.priced_tokens == sample.total_tokens;
                if consumed == 0.0 && (reliable || sample.calculation_status == "noUsage") {
                    baseline.pending.entry(pool).or_insert(from);
                    continue;
                }
                // Every positive counter observation consumes this pending span even when unusable.
                baseline.pending.insert(pool, q.clone());
                if consumed <= 0.0
                    || !reliable
                    || crate::estimates::finite_value(sample.cost, consumed, 1000.0).is_none()
                {
                    continue;
                }
                let win = record_window.map(window_id).unwrap_or("credits");
                let existing = self.estimates()?.into_iter().find(|e| {
                    e.sampling_mode == "automatic"
                        && e.status == "active"
                        && e.account_key == key
                        && e.window_id == win
                });
                let mut record = if let Some(e) = existing {
                    e
                } else {
                    let mut common = sample.clone();
                    common.id = format!(
                        "auto-{}-{}",
                        win,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_nanos()
                    );
                    common.status = "active".into();
                    common.window_id = win.into();
                    common.window_name = record_window
                        .map(|w| w.name.clone())
                        .unwrap_or("Credits".into());
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
                    common.calculation_version = 3;
                    if let Some(w) = record_window {
                        Estimate::Quota(QuotaEstimate {
                            common,
                            group_id: group.map(str::to_owned),
                            valuation_mode: if w.window_minutes
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
                    }
                };
                record.automatic_segments.push(AutoSegment {
                    pool: if weekly { "weekly" } else { "primary" }.into(),
                    prices: sample.prices.clone(),
                    valid: true,
                    started_at: from.updated_at,
                    ended_at: q.updated_at,
                    reset_at: window.and_then(|w| w.resets_at).unwrap_or(0),
                    cost: sample.cost,
                    tokens: sample.total_tokens,
                    priced_tokens: sample.priced_tokens,
                    consumed,
                    ..Default::default()
                });
                record.started_at = record.started_at.min(from.updated_at);
                record.checkpoint_at = q.updated_at;
                for price in sample.prices {
                    if !record.prices.contains(&price) {
                        record.prices.push(price);
                    }
                }
                if let Estimate::Quota(v) = &mut record {
                    v.capacity = self.capacity_for_group(&key, group.unwrap_or(""), now())?;
                }
                recalculate(&mut record);
                self.save_estimate(&record)?;
                accepted = true;
            }
        } else {
            // Mixed consumption, top-ups, reset, plan changes and data gaps break all pending pairs.
            baseline.pending.clear();
        }
        baseline.quota = Some(q);
        self.auto_put(&baseline_key(&key), &baseline)?;
        self.automatic_status_set(
            ids,
            if accepted { "sampling" } else { "waiting" },
            if accepted {
                "已更新可靠样本；小样本也会立即显示"
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
        ensure!(
            !enable || (p["confirmed"] == true && source.provider != "deepseek"),
            "请确认已纳入影响订阅额度的所有设备用量"
        );
        if let Some(expected) = p.get("baseConfig") {
            let expected: Option<AutoValuationConfig> = serde_json::from_value(expected.clone())?;
            let normalize = |mut config: Option<AutoValuationConfig>| {
                if let Some(c) = &mut config {
                    c.mode.clear();
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
            mode: String::new(),
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
            let last: Baseline = self.store.auto_get(&baseline_key(&key))?;
            let fingerprint = self.store.automatic_fingerprint(&settings, &ids, &key)?;
            let same_scope = last.version == BASELINE_VERSION
                && last.config == fingerprint
                && last.source_ids == ids;
            let observed: Option<String> = self.store.db.query_row("SELECT payload FROM quota_history WHERE account_key=?1 ORDER BY stamp DESC,id DESC LIMIT 1", [&key], |r|r.get(0)).optional()?;
            let observed = observed
                .map(|raw| serde_json::from_str::<QuotaSnapshot>(&raw))
                .transpose()?
                .filter(|q| {
                    // A cache can predate a login switch (including A -> B -> A).
                    // Only an established, unchanged scope may reuse it; a new scope
                    // needs a fresh read after identity verification and log sync.
                    same_scope
                        && valid(q)
                        && (now() - q.updated_at).abs() <= 120
                        && last
                            .quota
                            .as_ref()
                            .is_some_and(|p| q.updated_at > p.updated_at)
                });
            if (0..30).contains(&(now() - attempted)) && observed.is_none() {
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
            if same_scope
                && last
                    .quota
                    .as_ref()
                    .is_some_and(|q| now() - q.updated_at < 30 && now() >= q.updated_at)
                && observed.is_none()
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
            if observed.is_none() {
                self.store.auto_put(&attempt_key, &now())?;
            }
            let outcome = (|| -> Result<()> {
                self.verify_sampling_identities(&ids, &key)?;
                self.sync_estimate_sources(&ids)?;
                let rows = if let Some(q) = observed {
                    vec![q]
                } else {
                    self.read_quotas(&json!({"accountKey":key,"samplingSourceIds":ids}))?
                };
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
                        category: if api { "api" } else { "subscription" }.into(),
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
    #[cfg(unix)]
    #[test]
    fn switching_back_requires_a_new_quota_baseline_before_pairing_shared_usage() {
        use crate::codex_auth::files;
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let home = std::fs::canonicalize(home).unwrap();
        let auth = |user: &str| {
            let token = URL_SAFE_NO_PAD.encode(
                json!({"https://api.openai.com/auth":{
                    "chatgpt_user_id":user,"chatgpt_account_id":user,"chatgpt_plan_type":"plus"
                }})
                .to_string(),
            );
            json!({"auth_mode":"chatgpt","tokens":{
                "access_token":format!("header.{token}.signature"),
                "refresh_token":"fixture","account_id":user
            }})
        };
        let first = auth("a");
        files::atomic(&home.join("auth.json"), &first).unwrap();
        files::save_profile(&home, "one", "A", &first).unwrap();
        files::save_profile(&home, "two", "B", &auth("b")).unwrap();
        let mut e = Engine::open(&root.path().join("data")).unwrap();
        let source = Source {
            id: "s".into(),
            name: "Shared".into(),
            provider: "codex".into(),
            path: home.to_string_lossy().into(),
            codex_home_id: Some("home".into()),
            auto_valuation: Some(AutoValuationConfig {
                enabled: true,
                consent_version: CONSENT_VERSION,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut q = QuotaSnapshot {
            provider: "codex".into(),
            account_id: "a".into(),
            source_id: "s".into(),
            plan: Some("plus".into()),
            origin: "live".into(),
            updated_at: now() - 100,
            credits: Some(CreditsSnapshot {
                has_credits: true,
                balance: Some("1000".into()),
                ..Default::default()
            }),
            windows: vec![QuotaWindow {
                id: "week".into(),
                name: "7d".into(),
                window_minutes: Some(crate::contracts::WEEK_MINUTES),
                used_percent: 10.0,
                resets_at: Some(now() + 10000),
                ..Default::default()
            }],
            ..Default::default()
        };
        let response = root.path().join("quota.json");
        let mut live = q.clone();
        live.windows[0].used_percent = 30.0;
        std::fs::write(&response, serde_json::to_vec(&live).unwrap()).unwrap();
        e.store
            .save_settings(&Settings {
                sources: vec![
                    source.clone(),
                    Source {
                        id: "query".into(),
                        provider: "codex".into(),
                        account_id: "a".into(),
                        path: home.to_string_lossy().into(),
                        quota_command: format!(
                            "cat {}",
                            crate::process::quote(&response.to_string_lossy())
                        ),
                        ..Default::default()
                    },
                ],
                accounts: vec![
                    Account {
                        id: "a".into(),
                        name: "A".into(),
                        provider: "codex".into(),
                        quota_profile_id: Some("home:one".into()),
                        quota_source_id: Some("query".into()),
                        ..Default::default()
                    },
                    Account {
                        id: "b".into(),
                        name: "B".into(),
                        provider: "codex".into(),
                        quota_profile_id: Some("home:two".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            })
            .unwrap();
        e.store
            .save_prices(&[ModelPrice {
                id: "model".into(),
                input: Some(0.01),
                ..Default::default()
            }])
            .unwrap();
        observe(&e, &q);
        q.updated_at += 30;
        e.store.quota(&q).unwrap();
        files::atomic(&home.join("auth.json"), &auth("b")).unwrap();
        crate::accounts::codex_switched(&e.store, &source, &json!("switch-b")).unwrap();
        event(&e, &q, "other-account", 200, false);
        files::atomic(&home.join("auth.json"), &first).unwrap();
        crate::accounts::codex_switched(&e.store, &source, &json!("switch-a")).unwrap();

        let switched_at = now();
        e.tick_automatic().unwrap();
        let baseline: Baseline = e.store.auto_get(&baseline_key("codex:a")).unwrap();
        let mut fresh = baseline
            .quota
            .expect("a successful tick establishes a baseline");
        assert!(
            fresh.updated_at >= switched_at,
            "a pre-switch cached snapshot cannot start the new interval"
        );
        assert_eq!(fresh.windows[0].used_percent, 30.0);
        assert!(baseline.pending.is_empty());
        event(&e, &fresh, "this-account", 100, false);
        fresh.updated_at += 30;
        fresh.credits_updated_at = Some(fresh.updated_at);
        fresh.windows[0].used_percent += 5.0;
        observe(&e, &fresh);
        let records = e.store.estimates().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].account_key, "codex:a");
        assert_eq!(
            records[0].cost, 1.0,
            "usage from B must not be paired with A's quota"
        );
        assert_eq!(records[0].quota().unwrap().weekly_value, Some(20.0));
        // Normal refreshes may still reuse a newer snapshot in the unchanged scope.
        event(&e, &fresh, "this-account-again", 200, false);
        fresh.updated_at += 30;
        fresh.credits_updated_at = Some(fresh.updated_at);
        fresh.windows[0].used_percent += 10.0;
        e.store.quota(&fresh).unwrap();
        e.tick_automatic().unwrap();
        let records = e.store.estimates().unwrap();
        assert_eq!(records[0].checkpoint_at, fresh.updated_at);
        assert_eq!(records[0].cost, 3.0);
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
        assert_eq!(value.weekly_direct_value, Some(60.0));
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

    #[test]
    fn small_samples_display_immediately_and_zero_snapshots_preserve_cost() {
        let (_dir, e, mut q) = setup(false);
        observe(&e, &q);
        event(&e, &q, "rounded-1", 100, false);
        q.updated_at += 30;
        observe(&e, &q);
        event(&e, &q, "rounded-2", 200, false);
        q.updated_at += 30;
        observe(&e, &q);
        assert!(e.store.estimates().unwrap().is_empty());
        q.updated_at += 30;
        q.windows[0].used_percent += 1.0;
        observe(&e, &q);
        let all = e.store.estimates().unwrap();
        assert_eq!(all[0].cost, 3.0);
        assert_eq!(all[0].quota().unwrap().weekly_value, Some(300.0));
        assert_eq!(all[0].calculation_status, "ready");
        assert!(all[0].calculation_note.contains("小样本"));
        event(&e, &q, "next", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 3.0;
        observe(&e, &q);
        assert_eq!(
            e.store.estimates().unwrap()[0]
                .quota()
                .unwrap()
                .weekly_value,
            Some(100.0)
        );
    }

    #[test]
    fn reset_jitter_uses_fixed_anchor_and_uncertain_pending_is_discarded() {
        let (_dir, e, mut q) = setup(false);
        observe(&e, &q);
        let reset = q.windows[0].resets_at.unwrap();
        event(&e, &q, "small", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 1.0;
        q.windows[0].resets_at = Some(reset + 6);
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.0);
        event(&e, &q, "pending", 100, false);
        q.updated_at += 30;
        q.windows[0].resets_at = Some(reset + 9);
        observe(&e, &q);
        event(&e, &q, "drift", 100, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 1.0;
        q.windows[0].resets_at = Some(reset + 12);
        observe(&e, &q);
        assert_eq!(
            e.store.estimates().unwrap()[0].cost,
            1.0,
            "jitter must not accumulate beyond the original 10-second anchor"
        );
        event(&e, &q, "unknown", 100, false);
        e.store.db.execute("UPDATE events SET payload=json_set(payload,'$.billing.category','unknown') WHERE id='unknown'",[]).unwrap();
        q.updated_at += 30;
        observe(&e, &q);
        event(&e, &q, "confirmed", 50, false);
        q.updated_at += 30;
        q.windows[0].used_percent += 0.5;
        observe(&e, &q);
        assert_eq!(e.store.estimates().unwrap()[0].cost, 1.5);
    }

    #[test]
    fn tiny_credit_sample_after_zero_snapshots_is_finite() {
        let (_dir, e, mut q) = setup(false);
        q.windows[0].used_percent = 100.0;
        observe(&e, &q);
        event(&e, &q, "credit-rounded", 100, false);
        q.updated_at += 30;
        observe(&e, &q);
        q.updated_at += 30;
        q.credits.as_mut().unwrap().balance = Some("999.9".into());
        observe(&e, &q);
        let all = e.store.estimates().unwrap();
        let value = all[0].credit().unwrap();
        assert_eq!(value.consumed_credits, 0.1);
        assert_eq!(value.value_per1000, Some(10000.0));
    }
}
