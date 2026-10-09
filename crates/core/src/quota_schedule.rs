//! Per-account polling shared by all UI windows and engine lanes.
use crate::{Engine, file_lock::FileLock, models::Account};
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

fn interval(account: &Account, global: u64) -> i64 {
    account
        .quota_refresh_seconds
        .unwrap_or(global)
        .clamp(10, 86400) as i64
}
fn due(last: Option<i64>, now: i64, seconds: i64) -> bool {
    last.is_none_or(|last| now < last || now - last >= seconds)
}
impl Engine {
    pub(crate) fn refresh_scheduled_quotas(&mut self, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let automatic = params["dueOnly"].as_bool().unwrap_or(false);
        let mut result = vec![];
        for account in settings.accounts.iter().filter(|a| {
            a.quota_enabled
                && !a.archived
                && params["accountId"].as_str().is_none_or(|id| a.id == id)
                && params["accountKey"]
                    .as_str()
                    .is_none_or(|key| crate::accounts::key(a) == key)
        }) {
            if automatic
                && !settings.sources.iter().any(|s| {
                    s.enabled
                        && account.uses_source(s)
                        && s.host_id.as_ref().is_none_or(|id| {
                            settings.hosts.iter().any(|h| h.id == *id && h.enabled)
                        })
                })
            {
                continue;
            }
            let key = crate::accounts::key(account);
            let filename: String = key.bytes().map(|b| format!("{b:02x}")).collect();
            let root = std::path::Path::new(self.store.db.path().context("数据目录不可用")?)
                .parent()
                .context("数据目录不可用")?;
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(root.join(format!("quota-{filename}.lock")))?;
            let _lock = match FileLock::try_exclusive(lock) {
                Ok(lock) => lock,
                Err(std::fs::TryLockError::WouldBlock) => continue,
                Err(error) => return Err(error.into()),
            };
            let attempt_key = format!("quota-attempt:{key}");
            let last: Option<i64> = self
                .store
                .db
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM kv WHERE key=?1",
                    [&attempt_key],
                    |r| r.get(0),
                )
                .optional()?;
            let now = chrono::Utc::now().timestamp();
            if automatic && !due(last, now, interval(account, settings.refresh_seconds)) {
                continue;
            }
            self.store.db.execute(
                "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![attempt_key, now.to_string()],
            )?;
            // Sampling history belongs only to the account actually being queried.
            let mut ids: Vec<_> = self
                .store
                .estimates()?
                .iter()
                .filter(|e| e.status == "active" && e.account_key == key)
                .flat_map(|e| e.source_ids.clone())
                .collect();
            ids.sort();
            ids.dedup();
            if !ids.is_empty() {
                let _ = self.sync_estimate_sources(&ids);
            }
            let mut selected = params.clone();
            selected["accountKey"] = json!(key);
            let rows = self.read_quotas(&selected)?;
            for row in &rows {
                self.store.quota(row)?;
            }
            result.extend(rows);
        }
        Ok(serde_json::to_value(result)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_queries_obey_independent_intervals_and_manual_refresh_bypasses_due() {
        use crate::models::*;
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        let mut settings = Settings::default();
        for (id, seconds) in [("fast", 30), ("slow", 600)] {
            settings.accounts.push(Account {
                id: id.into(),
                name: id.into(),
                provider: "custom".into(),
                quota_refresh_seconds: Some(seconds),
                ..Default::default()
            });
            settings.sources.push(Source {
                id: id.into(),
                name: id.into(),
                provider: "custom".into(),
                account_id: id.into(),
                path: "/nonexistent/fixture".into(),
                ..Default::default()
            });
        }
        engine.store.save_settings(&settings).unwrap();
        let filename: String = "custom:fast".bytes().map(|b| format!("{b:02x}")).collect();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.path().join(format!("quota-{filename}.lock")))
            .unwrap();
        // Model a descriptor inherited by a subprocess started by a parallel test.
        let _inherited_lock = lock.try_clone().unwrap();
        let lock = FileLock::try_exclusive(lock).unwrap();
        assert!(
            engine
                .call("quotas.refresh", json!({"accountKey":"custom:fast"}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        drop(lock);
        let rows = engine
            .call("quotas.refresh", json!({"dueOnly":true}))
            .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2);
        assert!(rows[0]["error"].is_string());
        assert!(
            engine
                .call("quotas.refresh", json!({"dueOnly":true}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        engine
            .store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key LIKE 'quota-attempt:%'",
                [(chrono::Utc::now().timestamp() - 60).to_string()],
            )
            .unwrap();
        let rows = engine
            .call("quotas.refresh", json!({"dueOnly":true}))
            .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["accountId"], "fast");
        assert_eq!(
            engine
                .call("quotas.refresh", json!({"accountKey":"custom:slow"}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut settings = engine.store.settings().unwrap();
        settings.accounts[0].archived = true;
        settings.accounts[1].quota_enabled = false;
        engine.store.save_settings(&settings).unwrap();
        assert!(
            engine
                .call("quotas.refresh", json!({"dueOnly":true}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        settings.accounts[0].quota_refresh_seconds = Some(29);
        assert!(engine.store.save_settings(&settings).is_err());
    }

    #[test]
    fn independent_intervals_and_clock_recovery() {
        let a = Account {
            quota_refresh_seconds: Some(60),
            ..Default::default()
        };
        let b = Account::default();
        assert!(due(Some(100), 160, interval(&a, 300)));
        assert!(!due(Some(100), 160, interval(&b, 300)));
        assert!(due(None, 160, 300));
        assert!(due(Some(200), 160, 300));
        assert!(!due(Some(160), 161, 60));
        assert!(due(Some(100), 400, interval(&b, 300)));
    }
}
