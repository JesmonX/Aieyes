//! Manual quota valuation. Account association is not billing evidence.
use crate::contracts::{FIVE_HOUR_MINUTES, WEEK_MINUTES};
use crate::{Engine, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum Estimate {
    #[serde(rename = "quota")]
    Quota(QuotaEstimate),
    #[serde(rename = "credits")]
    Credit(CreditEstimate),
}
impl<'de> Deserialize<'de> for Estimate {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(decoder)?;
        // Legacy quota payloads used an empty or missing kind.
        match value["kind"].as_str().unwrap_or("") {
            "" | "quota" => serde_json::from_value(value).map(Self::Quota),
            "credits" => serde_json::from_value(value).map(Self::Credit),
            _ => return Err(serde::de::Error::custom("未知采样类型")),
        }
        .map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaEstimate {
    #[serde(flatten)]
    pub common: EstimateCommon,
    pub group_id: Option<String>,
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
    pub reset_at: i64,
    pub baseline_percent: f64,
    pub checkpoint_percent: f64,
    pub consumed_percent: f64,
    pub weekly_value: Option<f64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreditEstimate {
    #[serde(flatten)]
    pub common: EstimateCommon,
    pub baseline_balance: Option<String>,
    pub checkpoint_balance: Option<String>,
    pub consumed_credits: f64,
    pub value_per500: Option<f64>,
    pub value_per1000: Option<f64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EstimateCommon {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sampling_mode: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub automatic_segments: Vec<crate::automatic::AutoSegment>,
    pub calculation_status: String,
    pub calculation_version: u32,
    pub original_estimate_id: Option<String>,
    pub repaired_at: Option<i64>,
    #[serde(skip)]
    pub(crate) repair_prices: Option<Vec<Value>>,
    pub checkpoint_windows: Vec<QuotaWindow>,
    pub id: String,
    pub account_key: String,
    pub window_id: String,
    pub window_name: String,
    pub source_ids: Vec<String>,
    // Explicit manual sampling scope; shared history itself keeps its original attribution.
    pub shared_source_ids: Vec<String>,
    pub source_names: Vec<String>,
    pub source_config: String,
    pub status: String,
    pub reason: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub checkpoint_at: i64,
    pub cost: f64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub excluded_api_tokens: u64,
    pub calculation_note: String,
    pub prices: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EstimateSegment {
    pub calculation_status: String,
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
    pub(crate) fn clear_values(&mut self) {
        match self {
            Self::Quota(e) => {
                e.five_hour_value = None;
                e.weekly_value = None;
                e.weekly_direct_value = None;
                e.weekly_ratio_value = None;
            }
            Self::Credit(e) => {
                e.value_per500 = None;
                e.value_per1000 = None;
            }
        }
    }
    pub fn quota(&self) -> Option<&QuotaEstimate> {
        if let Self::Quota(e) = self {
            Some(e)
        } else {
            None
        }
    }
    pub fn credit(&self) -> Option<&CreditEstimate> {
        if let Self::Credit(e) = self {
            Some(e)
        } else {
            None
        }
    }
    pub(crate) fn storage_kind(&self) -> &'static str {
        match self {
            Self::Quota(_) => "quota",
            Self::Credit(_) => "credits",
        }
    }
    pub fn has_value(&self) -> bool {
        match self {
            Self::Quota(e) => [e.five_hour_value, e.weekly_value]
                .into_iter()
                .flatten()
                .any(f64::is_finite),
            Self::Credit(e) => e.value_per1000.is_some_and(f64::is_finite),
        }
    }
}
impl std::ops::Deref for Estimate {
    type Target = EstimateCommon;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Quota(e) => &e.common,
            Self::Credit(e) => &e.common,
        }
    }
}
impl std::ops::DerefMut for Estimate {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Quota(e) => &mut e.common,
            Self::Credit(e) => &mut e.common,
        }
    }
}
macro_rules! common_fields {
    ($type:ty) => {
        impl std::ops::Deref for $type {
            type Target = EstimateCommon;
            fn deref(&self) -> &Self::Target {
                &self.common
            }
        }
        impl std::ops::DerefMut for $type {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.common
            }
        }
    };
}
common_fields!(QuotaEstimate);
common_fields!(CreditEstimate);

fn decode_estimate(kind: &str, raw: &str) -> Result<Estimate> {
    let mut value: Value = serde_json::from_str(raw)?;
    ensure!(value.is_object(), "采样内容格式错误");
    let declared = value["kind"].as_str().unwrap_or("");
    ensure!(
        declared.is_empty() || declared == kind,
        "采样存储类型与内容不一致"
    );
    value["kind"] = kind.into();
    Ok(serde_json::from_value(value)?)
}

