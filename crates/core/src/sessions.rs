//! Shared local/SSH lifecycle monitoring. Each SSH machine has an independent bounded worker.
use crate::{
    models::{Settings, Source},
    session_events::{self, Activity, Event},
    session_hooks,
    store::expand,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Working,
    Thinking,
    Tool,
    Complete,
    Interrupted,
    #[default]
    Unknown,
    NeedsInput,
    Failed,
}
impl Phase {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Working | Self::Thinking | Self::Tool | Self::NeedsInput
        )
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Interrupted | Self::Failed)
    }
    pub fn priority(self) -> u8 {
        match self {
            Self::NeedsInput => 0,
            Self::Failed => 1,
            Self::Working | Self::Thinking | Self::Tool => 2,
            Self::Complete | Self::Interrupted => 3,
            Self::Unknown => 4,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "进行中",
            Self::Thinking => "思考中",
            Self::Tool => "执行工具",
            Self::Complete => "已完成",
            Self::Interrupted => "已中断",
            Self::Unknown => "状态待确认",
            Self::NeedsInput => "等待输入",
            Self::Failed => "执行失败",
        }
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveSession {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub provider: String,
    pub machine_id: String,
    pub machine: String,
    pub turn_id: String,
    pub phase: Phase,
    pub updated_at: u64,
    #[serde(skip)]
    pub evidence: String,
    #[serde(skip)]
    pub background: bool,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SourceHealth {
    pub source_id: String,
    pub provider: String,
    pub machine_id: String,
    pub state: String,
    pub error: Option<String>,
    pub checked_at: u64,
    pub hooks: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: Vec<LiveSession>,
    pub unavailable: bool,
    pub sources: Vec<SourceHealth>,
}
impl Snapshot {
    pub fn selected(&self) -> Option<&LiveSession> {
        self.sessions
            .iter()
            .min_by_key(|s| (s.phase.priority(), std::cmp::Reverse(s.updated_at)))
    }
    pub fn phase(&self) -> Option<Phase> {
        self.selected().map(|s| s.phase)
    }
    pub fn active_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.phase.active()).count()
    }
    pub fn summary(&self) -> String {
        let count = self.active_count();
        let text = match self.phase() {
            Some(p) if count > 0 => format!("{} · {} 个会话", p.label(), count),
            Some(p) => p.label().into(),
            None => "无活跃会话".into(),
        };
        if self.unavailable {
            format!("{text} · 部分状态不可用")
        } else {
            text
        }
    }
    fn finish(&mut self) {
        let mut unique: HashMap<String, LiveSession> = HashMap::new();
        for s in self.sessions.drain(..) {
            unique
                .entry(s.id.clone())
                .and_modify(|old| {
                    let old_hook = old.evidence == "hook" && old.phase != Phase::Unknown;
                    let new_hook = s.evidence == "hook" && s.phase != Phase::Unknown;
                    // A parallel tool return does not answer an outstanding DB permission request.
                    if old.evidence == "database"
                        && old.phase == Phase::NeedsInput
                        && s.phase.active()
                    {
                        return;
                    }
                    if s.evidence == "database"
                        && s.phase == Phase::NeedsInput
                        && old.phase.active()
                    {
                        *old = s.clone();
                        return;
                    }
                    if old_hook
                        && (s.phase == Phase::Unknown
                            || (old.background && s.phase == Phase::Complete))
                    {
                        return;
                    }
                    if (new_hook
                        && (old.phase == Phase::Unknown
                            || (s.background && old.phase == Phase::Complete)))
                        || s.updated_at > old.updated_at
                        || (s.updated_at == old.updated_at
                            && (s.phase.priority() < old.phase.priority()
                                || (new_hook && s.phase.priority() == old.phase.priority())))
                    {
                        *old = s.clone();
                    }
                })
                .or_insert(s);
        }
        self.sessions = unique.into_values().collect();
        self.sessions.sort_by_key(|s| {
            (
                s.phase.priority(),
                std::cmp::Reverse(s.updated_at),
                s.id.clone(),
            )
        });
        self.unavailable |= self.sources.iter().any(|s| s.state != "healthy");
    }
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FileBatch {
    key: String,
    identity: String,
    signature: String,
    size: u64,
    offset: u64,
    modified: u64,
    reset: bool,
    events: Vec<Event>,
    hook: bool,
    session: String,
    steps: Option<Vec<(i64, i64, i64)>>,
    alive: Option<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SourceBatch {
    source_id: String,
    files: Vec<FileBatch>,
    error: Option<String>,
}
#[derive(Default)]
struct Cursor {
    identity: String,
    signature: String,
    size: u64,
    offset: u64,
    modified: u64,
    activity: Activity,
}
struct Discovery {
    at: u64,
    paths: Vec<(PathBuf, bool)>,
    error: Option<String>,
}
#[derive(Default)]
struct Reader {
    cursors: HashMap<String, Cursor>,
    discovery: HashMap<String, Discovery>,
    hooks: HashMap<String, (Instant, session_hooks::HookStatus)>,
}
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn normalized_source(s: &Source) -> Source {
    let mut s = s.clone();
    if s.path.trim().is_empty() {
        s.path = crate::contracts::provider(&s.provider)
            .map(|p| p.default_path)
            .unwrap_or("")
            .into();
    }
    s
}
fn wire_source(s: &Source) -> Value {
    json!({"id":s.id,"name":s.name,"provider":s.provider,"path":s.path,"hostId":s.host_id})
}
impl Reader {
    fn manifest(&self) -> Value {
        Value::Object(self.cursors.iter().map(|(p,c)|(p.clone(),json!({"identity":c.identity,"signature":c.signature,"size":c.size,"offset":c.offset,"modified":c.modified,"pid":c.activity.pid,"processStart":c.activity.process_start}))).collect())
    }
    fn consume(
        &mut self,
        source: &Source,
        batch: SourceBatch,
        now: u64,
        remote_now: u64,
        machine: &str,
        hook_state: Option<String>,
    ) -> Snapshot {
        let mut out = Snapshot::default();
        for f in batch.files {
            let cursor = self.cursors.entry(f.key.clone()).or_default();
            if f.reset {
                cursor.activity = Activity::default();
            }
            if cursor.activity.session.is_empty() {
                cursor.activity.session = Path::new(&f.key)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
            }
            let database = f.steps.is_some();
            if let Some(steps) = f.steps {
                let (phase, turn) = session_events::agy_steps(&steps);
                cursor.activity.phase = phase;
                cursor.activity.turn = turn;
                cursor.activity.session = f.session;
            } else {
                for mut event in f.events {
                    if event.at == 0 {
                        event.at = f.modified * 1000;
                    }
                    cursor.activity.apply(event);
                }
            }
            cursor.offset = f.offset;
            cursor.size = f.size;
            cursor.modified = f.modified;
            cursor.identity = f.identity;
            cursor.signature = f.signature;
            let Some(mut phase) = cursor.activity.phase else {
                continue;
            };
            let elapsed = remote_now.saturating_sub(f.modified);
            let alive = if source.host_id.is_none() && cursor.activity.pid > 0 {
                Some(process_alive(
                    cursor.activity.pid,
                    cursor.activity.process_start,
                ))
            } else {
                f.alive
            };
            if phase.active() && alive == Some(false) {
                phase = Phase::Interrupted;
            } else if phase.active()
                && phase != Phase::NeedsInput
                && elapsed > 180
                && alive != Some(true)
            {
                phase = Phase::Unknown;
            }
            if (elapsed >= 3600 && alive != Some(true)) || (phase.terminal() && elapsed >= 90) {
                continue;
            }
            let session = &cursor.activity.session;
            out.sessions.push(LiveSession {
                id: crate::models::hash(&format!(
                    "{}:{}:{session}",
                    source.host_id.as_deref().unwrap_or("local"),
                    source.provider
                )),
                source: source.name.clone(),
                source_id: source.id.clone(),
                provider: source.provider.clone(),
                machine_id: source.host_id.clone().unwrap_or_else(|| "local".into()),
                machine: machine.into(),
                turn_id: cursor.activity.turn.clone(),
                phase,
                updated_at: now.saturating_sub(elapsed),
                evidence: if f.hook {
                    "hook"
                } else if database {
                    "database"
                } else {
                    "log"
                }
                .into(),
                background: cursor.activity.background_active(),
            });
        }
        let error = batch.error.or_else(|| {
            (hook_state.as_deref() == Some("error"))
                .then(|| "会话 hooks 未就绪，正在使用日志恢复；可在 Agents 设置中修复接入".into())
        });
        out.sources.push(SourceHealth {
            source_id: source.id.clone(),
            provider: source.provider.clone(),
            machine_id: source.host_id.clone().unwrap_or_else(|| "local".into()),
            state: if error.is_some() {
                "degraded"
            } else {
                "healthy"
            }
            .into(),
            error,
            checked_at: now,
            hooks: hook_state,
        });
        out
    }
    fn local(&mut self, sources: &[Source], now: u64) -> Snapshot {
        let mut out = Snapshot::default();
        for source in sources {
            let hook = if matches!(source.provider.as_str(), "claude" | "antigravity") {
                let key = session_hooks::key(source);
                if self
                    .hooks
                    .get(&key)
                    .is_none_or(|(t, _)| t.elapsed() >= Duration::from_secs(60))
                {
                    self.hooks.insert(
                        key.clone(),
                        (Instant::now(), session_hooks::local(source, "ensure")),
                    );
                }
                self.hooks.get(&key).map(|(_, s)| s.state.clone())
            } else {
                None
            };
            let batch = self.collect_local(source, now);
            let snapshot = self.consume(source, batch, now, now, "本机", hook);
            out.sessions.extend(snapshot.sessions);
            out.sources.extend(snapshot.sources);
        }
        out
    }
    fn collect_local(&mut self, source: &Source, now: u64) -> SourceBatch {
        let key = format!("{}:{}", source.provider, source.path);
        if self
            .discovery
            .get(&key)
            .is_none_or(|d| now.saturating_sub(d.at) >= 5)
        {
            let root = expand(&source.path);
            let mut paths = Vec::new();
            let mut error = None;
            if !root.exists() {
                error = Some("Agent 数据目录不存在".into());
            }
            let nested = root.join(match source.provider.as_str() {
                "codex" => "sessions",
                "claude" => "projects",
                _ => "conversations",
            });
            let mut roots = vec![(if nested.is_dir() { nested } else { root }, false)];
            if matches!(source.provider.as_str(), "claude" | "antigravity") {
                let events = session_hooks::event_root(source);
                if events.is_dir() {
                    roots.push((events, true));
                }
            }
            for (root, hook) in roots {
                for entry in walkdir::WalkDir::new(root)
                    .max_depth(12)
                    .into_iter()
                    .filter_entry(|e| hook || e.file_name() != ".aieyes")
                {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            error = Some(e.to_string());
                            continue;
                        }
                    };
                    let extension = if !hook && source.provider == "antigravity" {
                        "db"
                    } else {
                        "jsonl"
                    };
                    if !entry.file_type().is_file()
                        || entry.path().extension().is_none_or(|e| e != extension)
                    {
                        continue;
                    }
                    let modified = modified(entry.path());
                    if now.saturating_sub(modified) < 86400 {
                        paths.push((entry.path().to_owned(), hook));
                    }
                }
            }
            paths.sort_by_key(|(p, _)| std::cmp::Reverse(modified(p)));
            paths.dedup();
            if paths.len() > 256 {
                paths.truncate(256);
                error = Some("近期会话过多，显示最近 256 个".into());
            }
            self.discovery.insert(
                key.clone(),
                Discovery {
                    at: now,
                    paths,
                    error,
                },
            );
        }
        let Discovery { paths, error, .. } = self.discovery.get(&key).unwrap();
        let mut batch = SourceBatch {
            source_id: source.id.clone(),
            error: error.clone(),
            files: Vec::new(),
        };
        for (path, hook) in paths {
            let name = path.to_string_lossy().into_owned();
            let result = if !hook && source.provider == "antigravity" {
                read_db(path)
            } else {
                read_file(path, &source.provider, *hook, self.cursors.get(&name))
                    .map_err(anyhow::Error::from)
            };
            match result {
                Ok(f) => batch.files.push(f),
                Err(e) => batch.error = Some(e.to_string()),
            }
        }
        batch
    }
}
fn process_alive(pid: u32, start: u64) -> bool {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .is_some_and(|p| start == 0 || p.start_time() == start)
}
fn modified(path: &Path) -> u64 {
    let stamp = |p: &Path| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .map(seconds)
            .unwrap_or(0)
    };
    stamp(path).max(stamp(&PathBuf::from(format!("{}-wal", path.display()))))
}
fn read_file(
    path: &Path,
    provider: &str,
    hook: bool,
    cursor: Option<&Cursor>,
) -> std::io::Result<FileBatch> {
    let meta = fs::metadata(path)?;
    let modified = seconds(meta.modified()?);
    let signature = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        meta.ino().to_string()
    };
    #[cfg(not(unix))]
    let identity = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_default();
    let reset = cursor.is_none_or(|c| {
        c.identity != identity
            || c.offset > meta.len()
            || (c.size == meta.len() && c.signature != signature)
    });
    let mut offset = if reset {
        meta.len().saturating_sub(262144)
    } else {
        cursor.unwrap().offset
    };
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut data = Vec::new();
    (&mut file).take(524288).read_to_end(&mut data)?;
    if reset && offset > 0 {
        let skip = data
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(data.len());
        offset += skip as u64;
        data.drain(..skip);
    }
    let mut events = Vec::new();
    if reset && provider == "codex" {
        file.seek(SeekFrom::Start(0))?;
        let mut first = Vec::new();
        (&mut file).take(65536).read_to_end(&mut first)?;
        if let Some(line) = first.split(|b| *b == b'\n').next()
            && let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(e) = session_events::codex(&v)
            && e.kind == "identity"
        {
            events.push(e);
        }
    }
    let complete = data
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    for line in data[..complete].split(|b| *b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if hook {
            if let Ok(e) = serde_json::from_value(value) {
                events.push(e);
            }
        } else if provider == "codex" {
            events.extend(session_events::codex(&value));
        } else {
            events.extend(session_events::claude(&value));
        }
    }
    offset += if complete == 0 && data.len() == 524288 {
        data.len()
    } else {
        complete
    } as u64;
    Ok(FileBatch {
        key: path.to_string_lossy().into_owned(),
        identity,
        signature,
        size: meta.len(),
        offset,
        modified,
        reset,
        events,
        hook,
        ..Default::default()
    })
}
fn read_db(path: &Path) -> anyhow::Result<FileBatch> {
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(100))?;
    db.execute_batch("PRAGMA query_only=ON")?;
    let session: String =
        db.query_row("SELECT cascade_id FROM trajectory_meta LIMIT 1", [], |r| {
            r.get(0)
        })?;
    let mut stmt =
        db.prepare("SELECT idx,step_type,status FROM steps ORDER BY idx DESC LIMIT 2048")?;
    let mut steps = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    steps.reverse();
    Ok(FileBatch {
        key: path.to_string_lossy().into_owned(),
        modified: modified(path),
        session,
        steps: Some(steps),
        reset: true,
        ..Default::default()
    })
}
struct RemoteWorker {
    key: String,
    reader: Option<Reader>,
    pending: Option<mpsc::Receiver<(Reader, Result<Snapshot, String>)>>,
    last: Option<Instant>,
    snapshot: Snapshot,
}
impl RemoteWorker {
    fn new(key: String) -> Self {
        Self {
            key,
            reader: Some(Reader::default()),
            pending: None,
            last: None,
            snapshot: Snapshot::default(),
        }
    }
}
#[derive(Default)]
pub struct SessionMonitor {
    local: Reader,
    remote: HashMap<String, RemoteWorker>,
}
impl SessionMonitor {
    pub fn read(&mut self, settings: &Settings, now: SystemTime) -> Snapshot {
        let now = seconds(now);
        let mut seen = HashSet::new();
        let sources: Vec<_> = settings
            .sources
            .iter()
            .filter(|s| {
                settings.source_enabled(s)
                    && matches!(s.provider.as_str(), "codex" | "claude" | "antigravity")
            })
            .map(normalized_source)
            .filter(|s| {
                let path = if s.host_id.is_none() {
                    fs::canonicalize(expand(&s.path))
                        .unwrap_or_else(|_| expand(&s.path))
                        .to_string_lossy()
                        .into_owned()
                } else {
                    s.path.trim_end_matches('/').into()
                };
                seen.insert(format!(
                    "{}:{}:{path}",
                    s.host_id.as_deref().unwrap_or("local"),
                    s.provider
                ))
            })
            .collect();
        let local: Vec<_> = sources
            .iter()
            .filter(|s| s.host_id.is_none())
            .cloned()
            .collect();
        let mut snapshot = self.local.local(&local, now);
        let hosts: HashSet<_> = sources.iter().filter_map(|s| s.host_id.clone()).collect();
        self.remote.retain(|id, _| hosts.contains(id));
        for id in hosts {
            let Some(host) = settings.hosts.iter().find(|h| h.id == id) else {
                continue;
            };
            let sources: Vec<_> = sources
                .iter()
                .filter(|s| s.host_id.as_ref() == Some(&id))
                .cloned()
                .collect();
            let key = crate::models::hash(&format!(
                "{}:{}",
                serde_json::to_string(host).unwrap_or_default(),
                serde_json::to_string(&sources).unwrap_or_default()
            ));
            let worker = self
                .remote
                .entry(id.clone())
                .or_insert_with(|| RemoteWorker::new(key.clone()));
            if worker.key != key {
                *worker = RemoteWorker::new(key);
            }
            let completed = worker.pending.as_ref().and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    Reader::default(),
                    Err("远程会话采集任务已退出，将自动重试".into()),
                )),
            });
            if let Some((reader, result)) = completed {
                worker.pending = None;
                worker.reader = Some(reader);
                worker.last = Some(Instant::now());
                match result {
                    Ok(s) => worker.snapshot = s,
                    Err(error) => {
                        for s in &mut worker.snapshot.sessions {
                            if s.phase.active() {
                                s.phase = Phase::Unknown;
                            }
                        }
                        worker.snapshot.sources = sources
                            .iter()
                            .map(|s| SourceHealth {
                                source_id: s.id.clone(),
                                provider: s.provider.clone(),
                                machine_id: id.clone(),
                                state: "degraded".into(),
                                error: Some(error.clone()),
                                checked_at: now,
                                ..Default::default()
                            })
                            .collect();
                        worker.snapshot.unavailable = true;
                    }
                }
            }
            if worker.pending.is_none()
                && worker
                    .last
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
            {
                let mut reader = worker.reader.take().unwrap_or_default();
                let sources = sources.clone();
                let host = host.clone();
                let (tx, rx) = mpsc::channel();
                worker.pending = Some(rx);
                std::thread::spawn(move || {
                    let result=(||->anyhow::Result<Snapshot>{
                        let value=crate::ssh::session_request(&host,json!({"action":"collect","sources":sources.iter().map(wire_source).collect::<Vec<_>>(),"cursors":reader.manifest()}))?;
                        let remote_now=value["now"].as_u64().unwrap_or(now);let observed=seconds(SystemTime::now());
                        let mut snapshot=Snapshot::default();let polls:Vec<SourceBatch>=serde_json::from_value(value["polls"].clone())?;
                        for batch in polls {if let Some(source)=sources.iter().find(|s|s.id==batch.source_id) {
                            let hook=value["hooks"].as_array().and_then(|rows|rows.iter().find(|v|v["sourceId"]==source.id)).and_then(|v|v["state"].as_str()).map(String::from);
                            let state=reader.consume(source,batch,observed,remote_now,&host.name,hook);
                            snapshot.sessions.extend(state.sessions);snapshot.sources.extend(state.sources);
                        }}snapshot.finish();Ok(snapshot)
                    })().map_err(|e|format!("{e:#}"));
                    let _ = tx.send((reader, result));
                });
            }
            if worker.snapshot.sources.is_empty() {
                snapshot
                    .sources
                    .extend(sources.iter().map(|s| SourceHealth {
                        source_id: s.id.clone(),
                        provider: s.provider.clone(),
                        machine_id: id.clone(),
                        state: "connecting".into(),
                        checked_at: now,
                        ..Default::default()
                    }));
            }
            snapshot.sessions.extend(
                worker
                    .snapshot
                    .sessions
                    .iter()
                    .filter(|s| {
                        now.saturating_sub(s.updated_at)
                            < if s.phase.terminal() { 90 } else { 3600 }
                    })
                    .cloned(),
            );
            snapshot.sources.extend(worker.snapshot.sources.clone());
        }
        snapshot.finish();
        snapshot
    }
}
#[cfg(test)]
fn parse(data: &[u8]) -> Option<Phase> {
    let mut state = Activity::default();
    for line in data
        .split_inclusive(|b| *b == b'\n')
        .filter(|l| l.last() == Some(&b'\n'))
    {
        if let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(e) = session_events::codex(&v)
        {
            state.apply(e);
        }
    }
    state.phase
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentConfiguration, Source};
    use serde_json::json;
    fn row(kind: &str, event: &str) -> String {
        format!(
            "{}\n",
            json!({"type":kind,"payload":{"type":event,"text":"private prompt"}})
        )
    }
    #[test]
    fn pet_input_wait_survives_unrelated_parallel_tool_output() {
        let call=json!({"type":"response_item","payload":{"type":"function_call","name":"functions.request_user_input","call_id":"ask"}}).to_string()+"\n";
        assert_eq!(parse(call.as_bytes()), Some(Phase::NeedsInput));
        let other=json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"other"}}).to_string()+"\n";
        let log = call
            + &other
            + &row("response_item", "function_call")
            + &row("response_item", "reasoning");
        assert_eq!(parse(log.as_bytes()), Some(Phase::NeedsInput));
        let answer=json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"ask"}}).to_string()+"\n";
        assert_eq!(parse((log + &answer).as_bytes()), Some(Phase::Working));
        assert_eq!(
            parse(row("event_msg", "error").as_bytes()),
            Some(Phase::Failed)
        );
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
        let mut settings = Settings {
            sources: vec![source.clone(), source.clone()],
            ..Default::default()
        };
        let live = monitor.read(&settings, now);
        assert_eq!(live.active_count(), 1);
        assert!(
            !serde_json::to_string(&live)
                .unwrap()
                .contains("private prompt")
        );
        assert_eq!(
            monitor
                .read(&settings, now + Duration::from_secs(181))
                .phase(),
            Some(Phase::Unknown)
        );
        assert!(
            monitor
                .read(&settings, now + Duration::from_secs(3601))
                .sessions
                .is_empty()
        );
        settings.agents = vec![AgentConfiguration {
            provider: "codex".into(),
            enabled: false,
            machine_ids: vec!["local".into()],
        }];
        assert!(monitor.read(&settings, now).sessions.is_empty());
        settings.agents[0].enabled = true;
        assert_eq!(monitor.read(&settings, now).active_count(), 1);
        settings.agents[0].machine_ids.clear();
        assert!(monitor.read(&settings, now).sessions.is_empty());
        settings.agents[0].machine_ids.push("local".into());
        settings.agents.push(settings.agents[0].clone());
        assert!(monitor.read(&settings, now).sessions.is_empty());
        settings.agents.clear();
        let disabled = Source {
            enabled: false,
            ..source.clone()
        };
        let remote = Source {
            host_id: Some("host".into()),
            ..source.clone()
        };
        settings.sources = vec![disabled, remote];
        assert!(monitor.read(&settings, now).sessions.is_empty());
        let missing = Source {
            path: temp.path().join("missing").to_string_lossy().into(),
            ..source
        };
        settings.sources = vec![missing];
        assert!(monitor.read(&settings, now).unavailable);
    }
    #[test]
    fn completed_sessions_expire_even_when_cached() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        fs::write(&file, row("event_msg", "task_complete")).unwrap();
        let now = fs::metadata(&file).unwrap().modified().unwrap();
        let settings = Settings {
            sources: vec![Source {
                path: file.to_string_lossy().into(),
                ..Source::default()
            }],
            ..Default::default()
        };
        let mut monitor = SessionMonitor::default();
        assert_eq!(monitor.read(&settings, now).phase(), Some(Phase::Complete));
        assert!(
            monitor
                .read(&settings, now + Duration::from_secs(90))
                .sessions
                .is_empty()
        );
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use std::io::Write;
    fn source(root: &Path, provider: &str) -> Source {
        Source {
            id: provider.into(),
            provider: provider.into(),
            name: provider.into(),
            path: root.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }
    fn hooks_only(root: &Path) {
        fs::create_dir_all(root.join(".aieyes/session-events")).unwrap();
        fs::write(root.join(".aieyes/session-hooks-disabled"), b"disabled").unwrap();
    }
    #[test]
    fn three_providers_and_wal_waits_are_normalized_without_content() {
        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join("codex");
        let claude = temp.path().join("claude");
        let agy = temp.path().join("agy");
        fs::create_dir_all(&codex).unwrap();
        hooks_only(&claude);
        hooks_only(&agy);
        fs::create_dir(agy.join("conversations")).unwrap();
        fs::write(codex.join("s.jsonl"),b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"text\":\"SECRET\"}}\n").unwrap();
        let at = seconds(SystemTime::now()) * 1000;
        let events = [
            session_events::hook(
                "claude",
                "UserPromptSubmit",
                &json!({"session_id":"c","prompt":"SECRET"}),
                at,
            ),
            session_events::hook(
                "claude",
                "PermissionRequest",
                &json!({"session_id":"c","tool_use_id":"ask","tool_input":"SECRET"}),
                at + 1,
            ),
        ];
        fs::write(
            claude.join(".aieyes/session-events/c.jsonl"),
            events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let db = rusqlite::Connection::open(agy.join("conversations/a.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('a');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,step_payload TEXT);INSERT INTO steps VALUES(0,14,3,'SECRET'),(1,21,2,'SECRET');").unwrap();
        let mut settings = Settings {
            sources: vec![
                source(&codex, "codex"),
                source(&claude, "claude"),
                source(&agy, "antigravity"),
            ],
            ..Default::default()
        };
        let mut monitor = SessionMonitor::default();
        let snapshot = monitor.read(&settings, SystemTime::now());
        assert_eq!(snapshot.active_count(), 3);
        assert_eq!(snapshot.phase(), Some(Phase::NeedsInput));
        assert!(
            snapshot
                .sessions
                .iter()
                .all(|s| !s.provider.is_empty() && s.machine_id == "local")
        );
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("SECRET"));
        db.execute("UPDATE steps SET status=9 WHERE idx=1", [])
            .unwrap();
        assert_eq!(
            monitor
                .read(&settings, SystemTime::now())
                .sessions
                .iter()
                .find(|s| s.provider == "antigravity")
                .unwrap()
                .phase,
            Phase::NeedsInput
        );
        let mut missing = source(&temp.path().join("missing"), "codex");
        missing.id = "missing".into();
        settings.sources.push(missing);
        let partial = monitor.read(&settings, SystemTime::now());
        assert!(partial.unavailable);
        assert_eq!(crate::pets::project(&partial).behavior, "attention");
    }
    #[test]
    fn incremental_tail_retains_parallel_wait_and_handles_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("s.jsonl");
        fs::write(&path,format!("{}\n",json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"ask"}}))).unwrap();
        let settings = Settings {
            sources: vec![source(temp.path(), "codex")],
            ..Default::default()
        };
        let mut m = SessionMonitor::default();
        assert_eq!(
            m.read(&settings, SystemTime::now()).phase(),
            Some(Phase::NeedsInput)
        );
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            "{}",
            json!({"type":"unrelated","private": "x".repeat(300000)})
        )
        .unwrap();
        writeln!(f,"{}",json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"parallel"}})).unwrap();
        assert_eq!(
            m.read(&settings, SystemTime::now()).phase(),
            Some(Phase::NeedsInput)
        );
        writeln!(f,"{}",json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"ask"}})).unwrap();
        assert_eq!(
            m.read(&settings, SystemTime::now()).phase(),
            Some(Phase::Working)
        );
        drop(f);
        fs::write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n",
        )
        .unwrap();
        assert_eq!(
            m.read(&settings, SystemTime::now()).phase(),
            Some(Phase::Complete)
        );
    }
    #[test]
    fn remote_clock_skew_and_live_long_tools_are_not_false_failures() {
        let mut reader = Reader::default();
        let mut s = Source {
            provider: "claude".into(),
            id: "remote".into(),
            host_id: Some("ssh".into()),
            ..Default::default()
        };
        let file = FileBatch {
            key: "hook".into(),
            modified: 900,
            reset: true,
            hook: true,
            alive: Some(true),
            events: vec![Event {
                session: "s".into(),
                kind: "PreToolUse".into(),
                at: 900000,
                call: "tool".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = reader.consume(
            &s,
            SourceBatch {
                files: vec![file],
                ..Default::default()
            },
            10000,
            1500,
            "Server",
            Some("installed".into()),
        );
        assert_eq!(result.sessions[0].updated_at, 9400);
        assert_eq!(result.sessions[0].phase, Phase::Tool);
        s.host_id = None;
        let file = FileBatch {
            key: "exit".into(),
            modified: 10000,
            reset: true,
            hook: true,
            events: vec![Event {
                session: "exit".into(),
                kind: "PreToolUse".into(),
                at: 10000000,
                pid: u32::MAX - 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            reader
                .consume(
                    &s,
                    SourceBatch {
                        files: vec![file],
                        ..Default::default()
                    },
                    10000,
                    10000,
                    "本机",
                    None
                )
                .sessions[0]
                .phase,
            Phase::Interrupted
        );
    }
    #[test]
    fn a_waiting_ssh_worker_does_not_block_local_or_another_machine() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("s.jsonl"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        )
        .unwrap();
        let mut settings = Settings {
            sources: vec![source(temp.path(), "codex")],
            ..Default::default()
        };
        let mut monitor = SessionMonitor::default();
        let mut senders = Vec::new();
        for id in ["slow", "ready"] {
            let host = crate::models::Host {
                id: id.into(),
                name: id.into(),
                target: id.into(),
                ..Default::default()
            };
            let s = Source {
                id: id.into(),
                provider: "codex".into(),
                host_id: Some(id.into()),
                path: "/fixture".into(),
                ..Default::default()
            };
            let key = crate::models::hash(&format!(
                "{}:{}",
                serde_json::to_string(&host).unwrap(),
                serde_json::to_string(&vec![s.clone()]).unwrap()
            ));
            let mut worker = RemoteWorker::new(key);
            let (tx, rx) = mpsc::channel();
            worker.pending = Some(rx);
            if id == "ready" {
                tx.send((
                    Reader::default(),
                    Ok(Snapshot {
                        sessions: vec![LiveSession {
                            id: "remote-session".into(),
                            phase: Phase::Tool,
                            updated_at: seconds(SystemTime::now()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                ))
                .unwrap();
            }
            senders.push(tx);
            monitor.remote.insert(id.into(), worker);
            settings.hosts.push(host);
            settings.sources.push(s);
        }
        let start = Instant::now();
        let snapshot = monitor.read(&settings, SystemTime::now());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(snapshot.active_count(), 2);
        assert!(snapshot.unavailable);
    }
    #[test]
    fn hook_background_work_beats_a_premature_transcript_completion() {
        let mut s = Snapshot {
            sessions: vec![
                LiveSession {
                    id: "s".into(),
                    evidence: "hook".into(),
                    phase: Phase::Working,
                    background: true,
                    updated_at: 100,
                    ..Default::default()
                },
                LiveSession {
                    id: "s".into(),
                    evidence: "log".into(),
                    phase: Phase::Complete,
                    updated_at: 101,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.finish();
        assert_eq!(s.sessions.len(), 1);
        assert_eq!(s.phase(), Some(Phase::Working));
    }
    #[test]
    fn database_wait_survives_parallel_hook_returns_in_either_order() {
        let waiting = LiveSession {
            id: "s".into(),
            evidence: "database".into(),
            phase: Phase::NeedsInput,
            updated_at: 100,
            ..Default::default()
        };
        let parallel = LiveSession {
            id: "s".into(),
            evidence: "hook".into(),
            phase: Phase::Working,
            updated_at: 101,
            ..Default::default()
        };
        for sessions in [
            vec![waiting.clone(), parallel.clone()],
            vec![parallel, waiting.clone()],
        ] {
            let mut snapshot = Snapshot {
                sessions,
                ..Default::default()
            };
            snapshot.finish();
            assert_eq!(snapshot.sessions.len(), 1);
            assert_eq!(snapshot.active_count(), 1);
            assert_eq!(snapshot.phase(), Some(Phase::NeedsInput));
        }
        let mut snapshot = Snapshot {
            sessions: vec![
                waiting,
                LiveSession {
                    id: "s".into(),
                    evidence: "hook".into(),
                    phase: Phase::Complete,
                    updated_at: 102,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        snapshot.finish();
        assert_eq!(snapshot.phase(), Some(Phase::Complete));
    }
    #[test]
    fn disconnected_remote_worker_reports_failure_and_accepts_recovery() {
        let host = crate::models::Host {
            id: "ssh".into(),
            name: "Server".into(),
            target: "fixture".into(),
            ..Default::default()
        };
        let source = Source {
            id: "remote".into(),
            provider: "codex".into(),
            host_id: Some(host.id.clone()),
            path: "/fixture".into(),
            ..Default::default()
        };
        let key = crate::models::hash(&format!(
            "{}:{}",
            serde_json::to_string(&host).unwrap(),
            serde_json::to_string(&vec![source.clone()]).unwrap()
        ));
        let settings = Settings {
            hosts: vec![host],
            sources: vec![source],
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut worker = RemoteWorker::new(key);
        worker.pending = Some(rx);
        let mut monitor = SessionMonitor::default();
        monitor.remote.insert("ssh".into(), worker);
        let failed = monitor.read(&settings, SystemTime::now());
        assert!(failed.unavailable);
        assert_eq!(failed.sources[0].state, "degraded");
        assert!(
            failed.sources[0]
                .error
                .as_ref()
                .unwrap()
                .contains("自动重试")
        );
        let worker = monitor.remote.get_mut("ssh").unwrap();
        assert!(worker.pending.is_none());
        assert!(worker.reader.is_some());
        let (tx, rx) = mpsc::channel();
        tx.send((
            Reader::default(),
            Ok(Snapshot {
                sources: vec![SourceHealth {
                    source_id: "remote".into(),
                    state: "healthy".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        ))
        .unwrap();
        worker.pending = Some(rx);
        assert!(!monitor.read(&settings, SystemTime::now()).unavailable);
    }
}
