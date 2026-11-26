//! Passive capacity calibration from paired, authoritative quota observations.
use crate::contracts::{FIVE_HOUR_MINUTES, WEEK_MINUTES};
use crate::{estimates::window_id, models::*, store::Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capacity {
    pub ratio: Option<f64>,
    pub samples: usize,
    pub weekly_percent: f64,
    pub updated_at: i64,
}
pub(crate) struct CapacitySample {
    stamp: i64,
    five: f64,
    week: f64,
}

impl Capacity {
    pub(crate) fn from_samples(samples: &[CapacitySample], at: i64) -> Self {
        let mut result = Self::default();
        let (mut five, mut week) = (0.0, 0.0);
        for sample in samples
            .iter()
            .filter(|s| (at - 90 * 86400..=at).contains(&s.stamp))
        {
            let weight = 2_f64.powf(-(at - sample.stamp) as f64 / (7.0 * 86400.0));
            five += weight * sample.five;
            week += weight * sample.week;
            result.samples += 1;
            result.weekly_percent += sample.week;
            result.updated_at = sample.stamp;
        }
        result.ratio = (week > 0.0).then(|| five / week);
        result
    }
}

#[derive(Serialize, Deserialize)]
struct Baseline {
    first: QuotaSnapshot,
    last: QuotaSnapshot,
}

pub fn overall(q: &QuotaSnapshot, minutes: i64) -> Option<&QuotaWindow> {
    if crate::providers::antigravity(&q.provider) {
        return None;
    }
    let windows: Vec<_> = q
        .windows
        .iter()
        .filter(|w| w.window_minutes == Some(minutes))
        .collect();
    if windows.len() == 1 {
        return Some(windows[0]);
    }
    if q.provider == "claude" {
        let id = if minutes == FIVE_HOUR_MINUTES {
            "five_hour"
        } else {
            "seven_day"
        };
        return windows.into_iter().find(|w| w.id == id);
    }
    None
}
pub fn window<'a>(q: &'a QuotaSnapshot, minutes: i64, group: &str) -> Option<&'a QuotaWindow> {
    if group.is_empty() {
        return overall(q, minutes);
    }
    let mut rows = q.windows.iter().filter(|w| {
        w.window_minutes == Some(minutes) && crate::antigravity::window_group(w) == group
    });
    let first = rows.next();
    if rows.next().is_none() { first } else { None }
}
fn scope_key(account: &str, group: &str) -> String {
    if group.is_empty() {
        account.into()
    } else {
        format!("{account}#group:{}", hash(group))
    }
}
pub fn valid(w: &QuotaWindow) -> bool {
    w.used_percent.is_finite() && (0.0..=100.0).contains(&w.used_percent)
}
fn comparable(a: &QuotaSnapshot, b: &QuotaSnapshot, group: &str) -> bool {
    a.plan == b.plan
        && b.updated_at > a.updated_at
        && b.updated_at - a.updated_at <= 18000
        && [FIVE_HOUR_MINUTES, WEEK_MINUTES]
            .into_iter()
            .all(
                |minutes| match (window(a, minutes, group), window(b, minutes, group)) {
                    (Some(old), Some(new)) => {
                        valid(old)
                            && valid(new)
                            && window_id(old) == window_id(new)
                            && old.resets_at.is_some_and(|t| t > b.updated_at)
                            && old.resets_at == new.resets_at
                            && new.used_percent + 0.000001 >= old.used_percent
                    }
                    _ => false,
                },
            )
}
impl Store {
    pub fn learn_capacity(&self, q: &QuotaSnapshot) -> Result<()> {
        if crate::providers::antigravity(&q.provider) {
            let groups: std::collections::BTreeSet<_> = q
                .windows
                .iter()
                .map(crate::antigravity::window_group)
                .filter(|g| crate::antigravity::group_kind(g).is_some())
                .collect();
            for group in groups {
                self.learn_capacity_group(q, group)?;
            }
            Ok(())
        } else {
            self.learn_capacity_group(q, "")
        }
    }
    fn learn_capacity_group(&self, q: &QuotaSnapshot, group: &str) -> Result<()> {
        if q.origin == "log" || q.account_id.is_empty() {
            return Ok(());
        }
        let account = scope_key(&format!("{}:{}", q.provider, q.account_id), group);
        let key = format!("capacity:{account}");
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
            .optional()?;
        let old: Option<Baseline> = raw.map(|s| serde_json::from_str(&s)).transpose()?;
        if old
            .as_ref()
            .is_some_and(|s| q.updated_at <= s.last.updated_at)
            && q.error.is_none()
        {
            return Ok(());
        }
        if q.error.is_some()
            || ![FIVE_HOUR_MINUTES, WEEK_MINUTES]
                .into_iter()
                .all(|m| window(q, m, group).is_some_and(valid))
        {
            self.db.execute("DELETE FROM kv WHERE key=?1", [&key])?;
            return Ok(());
        }
        let first = old
            .as_ref()
            .filter(|s| comparable(&s.last, q, group))
            .map(|s| s.first.clone())
            .unwrap_or_else(|| q.clone());
        let five = window(q, FIVE_HOUR_MINUTES, group).unwrap().used_percent
            - window(&first, FIVE_HOUR_MINUTES, group)
                .unwrap()
                .used_percent;
        let week = window(q, WEEK_MINUTES, group).unwrap().used_percent
            - window(&first, WEEK_MINUTES, group).unwrap().used_percent;
        if old.as_ref().is_some_and(|s| s.last.plan != q.plan) {
            self.db.execute(
                "DELETE FROM capacity_samples WHERE account_key=?1",
                [&account],
            )?;
        }
        let committed = five + 0.000001 >= 5.0 && week + 0.000001 >= 1.0;
        if committed {
            self.db.execute(
                "INSERT OR IGNORE INTO capacity_samples VALUES(?1,?2,?3,?4)",
                params![account, q.updated_at, five, week],
            )?;
        }
        let next = Baseline {
            first: if committed { q.clone() } else { first },
            last: q.clone(),
        };
        self.db.execute(
            "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, serde_json::to_string(&next)?],
        )?;
        self.db.execute(
            "DELETE FROM capacity_samples WHERE stamp<?1",
            [now() - 90 * 86400],
        )?;
        Ok(())
    }
    pub fn capacity(&self, account: &str, at: i64) -> Result<Capacity> {
        self.capacity_for_group(account, "", at)
    }
    pub fn capacity_for_group(&self, account: &str, group: &str, at: i64) -> Result<Capacity> {
        let samples = self.capacity_samples(account, group, None, at - 90 * 86400, at)?;
        Ok(Capacity::from_samples(&samples, at))
    }
    pub(crate) fn capacity_samples(
        &self,
        account: &str,
        group: &str,
        plan: Option<&str>,
        from: i64,
        to: i64,
    ) -> Result<Vec<CapacitySample>> {
        // Legacy capacity rows have no plan column. Pair them with the quota
        // observation that produced the sample when reconstructing a plan's history.
        let mut stmt = self.db.prepare_cached(
            "SELECT s.stamp,s.five,s.week FROM capacity_samples s
             WHERE s.account_key=?1 AND s.stamp BETWEEN ?2 AND ?3
               AND (?4 IS NULL OR ?4=(
                   SELECT lower(trim(json_extract(q.payload,'$.plan'))) FROM quota_history q
                   WHERE q.account_key=?5 AND q.stamp=s.stamp
                   ORDER BY q.id DESC LIMIT 1))
             ORDER BY s.stamp",
        )?;
        let rows = stmt.query_map(
            params![scope_key(account, group), from, to, plan, account],
            |r| {
                Ok(CapacitySample {
                    stamp: r.get(0)?,
                    five: r.get(1)?,
                    week: r.get(2)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    pub fn bootstrap_capacity(&self) -> Result<()> {
        if self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv WHERE key='capacity.initialized.v2')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let mut stmt = self
            .db
            .prepare("SELECT payload FROM quota_history WHERE stamp>=?1 ORDER BY stamp,id")?;
        let rows = stmt
            .query_map([now() - 90 * 86400], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for raw in rows {
            self.learn_capacity(&serde_json::from_str(&raw)?)?;
        }
        self.db.execute(
            "INSERT OR IGNORE INTO kv VALUES('capacity.initialized.v2','true')",
            [],
        )?;
        Ok(())
    }
}
