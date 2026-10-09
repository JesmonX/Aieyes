//! Local Codex lifecycle metadata only. Never returns prompts or tool arguments.
use crate::{models::Source, store::expand};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Working,
    Thinking,
    Tool,
    Complete,
    Interrupted,
    Unknown,
}
impl Phase {
    pub fn active(self) -> bool {
        matches!(self, Self::Working | Self::Thinking | Self::Tool)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "进行中",
            Self::Thinking => "思考中",
            Self::Tool => "执行工具",
            Self::Complete => "已完成",
            Self::Interrupted => "已中断",
            Self::Unknown => "状态待确认",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub id: String,
    pub source: String,
    pub phase: Phase,
    pub updated_at: u64,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: Vec<LiveSession>,
    pub unavailable: bool,
}
impl Snapshot {
    pub fn phase(&self) -> Option<Phase> {
        self.sessions
            .iter()
            .find(|s| s.phase.active())
            .or_else(|| self.sessions.first())
            .map(|s| s.phase)
    }
    pub fn active_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.phase.active()).count()
    }
    pub fn summary(&self) -> String {
        let count = self.active_count();
        let text = match self.phase() {
            Some(phase) if count > 0 => format!("{} · {} 个会话", phase.label(), count),
            Some(phase) => phase.label().into(),
            None => "无活跃会话".into(),
        };
        if self.unavailable {
            format!("{text} · 部分状态不可用")
        } else {
            text
        }
    }
}

