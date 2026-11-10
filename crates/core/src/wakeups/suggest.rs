//! Wakeup time suggestion: start the 5 h window three hours before the usual start of work.
//!
//! Usage of the selected source over the last 30 complete natural days of the target machine is
//! grouped by its local date; each day's first use is the work start. Only sessions explicitly
//! identified by the Aieyes runner are excluded. Temporal overlap never proves ownership. Quota and identity checks call no model and leave no usage.
//! Without the target's time zone rules, or without the run records, no time is suggested.
use super::Store;
use super::identity::Evidence;
use crate::models::{Host, Settings, Source};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

const DAYS: i64 = 30;
const MINIMUM_DAYS: usize = 5;
const LEAD_MINUTES: i64 = 180;
/// Target zone: an IANA name with its history of rules, or a zone proven to have a fixed offset.
pub(crate) fn zone(name: Option<&str>, fixed: Option<i64>) -> Option<jiff::tz::TimeZone> {
    if let Some(zone) = name
        .filter(|n| !n.is_empty())
        .and_then(|n| jiff::tz::TimeZone::get(n).ok())
    {
        return Some(zone);
    }
    let offset = jiff::tz::Offset::from_seconds(i32::try_from(fixed?).ok()?).ok()?;
    Some(jiff::tz::TimeZone::fixed(offset))
}

/// Median of first-use minutes (two middle values averaged), three hours earlier, rounded to the
/// nearest quarter hour with exact halves rounded later, wrapped across midnight.
pub(crate) fn suggestion(mut firsts: Vec<f64>) -> Option<(i64, i64, i64)> {
    if firsts.len() < MINIMUM_DAYS {
        return None;
    }
    firsts.sort_by(f64::total_cmp);
    let middle = firsts.len() / 2;
    let median = if firsts.len() % 2 == 1 {
        firsts[middle]
    } else {
        (firsts[middle - 1] + firsts[middle]) / 2.0
    };
    let typical = median.round() as i64;
    let suggested =
        ((((median - LEAD_MINUTES as f64) + 7.5) / 15.0).floor() as i64 * 15).rem_euclid(1440);
    Some((
        typical.rem_euclid(1440),
        suggested,
        (suggested + 300).rem_euclid(1440),
    ))
}

