//! Per-account polling shared by all UI windows and engine lanes.
use crate::{
    Engine,
    models::{Account, QuotaSnapshot},
    quota_queries,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Mutex, mpsc},
};

fn interval(account: &Account, global: u64) -> i64 {
    account
        .quota_refresh_seconds
        .unwrap_or(global)
        .clamp(5, 86400) as i64
}
fn due(last: Option<i64>, now: i64, seconds: i64) -> bool {
    last.is_none_or(|last| now < last || now - last >= seconds)
}

/// Native desktop owner of polling; hidden webviews never need to keep timers alive.
#[derive(Default)]
pub struct Poller {
    confirmed: std::collections::HashSet<(String, i64)>,
}
impl Poller {
    pub fn next(&mut self, engine: &Engine) -> Result<Option<Value>> {
        let settings = engine.store.settings()?;
        let now = crate::models::now();
        let mut active = std::collections::HashSet::new();
        let mut expired = None;
        let mut query = engine.store.db.prepare("SELECT payload FROM quotas")?;
        for raw in query.query_map([], |r| r.get::<_, String>(0))? {
            let quota: QuotaSnapshot = serde_json::from_str(&raw?)?;
            let key = format!("{}:{}", quota.provider, quota.account_id);
            if !settings.accounts.iter().any(|a| {
                crate::accounts::key(a) == key
                    && a.quota_enabled
                    && !a.archived
                    && settings
                        .sources
                        .iter()
                        .any(|s| settings.source_enabled(s) && a.uses_source(s))
            }) {
                continue;
            }
            for window in quota.windows {
                if let Some(reset) = window.resets_at.filter(|r| *r > 0) {
                    let id = (key.clone(), reset);
                    if reset <= now && !self.confirmed.contains(&id) && expired.is_none() {
                        expired = Some(id.clone());
                    }
                    active.insert(id);
                }
            }
        }
        self.confirmed.retain(|id| active.contains(id));
        if let Some((key, reset)) = expired {
            self.confirmed.insert((key.clone(), reset));
            return Ok(Some(json!({"accountKey":key})));
        }
        let schedule = engine.quota_schedule()?;
        Ok(schedule["nextDueAt"]
            .as_i64()
            .filter(|t| *t <= now)
            .map(|_| json!({"dueOnly":true})))
    }
}
impl Engine {
    pub(crate) fn quota_schedule(&self) -> Result<Value> {
        let settings = self.store.settings()?;
        let now = crate::models::now();
        let mut accounts = vec![];
        for account in settings
            .accounts
            .iter()
            .filter(|a| a.quota_enabled && !a.archived)
        {
            if !settings
                .sources
                .iter()
                .any(|s| settings.source_enabled(s) && account.uses_source(s))
            {
                continue;
            }
            let key = crate::accounts::key(account);
            let last = quota_queries::finished(&self.store, &key)?;
            let seconds = interval(account, settings.refresh_seconds);
            let in_flight = quota_queries::try_lock(&quota_queries::account_lock(
                quota_queries::root(&self.store)?,
                &key,
            ))?
            .is_none();
            let next = if in_flight {
                now + seconds
            } else if due(last, now, seconds) {
                now
            } else {
                last.unwrap().saturating_add(seconds)
            };
            accounts.push(json!({"accountKey":key,"nextDueAt":next,"intervalSeconds":seconds,"inFlight":in_flight}));
        }
        let next = accounts
            .iter()
            .filter_map(|a| a["nextDueAt"].as_i64())
            .min();
        Ok(json!({"generatedAt":now,"nextDueAt":next,"accounts":accounts}))
    }
    fn refresh_one_quota(
        &mut self,
        params: &Value,
        observed: &std::collections::HashMap<String, Option<String>>,
    ) -> Result<Vec<QuotaSnapshot>> {
        let key = params["accountKey"].as_str().context("账户缺失")?;
        let automatic = params["dueOnly"].as_bool().unwrap_or(false);
        if automatic {
            let settings = self.store.settings()?;
            let Some(account) = settings
                .accounts
                .iter()
                .find(|a| crate::accounts::key(a) == key)
            else {
                return Ok(vec![]);
            };
            if !due(
                quota_queries::finished(&self.store, key)?,
                crate::models::now(),
                interval(account, settings.refresh_seconds),
            ) {
                return Ok(vec![]);
            }
        }
        let mut ids: Vec<_> = self
            .store
            .estimates()?
            .iter()
            .filter(|e| {
                e.status == "active" && e.account_key == key && e.sampling_mode != "automatic"
            })
            .flat_map(|e| e.source_ids.clone())
            .collect();
        ids.sort();
        ids.dedup();
        if !ids.is_empty() {
            if self.verify_sampling_identities(&ids, key).is_err() {
                for id in &ids {
                    self.store.pause_shared_source_estimates(id)?;
                }
                ids.clear();
            } else {
                let _ = self.sync_estimate_sources(&ids);
            }
        }
        let rows = self.coordinated_quotas(params, automatic, !ids.is_empty(), observed)?;
        if !ids.is_empty() && self.verify_sampling_identities(&ids, key).is_err() {
            for id in &ids {
                self.store.pause_shared_source_estimates(id)?;
            }
        }
        Ok(rows)
    }
    pub(crate) fn refresh_scheduled_quotas(&mut self, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let mut accounts: Vec<_> = settings
            .accounts
            .iter()
            .filter(|a| {
                a.quota_enabled
                    && !a.archived
                    && params["accountId"].as_str().is_none_or(|id| a.id == id)
                    && params["accountKey"]
                        .as_str()
                        .is_none_or(|key| crate::accounts::key(a) == key)
            })
            .cloned()
            .collect();
        let mut deadlines = std::collections::HashMap::new();
        let mut observed = std::collections::HashMap::new();
        for a in &accounts {
            let key = crate::accounts::key(a);
            deadlines.insert(
                key.clone(),
                quota_queries::finished(&self.store, &key)?.unwrap_or(0),
            );
            // Capture at request entry, before worker startup or a queue wait.
            observed.insert(key.clone(), quota_queries::completed(&self.store, &key)?);
        }
        accounts.sort_by_key(|a| deadlines[&crate::accounts::key(a)]);
        let count = accounts.len().min(4);
        let queue = Mutex::new(VecDeque::from(accounts));
        let root = quota_queries::root(&self.store)?.to_owned();
        let context = crate::operations::context();
        let (tx, rx) = mpsc::channel();
        let mut rows = vec![];
        std::thread::scope(|scope| {
            for _ in 0..count {
                let (queue, root, params, tx, context, observed) = (
                    &queue,
                    &root,
                    &params,
                    tx.clone(),
                    context.clone(),
                    &observed,
                );
                scope.spawn(move || {
                    let _operation = crate::operations::inherit(context);
                    let mut engine = Engine::open(root);
                    loop {
                        let Some(account) = queue.lock().unwrap().pop_front() else {
                            break;
                        };
                        let mut selected = params.clone();
                        selected["accountKey"] = json!(crate::accounts::key(&account));
                        let result = match &mut engine {
                            Ok(engine) => engine.refresh_one_quota(&selected, observed),
                            Err(error) => Err(anyhow::anyhow!(error.to_string())),
                        };
                        let result = result.unwrap_or_else(|error| {
                            vec![QuotaSnapshot {
                                account_id: account.id,
                                provider: account.provider,
                                name: account.name,
                                error: Some(error.to_string()),
                                ..Default::default()
                            }]
                        });
                        if tx.send(result).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            for finished in rx {
                for row in finished {
                    if let Some(progress) = &mut self.progress {
                        progress(json!({"kind":"quotas.refresh","row":row}));
                    }
                    rows.push(row);
                }
            }
        });
        rows.sort_by_key(|q| {
            settings
                .accounts
                .iter()
                .position(|a| a.id == q.account_id && a.provider == q.provider)
        });
        Ok(serde_json::to_value(rows)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_lock::FileLock;
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
                path: format!("/nonexistent/fixture/{id}"),
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
                .call(
                    "quotas.refresh",
                    json!({"accountKey":"custom:fast","dueOnly":true})
                )
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        drop(lock);
        let schedule = engine.call("quotas.schedule", json!({})).unwrap();
        assert_eq!(schedule["accounts"].as_array().unwrap().len(), 2);
        let rows = engine
            .call("quotas.refresh", json!({"dueOnly":true}))
            .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2);
        assert!(rows[0]["error"].is_string());
        let schedule = engine.call("quotas.schedule", json!({})).unwrap();
        let next = schedule["nextDueAt"].as_i64().unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!((now + 28..=now + 30).contains(&next));
        assert_eq!(schedule["accounts"][1]["intervalSeconds"], 600);
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
        assert!(engine.call("quotas.schedule", json!({})).unwrap()["nextDueAt"].is_null());
        settings.accounts[0].quota_refresh_seconds = Some(4);
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
