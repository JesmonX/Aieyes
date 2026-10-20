//! Rebuildable interval projection. Original estimates and frozen prices remain immutable.
use crate::{
    estimates::{CreditEstimate, Estimate, EstimateCommon, QuotaEstimate},
    models::*,
    store::Store,
};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Memory = Option<((i64, i64, i64, i64, u64), Vec<Summary>)>;

const DAYS: i64 = 90;
const HALF_LIFE: f64 = 7.0 * 86400.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    #[serde(flatten)]
    pub estimate: Estimate,
    pub sample_count: usize,
    pub lookback_days: i64,
    pub half_life_days: i64,
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS automatic_sample_dirty(kind TEXT NOT NULL,id TEXT NOT NULL,PRIMARY KEY(kind,id));
        CREATE TABLE IF NOT EXISTS automatic_samples(kind TEXT NOT NULL,record_id TEXT NOT NULL,segment INTEGER NOT NULL,account_key TEXT NOT NULL,plan TEXT NOT NULL,group_id TEXT NOT NULL,period TEXT NOT NULL,started_at INTEGER NOT NULL,ended_at INTEGER NOT NULL,cost REAL NOT NULL,consumed REAL NOT NULL,tokens INTEGER NOT NULL,source_ids TEXT NOT NULL,PRIMARY KEY(kind,record_id,segment,period));
        CREATE INDEX IF NOT EXISTS automatic_samples_time ON automatic_samples(ended_at,account_key);
        CREATE INDEX IF NOT EXISTS automatic_samples_account ON automatic_samples(account_key,plan,group_id,period,started_at,ended_at);
        CREATE TRIGGER IF NOT EXISTS automatic_sample_insert AFTER INSERT ON estimates WHEN COALESCE(json_extract(NEW.payload,'$.samplingMode'),'')='automatic' BEGIN INSERT INTO automatic_sample_dirty VALUES(NEW.kind,NEW.id) ON CONFLICT(kind,id) DO NOTHING; END;
        CREATE TRIGGER IF NOT EXISTS automatic_sample_update AFTER UPDATE ON estimates WHEN OLD.payload IS NOT NEW.payload AND (COALESCE(json_extract(OLD.payload,'$.samplingMode'),'')='automatic' OR COALESCE(json_extract(NEW.payload,'$.samplingMode'),'')='automatic') BEGIN INSERT INTO automatic_sample_dirty VALUES(NEW.kind,NEW.id) ON CONFLICT(kind,id) DO NOTHING; END;
        CREATE TRIGGER IF NOT EXISTS automatic_sample_delete AFTER DELETE ON estimates WHEN COALESCE(json_extract(OLD.payload,'$.samplingMode'),'')='automatic' BEGIN INSERT INTO automatic_sample_dirty VALUES(OLD.kind,OLD.id) ON CONFLICT(kind,id) DO NOTHING; END;")?;
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv WHERE key='automatic-samples.initialized.v1')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        db.execute_batch(
            "INSERT OR IGNORE INTO automatic_sample_dirty SELECT kind,id FROM estimates WHERE json_extract(payload,'$.samplingMode')='automatic';
            INSERT INTO kv VALUES('automatic-samples.initialized.v1','true');",
        )?;
    }
    Ok(())
}

#[derive(Default)]
struct Pool {
    cost: f64,
    consumed: f64,
    tokens: u64,
    count: usize,
    intervals: BTreeMap<i64, i64>,
}
impl Pool {
    fn value(&self, multiplier: f64) -> Option<f64> {
        crate::estimates::finite_value(self.cost, self.consumed, multiplier)
    }
    fn insert(
        &mut self,
        start: i64,
        end: i64,
        cost: f64,
        consumed: f64,
        tokens: u64,
        at: i64,
    ) -> bool {
        // Prefer live evidence over replay, then a containing interval over fragments.
        if self
            .intervals
            .range(..end)
            .next_back()
            .is_some_and(|(_, &e)| e > start)
        {
            return false;
        }
        self.intervals.insert(start, end);
        let weight = 2_f64.powf(-(at - end) as f64 / HALF_LIFE);
        self.cost += weight * cost;
        self.consumed += weight * consumed;
        self.tokens = self.tokens.saturating_add(tokens);
        self.count += 1;
        true
    }
}
#[derive(Default)]
struct Accumulator {
    pools: BTreeMap<String, Pool>,
    source_ids: BTreeSet<String>,
    first: i64,
    last: i64,
}

