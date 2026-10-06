//! Manual quota valuation. Account association is not billing evidence.
use crate::{Engine, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Estimate {
    pub valuation_mode: String,
    pub quota_plan: Option<String>,
    pub segment_started_at: i64,
    pub weekly_window_id: String,
    pub weekly_reset_at: i64,
    pub weekly_baseline_percent: f64,
    pub weekly_checkpoint_percent: f64,
    pub segments: Vec<EstimateSegment>,
    pub five_hour_value: Option<f64>,
    pub weekly_direct_value: Option<f64>,
    pub weekly_ratio_value: Option<f64>,
    pub capacity: crate::capacity::Capacity,
    pub kind: String,
    pub baseline_balance: Option<String>,
    pub checkpoint_balance: Option<String>,
    pub consumed_credits: f64,
    pub value_per500: Option<f64>,
    pub value_per1000: Option<f64>,
    pub checkpoint_windows: Vec<QuotaWindow>,
    pub id: String,
    pub account_key: String,
    pub window_id: String,
    pub window_name: String,
    pub source_ids: Vec<String>,
    pub source_names: Vec<String>,
    pub source_config: String,
    pub status: String,
    pub reason: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub checkpoint_at: i64,
    pub reset_at: i64,
    pub baseline_percent: f64,
    pub checkpoint_percent: f64,
    pub consumed_percent: f64,
    pub cost: f64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub excluded_api_tokens: u64,
    pub weekly_value: Option<f64>,
    pub calculation_note: String,
    pub prices: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EstimateSegment {
    pub started_at: i64,
    pub ended_at: i64,
    pub five_percent: f64,
    pub weekly_percent: Option<f64>,
    pub cost: f64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub excluded_api_tokens: u64,
    pub prices: Vec<Value>,
    pub valid: bool,
    pub note: String,
}

impl Estimate {
    pub fn is_credit(&self) -> bool {
        self.kind == "credits"
    }
}

// Exact subtraction before converting the small delta to a display/valuation float.
// Reject unsupported precision rather than rounding a balance into a false debit.
pub fn credit_units(value: &str) -> Option<i128> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || fraction.len() > 18
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    whole
        .parse::<i128>()
        .ok()?
        .checked_mul(1_000_000_000_000_000_000)?
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction
                .parse::<i128>()
                .ok()?
                .checked_mul(10_i128.pow(18 - fraction.len() as u32))?
        })
}
fn credit_balance(q: &QuotaSnapshot) -> Option<&str> {
    let c = q.credits.as_ref()?;
    if c.unlimited {
        return None;
    }
    let balance = c.balance.as_deref()?;
    credit_units(balance)?;
    Some(balance)
}

pub fn window_id(w: &QuotaWindow) -> &str {
    if w.id.is_empty() { &w.name } else { &w.id }
}

// Sub-pools overlap; without an explicit mapping we must not reuse all-account usage.
pub fn eligible(q: &QuotaSnapshot, w: &QuotaWindow) -> bool {
    (w.window_minutes == Some(300)
        && crate::capacity::overall(q, 300).is_some_and(|v| window_id(v) == window_id(w)))
        || (w.window_minutes == Some(10080)
            && (q
                .windows
                .iter()
                .filter(|v| v.window_minutes == Some(10080))
                .count()
                == 1
                || (q.provider == "claude" && (w.id == "seven_day" || w.name == "7d")))
            && q.provider != "agy")
}

fn source_config(settings: &Settings, ids: &[String], account_key: &str) -> Result<String> {
    let mut values = Vec::new();
    ensure!(!ids.is_empty(), "请选择用量数据源");
    for id in ids {
        let s = settings
            .sources
            .iter()
            .find(|s| &s.id == id)
            .context("采样数据源已删除")?;
        ensure!(
            s.enabled && format!("{}:{}", s.provider, s.account_id) == account_key,
            "采样数据源已停用或账户关联发生变化"
        );
        ensure!(
            !["agy", "deepseek"].contains(&s.provider.as_str()),
            "该数据源没有可采集的 Token 用量"
        );
        let host = s
            .host_id
            .as_ref()
            .map(|id| {
                settings
                    .hosts
                    .iter()
                    .find(|h| &h.id == id && h.enabled)
                    .context("采样主机已停用或删除")
            })
            .transpose()?;
        values.push(json!({"source":s,"host":host}));
    }
    Ok(hash(&serde_json::to_string(&values)?))
}