pub(crate) fn clock(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// First use per local date, excluding only source-scoped native automatic-session identities.
pub(crate) fn first_uses(
    events: &[(i64, String)],
    excluded: &HashSet<String>,
    zone: &jiff::tz::TimeZone,
) -> (BTreeMap<String, f64>, usize) {
    let mut days = BTreeMap::new();
    for (stamp, session) in events {
        if excluded.contains(session.as_str()) {
            continue;
        }
        let Ok(time) = jiff::Timestamp::from_second(*stamp) else {
            continue;
        };
        let local = time.to_zoned(zone.clone());
        let minutes = f64::from(local.hour()) * 60.0
            + f64::from(local.minute())
            + f64::from(local.second()) / 60.0;
        days.entry(local.date().to_string())
            .and_modify(|m: &mut f64| *m = m.min(minutes))
            .or_insert(minutes);
    }
    let count = events
        .iter()
        .map(|(_, id)| id)
        .filter(|id| excluded.contains(*id))
        .collect::<HashSet<_>>()
        .len();
    (days, count)
}

impl crate::Engine {
    pub(crate) fn wakeup_suggest(&self, settings: &Settings, p: &Value) -> Result<Value> {
        let source_id = p["sourceId"].as_str().context("请选择运行位置")?;
        // The profile proves the account binding; usage stays per source, as on the device.
        let source = super::source_for_draft(settings, source_id, p["codexProfileId"].as_str())?;
        let host = source
            .host_id
            .as_ref()
            .map(|id| {
                settings
                    .hosts
                    .iter()
                    .find(|h| &h.id == id)
                    .cloned()
                    .context("服务器已移除")
            })
            .transpose()?;
        let now = jiff::Timestamp::now();
        let local_root = super::root(self)?;
        let namespace = &crate::models::hash(&local_root.to_string_lossy())[..20];
        let since = now.as_second() - (DAYS + 2) * 86400;
        let (zone, records) = match &host {
            None => (
                Some(jiff::tz::TimeZone::system()),
                local_runs(&local_root, since)?,
            ),
            Some(host) => match remote_runs(host, namespace, since) {
                Ok((zone, runs)) => (zone, runs),
                Err(_) => return Ok(json!({"insufficient":true,"reason":"wakeupRuns"})),
            },
        };
        let Some(zone) = zone else {
            return Ok(json!({"insufficient":true,"reason":"timezone"}));
        };
        let today = now.to_zoned(zone.clone()).date();
        let first = today.checked_sub(jiff::Span::new().days(DAYS))?;
        let start = first.to_zoned(zone.clone())?.timestamp().as_second();
        let end = today.to_zoned(zone.clone())?.timestamp().as_second();
        let events = self.store.source_usage_sessions(&source, start, end)?;
        let Some(automatic) = super::identity::excluded(&source, &records) else {
            return Ok(json!({"insufficient":true,"reason":"wakeupIdentity"}));
        };
        let (days, excluded) = first_uses(&events, &automatic, &zone);
        let name = zone
            .iana_name()
            .map(str::to_owned)
            .unwrap_or_else(|| now.to_zoned(zone.clone()).offset().to_string());
        let Some((typical, suggested, reset)) = suggestion(days.values().copied().collect()) else {
            return Ok(
                json!({"insufficient":true,"reason":"days","days":days.len(),"timezone":name}),
            );
        };
        Ok(
            json!({"typicalStart":clock(typical),"suggested":clock(suggested),"resetAt":clock(reset),"days":days.len(),"excludedSessions":excluded,"timezone":name,"from":first.to_string(),"to":today.yesterday()?.to_string()}),
        )
    }
}

fn local_runs(root: &std::path::Path, since: i64) -> Result<Vec<Evidence>> {
    let path = root.join("runner.sqlite");
    if !path.exists() {
        return Ok(vec![]);
    }
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let exists = |table: &str| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
            [table],
            |r| r.get::<_, bool>(0),
        )
    };
    if !exists("runs")? {
        return Ok(vec![]);
    }
    let has_identity = exists("run_identity")?;
    let mut stmt =
        db.prepare("SELECT id,task FROM runs WHERE started>=?1 AND status!='skipped-overlap'")?;
    let mut records = Vec::new();
    for row in stmt.query_map([since], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })? {
        let (id, task) = row?;
        let evidence = if has_identity {
            use rusqlite::OptionalExtension;
            db.query_row(
                "SELECT payload FROM run_identity WHERE run_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|raw| serde_json::from_str(&raw).ok())
        } else {
            None
        };
        let fallback = || {
            if !task.starts_with("wake-")
                || !task.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            {
                return None;
            }
            let bytes = std::fs::read(root.join("tasks").join(format!("{task}.json"))).ok()?;
            let manifest = serde_json::from_slice::<super::Manifest>(&bytes).ok()?;
            Some(Evidence::manifest(&manifest))
        };
        records.push(evidence.or_else(fallback).unwrap_or_default());
    }
    Ok(records)
}

fn remote_runs(
    host: &Host,
    namespace: &str,
    since: i64,
) -> Result<(Option<jiff::tz::TimeZone>, Vec<Evidence>)> {
    let value = super::remote(
        host,
        json!({"action":"runs","root":format!("~/.local/share/aieyes-wakeups/{namespace}"),"since":since}),
    )?;
    let runs = value["runs"].as_array().context("服务器未返回唤醒记录")?;
    let records: Vec<Evidence> = match value["identityRecords"].as_array() {
        Some(rows) if rows.len() == runs.len() => rows
            .iter()
            .map(|r| serde_json::from_value(r.clone()).unwrap_or_default())
            .collect(),
        _ => runs.iter().map(|_| Evidence::default()).collect(),
    };
    Ok((
        zone(value["zone"].as_str(), value["fixedOffset"].as_i64()),
        records,
    ))
}