pub(crate) fn finite_value(cost: f64, consumed: f64, scale: f64) -> Option<f64> {
    if !cost.is_finite() || cost < 0.0 || !consumed.is_finite() || consumed <= 0.0 {
        return None;
    }
    let value = cost / consumed * scale;
    value.is_finite().then_some(value)
}

fn value_ready(e: &mut EstimateCommon, consumed: f64, note: &str) -> bool {
    if e.calculation_status != "ready" {
        return false;
    }
    if consumed <= 0.0 || !consumed.is_finite() {
        e.calculation_status = "insufficientUsage".into();
        e.calculation_note = "等待可确认的正向消耗；已保留有效估值".into();
    } else if finite_value(e.cost, consumed, 1000.0).is_none() {
        e.calculation_status = "invalidPrice".into();
        e.calculation_note = "价格数据无效".into();
    } else {
        if consumed < 5.0 {
            e.calculation_note = note.into();
        }
        return true;
    }
    false
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
    if crate::providers::antigravity(&q.provider) {
        let group = crate::antigravity::window_group(w);
        return crate::antigravity::group_kind(group).is_some()
            && w.window_minutes.is_some_and(|m| {
                [FIVE_HOUR_MINUTES, WEEK_MINUTES].contains(&m)
                    && crate::capacity::window(q, m, group)
                        .is_some_and(|v| window_id(v) == window_id(w))
            });
    }
    (w.window_minutes == Some(FIVE_HOUR_MINUTES)
        && crate::capacity::overall(q, FIVE_HOUR_MINUTES)
            .is_some_and(|v| window_id(v) == window_id(w)))
        || (w.window_minutes == Some(WEEK_MINUTES)
            && (q
                .windows
                .iter()
                .filter(|v| v.window_minutes == Some(WEEK_MINUTES))
                .count()
                == 1
                || (q.provider == "claude" && (w.id == "seven_day" || w.name == "7d"))))
}

pub(crate) fn source_config(
    settings: &Settings,
    ids: &[String],
    account_key: &str,
) -> Result<String> {
    let mut values = Vec::new();
    ensure!(!ids.is_empty(), "请选择用量数据源");
    for id in ids {
        let s = settings
            .sources
            .iter()
            .find(|s| &s.id == id)
            .context("采样数据源已删除")?;
        let account = settings
            .accounts
            .iter()
            .find(|a| format!("{}:{}", a.provider, a.id) == account_key);
        ensure!(
            settings.source_enabled(s) && account.is_some_and(|a| a.uses_source(s)),
            "采样数据源已停用或账户关联发生变化"
        );
        ensure!(s.provider != "deepseek", "该数据源没有可采集的 Token 用量");
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
        let mut value = json!({"source":s,"host":host});
        if let Some(home) = &s.codex_home_id {
            value["profiles"] = json!(
                account
                    .unwrap()
                    .profile_refs()
                    .into_iter()
                    .filter(|r| r.starts_with(&format!("{home}:")))
                    .collect::<Vec<_>>()
            );
        }
        values.push(value);
    }
    Ok(hash(&serde_json::to_string(&values)?))
}

fn shared_sources(settings: &Settings, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| {
            settings
                .sources
                .iter()
                .any(|s| &s.id == *id && s.provider == "codex" && s.codex_home_id.is_some())
        })
        .cloned()
        .collect()
}

