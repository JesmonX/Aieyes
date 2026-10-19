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
    pub fn key(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Thinking => "thinking",
            Self::Tool => "tool",
            Self::Complete => "complete",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
            Self::NeedsInput => "needsInput",
            Self::Failed => "failed",
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
    #[serde(skip)]
    pub event_at: u64,
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
    pub fn phase_counts(&self) -> std::collections::BTreeMap<String, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for session in &self.sessions {
            *counts.entry(session.phase.key().into()).or_default() += 1;
        }
        counts
    }
    pub fn count_labels(&self) -> Vec<String> {
        let counts = self.phase_counts();
        [
            ("needsInput", "等待"),
            ("failed", "失败"),
            ("working", "工作"),
            ("thinking", "思考"),
            ("tool", "工具"),
            ("complete", "完成"),
            ("interrupted", "中断"),
            ("unknown", "待定"),
        ]
        .into_iter()
        .filter_map(|(key, label)| {
            counts.get(key).map(|count| {
                format!(
                    "{label}{}",
                    if *count > 99 {
                        "99+".into()
                    } else {
                        count.to_string()
                    }
                )
            })
        })
        .collect()
    }
    pub fn summary(&self) -> String {
        let text = match self.phase() {
            Some(_) if self.sessions.len() > 1 => self.count_labels().join(" · "),
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
                    if s.provider == "zcode" {
                        // Stop hooks can request another model step. Only the
                        // settled turn record confirms that this turn ended.
                        let old_settled = old.evidence == "database" && old.phase.terminal();
                        let new_settled = s.evidence == "database" && s.phase.terminal();
                        if !s.turn_id.is_empty() && s.turn_id == old.turn_id {
                            if old_settled && !new_settled {
                                return;
                            }
                            if new_settled && !old_settled {
                                *old = s.clone();
                                return;
                            }
                        }
                        // A new turn and the previous settlement may share the
                        // same second; retain their native millisecond ordering.
                        if s.event_at > 0 && old.event_at > 0 && s.event_at != old.event_at {
                            if s.event_at > old.event_at {
                                *old = s.clone();
                            }
                            return;
                        }
                    }
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
#[derive(Default, Deserialize, Clone)]
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
    database: bool,
    continuation: Value,
    #[cfg(test)]
    read_bytes: u64,
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
    continuation: Value,
}
// FSEvents registration/teardown can wait for its run loop. Keep both off the session lane.
struct DirectoryWatch {
    state: std::sync::Arc<std::sync::atomic::AtomicU8>, // 0 starting, 1 active, 2 failed
    stop: mpsc::Sender<()>,
    changes: mpsc::Receiver<notify::Result<notify::Event>>,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl DirectoryWatch {
    fn new(path: PathBuf) -> Self {
        use notify::Watcher;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8, Ordering},
        };
        let (tx, changes) = mpsc::sync_channel(256);
        let (stop, stopped) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(0));
        let active = state.clone();
        let overflow = Arc::new(AtomicBool::new(false));
        let dropped = overflow.clone();
        let registered = overflow.clone();
        std::thread::spawn(move || {
            let watcher = if local_event_filesystem(&path) {
                notify::recommended_watcher(move |event| {
                    if tx.try_send(event).is_err() {
                        dropped.store(true, Ordering::Release);
                    }
                })
                .ok()
            } else {
                None
            };
            if let Some(mut watcher) = watcher
                && watcher
                    .watch(&path, notify::RecursiveMode::Recursive)
                    .is_ok()
            {
                active.store(1, Ordering::Release);
                registered.store(true, Ordering::Release);
                let _ = stopped.recv();
            } else {
                active.store(2, Ordering::Release);
            }
        });
        Self {
            state,
            stop,
            changes,
            overflow,
        }
    }
    fn active(&self) -> bool {
        self.state.load(std::sync::atomic::Ordering::Acquire) == 1
    }
}
impl Drop for DirectoryWatch {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}
struct Discovery {
    at: u64,
    real_at: Instant,
    roots: Vec<(PathBuf, bool)>,
    paths: Vec<(PathBuf, bool)>,
    error: Option<String>,
    watch: DirectoryWatch,
}
#[derive(Default)]
struct Reader {
    cursors: HashMap<String, Cursor>,
    discovery: HashMap<String, Discovery>,
    hooks: HashMap<String, (Instant, session_hooks::HookStatus)>,
    databases: HashMap<PathBuf, (String, FileBatch)>,
    native_databases: HashMap<
        String,
        (
            PathBuf,
            String,
            Vec<FileBatch>,
            crate::native_sessions::State,
        ),
    >,
    #[cfg(test)]
    database_reads: usize,
    processes: HashMap<(u32, u64), bool>,
}
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn local_session_path(path: &Path) -> PathBuf {
    // WalkDir and watcher backends may disagree on Windows' verbatim prefix or
    // symlink/relative components. Use one identity for discovery and events.
    dunce::canonicalize(path).unwrap_or_else(|_| dunce::simplified(path).to_owned())
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
        Value::Object(self.cursors.iter().map(|(p,c)|(p.clone(),json!({"identity":c.identity,"signature":c.signature,"size":c.size,"offset":c.offset,"modified":c.modified,"pid":c.activity.pid,"processStart":c.activity.process_start,"continuation":c.continuation}))).collect())
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
            if source.provider == "zcode" && f.hook && hook_state.as_deref() != Some("installed") {
                continue;
            }
            let cursor = self.cursors.entry(f.key.clone()).or_default();
            if f.reset {
                cursor.activity = Activity::default();
            }
            if cursor.activity.session.is_empty() {
                cursor.activity.session = if matches!(source.provider.as_str(), "kimi" | "grok") {
                    crate::native_agents::session_id(Path::new(&f.key))
                } else {
                    Path::new(&f.key)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
            }
            let database = f.database || f.steps.is_some();
            if let Some(steps) = f.steps {
                let (phase, turn) = session_events::agy_steps(&steps);
                cursor.activity.phase = phase;
                cursor.activity.turn = turn;
                cursor.activity.session = f.session;
            } else {
                for mut event in f.events {
                    if source.provider == "zcode" && f.hook && event.kind == "Stop" {
                        event.kind = "PostInvocation".into();
                    }
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
            cursor.continuation = f.continuation;
            let Some(mut phase) = cursor.activity.phase else {
                continue;
            };
            let elapsed = remote_now.saturating_sub(f.modified);
            let alive = if source.host_id.is_none() && cursor.activity.pid > 0 {
                Some(
                    *self
                        .processes
                        .entry((cursor.activity.pid, cursor.activity.process_start))
                        .or_insert_with(|| {
                            process_alive(cursor.activity.pid, cursor.activity.process_start)
                        }),
                )
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
                event_at: cursor.activity.at,
            });
        }
        let error = batch.error.or_else(|| {
            if source.provider == "cursor" {
                return (hook_state.as_deref()!=Some("installed")).then(|| "Cursor 实时监测未启用或配置不可用；原生私有会话库不能替代状态事件，请在接入与账户中启用或修复监测".into());
            }
            (hook_state.as_deref() == Some("error"))
                .then(|| "实时会话监测未就绪，暂从日志读取；可在接入与账户中修复监测".into())
        });
        if source.provider == "cursor" && error.is_some() {
            for session in &mut out.sessions {
                if session.phase.active() {
                    session.phase = Phase::Unknown;
                }
            }
        }
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
        self.processes.clear();
        let keys: HashSet<_> = sources
            .iter()
            .flat_map(|s| {
                [
                    format!("{}:{}:false", s.provider, s.path),
                    format!("{}:{}:true", s.provider, s.path),
                ]
            })
            .collect();
        self.discovery.retain(|key, _| keys.contains(key));
        let mut out = Snapshot::default();
        let mut retained = HashSet::new();
        for source in sources {
            let hook = if session_hooks::supported(&source.provider) {
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
            retained.extend(batch.files.iter().map(|f| PathBuf::from(&f.key)));
            let snapshot = self.consume(source, batch, now, now, "本机", hook);
            out.sessions.extend(snapshot.sessions);
            out.sources.extend(snapshot.sources);
        }
        retained.extend(
            self.discovery
                .values()
                .flat_map(|d| d.paths.iter().map(|(p, _)| p.clone())),
        );
        self.databases.retain(|path, _| retained.contains(path));
        self.native_databases
            .retain(|id, _| sources.iter().any(|s| s.id == *id));
        self.cursors
            .retain(|path, _| retained.contains(Path::new(path)));
        out
    }
    fn collect_local(&mut self, source: &Source, now: u64) -> SourceBatch {
        if matches!(source.provider.as_str(), "zcode" | "opencode")
            && let Some(path) = if source.provider == "zcode" {
                crate::zcode::database(&source.path)
            } else {
                crate::opencode::database(&source.path)
            }
        {
            let before = database_fingerprint(&path);
            let cached = self
                .native_databases
                .get(&source.id)
                .filter(|(p, f, _, _)| *p == path && *f == before);
            let mut batch = if let Some((_, _, files, _)) = cached {
                SourceBatch {
                    source_id: source.id.clone(),
                    files: files.clone(),
                    ..Default::default()
                }
            } else {
                #[cfg(test)]
                {
                    self.database_reads += 1;
                }
                let prior = self
                    .native_databases
                    .get(&source.id)
                    .filter(|(p, _, _, _)| *p == path)
                    .map(|(_, _, _, state)| state.clone())
                    .unwrap_or_default();
                match crate::native_sessions::refresh(&source.provider, &path, prior, now) {
                    Ok((state, rows)) => {
                        let files: Vec<_> = rows
                            .into_iter()
                            .map(|(session, turn, kind, at)| FileBatch {
                                key: format!("{}#{session}", path.display()),
                                modified: at / 1000,
                                reset: true,
                                database: true,
                                events: vec![
                                    Event {
                                        kind: "start".into(),
                                        session: session.clone(),
                                        turn: turn.clone(),
                                        at,
                                        ..Default::default()
                                    },
                                    Event {
                                        kind,
                                        session,
                                        turn,
                                        at,
                                        ..Default::default()
                                    },
                                ],
                                ..Default::default()
                            })
                            .collect();
                        let mut cached = files.clone();
                        for file in &mut cached {
                            file.reset = false;
                            file.events.clear();
                        }
                        self.native_databases.insert(
                            source.id.clone(),
                            (
                                path.clone(),
                                if before == database_fingerprint(&path) {
                                    before
                                } else {
                                    String::new()
                                },
                                cached,
                                state,
                            ),
                        );
                        SourceBatch {
                            source_id: source.id.clone(),
                            files,
                            ..Default::default()
                        }
                    }
                    Err(error) => SourceBatch {
                        source_id: source.id.clone(),
                        error: Some(error.to_string()),
                        files: self
                            .native_databases
                            .get(&source.id)
                            .map(|(_, _, files, _)| files.clone())
                            .unwrap_or_default(),
                    },
                }
            };
            batch
                .files
                .retain(|f| now.saturating_sub(f.modified) < 86400);
            if source.provider == "zcode" {
                let hooks = self.collect_logs_local(source, now, true);
                batch.files.extend(hooks.files);
                if hooks.error.is_some() {
                    batch.error = hooks.error;
                }
            }
            return batch;
        }
        self.collect_logs_local(source, now, false)
    }
    fn collect_logs_local(&mut self, source: &Source, now: u64, only_hooks: bool) -> SourceBatch {
        use std::sync::atomic::Ordering;
        let key = format!("{}:{}:{only_hooks}", source.provider, source.path);
        let mut lost = false;
        if let Some(discovery) = self.discovery.get_mut(&key) {
            lost = discovery.watch.overflow.swap(false, Ordering::AcqRel);
            while let Ok(event) = discovery.watch.changes.try_recv() {
                match event {
                    Ok(event) => {
                        if matches!(event.kind, notify::EventKind::Access(_)) {
                            continue;
                        }
                        if event.need_rescan() {
                            lost = true;
                        }
                        for path in event.paths {
                            let mut path = local_session_path(&path);
                            if path.is_dir() {
                                lost = true;
                                continue;
                            }
                            if source.provider == "antigravity"
                                && path.to_string_lossy().ends_with("-wal")
                            {
                                path =
                                    PathBuf::from(path.to_string_lossy().trim_end_matches("-wal"));
                            }
                            // Use the same roots as discovery: root-level AGY summary
                            // databases are not conversations. Hooks take precedence
                            // when discovery falls back to scanning the provider home.
                            let Some((_, hook)) = discovery
                                .roots
                                .iter()
                                .rev()
                                .find(|(root, _)| path.starts_with(root))
                            else {
                                continue;
                            };
                            let hook = *hook;
                            let extension = if !hook && source.provider == "antigravity" {
                                "db"
                            } else {
                                "jsonl"
                            };
                            if !native_log_path(&path, &source.provider, hook, extension)
                                || !hook && path.components().any(|c| c.as_os_str() == ".aieyes")
                            {
                                continue;
                            }
                            discovery.paths.retain(|(p, _)| p != &path);
                            if path.is_file() && now.saturating_sub(modified(&path)) < 86400 {
                                discovery.paths.push((path, hook));
                            }
                        }
                    }
                    Err(_) => {
                        lost = true;
                        discovery.watch.state.store(2, Ordering::Release);
                    }
                }
            }
            discovery
                .paths
                .retain(|(p, _)| now.saturating_sub(modified(p)) < 86400);
            discovery
                .paths
                .sort_by_key(|(p, _)| std::cmp::Reverse(modified(p)));
            discovery.paths.truncate(256);
        }
        if self.discovery.get(&key).is_none_or(|d| {
            lost || now < d.at
                || now.saturating_sub(d.at) > d.real_at.elapsed().as_secs() + 2
                || now.saturating_sub(d.at) >= if d.watch.active() { 60 } else { 5 }
        }) {
            let root = expand(&source.path);
            let root = local_session_path(&root);
            let mut paths = Vec::new();
            let mut error = None;
            if !root.exists() {
                error = Some("Agent 数据目录不存在".into());
            }
            let nested = root.join(match source.provider.as_str() {
                "codex" | "kimi" | "deepseek-harness" | "grok" | "pi" | "omp" => "sessions",
                "claude" | "codebuddy" | "qoder" => "projects",
                _ => "conversations",
            });
            let scan_root = if nested.is_dir() { nested } else { root };
            let mut roots = if only_hooks || source.provider == "cursor" {
                Vec::new()
            } else {
                vec![(local_session_path(&scan_root), false)]
            };
            if session_hooks::supported(&source.provider) {
                let events = session_hooks::event_root(source);
                if events.is_dir() {
                    roots.push((local_session_path(&events), true));
                }
            }
            // Reconciliation reuses the watcher; a failed backend retries on the five-second fallback.
            let watch = self
                .discovery
                .remove(&key)
                .map(|d| d.watch)
                .filter(|w| w.state.load(Ordering::Acquire) != 2)
                .unwrap_or_else(|| DirectoryWatch::new(expand(&source.path)));
            for (root, hook) in &roots {
                let hook = *hook;
                if !hook && source.provider == "deepseek-harness" {
                    match crate::harness_history::paths(root) {
                        Ok(files) => paths.extend(
                            files
                                .into_iter()
                                .filter(|p| now.saturating_sub(modified(p)) < 86400)
                                .map(|p| (p, false)),
                        ),
                        Err(e) => error = Some(e.to_string()),
                    }
                    continue;
                }
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
                        || !native_log_path(entry.path(), &source.provider, hook, extension)
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
                    real_at: Instant::now(),
                    roots,
                    paths,
                    error,
                    watch,
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
            let result = if !hook
                && matches!(source.provider.as_str(), "antigravity" | "deepseek-harness")
            {
                let before = database_fingerprint(path);
                if let Some((known, cached)) = self
                    .databases
                    .get(path)
                    .filter(|(known, _)| *known == before)
                {
                    let _ = known;
                    Ok(cached.clone())
                } else {
                    #[cfg(test)]
                    {
                        self.database_reads += 1;
                    }
                    let read = if source.provider == "deepseek-harness" {
                        read_dsh_session(path, self.cursors.get(&name))
                    } else {
                        read_db(path)
                    };
                    read.and_then(|row| {
                        anyhow::ensure!(
                            before == database_fingerprint(path),
                            "会话数据库读取期间发生变化，将重试"
                        );
                        let mut cached = row.clone();
                        cached.steps = None;
                        cached.reset = false;
                        cached.events.clear();
                        self.databases.insert(path.clone(), (before, cached));
                        Ok(row)
                    })
                }
            } else {
                read_file(path, &source.provider, *hook, self.cursors.get(&name))
                    .map_err(anyhow::Error::from)
            };
            match result {
                Ok(f) => batch.files.push(f),
                Err(e) => {
                    if !hook
                        && matches!(source.provider.as_str(), "antigravity" | "deepseek-harness")
                        && let Some((_, cached)) = self.databases.get(path)
                    {
                        batch.files.push(cached.clone());
                    }
                    batch.error = Some(e.to_string());
                }
            }
        }
        batch
    }
}
fn native_log_path(path: &Path, provider: &str, hook: bool, extension: &str) -> bool {
    if !hook && provider == "zcode" {
        return false;
    }
    if !hook && provider == "grok" {
        return path.file_name().is_some_and(|n| n == "events.jsonl");
    }
    if !hook && provider == "kimi" {
        return path.file_name().is_some_and(|n| n == "wire.jsonl");
    }
    if !hook && provider == "deepseek-harness" {
        return crate::native_agents::is_dsh_log(path);
    }
    path.extension().is_some_and(|e| e == extension)
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HarnessSessionCursor {
    prefix: String,
    offset: u64,
    checkpoint: crate::models::HarnessCheckpoint,
    inherited: bool,
}

fn read_dsh_session(path: &Path, previous: Option<&Cursor>) -> anyhow::Result<FileBatch> {
    use crate::native_agents::{read_dsh_from, region_digest};
    use std::io::{Read, Seek, SeekFrom};
    let mut state: HarnessSessionCursor = previous
        .and_then(|c| serde_json::from_value(c.continuation.clone()).ok())
        .unwrap_or_default();
    let mut file = fs::File::open(path)?;
    let meta = file.metadata()?;
    let size = meta.len();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created());
    let prefix = region_digest(&mut file, 0, size.min(128))?;
    let resume = state.offset > 0
        && state.offset <= size
        && state.prefix == prefix
        && state.checkpoint.identity == identity
        && size > state.checkpoint.size
        && region_digest(
            &mut file,
            state.offset.saturating_sub(128),
            state.offset.min(128),
        )? == state.checkpoint.anchor;
    if !resume {
        state = HarnessSessionCursor::default();
    }
    let offset = state.offset;
    let fingerprint = database_fingerprint(path);
    file.seek(SeekFrom::Start(offset))?;
    let mut events = std::collections::VecDeque::new();
    let mut header = None;
    let (version, seed, plain_bytes) = read_dsh_from(
        path,
        file.take(size - offset),
        offset,
        state.checkpoint.version,
        state.checkpoint.seed,
        |row| {
            if row["type"] == "session" {
                header = session_events::native("deepseek-harness", &row);
                state.inherited = row["isSeeded"] == true;
                return Ok(());
            }
            if row["type"] == "session/end-seed" {
                state.inherited = false;
            }
            if !state.inherited
                && let Some(event) = session_events::native("deepseek-harness", &row)
            {
                if events.len() >= 2048 {
                    events.pop_front();
                }
                events.push_back(event);
            }
            Ok(())
        },
    )?;
    anyhow::ensure!(
        fingerprint == database_fingerprint(path),
        "Harness 会话读取期间发生变化，将重试"
    );
    if let Some(header) = header {
        events.push_front(header);
    }
    state.offset = if path.extension().is_some_and(|e| e == "zstd") {
        size
    } else {
        offset + plain_bytes
    };
    state.prefix = prefix;
    state.checkpoint = crate::models::HarnessCheckpoint {
        fingerprint: fingerprint.clone(),
        identity: identity.clone(),
        anchor: region_digest(
            &mut fs::File::open(path)?,
            state.offset.saturating_sub(128),
            state.offset.min(128),
        )?,
        size,
        version,
        seed,
    };
    Ok(FileBatch {
        key: path.to_string_lossy().into_owned(),
        identity,
        signature: fingerprint,
        size,
        offset: state.offset,
        modified: modified(path),
        reset: !resume,
        events: events.into(),
        continuation: serde_json::to_value(state)?,
        #[cfg(test)]
        read_bytes: size - offset,
        ..Default::default()
    })
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
    if reset && matches!(provider, "codex" | "pi" | "omp") {
        file.seek(SeekFrom::Start(0))?;
        let mut first = Vec::new();
        (&mut file).take(65536).read_to_end(&mut first)?;
        for line in first.split(|b| *b == b'\n').take(2) {
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                let rows = if provider == "codex" {
                    session_events::codex(&v).into_iter().collect()
                } else {
                    session_events::pi(&v)
                };
                events.extend(
                    rows.into_iter()
                        .filter(|e| e.kind == "identity" && !e.session.is_empty()),
                );
            }
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
        } else if matches!(provider, "kimi" | "codebuddy" | "grok") {
            if provider == "kimi" && value["type"] == "forked" {
                events.clear();
            }
            events.extend(session_events::native(provider, &value));
        } else if matches!(provider, "pi" | "omp") {
            events.extend(session_events::pi(&value));
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
        database: true,
        ..Default::default()
    })
}
pub(crate) fn database_fingerprint(path: &Path) -> String {
    [
        path.to_owned(),
        PathBuf::from(format!("{}-wal", path.display())),
    ]
    .iter()
    .map(|p| {
        fs::metadata(p)
            .map(|m| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    format!(
                        "{}:{:?}:{}:{}:{}:{}",
                        m.len(),
                        m.modified(),
                        m.dev(),
                        m.ino(),
                        m.ctime(),
                        m.ctime_nsec()
                    )
                }
                #[cfg(not(unix))]
                {
                    format!("{}:{:?}:{:?}", m.len(), m.modified(), m.created())
                }
            })
            .unwrap_or_default()
    })
    .collect::<Vec<_>>()
    .join("|")
}
fn local_event_filesystem(path: &Path) -> bool {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // statfs initializes the output on success; the path CString outlives this call.
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return false;
        }
        let info = unsafe { info.assume_init() };
        #[cfg(target_os = "macos")]
        {
            let kind = unsafe { std::ffi::CStr::from_ptr(info.f_fstypename.as_ptr()) }.to_bytes();
            matches!(kind, b"apfs" | b"hfs" | b"msdos" | b"exfat" | b"tmpfs")
        }
        #[cfg(target_os = "linux")]
        {
            matches!(
                info.f_type as u64,
                0xef53 | 0x9123683e | 0x1021994 | 0x58465342 | 0x794c7630
            )
        }
    }
    #[cfg(windows)]
    {
        !path.to_string_lossy().starts_with("\\\\")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = path;
        false
    }
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
                s.subscription.is_none()
                    && settings.source_enabled(s)
                    && matches!(
                        s.provider.as_str(),
                        "codex"
                            | "claude"
                            | "antigravity"
                            | "kimi"
                            | "codebuddy"
                            | "deepseek-harness"
                            | "grok"
                            | "zcode"
                            | "opencode"
                            | "qoder"
                            | "pi"
                            | "omp"
                            | "cursor"
                    )
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
                        let value=crate::ssh::session_request(&host,json!({"action":"collect","databaseCache":true,"sources":sources.iter().map(wire_source).collect::<Vec<_>>(),"cursors":reader.manifest()}))?;
                        let remote_now=value["now"].as_u64().unwrap_or(now);let observed=seconds(SystemTime::now());
                        let mut snapshot=Snapshot::default();let polls:Vec<SourceBatch>=serde_json::from_value(value["polls"].clone())?;
                        let complete=polls.len()==sources.len() && polls.iter().all(|p|p.error.is_none());
                        let retained:HashSet<_>=polls.iter().flat_map(|p|p.files.iter().map(|f|f.key.clone())).collect();
                        for batch in polls {if let Some(source)=sources.iter().find(|s|s.id==batch.source_id) {
                            let hook=value["hooks"].as_array().and_then(|rows|rows.iter().find(|v|v["sourceId"]==source.id)).and_then(|v|v["state"].as_str()).map(String::from);
                            let state=reader.consume(source,batch,observed,remote_now,&host.name,hook);
                            snapshot.sessions.extend(state.sessions);snapshot.sources.extend(state.sources);
                        }}if complete { reader.cursors.retain(|key,_|retained.contains(key)); }snapshot.finish();Ok(snapshot)
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
    #[test]
    fn harness_session_state_resumes_at_validated_eof_and_retries_torn_frames() {
        for compressed in [false, true] {
            for count in [10, 3000] {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join(if compressed {
                    "session.v4.jsonl.zstd"
                } else {
                    "session.v4.jsonl"
                });
                let now = seconds(SystemTime::now());
                let mut content =
                    json!({"type":"session","version":4,"id":"native-session","isSeeded":true})
                        .to_string()
                        + "\n";
                for index in 0..count {
                    content += &format!(
                        "{}\n",
                        json!({"type":"turn/end","time":now*1000,"seq":index,"data":{"turn":index,"reason":{"kind":"completed"}},"content":format!("PRIVATE-{index}")})
                    );
                }
                let encode = |raw: &str| {
                    if compressed {
                        zstd::stream::encode_all(raw.as_bytes(), 1).unwrap()
                    } else {
                        raw.as_bytes().to_vec()
                    }
                };
                let initial = encode(&content);
                fs::write(&path, &initial).unwrap();
                let first = read_dsh_session(&path, None).unwrap();
                assert_eq!(first.read_bytes, initial.len() as u64);
                assert_eq!(
                    first.events.len(),
                    1,
                    "Inherited turns must not change live state"
                );
                let prior = Cursor {
                    continuation: first.continuation,
                    ..Default::default()
                };
                let added = encode(&format!(
                    "{{\"type\":\"session/end-seed\"}}\n{}\n",
                    json!({"type":"turn/start","time":now*1000,"data":{"turn":count}})
                ));
                if compressed {
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .unwrap()
                        .write_all(&added[..added.len() - 1])
                        .unwrap();
                    assert!(read_dsh_session(&path, Some(&prior)).is_err());
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .unwrap()
                        .write_all(&added[added.len() - 1..])
                        .unwrap();
                } else {
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .unwrap()
                        .write_all(&added)
                        .unwrap();
                }
                let next = read_dsh_session(&path, Some(&prior)).unwrap();
                assert!(!next.reset);
                assert_eq!(
                    next.read_bytes,
                    added.len() as u64,
                    "Prior history size {count} must not affect appended state reads"
                );
                assert_eq!(next.events.len(), 1);
                assert_eq!(next.events[0].kind, "start");
                assert!(!next.continuation.to_string().contains("PRIVATE"));
            }
        }
    }
    #[test]
    fn zcode_live_hooks_resume_until_settlement_and_respect_disabled_monitoring() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path(), "zcode");
        let path = temp.path().join("cli/db/db.sqlite");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE turn_usage(session_id TEXT,turn_id TEXT,status TEXT,started_at INTEGER,completed_at INTEGER);CREATE TABLE model_usage(session_id TEXT,turn_id TEXT,status TEXT);CREATE TABLE tool_usage(session_id TEXT,turn_id TEXT,status TEXT);").unwrap();
        let root = session_hooks::event_root(&source);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("session.jsonl");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let append = |kind: &str, turn: &str, offset: u64| {
            writeln!(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&file)
                    .unwrap(),
                "{}",
                json!({"kind":kind,"session":"s","turn":turn,"at":now*1000+offset})
            )
            .unwrap();
        };
        append("UserPromptSubmit", "t", 0);
        let mut reader = Reader::default();
        let key = session_hooks::key(&source);
        reader.hooks.insert(
            key.clone(),
            (
                Instant::now(),
                session_hooks::HookStatus {
                    source_id: source.id.clone(),
                    provider: "zcode".into(),
                    machine_id: "local".into(),
                    config_path: String::new(),
                    state: "installed".into(),
                    error: None,
                },
            ),
        );
        let poll = |reader: &mut Reader| {
            let mut snapshot = reader.local(std::slice::from_ref(&source), now);
            snapshot.finish();
            assert!(!snapshot.unavailable);
            snapshot
        };
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Working));
        assert!(
            reader
                .cursors
                .contains_key(local_session_path(&file).to_str().unwrap())
        );
        assert_eq!(reader.database_reads, 1);
        assert!(
            reader.collect_local(&source, now).files[0]
                .events
                .is_empty(),
            "An unchanged hook file must retain its incremental cursor"
        );
        assert_eq!(
            reader.database_reads, 1,
            "Unchanged native statistics must not be queried again"
        );
        append("Stop", "t", 1);
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Working));
        append("PreToolUse", "t", 2);
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Tool));
        db.execute(
            "INSERT INTO turn_usage VALUES('s','t','completed',?1,?2)",
            [now * 1000, now * 1000 + 3],
        )
        .unwrap();
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Complete));
        // The old failure and a new prompt share a second. The older turn
        // must not win solely because failures have a higher UI priority.
        db.execute("UPDATE turn_usage SET status='error'", [])
            .unwrap();
        append("UserPromptSubmit", "next", 4);
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Working));
        reader.hooks.get_mut(&key).unwrap().1.state = "disabled".into();
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Failed));
        reader.hooks.get_mut(&key).unwrap().1.state = "installed".into();
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Working));
        db.execute(
            "UPDATE turn_usage SET turn_id='next',status='future-status',completed_at=?1",
            [now * 1000 + 5],
        )
        .unwrap();
        assert_eq!(poll(&mut reader).phase(), Some(Phase::Unknown));
    }
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
    fn unchanged_database_is_not_reopened_and_wal_and_replacement_invalidate() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("a.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('a');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,step_payload TEXT);INSERT INTO steps VALUES(0,14,3,'');").unwrap();
        let source = source(temp.path(), "antigravity");
        let mut reader = Reader::default();
        let at = seconds(SystemTime::now());
        assert_eq!(reader.collect_local(&source, at).files.len(), 1);
        assert_eq!(reader.database_reads, 1);
        for offset in 0..10 {
            assert_eq!(reader.collect_local(&source, at + offset).files.len(), 1);
        }
        assert_eq!(
            reader.database_reads, 1,
            "No-change rounds must not reopen the database"
        );
        db.execute("INSERT INTO steps VALUES(1,21,9,'')", [])
            .unwrap();
        assert!(
            reader.collect_local(&source, at + 10).files[0]
                .steps
                .is_some()
        );
        assert_eq!(reader.database_reads, 2);
        drop(db);
        let replacement = dir.join("new.db");
        let db = rusqlite::Connection::open(&replacement).unwrap();
        db.execute_batch("CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('b');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,step_payload TEXT);").unwrap();
        drop(db);
        fs::rename(replacement, &path).unwrap();
        reader.collect_local(&source, at + 11);
        assert_eq!(reader.database_reads, 3);
    }
    #[test]
    fn watcher_path_aliases_share_one_database_cache_entry() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8},
        };
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("a.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('a');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER);INSERT INTO steps VALUES(0,14,3);").unwrap();
        let source = source(temp.path(), "antigravity");
        let mut reader = Reader::default();
        let at = seconds(SystemTime::now());
        assert_eq!(reader.collect_local(&source, at).files.len(), 1);

        // Inject the alternate representations reported by watcher backends,
        // independently of whether the native watcher has registered yet.
        let (events, changes) = mpsc::channel();
        let (stop, _stopped) = mpsc::channel();
        reader.discovery.values_mut().next().unwrap().watch = DirectoryWatch {
            state: Arc::new(AtomicU8::new(1)),
            stop,
            changes,
            overflow: Arc::new(AtomicBool::new(false)),
        };
        db.execute("INSERT INTO steps VALUES(1,21,9)", []).unwrap();
        events
            .send(Ok(notify::Event::new(notify::EventKind::Modify(
                notify::event::ModifyKind::Any,
            ))
            .add_path(dir.join("..").join("conversations/a.db"))
            .add_path(fs::canonicalize(&path).unwrap())
            .add_path(dunce::simplified(&path).to_owned())))
            .unwrap();
        let batch = reader.collect_local(&source, at + 1);
        assert_eq!(
            batch.files.len(),
            1,
            "Path aliases must not duplicate sessions"
        );
        assert_eq!(
            reader.database_reads, 2,
            "Read the changed database only once"
        );
        assert!(batch.files[0].steps.is_some());
        assert_eq!(reader.collect_local(&source, at + 2).files.len(), 1);
        assert_eq!(
            reader.database_reads, 2,
            "Retain the normalized cache identity"
        );
    }
    #[test]
    fn watcher_ignores_agy_summary_database_and_keeps_sessions_and_hooks() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8},
        };
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        fs::create_dir(&dir).unwrap();
        hooks_only(temp.path());
        let path = dir.join("a.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('a');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER);INSERT INTO steps VALUES(0,14,3);").unwrap();
        drop(db);
        let summary = temp.path().join("conversation_summaries.db");
        let db = rusqlite::Connection::open(&summary).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE conversation_summaries(id TEXT);")
            .unwrap();
        let source = source(temp.path(), "antigravity");
        let mut reader = Reader::default();
        let at = seconds(SystemTime::now());
        let initial = reader.collect_local(&source, at);
        assert!(initial.error.is_none());
        assert_eq!(initial.files.len(), 1);

        // Inject file notifications so the regression runs on every watcher backend.
        let (events, changes) = mpsc::channel();
        let (stop, _stopped) = mpsc::channel();
        reader.discovery.values_mut().next().unwrap().watch = DirectoryWatch {
            state: Arc::new(AtomicU8::new(1)),
            stop,
            changes,
            overflow: Arc::new(AtomicBool::new(false)),
        };
        let new_session = dir.join("b.db");
        fs::copy(&path, &new_session).unwrap();
        let new_db = rusqlite::Connection::open(&new_session).unwrap();
        new_db
            .execute_batch("PRAGMA journal_mode=WAL;INSERT INTO steps VALUES(1,21,9);")
            .unwrap();
        let hook = session_hooks::event_root(&source).join("event.jsonl");
        fs::write(&hook, b"{}\n").unwrap();
        db.execute("INSERT INTO conversation_summaries VALUES('updated')", [])
            .unwrap();
        events
            .send(Ok(notify::Event::new(notify::EventKind::Modify(
                notify::event::ModifyKind::Any,
            ))
            .add_path(summary.clone())
            .add_path(PathBuf::from(format!("{}-wal", summary.display())))
            .add_path(PathBuf::from(format!("{}-wal", new_session.display())))
            .add_path(hook)))
            .unwrap();
        let batch = reader.collect_local(&source, at + 1);
        assert!(batch.error.is_none(), "{:?}", batch.error);
        assert_eq!(batch.files.len(), 3);
        assert_eq!(reader.database_reads, 2, "Only read conversation databases");
        assert!(batch.files.iter().any(|file| file.hook));
        assert!(batch.files.iter().any(|file| {
            Path::new(&file.key) == local_session_path(&new_session) && file.database
        }));
    }
    #[test]
    fn failed_database_refresh_keeps_last_successful_activity_with_degraded_health() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        fs::create_dir(&dir).unwrap();
        hooks_only(temp.path());
        let path = dir.join("a.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('a');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER);INSERT INTO steps VALUES(0,14,3);").unwrap();
        drop(db);
        let source = source(temp.path(), "antigravity");
        let mut reader = Reader::default();
        let at = seconds(SystemTime::now());
        let first = reader.local(std::slice::from_ref(&source), at);
        assert_eq!(first.sessions.len(), 1);
        fs::write(path, b"invalid sqlite").unwrap();
        let next = reader.local(&[source], at + 1);
        assert_eq!(next.sessions.len(), 1);
        assert_eq!(next.sessions[0].id, first.sessions[0].id);
        assert!(next.sources[0].error.is_some());
    }
    #[test]
    fn watcher_events_and_overflow_reconcile_new_files_without_blocking_the_lane() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path(), "codex");
        let mut reader = Reader::default();
        let at = seconds(SystemTime::now());
        let start = Instant::now();
        reader.collect_local(&source, at);
        assert!(start.elapsed() < Duration::from_millis(500));
        fs::write(temp.path().join("new.jsonl"), b"{}\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if !reader
                .collect_local(&source, at + start.elapsed().as_secs())
                .files
                .is_empty()
            {
                break;
            }
            if reader
                .discovery
                .values()
                .next()
                .unwrap()
                .watch
                .state
                .load(std::sync::atomic::Ordering::Acquire)
                == 2
            {
                assert_eq!(
                    reader.collect_local(&source, at + 5).files.len(),
                    1,
                    "Unsupported backends retain five-second reconciliation"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "New paths not reconciled: state={} local={} candidates={:?} error={:?}",
                reader
                    .discovery
                    .values()
                    .next()
                    .unwrap()
                    .watch
                    .state
                    .load(std::sync::atomic::Ordering::Acquire),
                local_event_filesystem(temp.path()),
                reader.discovery.values().next().unwrap().paths,
                reader.discovery.values().next().unwrap().error
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(temp.path().join("overflow.jsonl"), b"{}\n").unwrap();
        reader
            .discovery
            .values()
            .next()
            .unwrap()
            .watch
            .overflow
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(reader.collect_local(&source, at).files.len(), 2);
        reader
            .discovery
            .values()
            .next()
            .unwrap()
            .watch
            .state
            .store(2, std::sync::atomic::Ordering::Release);
        fs::write(temp.path().join("fallback.jsonl"), b"{}\n").unwrap();
        assert_eq!(reader.collect_local(&source, at + 5).files.len(), 3);
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