impl Store {
    fn historical_plan(&self, e: &Estimate) -> Result<String> {
        if let Some(plan) = e
            .quota()
            .and_then(|q| q.quota_plan.as_ref())
            .or_else(|| e.credit().and_then(|c| c.quota_plan.as_ref()))
        {
            return Ok(plan.trim().to_ascii_lowercase());
        }
        // Old credit records did not retain the plan. Only recover it when both
        // endpoints and every intervening observation agree.
        let mut plans = BTreeSet::new();
        for stamp in [e.started_at, e.checkpoint_at] {
            let plan: Option<String> = self.db.query_row("SELECT json_extract(payload,'$.plan') FROM quota_history WHERE account_key=?1 AND stamp<=?2 ORDER BY stamp DESC,id DESC LIMIT 1", params![e.account_key,stamp], |r|r.get(0)).optional()?.flatten();
            let Some(plan) = plan else {
                return Ok(String::new());
            };
            plans.insert(plan.trim().to_ascii_lowercase());
        }
        let mut query = self.db.prepare("SELECT DISTINCT lower(json_extract(payload,'$.plan')) FROM quota_history WHERE account_key=?1 AND stamp>?2 AND stamp<=?3")?;
        for plan in query.query_map(params![e.account_key, e.started_at, e.checkpoint_at], |r| {
            r.get::<_, Option<String>>(0)
        })? {
            let Some(plan) = plan? else {
                return Ok(String::new());
            };
            plans.insert(plan);
        }
        Ok(if plans.len() == 1 {
            plans.into_iter().next().unwrap_or_default()
        } else {
            String::new()
        })
    }
    pub(crate) fn project_automatic_samples(&self) -> Result<()> {
        if !self.db.is_autocommit() {
            return Ok(());
        }
        if !self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM automatic_sample_dirty)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let tx = self.write_transaction()?;
        let mut query = self
            .db
            .prepare("SELECT kind,id FROM automatic_sample_dirty")?;
        let dirty = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (kind, id) in dirty {
            self.db.execute(
                "DELETE FROM automatic_samples WHERE kind=?1 AND record_id=?2",
                params![kind, id],
            )?;
            if let Ok(e) = self.estimate_for(&id, kind == "credits")
                && e.sampling_mode == "automatic"
                && e.rolling_excluded_reason.is_empty()
            {
                let plan = self.historical_plan(&e)?;
                let group = e.quota().and_then(|q| q.group_id.as_deref()).unwrap_or("");
                let group = crate::antigravity::group_kind(group).unwrap_or(group);
                let primary = if kind == "credits" {
                    "credits"
                } else if e.quota().is_some_and(|q| q.valuation_mode == "fiveHour") {
                    "fiveHour"
                } else {
                    "weekly"
                };
                for (index, s) in e.automatic_segments.iter().enumerate() {
                    if !s.valid
                        || (e.account_key.starts_with("codex:")
                            && s.attribution_version != crate::codex_attribution::VERSION)
                        || s.tokens == 0
                        || s.tokens != s.priced_tokens
                        || s.started_at >= s.ended_at
                    {
                        continue;
                    }
                    let add = |period: &str, cost: f64, consumed: f64| -> Result<()> {
                        if consumed > 0.0 && consumed.is_finite() && cost.is_finite() && cost >= 0.0
                        {
                            self.db.execute("INSERT INTO automatic_samples VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![kind,id,index as i64,e.account_key,plan,group,period,s.started_at,s.ended_at,cost,consumed,s.tokens,serde_json::to_string(&e.source_ids)?])?;
                        }
                        Ok(())
                    };
                    add(
                        if s.pool == "weekly" {
                            "weekly"
                        } else {
                            primary
                        },
                        s.cost,
                        s.consumed,
                    )?;
                    if s.pool != "weekly" && primary != "weekly" {
                        add("weekly", s.weekly_cost, s.weekly_consumed)?;
                    }
                }
            }
            self.db.execute(
                "DELETE FROM automatic_sample_dirty WHERE kind=?1 AND id=?2",
                params![kind, id],
            )?;
        }
        tx.commit()?;
        *self.automatic_summary_memory.borrow_mut() = None;
        Ok(())
    }
    pub fn automatic_estimate_summaries(&self, at: i64) -> Result<Vec<Summary>> {
        self.project_automatic_samples()?;
        let revision = crate::runtime::revision(&self.db, "estimates")?;
        let settings_revision = crate::runtime::revision(&self.db, "settings")?;
        let quota_revision = crate::runtime::revision(&self.db, "quotas")?;
        let key = (
            revision,
            settings_revision,
            quota_revision,
            at / 60,
            self.db.total_changes(),
        );
        if let Some((cached, rows)) = self.automatic_summary_memory.borrow().as_ref()
            && *cached == key
        {
            return Ok(rows.clone());
        }
        let settings = self.settings()?;
        let mut query = self.db.prepare("SELECT s.account_key,s.plan,s.group_id,s.period,s.started_at,s.ended_at,s.cost,s.consumed,s.tokens,s.source_ids,s.kind FROM automatic_samples s WHERE s.ended_at>=?1 AND s.ended_at<=?2 AND NOT EXISTS(SELECT 1 FROM estimates e WHERE e.kind=s.kind AND CAST(json_extract(e.payload,'$.originalEstimateId') AS TEXT)=s.record_id) ORDER BY (s.record_id LIKE 'auto-replay-%'),s.started_at,s.ended_at DESC,s.record_id")?;
        let mut groups = BTreeMap::<(String, String, String, String), Accumulator>::new();
        let rows = query.query_map(params![at - DAYS * 86400, at], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, f64>(7)?,
                r.get::<_, u64>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?;
        for row in rows {
            let (account, plan, group, period, start, end, cost, consumed, tokens, sources, kind) =
                row?;
            if plan.is_empty() {
                continue;
            }
            let acc = groups.entry((account, plan, group, kind)).or_default();
            if acc
                .pools
                .entry(period)
                .or_default()
                .insert(start, end, cost, consumed, tokens, at)
            {
                acc.first = if acc.first == 0 {
                    start
                } else {
                    acc.first.min(start)
                };
                acc.last = acc.last.max(end);
                acc.source_ids
                    .extend(serde_json::from_str::<Vec<String>>(&sources)?);
            }
        }
        let mut result = Vec::new();
        for ((account, plan, group, kind), acc) in groups {
            let empty = Pool::default();
            let five = acc.pools.get("fiveHour").unwrap_or(&empty);
            let week = acc.pools.get("weekly").unwrap_or(&empty);
            let credit = acc.pools.get("credits").unwrap_or(&empty);
            let main = if kind == "credits" {
                credit
            } else if five.count > 0 {
                five
            } else {
                week
            };
            let sources: Vec<_> = acc.source_ids.into_iter().collect();
            let active = settings.sources.iter().any(|s| {
                sources.contains(&s.id)
                    && settings.source_enabled(s)
                    && s.auto_valuation.as_ref().is_some_and(|c| c.enabled)
            });
            let common = EstimateCommon {
                id: format!(
                    "rolling-{}",
                    hash(&format!("{account}:{plan}:{group}:{kind}"))
                ),
                account_key: account.clone(),
                sampling_mode: "rolling".into(),
                status: if active { "active" } else { "completed" }.into(),
                started_at: acc.first,
                checkpoint_at: acc.last,
                ended_at: (!active).then_some(acc.last),
                source_names: settings
                    .sources
                    .iter()
                    .filter(|s| sources.contains(&s.id))
                    .map(|s| s.name.clone())
                    .collect(),
                source_ids: sources,
                cost: main.cost,
                total_tokens: main.tokens,
                priced_tokens: main.tokens,
                calculation_status: "ready".into(),
                calculation_version: 4,
                calculation_note: format!(
                    "滚动估值 · 最近 90 天 · 权重每 7 天减半 · {} 个有效额度片段",
                    acc.pools.values().map(|p| p.count).sum::<usize>()
                ),
                ..Default::default()
            };
            let estimate = if kind == "credits" {
                Estimate::Credit(CreditEstimate {
                    common,
                    quota_plan: Some(plan),
                    consumed_credits: credit.consumed,
                    value_per500: credit.value(500.0),
                    value_per1000: credit.value(1000.0),
                    ..Default::default()
                })
            } else {
                let capacity_group = if group.is_empty() {
                    String::new()
                } else {
                    // Capacity keys historically use the provider's original group label.
                    let raw: Option<String> = self
                        .db
                        .query_row(
                            "SELECT payload FROM quotas WHERE account_key=?1",
                            [&account],
                            |r| r.get(0),
                        )
                        .optional()?;
                    raw.and_then(|r| serde_json::from_str::<QuotaSnapshot>(&r).ok())
                        .and_then(|q| {
                            q.windows
                                .iter()
                                .map(crate::antigravity::window_group)
                                .find(|g| crate::antigravity::group_kind(g) == Some(group.as_str()))
                                .map(str::to_owned)
                        })
                        .unwrap_or(group.clone())
                };
                let capacity = self.capacity_for_group(&account, &capacity_group, at)?;
                Estimate::Quota(QuotaEstimate {
                    common,
                    quota_plan: Some(plan),
                    group_id: (!group.is_empty()).then_some(capacity_group),
                    valuation_mode: if five.count > 0 { "fiveHour" } else { "weekly" }.into(),
                    five_hour_value: five.value(100.0),
                    weekly_value: week.value(100.0),
                    weekly_direct_value: week.value(100.0),
                    weekly_ratio_value: five
                        .value(100.0)
                        .zip(capacity.ratio)
                        .map(|(v, r)| v * r)
                        .filter(|v| v.is_finite()),
                    capacity,
                    consumed_percent: main.consumed,
                    ..Default::default()
                })
            };
            if estimate.has_value() {
                result.push(Summary {
                    estimate,
                    sample_count: acc.pools.values().map(|p| p.count).sum(),
                    lookback_days: DAYS,
                    half_life_days: 7,
                });
            }
        }
        *self.automatic_summary_memory.borrow_mut() = Some((key, result.clone()));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automatic::AutoSegment;
    fn record(id: &str, at: i64, cost: f64, consumed: f64) -> Estimate {
        Estimate::Quota(QuotaEstimate {
            common: EstimateCommon {
                id: id.into(),
                account_key: "codex:a".into(),
                sampling_mode: "automatic".into(),
                calculation_version: 4,
                status: "completed".into(),
                started_at: at - 30,
                checkpoint_at: at,
                automatic_segments: vec![AutoSegment {
                    attribution_version: crate::codex_attribution::VERSION,
                    valid: true,
                    pool: "primary".into(),
                    started_at: at - 30,
                    ended_at: at,
                    cost,
                    consumed,
                    tokens: 10,
                    priced_tokens: 10,
                    ..Default::default()
                }],
                ..Default::default()
            },
            quota_plan: Some("plus".into()),
            valuation_mode: "fiveHour".into(),
            ..Default::default()
        })
    }
    #[test]
    fn legacy_codex_samples_remain_unchanged_history_but_leave_live_summaries() {
        let root = tempfile::tempdir().unwrap();
        let at = now();
        let s = Store::open(root.path()).unwrap();
        let mut old = record("legacy", at, 30.0, 10.0);
        old.calculation_version = 3;
        old.automatic_segments[0].attribution_version = 0;
        old.cost = 30.0;
        old.status = "active".into();
        s.save_estimate(&old).unwrap();
        drop(s);
        let s = Store::open(root.path()).unwrap();
        let retained = s.estimate_for("legacy", false).unwrap();
        assert_eq!(retained.cost, 30.0);
        assert_eq!(retained.automatic_segments[0].cost, 30.0);
        assert_eq!(retained.status, "completed");
        assert!(!retained.rolling_excluded_reason.is_empty());
        assert!(s.automatic_estimate_summaries(at).unwrap().is_empty());
        s.save_estimate(&record("verified", at + 1, 2.0, 10.0))
            .unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at + 1).unwrap()[0].sample_count,
            1
        );
        drop(s);
        let s = Store::open(root.path()).unwrap();
        assert_eq!(
            serde_json::to_value(s.estimate_for("legacy", false).unwrap()).unwrap(),
            serde_json::to_value(retained).unwrap()
        );
    }
    #[test]
    fn rolling_weights_consumption_and_time_across_sealed_records() {
        let root = tempfile::tempdir().unwrap();
        let s = Store::open(root.path()).unwrap();
        let at = now();
        s.save_estimate(&record("old", at - 7 * 86400, 100.0, 10.0))
            .unwrap();
        s.save_estimate(&record("new", at, 20.0, 10.0)).unwrap();
        let rows = s.automatic_estimate_summaries(at).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sample_count, 2);
        assert!(
            (rows[0].estimate.quota().unwrap().five_hour_value.unwrap() - 100.0 * 70.0 / 15.0)
                .abs()
                < 1e-8
        );
        let wire = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(wire["kind"], "quota");
        assert_eq!(wire["samplingMode"], "rolling");
        let decoded: Summary = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.sample_count, 2);
        drop(s);
        let s = Store::open(root.path()).unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0].sample_count,
            2
        );
    }
    #[test]
    fn deduplicates_replay_overlap_and_excludes_replaced_invalid_manual_and_old_samples() {
        let root = tempfile::tempdir().unwrap();
        let s = Store::open(root.path()).unwrap();
        let at = now();
        let mut live = record("live", at, 1.0, 1.0);
        s.save_estimate(&live).unwrap();
        live.automatic_segments[0].cost = 2.0;
        s.save_estimate(&live).unwrap();
        let mut replay = live.clone();
        replay.id = "auto-replay-v4-copy".into();
        replay.automatic_segments[0].cost = 200.0;
        s.save_estimate(&replay).unwrap();
        let mut overlapping = record("auto-replay-overlap", at - 5, 100.0, 1.0);
        overlapping.automatic_segments[0].started_at = at - 40;
        s.save_estimate(&overlapping).unwrap();
        let mut manual = record("manual", at, 400.0, 1.0);
        manual.sampling_mode = String::new();
        s.save_estimate(&manual).unwrap();
        s.save_estimate(&record("expired", at - 91 * 86400, 400.0, 1.0))
            .unwrap();
        let mut invalid = record("invalid", at - 60, 400.0, 1.0);
        invalid.automatic_segments[0].valid = false;
        s.save_estimate(&invalid).unwrap();
        let rows = s.automatic_estimate_summaries(at).unwrap();
        assert_eq!(rows[0].sample_count, 1);
        assert_eq!(
            rows[0].estimate.quota().unwrap().five_hour_value,
            Some(200.0)
        );
        let mut replacement = live.clone();
        replacement.id = "replacement".into();
        replacement.original_estimate_id = Some("live".into());
        replacement.automatic_segments[0].cost = 3.0;
        s.save_estimate(&replacement).unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0]
                .estimate
                .quota()
                .unwrap()
                .five_hour_value,
            Some(300.0)
        );
        replacement.automatic_segments[0].valid = false;
        s.save_estimate(&replacement).unwrap();
        // The old original is still excluded; replay can now supply the interval.
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0].sample_count,
            1
        );
    }
    #[test]
    fn separates_accounts_plans_groups_weekly_and_credits() {
        let root = tempfile::tempdir().unwrap();
        let s = Store::open(root.path()).unwrap();
        let at = now();
        let a = record("a", at, 2.0, 1.0);
        s.save_estimate(&a).unwrap();
        let mut other = a.clone();
        other.id = "other-account".into();
        other.account_key = "codex:b".into();
        s.save_estimate(&other).unwrap();
        let mut pro = a.clone();
        pro.id = "pro".into();
        if let Estimate::Quota(q) = &mut pro {
            q.quota_plan = Some("pro".into());
        }
        s.save_estimate(&pro).unwrap();
        for (id, group) in [
            ("gemini", "Gemini Models"),
            ("claude", "Claude and GPT models"),
        ] {
            let mut e = a.clone();
            e.id = id.into();
            e.account_key = "antigravity:c".into();
            if let Estimate::Quota(q) = &mut e {
                q.group_id = Some(group.into());
                q.common.automatic_segments.push(AutoSegment {
                    pool: "weekly".into(),
                    consumed: 0.5,
                    cost: 2.0,
                    ..q.common.automatic_segments[0].clone()
                });
            }
            s.save_estimate(&e).unwrap();
        }
        s.save_estimate(&Estimate::Credit(CreditEstimate {
            common: (*a).clone(),
            quota_plan: Some("plus".into()),
            ..Default::default()
        }))
        .unwrap();
        let rows = s.automatic_estimate_summaries(at).unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows.iter()
                .find(|r| r.estimate.credit().is_some())
                .unwrap()
                .estimate
                .credit()
                .unwrap()
                .value_per1000,
            Some(2000.0)
        );
        for row in rows
            .iter()
            .filter(|r| r.estimate.account_key == "antigravity:c")
        {
            assert_eq!(row.sample_count, 2);
            assert_eq!(row.estimate.quota().unwrap().weekly_value, Some(400.0));
        }
    }
    #[test]
    fn projection_and_cache_recover_after_rolled_back_updates() {
        let root = tempfile::tempdir().unwrap();
        let s = Store::open(root.path()).unwrap();
        let at = now();
        let e = record("live", at, 2.0, 1.0);
        s.save_estimate(&e).unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0]
                .estimate
                .quota()
                .unwrap()
                .five_hour_value,
            Some(200.0)
        );
        s.db.execute_batch("BEGIN; UPDATE automatic_samples SET cost=40; UPDATE kv SET value=CAST(value AS INTEGER)+1 WHERE key='runtime.revision:estimates';").unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0]
                .estimate
                .quota()
                .unwrap()
                .five_hour_value,
            Some(4000.0)
        );
        s.db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0]
                .estimate
                .quota()
                .unwrap()
                .five_hour_value,
            Some(200.0)
        );
        let mut e = e.clone();
        e.automatic_segments[0].cost = 3.0;
        s.save_estimate(&e).unwrap();
        assert_eq!(
            s.automatic_estimate_summaries(at).unwrap()[0]
                .estimate
                .quota()
                .unwrap()
                .five_hour_value,
            Some(300.0)
        );
    }
}