impl Store {
    pub(crate) fn recalculate_history_values(&self, e: &mut Estimate) -> Result<()> {
        // A prior invalidation may have found no usable rows. Preserve the
        // original price basis so later parent arrival can repair the same span.
        let frozen = e.repair_prices.clone().unwrap_or_else(|| e.prices.clone());
        e.repair_prices = Some(frozen.clone());
        if e.sampling_mode == "automatic" {
            let group = e.quota().and_then(|v| v.group_id.clone());
            for index in 0..e.automatic_segments.len() {
                let segment = &e.automatic_segments[index];
                let mut sample = EstimateCommon {
                    sampling_mode: "automatic".into(),
                    account_key: e.account_key.clone(),
                    source_ids: e.source_ids.clone(),
                    shared_source_ids: e.shared_source_ids.clone(),
                    started_at: segment.started_at,
                    checkpoint_at: segment.ended_at,
                    repair_prices: Some(if segment.prices.is_empty() {
                        frozen.clone()
                    } else {
                        segment.prices.clone()
                    }),
                    ..Default::default()
                };
                self.calculate_usage(&mut sample, group.as_deref())?;
                let segment = &mut e.automatic_segments[index];
                if segment.pool.is_empty() && segment.weekly_consumed > 0.0 {
                    if segment.weekly_cost == segment.cost {
                        segment.weekly_cost = sample.cost;
                    } else {
                        segment.weekly_consumed = 0.0;
                        segment.weekly_cost = 0.0;
                    }
                }
                segment.cost = sample.cost;
                segment.tokens = sample.total_tokens;
                segment.priced_tokens = sample.priced_tokens;
                segment.valid = sample.calculation_status == "ready"
                    && finite_value(sample.cost, segment.consumed, 1000.0).is_some();
                segment.note = sample.calculation_note;
                if !sample.prices.is_empty() {
                    segment.prices = sample.prices;
                }
            }
            crate::automatic::recalculate(e);
            return Ok(());
        }
        if let Estimate::Quota(quota) = e
            && quota.valuation_mode == "fiveHour"
        {
            for index in 0..quota.segments.len() {
                let old = &quota.segments[index];
                let mut part = quota.clone();
                part.repair_prices = Some(old.prices.clone());
                part.segment_started_at = old.started_at;
                part.checkpoint_at = old.ended_at;
                part.baseline_percent = 0.0;
                part.checkpoint_percent = old.five_percent;
                part.weekly_baseline_percent = 0.0;
                part.weekly_checkpoint_percent = old.weekly_percent.unwrap_or(0.0);
                quota.segments[index] = self.current_segment(&part)?;
            }
        }
        self.calculate_estimate(e)?;
        e.prices = frozen;
        Ok(())
    }

    fn ensure_shared_sample_available(&self, ids: &[String], account_key: &str) -> Result<()> {
        ensure!(
            !self.active_estimates()?.iter().any(|e| {
                e.account_key != account_key
                    && e.shared_source_ids.iter().any(|id| ids.contains(id))
            }),
            "共享目录正在为其他账户采样，请先结束该采样"
        );
        Ok(())
    }

    pub(crate) fn pause_shared_source_estimates(&self, source: &str) -> Result<()> {
        for mut e in self
            .active_estimates()?
            .into_iter()
            .filter(|e| e.status == "active" && e.shared_source_ids.iter().any(|id| id == source))
        {
            e.status = "pending".into();
            e.reason = "共享目录的登录账户已变化；请确认后开始新一段".into();
            self.save_estimate(&e)?;
        }
        Ok(())
    }