impl Store {
    /// (stamp, session id) of the source's usage in [start, end).
    pub(crate) fn source_usage_sessions(
        &self,
        source: &Source,
        start: i64,
        end: i64,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT e.stamp,COALESCE(json_extract(e.payload,'$.sessionId'),'') FROM events e WHERE e.stamp>=?2 AND e.stamp<?3 AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?1)",
        )?;
        Ok(stmt
            .query_map(rusqlite::params![source.id, start, end], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(zone: &jiff::tz::TimeZone, date: &str, time: &str) -> i64 {
        let civil: jiff::civil::DateTime = format!("{date}T{time}").parse().unwrap();
        civil
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp()
            .as_second()
    }
    #[test]
    fn median_rounding_and_midnight() {
        let minutes = |v: &[i64]| v.iter().map(|m| *m as f64).collect::<Vec<_>>();
        // 09:10 median → 06:10 → nearest quarter 06:15; reset 11:15.
        assert_eq!(
            suggestion(minutes(&[540, 545, 550, 555, 560])),
            Some((550, 375, 675))
        );
        // Even count: 09:00 and 09:15 → 09:07.5 → 06:07.5, an exact half, rounds later to 06:15.
        assert_eq!(
            suggestion(minutes(&[530, 540, 555, 600, 500, 700])).map(|r| r.1),
            Some(375)
        );
        // 01:30 → 22:30 the previous evening.
        assert_eq!(
            suggestion(minutes(&[80, 90, 90, 95, 100])),
            Some((90, 1350, 210))
        );
        assert_eq!(
            suggestion(minutes(&[540, 545, 550, 555])),
            None,
            "Fewer than five active days"
        );
        assert_eq!(clock(1350), "22:30");
    }
    #[test]
    fn days_follow_the_target_zone_rules_and_runs_are_excluded() {
        let zone = jiff::tz::TimeZone::get("America/New_York").unwrap();
        // 23:30 local on Mar 7 is already Mar 8 in UTC; DST starts Mar 8, 2026.
        let events = vec![
            (at(&zone, "2026-03-07", "23:30:00"), "late".into()),
            (at(&zone, "2026-03-08", "09:00:00"), "a".into()),
            (at(&zone, "2026-03-09", "09:00:00"), "b".into()),
            (at(&zone, "2026-03-09", "06:00:20"), "wakeup".into()),
            (at(&zone, "2026-03-09", "06:30:00"), "wakeup".into()),
        ];
        let automatic = HashSet::from(["wakeup".to_string()]);
        let (days, excluded) = first_uses(&events, &automatic, &zone);
        assert_eq!(
            excluded, 1,
            "The whole session that started during a run is removed"
        );
        assert_eq!(
            days.get("2026-03-07"),
            Some(&(23.0 * 60.0 + 30.0)),
            "Dates are target-local"
        );
        assert_eq!(
            days.get("2026-03-08"),
            Some(&540.0),
            "09:00 after the DST change stays 09:00"
        );
        assert_eq!(
            days.get("2026-03-09"),
            Some(&540.0),
            "The wakeup run does not count as the work start"
        );
        let (unfiltered, _) = first_uses(&events, &HashSet::new(), &zone);
        assert_eq!(
            unfiltered.get("2026-03-09"),
            Some(&(6.0 * 60.0 + 20.0 / 60.0))
        );
    }
    #[test]
    fn zones_need_rules_or_a_proven_fixed_offset() {
        assert!(zone(Some("Asia/Shanghai"), None).is_some());
        assert!(
            zone(Some("Not/AZone"), None).is_none(),
            "An unknown name is not guessed"
        );
        assert!(zone(None, Some(28_800)).is_some());
        assert!(zone(None, None).is_none());
    }
}

#[cfg(test)]
mod engine_tests {
    use crate::models::*;
    use serde_json::json;
    #[test]
    fn local_suggestions_use_complete_days_exclude_runs_and_never_save_a_task() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = crate::Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            provider: "claude".into(),
            account_id: "a".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        let other = Source {
            id: "other".into(),
            account_id: "a".into(),
            ..source.clone()
        };
        let settings = Settings {
            sources: vec![source.clone(), other.clone()],
            accounts: vec![Account {
                id: "a".into(),
                provider: "claude".into(),
                name: "A".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        engine.store.save_settings(&settings).unwrap();
        let settings = engine.store.settings().unwrap();
        let zone = jiff::tz::TimeZone::system();
        let today = jiff::Timestamp::now().to_zoned(zone.clone()).date();
        let put = |store: &crate::store::Store,
                   source: &Source,
                   id: &str,
                   session: &str,
                   days_ago: i64,
                   time: &str| {
            let date = today.checked_sub(jiff::Span::new().days(days_ago)).unwrap();
            let civil: jiff::civil::DateTime = format!("{date}T{time}").parse().unwrap();
            let stamp = civil
                .to_zoned(zone.clone())
                .unwrap()
                .timestamp()
                .as_second();
            let time = jiff::Timestamp::from_second(stamp).unwrap().to_string();
            let row = json!({"type":"assistant","timestamp":time,"sessionId":session,"message":{"id":id,"model":"claude-fixture","usage":{"input_tokens":10,"output_tokens":1}}});
            let event = crate::usage::parse(source, &mut ParseState::default(), &row).unwrap();
            store.put_event(&event, &[], &settings).unwrap();
            stamp
        };
        let suggest = |engine: &mut crate::Engine| {
            engine
                .wakeup_call("wakeups.suggest", json!({"sourceId":"s"}))
                .unwrap()
        };
        for day in 1..=4 {
            put(
                &engine.store,
                &source,
                &format!("work-{day}"),
                &format!("w{day}"),
                day,
                "09:10:00",
            );
        }
        let few = suggest(&mut engine);
        assert_eq!(
            (
                few["insufficient"].as_bool(),
                few["reason"].as_str(),
                few["days"].as_i64()
            ),
            (Some(true), Some("days"), Some(4))
        );
        put(&engine.store, &source, "work-5", "w5", 5, "09:10:00");
        put(&engine.store, &source, "today", "t", 0, "00:01:00");
        put(&engine.store, &source, "too-old", "o", 31, "00:01:00");
        put(&engine.store, &other, "other-source", "x", 2, "00:01:00");
        // A local wakeup run at 06:00 two days ago; its session must not become the work start.
        let run = put(
            &engine.store,
            &source,
            "wakeup",
            "wake-session",
            2,
            "06:00:10",
        ) - 10;
        let root = dir.path().join("wakeups");
        std::fs::create_dir_all(&root).unwrap();
        let db = rusqlite::Connection::open(root.join("runner.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE runs(id INTEGER PRIMARY KEY,task TEXT,occurrence TEXT UNIQUE,started INTEGER,ended INTEGER,status TEXT,model TEXT,effort TEXT)").unwrap();
        db.execute("INSERT INTO runs(task,occurrence,started,ended,status) VALUES('wake-x','o',?1,?2,'success')", [run, run + 20]).unwrap();
        db.execute_batch(
            "CREATE TABLE run_identity(run_id INTEGER PRIMARY KEY,payload TEXT NOT NULL)",
        )
        .unwrap();
        db.execute("INSERT INTO run_identity VALUES(1,?1)", [json!({"sourceId":"s","provider":"claude","sessionIds":["wake-session"],"complete":true}).to_string()]).unwrap();
        // An unrelated run overlaps a genuine manual session. Time overlap must not remove it.
        db.execute("INSERT INTO runs(id,task,occurrence,started,ended,status) VALUES(2,'wake-other','other',?1,?2,'success')", [run + 3 * 3600, run + 3 * 3600 + 30]).unwrap();
        db.execute("INSERT INTO run_identity VALUES(2,?1)", [json!({"sourceId":"other","provider":"claude","sessionIds":["w2"],"complete":false}).to_string()]).unwrap();
        let before = engine.store.wakeups().unwrap().len();
        let value = suggest(&mut engine);
        assert_eq!(value["typicalStart"], "09:10", "{value}");
        assert_eq!(
            (value["suggested"].as_str(), value["resetAt"].as_str()),
            (Some("06:15"), Some("11:15"))
        );
        assert_eq!(
            (value["days"].as_i64(), value["excludedSessions"].as_i64()),
            (Some(5), Some(1)),
            "Today, older days and other sources are outside the sample"
        );
        assert_eq!(
            engine.store.wakeups().unwrap().len(),
            before,
            "A suggestion never saves or deploys a task"
        );
        db.execute("INSERT INTO runs(id,task,occurrence,started,status) VALUES(3,'wake-legacy','legacy',?1,'success')", [run]).unwrap();
        let unknown = suggest(&mut engine);
        assert_eq!(
            unknown["reason"], "wakeupIdentity",
            "Unknown automatic ownership is explicit, never inferred from overlap"
        );
        assert!(
            engine
                .wakeup_call("wakeups.suggest", json!({"sourceId":"missing"}))
                .is_err()
        );
    }
}