#[derive(Default)]
pub struct SessionMonitor {
    source_key: Vec<(String, String, String)>,
    discovered: Option<SystemTime>,
    discovery_failed: bool,
    files: Vec<(PathBuf, String)>,
    cache: HashMap<PathBuf, (SystemTime, u64, Option<Phase>)>,
}
impl SessionMonitor {
    pub fn read(&mut self, sources: &[Source], now: SystemTime) -> Snapshot {
        let local: Vec<_> = sources
            .iter()
            .filter(|s| s.enabled && s.host_id.is_none() && s.provider == "codex")
            .collect();
        let key: Vec<_> = local
            .iter()
            .map(|s| (s.id.clone(), s.path.clone(), s.name.clone()))
            .collect();
        if self.source_key != key || self.discovered.is_none_or(|t| age(now, t) >= 30) {
            self.source_key = key;
            self.discovered = Some(now);
            self.discovery_failed = false;
            self.files.clear();
            let mut seen = HashSet::new();
            for source in local {
                let root = expand(&source.path);
                let root = if root.join("sessions").is_dir() {
                    root.join("sessions")
                } else {
                    root
                };
                for entry in walkdir::WalkDir::new(root)
                    .into_iter()
                    .filter_entry(|e| e.file_name() != ".aieyes")
                {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => {
                            self.discovery_failed = true;
                            continue;
                        }
                    };
                    if !entry.file_type().is_file()
                        || entry.path().extension().is_none_or(|e| e != "jsonl")
                    {
                        continue;
                    }
                    let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok())
                    else {
                        self.discovery_failed = true;
                        continue;
                    };
                    if age(now, modified) >= 3600 {
                        continue;
                    }
                    let path =
                        fs::canonicalize(entry.path()).unwrap_or_else(|_| entry.path().to_owned());
                    if seen.insert(path.clone()) {
                        self.files.push((path, source.name.clone()));
                    }
                }
            }
            self.cache.retain(|p, _| seen.contains(p));
        }
        let mut snapshot = Snapshot {
            unavailable: self.discovery_failed,
            ..Snapshot::default()
        };
        for (path, source) in &self.files {
            let Ok(meta) = fs::metadata(path) else {
                snapshot.unavailable = true;
                continue;
            };
            let Ok(modified) = meta.modified() else {
                snapshot.unavailable = true;
                continue;
            };
            let elapsed = age(now, modified);
            if elapsed >= 3600 {
                continue;
            }
            let phase = match self.cache.get(path) {
                Some((old, size, phase)) if *old == modified && *size == meta.len() => *phase,
                _ => match read_tail(path) {
                    Ok(phase) => {
                        self.cache
                            .insert(path.clone(), (modified, meta.len(), phase));
                        phase
                    }
                    Err(_) => {
                        snapshot.unavailable = true;
                        continue;
                    }
                },
            };
            if let Some(mut phase) = phase {
                if phase.active() && elapsed > 180 {
                    phase = Phase::Unknown;
                }
                if phase.active() || phase == Phase::Unknown || elapsed < 90 {
                    snapshot.sessions.push(LiveSession {
                        id: crate::models::hash(&path.to_string_lossy()),
                        source: source.clone(),
                        phase,
                        updated_at: modified
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    });
                }
            }
        }
        snapshot
            .sessions
            .sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        snapshot
    }
}
fn age(now: SystemTime, then: SystemTime) -> u64 {
    now.duration_since(then).unwrap_or(Duration::ZERO).as_secs()
}
fn read_tail(path: &PathBuf) -> std::io::Result<Option<Phase>> {
    let mut file = File::open(path)?;
    let end = file.seek(SeekFrom::End(0))?;
    let start = end.saturating_sub(262144);
    file.seek(SeekFrom::Start(start))?;
    let mut data = Vec::new();
    file.take(262144).read_to_end(&mut data)?;
    // A tail may begin halfway through a record. A final incomplete record is retried next poll.
    let data = if start > 0 {
        data.splitn(2, |b| *b == b'\n').nth(1).unwrap_or_default()
    } else {
        &data
    };
    Ok(parse(data))
}
fn parse(data: &[u8]) -> Option<Phase> {
    let mut phase = None;
    for line in data
        .split_inclusive(|b| *b == b'\n')
        .filter(|l| l.last() == Some(&b'\n'))
    {
        let Ok(row) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let kind = row["payload"]["type"].as_str().unwrap_or("");
        match row["type"].as_str() {
            Some("event_msg") => match kind {
                "task_started" => phase = Some(Phase::Working),
                "task_complete" => phase = Some(Phase::Complete),
                "turn_aborted" => phase = Some(Phase::Interrupted),
                _ => {}
            },
            Some("response_item")
                if !matches!(phase, Some(Phase::Complete | Phase::Interrupted)) =>
            {
                match kind {
                    "reasoning" => phase = Some(Phase::Thinking),
                    "function_call" | "custom_tool_call" => phase = Some(Phase::Tool),
                    "function_call_output" | "custom_tool_call_output" => {
                        phase = Some(Phase::Working)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    phase
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn row(kind: &str, event: &str) -> String {
        format!(
            "{}\n",
            json!({"type":kind,"payload":{"type":event,"text":"private prompt"}})
        )
    }
    #[test]
    fn lifecycle_terminal_events_and_partial_records() {
        let mut log = row("event_msg", "task_started") + &row("response_item", "reasoning");
        assert_eq!(parse(log.as_bytes()), Some(Phase::Thinking));
        log += &row("response_item", "custom_tool_call");
        assert_eq!(parse(log.as_bytes()), Some(Phase::Tool));
        log += &row("event_msg", "task_complete");
        log += &row("response_item", "reasoning");
        assert_eq!(parse(log.as_bytes()), Some(Phase::Complete));
        log += &row("event_msg", "task_started");
        log += &row("event_msg", "turn_aborted");
        assert_eq!(parse(log.as_bytes()), Some(Phase::Interrupted));
        log += row("event_msg", "task_started").trim_end();
        assert_eq!(parse(log.as_bytes()), Some(Phase::Interrupted));
        log += "\n";
        assert_eq!(parse(log.as_bytes()), Some(Phase::Working));
    }
    #[test]
    fn source_scope_expiry_deduplication_and_unavailability() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        fs::write(&file, row("event_msg", "task_started")).unwrap();
        let now = fs::metadata(&file).unwrap().modified().unwrap();
        let source = Source {
            id: "one".into(),
            name: "本机".into(),
            path: temp.path().to_string_lossy().into(),
            ..Source::default()
        };
        let mut monitor = SessionMonitor::default();
        let sources = [source.clone(), source.clone()];
        let live = monitor.read(&sources, now);
        assert_eq!(live.active_count(), 1);
        assert!(
            !serde_json::to_string(&live)
                .unwrap()
                .contains("private prompt")
        );
        assert_eq!(
            monitor
                .read(&sources, now + Duration::from_secs(181))
                .phase(),
            Some(Phase::Unknown)
        );
        assert!(
            monitor
                .read(&sources, now + Duration::from_secs(3601))
                .sessions
                .is_empty()
        );
        let disabled = Source {
            enabled: false,
            ..source.clone()
        };
        let remote = Source {
            host_id: Some("host".into()),
            ..source.clone()
        };
        assert!(monitor.read(&[disabled, remote], now).sessions.is_empty());
        let missing = Source {
            path: temp.path().join("missing").to_string_lossy().into(),
            ..source
        };
        assert!(monitor.read(&[missing], now).unavailable);
    }
    #[test]
    fn completed_sessions_expire_even_when_cached() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        fs::write(&file, row("event_msg", "task_complete")).unwrap();
        let now = fs::metadata(&file).unwrap().modified().unwrap();
        let sources = [Source {
            path: file.to_string_lossy().into(),
            ..Source::default()
        }];
        let mut monitor = SessionMonitor::default();
        assert_eq!(monitor.read(&sources, now).phase(), Some(Phase::Complete));
        assert!(
            monitor
                .read(&sources, now + Duration::from_secs(90))
                .sessions
                .is_empty()
        );
    }
}
