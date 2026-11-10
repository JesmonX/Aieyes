//! Token totals of live sessions, joined from imported history.
//!
//! The join uses the native session id that both the live monitor and the history import record
//! (see `docs/changes.md` for the per-Agent list), restricted to the session's own source. The
//! current turn is matched by the native turn id where the import records it (Codex, DeepSeek
//! Harness); otherwise by explicit millisecond bounds of the turn. Output speed is reported only
//! when every recorded request of the turn carries model request timing evidence.
use crate::{
    models::{OutputTiming, Settings, Source, Tokens},
    sessions::{LiveSession, Snapshot},
    store::Store,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionUsage {
    /// All tokens recorded for the session, including merged child sessions.
    pub session_tokens: u64,
    /// Tokens of the current turn; absent when the turn cannot be attributed or has no records yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_output_tokens: Option<u64>,
    /// Output tokens ÷ model request time (including first-token wait), excluding tools/user waits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_per_second: Option<f64>,
}

/// Providers whose imported usage records the same native turn id as the live session.
fn native_turns(provider: &str) -> bool {
    matches!(provider, "codex" | "deepseek-harness")
}
/// Monitor-generated turn ids ("<ms>-<generation>") are not native and never match history.
fn synthetic_turn(turn: &str) -> bool {
    turn.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty()
            && !b.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit())
            && b.bytes().all(|c| c.is_ascii_digit())
    })
}
/// History identities of a live session: native ids, with Claude subagents mapped to the parent
/// conversation they are imported under, plus the import default for path-keyed transcripts.
pub(crate) fn history_ids(provider: &str, session: &LiveSession) -> Vec<String> {
    let mut ids = Vec::new();
    let mut push = |id: String| {
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    };
    let natives = if session.member_sessions.is_empty() {
        std::slice::from_ref(&session.native_session)
    } else {
        &session.member_sessions[..]
    };
    for native in natives {
        if provider == "claude" {
            push(
                native
                    .split_once(':')
                    .map_or(native.as_str(), |(parent, _)| parent)
                    .to_string(),
            );
        } else {
            push(native.clone());
        }
    }
    for path in &session.log_paths {
        push(format!(
            "{}:{}",
            session.source_id,
            crate::models::hash(path)
        ));
    }
    ids
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
struct Row {
    turn_id: String,
    tokens: Tokens,
    output_timing: Option<OutputTiming>,
}

/// Pure aggregation, shared with tests. `rows` are (stamp seconds, record).
fn aggregate(provider: &str, session: &LiveSession, rows: &[(i64, Row)]) -> Option<SessionUsage> {
    if rows.is_empty() {
        return None;
    }
    let mut usage = SessionUsage {
        session_tokens: rows.iter().map(|(_, r)| r.tokens.total()).sum(),
        ..Default::default()
    };
    let by_turn =
        native_turns(provider) && !session.turn_id.is_empty() && !synthetic_turn(&session.turn_id);
    let in_turn = |stamp: i64, row: &Row| {
        if by_turn {
            return row.turn_id == session.turn_id;
        }
        // Seconds-resolution records count only when the whole second lies inside the turn.
        session.turn_started_at > 0
            && stamp.saturating_mul(1000) >= session.turn_started_at as i64
            && session
                .turn_ended_at
                .is_none_or(|end| stamp.saturating_mul(1000) <= end as i64)
    };
    let turn: Vec<&Row> = rows
        .iter()
        .filter(|(stamp, row)| in_turn(*stamp, row))
        .map(|(_, row)| row)
        .collect();
    if turn.is_empty() {
        return Some(usage);
    }
    let output: u64 = turn.iter().map(|r| r.tokens.output).sum();
    usage.turn_tokens = Some(turn.iter().map(|r| r.tokens.total()).sum());
    usage.turn_output_tokens = Some(output);
    // Every request that produced output needs its own timing; one untimed request hides the rate.
    let producing: Vec<&&Row> = turn.iter().filter(|r| r.tokens.output > 0).collect();
    let expected = format!("{provider}-request-v1");
    let timed = producing.iter().all(|r| {
        r.output_timing
            .as_ref()
            .is_some_and(|t| t.duration_ms > 0 && t.evidence == expected)
    });
    let mut durations = HashMap::<String, u64>::new();
    for (index, row) in producing.iter().enumerate() {
        if let Some(timing) = &row.output_timing {
            let key = if timing.request_id.is_empty() {
                format!("record:{index}")
            } else {
                timing.request_id.clone()
            };
            durations
                .entry(key)
                .and_modify(|n| *n = (*n).max(timing.duration_ms))
                .or_insert(timing.duration_ms);
        }
    }
    let duration: u64 = durations.values().sum();
    if matches!(
        provider,
        "codex" | "claude" | "antigravity" | "deepseek-harness"
    ) && output > 0
        && timed
        && duration > 0
    {
        usage.output_tokens_per_second = Some(output as f64 * 1000.0 / duration as f64);
    }
    Some(usage)
}

type Key = (String, Vec<String>, String, u64, Option<u64>, (i64, i64));

type ImportRequest = (Settings, Source, Option<Vec<PathBuf>>);
struct ImportWorker {
    requests: std::sync::mpsc::SyncSender<ImportRequest>,
    completed: std::sync::mpsc::Receiver<(String, bool)>,
}

#[derive(Default)]
pub struct SessionUsageReader {
    cache: HashMap<String, (Key, Option<SessionUsage>)>,
    /// Last prompt import request per source and the transcript sizes it saw.
    imported: HashMap<String, (Instant, Vec<(PathBuf, u64)>)>,
    indexed: bool,
    /// Imports run on their own connection so a busy writer never delays live status.
    worker: Option<ImportWorker>,
    pending: HashMap<String, Vec<(PathBuf, u64)>>,
    attempted: HashMap<String, Instant>,
}

/// JSONL sources whose live transcripts can be imported one file at a time.
fn per_file(provider: &str) -> bool {
    matches!(provider, "codex" | "claude")
}

impl SessionUsageReader {
    /// Import the history of active local sessions promptly (the regular schedule can be minutes),
    /// then attach token totals to every session of the snapshot.
    pub fn enrich(&mut self, snapshot: &mut Snapshot, store: &Store, settings: &Settings) {
        self.import_active(snapshot, store, settings);
        if !self.indexed {
            // Lazily, in the session monitor's own connection: other engines never wait on it.
            self.indexed = store
                .db
                .execute_batch("CREATE INDEX IF NOT EXISTS events_session ON events(json_extract(payload,'$.sessionId'))")
                .is_ok();
        }
        let revision: (i64, i64) = store
            .db
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(revision),0) FROM usage_day_revision",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((-1, -1));
        let mut keep = std::collections::HashSet::new();
        for session in &mut snapshot.sessions {
            let Some(source) = settings.sources.iter().find(|s| s.id == session.source_id) else {
                continue;
            };
            let ids = history_ids(&source.provider, session);
            let key: Key = (
                session.source_id.clone(),
                ids.clone(),
                session.turn_id.clone(),
                session.turn_started_at,
                session.turn_ended_at,
                revision,
            );
            keep.insert(session.id.clone());
            if let Some((cached, usage)) = self.cache.get(&session.id)
                && *cached == key
            {
                session.usage = usage.clone();
                continue;
            }
            let usage = rows(store, &session.source_id, &ids)
                .ok()
                .and_then(|rows| aggregate(&source.provider, session, &rows));
            session.usage = usage.clone();
            self.cache.insert(session.id.clone(), (key, usage));
        }
        self.cache.retain(|id, _| keep.contains(id));
    }

    fn import_active(&mut self, snapshot: &Snapshot, store: &Store, settings: &Settings) {
        let mut disconnected = false;
        if let Some(worker) = &self.worker {
            loop {
                match worker.completed.try_recv() {
                    Ok((id, success)) => {
                        if let Some(sizes) = self.pending.remove(&id)
                            && success
                        {
                            self.imported.insert(id, (Instant::now(), sizes));
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.worker = None;
            self.pending.clear();
        }
        let mut wanted: HashMap<String, (&Source, Vec<PathBuf>)> = HashMap::new();
        for session in snapshot
            .sessions
            .iter()
            .filter(|s| s.machine_id == "local" && s.phase.active())
        {
            let Some(source) = settings
                .sources
                .iter()
                .find(|s| s.id == session.source_id && s.host_id.is_none())
            else {
                continue;
            };
            let entry = wanted
                .entry(source.id.clone())
                .or_insert_with(|| (source, vec![]));
            if per_file(&source.provider) {
                entry.1.extend(
                    session
                        .log_paths
                        .iter()
                        .map(PathBuf::from)
                        .filter(|p| p.is_file()),
                );
            }
        }
        for (id, (source, paths)) in wanted {
            if per_file(&source.provider) && paths.is_empty() {
                continue;
            }
            let sizes: Vec<(PathBuf, u64)> = paths
                .iter()
                .map(|p| {
                    (
                        p.clone(),
                        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                    )
                })
                .collect();
            let changed = self
                .imported
                .get(&id)
                .is_none_or(|(_, seen)| *seen != sizes || !per_file(&source.provider));
            let minimum = if per_file(&source.provider) {
                Duration::from_secs(3)
            } else {
                Duration::from_secs(10)
            };
            if !changed
                || self.pending.contains_key(&id)
                || self
                    .attempted
                    .get(&id)
                    .is_some_and(|at| at.elapsed() < minimum)
            {
                continue;
            }
            let only = per_file(&source.provider).then(|| paths.clone());
            let worker = self
                .worker
                .get_or_insert_with(|| spawn_worker(store.root.clone()));
            match worker
                .requests
                .try_send((settings.clone(), source.clone(), only))
            {
                Ok(()) => {
                    self.attempted.insert(id.clone(), Instant::now());
                    self.pending.insert(id, sizes);
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    // Recreate a dead worker on the next poll; no file version was acknowledged.
                    self.worker = None;
                    self.pending.clear();
                }
                Err(std::sync::mpsc::TrySendError::Full(_)) => {}
            }
        }
    }
}

fn spawn_worker(root: PathBuf) -> ImportWorker {
    let (tx, rx) = std::sync::mpsc::sync_channel::<ImportRequest>(4);
    let (completed, results) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Ok(store) = Store::open(&root) else {
            return;
        };
        while let Ok((settings, source, only)) = rx.recv() {
            let success = matches!(
                crate::import::scan_when_idle(&store, &settings, &source, only.as_deref()),
                Ok(Some(_))
            );
            if completed.send((source.id, success)).is_err() {
                break;
            }
        }
    });
    ImportWorker {
        requests: tx,
        completed: results,
    }
}

fn rows(store: &Store, source_id: &str, ids: &[String]) -> Result<Vec<(i64, Row)>> {
    let mut query = store.db.prepare_cached(
        "SELECT e.stamp,e.payload FROM events e WHERE json_extract(e.payload,'$.sessionId') IN (SELECT value FROM json_each(?2)) AND EXISTS(SELECT 1 FROM event_sources es WHERE es.event_id=e.id AND es.source_id=?1)",
    )?;
    let rows = query
        .query_map(
            rusqlite::params![source_id, serde_json::to_string(ids)?],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )?
        .filter_map(|row| row.ok())
        .filter_map(|(stamp, payload)| {
            serde_json::from_str::<Row>(&payload)
                .ok()
                .map(|row| (stamp, row))
        })
        .collect();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(stamp: i64, turn: &str, output: u64, timing: Option<u64>) -> (i64, Row) {
        (
            stamp,
            Row {
                turn_id: turn.into(),
                tokens: Tokens {
                    input: 100,
                    output,
                    ..Default::default()
                },
                output_timing: timing.map(|duration_ms| OutputTiming {
                    duration_ms,
                    evidence: "codex-request-v1".into(),
                    request_id: String::new(),
                }),
            },
        )
    }
    fn live(turn: &str, started: u64, ended: Option<u64>) -> LiveSession {
        LiveSession {
            turn_id: turn.into(),
            turn_started_at: started,
            turn_ended_at: ended,
            ..Default::default()
        }
    }
    #[test]
    fn native_turn_ids_attribute_the_turn_and_every_request_needs_output_timing() {
        let rows = vec![
            row(10, "old", 50, Some(500)),
            row(20, "t", 40, Some(1000)),
            row(21, "t", 60, Some(1000)),
        ];
        let usage = aggregate("codex", &live("t", 1, None), &rows).unwrap();
        assert_eq!(
            (
                usage.session_tokens,
                usage.turn_tokens,
                usage.turn_output_tokens
            ),
            (450, Some(300), Some(100))
        );
        assert_eq!(
            usage.output_tokens_per_second,
            Some(50.0),
            "Sum of output over sum of output time"
        );
        let partial = vec![row(20, "t", 40, Some(1000)), row(21, "t", 60, None)];
        assert_eq!(
            aggregate("codex", &live("t", 1, None), &partial)
                .unwrap()
                .output_tokens_per_second,
            None,
            "Partial timing never represents the turn"
        );
        assert_eq!(
            aggregate("codex", &live("t", 1, None), &[row(20, "t", 0, None)])
                .unwrap()
                .output_tokens_per_second,
            None,
            "No output, no rate"
        );
        assert_eq!(
            aggregate("codex", &live("new", 1, None), &rows)
                .unwrap()
                .turn_tokens,
            None,
            "A turn without records yet is unknown, not zero"
        );
    }
    #[test]
    fn only_four_request_timing_providers_are_enabled_and_shared_request_time_is_counted_once() {
        for provider in [
            "codex",
            "claude",
            "antigravity",
            "deepseek-harness",
            "opencode",
        ] {
            let mut rows = vec![row(20, "t", 40, Some(1000)), row(21, "t", 60, Some(2000))];
            for (_, row) in &mut rows {
                let t = row.output_timing.as_mut().unwrap();
                t.evidence = format!("{provider}-request-v1");
                t.request_id = "same-native-request".into();
            }
            let usage = aggregate(provider, &live("t", 1, None), &rows).unwrap();
            assert_eq!(
                usage.output_tokens_per_second,
                (provider != "opencode").then_some(50.0)
            );
            rows[1].1.output_timing = None;
            assert_eq!(
                aggregate(provider, &live("t", 1, None), &rows)
                    .unwrap()
                    .output_tokens_per_second,
                None
            );
        }
    }
    #[test]
    fn other_providers_use_millisecond_turn_bounds_and_seconds_inside_them() {
        // Records carry seconds; the turn started at 20.500 s and ended at 30.000 s.
        let rows = vec![
            row(19, "", 10, None),
            row(20, "", 20, None),
            row(21, "", 30, None),
            row(30, "", 40, None),
            row(31, "", 50, None),
        ];
        let usage = aggregate("claude", &live("prompt", 20_500, Some(30_000)), &rows).unwrap();
        assert_eq!(
            usage.turn_output_tokens,
            Some(70),
            "Only whole seconds within the bounds count"
        );
        assert_eq!(
            usage.output_tokens_per_second, None,
            "Claude records have no output timing"
        );
        let open = aggregate("claude", &live("prompt", 20_500, None), &rows).unwrap();
        assert_eq!(
            open.turn_output_tokens,
            Some(120),
            "A running turn has no upper bound"
        );
        assert_eq!(
            aggregate("claude", &live("", 0, None), &rows)
                .unwrap()
                .turn_tokens,
            None,
            "No turn start, no turn tokens"
        );
        // A monitor-generated Codex turn id falls back to bounds.
        assert_eq!(
            aggregate("codex", &live("20500-1", 20_500, None), &rows)
                .unwrap()
                .turn_output_tokens,
            Some(120)
        );
        assert!(aggregate("claude", &live("p", 1, None), &[]).is_none());
    }
    #[test]
    fn history_ids_map_subagents_and_keep_path_defaults() {
        let mut session = LiveSession {
            source_id: "s".into(),
            native_session: "parent".into(),
            ..Default::default()
        };
        session.member_sessions = vec!["parent".into(), "parent:agent".into()];
        session.log_paths = vec!["/logs/a.jsonl".into()];
        assert_eq!(
            history_ids("claude", &session),
            vec![
                "parent".to_string(),
                format!("s:{}", crate::models::hash("/logs/a.jsonl"))
            ]
        );
        assert_eq!(
            history_ids("codex", &session)[..2],
            ["parent".to_string(), "parent:agent".to_string()]
        );
    }
}

#[cfg(test)]
mod live_tests {
    use crate::{models::*, sessions::SessionMonitor, store::Store};
    use serde_json::json;
    use std::{io::Write, time::SystemTime};
    #[test]
    fn live_codex_sessions_join_imported_tokens_and_import_new_requests_promptly() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let logs = root.path().join("sessions/2026/11/09");
        std::fs::create_dir_all(&logs).unwrap();
        let path = logs.join("rollout-2026-11-09T00-00-00-session.jsonl");
        let now = chrono::Utc::now();
        let at = |seconds: i64| (now - chrono::Duration::seconds(seconds)).to_rfc3339();
        let ms =
            |seconds: i64| (now - chrono::Duration::seconds(seconds)).timestamp_millis() as u64;
        let count = |input: u64, output: u64, last: (u64, u64), ago: i64| {
            json!({"type":"event_msg","timestamp":at(ago),"payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":input,"output_tokens":output},"last_token_usage":{"input_tokens":last.0,"output_tokens":last.1}}}}).to_string()
        };
        let lines = [
            json!({"type":"session_meta","timestamp":at(60),"payload":{"id":"session","source":"cli","cwd":root.path()}}).to_string(),
            json!({"type":"event_msg","timestamp":at(50),"payload":{"type":"task_started","turn_id":"t1"}}).to_string(),
            json!({"type":"turn_context","timestamp":at(50),"payload":{"turn_id":"t1","model":"gpt-fixture"}}).to_string(),
            json!({"type":"response_item","timestamp":at(45),"payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"PRIVATE"}]}}).to_string(),
            json!({"type":"event_msg","timestamp":at(44),"payload":{"type":"item_completed","turn_id":"t1","item":{"type":"AgentMessage"},"started_at_ms":ms(46),"completed_at_ms":ms(44)}}).to_string(),
            count(1000, 100, (1000, 100), 43),
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let store = Store::open(data.path()).unwrap();
        let source = Source {
            id: "codex-local".into(),
            provider: "codex".into(),
            path: root.path().to_string_lossy().into(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let settings = store.settings().unwrap();
        // A previous scheduled scan's status must survive both skipped and successful prompt imports.
        store
            .source_status(&source.id, &json!({"sentinel":"scheduled-status"}))
            .unwrap();
        let scheduled_status: String = store
            .db
            .query_row(
                "SELECT payload FROM source_status WHERE source_id=?1",
                [&source.id],
                |r| r.get(0),
            )
            .unwrap();
        // The first prompt import is skipped under the same lock as a regular history import.
        // It must retry after release even when the final transcript never changes again.
        let file_lock = std::fs::File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(
                store
                    .root
                    .join(format!("history-{}.lock", crate::models::hash(&source.id))),
            )
            .unwrap();
        let lock = crate::file_lock::FileLock::try_exclusive(file_lock).unwrap();
        let mut monitor = SessionMonitor::default();
        let initial = monitor.read_with_usage(&settings, SystemTime::now(), &store);
        assert!(initial.sessions[0].usage.is_none());
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(lock);
        let retry = std::time::Instant::now();
        let snapshot = loop {
            let snapshot = monitor.read_with_usage(&settings, SystemTime::now(), &store);
            if snapshot.sessions.iter().any(|s| s.usage.is_some()) {
                break snapshot;
            }
            assert!(
                retry.elapsed() < std::time::Duration::from_secs(8),
                "A skipped import must retry without another transcript write"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let session = &snapshot.sessions[0];
        let usage = session.usage.as_ref().expect("joined usage");
        assert_eq!(
            (
                usage.session_tokens,
                usage.turn_tokens,
                usage.turn_output_tokens
            ),
            (1100, Some(1100), Some(100))
        );
        let rate = usage.output_tokens_per_second.unwrap();
        assert!(
            (rate - 100.0 / 6.0).abs() < 0.1,
            "100 output tokens over a 6 s model request (including first-token wait): {rate}"
        );
        let wire = serde_json::to_value(session).unwrap();
        assert_eq!(wire["usage"]["sessionTokens"], 1100);
        assert!(!wire.to_string().contains("PRIVATE"));
        // A new request with a tool call: imported promptly, totals grow and the turn has no rate.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", json!({"type":"response_item","timestamp":at(5),"payload":{"type":"function_call","name":"shell","call_id":"c","arguments":"PRIVATE"}})).unwrap();
        writeln!(file, "{}", count(1500, 160, (500, 60), 4)).unwrap();
        drop(file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let updated = loop {
            let snapshot = monitor.read_with_usage(&settings, SystemTime::now(), &store);
            if snapshot.sessions[0]
                .usage
                .as_ref()
                .is_some_and(|u| u.session_tokens == 1660)
                || std::time::Instant::now() > deadline
            {
                break snapshot.sessions[0].usage.clone().unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        assert_eq!(
            (updated.session_tokens, updated.turn_output_tokens),
            (1660, Some(160)),
            "Prompt import without waiting for the schedule"
        );
        assert_eq!(
            updated.output_tokens_per_second, None,
            "A request without output timing hides the turn rate"
        );
        let status: Option<String> = store
            .db
            .query_row(
                "SELECT payload FROM source_status WHERE source_id='codex-local'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(
            status.as_deref(),
            Some(scheduled_status.as_str()),
            "Prompt file imports preserve the previous scheduled scan's status exactly"
        );
    }
}
