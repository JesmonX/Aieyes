//! Disposable daily projections. Raw events and their source memberships remain authoritative.
use crate::{models::*, pricing, store::Store};
use anyhow::Result;
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
    payload: String,
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
            self.bytes -= key.len() + old.payload.len() + old.version.len()
        }
        let size = key.len() + entry.payload.len() + entry.version.len();
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
            self.bytes -= old.len() + entry.payload.len() + entry.version.len();
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
        let tx = db.unchecked_transaction()?;
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

fn boundary(day: chrono::NaiveDate) -> i64 {
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
}

pub(crate) fn read(store: &Store, f: &Filter, summary: bool) -> Result<(Dashboard, Vec<Pending>)> {
    let today = Local::now().date_naive();
    let now = now();
    let start = today
        - Duration::days(if summary {
            0
        } else {
            f.days.unwrap_or(1).clamp(1, 3660) as i64 - 1
        });
    let trend_start = if summary {
        today
    } else {
        start.min(today - Duration::days(6))
    };
    let heat_start = if summary {
        today
    } else {
        today - Duration::days(364)
    };
    let earliest = start.min(heat_start);
    let last = boundary(today + Duration::days(1));
    let settings = store.settings()?;
    let mut shared: Vec<_> = settings
        .sources
        .iter()
        .filter(|s| s.codex_home_id.is_some())
        .map(|s| s.id.clone())
        .collect();
    shared.sort();
    let shared_json = serde_json::to_string(&shared)?;
    let scope = hash(&serde_json::to_string(&(
        "usage-v1",
        &f.provider,
        &f.account_id,
        &f.source_id,
        &shared,
    ))?);
    let revisions = versions(&store.db, boundary(earliest), last)?;
    let prices = store.prices()?;
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
    for i in 0..=(today - earliest).num_days() {
        let date = earliest + Duration::days(i);
        let day = date.to_string();
        let from = boundary(date);
        let end = boundary(date + Duration::days(1));
        if end <= from {
            continue;
        }
        let key = format!("{scope}:{from}:{end}");
        let rev = version(&revisions, from, end);
        let memory = store.usage_memory.borrow_mut().get(&key, &rev, now);
        let cached = if memory.is_some() {
            memory
        } else {
            store.db.query_row("SELECT payload,valid_until,accessed FROM usage_day_cache WHERE cache_key=?1 AND version=?2 AND valid_until>?3",params![key,rev,now],|r|Ok(Entry {payload:r.get(0)?,until:r.get(1)?,disk_touch:r.get(2)?,version:rev.clone(),touched:0})).optional()?
        };
        let (data, payload, until, disk_touch) = if let Some(entry) = cached
            .as_ref()
            .filter(|e| serde_json::from_str::<Day>(&e.payload).is_ok())
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
                serde_json::from_str::<Day>(&entry.payload)?,
                entry.payload.clone(),
                entry.until,
                if touch { now } else { entry.disk_touch },
            )
        } else {
            let mut data = Day::new();
            let mut query=store.db.prepare_cached("SELECT payload,cost,priced_tokens,price FROM events e WHERE stamp>=?1 AND stamp<?2 AND stamp<=?3 AND (?4 IS NULL OR provider=?4) AND (?5 IS NULL OR (?5='' AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7)))) OR (account_id=?5 AND NOT EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id IN (SELECT value FROM json_each(?7))))) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?6)) ORDER BY stamp")?;
            let mut rows = query.query(params![
                from,
                end,
                now,
                f.provider,
                f.account_id,
                f.source_id,
                shared_json
            ])?;
            while let Some(row) = rows.next()? {
                let e: UsageEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
                let cost: f64 = row.get(1)?;
                let covered: u64 = row.get(2)?;
                let entry = data.entry(e.model.clone()).or_default();
                let a = &mut entry.usage;
                a.key = e.model;
                a.tokens.add(&e.tokens);
                a.total = a.tokens.total();
                a.cost += cost;
                a.priced_tokens += covered;
                a.events += 1;
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
            (data, payload, until, now)
        };
        store.usage_memory.borrow_mut().put(
            key,
            Entry {
                version: rev,
                until,
                payload,
                touched: 0,
                disk_touch,
            },
        );
        for (model, entry) in data {
            if date >= trend_start {
                options.insert(model.clone());
            }
            if f.model.as_ref().is_some_and(|m| m != &model) {
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
                        price_id: pricing::find_price(&model, &prices, &settings.model_mappings)
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
        (0..=(today - from).num_days())
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
    d.pricing_gaps = gaps.into_values().collect();
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
                    payload: "x".repeat(1024 * 1024),
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