impl Store {
    pub fn quota_order(&self) -> Result<Vec<String>> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key='quotaOrder'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let mut order: Vec<String> = raw
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        let settings = self.settings()?;
        let keys: Vec<_> = settings
            .accounts
            .iter()
            .map(|a| format!("{}:{}", a.provider, a.id))
            .collect();
        order.retain(|key| keys.contains(key));
        // Initial ordering follows the existing alphabetical presentation; new accounts append.
        let mut additions: Vec<_> = settings
            .accounts
            .iter()
            .filter(|a| !order.contains(&format!("{}:{}", a.provider, a.id)))
            .collect();
        additions.sort_by(|a, b| a.name.cmp(&b.name));
        order.extend(additions.iter().map(|a| format!("{}:{}", a.provider, a.id)));
        Ok(order)
    }
    pub fn set_quota_order(&self, keys: Vec<String>) -> Result<Vec<String>> {
        let all = self.quota_order()?;
        ensure!(
            keys.iter().all(|k| all.contains(k)),
            "账户不存在，请刷新后重试"
        );
        let mut order = Vec::new();
        for key in keys {
            ensure!(!order.contains(&key), "账户排序包含重复项");
            order.push(key);
        }
        order.extend(all);
        let mut unique = Vec::new();
        for key in order {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        self.db.execute("INSERT INTO kv VALUES('quotaOrder',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&unique)?])?;
        Ok(unique)
    }
    pub fn estimates(&self) -> Result<Vec<Estimate>> {
        self.read_estimates(false)
    }
    fn active_estimates(&self) -> Result<Vec<Estimate>> {
        self.read_estimates(true)
    }
    fn read_estimates(&self, active_only: bool) -> Result<Vec<Estimate>> {
        let sql = if active_only {
            "SELECT payload FROM (SELECT payload FROM quota_estimates WHERE status='active' UNION ALL SELECT payload FROM credit_estimates WHERE status='active') ORDER BY json_extract(payload,'$.startedAt') DESC"
        } else {
            "SELECT payload FROM (SELECT payload FROM quota_estimates UNION ALL SELECT payload FROM credit_estimates) ORDER BY json_extract(payload,'$.startedAt') DESC"
        };
        let mut stmt = self.db.prepare(sql)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter()
            .map(|raw| Ok(serde_json::from_str(raw)?))
            .collect()
    }
    pub fn estimate(&self, id: &str) -> Result<Estimate> {
        let raw: String = self
            .db
            .query_row(
                "SELECT payload FROM quota_estimates WHERE id=?1 UNION ALL SELECT payload FROM credit_estimates WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .context("采样记录不存在")?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub(crate) fn save_estimate(&self, e: &Estimate) -> Result<()> {
        let table = if e.is_credit() {
            "credit_estimates"
        } else {
            "quota_estimates"
        };
        self.db.execute(&format!("INSERT INTO {table} VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET status=excluded.status,payload=excluded.payload"),params![e.id,e.account_key,e.status,serde_json::to_string(e)?])?;
        Ok(())
    }
    pub fn reconcile_estimates(&self, settings: &Settings) -> Result<()> {
        for mut e in self
            .active_estimates()?
            .into_iter()
            .filter(|e| e.status == "active")
        {
            let enabled = settings.accounts.iter().any(|a| {
                format!("{}:{}", a.provider, a.id) == e.account_key
                    && a.quota_enabled
                    && !a.archived
            });
            if !enabled
                || source_config(settings, &e.source_ids, &e.account_key)
                    .as_ref()
                    .ok()
                    != Some(&e.source_config)
            {
                e.status = "pending".into();
                e.reason = "账户或数据源配置发生变化；请确认后开始新一段".into();
                self.save_estimate(&e)?;
            }
        }
        Ok(())
    }
    pub fn pause_source_estimates(&self, source: &str) -> Result<()> {
        for mut e in self
            .active_estimates()?
            .into_iter()
            .filter(|e| e.status == "active" && e.source_ids.iter().any(|id| id == source))
        {
            e.status = "pending".into();
            e.reason = "用量同步失败，已保留上一次有效区间；请重试或确认后开始新一段".into();
            self.save_estimate(&e)?;
        }
        Ok(())
    }
    pub fn observe_estimates(&self, q: &QuotaSnapshot) -> Result<()> {
        // Historical log snapshots are not authoritative observations of the current cycle.
        if q.origin == "log" {
            return Ok(());
        }
        let key = format!("{}:{}", q.provider, q.account_id);
        for mut e in self
            .active_estimates()?
            .into_iter()
            .filter(|e| e.account_key == key && e.status == "active")
        {
            if e.is_credit() {
                self.observe_credit(&mut e, q)?;
                continue;
            }
            if e.valuation_mode == "fiveHour" {
                self.observe_five_hour(&mut e, q)?;
                continue;
            }
            let w = q.windows.iter().find(|w| window_id(w) == e.window_id);
            let config = source_config(&self.settings()?, &e.source_ids, &e.account_key);
            let failure = if q.error.is_some() {
                Some("限额查询失败，采样已暂停；请确认后重新开始")
            } else if config.as_ref().ok() != Some(&e.source_config) {
                Some("数据源配置发生变化，采样已暂停")
            } else if let Some(w) = w {
                if q.updated_at <= e.checkpoint_at {
                    continue;
                }
                if !w.used_percent.is_finite() || !(0.0..=100.0).contains(&w.used_percent) {
                    Some("额度数值异常")
                } else if w.resets_at != Some(e.reset_at)
                    || q.updated_at >= e.reset_at
                    || w.used_percent + 0.001 < e.checkpoint_percent
                {
                    Some("额度已重置或异常回升；确认后开始新一段")
                } else {
                    None
                }
            } else {
                Some("7d 额度池已变化或不可用")
            };
            if let Some(reason) = failure {
                e.status = "pending".into();
                e.reason = reason.into();
            } else if let Some(w) = w {
                e.checkpoint_at = q.updated_at;
                e.checkpoint_percent = w.used_percent;
            }
            self.calculate_estimate(&mut e)?;
            self.save_estimate(&e)?;
        }
        Ok(())
    }
    pub fn calculate_estimate(&self, e: &mut Estimate) -> Result<()> {
        if e.valuation_mode == "fiveHour" {
            return self.calculate_five_hour(e);
        }
        let (provider, account) = e.account_key.split_once(':').context("账户格式错误")?;
        e.cost = 0.0;
        e.total_tokens = 0;
        e.priced_tokens = 0;
        e.excluded_api_tokens = 0;
        e.prices.clear();
        e.weekly_value = None;
        e.value_per500 = None;
        e.value_per1000 = None;
        e.consumed_percent = e.checkpoint_percent - e.baseline_percent;
        e.consumed_credits = e
            .baseline_balance
            .as_deref()
            .and_then(credit_units)
            .zip(e.checkpoint_balance.as_deref().and_then(credit_units))
            .and_then(|(a, b)| a.checked_sub(b))
            .map(|n| n as f64 / 1e18)
            .unwrap_or(0.0);
        let mut boundary = false;
        // EXISTS avoids multiplying events observed through several selected sources.
        let mut stmt = self.db.prepare("SELECT e.payload,e.cost,e.priced_tokens,e.price FROM events e WHERE e.provider=?1 AND e.account_id=?2 AND e.stamp>?3 AND e.stamp<=?4 AND EXISTS(SELECT 1 FROM event_sources s WHERE s.event_id=e.id AND s.source_id IN (SELECT value FROM json_each(?5)))")?;
        let rows = stmt.query_map(
            params![
                provider,
                account,
                e.started_at,
                e.checkpoint_at,
                serde_json::to_string(&e.source_ids)?
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        for row in rows {
            let (raw, cost, priced, price) = row?;
            let event: UsageEvent = serde_json::from_str(&raw)?;
            if event.billing.category == "api" {
                e.excluded_api_tokens += event.tokens.total();
                continue;
            }
            if (event.provider == "codex" || event.attribution == "session-summary")
                && event.interval_start.is_none_or(|t| t < e.started_at)
            {
                boundary = true;
                continue;
            }
            e.total_tokens += event.tokens.total();
            e.priced_tokens += priced;
            e.cost += cost;
            if let Some(raw) = price {
                let p: Value = serde_json::from_str(&raw)?;
                if !e.prices.contains(&p) {
                    e.prices.push(p);
                }
            }
        }
        e.calculation_note = if boundary {
            "存在跨采样边界的累计用量，请在开始采样后新建会话再试"
        } else if e.total_tokens == 0 {
            "暂无可计入的订阅用量"
        } else if e.priced_tokens != e.total_tokens {
            "缺少模型价格，请补全价格后重新计算本次采样"
        } else if e.is_credit() && e.consumed_credits < 5.0 {
            "样本不足：至少消耗 5 credits"
        } else if !e.is_credit() && e.consumed_percent + 0.000001 < 5.0 {
            "样本不足：至少消耗 5 个百分点"
        } else if !e.cost.is_finite() || e.cost < 0.0 {
            "价格数据无效"
        } else {
            if e.is_credit() {
                e.value_per500 = Some(e.cost * 500.0 / e.consumed_credits);
                e.value_per1000 = Some(e.cost * 1000.0 / e.consumed_credits);
            } else {
                e.weekly_value = Some(e.cost * 100.0 / e.consumed_percent);
            }
            "手动采样估值 · 按本次模型组合估算"
        }
        .into();
        Ok(())
    }
    pub fn begin_estimate(
        &self,
        q: &QuotaSnapshot,
        window: &str,
        ids: Vec<String>,
        previous: Option<&str>,
    ) -> Result<Estimate> {
        if window == "credits" {
            return self.begin_credit(q, ids, previous);
        }
        let w = q
            .windows
            .iter()
            .find(|w| window_id(w) == window || w.name == window)
            .context("额度窗口不存在")?;
        ensure!(eligible(q, w), "此额度池缺少可靠的模型映射，暂不支持估值");
        ensure!(
            q.error.is_none() && q.origin != "log" && (now() - q.updated_at).abs() <= 120,
            "需要新的实时限额快照"
        );
        let reset = w
            .resets_at
            .filter(|t| *t > q.updated_at)
            .context("重置时间不可用，请刷新后重试")?;
        ensure!((0.0..=100.0).contains(&w.used_percent), "额度数值异常");
        let key = format!("{}:{}", q.provider, q.account_id);
        let settings = self.settings()?;
        let config = source_config(&settings, &ids, &key)?;
        let tx = self.db.unchecked_transaction()?;
        if let Some(id) = previous {
            let mut old = self.estimate(id)?;
            ensure!(
                old.account_key == key && old.status == "pending" && !old.is_credit(),
                "只有待确认采样可以开启新一段"
            );
            old.status = "completed".into();
            old.ended_at = Some(old.checkpoint_at);
            self.save_estimate(&old)?;
        }
        ensure!(
            !self
                .estimates()?
                .iter()
                .any(|e| e.account_key == key && e.status != "completed" && !e.is_credit()),
            "该账户已有采样，请先结束"
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let five_hour = w.window_minutes == Some(300);
        let weekly = crate::capacity::overall(q, 10080)
            .filter(|w| crate::capacity::valid(w) && w.resets_at.is_some_and(|t| t > q.updated_at));
        let e = Estimate {
            quota_plan: q.plan.clone(),
            valuation_mode: if five_hour {
                "fiveHour".into()
            } else {
                String::new()
            },
            segment_started_at: q.updated_at,
            weekly_window_id: if five_hour {
                weekly.map(window_id).unwrap_or("").into()
            } else {
                String::new()
            },
            weekly_reset_at: weekly.and_then(|w| w.resets_at).unwrap_or(0),
            weekly_baseline_percent: weekly.map(|w| w.used_percent).unwrap_or(0.0),
            weekly_checkpoint_percent: weekly.map(|w| w.used_percent).unwrap_or(0.0),
            id: format!("sample-{stamp}"),
            account_key: key,
            window_id: window_id(w).into(),
            window_name: w.name.clone(),
            source_names: ids
                .iter()
                .filter_map(|id| {
                    settings
                        .sources
                        .iter()
                        .find(|s| &s.id == id)
                        .map(|s| s.name.clone())
                })
                .collect(),
            source_ids: ids,
            source_config: config,
            status: "active".into(),
            started_at: q.updated_at,
            checkpoint_at: q.updated_at,
            reset_at: reset,
            baseline_percent: w.used_percent,
            checkpoint_percent: w.used_percent,
            calculation_note: "样本不足：至少消耗 5 个百分点".into(),
            ..Default::default()
        };
        self.save_estimate(&e)?;
        tx.commit()?;
        Ok(e)
    }
    pub fn finish_estimate(&self, id: &str) -> Result<Estimate> {
        let mut e = self.estimate(id)?;
        if e.status == "completed" {
            return Ok(e);
        }
        if e.status == "active" {
            self.calculate_estimate(&mut e)?;
        }
        e.status = "completed".into();
        e.ended_at = Some(e.checkpoint_at);
        self.save_estimate(&e)?;
        Ok(e)
    }
}

impl Store {
    fn current_segment(&self, e: &Estimate) -> Result<EstimateSegment> {
        let mut part = e.clone();
        part.valuation_mode.clear();
        part.started_at = e.segment_started_at;
        self.calculate_estimate(&mut part)?;
        Ok(EstimateSegment {
            started_at: part.started_at,
            ended_at: part.checkpoint_at,
            five_percent: part.consumed_percent,
            weekly_percent: (!e.weekly_window_id.is_empty())
                .then_some(e.weekly_checkpoint_percent - e.weekly_baseline_percent),
            cost: part.cost,
            total_tokens: part.total_tokens,
            priced_tokens: part.priced_tokens,
            excluded_api_tokens: part.excluded_api_tokens,
            prices: part.prices,
            valid: !part.calculation_note.contains("边界")
                && part.priced_tokens == part.total_tokens
                && part.cost.is_finite()
                && part.cost >= 0.0
                && (part.total_tokens > 0 || part.consumed_percent <= 0.0),
            note: part.calculation_note,
        })
    }
    fn calculate_five_hour(&self, e: &mut Estimate) -> Result<()> {
        let current = self.current_segment(e)?;
        let mut parts = e.segments.clone();
        parts.push(current);
        e.cost = 0.0;
        e.total_tokens = 0;
        e.priced_tokens = 0;
        e.excluded_api_tokens = 0;
        e.consumed_percent = 0.0;
        e.prices.clear();
        e.five_hour_value = None;
        e.weekly_direct_value = None;
        e.weekly_ratio_value = None;
        e.weekly_value = None;
        let (mut week_cost, mut week_percent) = (0.0, 0.0);
        let mut valid = true;
        for part in &parts {
            e.cost += part.cost;
            e.total_tokens += part.total_tokens;
            e.priced_tokens += part.priced_tokens;
            e.excluded_api_tokens += part.excluded_api_tokens;
            e.consumed_percent += part.five_percent;
            if let Some(percent) = part.weekly_percent {
                week_percent += percent;
                week_cost += part.cost;
            }
            for price in &part.prices {
                if !e.prices.contains(price) {
                    e.prices.push(price.clone());
                }
            }
            valid &= part.valid;
        }
        e.capacity = self.capacity(&e.account_key, now())?;
        e.calculation_note = if !valid {
            parts
                .iter()
                .find(|p| !p.valid)
                .map(|p| p.note.clone())
                .unwrap_or_default()
        } else if e.total_tokens == 0 {
            "暂无可计入的订阅用量".into()
        } else if e.consumed_percent + 0.000001 < 5.0 {
            "样本不足：5h 至少消耗 5 个百分点".into()
        } else {
            e.five_hour_value = Some(e.cost * 100.0 / e.consumed_percent);
            if week_percent > 0.000001 {
                e.weekly_direct_value = Some(week_cost * 100.0 / week_percent);
                e.weekly_value = e.weekly_direct_value;
            }
            e.weekly_ratio_value = e
                .five_hour_value
                .zip(e.capacity.ratio)
                .map(|(value, ratio)| value * ratio);
            if week_percent > 0.0 && week_percent < 5.0 {
                "5h 采样估值 · 7d 同期样本较少".into()
            } else {
                "5h 采样估值 · 正常重置后自动接续".into()
            }
        };
        Ok(())
    }
    fn observe_five_hour(&self, e: &mut Estimate, q: &QuotaSnapshot) -> Result<()> {
        if q.error.is_none() && q.updated_at <= e.checkpoint_at {
            return Ok(());
        }
        let primary = crate::capacity::overall(q, 300).filter(|w| window_id(w) == e.window_id);
        let weekly =
            crate::capacity::overall(q, 10080).filter(|w| window_id(w) == e.weekly_window_id);
        let config = source_config(&self.settings()?, &e.source_ids, &e.account_key);
        let mut failure = if q.error.is_some() {
            Some("限额查询失败，已保留有效片段")
        } else if e.quota_plan != q.plan {
            Some("订阅方案发生变化，请确认后开始新一段")
        } else if config.as_ref().ok() != Some(&e.source_config) {
            Some("账户或用量来源已变化")
        } else if q.updated_at - e.checkpoint_at > 18000 {
            Some("超过 5h 未取得有效快照，无法确认额度周期")
        } else {
            None
        };
        if failure.is_none() {
            if let Some(w) = primary.filter(|w| {
                crate::capacity::valid(w) && w.resets_at.is_some_and(|t| t > q.updated_at)
            }) {
                let reset = w.resets_at != Some(e.reset_at) || q.updated_at >= e.reset_at;
                let weekly_reset = !e.weekly_window_id.is_empty()
                    && weekly.is_some_and(|w| {
                        w.resets_at != Some(e.weekly_reset_at) || q.updated_at >= e.weekly_reset_at
                    });
                if !e.weekly_window_id.is_empty()
                    && weekly.is_none_or(|w| {
                        !crate::capacity::valid(w) || w.resets_at.is_none_or(|t| t <= q.updated_at)
                    })
                {
                    failure = Some("7d 额度池已变化或不可用");
                } else if (reset && q.updated_at < e.reset_at)
                    || (weekly_reset && q.updated_at < e.weekly_reset_at)
                {
                    failure = Some("额度提前重置，请确认后开始新一段");
                } else if reset || weekly_reset {
                    let segment = self.current_segment(e)?;
                    if segment.ended_at > segment.started_at {
                        e.segments.push(segment);
                    }
                    e.segment_started_at = q.updated_at;
                    e.baseline_percent = w.used_percent;
                    e.checkpoint_percent = w.used_percent;
                    e.reset_at = w.resets_at.unwrap();
                    e.checkpoint_at = q.updated_at;
                    if let Some(week) = crate::capacity::overall(q, 10080) {
                        e.weekly_window_id = window_id(week).into();
                        e.weekly_reset_at = week.resets_at.unwrap_or(0);
                        e.weekly_baseline_percent = week.used_percent;
                        e.weekly_checkpoint_percent = week.used_percent;
                    }
                } else if w.used_percent + 0.000001 < e.checkpoint_percent
                    || weekly
                        .is_some_and(|w| w.used_percent + 0.000001 < e.weekly_checkpoint_percent)
                {
                    failure = Some("额度异常回升，请确认后开始新一段");
                } else {
                    e.checkpoint_at = q.updated_at;
                    e.checkpoint_percent = w.used_percent;
                    if let Some(week) = weekly {
                        e.weekly_checkpoint_percent = week.used_percent;
                    }
                }
            } else {
                failure = Some("5h 额度池已变化或不可用");
            }
        }
        if let Some(reason) = failure {
            e.status = "pending".into();
            e.reason = reason.into();
        }
        self.calculate_five_hour(e)?;
        self.save_estimate(e)
    }
}

impl Engine {
    pub(crate) fn sync_estimate_sources(&mut self, ids: &[String]) -> Result<()> {
        for batch in ids.chunks(3) {
            let settings = self.store.settings()?;
            let names = batch
                .iter()
                .map(|id| {
                    settings
                        .sources
                        .iter()
                        .find(|s| &s.id == id)
                        .map(|s| s.name.as_str())
                        .unwrap_or(id)
                })
                .collect::<Vec<_>>()
                .join("、");
            self.progress(&format!("同步来源 · {names}"));
            let value = self.call("sources.scan", json!({"sourceIds":batch}))?;
            ensure!(
                value
                    .as_array()
                    .is_some_and(|rows| rows.len() == batch.len()
                        && rows.iter().all(|r| r.get("error").is_none())),
                "用量同步失败，请重试；尚未提交采样边界"
            );
        }
        Ok(())
    }
    pub fn estimate_call(&mut self, method: &str, p: Value) -> Result<Value> {
        let credits = method.starts_with("creditEstimates.");
        let action = method.split_once('.').map(|(_, a)| a).unwrap_or("");
        if action == "list" {
            return Ok(serde_json::to_value(
                self.store
                    .estimates()?
                    .into_iter()
                    .filter(|e| e.is_credit() == credits)
                    .collect::<Vec<_>>(),
            )?);
        }
        let id = p["id"].as_str().unwrap_or("");
        if !id.is_empty() {
            ensure!(
                self.store.estimate(id)?.is_credit() == credits,
                "采样类型不匹配"
            );
        }
        if action == "get" {
            return Ok(serde_json::to_value(self.store.estimate(id)?)?);
        }
        if action == "stop" {
            let e = self.store.estimate(id)?;
            if e.status == "completed" {
                return Ok(serde_json::to_value(e)?);
            }
            if e.status == "pending" {
                return Ok(serde_json::to_value(self.store.finish_estimate(id)?)?);
            }
            self.sync_estimate_sources(&e.source_ids)?;
            if e.status == "active" {
                let (_, account) = e.account_key.split_once(':').context("账户格式错误")?;
                self.progress("读取实时限额");
                let results =
                    self.read_quotas(&json!({"accountId":account,"accountKey":e.account_key}))?;
                let q = results
                    .iter()
                    .find(|q| format!("{}:{}", q.provider, q.account_id) == e.account_key)
                    .context("未读取到账户限额")?;
                if q.error.is_some() {
                    self.store.quota(q)?;
                    anyhow::bail!("限额查询失败，已保留有效采样，请重试");
                }
                self.progress("补齐尾部记录");
                self.sync_estimate_sources(&e.source_ids)?;
                self.store.quota(q)?;
            }
            self.progress("保存采样结果");
            return Ok(serde_json::to_value(self.store.finish_estimate(id)?)?);
        }
        ensure!(["start", "restart"].contains(&action), "未知估值操作");
        ensure!(
            p["confirmed"] == true,
            "请确认采样期间仅使用目标订阅，且已纳入所有用量来源"
        );
        let old = if method.ends_with("restart") {
            Some(self.store.estimate(id)?)
        } else {
            None
        };
        let key = p["accountKey"]
            .as_str()
            .or(old.as_ref().map(|e| e.account_key.as_str()))
            .context("请选择账户")?;
        let window = if credits {
            "credits"
        } else {
            p["windowId"]
                .as_str()
                .or(old.as_ref().map(|e| e.window_id.as_str()))
                .context("请选择额度窗口")?
        };
        let mut ids: Vec<String> = if let Some(ids) = p.get("sourceIds") {
            serde_json::from_value(ids.clone())?
        } else {
            old.as_ref()
                .map(|e| e.source_ids.clone())
                .unwrap_or_default()
        };
        ids.sort();
        ids.dedup();
        source_config(&self.store.settings()?, &ids, key)?;
        self.sync_estimate_sources(&ids)?;
        let (_, account) = key.split_once(':').context("账户格式错误")?;
        self.progress("读取实时限额");
        let results = self.read_quotas(&json!({"accountId":account,"accountKey":key}))?;
        let q = results
            .iter()
            .find(|q| format!("{}:{}", q.provider, q.account_id) == key)
            .context("未读取到账户限额")?;
        self.store.quota(q)?;
        self.progress("保存采样起点");
        Ok(serde_json::to_value(self.store.begin_estimate(
            q,
            window,
            ids,
            old.as_ref().map(|e| e.id.as_str()),
        )?)?)
    }
}

impl Store {
    fn observe_credit(&self, e: &mut Estimate, q: &QuotaSnapshot) -> Result<()> {
        if q.error.is_none() && q.updated_at <= e.checkpoint_at {
            return Ok(());
        }
        let balance = credit_balance(q);
        let config = source_config(&self.settings()?, &e.source_ids, &e.account_key);
        let restored = q.windows.len() != e.checkpoint_windows.len()
            || e.checkpoint_windows.iter().any(|old| {
                q.windows
                    .iter()
                    .find(|w| window_id(w) == window_id(old))
                    .is_none_or(|w| {
                        w.resets_at != old.resets_at
                            || !w.used_percent.is_finite()
                            || w.used_percent + 0.001 < old.used_percent
                    })
            });
        let reason = if q.error.is_some() {
            Some("credits 查询失败，请确认后开始新一段")
        } else if config.as_ref().ok() != Some(&e.source_config) {
            Some("数据源配置发生变化")
        } else if balance.is_none() {
            Some("credits 余额未知或变为无限额度")
        } else if balance.and_then(credit_units)
            > e.checkpoint_balance.as_deref().and_then(credit_units)
        {
            Some("credits 余额回升，可能已充值；请开始新一段")
        } else if restored {
            Some("订阅额度恢复或额度池变化；请确认仅消耗 credits 后开始新一段")
        } else {
            None
        };
        if let Some(reason) = reason {
            e.status = "pending".into();
            e.reason = reason.into();
        } else {
            e.checkpoint_at = q.updated_at;
            e.checkpoint_balance = balance.map(str::to_owned);
            e.checkpoint_windows = q.windows.clone();
            self.calculate_estimate(e)?;
        }
        self.save_estimate(e)
    }
    fn begin_credit(
        &self,
        q: &QuotaSnapshot,
        ids: Vec<String>,
        previous: Option<&str>,
    ) -> Result<Estimate> {
        ensure!(q.provider == "codex", "仅 Codex 支持 credits 采样");
        ensure!(
            q.error.is_none() && q.origin != "log" && (now() - q.updated_at).abs() <= 120,
            "需要新的实时 credits 快照"
        );
        let balance = credit_balance(q).context("需要明确、有限的 credits 余额")?;
        let key = format!("{}:{}", q.provider, q.account_id);
        let settings = self.settings()?;
        let config = source_config(&settings, &ids, &key)?;
        let tx = self.db.unchecked_transaction()?;
        if let Some(id) = previous {
            let mut old = self.estimate(id)?;
            ensure!(
                old.is_credit() && old.account_key == key && old.status == "pending",
                "只有待确认的 credits 采样可以重开"
            );
            old.status = "completed".into();
            old.ended_at = Some(old.checkpoint_at);
            self.save_estimate(&old)?;
        }
        ensure!(
            !self
                .estimates()?
                .iter()
                .any(|e| e.is_credit() && e.account_key == key && e.status != "completed"),
            "该账户已有 credits 采样，请先结束"
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let e = Estimate {
            id: format!("credit-sample-{stamp}"),
            kind: "credits".into(),
            account_key: key,
            window_id: "credits".into(),
            window_name: "Credits".into(),
            source_config: config,
            source_names: ids
                .iter()
                .filter_map(|id| {
                    settings
                        .sources
                        .iter()
                        .find(|s| &s.id == id)
                        .map(|s| s.name.clone())
                })
                .collect(),
            source_ids: ids,
            status: "active".into(),
            started_at: q.updated_at,
            checkpoint_at: q.updated_at,
            baseline_balance: Some(balance.into()),
            checkpoint_balance: Some(balance.into()),
            checkpoint_windows: q.windows.clone(),
            calculation_note: "样本不足：至少消耗 5 credits".into(),
            ..Default::default()
        };
        self.save_estimate(&e)?;
        tx.commit()?;
        Ok(e)
    }
}