    pub(crate) fn migrate_estimate_tables(&self) -> Result<()> {
        let tx = self.write_transaction()?;
        for (table, kind) in [
            ("quota_estimates", "quota"),
            ("credit_estimates", "credits"),
        ] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )?;
            if !exists {
                continue;
            }
            // No OR IGNORE: any conflict aborts the entire migration instead of losing history.
            tx.execute(
                &format!(
                    "INSERT INTO estimates SELECT ?1,id,account_key,status,payload FROM {table}"
                ),
                [kind],
            )?;
            tx.execute_batch(&format!("DROP TABLE {table}"))?;
        }
        tx.commit()?;
        Ok(())
    }
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
    pub(crate) fn active_estimates(&self) -> Result<Vec<Estimate>> {
        self.read_estimates(true)
    }
    fn read_estimates(&self, active_only: bool) -> Result<Vec<Estimate>> {
        let sql = if active_only {
            "SELECT kind,payload FROM estimates WHERE status='active' ORDER BY json_extract(payload,'$.startedAt') DESC"
        } else {
            "SELECT kind,payload FROM estimates ORDER BY json_extract(payload,'$.startedAt') DESC, COALESCE(json_extract(payload,'$.repairedAt'),0) DESC"
        };
        let mut stmt = self.db.prepare(sql)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter()
            .map(|(kind, raw)| decode_estimate(kind, raw))
            .collect()
    }
    pub fn estimate_for(&self, id: &str, credits: bool) -> Result<Estimate> {
        let kind = if credits { "credits" } else { "quota" };
        let raw: String = self
            .db
            .query_row(
                "SELECT payload FROM estimates WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .context("采样记录不存在或类型不匹配")?;
        decode_estimate(kind, &raw)
    }
    pub(crate) fn save_estimate(&self, e: &Estimate) -> Result<()> {
        let kind = e.storage_kind();
        self.db.execute("INSERT INTO estimates VALUES(?1,?2,?3,?4,?5) ON CONFLICT(kind,id) DO UPDATE SET status=excluded.status,payload=excluded.payload",params![kind,e.id,e.account_key,e.status,serde_json::to_string(e)?])?;
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
                if e.sampling_mode == "automatic" {
                    e.status = "completed".into();
                    e.ended_at = Some(e.checkpoint_at);
                } else {
                    e.status = "pending".into();
                }
                e.reason = "账户或数据源配置发生变化；已保留有效区间".into();
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
        for mut e in self.active_estimates()?.into_iter().filter(|e| {
            e.account_key == key && e.status == "active" && e.sampling_mode != "automatic"
        }) {
            let e = match &mut e {
                Estimate::Credit(credit) => {
                    self.observe_credit(credit, q)?;
                    continue;
                }
                Estimate::Quota(quota) => quota,
            };
            if e.valuation_mode == "fiveHour" {
                self.observe_five_hour(e, q)?;
                continue;
            }
            let w = q.windows.iter().find(|w| window_id(w) == e.window_id);
            let config = source_config(&self.settings()?, &e.source_ids, &e.account_key);
            let failure = if q.error.is_some() {
                Some("额度查询失败，采样已暂停；请确认后重新开始")
            } else if config.as_ref().ok() != Some(&e.source_config) {
                Some("数据源配置发生变化，采样已暂停")
            } else if let Some(w) = w {
                if q.updated_at <= e.checkpoint_at {
                    continue;
                }
                if !w.used_percent.is_finite() || !(0.0..=100.0).contains(&w.used_percent) {
                    Some("额度数值异常")
                } else if !crate::automatic::same_cycle(Some(e.reset_at), w.resets_at, q.updated_at)
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
            self.calculate_quota(e)?;
            self.save_estimate(&Estimate::Quota(e.clone()))?;
        }
        Ok(())
    }
    pub fn calculate_estimate(&self, e: &mut Estimate) -> Result<()> {
        if e.sampling_mode == "automatic" {
            return Ok(());
        }
        match e {
            Estimate::Quota(e) => self.calculate_quota(e),
            Estimate::Credit(e) => self.calculate_credit(e),
        }
    }
    fn calculate_quota(&self, e: &mut QuotaEstimate) -> Result<()> {
        e.calculation_version = 3;
        if e.valuation_mode == "fiveHour" {
            return self.calculate_five_hour(e);
        }
        e.consumed_percent = e.checkpoint_percent - e.baseline_percent;
        self.calculate_usage(&mut e.common, e.group_id.as_deref())?;
        if value_ready(
            &mut e.common,
            e.consumed_percent,
            "小样本：额度消耗较少，估值会随样本积累更新",
        ) {
            e.weekly_value = finite_value(e.cost, e.consumed_percent, 100.0);
        }
        Ok(())
    }
    fn calculate_credit(&self, e: &mut CreditEstimate) -> Result<()> {
        e.calculation_version = 3;
        e.consumed_credits = e
            .baseline_balance
            .as_deref()
            .and_then(credit_units)
            .zip(e.checkpoint_balance.as_deref().and_then(credit_units))
            .and_then(|(a, b)| a.checked_sub(b))
            .map(|n| n as f64 / 1e18)
            .unwrap_or(0.0);
        self.calculate_usage(&mut e.common, None)?;
        if value_ready(
            &mut e.common,
            e.consumed_credits,
            "小样本：credits 消耗较少，估值会随样本积累更新",
        ) {
            e.value_per500 = finite_value(e.cost, e.consumed_credits, 500.0);
            e.value_per1000 = finite_value(e.cost, e.consumed_credits, 1000.0);
        }
        Ok(())
    }
    pub(crate) fn calculate_usage(
        &self,
        e: &mut EstimateCommon,
        group: Option<&str>,
    ) -> Result<()> {
        let key = e.account_key.clone();
        let (provider, account) = key.split_once(':').context("账户格式错误")?;
        e.cost = 0.0;
        e.total_tokens = 0;
        e.priced_tokens = 0;
        e.excluded_api_tokens = 0;
        e.prices.clear();
        let settings = self.settings()?;
        let strict = provider == "codex";
        let mut unknown_billing = strict
            && (!self.api_interval_complete(&e.source_ids, e.started_at, e.checkpoint_at)?
                || !self.native_interval_complete(&e.source_ids, e.started_at, e.checkpoint_at)?);
        let mut boundary = false;
        let mut unmapped_group = false;
        // EXISTS avoids multiplying events observed through several selected sources.
        let mut stmt = self.db.prepare("SELECT e.payload,e.cost,e.priced_tokens,e.price FROM events e WHERE e.provider=?1 AND e.stamp>?3 AND e.stamp<=?4 AND EXISTS(SELECT 1 FROM event_sources s WHERE s.event_id=e.id AND s.source_id IN (SELECT value FROM json_each(?5)) AND (e.account_id=?2 OR s.source_id IN (SELECT value FROM json_each(?6))))")?;
        let rows = stmt.query_map(
            params![
                provider,
                account,
                e.started_at,
                e.checkpoint_at,
                serde_json::to_string(&e.source_ids)?,
                serde_json::to_string(&e.shared_source_ids)?
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
            let (raw, mut cost, mut priced, mut price) = row?;
            let event: UsageEvent = serde_json::from_str(&raw)?;
            if event.attribution == "unresolved-inheritance" {
                unknown_billing = true;
                continue;
            }
            if event.tokens.total() == 0 {
                continue;
            }
            let billing = self.effective_billing(&event)?;
            if billing == "api" {
                e.excluded_api_tokens += event.tokens.total();
                continue;
            }
            if strict && billing != "subscription" {
                unknown_billing = true;
                continue;
            }
            if let Some(group) = group {
                let model = settings
                    .model_mappings
                    .get(&event.model)
                    .map(String::as_str)
                    .unwrap_or(&event.model);
                match crate::antigravity::group_for_model(model) {
                    Some(kind) if Some(kind) == crate::antigravity::group_kind(group) => {}
                    Some(_) => continue,
                    None => {
                        unmapped_group = true;
                        continue;
                    }
                }
            }
            if (event.provider == "codex"
                || event.attribution == "session-summary"
                || event.interval_evidence == "antigravity-step-v1")
                && event.interval_start.is_none_or(|t| t < e.started_at)
            {
                boundary = true;
                continue;
            }
            e.total_tokens += event.tokens.total();
            if let Some(frozen) = &e.repair_prices {
                let observed: Option<Value> =
                    price.as_deref().map(serde_json::from_str).transpose()?;
                let matched = observed.as_ref().and_then(|p| {
                    frozen.iter().find(|f| *f == p).or_else(|| {
                        let mut candidates = frozen.iter().filter(|f| f["id"] == p["id"]);
                        let first = candidates.next();
                        if candidates.next().is_none() {
                            first
                        } else {
                            None
                        }
                    })
                });
                let model_price: Option<ModelPrice> =
                    matched.cloned().map(serde_json::from_value).transpose()?;
                (cost, priced) = crate::pricing::estimate(&event.tokens, model_price.as_ref());
                price = matched.map(serde_json::to_string).transpose()?;
            }
            e.priced_tokens += priced;
            e.cost += cost;
            if let Some(raw) = price {
                let p: Value = serde_json::from_str(&raw)?;
                if !e.prices.contains(&p) {
                    e.prices.push(p);
                }
            }
        }
        e.calculation_note = if unknown_billing {
            e.calculation_status = "unknownBilling".into();
            "存在未归属、缺失父历史或不完整请求，本区间不参与估值；已保留有效值"
        } else if unmapped_group {
            e.calculation_status = "unmappedGroup".into();
            "存在无法归属额度组的模型，请补全模型映射后重新计算"
        } else if boundary {
            e.calculation_status = "boundary".into();
            "存在跨采样边界的累计用量，请在开始采样后新建会话再试"
        } else if e.total_tokens == 0 {
            e.calculation_status = "noUsage".into();
            "暂无可计入的订阅用量"
        } else if e.priced_tokens != e.total_tokens {
            e.calculation_status = "unpriced".into();
            "缺少模型价格，请补全价格后重新计算本次采样"
        } else {
            e.calculation_status = "ready".into();
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
            "请刷新订阅额度后重试"
        );
        let reset = w
            .resets_at
            .filter(|t| *t > q.updated_at)
            .context("重置时间不可用，请刷新后重试")?;
        ensure!((0.0..=100.0).contains(&w.used_percent), "额度数值异常");
        let key = format!("{}:{}", q.provider, q.account_id);
        let settings = self.settings()?;
        let config = source_config(&settings, &ids, &key)?;
        let tx = self.write_transaction()?;
        let shared_source_ids = shared_sources(&settings, &ids);
        self.ensure_shared_sample_available(&shared_source_ids, &key)?;
        if let Some(id) = previous {
            let mut old = self.estimate_for(id, false)?;
            ensure!(
                old.account_key == key
                    && old.status == "pending"
                    && matches!(old, Estimate::Quota(_)),
                "只有待确认采样可以开启新一段"
            );
            old.status = "completed".into();
            old.ended_at = Some(old.checkpoint_at);
            self.save_estimate(&old)?;
        }
        ensure!(
            !self.estimates()?.iter().any(|e| e.account_key == key
                && e.status != "completed"
                && matches!(e, Estimate::Quota(_))),
            "该账户已有采样，请先结束"
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let five_hour = w.window_minutes == Some(FIVE_HOUR_MINUTES);
        let group = if crate::providers::antigravity(&q.provider) {
            Some(crate::antigravity::window_group(w).to_owned())
        } else {
            None
        };
        let weekly = crate::capacity::window(q, WEEK_MINUTES, group.as_deref().unwrap_or(""))
            .filter(|w| crate::capacity::valid(w) && w.resets_at.is_some_and(|t| t > q.updated_at));
        let e = QuotaEstimate {
            common: EstimateCommon {
                calculation_status: "insufficientUsage".into(),
                calculation_version: 2,
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
                shared_source_ids,
                source_config: config,
                status: "active".into(),
                started_at: q.updated_at,
                checkpoint_at: q.updated_at,
                calculation_note: "样本不足：至少消耗 5 个百分点".into(),
                ..Default::default()
            },
            group_id: group,
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
            reset_at: reset,
            baseline_percent: w.used_percent,
            checkpoint_percent: w.used_percent,
            ..Default::default()
        };
        self.save_estimate(&Estimate::Quota(e.clone()))?;
        tx.commit()?;
        Ok(Estimate::Quota(e))
    }
    pub fn finish_estimate_for(&self, id: &str, credits: bool) -> Result<Estimate> {
        self.finish_record(self.estimate_for(id, credits)?)
    }
    fn finish_record(&self, mut e: Estimate) -> Result<Estimate> {
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

    pub fn repair_estimate_for(&self, id: &str, credits: bool) -> Result<Estimate> {
        self.repair_record(self.estimate_for(id, credits)?)
    }
    fn repair_record(&self, original: Estimate) -> Result<Estimate> {
        ensure!(
            original.status == "completed",
            "请先结束采样，再修复历史记录"
        );
        ensure!(original.original_estimate_id.is_none(), "此记录已是修正版");
        let repaired_id = format!("{}-repair-v2", original.id);
        if let Ok(existing) =
            self.estimate_for(&repaired_id, matches!(original, Estimate::Credit(_)))
        {
            return Ok(existing);
        }
        ensure!(!original.has_value(), "此记录已有有效估值，无需修复");
        let mut repaired = original.clone();
        repaired.repair_prices = Some(original.prices.clone());
        if let (Estimate::Quota(original), Estimate::Quota(repaired)) = (&original, &mut repaired) {
            for (index, segment) in original.segments.iter().enumerate() {
                if segment.valid {
                    continue;
                }
                let mut part = original.clone();
                part.repair_prices = Some(original.prices.clone());
                part.segment_started_at = segment.started_at;
                part.checkpoint_at = segment.ended_at;
                part.baseline_percent = 0.0;
                part.checkpoint_percent = segment.five_percent;
                part.weekly_baseline_percent = 0.0;
                part.weekly_checkpoint_percent = segment.weekly_percent.unwrap_or(0.0);
                repaired.segments[index] = self.current_segment(&part)?;
            }
        }
        self.calculate_estimate(&mut repaired)?;
        ensure!(
            repaired.has_value(),
            "无法修复：{}",
            repaired.calculation_note
        );
        repaired.id = repaired_id;
        repaired.original_estimate_id = Some(original.id.clone());
        repaired.repaired_at = Some(now());
        self.save_estimate(&repaired)?;
        Ok(repaired)
    }
}

impl Store {
    fn current_segment(&self, e: &QuotaEstimate) -> Result<EstimateSegment> {
        let mut part = e.clone();
        part.valuation_mode.clear();
        part.started_at = e.segment_started_at;
        self.calculate_quota(&mut part)?;
        let valid = ["ready", "insufficientUsage", "noUsage"]
            .contains(&part.calculation_status.as_str())
            && part.priced_tokens == part.total_tokens
            && part.cost.is_finite()
            && part.cost >= 0.0
            && (part.total_tokens > 0 || part.consumed_percent <= 0.0);
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
            prices: part.common.prices,
            valid,
            note: part.common.calculation_note,
            calculation_status: part.common.calculation_status,
        })
    }
    fn calculate_five_hour(&self, e: &mut QuotaEstimate) -> Result<()> {
        let current = self.current_segment(e)?;
        let mut parts = e.segments.clone();
        parts.push(current);
        e.cost = 0.0;
        e.total_tokens = 0;
        e.priced_tokens = 0;
        e.excluded_api_tokens = 0;
        e.consumed_percent = 0.0;
        e.prices.clear();
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
        if e.repair_prices.is_none() {
            e.capacity = self.capacity_for_group(
                &e.account_key,
                e.group_id.as_deref().unwrap_or(""),
                now(),
            )?;
        }
        e.calculation_note = if !valid {
            e.calculation_status = parts
                .iter()
                .find(|p| !p.valid)
                .map(|p| p.calculation_status.clone())
                .unwrap_or_default();
            parts
                .iter()
                .find(|p| !p.valid)
                .map(|p| p.note.clone())
                .unwrap_or_default()
        } else if e.total_tokens == 0 {
            e.calculation_status = "noUsage".into();
            "暂无可计入的订阅用量".into()
        } else if finite_value(e.cost, e.consumed_percent, 100.0).is_none() {
            e.calculation_status = "insufficientUsage".into();
            "等待可确认的正向消耗；已保留有效估值".into()
        } else {
            e.calculation_status = "ready".into();
            e.five_hour_value = finite_value(e.cost, e.consumed_percent, 100.0);
            if let Some(value) = finite_value(week_cost, week_percent, 100.0) {
                e.weekly_direct_value = Some(value);
                e.weekly_value = e.weekly_direct_value;
            }
            e.weekly_ratio_value = e
                .five_hour_value
                .zip(e.capacity.ratio)
                .map(|(value, ratio)| value * ratio)
                .filter(|v| v.is_finite());
            if e.consumed_percent < 5.0 || week_percent > 0.0 && week_percent < 5.0 {
                "小样本：估值会随 5h / 7d 用量积累更新".into()
            } else {
                "5h 采样估值 · 正常重置后自动接续".into()
            }
        };
        Ok(())
    }
    fn observe_five_hour(&self, e: &mut QuotaEstimate, q: &QuotaSnapshot) -> Result<()> {
        if q.error.is_none() && q.updated_at <= e.checkpoint_at {
            return Ok(());
        }
        let primary =
            crate::capacity::window(q, FIVE_HOUR_MINUTES, e.group_id.as_deref().unwrap_or(""))
                .filter(|w| window_id(w) == e.window_id);
        let weekly = crate::capacity::window(q, WEEK_MINUTES, e.group_id.as_deref().unwrap_or(""))
            .filter(|w| window_id(w) == e.weekly_window_id);
        let config = source_config(&self.settings()?, &e.source_ids, &e.account_key);
        let mut failure = if q.error.is_some() {
            Some("额度查询失败，已保留有效片段")
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
                let reset =
                    !crate::automatic::same_cycle(Some(e.reset_at), w.resets_at, q.updated_at);
                let weekly_reset = !e.weekly_window_id.is_empty()
                    && weekly.is_some_and(|w| {
                        !crate::automatic::same_cycle(
                            Some(e.weekly_reset_at),
                            w.resets_at,
                            q.updated_at,
                        )
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
                    if let Some(week) = crate::capacity::window(
                        q,
                        WEEK_MINUTES,
                        e.group_id.as_deref().unwrap_or(""),
                    ) {
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
        self.save_estimate(&Estimate::Quota(e.clone()))
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
                        && rows.iter().all(|r| r.get("error").is_none()
                            && r["result"]["failedFiles"].as_u64().unwrap_or(0) == 0
                            && r["result"]["invalidRecords"].as_u64().unwrap_or(0) == 0
                            && !r["issues"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|issue| issue["code"] == "invalidRecord"))),
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
                    .filter(|e| matches!(e, Estimate::Credit(_)) == credits)
                    .collect::<Vec<_>>(),
            )?);
        }
        let id = p["id"].as_str().unwrap_or("");
        if !id.is_empty() {
            self.store.estimate_for(id, credits)?;
        }
        if action == "get" {
            return Ok(serde_json::to_value(self.store.estimate_for(id, credits)?)?);
        }
        if action == "repair" {
            let original = self.store.estimate_for(id, credits)?;
            if let Ok(existing) = self.store.estimate_for(&format!("{id}-repair-v2"), credits) {
                return Ok(serde_json::to_value(existing)?);
            }
            ensure!(
                original.status == "completed"
                    && original.sampling_mode != "automatic"
                    && !original.has_value(),
                "仅支持修复已结束且无估值的记录"
            );
            self.sync_estimate_sources(&original.source_ids)?;
            self.progress("按原价格修复采样边界");
            return Ok(serde_json::to_value(
                self.store.repair_estimate_for(id, credits)?,
            )?);
        }
        if action == "stop" {
            let e = self.store.estimate_for(id, credits)?;
            if e.sampling_mode == "automatic" {
                for source in &e.source_ids {
                    self.configure_automatic(json!({"sourceId":source,"enabled":false}))?;
                }
                return Ok(serde_json::to_value(self.store.estimate_for(id, credits)?)?);
            }
            if e.status == "completed" {
                return Ok(serde_json::to_value(e)?);
            }
            if e.status == "pending" {
                return Ok(serde_json::to_value(
                    self.store.finish_estimate_for(id, credits)?,
                )?);
            }
            if self
                .verify_sampling_identities(&e.source_ids, &e.account_key)
                .is_err()
            {
                for source in &e.shared_source_ids {
                    self.store.pause_shared_source_estimates(source)?;
                }
                return Ok(serde_json::to_value(
                    self.store.finish_estimate_for(id, credits)?,
                )?);
            }
            self.sync_estimate_sources(&e.source_ids)?;
            if e.status == "active" {
                let (_, account) = e.account_key.split_once(':').context("账户格式错误")?;
                self.progress("读取实时额度");
                let results =
                    self.read_quotas(&json!({"accountId":account,"accountKey":e.account_key,"samplingSourceIds":e.source_ids}))?;
                let q = results
                    .iter()
                    .find(|q| format!("{}:{}", q.provider, q.account_id) == e.account_key)
                    .context("未读取到账户额度")?;
                if q.error.is_some() {
                    anyhow::bail!("额度查询失败，已保留有效采样，请重试");
                }
                if self
                    .verify_sampling_identities(&e.source_ids, &e.account_key)
                    .is_err()
                {
                    for source in &e.shared_source_ids {
                        self.store.pause_shared_source_estimates(source)?;
                    }
                    return Ok(serde_json::to_value(
                        self.store.finish_estimate_for(id, credits)?,
                    )?);
                }
            }
            self.progress("保存采样结果");
            return Ok(serde_json::to_value(
                self.store.finish_estimate_for(id, credits)?,
            )?);
        }
        ensure!(["start", "restart"].contains(&action), "未知估值操作");
        ensure!(
            p["confirmed"] == true,
            "请确认采样期间仅使用目标订阅，且已纳入所有用量来源"
        );
        let old = if method.ends_with("restart") {
            Some(self.store.estimate_for(id, credits)?)
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
        ensure!(
            !self.store.settings()?.sources.iter().any(
                |s| ids.contains(&s.id) && s.auto_valuation.as_ref().is_some_and(|a| a.enabled)
            ),
            "请先关闭所选来源的自动估值，再开始手动采样"
        );
        source_config(&self.store.settings()?, &ids, key)?;
        self.verify_sampling_identities(&ids, key)?;
        self.sync_estimate_sources(&ids)?;
        let (_, account) = key.split_once(':').context("账户格式错误")?;
        self.progress("读取实时额度");
        let results = self
            .read_quotas(&json!({"accountId":account,"accountKey":key,"samplingSourceIds":ids}))?;
        let q = results
            .iter()
            .find(|q| format!("{}:{}", q.provider, q.account_id) == key)
            .context("未读取到账户额度")?;
        self.verify_sampling_identities(&ids, key)?;
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
    fn observe_credit(&self, e: &mut CreditEstimate, q: &QuotaSnapshot) -> Result<()> {
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
            self.calculate_credit(e)?;
        }
        self.save_estimate(&Estimate::Credit(e.clone()))
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
        let tx = self.write_transaction()?;
        let shared_source_ids = shared_sources(&settings, &ids);
        self.ensure_shared_sample_available(&shared_source_ids, &key)?;
        if let Some(id) = previous {
            let mut old = self.estimate_for(id, true)?;
            ensure!(
                matches!(old, Estimate::Credit(_))
                    && old.account_key == key
                    && old.status == "pending",
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
                .any(|e| matches!(e, Estimate::Credit(_))
                    && e.account_key == key
                    && e.status != "completed"),
            "该账户已有 credits 采样，请先结束"
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let e = CreditEstimate {
            common: EstimateCommon {
                calculation_status: "insufficientUsage".into(),
                calculation_version: 2,
                id: format!("credit-sample-{stamp}"),
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
                shared_source_ids,
                status: "active".into(),
                started_at: q.updated_at,
                checkpoint_at: q.updated_at,
                checkpoint_windows: q.windows.clone(),
                calculation_note: "样本不足：至少消耗 5 credits".into(),
                ..Default::default()
            },
            baseline_balance: Some(balance.into()),
            checkpoint_balance: Some(balance.into()),
            ..Default::default()
        };
        self.save_estimate(&Estimate::Credit(e.clone()))?;
        tx.commit()?;
        Ok(Estimate::Credit(e))
    }
}
