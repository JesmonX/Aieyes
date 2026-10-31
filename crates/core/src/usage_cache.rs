//! Disposable daily projections. Raw events and their source memberships remain authoritative.
use crate::{models::*, pricing, store::Store};
use anyhow::{Context, Result, ensure};
use chrono::{Duration, Local, TimeZone};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
const DISK_LIMIT: i64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Memory {
    entries: HashMap<String, Entry>,
    bytes: usize,
    clock: u64,
}
#[derive(Clone)]
struct Entry {
    version: String,
    until: i64,
    payload: std::sync::Arc<str>,
    data: Option<std::sync::Arc<Day>>,
    touched: u64,
    disk_touch: i64,
}
impl Memory {
    fn get(&mut self, key: &str, version: &str, now: i64) -> Option<Entry> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        if entry.version != version || now >= entry.until {
            return None;
        }
        entry.touched = self.clock;
        Some(entry.clone())
    }
    fn put(&mut self, key: String, mut entry: Entry) {
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= key.len() + old.payload.len() * 4 + old.version.len()
        }
        let size = key.len() + entry.payload.len() * 4 + entry.version.len();
        if size > MEMORY_LIMIT {
            return;
        }
        while self.bytes + size > MEMORY_LIMIT {
            let Some(old) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.touched)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            let entry = self.entries.remove(&old).unwrap();
            self.bytes -= old.len() + entry.payload.len() * 4 + entry.version.len();
        }
        self.clock += 1;
        entry.touched = self.clock;
        self.bytes += size;
        self.entries.insert(key, entry);
    }
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS usage_day_revision(bucket INTEGER PRIMARY KEY,revision INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS usage_day_cache(cache_key TEXT PRIMARY KEY,start INTEGER NOT NULL,end INTEGER NOT NULL,version TEXT NOT NULL,valid_until INTEGER NOT NULL,payload TEXT NOT NULL,bytes INTEGER NOT NULL,accessed INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS usage_cache_access ON usage_day_cache(accessed);")?;
    // SQL triggers also cover older app builds and maintenance paths writing the same database.
    for (name, event, when, stamps) in [
        ("insert", "INSERT", "", vec!["NEW.stamp"]),
        ("delete", "DELETE", "", vec!["OLD.stamp"]),
        (
            "update",
            "UPDATE",
            "WHEN OLD.provider IS NOT NEW.provider OR OLD.account_id IS NOT NEW.account_id OR OLD.model IS NOT NEW.model OR OLD.stamp IS NOT NEW.stamp OR OLD.payload IS NOT NEW.payload OR OLD.cost IS NOT NEW.cost OR OLD.priced_tokens IS NOT NEW.priced_tokens OR OLD.price IS NOT NEW.price",
            vec!["OLD.stamp", "NEW.stamp"],
        ),
    ] {
        let body = stamps.iter().map(|stamp| format!("INSERT INTO usage_day_revision VALUES((({stamp}-CASE WHEN {stamp}<0 THEN 86399 ELSE 0 END)/86400),1) ON CONFLICT(bucket) DO UPDATE SET revision=revision+1;")).collect::<String>();
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS usage_events_{name} AFTER {event} ON events {when} BEGIN {body} END;"))?;
    }
    for (name, event, row) in [("insert", "INSERT", "NEW"), ("delete", "DELETE", "OLD")] {
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS usage_membership_{name} AFTER {event} ON event_sources BEGIN
          INSERT INTO usage_day_revision SELECT ((stamp-CASE WHEN stamp<0 THEN 86399 ELSE 0 END)/86400),1 FROM events WHERE id={row}.event_id
          ON CONFLICT(bucket) DO UPDATE SET revision=revision+1; END;"))?;
    }
    db.execute_batch("CREATE TRIGGER IF NOT EXISTS usage_membership_update AFTER UPDATE ON event_sources BEGIN
      INSERT INTO usage_day_revision SELECT ((stamp-CASE WHEN stamp<0 THEN 86399 ELSE 0 END)/86400),1 FROM events WHERE id IN (OLD.event_id,NEW.event_id)
      GROUP BY ((stamp-CASE WHEN stamp<0 THEN 86399 ELSE 0 END)/86400) ON CONFLICT(bucket) DO UPDATE SET revision=revision+1; END;")?;
    // Source selectors need historical account membership, not event payloads.
    // Maintain exact counts to avoid rescanning the entire history on every dashboard.
    db.execute_batch("CREATE TABLE IF NOT EXISTS usage_source_accounts(source_id TEXT NOT NULL,provider TEXT NOT NULL,account_id TEXT NOT NULL,event_count INTEGER NOT NULL,PRIMARY KEY(source_id,provider,account_id));")?;
    for (name, action, row, delta) in [
        ("insert", "INSERT", "NEW", 1),
        ("delete", "DELETE", "OLD", -1),
    ] {
        let update = if delta > 0 {
            format!(
                "INSERT INTO usage_source_accounts SELECT {row}.source_id,provider,account_id,1 FROM events WHERE id={row}.event_id ON CONFLICT(source_id,provider,account_id) DO UPDATE SET event_count=event_count+1;"
            )
        } else {
            format!(
                "UPDATE usage_source_accounts SET event_count=event_count-1 WHERE source_id={row}.source_id AND (provider,account_id) IN (SELECT provider,account_id FROM events WHERE id={row}.event_id); DELETE FROM usage_source_accounts WHERE event_count<=0;"
            )
        };
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS usage_source_membership_{name} AFTER {action} ON event_sources BEGIN {update} END;"))?;
    }
    db.execute_batch("CREATE TRIGGER IF NOT EXISTS usage_source_membership_update AFTER UPDATE OF event_id,source_id ON event_sources WHEN OLD.event_id IS NOT NEW.event_id OR OLD.source_id IS NOT NEW.source_id BEGIN
      UPDATE usage_source_accounts SET event_count=event_count-1 WHERE source_id=OLD.source_id AND (provider,account_id) IN (SELECT provider,account_id FROM events WHERE id=OLD.event_id);
      DELETE FROM usage_source_accounts WHERE event_count<=0;
      INSERT INTO usage_source_accounts SELECT NEW.source_id,provider,account_id,1 FROM events WHERE id=NEW.event_id ON CONFLICT(source_id,provider,account_id) DO UPDATE SET event_count=event_count+1; END;
      CREATE TRIGGER IF NOT EXISTS usage_source_event_update AFTER UPDATE OF provider,account_id ON events WHEN OLD.provider IS NOT NEW.provider OR OLD.account_id IS NOT NEW.account_id BEGIN
      UPDATE usage_source_accounts SET event_count=event_count-1 WHERE provider=OLD.provider AND account_id=OLD.account_id AND source_id IN (SELECT source_id FROM event_sources WHERE event_id=OLD.id);
      DELETE FROM usage_source_accounts WHERE event_count<=0;
      INSERT INTO usage_source_accounts SELECT source_id,NEW.provider,NEW.account_id,1 FROM event_sources WHERE event_id=NEW.id ON CONFLICT(source_id,provider,account_id) DO UPDATE SET event_count=event_count+1; END;
      CREATE TRIGGER IF NOT EXISTS usage_source_event_delete AFTER DELETE ON events BEGIN
      UPDATE usage_source_accounts SET event_count=event_count-1 WHERE provider=OLD.provider AND account_id=OLD.account_id AND source_id IN (SELECT source_id FROM event_sources WHERE event_id=OLD.id);
      DELETE FROM usage_source_accounts WHERE event_count<=0; END;
      CREATE TRIGGER IF NOT EXISTS usage_source_event_insert AFTER INSERT ON events BEGIN
      INSERT INTO usage_source_accounts SELECT source_id,NEW.provider,NEW.account_id,1 FROM event_sources WHERE event_id=NEW.id ON CONFLICT(source_id,provider,account_id) DO UPDATE SET event_count=event_count+1; END;")?;
    let initialized: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv WHERE key='usage-source-accounts-v1')",
        [],
        |r| r.get(0),
    )?;
    if !initialized {
        let tx =
            rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
        // A second process may have completed initialization while this one waited.
        db.execute("INSERT OR REPLACE INTO usage_source_accounts SELECT es.source_id,e.provider,e.account_id,COUNT(*) FROM event_sources es JOIN events e ON e.id=es.event_id WHERE NOT EXISTS(SELECT 1 FROM kv WHERE key='usage-source-accounts-v1') GROUP BY es.source_id,e.provider,e.account_id",[])?;
        db.execute(
            "INSERT OR IGNORE INTO kv VALUES('usage-source-accounts-v1','1')",
            [],
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
struct ModelDay {
    usage: Aggregate,
    missing: Tokens,
    unpriced: u64,
}
type Day = BTreeMap<String, ModelDay>;
pub(crate) struct Pending {
    key: String,
    start: i64,
    end: i64,
    version: String,
    until: i64,
    payload: Option<String>,
}

pub(crate) fn boundary(day: chrono::NaiveDate) -> i64 {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap();
    // Some time zones advance their clocks at midnight. Use the first valid
    // local instant, including the rare case of an entirely skipped date.
    for minutes in 0..=2880 {
        if let Some(instant) = Local
            .from_local_datetime(&(midnight + Duration::minutes(minutes)))
            .earliest()
        {
            return instant.timestamp();
        }
    }
    midnight.and_utc().timestamp()
}
fn versions(db: &Connection, start: i64, end: i64) -> Result<BTreeMap<i64, i64>> {
    let mut q = db.prepare_cached(
        "SELECT bucket,revision FROM usage_day_revision WHERE bucket BETWEEN ?1 AND ?2",
    )?;
    Ok(q.query_map(
        params![start.div_euclid(86400), (end - 1).div_euclid(86400)],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?
    .collect::<rusqlite::Result<_>>()?)
}
fn version(revisions: &BTreeMap<i64, i64>, start: i64, end: i64) -> String {
    revisions
        .range(start.div_euclid(86400)..=(end - 1).div_euclid(86400))
        .map(|(k, v)| format!("{k}:{v};"))
        .collect()
}
fn merge(to: &mut Aggregate, from: &Aggregate) {
    to.tokens.add(&from.tokens);
    to.total += from.total;
    to.cost += from.cost;
    to.priced_tokens += from.priced_tokens;
    to.events += from.events;
    for (provider, credits) in &from.native_credits {
        to.native_credits
            .entry(provider.clone())
            .or_default()
            .add(credits);
    }
}

pub(crate) struct DateBounds {
    pub start: chrono::NaiveDate,
    pub end: chrono::NaiveDate,
    pub trend_start: chrono::NaiveDate,
    pub heat_start: chrono::NaiveDate,
}
pub(crate) fn date_bounds(store: &Store, f: &Filter, summary: bool) -> Result<DateBounds> {
    let today = Local::now().date_naive();
    let parse = |raw: &str| -> Result<chrono::NaiveDate> {
        let day = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .context("日期格式必须为 YYYY-MM-DD")?;
        ensure!(
            raw.len() == 10 && day.to_string() == raw,
            "日期格式必须为 YYYY-MM-DD"
        );
        Ok(day)
    };
    let (start, end) = match &f.date_range {
        Some(DateRange::Custom { start, end }) => {
            let (start, end) = (parse(start)?, parse(end)?);
            ensure!(start <= end, "开始日期不能晚于结束日期");
            (start, end)
        }
        Some(DateRange::All) => {
            let settings = store.settings()?;
            let shared = serde_json::to_string(
                &settings
                    .sources
                    .iter()
                    .filter(|s| s.codex_home_id.is_some())
                    .map(|s| &s.id)
                    .collect::<Vec<_>>(),
            )?;
            let first: Option<i64> = store.db.query_row("SELECT MIN(stamp) FROM events e WHERE stamp<=?1 AND (?2 IS NULL OR (CASE WHEN provider='codex' AND json_extract(payload,'$.agent')='chatgpt-work' THEN 'chatgpt-work' ELSE provider END)=?2 OR provider=?7) AND (?3 IS NULL OR (?3='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6)))) OR (account_id=?3 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?6))))) AND (?4 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?4)) AND (?5 IS NULL OR model=?5)", params![now(), f.provider, f.account_id, f.source_id, f.model, shared, f.usage_provider_alias()], |r| r.get(0))?;
            (
                first
                    .and_then(|stamp| Local.timestamp_opt(stamp, 0).single())
                    .map(|time| time.date_naive())
                    .unwrap_or(today),
                today,
            )
        }
        None => (
            today
                - Duration::days(if summary {
                    0
                } else {
                    f.days.unwrap_or(1).clamp(1, 3660) as i64 - 1
                }),
            today,
        ),
    };
    let trend_start = if f.date_range.is_some() || summary {
        start
    } else {
        start.min(today - Duration::days(6))
    };
    let heat_start = if f.date_range.is_some() || summary {
        start
    } else {
        today - Duration::days(364)
    };
    Ok(DateBounds {
        start,
        end,
        trend_start,
        heat_start,
    })
}

pub(crate) fn read(store: &Store, f: &Filter, summary: bool) -> Result<(Dashboard, Vec<Pending>)> {
    let DateBounds {
        start,
        end: last_day,
        trend_start,
        heat_start,
    } = date_bounds(store, f, summary)?;
    let now = now();
    let earliest = start.min(heat_start);
    let last = boundary(last_day + Duration::days(1));
    let settings = store.settings()?;
    let cacheable = store.db.transaction_state(None::<&str>)? != rusqlite::TransactionState::Write;
    let mut shared: Vec<_> = settings
        .sources
        .iter()
        .filter(|s| s.codex_home_id.is_some())
        .map(|s| s.id.clone())
        .collect();
    shared.sort();
    let shared_json = serde_json::to_string(&shared)?;
    let scope = hash(&serde_json::to_string(&(
        "usage-v5-display-agents",
        &f.provider,
        &f.account_id,
        &f.source_id,
        &shared,
    ))?);
    let revisions = versions(&store.db, boundary(earliest), last)?;
    let prices = store.cached_prices()?;
    let mut d = Dashboard {
        generated_at: now,
        ..Default::default()
    };
    let mut days = BTreeMap::<String, Aggregate>::new();
    let mut heat = BTreeMap::<String, Aggregate>::new();
    let mut trend = BTreeMap::<String, Aggregate>::new();
    let mut models = BTreeMap::<String, Aggregate>::new();
    let mut gaps = BTreeMap::<String, PricingGap>::new();
    let mut options = std::collections::BTreeSet::new();
    let mut pending = vec![];
    for i in 0..=(last_day - earliest).num_days() {
        crate::operations::check()?;
        let date = earliest + Duration::days(i);
        let day = date.to_string();
        let from = boundary(date);
        let end = boundary(date + Duration::days(1));
        if end <= from {
            continue;
        }
        let key = format!("{scope}:{from}:{end}");
        let rev = version(&revisions, from, end);
        let memory = if cacheable {
            store.usage_memory.borrow_mut().get(&key, &rev, now)
        } else {
            None
        };
        let cached = if !cacheable {
            None
        } else if memory.is_some() {
            memory
        } else {
            store.db.query_row("SELECT payload,valid_until,accessed FROM usage_day_cache WHERE cache_key=?1 AND version=?2 AND valid_until>?3",params![key,rev,now],|r|Ok(Entry {payload:r.get::<_,String>(0)?.into(),data:None,until:r.get(1)?,disk_touch:r.get(2)?,version:rev.clone(),touched:0})).optional()?
        };
        let decoded = cached.as_ref().and_then(|e| {
            e.data.clone().or_else(|| {
                serde_json::from_str::<Day>(&e.payload)
                    .ok()
                    .map(std::sync::Arc::new)
            })
        });
        let (data, payload, until, disk_touch) = if let Some((entry, data)) =
            cached.as_ref().zip(decoded)
        {
            let touch = now.saturating_sub(entry.disk_touch) >= 60;
            if touch {
                pending.push(Pending {
                    key: key.clone(),
                    start: from,
                    end,
                    version: rev.clone(),
                    until: entry.until,
                    payload: None,
                });
            }
            (
                data,
                entry.payload.clone(),
                entry.until,
                if touch { now } else { entry.disk_touch },
            )
        } else {
            let mut data = Day::new();
            let mut query=store.db.prepare_cached("SELECT payload,cost,priced_tokens,price FROM events e WHERE stamp>=?1 AND stamp<?2 AND stamp<=?3 AND (?4 IS NULL OR (CASE WHEN provider='codex' AND json_extract(payload,'$.agent')='chatgpt-work' THEN 'chatgpt-work' ELSE provider END)=?4 OR provider=?8) AND (?5 IS NULL OR (?5='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7)))) OR (account_id=?5 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7))))) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?6)) ORDER BY stamp")?;
            let mut rows = query.query(params![
                from,
                end,
                now,
                f.provider,
                f.account_id,
                f.source_id,
                shared_json,
                f.usage_provider_alias()
            ])?;
            while let Some(row) = rows.next()? {
                let mut e: UsageEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
                if !crate::native_credits::prepare_view(store, &mut e, f.source_id.as_deref())? {
                    continue;
                }
                let cost: f64 = row.get(1)?;
                let covered: u64 = row.get(2)?;
                let entry = data.entry(e.model.clone()).or_default();
                let a = &mut entry.usage;
                a.add_native_credits(&e);
                a.key = e.model;
                a.tokens.add(&e.tokens);
                a.total = a.tokens.total();
                a.cost += cost;
                a.priced_tokens += covered;
                if e.attribution != "qoder-residual" {
                    a.events += 1;
                }
                if covered < e.tokens.total() {
                    let price: Option<ModelPrice> = row
                        .get::<_, Option<String>>(3)?
                        .map(|s| serde_json::from_str(&s))
                        .transpose()?;
                    entry
                        .missing
                        .add(&pricing::missing_tokens(&e.tokens, price.as_ref()));
                    entry.unpriced += e.tokens.total() - covered;
                }
            }
            let future: Option<i64> = store.db.query_row(
                "SELECT MIN(stamp) FROM events WHERE stamp>?1 AND stamp<?2",
                params![now, end],
                |r| r.get(0),
            )?;
            let until = future.unwrap_or(i64::MAX);
            let payload = serde_json::to_string(&data)?;
            pending.push(Pending {
                key: key.clone(),
                start: from,
                end,
                version: rev.clone(),
                until,
                payload: Some(payload.clone()),
            });
            (
                std::sync::Arc::new(data),
                std::sync::Arc::<str>::from(payload),
                until,
                now,
            )
        };
        if cacheable {
            store.usage_memory.borrow_mut().put(
                key,
                Entry {
                    version: rev,
                    until,
                    payload,
                    touched: 0,
                    disk_touch,
                    data: Some(data.clone()),
                },
            );
        }
        for (model, entry) in data.iter() {
            if date >= trend_start {
                options.insert(model.clone());
            }
            if f.model.as_ref().is_some_and(|m| m != model) {
                continue;
            }
            let a = &entry.usage;
            if date >= heat_start {
                merge(heat.entry(day.clone()).or_default(), a);
            }
            if date >= trend_start {
                merge(trend.entry(day.clone()).or_default(), a);
                d.day_models.push(DayModel {
                    day: day.clone(),
                    model: model.clone(),
                    usage: a.clone(),
                });
            }
            if date >= start {
                merge(&mut d.summary, a);
                merge(days.entry(day.clone()).or_default(), a);
                merge(models.entry(model.clone()).or_default(), a);
                if entry.unpriced > 0 {
                    let gap = gaps.entry(model.clone()).or_insert_with(|| PricingGap {
                        model: model.clone(),
                        price_id: pricing::find_price(model, &prices, &settings.model_mappings)
                            .map(|p| p.id.clone()),
                        ..Default::default()
                    });
                    gap.tokens.add(&entry.missing);
                    gap.unpriced_tokens += entry.unpriced;
                }
            }
        }
    }
    let sequence = |from: chrono::NaiveDate, mut map: BTreeMap<String, Aggregate>| {
        (0..=(last_day - from).num_days())
            .map(|i| {
                let key = (from + Duration::days(i)).to_string();
                let mut row = map.remove(&key).unwrap_or_default();
                row.key = key;
                row
            })
            .collect()
    };
    if !summary {
        d.days = sequence(start, days);
        d.heatmap = sequence(heat_start, heat);
        d.trend_days = sequence(trend_start, trend);
    }
    d.models = models
        .into_iter()
        .map(|(key, mut row)| {
            row.key = key;
            row
        })
        .collect();
    d.models.sort_by_key(|a| std::cmp::Reverse(a.total));
    d.model_options = options.into_iter().collect();
    if f.provider
        .as_deref()
        .is_none_or(crate::qoder_family::is_qoder)
    {
        let mut query = store.db.prepare_cached(
            "SELECT DISTINCT model FROM events WHERE provider IN ('qoder','qoder-cn') AND model IN ('dfmodel','qfmodel','Qoder')",
        )?;
        for row in query.query_map([], |r| r.get::<_, String>(0))? {
            let model = row?;
            if let Some(name) = crate::qoder_state::model_display_name(&model) {
                d.model_display_names.insert(model, name.into());
            }
        }
    }
    d.pricing_gaps = gaps.into_values().collect();
    if !cacheable {
        pending.clear();
    }
    Ok((d, pending))
}

pub(crate) fn persist(store: &Store, pending: Vec<Pending>) -> Result<()> {
    if pending.is_empty() || !store.db.is_autocommit() {
        return Ok(());
    }
    // Never wait behind a writer to persist disposable projections. Restore the
    // connection's normal timeout even if opening or releasing the savepoint fails.
    let timeout: u64 = store
        .db
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
    store.db.busy_timeout(std::time::Duration::ZERO)?;
    let result = (|| -> Result<()> {
        store.db.execute_batch("SAVEPOINT usage_cache_write")?;
        let result = (|| -> Result<()> {
            let mut wrote = false;
            for p in pending {
                let Some(payload) = p.payload else {
                    store.db.execute(
                        "UPDATE usage_day_cache SET accessed=?1 WHERE cache_key=?2 AND accessed<?3",
                        params![now(), p.key, now() - 60],
                    )?;
                    continue;
                };
                if payload.len() > DISK_LIMIT as usize {
                    continue;
                }
                let current = version(&versions(&store.db, p.start, p.end)?, p.start, p.end);
                if current != p.version {
                    continue;
                }
                store.db.execute("INSERT INTO usage_day_cache VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(cache_key) DO UPDATE SET version=excluded.version,valid_until=excluded.valid_until,payload=excluded.payload,bytes=excluded.bytes,accessed=excluded.accessed",params![p.key,p.start,p.end,p.version,p.until,payload,payload.len() as i64,now()])?;
                wrote = true;
            }
            if wrote {
                loop {
                    let bytes: i64 = store.db.query_row(
                        "SELECT COALESCE(SUM(bytes),0) FROM usage_day_cache",
                        [],
                        |r| r.get(0),
                    )?;
                    if bytes <= DISK_LIMIT {
                        break;
                    }
                    store.db.execute("DELETE FROM usage_day_cache WHERE cache_key IN (SELECT cache_key FROM usage_day_cache ORDER BY accessed LIMIT 64)",[])?;
                }
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = store.db.execute_batch("ROLLBACK TO usage_cache_write");
        }
        store.db.execute_batch("RELEASE usage_cache_write")?;
        result
    })();
    store
        .db
        .busy_timeout(std::time::Duration::from_millis(timeout))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage;
    use serde_json::{Value, json};
    fn fixture() -> (tempfile::TempDir, Store, Settings) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = |id: &str| Source {
            id: id.into(),
            provider: "custom".into(),
            account_id: "account".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source("a"), source("b")],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        (root, store, settings)
    }
    fn event(source: &Source, id: &str, stamp: i64) -> UsageEvent {
        usage::parse(source,&mut ParseState::default(),&json!({"id":id,"sessionId":"session","model":"fixture","timestamp":stamp,"tokens":{"input":100,"output":20}})).unwrap()
    }
    #[test]
    fn typed_days_survive_rollback_and_a_reused_revision() {
        let (_root, store, settings) = fixture();
        store
            .put_event(
                &event(&settings.sources[0], "rollback", now()),
                &[],
                &settings,
            )
            .unwrap();
        let filter = Filter {
            days: Some(1),
            ..Default::default()
        };
        assert_eq!(store.dashboard(&filter).unwrap().summary.total, 120);
        store
            .db
            .execute_batch(
                "BEGIN; UPDATE events SET payload=json_set(payload,'$.tokens.input',500);",
            )
            .unwrap();
        assert_eq!(store.dashboard(&filter).unwrap().summary.total, 520);
        store.db.execute_batch("ROLLBACK;").unwrap();
        assert_eq!(store.dashboard(&filter).unwrap().summary.total, 120);
        store
            .db
            .execute(
                "UPDATE events SET payload=json_set(payload,'$.tokens.input',600)",
                [],
            )
            .unwrap();
        assert_eq!(store.dashboard(&filter).unwrap().summary.total, 620);
    }
    fn projection(d: Dashboard) -> Value {
        json!({"summary":d.summary,"days":d.days,"trend":d.trend_days,"heat":d.heatmap,"models":d.models,"options":d.model_options,"dayModels":d.day_models,"gaps":d.pricing_gaps})
    }
    fn parity(store: &Store, filter: &Filter) {
        let (cached, pending) = read(store, filter, false).unwrap();
        assert_eq!(
            projection(cached),
            projection(store.dashboard_usage_reference(filter).unwrap())
        );
        persist(store, pending).unwrap();
        assert!(
            read(store, filter, false).unwrap().1.is_empty(),
            "second read must use cache, not fallback"
        );
    }
    #[test]
    fn codex_and_work_filters_partition_usage_in_cached_and_reference_queries() {
        let (_root, store, mut settings) = fixture();
        let mut rows = Vec::new();
        for (id, agent) in [("work", "chatgpt-work"), ("codex", "codex"), ("legacy", "")] {
            let mut row = event(&settings.sources[0], id, now() - 10);
            row.provider = "codex".into();
            row.agent = agent.into();
            rows.push(row);
        }
        for source in &mut settings.sources {
            source.provider = "codex".into();
        }
        store.save_settings(&settings).unwrap();
        for row in rows {
            store.put_event(&row, &[], &settings).unwrap();
        }
        for days in [None, Some(1)] {
            for (provider, expected) in [
                (None, 360),
                (Some("codex"), 240),
                (Some("chatgpt-work"), 120),
            ] {
                let filter = Filter {
                    provider: provider.map(str::to_owned),
                    days,
                    ..Default::default()
                };
                parity(&store, &filter);
                assert_eq!(
                    store.dashboard(&filter).unwrap().summary.tokens.total(),
                    expected
                );
            }
        }
        let work = Filter {
            provider: Some("chatgpt-work".into()),
            ..Default::default()
        };
        assert_eq!(
            work.provider_alias(),
            Some("codex"),
            "billing keeps the shared pool"
        );
        assert_eq!(
            work.usage_provider_alias(),
            None,
            "usage has its own display identity"
        );
    }
    #[test]
    fn native_credits_preserve_units_zero_tokens_filters_and_cache_parity() {
        let (_root, store, mut settings) = fixture();
        for source in &mut settings.sources {
            source.provider = "qoder".into();
        }
        store.save_settings(&settings).unwrap();
        for (id, credits, billable) in [
            ("one", 0.25, Some(true)),
            ("two", 0.125, Some(false)),
            ("three", 0.0625, None),
        ] {
            for source in &settings.sources {
                let event = usage::parse(source, &mut ParseState::default(), &json!({"type":"assistant","sessionId":"session","timestamp":now()-10,"message":{"id":id,"model":"qfmodel","usage":{"input_tokens":0,"output_tokens":0,"credits":credits,"original_credits":if id=="one" {Some(0.5)} else {None},"billable":billable}}})).unwrap();
                store.put_event(&event, &[], &settings).unwrap();
                let mut partial = event.clone();
                let metadata = partial.native_credits.as_mut().unwrap();
                metadata.original_credits = None;
                metadata.billable = None;
                store.put_event(&partial, &[], &settings).unwrap();
                let mut copy = event;
                copy.native_credits = None;
                store.put_event(&copy, &[], &settings).unwrap();
            }
        }
        for source in [None, Some("a"), Some("b")] {
            let filter = Filter {
                provider: Some("qoder".into()),
                source_id: source.map(String::from),
                ..Default::default()
            };
            parity(&store, &filter);
            let d = store.dashboard(&filter).unwrap();
            assert_eq!(d.summary.events, 3);
            assert_eq!(d.summary.total, 0);
            assert_eq!(d.summary.cost, 0.0);
            assert!(d.pricing_gaps.is_empty());
            let row = &d.summary.native_credits["qoder"];
            assert_eq!(
                (
                    row.credits,
                    row.billable,
                    row.non_billable,
                    row.unknown_billability,
                    row.requests
                ),
                (0.4375, 0.25, 0.125, 0.0625, 3)
            );
            assert_eq!(d.models[0].native_credits["qoder"].credits, row.credits);
            assert_eq!(
                (
                    row.billable_requests,
                    row.non_billable_requests,
                    row.unknown_billability_requests
                ),
                (1, 1, 1)
            );
            assert_eq!(
                (row.original_credits_requests, row.original_credits),
                (1, 0.5)
            );
            assert_eq!(
                d.day_models.last().unwrap().usage.native_credits["qoder"].credits,
                row.credits
            );
        }
        let other = store
            .dashboard(&Filter {
                provider: Some("claude".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(other.summary.native_credits.is_empty());
    }

    #[test]
    fn explicit_ranges_include_both_endpoints_leap_days_and_follow_filters() {
        let (_root, store, settings) = fixture();
        let stamp = |year, month, day| {
            boundary(chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()) + 3600
        };
        for (source, id, year, month, day, model) in [
            (0, "old", 2022, 12, 31, "older"),
            (0, "first", 2024, 2, 28, "fixture"),
            (0, "leap", 2024, 2, 29, "fixture"),
            (0, "last", 2024, 3, 1, "fixture"),
            (0, "outside", 2024, 3, 2, "fixture"),
            (1, "other-source", 2020, 1, 1, "fixture"),
        ] {
            let mut row = event(&settings.sources[source], id, stamp(year, month, day));
            row.model = model.into();
            store.put_event(&row, &[], &settings).unwrap();
        }
        let filter = Filter {
            source_id: Some("a".into()),
            days: Some(1),
            date_range: Some(DateRange::Custom {
                start: "2024-02-28".into(),
                end: "2024-03-01".into(),
            }),
            ..Default::default()
        };
        parity(&store, &filter);
        let dashboard = store.dashboard(&filter).unwrap();
        assert_eq!(dashboard.summary.total, 360);
        assert_eq!(dashboard.heatmap.len(), 3);
        assert_eq!(dashboard.trend_days.len(), 3);
        assert_eq!(dashboard.heatmap[1].key, "2024-02-29");
        let all = Filter {
            date_range: Some(DateRange::All),
            ..filter.clone()
        };
        parity(&store, &all);
        let dashboard = store.dashboard(&all).unwrap();
        assert_eq!(dashboard.heatmap.first().unwrap().key, "2022-12-31");
        assert_eq!(dashboard.summary.total, 600);
        let model = Filter {
            model: Some("fixture".into()),
            ..all
        };
        parity(&store, &model);
        assert_eq!(
            store
                .dashboard(&model)
                .unwrap()
                .heatmap
                .first()
                .unwrap()
                .key,
            "2024-02-28"
        );
        let no_match = Filter {
            provider: Some("claude".into()),
            ..model
        };
        assert_eq!(store.dashboard(&no_match).unwrap().heatmap.len(), 1);
    }
    #[test]
    fn custom_dates_reject_invalid_or_reversed_bounds_and_keep_year_boundaries() {
        let (_root, store, _) = fixture();
        for (start, end) in [
            ("2023-02-29", "2023-03-01"),
            ("2024-2-28", "2024-03-01"),
            ("2024-03-01", "2024-02-28"),
            ("no-date", "2024-01-01"),
        ] {
            assert!(
                store
                    .dashboard(&Filter {
                        date_range: Some(DateRange::Custom {
                            start: start.into(),
                            end: end.into()
                        }),
                        ..Default::default()
                    })
                    .is_err()
            );
        }
        let filter = Filter {
            date_range: Some(DateRange::Custom {
                start: "2023-12-31".into(),
                end: "2024-01-01".into(),
            }),
            ..Default::default()
        };
        parity(&store, &filter);
        assert_eq!(
            store
                .dashboard(&filter)
                .unwrap()
                .heatmap
                .iter()
                .map(|d| d.key.as_str())
                .collect::<Vec<_>>(),
            ["2023-12-31", "2024-01-01"]
        );
    }
    #[test]
    fn cache_hits_match_raw_filters_and_invalidate_across_connections() {
        let (root, store, settings) = fixture();
        for (id, stamp) in [("today", now()), ("older", now() - 40 * 86400)] {
            let e = event(&settings.sources[0], id, stamp);
            store.put_event(&e, &[], &settings).unwrap();
            let mut copy = e.clone();
            copy.source_id = "b".into();
            store.put_event(&copy, &[], &settings).unwrap();
        }
        for days in [1, 7, 30, 365, 400] {
            for account in [None, Some("account"), Some("")] {
                for source in [None, Some("a"), Some("b")] {
                    parity(
                        &store,
                        &Filter {
                            days: Some(days),
                            account_id: account.map(str::to_owned),
                            source_id: source.map(str::to_owned),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        for model in ["fixture", "missing"] {
            parity(
                &store,
                &Filter {
                    model: Some(model.into()),
                    ..Default::default()
                },
            );
        }
        let count: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM usage_day_cache", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 365);
        let writer = Store::open(root.path()).unwrap();
        let e = event(&settings.sources[0], "new", now());
        writer.put_event(&e, &[], &settings).unwrap();
        parity(&store, &Filter::default());
        assert_eq!(
            read(&store, &Filter::default(), true)
                .unwrap()
                .0
                .summary
                .total,
            240
        );
        writer
            .db
            .execute("DELETE FROM event_sources WHERE source_id='b'", [])
            .unwrap();
        parity(
            &store,
            &Filter {
                source_id: Some("b".into()),
                ..Default::default()
            },
        );
        writer
            .db
            .execute("DELETE FROM event_sources WHERE event_id=?1", [&e.id])
            .unwrap();
        writer
            .db
            .execute("DELETE FROM events WHERE id=?1", [e.id])
            .unwrap();
        parity(&store, &Filter::default());
        *store.usage_memory.borrow_mut() = Memory::default();
        let disk = read(&store, &Filter::default(), false).unwrap();
        assert!(disk.1.is_empty());
        assert_eq!(
            projection(disk.0),
            projection(store.dashboard_usage_reference(&Filter::default()).unwrap())
        );
    }
    #[test]
    fn frozen_prices_future_records_and_busy_writers_remain_correct() {
        let (root, store, settings) = fixture();
        let today = event(&settings.sources[0], "today", now());
        let price = ModelPrice {
            id: "fixture".into(),
            input: Some(0.01),
            output: Some(0.02),
            ..Default::default()
        };
        store.save_prices(std::slice::from_ref(&price)).unwrap();
        store
            .put_event(&today, std::slice::from_ref(&price), &settings)
            .unwrap();
        parity(&store, &Filter::default());
        store
            .save_prices(&[ModelPrice {
                input: Some(0.02),
                ..price
            }])
            .unwrap();
        parity(&store, &Filter::default());
        assert_eq!(
            read(&store, &Filter::default(), true)
                .unwrap()
                .0
                .summary
                .cost,
            1.4
        );
        store.reprice(&settings, false).unwrap();
        parity(&store, &Filter::default());
        assert_eq!(
            read(&store, &Filter::default(), true)
                .unwrap()
                .0
                .summary
                .cost,
            2.4
        );
        // Put a future event inside today's bucket, then expire its cache without sleeping.
        let end = boundary(Local::now().date_naive() + Duration::days(1));
        let future = event(&settings.sources[0], "future", end - 1);
        store.put_event(&future, &[], &settings).unwrap();
        let (_, pending) = read(&store, &Filter::default(), true).unwrap();
        assert!(pending.iter().any(|p| p.until == end - 1));
        persist(&store, pending).unwrap();
        for entry in store.usage_memory.borrow_mut().entries.values_mut() {
            entry.until = 0;
        }
        store
            .db
            .execute("UPDATE usage_day_cache SET valid_until=0", [])
            .unwrap();
        assert!(!read(&store, &Filter::default(), true).unwrap().1.is_empty());
        let writer = Store::open(root.path()).unwrap();
        writer.db.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (_, pending) = read(&store, &Filter::default(), false).unwrap();
        let started = std::time::Instant::now();
        assert!(persist(&store, pending).is_err());
        assert!(started.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(
            store
                .db
                .query_row("PRAGMA busy_timeout", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            5000
        );
        writer.db.execute_batch("ROLLBACK").unwrap();
    }
    #[test]
    fn memory_and_disk_eviction_are_bounded() {
        let (_, store, _) = fixture();
        let mut memory = Memory::default();
        for i in 0..20 {
            memory.put(
                i.to_string(),
                Entry {
                    version: "v".into(),
                    until: i64::MAX,
                    payload: "x".repeat(1024 * 1024).into(),
                    data: None,
                    touched: 0,
                    disk_touch: now(),
                },
            );
        }
        assert!(memory.bytes <= MEMORY_LIMIT);
        assert!(!memory.entries.contains_key("0"));
        assert!(memory.entries.contains_key("19"));
        let pending = (0..70)
            .map(|i| Pending {
                key: i.to_string(),
                start: 0,
                end: 1,
                version: "".into(),
                until: i64::MAX,
                payload: Some("x".repeat(1024 * 1024)),
            })
            .collect();
        persist(&store, pending).unwrap();
        assert!(
            store
                .db
                .query_row("SELECT SUM(bytes) FROM usage_day_cache", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
                <= DISK_LIMIT
        );
    }
    #[test]
    fn source_account_counts_cover_membership_updates_deletion_and_existing_database_migration() {
        let (root, store, settings) = fixture();
        let first = event(&settings.sources[0], "one", now());
        store.put_event(&first, &[], &settings).unwrap();
        let mut copied = first.clone();
        copied.source_id = "b".into();
        store.put_event(&copied, &[], &settings).unwrap();
        let exact = |store: &Store| {
            let collect = |sql: &str| {
                store
                    .db
                    .prepare(sql)
                    .unwrap()
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            };
            assert_eq!(
                collect(
                    "SELECT source_id,provider,account_id,event_count FROM usage_source_accounts ORDER BY 1,2,3"
                ),
                collect(
                    "SELECT es.source_id,e.provider,e.account_id,COUNT(*) FROM event_sources es JOIN events e ON es.event_id=e.id GROUP BY 1,2,3 ORDER BY 1,2,3"
                )
            );
        };
        exact(&store);
        store
            .db
            .execute("UPDATE events SET account_id='historical'", [])
            .unwrap();
        exact(&store);
        store
            .db
            .execute(
                "UPDATE event_sources SET source_id='c' WHERE source_id='b'",
                [],
            )
            .unwrap();
        exact(&store);
        store
            .db
            .execute("DELETE FROM event_sources WHERE source_id='c'", [])
            .unwrap();
        exact(&store);
        store
            .db
            .execute("DELETE FROM usage_source_accounts", [])
            .unwrap();
        store
            .db
            .execute("DELETE FROM kv WHERE key='usage-source-accounts-v1'", [])
            .unwrap();
        let reopened = Store::open(root.path()).unwrap();
        exact(&reopened);
        reopened
            .db
            .execute("DELETE FROM event_sources", [])
            .unwrap();
        exact(&reopened);
    }

    #[test]
    #[ignore = "manual release benchmark against a disposable synthetic database"]
    fn benchmark_reference_and_cache() {
        let root =
            std::env::var("AIEYES_PERFORMANCE_DATA_DIR").expect("set the synthetic data directory");
        let store = Store::open(std::path::Path::new(&root)).unwrap();
        let count: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 100000, "benchmark must contain actual events");
        for days in [1, 30, 365] {
            let filter = Filter {
                days: Some(days),
                ..Default::default()
            };
            let started = std::time::Instant::now();
            let reference = store.dashboard_usage_reference(&filter).unwrap();
            let raw_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            let cached = read(&store, &filter, false).unwrap().0;
            let cached_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(projection(reference), projection(cached));
            println!(
                "BENCH {}",
                json!({"events":count,"days":days,"rawMs":raw_ms,"cachedMs":cached_ms})
            );
        }
    }
    #[test]
    fn local_day_boundaries_match_event_dates_even_when_midnight_does_not_exist() {
        let (_, store, settings) = fixture();
        let date = chrono::NaiveDate::from_ymd_opt(2018, 11, 4).unwrap();
        let start = boundary(date);
        for (id, stamp) in [("before", start - 1800), ("after", start + 1800)] {
            store
                .put_event(&event(&settings.sources[0], id, stamp), &[], &settings)
                .unwrap();
        }
        parity(
            &store,
            &Filter {
                days: Some(3660),
                ..Default::default()
            },
        );
        assert_eq!(Local.timestamp_opt(start, 0).unwrap().date_naive(), date);
    }
}
