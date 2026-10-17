//! Confirm resets only from accepted live queries, independently of imported history.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const TIME_TOLERANCE: i64 = 60;
const PERCENT_TOLERANCE: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub unexpected: bool,
    pub natural: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            unexpected: true,
            natural: false,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Overrides {
    pub unexpected: Option<bool>,
    pub natural: Option<bool>,
}
impl Overrides {
    pub fn resolve(&self, global: &Preferences) -> Preferences {
        Preferences {
            unexpected: self.unexpected.unwrap_or(global.unexpected),
            natural: self.natural.unwrap_or(global.natural),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Unexpected,
    Natural,
}
impl Preferences {
    fn allows(&self, kind: Kind) -> bool {
        match kind {
            Kind::Unexpected => self.unexpected,
            Kind::Natural => self.natural,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowReset {
    pub id: String,
    pub name: String,
    pub previous_used_percent: f64,
    pub used_percent: f64,
    pub expected_reset_at: i64,
    pub resets_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetEvent {
    pub id: String,
    pub account_key: String,
    pub provider: String,
    pub name: String,
    pub kind: Kind,
    pub detected_at: i64,
    pub windows: Vec<WindowReset>,
}
#[derive(Serialize, Deserialize)]
struct Baseline {
    configuration: String,
    quota: QuotaSnapshot,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    configuration: String,
    event: ResetEvent,
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS quota_reset_baselines(account_key TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS quota_reset_notifications(id TEXT PRIMARY KEY,account_key TEXT NOT NULL,payload TEXT NOT NULL);")?;
    Ok(())
}
fn eligible<'a>(settings: &'a Settings, key: &str) -> Option<&'a Account> {
    settings.accounts.iter().find(|a| {
        crate::accounts::key(a) == key
            && a.quota_enabled
            && !a.archived
            && a.provider != "deepseek"
            && settings
                .sources
                .iter()
                .any(|s| settings.source_enabled(s) && a.uses_source(s))
    })
}
pub fn monitoring_enabled(settings: &Settings) -> bool {
    settings.accounts.iter().any(|a| {
        eligible(settings, &crate::accounts::key(a)).is_some() && {
            let p = a
                .quota_reset_notifications
                .resolve(&settings.quota_reset_notifications);
            p.unexpected || p.natural
        }
    })
}
fn valid(w: &QuotaWindow) -> bool {
    !w.id.is_empty() && w.used_percent.is_finite() && (0.0..=100.0).contains(&w.used_percent)
}
fn classify(old: &QuotaWindow, new: &QuotaWindow, stamp: i64) -> Option<Kind> {
    if !valid(old)
        || !valid(new)
        || old.window_minutes != new.window_minutes
        || old.group_id != new.group_id
        || old.used_percent < PERCENT_TOLERANCE
    {
        return None;
    }
    let expected = old.resets_at.filter(|t| *t > 0)?;
    let recovered = old.used_percent - new.used_percent + 1e-9 >= PERCENT_TOLERANCE;
    let full = recovered && new.used_percent <= PERCENT_TOLERANCE;
    let advanced = new
        .resets_at
        .is_some_and(|t| t > stamp && t.saturating_sub(expected) > TIME_TOLERANCE);
    let natural = stamp >= expected.saturating_sub(TIME_TOLERANCE);
    if full || (advanced && (natural || recovered)) {
        Some(if natural {
            Kind::Natural
        } else {
            Kind::Unexpected
        })
    } else {
        None
    }
}
impl Store {
    /// Caller holds the accepted-query write transaction: baseline, quota and events commit together.
    pub(crate) fn observe_quota_reset(
        &self,
        q: &QuotaSnapshot,
        settings: &Settings,
        configuration: &str,
    ) -> Result<()> {
        if q.error.is_some()
            || !matches!(q.origin.as_str(), "live" | "command")
            || q.updated_at <= 0
        {
            return Ok(());
        }
        let key = format!("{}:{}", q.provider, q.account_id);
        let Some(account) = eligible(settings, &key) else {
            return Ok(());
        };
        let previous: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM quota_reset_baselines WHERE account_key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let previous: Option<Baseline> = previous.map(|v| serde_json::from_str(&v)).transpose()?;
        if previous
            .as_ref()
            .is_some_and(|b| b.quota.updated_at >= q.updated_at)
        {
            return Ok(());
        }
        if matches!(
            q.plan.as_deref().map(str::to_ascii_lowercase).as_deref(),
            Some("api" | "free")
        ) {
            self.db.execute(
                "DELETE FROM quota_reset_baselines WHERE account_key=?1",
                [&key],
            )?;
            return Ok(());
        }
        if q.windows.is_empty() || q.windows.iter().any(|w| !valid(w)) {
            return Ok(());
        }
        // Ambiguous window IDs cannot provide a stable comparison.
        let ids: std::collections::HashSet<_> = q.windows.iter().map(|w| &w.id).collect();
        if ids.len() != q.windows.len() {
            return Ok(());
        }
        let preferences = account
            .quota_reset_notifications
            .resolve(&settings.quota_reset_notifications);
        if let Some(baseline) = previous.filter(|b| {
            b.configuration == configuration
                && b.quota.source_id == q.source_id
                && b.quota.plan == q.plan
                && b.quota.identity.as_ref().map(|i| &i.key) == q.identity.as_ref().map(|i| &i.key)
        }) {
            for kind in [Kind::Unexpected, Kind::Natural] {
                let windows: Vec<_> = q
                    .windows
                    .iter()
                    .filter_map(|new| {
                        let old = baseline.quota.windows.iter().find(|w| w.id == new.id)?;
                        if classify(old, new, q.updated_at) != Some(kind) {
                            return None;
                        }
                        Some(WindowReset {
                            id: new.id.clone(),
                            name: new.name.clone(),
                            previous_used_percent: old.used_percent,
                            used_percent: new.used_percent,
                            expected_reset_at: old.resets_at.unwrap(),
                            resets_at: new.resets_at,
                        })
                    })
                    .collect();
                if windows.is_empty() || !preferences.allows(kind) {
                    continue;
                }
                let event = ResetEvent {
                    id: crate::codex_auth::files::new_id(),
                    account_key: key.clone(),
                    provider: q.provider.clone(),
                    name: account.name.clone(),
                    kind,
                    detected_at: q.updated_at,
                    windows,
                };
                let pending = Pending {
                    configuration: configuration.into(),
                    event,
                };
                self.db.execute(
                    "INSERT INTO quota_reset_notifications VALUES(?1,?2,?3)",
                    params![pending.event.id, key, serde_json::to_string(&pending)?],
                )?;
            }
        }
        let next = Baseline {
            configuration: configuration.into(),
            quota: q.clone(),
        };
        self.db.execute("INSERT INTO quota_reset_baselines VALUES(?1,?2) ON CONFLICT(account_key) DO UPDATE SET payload=excluded.payload", params![key, serde_json::to_string(&next)?])?;
        Ok(())
    }
    fn pending_resets(&self) -> Result<Vec<Pending>> {
        let mut statement = self
            .db
            .prepare("SELECT payload FROM quota_reset_notifications ORDER BY rowid")?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }
    fn reset_still_enabled(settings: &Settings, pending: &Pending) -> Result<bool> {
        let Some(account) = eligible(settings, &pending.event.account_key) else {
            return Ok(false);
        };
        Ok(account
            .quota_reset_notifications
            .resolve(&settings.quota_reset_notifications)
            .allows(pending.event.kind)
            && crate::quota_queries::fingerprint(settings, account)? == pending.configuration)
    }
    pub(crate) fn reconcile_reset_notifications(&self, settings: &Settings) -> Result<()> {
        for pending in self.pending_resets()? {
            if !Self::reset_still_enabled(settings, &pending)? {
                self.db.execute(
                    "DELETE FROM quota_reset_notifications WHERE id=?1",
                    [&pending.event.id],
                )?;
            }
        }
        let keys = self
            .db
            .prepare("SELECT account_key FROM quota_reset_baselines")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for key in keys {
            if eligible(settings, &key).is_none() {
                self.db.execute(
                    "DELETE FROM quota_reset_baselines WHERE account_key=?1",
                    [&key],
                )?;
            }
        }
        Ok(())
    }
    /// At most once: consume before handing to an OS, including denied/failed deliveries.
    pub fn take_reset_notifications(&self, since: i64) -> Result<Vec<ResetEvent>> {
        let pending: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM quota_reset_notifications)",
            [],
            |r| r.get(0),
        )?;
        if !pending {
            return Ok(Vec::new());
        }
        let tx = self.write_transaction()?;
        let settings = self.settings()?;
        let mut events = Vec::new();
        for pending in self.pending_resets()? {
            if pending.event.detected_at >= since && Self::reset_still_enabled(&settings, &pending)?
            {
                let mut event = pending.event;
                if let Some(account) = eligible(&settings, &event.account_key) {
                    event.name = account.name.clone();
                }
                events.push(event);
            }
        }
        self.db
            .execute("DELETE FROM quota_reset_notifications", [])?;
        tx.commit()?;
        Ok(events)
    }
}

#[cfg(test)]
mod tests;
