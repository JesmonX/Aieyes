//! On-demand desktop interaction for sessions waiting for user input.
//! Reuses the verified session adapters (Codex app-server, OpenCode loopback HTTP,
//! Claude/Qoder PermissionRequest hooks, bounded transcript readers, tmux and macOS terminal)
//! independently of the Feishu chat channel. Prompts are inspected on demand and never archived.
//!
//! `contract` holds the wire types every adapter fills in; docs/session-interaction.md is the
//! client-facing description. Safety rule for every path here: a reply is delivered only to the
//! request it was written for, and anything that cannot be verified is refused, not guessed.
pub(crate) mod contract;

use crate::{
    channels::{Session, claude, codex, content, control, opencode},
    models::{Source, now},
    sessions::{LiveSession, Phase, Snapshot},
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use contract::{Approval, Capabilities, FollowUp, Verb};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

const CONFIG_KEY: &str = "desktop.interaction.v1";
const HEARTBEAT_KEY: &str = "interaction.heartbeat";
const ANSWERED_PREFIX: &str = "interaction.answered:";
/// Seconds an answered request hides the non-answerable card that the lagging session state
/// would otherwise show (Codex re-probes approvals every 2 s; transcripts trail hooks).
const ANSWERED_TTL: i64 = 12;
const RETENTION: i64 = 86_400;
const TERMINAL_FRESH: i64 = 20;
pub(crate) const CODEX_APPROVAL_INPUT: &str = "codex:approval";

static LAST_HEARTBEAT: AtomicI64 = AtomicI64::new(0);
static LAST_HOOK_SYNC: AtomicI64 = AtomicI64::new(0);
static LAST_CLEANUP: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub auto_popup: bool,
    pub claude_questions: bool,
    pub claude_approvals: bool,
    pub kiro_approvals: bool,
    pub allow_terminal: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_popup: false,
            claude_questions: false,
            claude_approvals: false,
            kiro_approvals: false,
            allow_terminal: false,
        }
    }
}

impl Config {
    pub(crate) fn approvals_for(&self, provider: &str) -> bool {
        self.enabled
            && if provider == "kiro" {
                self.kiro_approvals
            } else {
                self.claude_approvals
            }
    }
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS interaction_sessions (
            id TEXT PRIMARY KEY,
            state_key TEXT NOT NULL,
            observed_at INTEGER NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS interaction_sessions_recent ON interaction_sessions(observed_at);",
    )?;
    Ok(())
}

pub(crate) fn config(db: &Connection) -> Result<Config> {
    let raw: Option<String> = db
        .query_row("SELECT value FROM kv WHERE key=?1", [CONFIG_KEY], |r| {
            r.get(0)
        })
        .optional()?;
    raw.map(|v| serde_json::from_str(&v).context("无法读取桌面交互配置"))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn save_config(db: &Connection, cfg: &Config) -> Result<()> {
    db.execute(
        "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![CONFIG_KEY, serde_json::to_string(cfg)?],
    )?;
    Ok(())
}

pub(crate) fn state_key(s: &LiveSession) -> String {
    if s.phase == Phase::NeedsInput && !s.input_request.is_empty() {
        waiting_key(&s.turn_id, &s.input_request)
    } else {
        format!(
            "{}:{}:{}",
            s.phase.key(),
            s.turn_id,
            if s.phase == Phase::NeedsInput || s.turn_id.is_empty() {
                s.event_at.max(s.updated_at.saturating_mul(1000))
            } else {
                s.turn_started_at
            }
        )
    }
}

fn waiting_key(turn: &str, request: &str) -> String {
    format!("needsInput:{turn}:{}", crate::models::hash(request))
}

fn is_codex_approval(target: &Session) -> bool {
    target.state_key == waiting_key(&target.session.turn_id, CODEX_APPROVAL_INPUT)
}

pub(crate) fn session(db: &Connection, id: &str) -> Result<Option<Session>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT payload FROM interaction_sessions WHERE id=?1 AND observed_at>?2",
            params![id, now() - RETENTION],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|v| serde_json::from_str(&v).context("无法读取会话控制信息"))
        .transpose()
}

pub(crate) fn touch_heartbeat(store: &Store) {
    let at = now();
    let _ = store.db.execute(
        "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![HEARTBEAT_KEY, json!({"at": at}).to_string()],
    );
    LAST_HEARTBEAT.store(at, Ordering::Relaxed);
}

/// The oldest open interaction-hook request of a native session; identity and kind only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookWait {
    pub id: String,
    pub kind: String,
    /// Seconds.
    pub at: i64,
    pub process: crate::sessions::ControlProcess,
}

/// Open interaction-hook requests, keyed by `(source id, native session)`.
pub(crate) fn pending_hook_requests(db: &Connection) -> HashMap<(String, String), HookWait> {
    let mut out = HashMap::new();
    let Ok(mut stmt) = db.prepare(
        "SELECT source_id,native,id,kind,at,pid,process_start FROM channel_claude_requests \
         WHERE consumed=0 AND response IS NULL AND heartbeat>?1 ORDER BY at DESC, rowid DESC",
    ) else {
        return out;
    };
    let rows = stmt.query_map([now() - 5], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            HookWait {
                id: r.get(2)?,
                kind: r.get(3)?,
                at: r.get(4)?,
                process: crate::sessions::ControlProcess {
                    pid: r.get(5)?,
                    start: r.get(6)?,
                },
            },
        ))
    });
    // Later rows are older, so the oldest request of each session wins.
    for (key, wait) in rows.into_iter().flatten().flatten() {
        out.insert(key, wait);
    }
    out
}

/// A CLI without observer hooks (Qoder) logs a tool call as running while its permission
/// request is held by the interaction hook, and CodeBuddy / ZCode / Kiro report no approval until
/// the hook returns. Surface that wait, or the popup never shows a request the hook has
/// already taken from the terminal.
pub(crate) fn promote_hook_requests(
    sessions: &mut [LiveSession],
    pending: &HashMap<(String, String), HookWait>,
) {
    if pending.is_empty() {
        return;
    }
    for session in sessions {
        if session.machine_id != "local"
            || !claude::approval_hooked(&session.provider)
            || session.background
            || session.expired
            // Hook-observed Claude sessions already track their own approvals and their
            // settlement.
            || (session.evidence == "hook" && claude::hooked(&session.provider))
            || !matches!(
                session.phase,
                Phase::Working | Phase::Thinking | Phase::Tool
            )
        {
            continue;
        }
        let Some(wait) = pending.get(&(session.source_id.clone(), session.native_session.clone()))
        else {
            continue;
        };
        // A session event newer than the request means it was answered in the terminal and
        // the hook helper has not noticed yet.
        if session.provider != "kiro"
            && session.event_at
                > (wait.at.max(0) as u64)
                    .saturating_add(2)
                    .saturating_mul(1000)
        {
            continue;
        }
        session.phase = Phase::NeedsInput;
        if session.provider == "kiro" {
            session.pid = Some(wait.process.pid);
            session.process_start = Some(wait.process.start);
        }
        session.input_request = format!("claudeHook:{}", wait.id);
        session.input_kind = match wait.kind.as_str() {
            "approval" => "approval",
            "elicitation" => "elicitation",
            _ => "question",
        }
        .into();
    }
}

/// Called by the session monitor lane. Stores metadata-only control identity; never reads prompts.
pub fn observe(store: &Store, snapshot: &Snapshot) -> Result<()> {
    let at = now();
    let cfg = config(&store.db)?;
    if cfg.enabled && at - LAST_HEARTBEAT.load(Ordering::Relaxed) >= 4 {
        touch_heartbeat(store);
    }
    if cfg.enabled
        && (cfg.claude_questions || cfg.claude_approvals || cfg.kiro_approvals)
        && at - LAST_HOOK_SYNC.load(Ordering::Relaxed) >= 30
    {
        LAST_HOOK_SYNC.store(at, Ordering::Relaxed);
        let _ = claude::sync_hooks(store);
    }
    let tx = store.db.unchecked_transaction()?;
    for s in snapshot
        .sessions
        .iter()
        .filter(|s| !s.background && !s.expired && !s.native_session.is_empty())
    {
        let key = state_key(s);
        let observed_at = snapshot
            .sources
            .iter()
            .find(|h| h.source_id == s.source_id && h.machine_id == s.machine_id)
            .filter(|h| h.error.is_none() && h.state != "connecting")
            .map(|h| h.checked_at as i64)
            .unwrap_or(if s.machine_id == "local" { at } else { 0 });
        let existing: Option<(String, i64)> = tx
            .query_row(
                "SELECT state_key,observed_at FROM interaction_sessions WHERE id=?1",
                [&s.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if existing
            .as_ref()
            .is_some_and(|(old_key, old_at)| *old_key == key && observed_at - *old_at < 5)
        {
            continue;
        }
        // The session left the wait that was answered. A later request under the same key
        // (the next approval of a Codex turn) is a new one and must not be shown as submitted.
        if existing
            .as_ref()
            .is_some_and(|(old_key, _)| *old_key != key)
        {
            forget_answer_unless(&tx, &s.id, &key)?;
        }
        let log_paths = if s.usage_members.is_empty() && s.subagent_count == 0 {
            s.log_paths.clone()
        } else {
            s.usage_members
                .iter()
                .filter(|m| m.source_id == s.source_id && m.native_session == s.native_session)
                .flat_map(|m| m.log_paths.iter().cloned())
                .collect()
        };
        let process = s.control_process.or_else(|| {
            s.pid
                .zip(s.process_start)
                .map(|(pid, start)| crate::sessions::ControlProcess { pid, start })
        });
        let row = Session {
            key: format!("s{}", &crate::models::hash(&s.id)[..10]),
            state_key: key.clone(),
            observed_at,
            session: s.clone(),
            path: s.path.clone(),
            log_paths,
            process,
        };
        tx.execute(
            "INSERT INTO interaction_sessions(id,state_key,observed_at,payload) VALUES(?1,?2,?3,?4) \
             ON CONFLICT(id) DO UPDATE SET state_key=excluded.state_key,observed_at=excluded.observed_at,payload=excluded.payload",
            params![s.id, key, observed_at, serde_json::to_string(&row)?],
        )?;
    }
    if at - LAST_CLEANUP.load(Ordering::Relaxed) >= 120 {
        LAST_CLEANUP.store(at, Ordering::Relaxed);
        tx.execute(
            "DELETE FROM interaction_sessions WHERE observed_at<?1",
            [at - RETENTION],
        )?;
        tx.execute(
            "DELETE FROM channel_claude_requests WHERE at<?1 OR (response IS NULL AND heartbeat<?2)",
            params![at - RETENTION, at - 600],
        )?;
        scrub_consumed(&tx, at)?;
        forget_answered(&tx, at)?;
    }
    tx.commit()?;
    Ok(())
}

/// A request the hook has delivered keeps its identity for delivery receipts, not its tool
/// input or the answer: prompts are not archived.
fn scrub_consumed(db: &Connection, at: i64) -> Result<()> {
    db.execute(
        "UPDATE channel_claude_requests SET input='{}',meta='{}',response='' \
         WHERE consumed=1 AND heartbeat<?1 AND (input<>'{}' OR meta<>'{}')",
        [at - 60],
    )?;
    Ok(())
}

/// Remember that the request behind `state_key` was answered. Identity and time only.
fn mark_answered(
    db: &Connection,
    session_id: &str,
    state_key: &str,
    prompt_hash: Option<&str>,
    request_id: Option<&str>,
) {
    let _ = db.execute(
        "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![
            format!("{ANSWERED_PREFIX}{session_id}"),
            json!({"stateKey": state_key, "at": now(),
                "promptHash": prompt_hash.unwrap_or(""), "requestId": request_id.unwrap_or("")})
            .to_string()
        ],
    );
}

/// A state key can cover several requests (every Codex approval of a turn shares one), so a
/// card that names a different request than the answered one is a new request, not a lagging one.
fn recently_answered(
    db: &Connection,
    session_id: &str,
    state_key: &str,
    prompt_hash: &str,
    request_id: &str,
) -> bool {
    let differs = |marked: &Value, shown: &str| {
        marked
            .as_str()
            .is_some_and(|m| !m.is_empty() && !shown.is_empty() && m != shown)
    };
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("{ANSWERED_PREFIX}{session_id}")],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    raw.and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .is_some_and(|v| {
            !state_key.is_empty()
                && v["stateKey"].as_str() == Some(state_key)
                && !differs(&v["promptHash"], prompt_hash)
                && !differs(&v["requestId"], request_id)
                && v["at"]
                    .as_i64()
                    .is_some_and(|at| (0..ANSWERED_TTL).contains(&(now() - at)))
        })
}

fn forget_answer_unless(db: &Connection, session_id: &str, state_key: &str) -> Result<()> {
    let key = format!("{ANSWERED_PREFIX}{session_id}");
    let raw: Option<String> = db
        .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
        .optional()?;
    let kept = raw
        .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .is_some_and(|v| v["stateKey"].as_str() == Some(state_key));
    if !kept {
        db.execute("DELETE FROM kv WHERE key=?1", [&key])?;
    }
    Ok(())
}

fn forget_answered(db: &Connection, at: i64) -> Result<()> {
    let mut stmt = db.prepare("SELECT key,value FROM kv WHERE key LIKE ?1")?;
    let stale = stmt
        .query_map([format!("{ANSWERED_PREFIX}%")], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .flatten()
        .filter(|(_, value)| {
            serde_json::from_str::<Value>(value)
                .ok()
                .and_then(|v| v["at"].as_i64())
                .is_none_or(|seen| at - seen >= ANSWERED_TTL)
        })
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    for key in stale {
        db.execute("DELETE FROM kv WHERE key=?1", [key])?;
    }
    Ok(())
}

pub fn call_status(store: &Store) -> Result<(bool, bool)> {
    let cfg = config(&store.db)?;
    Ok((cfg.enabled, cfg.auto_popup))
}

pub(crate) fn call(store: &Store, method: &str, params: Value) -> Result<Value> {
    match method {
        "sessions.interaction.status" => status(store),
        "sessions.interaction.configure" => configure(store, &params),
        "sessions.interaction.inspect" => inspect(store, &params),
        "sessions.interaction.respond" => respond(store, &params),
        "sessions.interaction.handoff" => handoff(store, &params),
        _ => anyhow::bail!("未知的会话交互操作"),
    }
}

/// Whether each local Codex source exposes the control socket the popup needs. A path check
/// only: the status call must stay cheap and must not connect to anything.
fn codex_control(store: &Store) -> Value {
    let Ok(settings) = store.settings() else {
        return json!([]);
    };
    let mut seen = std::collections::HashSet::new();
    json!(
        settings
            .sources
            .iter()
            .filter(|s| s.provider == "codex" && s.host_id.is_none() && settings.source_enabled(s))
            .filter_map(|s| {
                let root = expand(if s.path.is_empty() {
                    crate::contracts::provider(&s.provider).map_or("", |p| p.default_path)
                } else {
                    &s.path
                });
                seen.insert(root.clone()).then(|| {
                    json!({
                        "sourceId": s.id,
                        "available": cfg!(unix)
                            && root.join("app-server-control/app-server-control.sock").exists(),
                    })
                })
            })
            .collect::<Vec<_>>()
    )
}

fn status(store: &Store) -> Result<Value> {
    let cfg = config(&store.db)?;
    let feishu = crate::channels::configuration(&store.db)?;
    Ok(json!({
        "enabled": cfg.enabled,
        "autoPopup": cfg.auto_popup,
        "claudeQuestions": cfg.claude_questions,
        "claudeApprovals": cfg.claude_approvals,
        "kiroApprovals": cfg.kiro_approvals,
        "allowTerminal": cfg.allow_terminal || feishu.allow_terminal,
        "desktopAllowTerminal": cfg.allow_terminal,
        "feishuPaired": feishu.enabled && feishu.owner.is_some(),
        "hooks": claude::hook_summary(store),
        "codexControl": codex_control(store),
    }))
}

fn configure(store: &Store, input: &Value) -> Result<Value> {
    let mut cfg = config(&store.db)?;
    let was_kiro = cfg.kiro_approvals;
    if let Some(v) = input["enabled"].as_bool() {
        cfg.enabled = v;
    }
    if let Some(v) = input["autoPopup"].as_bool() {
        cfg.auto_popup = v;
    }
    if let Some(v) = input["claudeQuestions"].as_bool() {
        cfg.claude_questions = v;
    }
    if let Some(v) = input["claudeApprovals"].as_bool() {
        cfg.claude_approvals = v;
        if v {
            cfg.claude_questions = true;
        }
    }
    if let Some(v) = input["kiroApprovals"].as_bool() {
        if v && !was_kiro {
            claude::validate_kiro_approval_setup(&store.settings()?)?;
        }
        cfg.kiro_approvals = v;
    }
    if let Some(v) = input["allowTerminal"].as_bool() {
        cfg.allow_terminal = v;
    }
    save_config(&store.db, &cfg)?;
    if cfg.enabled {
        touch_heartbeat(store);
    }
    let sync = claude::sync_hooks(store);
    if was_kiro != cfg.kiro_approvals
        && let Err(error) = sync
    {
        cfg.kiro_approvals = was_kiro;
        save_config(&store.db, &cfg)?;
        let _ = claude::sync_hooks(store);
        return Err(error);
    }
    LAST_HOOK_SYNC.store(now(), Ordering::Relaxed);
    status(store)
}

fn effective_allow_terminal(store: &Store, cfg: &Config) -> bool {
    if cfg.allow_terminal {
        return true;
    }
    crate::channels::configuration(&store.db)
        .map(|c| c.allow_terminal)
        .unwrap_or(false)
}

fn language(store: &Store) -> String {
    store
        .settings()
        .map(|s| s.appearance.language)
        .unwrap_or_else(|_| "zh-CN".into())
}

/// Every key a client may read is always present, so older and newer clients decode one shape.
fn empty_payload(session_id: &str) -> Value {
    json!({
        "sessionId": session_id,
        "stateKey": "",
        "promptHash": "",
        "kind": "none",
        "adapter": "focusOnly",
        "transport": "local",
        "canRespond": false,
        "canHandoff": false,
        "needsClaudeHook": false,
        "needsClaudeApprovals": false,
        "quickEnable": "",
        "requestId": "",
        "pendingCount": 0,
        "pendingRequests": [],
        "requestedAt": 0,
        "inputKind": "",
        "answered": false,
        "title": "",
        "approval": Value::Null,
        "questions": [],
        "form": Value::Null,
        "textPrompt": "",
        "promptSource": "none",
        "followUp": FollowUp::default(),
        "readonlyReason": "",
    })
}

fn base_payload(target: &Session, lang: &str) -> Value {
    let mut payload = empty_payload(&target.session.id);
    payload["stateKey"] = json!(target.state_key);
    payload["title"] = json!(target.title(lang));
    payload["inputKind"] = json!(if target.session.phase == Phase::NeedsInput {
        target.session.input_kind.as_str()
    } else {
        ""
    });
    payload["transport"] = json!(if target.session.machine_id == "local" {
        "local"
    } else {
        "ssh"
    });
    payload
}

/// The end of a long message: a question to the user is normally its last paragraph.
fn tail(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

/// `textPrompt` is the pending question when a reader found one, otherwise the end of the
/// agent's last message so that a card without a dedicated reader still shows what was asked.
fn set_prompt(payload: &mut Value, question: &str, last_message: &str) {
    let (text, source) = if !question.trim().is_empty() {
        (question.to_owned(), "question")
    } else if !last_message.trim().is_empty() {
        (tail(last_message, 1600), "lastMessage")
    } else {
        (String::new(), "none")
    };
    payload["textPrompt"] = json!(text);
    payload["promptSource"] = json!(source);
}

/// The size and modification time of a finished session's record files.
type RecordStamp = Vec<(String, u64, Option<std::time::SystemTime>)>;

/// Last replies of finished sessions, keyed by session id. In memory only and bounded; an
/// entry holds for as long as the state key and the record files stay the same.
static FINISHED_TEXT: Mutex<Vec<(String, String, RecordStamp, String)>> = Mutex::new(Vec::new());
const FINISHED_TEXT_ENTRIES: usize = 16;

fn record_stamp(target: &Session) -> RecordStamp {
    std::iter::once(&target.path)
        .chain(&target.log_paths)
        .filter(|p| !p.is_empty())
        .take(8)
        .filter_map(|p| {
            let meta = std::fs::metadata(p).ok()?;
            Some((p.clone(), meta.len(), meta.modified().ok()))
        })
        .collect()
}

/// The last reply of a session that is not running. An open card asks for it every few
/// seconds, and some records are costly to read (a DeepSeek Harness log is decompressed from
/// its start), so it is read again only when the session state or a record file changed.
/// A session without record files on disk is always read.
fn finished_text(source: &Source, target: &Session) -> String {
    let stamp = record_stamp(target);
    if stamp.is_empty() {
        return control::local_excerpt(source, target).text;
    }
    let id = &target.session.id;
    if let Ok(cache) = FINISHED_TEXT.lock()
        && let Some(hit) = cache
            .iter()
            .find(|e| e.0 == *id && e.1 == target.state_key && e.2 == stamp)
    {
        return hit.3.clone();
    }
    let text = control::local_excerpt(source, target).text;
    // Stamp again: a file that grew during the read must not be remembered as current.
    if record_stamp(target) == stamp
        && let Ok(mut cache) = FINISHED_TEXT.lock()
    {
        cache.retain(|e| e.0 != *id);
        if cache.len() >= FINISHED_TEXT_ENTRIES {
            cache.remove(0);
        }
        cache.push((id.clone(), target.state_key.clone(), stamp, text.clone()));
    }
    text
}

/// Binds a free-text card to the question it showed, where no native request id exists.
fn text_prompt_hash(target: &Session, question: &str) -> String {
    crate::models::hash(&json!(["text", target.state_key, question]).to_string())
}

/// The hash a terminal reply must carry. Only a waiting session is bound to the question the
/// reader found: a finished or interrupted turn can still carry a question nobody answered,
/// and the follow-up card `inspect` issues for it is bound to the session state alone.
fn terminal_prompt_hash(target: &Session, excerpt: &content::Excerpt) -> String {
    let question = if target.session.phase == Phase::NeedsInput {
        excerpt.question.as_str()
    } else {
        ""
    };
    text_prompt_hash(target, question)
}

fn qoder_desktop_gui(source: &Source) -> bool {
    matches!(source.provider.as_str(), "qoder" | "qoder-cn")
        && crate::qoder_desktop::database(&source.path).is_some()
}

/// Why free text may not be typed into the original terminal right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalBlock {
    Running,
    UnknownPhase,
    DesktopGui,
    Approval,
    Structured,
    Disabled,
    Stale,
    Subagent,
    NoAdapter,
    NoLocalTerminal,
}

impl TerminalBlock {
    fn reason(self) -> &'static str {
        match self {
            Self::Running => "原终端仍在运行，请等待本轮结束后重试",
            Self::UnknownPhase => "会话状态待确认，请回到原窗口处理",
            Self::DesktopGui => "此 Agent 暂无原生回传接口，请点击跳转到原窗口处理",
            Self::Approval => "此终端没有可核验的审批接口，请在原窗口确认",
            Self::Structured => "原会话正在等待选择或确认，终端输入无法可靠送达，请在原窗口处理",
            Self::Disabled => "此会话需要启用终端自动化，或回到原窗口输入",
            Self::Stale => "缺少原终端的最新状态，请刷新后重试",
            Self::Subagent => "等待输入可能来自子 Agent，请回到原窗口确认目标",
            Self::NoAdapter => "未找到可连接的原会话接口或 tmux 分屏，请回到原窗口处理",
            Self::NoLocalTerminal => "此客户端无法向原终端输入，请回到原窗口处理",
        }
    }
}

/// The terminal a local or remote session can be reached through, if any. Identity only.
fn terminal_adapter(target: &Session) -> Option<&'static str> {
    if target.session.machine_id != "local" {
        return target.process.is_some().then_some("tmux");
    }
    let (pid, start) = target
        .session
        .pid
        .zip(target.session.process_start.filter(|n| *n > 0))?;
    let identity = crate::sessions::focus_target(pid, start);
    if identity["status"] != "found" {
        return None;
    }
    if let Some(tty) = identity["tty"].as_str() {
        let mut cmd = crate::process::cli_command("tmux");
        cmd.args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_id}\t#{pane_tty}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}",
        ]);
        if let Ok(output) = crate::process::run(cmd, vec![], std::time::Duration::from_secs(2)) {
            let text = String::from_utf8_lossy(&output);
            let hits: Vec<Vec<&str>> = text
                .lines()
                .map(|l| l.split('\t').collect())
                .filter(|p: &Vec<&str>| p.len() == 5 && p[1] == tty)
                .collect();
            if hits.len() == 1 {
                let hit = &hits[0];
                let pane_pid = hit[2].parse::<u64>().ok();
                let ancestor = identity["ancestors"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|p| p["pid"].as_u64() == pane_pid));
                if ancestor
                    && hit[4] == "0"
                    && control::agent_foreground(&target.session.provider, hit[3])
                {
                    return Some("tmux");
                }
            }
        }
    }
    if cfg!(target_os = "macos") && identity["foreground"] == true {
        return Some("macosTerminal");
    }
    None
}

/// The single gate for typing free text into the original terminal, shared by `inspect` and
/// `respond` so that a card can never be stricter than the reply path behind it. Text followed
/// by Enter must never reach a permission dialog or a selection menu, and a `terminalJob` is
/// only issued to a client that said it can deliver one.
fn terminal_gate(
    source: &Source,
    target: &Session,
    excerpt: Option<&content::Excerpt>,
    allow_terminal: bool,
    caps: Capabilities,
) -> std::result::Result<&'static str, TerminalBlock> {
    let s = &target.session;
    if s.phase != Phase::NeedsInput && !s.phase.terminal() {
        return Err(if s.phase == Phase::Unknown {
            TerminalBlock::UnknownPhase
        } else {
            TerminalBlock::Running
        });
    }
    if qoder_desktop_gui(source) {
        return Err(TerminalBlock::DesktopGui);
    }
    if s.phase == Phase::NeedsInput {
        if matches!(s.input_kind.as_str(), "approval" | "elicitation") || is_codex_approval(target)
        {
            return Err(TerminalBlock::Approval);
        }
        // A question tool opens a selection menu in every agent observed, whether or not a
        // reader could list its options.
        if s.input_kind == "question" {
            return Err(TerminalBlock::Structured);
        }
        if excerpt.is_some_and(|e| {
            e.claude_request.is_some()
                || e.async_request.is_some()
                || e.structured_questions.is_some()
                || e.structured_approval.is_some()
        }) {
            return Err(TerminalBlock::Structured);
        }
    }
    if !allow_terminal {
        return Err(TerminalBlock::Disabled);
    }
    if now() - target.observed_at > TERMINAL_FRESH {
        return Err(TerminalBlock::Stale);
    }
    if s.subagent_count > 0 && s.phase == Phase::NeedsInput {
        return Err(TerminalBlock::Subagent);
    }
    usable_adapter(terminal_adapter(target), caps)
}

/// tmux is driven by the core itself; the macOS terminal only through a client that types.
fn usable_adapter(
    found: Option<&'static str>,
    caps: Capabilities,
) -> std::result::Result<&'static str, TerminalBlock> {
    match found {
        Some("macosTerminal") if !caps.local_terminal => Err(TerminalBlock::NoLocalTerminal),
        Some(adapter) => Ok(adapter),
        None => Err(TerminalBlock::NoAdapter),
    }
}

/// Whether a new instruction can be sent to a session that is not waiting for an answer.
fn follow_up(
    source: &Source,
    target: &Session,
    allow_terminal: bool,
    caps: Capabilities,
    lang: &str,
) -> FollowUp {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let phase = target.session.phase;
    // OpenCode's server can stop a turn and hold an instruction until the turn ends.
    let opencode_live = source.provider == "opencode"
        && source.host_id.is_none()
        && phase.active()
        && opencode::hosted(source, target);
    if phase == Phase::NeedsInput {
        let mut follow = FollowUp::unavailable(tr("原会话正在等待回答，请先处理当前请求"));
        follow.can_interrupt = opencode_live;
        return follow;
    }
    if opencode_live {
        let mut follow = FollowUp::available("opencode", "queue");
        follow.can_interrupt = true;
        return follow;
    }
    if !phase.terminal() {
        // Only OpenCode (above) can queue an instruction behind a running turn.
        return FollowUp::unavailable(tr(if phase == Phase::Unknown {
            "会话状态待确认，请回到原窗口处理"
        } else {
            "原会话仍在执行，请等待本轮结束后再发送"
        }));
    }
    let codex = matches!(source.provider.as_str(), "codex" | "chatgpt-work");
    if source.host_id.is_some() {
        if codex {
            // The remote helper verifies the thread and reports `blocked` when it cannot send.
            return FollowUp::available("codex", "newTurn");
        }
    } else if codex {
        return if codex::Client::open(&expand(&source.path)).is_ok() {
            FollowUp::available("codex", "newTurn")
        } else {
            FollowUp::unavailable(tr("未检测到 Codex 控制接口，请点击跳转到原窗口继续"))
        };
    } else if source.provider == "opencode" && opencode::hosted(source, target) {
        return FollowUp::available("opencode", "newTurn");
    }
    match terminal_gate(source, target, None, allow_terminal, caps) {
        Ok(adapter) => FollowUp::available(adapter, "terminal"),
        Err(block) => FollowUp::unavailable(tr(block.reason())),
    }
}

/// One row of `pendingRequests`: enough to tell the waiting requests apart.
fn opencode_queue_entry(request: &Value, lang: &str) -> Value {
    let p = &request["params"];
    let (kind, title) = if request["method"] == opencode::PERMISSION {
        let card = opencode::approval_card(request, lang);
        let subject = card.command.lines().next().unwrap_or("").to_owned();
        (
            "approval",
            if subject.is_empty() {
                card.tool_name
            } else {
                format!("{} · {subject}", card.tool_name)
            },
        )
    } else {
        let title = [&p["form"]["title"], &p["questions"][0]["question"]]
            .into_iter()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
            .unwrap_or("");
        (
            if request["method"] == opencode::FORM {
                "questions"
            } else {
                "readonly"
            },
            title.to_owned(),
        )
    };
    json!({
        "requestId": p["itemId"],
        "kind": kind,
        "title": content::clip(&title, 160),
    })
}

fn set_approval(payload: &mut Value, card: Approval) {
    payload["kind"] = json!("approval");
    payload["approval"] = serde_json::to_value(card.bounded()).unwrap_or(Value::Null);
}

fn set_questions(payload: &mut Value, request: &Value) {
    payload["questions"] = json!(contract::normalize_questions(request));
    payload["form"] = contract::form_meta(request);
}

/// A Codex approval. A request kind the card cannot spell out has no decisions and stays in
/// the original window.
fn set_codex_approval(payload: &mut Value, card: Approval, lang: &str) {
    let answerable = !card.decisions.is_empty();
    payload["canRespond"] = json!(answerable);
    if !answerable {
        payload["readonlyReason"] =
            json!(crate::localization::translate(lang, "此审批需要在原窗口确认").into_owned());
    }
    set_approval(payload, card);
}

/// A Codex question (`request_user_input`) or MCP form. Forms that cannot be shown field by
/// field (URL mode, nested or vendor schemas) are described and left to the original window.
fn set_codex_questions(payload: &mut Value, request: &Value, lang: &str) {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    if !codex::is_form(request) {
        payload["kind"] = json!("questions");
        payload["canRespond"] = json!(true);
        set_questions(payload, request);
        return;
    }
    match codex::form_card(request, lang) {
        Some(card) => {
            payload["kind"] = json!("questions");
            payload["inputKind"] = json!("elicitation");
            payload["canRespond"] = json!(true);
            set_questions(payload, &card);
        }
        None => {
            let p = &request["params"];
            payload["kind"] = json!("readonly");
            payload["inputKind"] = json!("elicitation");
            payload["canRespond"] = json!(false);
            let message = p["message"].as_str().or_else(|| p["description"].as_str());
            set_prompt(payload, &content::clip(message.unwrap_or(""), 1600), "");
            payload["form"] = json!({
                "title": content::clip(p["serverName"].as_str().unwrap_or(""), 200),
                "description": content::clip(message.unwrap_or(""), 1600),
                "source": "mcp",
                "serverName": content::clip(p["serverName"].as_str().unwrap_or(""), 200),
                "url": content::clip(p["url"].as_str().unwrap_or(""), 2000),
                "canCancel": false,
                "canSkip": false,
            });
            payload["readonlyReason"] = json!(tr("此表单包含条件或外链字段，请回到原窗口处理"));
        }
    }
}

fn inspect(store: &Store, params: &Value) -> Result<Value> {
    let lang = language(store);
    let mut payload = inspect_card(store, params, &lang)?;
    // Session state trails an answer by a poll or two. Until it catches up, the fallbacks
    // below would describe the request that was just answered as one that cannot be answered.
    if payload["canRespond"] != true
        && !matches!(payload["kind"].as_str(), Some("none" | "followup"))
        && recently_answered(
            &store.db,
            payload["sessionId"].as_str().unwrap_or(""),
            payload["stateKey"].as_str().unwrap_or(""),
            payload["promptHash"].as_str().unwrap_or(""),
            payload["requestId"].as_str().unwrap_or(""),
        )
    {
        payload["kind"] = json!("none");
        payload["adapter"] = json!("focusOnly");
        payload["answered"] = json!(true);
        payload["approval"] = Value::Null;
        payload["questions"] = json!([]);
        payload["form"] = Value::Null;
        payload["needsClaudeHook"] = json!(false);
        payload["needsClaudeApprovals"] = json!(false);
        payload["quickEnable"] = json!("");
        payload["pendingCount"] = json!(0);
        payload["readonlyReason"] =
            json!(crate::localization::translate(&lang, "已提交，等待原会话继续").into_owned());
    }
    Ok(payload)
}

/// The prompt hash of a held hook request and, unless it is an approval, its question card.
/// `None` for a request that cannot be presented.
fn hook_card(
    target: &Session,
    req: &claude::PendingClaude,
    lang: &str,
) -> Option<(String, Option<Value>)> {
    if req.kind == "approval" {
        return Some((req.question_hash.clone(), None));
    }
    claude::question_card(target, req, lang)
        .map(|question| (codex::fingerprint(&question), Some(question)))
}

/// A remote `read` that could not look at the session reports a status and no excerpt. That
/// is not the same as a session with nothing pending.
fn remote_read_ok(value: &Value) -> bool {
    value["excerpt"].is_object()
}

/// Options and approvals read from a transcript have no reply interface. Typing into the
/// terminal cannot be shown to pick the intended option in each agent's menu, so these stay
/// read-only even when the terminal adapter is enabled. `false` when nothing structured is
/// pending.
fn set_transcript_prompt(payload: &mut Value, excerpt: &content::Excerpt, lang: &str) -> bool {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    if let Some(questions) = excerpt.structured_questions.as_ref() {
        let synthetic =
            json!({"method": "aieyes/readonlyQuestions", "params": {"questions": questions}});
        payload["kind"] = json!("questions");
        payload["inputKind"] = json!("question");
        set_questions(payload, &synthetic);
        payload["readonlyReason"] = json!(tr("此 Agent 的选项需要在原窗口选择，点击跳转后处理"));
        return true;
    }
    if let Some(approval) = excerpt.structured_approval.as_ref() {
        let text = |key: &str| approval[key].as_str().unwrap_or("").to_owned();
        let mut card = Approval::new(
            approval["toolName"].as_str().unwrap_or("Tool"),
            approval["kind"].as_str().unwrap_or("generic"),
        );
        card.reason = text("reason");
        card.command = text("command");
        card.details = text("details");
        card.cwd = text("cwd");
        payload["inputKind"] = json!("approval");
        set_approval(payload, card);
        payload["readonlyReason"] = json!(tr("此 Agent 的授权需要在原窗口确认，点击跳转后处理"));
        return true;
    }
    false
}

fn inspect_card(store: &Store, params: &Value, lang: &str) -> Result<Value> {
    let cfg = config(&store.db)?;
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let caps = Capabilities::from_params(params);
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let Some(target) = session(&store.db, session_id)? else {
        let mut payload = empty_payload(session_id);
        payload["readonlyReason"] = json!(tr("当前会话已结束或不可用"));
        return Ok(payload);
    };
    let mut payload = base_payload(&target, lang);
    if !cfg.enabled {
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(tr("桌宠快捷交互已关闭，可在设置中开启"));
        return Ok(payload);
    }
    let settings = store.settings()?;
    let source = match control::source(&settings, &target) {
        Ok(s) => s,
        Err(error) => {
            if target.session.phase == Phase::NeedsInput {
                payload["kind"] = json!("readonly");
            }
            payload["readonlyReason"] = json!(tr(&error.to_string()));
            return Ok(payload);
        }
    };
    let allow_terminal = effective_allow_terminal(store, &cfg);

    // 0. Not waiting: say whether a follow-up can be sent, and show the last reply for context.
    if target.session.phase != Phase::NeedsInput {
        let follow = follow_up(&source, &target, allow_terminal, caps, lang);
        if target.session.phase.terminal() {
            let last = if source.host_id.is_some() {
                control::remote(&settings, &source, &target, "read", "", false, None)
                    .ok()
                    .and_then(|v| v["excerpt"]["text"].as_str().map(str::to_owned))
                    .unwrap_or_default()
            } else {
                finished_text(&source, &target)
            };
            set_prompt(&mut payload, "", &last);
        }
        // A running session takes a follow-up only where the adapter can queue it.
        if follow.available && (target.session.phase.terminal() || follow.mode == "queue") {
            payload["kind"] = json!("followup");
            payload["adapter"] = json!(follow.adapter);
            payload["canRespond"] = json!(true);
            if source.host_id.is_none() {
                payload["promptHash"] =
                    json!(terminal_prompt_hash(&target, &content::Excerpt::default()));
            }
        }
        payload["followUp"] = json!(follow);
        return Ok(payload);
    }
    payload["followUp"] = json!(follow_up(&source, &target, allow_terminal, caps, lang));

    // 1. Remote SSH session
    if source.host_id.is_some() {
        if let Some(value) = control::remote(&settings, &source, &target, "read", "", false, None)
            .ok()
            .filter(remote_read_ok)
        {
            let excerpt: content::Excerpt =
                serde_json::from_value(value["excerpt"].clone()).unwrap_or_default();
            let prompt_hash = value["promptHash"].as_str().unwrap_or("").to_owned();
            // Only the remote Codex control socket yields structured requests. The card is
            // built here from the request itself, exactly as for a local session.
            let structured = ["approvalRequest", "questionRequest"]
                .into_iter()
                .find_map(|key| value.get(key).filter(|v| v.is_object()));
            if let Some(req) = structured {
                payload["adapter"] = json!("codex");
                payload["promptHash"] = json!(prompt_hash);
                payload["pendingCount"] = json!(contract::pending_count(req));
                if codex::is_approval(req) {
                    let changes = value["approvalPatch"]
                        .as_str()
                        .map(codex::patch_changes)
                        .unwrap_or_default();
                    set_codex_approval(
                        &mut payload,
                        codex::approval_card_with(req, changes, lang),
                        lang,
                    );
                } else {
                    set_codex_questions(&mut payload, req, lang);
                }
                return Ok(payload);
            }
            let question = value["question"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&excerpt.question);
            set_prompt(&mut payload, question, &excerpt.text);
            // A remote Claude / Qoder question has options but no hook to answer them through.
            if claude::hooked(&source.provider)
                && let Some(req) = claude::request(&target, &excerpt)
            {
                payload["kind"] = json!("questions");
                set_questions(&mut payload, &req);
                payload["readonlyReason"] =
                    json!(tr("远程会话的选项题暂不能在弹窗回答，请回到原终端选择"));
                return Ok(payload);
            }
            // What the remote reader found is shown as it is for a local session.
            if set_transcript_prompt(&mut payload, &excerpt, lang) {
                return Ok(payload);
            }
            match terminal_gate(&source, &target, Some(&excerpt), allow_terminal, caps) {
                Ok(adapter) => {
                    payload["kind"] = json!("text");
                    payload["adapter"] = json!(adapter);
                    payload["canRespond"] = json!(true);
                }
                Err(TerminalBlock::Approval) => {
                    set_approval(
                        &mut payload,
                        Approval::readonly(&source.provider, &tr("原会话请求确认")),
                    );
                    payload["readonlyReason"] = json!(tr(TerminalBlock::Approval.reason()));
                }
                Err(block) => {
                    payload["kind"] = json!("readonly");
                    payload["readonlyReason"] = json!(match block {
                        TerminalBlock::Disabled => tr("此会话需要启用终端适配或在原终端处理"),
                        TerminalBlock::NoAdapter =>
                            tr("此远程会话暂无可直连的回传接口，请回到原终端处理"),
                        other => tr(other.reason()),
                    });
                }
            }
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(tr("暂时无法读取远程会话的等待内容"));
        return Ok(payload);
    }

    // 2. Local interaction hook holding a request (tool approval, AskUserQuestion or MCP form).
    // Requests are served oldest first, the order the terminal shows its own dialogs in.
    // Nothing is written to the card until a row is known to be presentable.
    if claude::approval_hooked(&source.provider)
        && (source.provider != "kiro" || cfg.kiro_approvals)
        && let Ok(rows) =
            claude::pending_requests(store, &source.id, &target.session.native_session)
        && let Some((req, hash, question)) = rows.iter().find_map(|req| {
            hook_card(&target, req, lang).map(|(hash, question)| (req, hash, question))
        })
    {
        payload["adapter"] = json!("claudeHook");
        payload["canRespond"] = json!(true);
        payload["canHandoff"] = json!(true);
        payload["requestId"] = json!(req.id);
        payload["promptHash"] = json!(hash);
        payload["pendingCount"] = json!(rows.len());
        payload["requestedAt"] = json!(req.at);
        match question {
            Some(question) => {
                payload["kind"] = json!("questions");
                set_questions(&mut payload, &question);
            }
            None => set_approval(&mut payload, claude::approval_card(req, lang)),
        }
        return Ok(payload);
    }

    // 3. Local Codex / ChatGPT Work
    if matches!(source.provider.as_str(), "codex" | "chatgpt-work") {
        let excerpt = content::local(&source, &target);
        let waiting_on_approval = is_codex_approval(&target);
        if !waiting_on_approval && let Some(async_req) = codex::async_request(&target, &excerpt) {
            let can_connect = codex::Client::open(&expand(&source.path)).is_ok();
            payload["promptHash"] = json!(codex::fingerprint(&async_req));
            payload["pendingCount"] = json!(1);
            set_questions(&mut payload, &async_req);
            if can_connect {
                payload["kind"] = json!("questions");
                payload["adapter"] = json!("codex");
                payload["canRespond"] = json!(true);
            } else {
                payload["kind"] = json!("readonly");
                set_prompt(&mut payload, &excerpt.question, &excerpt.text);
                payload["readonlyReason"] =
                    json!(tr("未检测到 Codex 控制接口，请点击跳转到原窗口回答"));
            }
            return Ok(payload);
        }
        let mut connected = false;
        if let Ok(mut client) = codex::Client::open(&expand(&source.path)) {
            connected = true;
            if client.attach(&target, true).is_ok()
                && let Ok(Some(request)) = if waiting_on_approval {
                    client.pending_approval(&target)
                } else {
                    client.pending(&target)
                }
            {
                let approval = codex::is_approval(&request);
                if approval || codex::is_form(&request) || codex::is_user_input(&request) {
                    payload["promptHash"] = json!(codex::fingerprint(&request));
                    payload["adapter"] = json!("codex");
                    payload["pendingCount"] = json!(contract::pending_count(&request));
                    if approval {
                        let changes = codex::file_changes(&client, &source, &target, &request);
                        set_codex_approval(
                            &mut payload,
                            codex::approval_card_with(&request, changes, lang),
                            lang,
                        );
                    } else {
                        set_codex_questions(&mut payload, &request, lang);
                    }
                    return Ok(payload);
                }
            }
        }
        set_prompt(&mut payload, &excerpt.question, &excerpt.text);
        if waiting_on_approval {
            set_approval(
                &mut payload,
                Approval::readonly("Codex", &tr("原会话请求确认")),
            );
            payload["readonlyReason"] = json!(tr("此审批请求暂时无法在弹窗核验，请在原窗口确认"));
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(if cfg!(windows) {
            tr("Windows 上暂未暴露 Codex 原生控制接口，请点击跳转到原窗口处理")
        } else if !connected {
            tr("未检测到 Codex 控制接口，请点击跳转到原窗口回答")
        } else {
            tr("原会话暂无可直接提交的请求，请点击跳转到原窗口处理")
        });
        return Ok(payload);
    }

    // 4. Local OpenCode hosted by its server. Requests are shown oldest first; `requestId`
    // (one of `pendingRequests`) shows another one that is still pending.
    if source.provider == "opencode"
        && let Some(all) = opencode::requests(&source, &target, lang)
        && let Some(request) =
            opencode::select(&all, params["requestId"].as_str()).or_else(|| all.first())
    {
        payload["promptHash"] = json!(codex::fingerprint(request));
        payload["adapter"] = json!("opencode");
        payload["requestId"] = request["params"]["itemId"].clone();
        payload["pendingCount"] = json!(all.len());
        payload["pendingRequests"] = json!(
            all.iter()
                .take(50)
                .map(|r| opencode_queue_entry(r, lang))
                .collect::<Vec<_>>()
        );
        if request["method"] == opencode::PERMISSION {
            payload["canRespond"] = json!(true);
            payload["inputKind"] = json!("approval");
            set_approval(&mut payload, opencode::approval_card(request, lang));
            return Ok(payload);
        }
        payload["inputKind"] = json!("question");
        if request["method"] == opencode::FORM {
            payload["kind"] = json!("questions");
            payload["canRespond"] = json!(true);
            set_questions(&mut payload, request);
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["form"] = contract::form_meta(request);
        payload["readonlyReason"] = json!(tr("此表单包含条件或外链字段，请回到原窗口处理"));
        return Ok(payload);
    }

    // 5. Fallback readers (Claude without active hook, Kiro, DeepSeek Harness, Antigravity, Qoder Desktop, generic CLI)
    let excerpt = control::local_excerpt(&source, &target);
    let hooked = claude::hooked(&source.provider);
    let input_kind = target.session.input_kind.as_str();
    set_prompt(&mut payload, &excerpt.question, &excerpt.text);
    if hooked && excerpt.claude_request.is_some() {
        let request = claude::request(&target, &excerpt);
        let hook_on = cfg.claude_questions || cfg.claude_approvals;
        payload["kind"] = json!(if request.is_some() {
            "questions"
        } else {
            "readonly"
        });
        if let Some(request) = &request {
            set_questions(&mut payload, request);
        }
        payload["needsClaudeHook"] = json!(!hook_on);
        payload["quickEnable"] = json!(if hook_on { "" } else { "questions" });
        payload["readonlyReason"] = json!(if hook_on {
            tr("此提问在交互 Hook 就绪前发起；请回原窗口回答，或重开会话后即可在弹窗操作")
        } else {
            tr("启用 Claude / Qoder 交互 Hook 后可直接在弹窗选择回答")
        });
        return Ok(payload);
    }

    if set_transcript_prompt(&mut payload, &excerpt, lang) {
        // CodeBuddy's approval can be decided in the popup once its hook is on.
        let offer = payload["kind"] == "approval"
            && claude::approval_hooked(&source.provider)
            && !cfg.claude_approvals
            && !qoder_desktop_gui(&source);
        if offer {
            payload["needsClaudeApprovals"] = json!(true);
            payload["quickEnable"] = json!("approvals");
            payload["readonlyReason"] = json!(tr(
                "可在设置中开启工具授权弹窗（CodeBuddy / ZCode 同样适用），或点击跳转到原窗口确认"
            ));
        }
        return Ok(payload);
    }

    if input_kind == "elicitation" {
        payload["kind"] = json!("readonly");
        let hook_on = cfg.claude_questions || cfg.claude_approvals;
        let offer = hooked && !hook_on && !qoder_desktop_gui(&source);
        payload["needsClaudeHook"] = json!(offer);
        payload["quickEnable"] = json!(if offer { "questions" } else { "" });
        payload["readonlyReason"] = json!(if offer {
            tr("启用 Claude / Qoder 交互 Hook 后可直接在弹窗填写 MCP 表单")
        } else if hooked && hook_on {
            // The hook leaves these to the terminal: it cannot show them faithfully.
            tr(
                "此 MCP 表单需要在原窗口处理（外链或嵌套表单、发起于交互 Hook 就绪前，或已交还终端）",
            )
        } else {
            tr("此 MCP 表单暂不能在弹窗填写，请回到原窗口处理")
        });
        return Ok(payload);
    }

    // A permission wait nobody can verify is an approval card without buttons, never a text box.
    if input_kind == "approval" {
        set_approval(
            &mut payload,
            Approval::readonly(&source.provider, &tr("原会话请求确认")),
        );
        let decision_hook = source.provider != "kiro" && claude::approval_hooked(&source.provider);
        let offer = decision_hook && !cfg.claude_approvals && !qoder_desktop_gui(&source);
        payload["needsClaudeApprovals"] = json!(offer);
        payload["quickEnable"] = json!(if offer { "approvals" } else { "" });
        payload["readonlyReason"] = json!(if offer && hooked {
            tr("可在设置中开启 Claude / Qoder 工具授权弹窗，或点击跳转到原窗口确认")
        } else if offer {
            tr("可在设置中开启工具授权弹窗（CodeBuddy / ZCode 同样适用），或点击跳转到原窗口确认")
        } else if hooked && cfg.claude_approvals {
            // Sandbox network prompts and prompts raised by other hooks never reach the hook.
            tr(
                "此授权未经交互 Hook 送达（发起于 Hook 就绪前、已交还终端、内容过大，或属于沙箱网络等 Hook 无法回答的确认），请回原窗口确认",
            )
        } else if decision_hook && cfg.claude_approvals {
            tr("此授权未经交互 Hook 送达或已超过弹窗等待时间，请回原窗口确认")
        } else {
            tr(TerminalBlock::Approval.reason())
        });
        return Ok(payload);
    }

    match terminal_gate(&source, &target, Some(&excerpt), allow_terminal, caps) {
        Ok(adapter) => {
            payload["kind"] = json!("text");
            payload["adapter"] = json!(adapter);
            payload["canRespond"] = json!(true);
            payload["promptHash"] = json!(terminal_prompt_hash(&target, &excerpt));
        }
        Err(block) => {
            payload["kind"] = json!("readonly");
            payload["readonlyReason"] = json!(match block {
                // The agent opened a question whose options no reader could list.
                TerminalBlock::Structured if excerpt.question.trim().is_empty() =>
                    tr("此 Agent 正在等待选择，选项内容暂不可读取，请点击跳转到原窗口处理"),
                TerminalBlock::Disabled if target.session.pid.is_some() =>
                    tr("此 Agent 无原生回传接口；可点击跳转到原窗口，或在设置中启用终端适配输入"),
                TerminalBlock::Disabled | TerminalBlock::NoAdapter | TerminalBlock::DesktopGui =>
                    tr("此 Agent 暂无原生回传接口，请点击跳转到原窗口处理"),
                other => tr(other.reason()),
            });
        }
    }
    Ok(payload)
}

fn outcome_json(outcome: control::Outcome, lang: &str) -> Value {
    let msg = crate::localization::translate(lang, &outcome.reason).into_owned();
    json!({
        "status": outcome.status,
        "message": msg,
        "receipt": outcome.receipt,
    })
}

fn blocked(reason: &str, lang: &str) -> Result<Value> {
    Ok(outcome_json(control::Outcome::blocked(reason), lang))
}

/// What a reply carries, already checked for shape and size.
struct Reply<'a> {
    verb: Verb,
    answers: Option<&'a Value>,
    text: Option<&'a str>,
    reason: Option<&'a str>,
    hash: Option<&'a str>,
}

/// The action string the adapters' reply code understands.
fn legacy_action(verb: &Verb) -> &str {
    match verb {
        Verb::Answer | Verb::FollowUp => "answer",
        Verb::Cancel => "cancel",
        Verb::Skip => "skip",
        Verb::Decide(id) => id,
    }
}

/// A question card accepts answers, and cancel or skip only where the form offers them.
fn check_question_reply(request: &Value, reply: &Reply) -> Result<()> {
    let form = contract::form_meta(request);
    match &reply.verb {
        Verb::Answer => {}
        Verb::Cancel if form["canCancel"] == true => return Ok(()),
        Verb::Skip if form["canSkip"] == true => return Ok(()),
        Verb::Cancel | Verb::Skip => anyhow::bail!("此请求不支持取消或跳过，请在原窗口处理"),
        Verb::FollowUp => anyhow::bail!("原会话正在等待回答，请先处理当前请求"),
        Verb::Decide(_) => anyhow::bail!("当前请求不是审批，请刷新后再操作"),
    }
    ensure!(
        reply.reason.is_none_or(|r| r.trim().is_empty()),
        "此审批选项不支持附加说明"
    );
    if let Some(answers) = reply.answers {
        contract::check_answers(&contract::normalize_questions(request), answers)?;
    }
    Ok(())
}

fn respond(store: &Store, params: &Value) -> Result<Value> {
    let cfg = config(&store.db)?;
    let lang = language(store);
    ensure!(cfg.enabled, "桌宠快捷交互已关闭");
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let expected_state = params["stateKey"].as_str().unwrap_or("");
    let caps = Capabilities::from_params(params);
    let request_id = params["requestId"].as_str().filter(|s| !s.is_empty());
    let reply = Reply {
        verb: match contract::parse_verb(params) {
            Ok(verb) => verb,
            Err(error) => return blocked(&error.to_string(), &lang),
        },
        answers: params.get("answers").filter(|v| !v.is_null()),
        text: params["text"].as_str(),
        reason: params["reason"].as_str(),
        hash: params["promptHash"].as_str().filter(|s| !s.is_empty()),
    };
    if let Err(error) = contract::validate_text(reply.text)
        .and_then(|_| contract::validate_reason(reply.reason))
        .and_then(|_| reply.answers.map_or(Ok(()), contract::validate_answers))
    {
        return blocked(&error.to_string(), &lang);
    }

    let Some(target) = session(&store.db, session_id)? else {
        return blocked("原会话已结束或不可用", &lang);
    };
    // The state key is the only proof that the caller saw the request it is answering.
    if expected_state.is_empty() || target.state_key != expected_state {
        return blocked("会话状态或问题已变化，请刷新后再操作", &lang);
    }
    let waiting = target.session.phase == Phase::NeedsInput;
    if waiting && reply.verb == Verb::FollowUp {
        return blocked("原会话正在等待回答，请先处理当前请求", &lang);
    }
    // Stopping a turn is the one action that is not an answer to a pending request.
    if !waiting && !reply.verb.is_text() && !is_interrupt(&reply.verb) {
        return blocked("当前没有待处理的请求，请刷新后再操作", &lang);
    }
    let settings = store.settings()?;
    let source = match control::source(&settings, &target) {
        Ok(s) => s,
        Err(error) => return blocked(&error.to_string(), &lang),
    };
    let allow_terminal = effective_allow_terminal(store, &cfg);
    let outcome = dispatch(
        store,
        &settings,
        &source,
        &target,
        &reply,
        request_id,
        allow_terminal,
        caps,
        &lang,
    );
    let value = match outcome {
        Delivery::Outcome(outcome) => {
            if matches!(outcome.status.as_str(), "confirmed" | "submitted") && waiting {
                mark_answered(
                    &store.db,
                    &target.session.id,
                    &target.state_key,
                    reply.hash,
                    request_id,
                );
            }
            outcome_json(outcome, &lang)
        }
        Delivery::LocalTerminal(body) => json!({
            "status": "needsLocalTerminal",
            "message": crate::localization::translate(&lang, "正在向原终端发送输入…"),
            "receipt": {"adapter": "macos"},
            "terminalJob": {
                "text": body,
                "session": target.session,
            }
        }),
    };
    Ok(value)
}

fn is_interrupt(verb: &Verb) -> bool {
    matches!(verb, Verb::Decide(id) if id == "interrupt")
}

enum Delivery {
    Outcome(control::Outcome),
    /// Text the calling client must type into the session's own terminal.
    LocalTerminal(String),
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    store: &Store,
    settings: &crate::models::Settings,
    source: &Source,
    target: &Session,
    reply: &Reply,
    request_id: Option<&str>,
    allow_terminal: bool,
    caps: Capabilities,
    lang: &str,
) -> Delivery {
    use control::Outcome;
    let block = |reason: &str| Delivery::Outcome(Outcome::blocked(reason));
    let waiting = target.session.phase == Phase::NeedsInput;
    let codex = matches!(source.provider.as_str(), "codex" | "chatgpt-work");

    // 0. Stop the running turn. Only OpenCode's server exposes this for a session it hosts.
    if is_interrupt(&reply.verb) {
        if source.host_id.is_none()
            && source.provider == "opencode"
            && target.session.phase.active()
            && let Some(out) = opencode::interrupt(source, target)
        {
            return Delivery::Outcome(out);
        }
        return block("此会话不支持在弹窗中断，请回到原窗口处理");
    }

    // 1. Remote SSH session
    if source.host_id.is_some() {
        if !codex {
            // Everything but Codex is typed into a remote tmux pane.
            if !reply.verb.is_text() {
                return block(TerminalBlock::Approval.reason());
            }
            if let Err(gate) = terminal_gate(source, target, None, allow_terminal, caps) {
                return block(gate.reason());
            }
        }
        let native = if codex && waiting {
            match remote_codex_reply(settings, source, target, reply, lang) {
                Ok(native) => native,
                Err(error) => return block(&error.to_string()),
            }
        } else {
            None
        };
        let formatted = match &native {
            Some(_) => String::new(),
            None => match format_remote_reply(reply) {
                Ok(text) => text,
                Err(error) => return block(&error.to_string()),
            },
        };
        return Delivery::Outcome(
            match control::remote_reply(
                settings,
                source,
                target,
                &formatted,
                native.as_ref(),
                allow_terminal,
                reply.hash,
            ) {
                Ok(v) => serde_json::from_value::<Outcome>(v)
                    .unwrap_or_else(|_| Outcome::uncertain("远程输入结果无效，请核对原会话")),
                Err(_) => Outcome::uncertain("远程连接中断，无法确认提交结果；请核对原会话"),
            },
        );
    }

    // 2. Local interaction hook holding a request
    if waiting
        && claude::approval_hooked(&source.provider)
        && let Ok(rows) =
            claude::pending_requests(store, &source.id, &target.session.native_session)
        && !rows.is_empty()
    {
        if source.provider == "kiro" && request_id.is_none() {
            return block("原请求已结束或已被处理，请刷新后再操作");
        }
        // A card names its request; without a name the oldest one is the one that was shown.
        let Some(req) = (match request_id {
            Some(id) => rows.iter().find(|r| r.id == id),
            None => rows.iter().find(|r| hook_card(target, r, lang).is_some()),
        }) else {
            return block("原请求已结束或已被处理，请刷新后再操作");
        };
        if req.kind == "approval" {
            if reply.hash != Some(req.question_hash.as_str()) {
                return block("审批请求已变化，请刷新后再操作");
            }
            let card = claude::approval_card(req, lang);
            return Delivery::Outcome(
                match contract::pick_decision(&card, &reply.verb, reply.reason) {
                    Ok(decision) => {
                        claude::answer_pending_approval(store, req, target, decision, reply.reason)
                    }
                    Err(error) => Outcome::blocked(&error.to_string()),
                },
            );
        }
        let Some(question) = claude::question_card(target, req, lang) else {
            return block("原会话问题已不可用");
        };
        if reply.hash != Some(codex::fingerprint(&question).as_str()) {
            return block("问题已变化，请刷新后再操作");
        }
        if let Err(error) = check_question_reply(&question, reply) {
            return block(&error.to_string());
        }
        if req.kind == "elicitation" {
            // An MCP form can be filled, declined ("skip") or dismissed ("cancel").
            let form = match reply.verb {
                Verb::Cancel => claude::FormReply::Cancel,
                Verb::Skip => claude::FormReply::Decline,
                _ => claude::FormReply::Accept,
            };
            return Delivery::Outcome(claude::answer_pending_form(
                store,
                req,
                target,
                form,
                reply.answers,
                reply.text,
            ));
        }
        return Delivery::Outcome(claude::answer_pending_question(
            store,
            req,
            target,
            reply.answers,
            reply.text,
        ));
    }

    // 3. Local Codex / ChatGPT Work
    if codex {
        return Delivery::Outcome(respond_local_codex(source, target, reply, lang));
    }

    // 4. Local OpenCode
    if source.provider == "opencode" {
        if let Some(all) = opencode::requests(source, target, lang)
            && !all.is_empty()
        {
            // A card names its request; without a name the oldest one is the one that was shown.
            let Some(request) = opencode::select(&all, request_id) else {
                return block("原请求已结束或已被处理，请刷新后再操作");
            };
            if reply.hash != Some(codex::fingerprint(request).as_str()) {
                return block("问题已改变或尚未同步，请刷新后再操作");
            }
            let checked = if request["method"] == opencode::PERMISSION {
                contract::pick_decision(
                    &opencode::approval_card(request, lang),
                    &reply.verb,
                    reply.reason,
                )
                .map(|_| ())
            } else {
                check_question_reply(request, reply)
            };
            if let Err(error) = checked {
                return block(&error.to_string());
            }
        }
        // The adapter reads the pending requests again and compares the hash before replying.
        // `None` means no reachable server hosts this session.
        if let Some(out) = opencode::execute_structured(
            source,
            target,
            &opencode::Reply {
                hash: reply.hash,
                action: legacy_action(&reply.verb),
                answers: reply.answers,
                text: reply.text,
                reason: reply.reason,
                request_id,
                lang,
            },
        ) {
            return Delivery::Outcome(out);
        }
    }

    // 5. Local terminal fallback (tmux / macOS terminal automation)
    if !reply.verb.is_text() {
        return block(TerminalBlock::Approval.reason());
    }
    let body = reply.text.unwrap_or("").trim();
    if body.is_empty() {
        return block("输入内容不能为空");
    }
    let excerpt = control::local_excerpt(source, target);
    let adapter = match terminal_gate(source, target, Some(&excerpt), allow_terminal, caps) {
        Ok(adapter) => adapter,
        Err(gate) => return block(gate.reason()),
    };
    if reply.hash != Some(terminal_prompt_hash(target, &excerpt).as_str()) {
        return block("问题已变化，请刷新后再操作");
    }
    // tmux verifies the pane and its foreground process again before it pastes.
    if let Some(out) = control::tmux(target, body) {
        return Delivery::Outcome(out);
    }
    if adapter == "macosTerminal" {
        return Delivery::LocalTerminal(body.to_owned());
    }
    block(TerminalBlock::NoAdapter.reason())
}

fn respond_local_codex(
    source: &Source,
    target: &Session,
    reply: &Reply,
    lang: &str,
) -> control::Outcome {
    use control::Outcome;
    let root = expand(&source.path);
    let Ok(mut client) = codex::Client::open(&root) else {
        return Outcome::blocked("无法连接原会话的提问接口，请回到原窗口处理");
    };
    let waiting = target.session.phase == Phase::NeedsInput;
    let waiting_on_approval = is_codex_approval(target);
    if waiting
        && !waiting_on_approval
        && let Some(request) = codex::async_request(target, &content::local(source, target))
    {
        if reply.hash != Some(codex::fingerprint(&request).as_str()) {
            return Outcome::blocked("问题已改变或尚未同步，请刷新后再操作");
        }
        if let Err(error) = check_question_reply(&request, reply) {
            return Outcome::blocked(&error.to_string());
        }
        return client.answer_async_structured(target, &request, reply.answers, reply.text);
    }
    let thread = match client.attach(target, waiting) {
        Ok(thread) => thread,
        Err(error) => return Outcome::blocked(&error.to_string()),
    };
    if waiting {
        let pending = if waiting_on_approval {
            client.pending_approval(target)
        } else {
            client.pending(target)
        };
        let request = match pending {
            Ok(Some(r)) => r,
            _ => return Outcome::blocked("原会话没有可回答的请求，请核对原窗口"),
        };
        if reply.hash != Some(codex::fingerprint(&request).as_str()) {
            return Outcome::blocked("问题已改变或尚未同步，请刷新后再操作");
        }
        if codex::is_approval(&request) {
            let card = codex::approval_card(&request, lang);
            return match contract::pick_decision(&card, &reply.verb, reply.reason) {
                Ok(decision) => client.answer_decision(target, &request, decision, reply.reason),
                Err(error) => Outcome::blocked(&error.to_string()),
            };
        }
        if let Err(error) = check_codex_questions(&request, reply, lang) {
            return Outcome::blocked(&error.to_string());
        }
        return client.answer_structured(
            target,
            &request,
            legacy_action(&reply.verb),
            reply.answers,
            reply.text,
        );
    }
    if thread["status"]["type"] == "idle" {
        let body = reply.text.unwrap_or("").trim();
        if body.is_empty() {
            return Outcome::blocked("输入内容不能为空");
        }
        return client.followup(target, body);
    }
    Outcome::blocked("原会话状态已改变，请核对原窗口")
}

/// A reply to a Codex question or MCP form is checked against the card that inspect showed.
fn check_codex_questions(request: &Value, reply: &Reply, lang: &str) -> Result<()> {
    if codex::is_form(request) {
        let card = codex::form_card(request, lang).context("此类输入需要回到原窗口处理")?;
        return check_question_reply(&card, reply);
    }
    ensure!(codex::is_user_input(request), "此类输入需要回到原窗口处理");
    check_question_reply(request, reply)
}

/// The native reply to the structured request a remote Codex thread holds, built here from
/// that request exactly as for a local session; the helper sends it only if the request it
/// then finds still has this hash. `None`: nothing structured is pending (an async question
/// or a follow-up), which goes as text.
fn remote_codex_reply(
    settings: &crate::models::Settings,
    source: &Source,
    target: &Session,
    reply: &Reply,
    lang: &str,
) -> Result<Option<Value>> {
    let read = control::remote(settings, source, target, "read", "", false, None)
        .map_err(|_| anyhow::anyhow!("暂时无法读取远程会话的等待内容"))?;
    remote_codex_native(&read, reply, lang)
}

/// The reply for what the helper's `read` returned; kept apart from the SSH call so it can be
/// tested against recorded requests.
fn remote_codex_native(read: &Value, reply: &Reply, lang: &str) -> Result<Option<Value>> {
    let request = ["approvalRequest", "questionRequest"]
        .into_iter()
        .find_map(|key| read.get(key).filter(|v| v.is_object()));
    let Some(request) = request.filter(|r| r["method"] != "aieyes/asyncQuestion") else {
        ensure!(reply.verb.is_text(), "原会话没有可回答的请求，请核对原窗口");
        return Ok(None);
    };
    ensure!(
        reply.hash.is_some() && reply.hash == read["promptHash"].as_str(),
        "问题已改变或尚未同步，请刷新后再操作"
    );
    if codex::is_approval(request) {
        let card = codex::approval_card(request, lang);
        let decision = contract::pick_decision(&card, &reply.verb, reply.reason)?;
        return codex::decision_reply(decision, reply.reason).map(Some);
    }
    check_codex_questions(request, reply, lang)?;
    codex::structured_payload(
        request,
        legacy_action(&reply.verb),
        reply.answers,
        reply.text,
    )
    .map(Some)
}

/// The remote helper takes one line of text: a slash decision, an answer, or `id=answer` rows.
fn format_remote_reply(reply: &Reply) -> Result<String> {
    ensure!(
        reply.reason.is_none_or(|r| r.trim().is_empty()),
        "此审批选项不支持附加说明"
    );
    match &reply.verb {
        Verb::Decide(id) if id == "approve" => Ok("/approve".into()),
        Verb::Decide(id) if id == "decline" => Ok("/decline".into()),
        Verb::Cancel => Ok("/cancel-request".into()),
        Verb::Decide(_) => anyhow::bail!("原会话不允许此审批选项"),
        Verb::Skip => anyhow::bail!("此请求不支持取消或跳过，请在原窗口处理"),
        Verb::Answer | Verb::FollowUp => {
            if let Some(map) = reply.answers.and_then(Value::as_object) {
                ensure!(!map.is_empty(), "问题回答不能为空");
                if map.len() == 1 {
                    let answer = stringify_answer(map.values().next().unwrap());
                    ensure!(!answer.is_empty(), "问题回答不能为空");
                    return Ok(answer);
                }
                let mut rows: Vec<_> = map.iter().collect();
                rows.sort_by_key(|(k, _)| k.parse::<usize>().unwrap_or(usize::MAX));
                let joined = rows
                    .into_iter()
                    .map(|(k, v)| format!("{k}={}", stringify_answer(v)))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(joined);
            }
            let body = reply.text.unwrap_or("").trim();
            ensure!(!body.is_empty(), "输入内容不能为空");
            Ok(body.to_owned())
        }
    }
}

fn stringify_answer(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        _ => String::new(),
    }
}

/// Return a pending hook request to the terminal's own dialog.
fn handoff(store: &Store, params: &Value) -> Result<Value> {
    let lang = language(store);
    let tr = |t: &str| crate::localization::translate(&lang, t).into_owned();
    let refused = |message: String| {
        Ok(json!({"handedOff": false, "confirmed": false, "requestId": "", "message": message}))
    };
    if !config(&store.db)?.enabled {
        return refused(tr("桌宠快捷交互已关闭"));
    }
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let request_id = params["requestId"].as_str().unwrap_or("");
    let Some(target) = session(&store.db, session_id)? else {
        return refused(tr("原会话已结束或不可用"));
    };
    // Only a request that belongs to this session may be released, whatever id was sent.
    let rows = claude::pending_requests(
        store,
        &target.session.source_id,
        &target.session.native_session,
    )?;
    let owned = if request_id.is_empty() {
        rows.first()
    } else {
        rows.iter().find(|r| r.id == request_id)
    };
    let Some(req) = owned else {
        return refused(tr(if request_id.is_empty() {
            "当前没有可交还终端的挂起请求"
        } else {
            "原请求已结束或已被处理"
        }));
    };
    let Some(confirmed) = claude::handoff_request(store, &req.id)? else {
        return refused(tr("原请求已结束或已被处理"));
    };
    Ok(json!({
        "handedOff": true,
        // False while the hook has not yet picked the release up; it does so within a second.
        "confirmed": confirmed,
        "requestId": req.id,
        "message": tr(if confirmed {
            "已交还给原终端确认"
        } else {
            "已交还，等待原终端接管"
        }),
    }))
}

#[cfg(test)]
mod opencode_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::Settings,
        sessions::{ControlProcess, SourceHealth},
    };

    fn make_session(source: &Source, native: &str, phase: Phase) -> LiveSession {
        LiveSession {
            id: crate::models::hash(&format!("local:{}:{native}", source.provider)),
            source: source.provider.clone(),
            source_id: source.id.clone(),
            provider: source.provider.clone(),
            machine_id: "local".into(),
            machine: "本机".into(),
            turn_id: "turn-1".into(),
            phase,
            updated_at: now() as u64,
            event_at: (now() * 1000) as u64,
            native_session: native.into(),
            pid: Some(std::process::id()),
            process_start:
                crate::sessions::focus_target(std::process::id(), 0)["ancestors"][0]["start"]
                    .as_u64(),
            ..Default::default()
        }
    }

    #[test]
    fn desktop_config_defaults_and_claude_hook_sync_respect_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data")).unwrap();
        let claude_home = dir.path().join("claude");
        std::fs::create_dir(&claude_home).unwrap();
        let source = Source {
            id: "claude-src".into(),
            provider: "claude".into(),
            path: claude_home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source],
                ..Default::default()
            })
            .unwrap();
        let settings_path = claude_home.join("settings.json");
        let original = json!({"env": {"KEEP": "1"}});
        std::fs::write(&settings_path, serde_json::to_vec(&original).unwrap()).unwrap();
        // A configuration the user keeps group-readable stays that way when hooks are added.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&settings_path, std::fs::Permissions::from_mode(0o640))
                .unwrap();
        }

        // Default config has claudeQuestions=false and claudeApprovals=false: never modifies settings.json
        let st = call(&store, "sessions.interaction.status", json!({})).unwrap();
        assert_eq!(st["enabled"], true);
        assert_eq!(st["autoPopup"], false);
        assert_eq!(st["claudeQuestions"], false);
        assert_eq!(st["claudeApprovals"], false);
        claude::sync_hooks(&store).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&settings_path).unwrap()).unwrap(),
            original
        );

        // Enabling claudeQuestions installs AskUserQuestion matcher
        call(
            &store,
            "sessions.interaction.configure",
            json!({"claudeQuestions": true}),
        )
        .unwrap();
        let after_q: Value =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(
            after_q["hooks"]["PermissionRequest"][0]["matcher"],
            "AskUserQuestion"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&settings_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o640);
        }

        // Enabling claudeApprovals widens matcher to "*"
        call(
            &store,
            "sessions.interaction.configure",
            json!({"claudeApprovals": true}),
        )
        .unwrap();
        let after_a: Value =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(after_a["hooks"]["PermissionRequest"][0]["matcher"], "*");

        // Disabling both cleanly removes only the owned entry and preserves env
        call(
            &store,
            "sessions.interaction.configure",
            json!({"claudeQuestions": false, "claudeApprovals": false}),
        )
        .unwrap();
        let after_off: Value =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(after_off["env"]["KEEP"], "1");
        assert!(
            after_off["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn claude_pending_approval_inspect_respond_and_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data")).unwrap();
        let claude_home = dir.path().join("claude");
        std::fs::create_dir(&claude_home).unwrap();
        let source = Source {
            id: "claude-src".into(),
            provider: "claude".into(),
            path: claude_home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let native = "11111111-2222-4333-8444-555555555555";
        let mut live = make_session(&source, native, Phase::NeedsInput);
        let pid = live.pid.unwrap();
        let start = live.process_start.unwrap();
        live.control_process = Some(ControlProcess { pid, start });
        observe(
            &store,
            &Snapshot {
                sessions: vec![live.clone()],
                sources: vec![SourceHealth {
                    source_id: source.id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let tool_input = json!({"command": "cargo test --workspace", "description": "Run tests"});
        let hash = claude::approval_hash("Bash", &tool_input);
        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name) VALUES('req-1','',?1,?2,?3,'toolu_1',?4,?5,?6,?7,?7,'approval','Bash')",
            params![source.id, native, hash, tool_input.to_string(), pid, start, now()],
        ).unwrap();

        let inspected = call(
            &store,
            "sessions.interaction.inspect",
            json!({"sessionId": live.id}),
        )
        .unwrap();
        assert_eq!(inspected["kind"], "approval");
        assert_eq!(inspected["adapter"], "claudeHook");
        assert_eq!(inspected["canRespond"], true);
        assert_eq!(inspected["canHandoff"], true);
        assert_eq!(inspected["approval"]["toolName"], "Bash");
        assert_eq!(inspected["approval"]["command"], "cargo test --workspace");

        // Wrong promptHash is rejected by CAS
        let bad = call(
            &store,
            "sessions.interaction.respond",
            json!({
                "sessionId": live.id,
                "stateKey": inspected["stateKey"],
                "promptHash": "wrong",
                "action": "approve"
            }),
        )
        .unwrap();
        assert_eq!(bad["status"], "blocked");

        // Valid handoff writes PASSTHROUGH marker
        let handed = call(
            &store,
            "sessions.interaction.handoff",
            json!({"sessionId": live.id, "requestId": "req-1"}),
        )
        .unwrap();
        assert_eq!(handed["handedOff"], true);
        let stored_resp: String = store
            .db
            .query_row(
                "SELECT response FROM channel_claude_requests WHERE id='req-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_resp, claude::PASSTHROUGH);

        // Insert a second approval and approve it
        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name) VALUES('req-2','',?1,?2,?3,'toolu_2',?4,?5,?6,?7,?7,'approval','Bash')",
            params![source.id, native, hash, tool_input.to_string(), pid, start, now()],
        ).unwrap();
        let ok = call(
            &store,
            "sessions.interaction.respond",
            json!({
                "sessionId": live.id,
                "stateKey": inspected["stateKey"],
                "promptHash": hash,
                "action": "approve"
            }),
        )
        .unwrap();
        assert!(matches!(
            ok["status"].as_str(),
            Some("submitted" | "confirmed")
        ));
        let decision_json: String = store
            .db
            .query_row(
                "SELECT response FROM channel_claude_requests WHERE id='req-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let parsed: Value = serde_json::from_str(&decision_json).unwrap();
        assert_eq!(
            parsed["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
    }

    #[test]
    fn claude_pending_multiselect_question_accepts_structured_answers_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data")).unwrap();
        let claude_home = dir.path().join("claude");
        std::fs::create_dir(&claude_home).unwrap();
        let source = Source {
            id: "claude-src".into(),
            provider: "claude".into(),
            path: claude_home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let native = "22222222-3333-4444-8555-666666666666";
        let mut live = make_session(&source, native, Phase::NeedsInput);
        let pid = live.pid.unwrap();
        let start = live.process_start.unwrap();
        live.control_process = Some(ControlProcess { pid, start });
        observe(
            &store,
            &Snapshot {
                sessions: vec![live.clone()],
                sources: vec![SourceHealth {
                    source_id: source.id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let q_input = json!({
            "questions": [
                {
                    "question": "Select targets",
                    "multiSelect": true,
                    "options": [
                        {"label": "macOS", "description": "Native SwiftUI"},
                        {"label": "Windows", "description": "Tauri desktop", "preview": "fn main() {}"}
                    ]
                }
            ]
        });
        let q_hash = claude::question_hash(&q_input);
        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name) VALUES('q-1','',?1,?2,?3,'toolu_q1',?4,?5,?6,?7,?7,'question','AskUserQuestion')",
            params![source.id, native, q_hash, q_input.to_string(), pid, start, now()],
        ).unwrap();

        let inspected = call(
            &store,
            "sessions.interaction.inspect",
            json!({"sessionId": live.id}),
        )
        .unwrap();
        assert_eq!(inspected["kind"], "questions");
        assert_eq!(inspected["questions"][0]["multiSelect"], true);
        assert_eq!(
            inspected["questions"][0]["options"][1]["preview"],
            "fn main() {}"
        );

        let res = call(
            &store,
            "sessions.interaction.respond",
            json!({
                "sessionId": live.id,
                "stateKey": inspected["stateKey"],
                "promptHash": inspected["promptHash"],
                "action": "answer",
                "answers": { "1": ["macOS", "Windows"] }
            }),
        )
        .unwrap();
        assert!(matches!(
            res["status"].as_str(),
            Some("submitted" | "confirmed")
        ));
        let stored: String = store
            .db
            .query_row(
                "SELECT response FROM channel_claude_requests WHERE id='q-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let parsed: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            parsed["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Select targets"],
            "macOS, Windows"
        );
    }

    #[test]
    fn opencode_in_memory_permission_is_probed_inspected_and_approved() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("xdg");
        let share = base.join("share/opencode");
        let state = base.join("state/opencode");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(share.join("opencode.db"), b"").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            state.join("service.json"),
            json!({"url": format!("http://127.0.0.1:{port}"), "password": "secret"}).to_string(),
        )
        .unwrap();

        let replied = std::sync::Arc::new(std::sync::Mutex::new(None::<Value>));
        let replied_server = replied.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..24 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let first = req.lines().next().unwrap_or("");
                let body_str = req.split("\r\n\r\n").nth(1).unwrap_or("");
                let (code, payload) = if first.starts_with("GET /api/session/ses_demo ") {
                    ("200 OK", json!({"data":{"id":"ses_demo"}}))
                } else if first.starts_with("GET /api/session/ses_demo/form ") {
                    ("200 OK", json!({"data":[]}))
                } else if first.starts_with("GET /api/session/ses_demo/permission ") {
                    (
                        "200 OK",
                        json!({"data":[{"id":"per_1","action":"bash","message":"Run tests","resources":["cargo test"]}]}),
                    )
                } else if first.starts_with("POST /api/session/ses_demo/permission/per_1/reply ") {
                    *replied_server.lock().unwrap() = serde_json::from_str(body_str).ok();
                    ("200 OK", json!({"ok":true}))
                } else {
                    ("404 Not Found", json!({}))
                };
                let body = payload.to_string();
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                if replied_server.lock().unwrap().is_some() {
                    break;
                }
            }
        });

        let store = Store::open(&dir.path().join("data")).unwrap();
        let source = Source {
            id: "opencode-src".into(),
            provider: "opencode".into(),
            path: share.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();

        // Fast loopback probe detects in-memory permission
        let probed = opencode::probe_pending_id(&source, "ses_demo");
        assert_eq!(probed, Some(("permission".into(), "per_1".into())));

        let live = make_session(&source, "ses_demo", Phase::NeedsInput);
        observe(
            &store,
            &Snapshot {
                sessions: vec![live.clone()],
                sources: vec![SourceHealth {
                    source_id: source.id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let inspected = call(
            &store,
            "sessions.interaction.inspect",
            json!({"sessionId": live.id}),
        )
        .unwrap();
        assert_eq!(inspected["kind"], "approval");
        assert_eq!(inspected["adapter"], "opencode");
        assert_eq!(inspected["approval"]["command"], "cargo test");

        let res = call(
            &store,
            "sessions.interaction.respond",
            json!({
                "sessionId": live.id,
                "stateKey": inspected["stateKey"],
                "promptHash": inspected["promptHash"],
                "action": "approve"
            }),
        )
        .unwrap();
        assert_eq!(res["status"], "confirmed");
        let _ = server.join();
        assert_eq!(*replied.lock().unwrap(), Some(json!({"decision": "once"})));
    }

    #[test]
    fn antigravity_json_encoded_questions_are_parsed_into_structured_options() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data")).unwrap();
        let agy_home = dir.path().join("antigravity-cli");
        let native = "014b6d36-4a51-4a26-8c5f-0d7e314aa149";
        let log_dir = agy_home
            .join("brain")
            .join(native)
            .join(".system_generated/logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        let questions_str = json!([
            {
                "is_multi_select": true,
                "options": ["Option A", "Option B"],
                "question": "Pick options?"
            }
        ])
        .to_string();
        let row = json!({
            "type": "PLANNER_RESPONSE",
            "tool_calls": [
                {
                    "name": "ask_question",
                    "args": { "questions": questions_str }
                }
            ]
        });
        std::fs::write(log_dir.join("transcript.jsonl"), format!("{}\n", row)).unwrap();

        let source = Source {
            id: "agy-src".into(),
            provider: "antigravity".into(),
            path: agy_home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let live = make_session(&source, native, Phase::NeedsInput);
        observe(
            &store,
            &Snapshot {
                sessions: vec![live.clone()],
                sources: vec![SourceHealth {
                    source_id: source.id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

        let inspected = call(
            &store,
            "sessions.interaction.inspect",
            json!({"sessionId": live.id}),
        )
        .unwrap();
        assert_eq!(inspected["kind"], "questions");
        assert_eq!(inspected["canRespond"], false);
        assert_eq!(inspected["questions"][0]["question"], "Pick options?");
        assert_eq!(inspected["questions"][0]["multiSelect"], true);
        assert_eq!(inspected["questions"][0]["options"][0]["label"], "Option A");
        assert_eq!(inspected["questions"][0]["options"][1]["label"], "Option B");
    }

    const NATIVE: &str = "11111111-2222-4333-8444-555555555555";
    const OTHER: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

    fn fixture(provider: &str) -> (tempfile::TempDir, Store, Source) {
        // Unix socket paths are short; the default temp directory on macOS is not.
        #[cfg(unix)]
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        #[cfg(not(unix))]
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data")).unwrap();
        let home = dir.path().canonicalize().unwrap().join(provider);
        std::fs::create_dir(&home).unwrap();
        let source = Source {
            id: format!("{provider}-src"),
            provider: provider.into(),
            path: home.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        (dir, store, source)
    }

    /// Record the session as the monitor would and return its control row.
    fn watch(store: &Store, live: &LiveSession) -> Session {
        store
            .db
            .execute("DELETE FROM interaction_sessions WHERE id=?1", [&live.id])
            .unwrap();
        observe(
            store,
            &Snapshot {
                sessions: vec![live.clone()],
                sources: vec![SourceHealth {
                    source_id: live.source_id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        session(&store.db, &live.id).unwrap().unwrap()
    }

    fn inspect_card_of(store: &Store, live: &LiveSession) -> Value {
        call(
            store,
            "sessions.interaction.inspect",
            json!({"sessionId": live.id}),
        )
        .unwrap()
    }

    fn respond_to(store: &Store, card: &Value, extra: Value) -> Value {
        let mut params = json!({
            "sessionId": card["sessionId"],
            "stateKey": card["stateKey"],
            "promptHash": card["promptHash"],
        });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        call(store, "sessions.interaction.respond", params).unwrap()
    }

    fn hook_row(
        store: &Store,
        live: &LiveSession,
        id: &str,
        tool_input: &Value,
        at: i64,
        heartbeat: i64,
    ) -> String {
        let hash = claude::approval_hash("Bash", tool_input);
        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name) VALUES(?1,'',?2,?3,?4,?1,?5,?6,?7,?8,?9,'approval','Bash')",
            params![id, live.source_id, live.native_session, hash, tool_input.to_string(), live.pid.unwrap(), live.process_start.unwrap(), at, heartbeat],
        ).unwrap();
        hash
    }

    fn hook_response(store: &Store, id: &str) -> Option<String> {
        store
            .db
            .query_row(
                "SELECT response FROM channel_claude_requests WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn controlled(source: &Source, native: &str, phase: Phase) -> LiveSession {
        let mut live = make_session(source, native, phase);
        live.control_process = Some(ControlProcess {
            pid: live.pid.unwrap(),
            start: live.process_start.unwrap(),
        });
        live
    }

    #[test]
    fn unverifiable_permission_wait_is_an_approval_card_and_never_accepts_text() {
        let (_dir, store, source) = fixture("claude");
        save_config(
            &store.db,
            &Config {
                allow_terminal: true,
                ..Default::default()
            },
        )
        .unwrap();
        for (native, kind, card_kind) in [
            (NATIVE, "approval", "approval"),
            (OTHER, "elicitation", "readonly"),
        ] {
            let mut live = make_session(&source, native, Phase::NeedsInput);
            live.input_request = "prompt-1".into();
            live.input_kind = kind.into();
            watch(&store, &live);
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["kind"], card_kind);
            assert_eq!(card["inputKind"], kind);
            assert_eq!(card["canRespond"], false);
            assert_eq!(card["adapter"], "focusOnly");
            if kind == "approval" {
                assert_eq!(card["approval"]["decisions"], json!([]));
                assert_eq!(card["needsClaudeApprovals"], true);
                assert_eq!(card["quickEnable"], "approvals");
            }
            // Neither typed text nor a decision may reach the terminal's dialog.
            for extra in [
                json!({"action": "answer", "text": "yes"}),
                json!({"action": "approve"}),
                json!({"action": "decide", "decisionId": "approveAlways"}),
            ] {
                let mut params = extra;
                params["promptHash"] = json!(text_prompt_hash(
                    &session(&store.db, &live.id).unwrap().unwrap(),
                    ""
                ));
                params["capabilities"] = json!({"localTerminal": true});
                let res = respond_to(&store, &card, params);
                assert_eq!(res["status"], "blocked", "{kind}: {res}");
                assert!(res.get("terminalJob").is_none());
            }
        }
    }

    #[test]
    fn terminal_gate_refuses_everything_it_cannot_verify() {
        let (_dir, store, source) = fixture("claude");
        let caps = Capabilities::from_params(&json!({"capabilities": {"localTerminal": true}}));
        let gate = |live: &LiveSession, excerpt: Option<&content::Excerpt>, allow: bool| {
            terminal_gate(&source, &watch(&store, live), excerpt, allow, caps)
        };
        for (phase, block) in [
            (Phase::Working, TerminalBlock::Running),
            (Phase::Thinking, TerminalBlock::Running),
            (Phase::Tool, TerminalBlock::Running),
            (Phase::Unknown, TerminalBlock::UnknownPhase),
        ] {
            assert_eq!(
                gate(&make_session(&source, NATIVE, phase), None, true),
                Err(block)
            );
        }
        let mut waiting = make_session(&source, NATIVE, Phase::NeedsInput);
        waiting.input_request = "prompt".into();
        for kind in ["approval", "elicitation"] {
            waiting.input_kind = kind.into();
            assert_eq!(gate(&waiting, None, true), Err(TerminalBlock::Approval));
        }
        waiting.input_kind = "notification".into();
        for excerpt in [
            content::Excerpt {
                claude_request: Some(json!({"callId": "c", "input": {}})),
                ..Default::default()
            },
            content::Excerpt {
                structured_questions: Some(json!([])),
                ..Default::default()
            },
            content::Excerpt {
                structured_approval: Some(json!({})),
                ..Default::default()
            },
            content::Excerpt {
                async_request: Some(json!({})),
                ..Default::default()
            },
        ] {
            assert_eq!(
                gate(&waiting, Some(&excerpt), true),
                Err(TerminalBlock::Structured)
            );
        }
        let plain = content::Excerpt::default();
        assert_eq!(
            gate(&waiting, Some(&plain), false),
            Err(TerminalBlock::Disabled)
        );
        let mut stale = watch(&store, &waiting);
        stale.observed_at = now() - TERMINAL_FRESH - 1;
        assert_eq!(
            terminal_gate(&source, &stale, Some(&plain), true, caps),
            Err(TerminalBlock::Stale)
        );
        waiting.subagent_count = 1;
        assert_eq!(
            gate(&waiting, Some(&plain), true),
            Err(TerminalBlock::Subagent)
        );
        // A Codex approval is recognised by its state key even without an input kind.
        let mut codex = make_session(&source, NATIVE, Phase::NeedsInput);
        codex.input_request = CODEX_APPROVAL_INPUT.into();
        assert_eq!(gate(&codex, None, true), Err(TerminalBlock::Approval));

        // tmux is driven by the core; the macOS terminal needs a client that can type.
        let none = Capabilities::from_params(&json!({}));
        assert_eq!(
            usable_adapter(Some("macosTerminal"), none),
            Err(TerminalBlock::NoLocalTerminal)
        );
        assert_eq!(
            usable_adapter(Some("macosTerminal"), caps),
            Ok("macosTerminal")
        );
        assert_eq!(usable_adapter(Some("tmux"), none), Ok("tmux"));
        assert_eq!(usable_adapter(None, caps), Err(TerminalBlock::NoAdapter));
    }

    #[test]
    fn generic_cards_show_the_last_message_and_report_follow_up_availability() {
        let (_dir, store, source) = fixture("claude");
        let transcript = std::path::Path::new(&source.path).join("session.jsonl");
        std::fs::write(
            &transcript,
            json!({"type":"assistant","sessionId":NATIVE,"timestamp":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Which database should I use?"}]}}).to_string() + "\n",
        )
        .unwrap();
        let session_in = |phase: Phase| {
            let mut live = make_session(&source, NATIVE, phase);
            live.path = transcript.to_string_lossy().into();
            live.turn_id.clear();
            if phase == Phase::NeedsInput {
                live.input_request = "idle".into();
                live.input_kind = "notification".into();
            }
            live
        };

        let waiting = session_in(Phase::NeedsInput);
        watch(&store, &waiting);
        let card = inspect_card_of(&store, &waiting);
        assert_eq!(card["kind"], "readonly");
        assert_eq!(card["textPrompt"], "Which database should I use?");
        assert_eq!(card["promptSource"], "lastMessage");
        assert_eq!(card["followUp"]["available"], false);
        let res = respond_to(&store, &card, json!({"action": "followup", "text": "next"}));
        assert_eq!(res["status"], "blocked");

        let done = session_in(Phase::Complete);
        watch(&store, &done);
        let card = inspect_card_of(&store, &done);
        assert_eq!(card["kind"], "none");
        assert_eq!(card["textPrompt"], "Which database should I use?");
        assert_eq!(card["followUp"]["available"], false);
        assert_ne!(card["followUp"]["reason"], "");
        // A decision has nothing to decide once the session stopped waiting.
        let res = respond_to(&store, &card, json!({"action": "approve"}));
        assert_eq!(res["status"], "blocked");
        let res = respond_to(&store, &card, json!({"action": "followup", "text": "next"}));
        assert_eq!(res["status"], "blocked");

        let running = session_in(Phase::Working);
        watch(&store, &running);
        let card = inspect_card_of(&store, &running);
        assert_eq!(card["kind"], "none");
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["followUp"]["available"], false);
        assert_eq!(
            card["followUp"]["reason"],
            crate::localization::translate(
                &language(&store),
                "原会话仍在执行，请等待本轮结束后再发送"
            )
            .as_ref()
        );
        let res = respond_to(
            &store,
            &card,
            json!({"action": "followup", "text": "next", "capabilities": {"localTerminal": true}}),
        );
        assert_eq!(res["status"], "blocked");
    }

    #[test]
    fn claude_requests_are_served_oldest_first_and_bound_to_their_session() {
        let (_dir, store, source) = fixture("claude");
        let mut live = controlled(&source, NATIVE, Phase::NeedsInput);
        live.input_request = "approval".into();
        live.input_kind = "approval".into();
        watch(&store, &live);
        let mut other = controlled(&source, OTHER, Phase::NeedsInput);
        other.input_request = "approval".into();
        watch(&store, &other);

        let at = now();
        let old_input = json!({"command": "cargo build"});
        let new_input = json!({"command": "cargo test"});
        let old_hash = hook_row(&store, &live, "req-old", &old_input, at - 3, at);
        let new_hash = hook_row(&store, &live, "req-new", &new_input, at, at);
        hook_row(&store, &other, "req-other", &old_input, at, at);
        // A request whose hook stopped heartbeating is never presented or answered.
        hook_row(
            &store,
            &live,
            "req-dead",
            &json!({"command": "rm"}),
            at - 9,
            at - 30,
        );

        let card = inspect_card_of(&store, &live);
        assert_eq!(card["adapter"], "claudeHook");
        assert_eq!(card["requestId"], "req-old");
        assert_eq!(card["promptHash"], old_hash);
        assert_eq!(card["pendingCount"], 2);
        assert_eq!(card["approval"]["command"], "cargo build");
        assert_eq!(card["approval"]["kind"], "command");
        let ids = card["approval"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["approve", "decline", "declineInterrupt"]);
        assert_eq!(card["approval"]["decisions"][1]["acceptsReason"], true);
        assert_eq!(card["approval"]["decisions"][2]["interrupts"], true);
        assert_eq!(card["approval"]["decisions"][0]["scope"], "once");

        // No state key, a stale one, an unknown request and unoffered decisions are refused.
        for params in [
            json!({"sessionId": live.id, "promptHash": old_hash, "action": "approve"}),
            json!({"sessionId": live.id, "stateKey": "needsInput:old", "promptHash": old_hash, "action": "approve"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "approve", "requestId": "req-other"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "approve", "requestId": "req-dead"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "approveSession"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "approve", "reason": "because"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "answer", "text": "yes"}),
            json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": new_hash, "action": "approve"}),
        ] {
            let res = call(&store, "sessions.interaction.respond", params.clone()).unwrap();
            assert_eq!(res["status"], "blocked", "{params}");
        }
        for id in ["req-old", "req-new", "req-other", "req-dead"] {
            assert_eq!(hook_response(&store, id), None);
        }

        // A named request is answered even when it is not the oldest; the reason travels with it.
        let res = respond_to(
            &store,
            &card,
            json!({"promptHash": new_hash, "requestId": "req-new", "action": "decline", "reason": "use nextest"}),
        );
        assert!(
            matches!(res["status"].as_str(), Some("submitted" | "confirmed")),
            "{res}"
        );
        let stored: Value =
            serde_json::from_str(&hook_response(&store, "req-new").unwrap()).unwrap();
        assert_eq!(stored["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(stored.to_string().contains("use nextest"));
        assert_eq!(hook_response(&store, "req-old"), None);
        let again = respond_to(
            &store,
            &card,
            json!({"promptHash": new_hash, "requestId": "req-new", "action": "approve"}),
        );
        assert_eq!(again["status"], "blocked");

        // Handoff releases only a request of the named session.
        for params in [
            json!({"sessionId": live.id, "requestId": "req-other"}),
            json!({"sessionId": live.id, "requestId": "req-dead"}),
            json!({"sessionId": "missing", "requestId": "req-old"}),
        ] {
            let res = call(&store, "sessions.interaction.handoff", params).unwrap();
            assert_eq!(res["handedOff"], false);
        }
        assert_eq!(hook_response(&store, "req-other"), None);
        assert_eq!(hook_response(&store, "req-old"), None);

        // Disabled interaction answers and releases nothing.
        save_config(
            &store.db,
            &Config {
                enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
        let res = call(
            &store,
            "sessions.interaction.handoff",
            json!({"sessionId": live.id, "requestId": "req-old"}),
        )
        .unwrap();
        assert_eq!(res["handedOff"], false);
        assert!(
            call(
                &store,
                "sessions.interaction.respond",
                json!({"sessionId": live.id, "stateKey": card["stateKey"], "promptHash": old_hash, "action": "approve"}),
            )
            .is_err()
        );
        assert_eq!(inspect_card_of(&store, &live)["canRespond"], false);
        assert_eq!(hook_response(&store, "req-old"), None);

        save_config(&store.db, &Config::default()).unwrap();
        let res = call(
            &store,
            "sessions.interaction.handoff",
            json!({"sessionId": live.id}),
        )
        .unwrap();
        assert_eq!(res["handedOff"], true);
        assert_eq!(res["requestId"], "req-old");
        assert_eq!(
            hook_response(&store, "req-old").as_deref(),
            Some(claude::PASSTHROUGH)
        );
    }

    /// A held request of any kind, as the hook helper registers it.
    #[allow(clippy::too_many_arguments)]
    fn held(
        store: &Store,
        live: &LiveSession,
        id: &str,
        kind: &str,
        tool: &str,
        hash: &str,
        input: &Value,
        meta: Value,
    ) {
        let at = now();
        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name,meta) VALUES(?1,'',?2,?3,?4,?1,?5,?6,?7,?8,?8,?9,?10,?11)",
            params![id, live.source_id, live.native_session, hash, input.to_string(), live.pid.unwrap(), live.process_start.unwrap(), at, kind, tool, meta.to_string()],
        ).unwrap();
    }

    fn decision_ids(card: &Value) -> Vec<String> {
        card["approval"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn kiro_approval_hook_is_opt_in_and_submits_only_one_native_decision() {
        let (_dir, store, source) = fixture("kiro");
        let home = std::path::Path::new(&source.path);
        std::fs::create_dir(home.join("sessions")).unwrap();
        let hook_path = home.join("hooks/aieyes-desktop-approval.json");
        save_config(
            &store.db,
            &Config {
                claude_approvals: true,
                ..Default::default()
            },
        )
        .unwrap();
        claude::sync_hooks(&store).unwrap();
        assert!(!hook_path.exists());
        save_config(
            &store.db,
            &Config {
                kiro_approvals: true,
                ..Default::default()
            },
        )
        .unwrap();
        claude::sync_hooks(&store).unwrap();
        let hook: Value = serde_json::from_slice(&std::fs::read(&hook_path).unwrap()).unwrap();
        assert_eq!(hook["version"], "v1");
        assert_eq!(hook["hooks"][0]["trigger"], "PermissionRequest");
        assert_eq!(hook["hooks"][0]["action"]["type"], "command");

        let mut live = controlled(&source, "sess_cli", Phase::Working);
        let input = json!({"path":"/tmp/fixture.txt"});
        let meta = json!({"provider":"kiro","permission":{"capability":"fs_read","paths":["/tmp/fixture.txt"]}});
        for (action, behavior) in [("approve", "allow"), ("decline", "deny")] {
            live.phase = Phase::Working;
            held(
                &store,
                &live,
                action,
                "approval",
                "read_file",
                "read-hash",
                &input,
                meta.clone(),
            );
            promote_hook_requests(
                std::slice::from_mut(&mut live),
                &pending_hook_requests(&store.db),
            );
            assert_eq!(live.phase, Phase::NeedsInput);
            watch(&store, &live);
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["requestId"], action);
            assert_eq!(card["approval"]["paths"], json!(["/tmp/fixture.txt"]));
            assert_eq!(decision_ids(&card), ["approve", "decline"]);
            for invalid in [
                json!({"action":action}),
                json!({"action":"approveAlways", "requestId":action}),
            ] {
                assert_eq!(respond_to(&store, &card, invalid)["status"], "blocked");
            }
            let result = respond_to(&store, &card, json!({"action":action,"requestId":action}));
            assert_eq!(result["status"], "submitted");
            let reply: Value =
                serde_json::from_str(&hook_response(&store, action).unwrap()).unwrap();
            assert_eq!(
                reply,
                json!({"hookSpecificOutput":{"decision":{"behavior":behavior}}})
            );
        }
        save_config(&store.db, &Config::default()).unwrap();
        claude::sync_hooks(&store).unwrap();
        let hook: Value = serde_json::from_slice(&std::fs::read(hook_path).unwrap()).unwrap();
        assert_eq!(hook["hooks"], json!([]));
    }

    #[test]
    fn claude_plan_and_suggested_permissions_are_answered_with_their_native_payload() {
        let (_dir, store, source) = fixture("claude");
        let mut live = controlled(&source, NATIVE, Phase::NeedsInput);
        live.input_kind = "approval".into();
        watch(&store, &live);
        let plan = json!({"plan": "# Plan\n\n- step"});
        held(
            &store,
            &live,
            "plan",
            "approval",
            "ExitPlanMode",
            "plan-hash",
            &plan,
            json!({"provider": "claude"}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["approval"]["kind"], "plan");
        assert_eq!(card["approval"]["plan"]["markdown"], "# Plan\n\n- step");
        assert_eq!(
            decision_ids(&card),
            [
                "approve",
                "approveAcceptEdits",
                "decline",
                "declineInterrupt"
            ]
        );
        let res = respond_to(
            &store,
            &card,
            json!({"requestId": "plan", "action": "decide", "decisionId": "approveAcceptEdits"}),
        );
        assert!(
            matches!(res["status"].as_str(), Some("submitted" | "confirmed")),
            "{res}"
        );
        // The helper holds the full input and adds it as `updatedInput` when it delivers.
        let stored: Value = serde_json::from_str(&hook_response(&store, "plan").unwrap()).unwrap();
        assert_eq!(stored["aieyesEchoInput"], true);
        assert_eq!(
            stored["hookSpecificOutput"]["decision"],
            json!({"behavior": "allow", "updatedPermissions": [
                {"type": "setMode", "mode": "acceptEdits", "destination": "session"}]})
        );

        // Suggestions recorded by the helper become decisions; a client cannot invent one.
        let input = json!({"command": "git init"});
        held(
            &store,
            &live,
            "bash",
            "approval",
            "Bash",
            "bash-hash",
            &input,
            json!({"provider": "claude", "suggestions": [
                {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "git init *"}], "behavior": "allow", "destination": "localSettings"}]}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["requestId"], "bash");
        assert_eq!(
            decision_ids(&card),
            ["approve", "approveAlways", "decline", "declineInterrupt"]
        );
        assert_eq!(card["approval"]["decisions"][1]["scope"], "always");
        assert!(
            card["approval"]["decisions"][1]["description"]
                .as_str()
                .unwrap()
                .starts_with("Bash(git init *)")
        );
        for extra in [
            json!({"action": "approveSession"}),
            json!({"action": "approveAcceptEdits"}),
            json!({"action": "approveAlways", "updatedPermissions": [{"type": "setMode", "mode": "bypassPermissions"}], "reason": "x"}),
        ] {
            assert_eq!(respond_to(&store, &card, extra)["status"], "blocked");
        }
        assert_eq!(hook_response(&store, "bash"), None);
        let res = respond_to(&store, &card, json!({"action": "approveAlways"}));
        assert_ne!(res["status"], "blocked", "{res}");
        let stored: Value = serde_json::from_str(&hook_response(&store, "bash").unwrap()).unwrap();
        assert_eq!(
            stored["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["rules"][0]["ruleContent"],
            "git init *"
        );
        assert!(stored.get("aieyesEchoInput").is_none());

        held(
            &store,
            &live,
            "stop",
            "approval",
            "Bash",
            "stop-hash",
            &input,
            json!({"provider": "claude"}),
        );
        let card = inspect_card_of(&store, &live);
        let res = respond_to(&store, &card, json!({"action": "declineInterrupt"}));
        assert_ne!(res["status"], "blocked", "{res}");
        let stored: Value = serde_json::from_str(&hook_response(&store, "stop").unwrap()).unwrap();
        assert_eq!(
            stored["hookSpecificOutput"]["decision"],
            json!({"behavior": "deny", "interrupt": true})
        );
    }

    #[test]
    fn mcp_forms_are_filled_skipped_or_cancelled_through_the_elicitation_hook() {
        let (_dir, store, source) = fixture("qoder");
        let mut live = controlled(&source, NATIVE, Phase::NeedsInput);
        live.input_kind = "elicitation".into();
        watch(&store, &live);
        let form = json!({"message": "Deploy where?", "server": "ops",
        "order": ["env", "replicas", "notify"], "requested_schema": {
        "type": "object", "required": ["env"], "properties": {
            "env": {"type": "string", "title": "Environment", "enum": ["prod", "stg"], "enumNames": ["Production", "Staging"]},
            "replicas": {"type": "integer", "minimum": 1, "maximum": 3},
            "notify": {"type": "boolean", "title": "Notify"}
        }}});
        // A form the card cannot show is skipped, not shown as an empty card.
        held(
            &store,
            &live,
            "nested",
            "elicitation",
            "Elicitation",
            "n",
            &json!({"message": "?", "requested_schema": {"type": "object", "properties": {"a": {"type": "object"}}}}),
            json!({"provider": "qoder"}),
        );
        for id in ["form-1", "form-2", "form-3"] {
            held(
                &store,
                &live,
                id,
                "elicitation",
                "Elicitation",
                "f",
                &form,
                json!({"provider": "qoder"}),
            );
        }
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "questions");
        assert_eq!(card["adapter"], "claudeHook");
        assert_eq!(card["requestId"], "form-1");
        assert_eq!(card["canHandoff"], true);
        assert_eq!(card["form"]["source"], "mcp");
        assert_eq!(card["form"]["serverName"], "ops");
        assert_eq!(card["form"]["description"], "Deploy where?");
        assert_eq!(card["form"]["canCancel"], true);
        assert_eq!(card["form"]["canSkip"], true);
        let questions = card["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 3);
        assert_eq!(questions[0]["key"], "env");
        assert_eq!(
            questions[0]["options"][0],
            json!({"label": "Production", "value": "prod"})
        );
        assert_eq!(questions[0]["allowCustom"], false);
        assert_eq!(questions[1]["kind"], "integer");
        assert_eq!(questions[1]["required"], false);
        assert_eq!(
            questions[1]["constraints"],
            json!({"minimum": 1, "maximum": 3})
        );
        assert_eq!(questions[2]["options"][1]["value"], false);

        for extra in [
            json!({"action": "answer", "answers": {"1": "Moon"}}),
            json!({"action": "answer", "answers": {"2": 2}}),
            json!({"action": "answer", "answers": {"1": "Production", "2": 9}}),
            json!({"action": "answer", "answers": {"1": "Production", "9": "x"}}),
            json!({"action": "approve"}),
        ] {
            let res = respond_to(&store, &card, extra.clone());
            assert_eq!(res["status"], "blocked", "{extra}");
        }
        assert_eq!(hook_response(&store, "form-1"), None);

        let res = respond_to(
            &store,
            &card,
            json!({"requestId": "form-1", "action": "answer", "answers": {"1": "Production", "2": "2", "3": false}}),
        );
        assert_ne!(res["status"], "blocked", "{res}");
        let stored: Value =
            serde_json::from_str(&hook_response(&store, "form-1").unwrap()).unwrap();
        assert_eq!(
            stored,
            json!({"hookSpecificOutput": {"hookEventName": "Elicitation", "action": "accept",
                "content": {"env": "prod", "replicas": 2, "notify": false}}})
        );
        // A reply made for one form is never delivered to another.
        let stale = respond_to(
            &store,
            &card,
            json!({"requestId": "form-2", "action": "skip"}),
        );
        assert_eq!(stale["status"], "blocked");
        for (id, action, native) in [
            ("form-2", "skip", "decline"),
            ("form-3", "cancel", "cancel"),
        ] {
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["requestId"], id);
            let res = respond_to(&store, &card, json!({"requestId": id, "action": action}));
            assert_ne!(res["status"], "blocked", "{res}");
            let stored: Value = serde_json::from_str(&hook_response(&store, id).unwrap()).unwrap();
            assert_eq!(
                stored["hookSpecificOutput"],
                json!({"hookEventName": "Elicitation", "action": native})
            );
        }
        // Only the unpresentable form is left: it is never offered for a reply.
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["requestId"], "");
        assert_eq!(hook_response(&store, "nested"), None);
        // Without a held request the card says how to get one.
        let mut later = controlled(&source, OTHER, Phase::NeedsInput);
        later.input_kind = "elicitation".into();
        watch(&store, &later);
        let card = inspect_card_of(&store, &later);
        assert_eq!(card["kind"], "readonly");
        assert_eq!(card["needsClaudeHook"], true);
        assert_eq!(card["quickEnable"], "questions");
    }

    #[test]
    fn codebuddy_and_zcode_approvals_use_the_decision_hook_with_their_own_limits() {
        for (provider, expected) in [
            ("codebuddy", vec!["approve", "decline"]),
            ("zcode", vec!["approve", "decline"]),
        ] {
            let (_dir, store, source) = fixture(provider);
            // The monitor binds no process to these agents; the hook row carries it.
            let mut live = make_session(&source, NATIVE, Phase::NeedsInput);
            live.input_kind = "approval".into();
            watch(&store, &live);
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["canRespond"], false);
            assert_eq!(card["needsClaudeApprovals"], true, "{provider}");
            assert_eq!(card["quickEnable"], "approvals");

            let input = json!({"command": "npm test"});
            held(
                &store,
                &live,
                "req",
                "approval",
                "Bash",
                "h",
                &input,
                json!({"provider": provider, "suggestions": [
                    {"type": "setMode", "mode": "acceptEdits", "destination": "session"}]}),
            );
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["adapter"], "claudeHook", "{provider}");
            assert_eq!(decision_ids(&card), expected);
            assert_eq!(
                respond_to(&store, &card, json!({"action": "approveAcceptEdits"}))["status"],
                "blocked"
            );
            let res = respond_to(
                &store,
                &card,
                json!({"action": "decline", "reason": "not now"}),
            );
            assert_ne!(res["status"], "blocked", "{provider}: {res}");
            let stored: Value =
                serde_json::from_str(&hook_response(&store, "req").unwrap()).unwrap();
            assert_eq!(
                stored["hookSpecificOutput"],
                json!({"hookEventName": "PermissionRequest",
                    "decision": {"behavior": "deny", "message": "not now"}})
            );

            // A row whose agent process is gone is refused.
            store
                .db
                .execute(
                    "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name,meta) VALUES('gone','',?1,?2,'g','gone','{}',?3,1,?4,?4,'approval','Bash',?5)",
                    params![live.source_id, live.native_session, live.pid.unwrap(), now(), json!({"provider": provider}).to_string()],
                )
                .unwrap();
            let card = inspect_card_of(&store, &live);
            assert_eq!(card["requestId"], "gone");
            assert_eq!(
                respond_to(&store, &card, json!({"action": "approve"}))["status"],
                "blocked"
            );
            assert_eq!(hook_response(&store, "gone"), None);
        }
    }

    #[test]
    fn held_hook_requests_surface_sessions_whose_transcript_still_says_running() {
        let (_dir, store, source) = fixture("qoder");
        let at = now();
        let running = controlled(&source, NATIVE, Phase::Tool);
        hook_row(
            &store,
            &running,
            "req-new",
            &json!({"command": "b"}),
            at,
            at,
        );
        hook_row(
            &store,
            &running,
            "req-old",
            &json!({"command": "a"}),
            at - 1,
            at,
        );
        let dead = controlled(&source, OTHER, Phase::Tool);
        hook_row(
            &store,
            &dead,
            "req-dead",
            &json!({"command": "c"}),
            at,
            at - 30,
        );

        let pending = pending_hook_requests(&store.db);
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[&(source.id.clone(), NATIVE.to_owned())].id,
            "req-old"
        );

        let mut hooked = running.clone();
        hooked.evidence = "hook".into();
        let mut answered = running.clone();
        answered.event_at = ((at + 10) * 1000) as u64;
        let mut finished = running.clone();
        finished.phase = Phase::Complete;
        let mut background = running.clone();
        background.background = true;
        let mut remote = running.clone();
        remote.machine_id = "host".into();
        let mut sessions = vec![
            running.clone(),
            dead,
            hooked,
            answered,
            finished,
            background,
            remote,
        ];
        promote_hook_requests(&mut sessions, &pending);
        assert_eq!(sessions[0].phase, Phase::NeedsInput);
        assert_eq!(sessions[0].input_request, "claudeHook:req-old");
        assert_eq!(sessions[0].input_kind, "approval");
        assert!(sessions[1..].iter().all(|s| s.phase != Phase::NeedsInput));
        assert!(sessions[1..].iter().all(|s| s.input_request.is_empty()));

        // A held MCP form surfaces as one, and agents that are silent until their hook returns
        // are surfaced even when their own observer hooks are installed.
        store
            .db
            .execute("UPDATE channel_claude_requests SET kind='elicitation'", [])
            .unwrap();
        let pending = pending_hook_requests(&store.db);
        let mut sessions = vec![running.clone()];
        promote_hook_requests(&mut sessions, &pending);
        assert_eq!(sessions[0].input_kind, "elicitation");
        let mut silent = running.clone();
        silent.provider = "codebuddy".into();
        silent.evidence = "hook".into();
        let mut unrelated = running.clone();
        unrelated.provider = "kimi".into();
        let mut sessions = vec![silent, unrelated];
        promote_hook_requests(&mut sessions, &pending);
        assert_eq!(sessions[0].phase, Phase::NeedsInput);
        assert_eq!(sessions[1].phase, Phase::Tool);
    }

    #[test]
    fn remote_replies_carry_only_what_the_helper_can_deliver() {
        fn reply<'a>(
            verb: Verb,
            answers: Option<&'a Value>,
            text: Option<&'a str>,
            reason: Option<&'a str>,
        ) -> Reply<'a> {
            Reply {
                verb,
                answers,
                text,
                reason,
                hash: None,
            }
        }
        let decide = |id: &str| Verb::Decide(id.into());
        let line = |r: Reply| format_remote_reply(&r).ok();
        assert_eq!(
            line(reply(decide("approve"), None, None, None)).as_deref(),
            Some("/approve")
        );
        assert_eq!(
            line(reply(decide("decline"), None, None, None)).as_deref(),
            Some("/decline")
        );
        assert_eq!(
            line(reply(Verb::Cancel, None, None, None)).as_deref(),
            Some("/cancel-request")
        );
        // A scope the helper cannot express must not degrade to a one-off approval.
        assert_eq!(
            line(reply(decide("approveSession"), None, None, None)),
            None
        );
        assert_eq!(line(reply(decide("approveAlways"), None, None, None)), None);
        assert_eq!(line(reply(Verb::Skip, None, None, None)), None);
        assert_eq!(
            line(reply(decide("decline"), None, None, Some("why"))),
            None
        );
        // Text that looks like a decision is still sent as the text the user typed.
        assert_eq!(
            line(reply(Verb::Answer, None, Some(" hello "), None)).as_deref(),
            Some("hello")
        );
        assert_eq!(line(reply(Verb::Answer, None, Some("  "), None)), None);
        let one = json!({"1": ["A", "B"]});
        assert_eq!(
            line(reply(Verb::Answer, Some(&one), None, None)).as_deref(),
            Some("A, B")
        );
        let many = json!({"2": "B", "1": "A", "10": "C"});
        assert_eq!(
            line(reply(Verb::Answer, Some(&many), None, None)).as_deref(),
            Some("1=A\n2=B\n10=C")
        );
        assert_eq!(
            line(reply(Verb::Answer, Some(&json!({})), None, None)),
            None
        );
        assert_eq!(
            line(reply(Verb::Answer, Some(&json!({"1": ""})), None, None)),
            None
        );
    }

    #[test]
    fn delivered_requests_lose_their_input_and_answer_but_keep_their_receipt() {
        let (_dir, store, source) = fixture("claude");
        let live = controlled(&source, NATIVE, Phase::NeedsInput);
        let at = now();
        hook_row(
            &store,
            &live,
            "done",
            &json!({"command": "secret"}),
            at - 200,
            at - 100,
        );
        hook_row(
            &store,
            &live,
            "fresh",
            &json!({"command": "secret"}),
            at,
            at - 10,
        );
        hook_row(
            &store,
            &live,
            "open",
            &json!({"command": "secret"}),
            at,
            at - 100,
        );
        store
            .db
            .execute(
                "UPDATE channel_claude_requests SET consumed=1,response='{\"a\":1}' WHERE id IN ('done','fresh')",
                [],
            )
            .unwrap();
        scrub_consumed(&store.db, at).unwrap();
        let row = |id: &str| -> (String, Option<String>, i64) {
            store
                .db
                .query_row(
                    "SELECT input,response,consumed FROM channel_claude_requests WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap()
        };
        assert_eq!(row("done"), ("{}".into(), Some(String::new()), 1));
        assert!(row("fresh").0.contains("secret"));
        assert!(row("open").0.contains("secret"));
        assert_eq!(row("open").1, None);
    }

    #[test]
    fn answered_marker_is_bound_to_one_request_and_expires() {
        let (_dir, store, _source) = fixture("claude");
        let key = "needsInput:turn:abc";
        let recent = |session: &str, key: &str| recently_answered(&store.db, session, key, "", "");
        mark_answered(&store.db, "session", key, None, None);
        assert!(recent("session", key));
        assert!(!recent("session", "needsInput:turn:next"));
        assert!(!recent("other", key));
        assert!(!recent("session", ""));
        forget_answered(&store.db, now()).unwrap();
        assert!(recent("session", key));
        forget_answered(&store.db, now() + ANSWERED_TTL).unwrap();
        assert!(!recent("session", key));

        // Two requests under one state key: only the answered one counts as submitted.
        mark_answered(&store.db, "session", key, Some("hash-a"), Some("req-a"));
        assert!(recently_answered(
            &store.db, "session", key, "hash-a", "req-a"
        ));
        assert!(recently_answered(&store.db, "session", key, "", ""));
        assert!(!recently_answered(&store.db, "session", key, "hash-b", ""));
        assert!(!recently_answered(
            &store.db, "session", key, "hash-a", "req-b"
        ));
    }

    /// Every Codex approval of a turn shares one state key. Once the monitor has seen the
    /// session leave the answered wait, the next approval is not reported as submitted.
    #[test]
    fn answered_marker_is_dropped_when_the_session_leaves_the_wait() {
        let (_dir, store, source) = fixture("codex");
        let mut waiting = make_session(&source, NATIVE, Phase::NeedsInput);
        waiting.input_request = CODEX_APPROVAL_INPUT.into();
        waiting.input_kind = "approval".into();
        let observe_one = |live: &LiveSession| {
            observe(
                &store,
                &Snapshot {
                    sessions: vec![live.clone()],
                    sources: vec![SourceHealth {
                        source_id: live.source_id.clone(),
                        machine_id: "local".into(),
                        state: "ready".into(),
                        checked_at: now() as u64,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            session(&store.db, &live.id).unwrap().unwrap()
        };
        let target = observe_one(&waiting);
        mark_answered(&store.db, &waiting.id, &target.state_key, None, None);
        // The same wait observed again keeps the marker for the lagging card.
        store
            .db
            .execute(
                "UPDATE interaction_sessions SET observed_at=observed_at-10",
                [],
            )
            .unwrap();
        observe_one(&waiting);
        assert!(recently_answered(
            &store.db,
            &waiting.id,
            &target.state_key,
            "",
            ""
        ));
        let working = make_session(&source, NATIVE, Phase::Working);
        observe_one(&working);
        let again = observe_one(&waiting);
        assert_eq!(again.state_key, target.state_key);
        assert!(!recently_answered(
            &store.db,
            &waiting.id,
            &again.state_key,
            "",
            ""
        ));
    }

    /// A Codex control service that serves every connection and holds one approval request
    /// until it is answered.
    #[cfg(unix)]
    fn codex_service(
        root: &std::path::Path,
    ) -> (
        std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        codex_service_holding(
            root,
            json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":NATIVE,"turnId":"turn-1","itemId":"item-1","command":"cargo test","cwd":"/fixture","reason":"Run the tests","availableDecisions":["accept","acceptForSession",{"acceptWithExecpolicyAmendment":{}},"decline"]}}),
        )
    }

    /// The same service holding `request` (its id must be 7).
    #[cfg(unix)]
    fn codex_service_holding(
        root: &std::path::Path,
        request: Value,
    ) -> (
        std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        use std::os::unix::net::UnixListener;
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let socket = root.join("app-server-control");
        std::fs::create_dir_all(&socket).unwrap();
        let listener = UnixListener::bind(socket.join("app-server-control.sock")).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(AtomicBool::new(true));
        let (log, open) = (received.clone(), pending.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let (log, open, request) = (log.clone(), open.clone(), request.clone());
                std::thread::spawn(move || {
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let Ok(mut ws) = tungstenite::accept(stream) else {
                        return;
                    };
                    let send = |ws: &mut tungstenite::WebSocket<_>, value: Value| {
                        ws.send(tungstenite::Message::Text(value.to_string().into()))
                            .is_ok()
                    };
                    while let Ok(message) = ws.read() {
                        let Ok(text) = message.to_text() else {
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<Value>(text) else {
                            continue;
                        };
                        log.lock().unwrap().push(value.clone());
                        let Some(method) = value["method"].as_str() else {
                            if value["id"] == 7 {
                                open.store(false, Ordering::SeqCst);
                                send(
                                    &mut ws,
                                    json!({"method":"serverRequest/resolved","params":{"threadId":NATIVE,"requestId":7}}),
                                );
                            }
                            continue;
                        };
                        if method == "initialized" {
                            continue;
                        }
                        let waiting = open.load(Ordering::SeqCst);
                        let thread = json!({"id":NATIVE,"cwd":"/fixture","status":{"type":if waiting {"active"} else {"idle"}}});
                        let result = match method {
                            "thread/read" | "thread/resume" => json!({"thread": thread}),
                            "thread/turns/list" => json!({"data":[{"id":"turn-1"}]}),
                            "thread/loaded/list" => json!({"data":[NATIVE]}),
                            "turn/start" => {
                                json!({"turn":{"id":"turn-2","status":"inProgress","error":null}})
                            }
                            _ => json!({}),
                        };
                        if !send(&mut ws, json!({"id": value["id"], "result": result})) {
                            break;
                        }
                        if method == "thread/resume" && waiting {
                            send(&mut ws, request.clone());
                        }
                    }
                });
            }
        });
        (received, pending)
    }

    #[test]
    #[cfg(unix)]
    fn codex_approval_offers_listed_decisions_and_follow_up_reuses_the_thread() {
        let (_dir, store, source) = fixture("codex");
        let (received, pending) = codex_service(std::path::Path::new(&source.path));
        let mut live = make_session(&source, NATIVE, Phase::NeedsInput);
        live.input_request = CODEX_APPROVAL_INPUT.into();
        live.input_kind = "approval".into();
        watch(&store, &live);

        let status = call(&store, "sessions.interaction.status", json!({})).unwrap();
        assert_eq!(
            status["codexControl"],
            json!([{"sourceId": source.id, "available": true}])
        );

        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "approval");
        assert_eq!(card["adapter"], "codex");
        assert_eq!(card["canRespond"], true);
        assert_eq!(card["approval"]["kind"], "command");
        assert_eq!(card["approval"]["command"], "cargo test");
        assert_eq!(card["approval"]["cwd"], "/fixture");
        let decisions = card["approval"]["decisions"].as_array().unwrap();
        let ids = decisions
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        // Only string decisions the request lists; the amendment object has no safe mapping.
        assert_eq!(ids, ["approve", "approveSession", "decline"]);
        assert_eq!(decisions[1]["scope"], "session");
        assert!(decisions.iter().all(|d| d.get("native").is_none()));

        for extra in [
            json!({"action": "cancel"}),
            json!({"action": "approveAlways"}),
            json!({"action": "decline", "reason": "no"}),
            json!({"action": "answer", "text": "/approve"}),
            json!({"action": "approve", "promptHash": "stale"}),
        ] {
            let res = respond_to(&store, &card, extra.clone());
            assert_eq!(res["status"], "blocked", "{extra}");
        }
        assert!(pending.load(Ordering::SeqCst));

        let res = respond_to(&store, &card, json!({"action": "approveSession"}));
        assert_eq!(res["status"], "confirmed", "{res}");
        assert!(
            received
                .lock()
                .unwrap()
                .iter()
                .any(|v| v["id"] == 7 && v["result"] == json!({"decision": "acceptForSession"}))
        );
        assert_eq!(
            received
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v["id"] == 7 && v["method"].is_null())
                .count(),
            1
        );

        // The session monitor has not caught up yet: the answered request is not shown again
        // as an approval that cannot be answered.
        let after = inspect_card_of(&store, &live);
        assert_eq!(after["kind"], "none");
        assert_eq!(after["answered"], true);
        assert_eq!(after["approval"], Value::Null);
        assert_eq!(after["canRespond"], false);

        // An unanswerable approval without that marker stays an approval card with no buttons.
        store
            .db
            .execute("DELETE FROM kv WHERE key LIKE 'interaction.answered:%'", [])
            .unwrap();
        let orphan = inspect_card_of(&store, &live);
        assert_eq!(orphan["kind"], "approval");
        assert_eq!(orphan["canRespond"], false);
        assert_eq!(orphan["approval"]["decisions"], json!([]));

        // Finished turn: a follow-up starts a new turn on the same thread.
        let done = make_session(&source, NATIVE, Phase::Complete);
        watch(&store, &done);
        let card = inspect_card_of(&store, &done);
        assert_eq!(card["kind"], "followup");
        assert_eq!(card["canRespond"], true);
        assert_eq!(
            card["followUp"],
            json!({
                "available": true,
                "adapter": "codex",
                "mode": "newTurn",
                "reason": "",
                "canInterrupt": false
            })
        );
        let res = respond_to(&store, &card, json!({"action": "approve"}));
        assert_eq!(res["status"], "blocked");
        let res = respond_to(
            &store,
            &card,
            json!({"action": "followup", "text": "Now add tests"}),
        );
        assert_eq!(res["status"], "confirmed", "{res}");
        let sent = received.lock().unwrap();
        let start = sent.iter().find(|v| v["method"] == "turn/start").unwrap();
        assert_eq!(start["params"]["threadId"], NATIVE);
        assert_eq!(start["params"]["input"][0]["text"], "Now add tests");
    }

    /// Requests as codex-cli 0.172.0 sends them, held by the fake service.
    #[cfg(unix)]
    fn codex_waiting(
        request: Value,
    ) -> (
        tempfile::TempDir,
        Store,
        LiveSession,
        std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    ) {
        let (dir, store, source) = fixture("codex");
        let (received, _pending) =
            codex_service_holding(std::path::Path::new(&source.path), request);
        let mut live = make_session(&source, NATIVE, Phase::NeedsInput);
        live.input_request = CODEX_APPROVAL_INPUT.into();
        live.input_kind = "approval".into();
        watch(&store, &live);
        (dir, store, live, received)
    }

    #[cfg(unix)]
    fn codex_answer(received: &std::sync::Mutex<Vec<Value>>) -> Vec<Value> {
        received
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["id"] == 7 && v["method"].is_null())
            .map(|v| v["result"].clone())
            .collect()
    }

    #[test]
    #[cfg(unix)]
    fn codex_amendment_and_stop_decisions_reach_the_thread_as_listed() {
        let amendment =
            json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["touch","hello.txt"]}});
        let (_dir, store, live, received) = codex_waiting(
            json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"kind":"command","threadId":NATIVE,"turnId":"turn-1","itemId":"call_1","command":"/bin/zsh -lc 'touch hello.txt'","cwd":"/fixture","proposedExecpolicyAmendment":["touch","hello.txt"],"availableDecisions":["accept",amendment.clone(),"cancel"]}}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["pendingCount"], 1);
        let decisions = card["approval"]["decisions"].as_array().unwrap();
        let ids: Vec<_> = decisions
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        // `decline` is not listed by this server but is always a valid answer to a command.
        assert_eq!(ids, ["approve", "approveAlways", "decline", "cancel"]);
        assert_eq!(decisions[1]["scope"], "always");
        assert!(
            decisions[1]["description"]
                .as_str()
                .unwrap()
                .ends_with("touch hello.txt")
        );
        assert_eq!(decisions[3]["interrupts"], true);
        // Not offered here, and a refusal takes no note on this protocol.
        for extra in [
            json!({"action": "approveSession"}),
            json!({"action": "decline", "reason": "no"}),
        ] {
            assert_eq!(respond_to(&store, &card, extra)["status"], "blocked");
        }
        let res = respond_to(&store, &card, json!({"action": "approveAlways"}));
        assert_eq!(res["status"], "confirmed", "{res}");
        assert_eq!(codex_answer(&received), [json!({"decision": amendment})]);
    }

    #[test]
    #[cfg(unix)]
    fn codex_permission_and_mcp_tool_requests_are_answered_in_their_own_shape() {
        let (_dir, store, live, received) = codex_waiting(
            json!({"id":7,"method":"item/permissions/requestApproval","params":{"threadId":NATIVE,"turnId":"turn-1","itemId":"call_2","environmentId":"local","cwd":"/fixture","reason":"need to fetch data","permissions":{"network":{"enabled":true},"fileSystem":{"read":null,"write":["/tmp/probe-out"],"entries":[{"path":{"type":"path","path":"/tmp/probe-out"},"access":"write"}]}}}}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "approval");
        assert_eq!(card["approval"]["kind"], "permissions");
        assert_eq!(card["approval"]["reason"], "need to fetch data");
        let decisions = card["approval"]["decisions"].as_array().unwrap();
        let ids: Vec<_> = decisions
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["approve", "approveSession", "decline"]);
        assert_eq!(decisions[0]["scope"], "turn");
        assert_eq!(
            respond_to(&store, &card, json!({"action": "cancel"}))["status"],
            "blocked"
        );
        let res = respond_to(&store, &card, json!({"action": "decline"}));
        assert_eq!(res["status"], "confirmed", "{res}");
        // Refusing grants nothing; a `{"decision": ...}` reply would not parse here.
        assert_eq!(
            codex_answer(&received),
            [json!({"permissions": {}, "scope": "turn"})]
        );

        // The thread's turn is unknown to an MCP server's request, so its turn id may be null.
        let (_dir, store, live, received) = codex_waiting(
            json!({"id":7,"method":"mcpServer/elicitation/request","params":{"threadId":NATIVE,"turnId":null,"serverName":"probe","mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"],"tool_name":"delete_record","tool_description":"Delete a record by id (destructive).","tool_params":{"id":"42"},"tool_params_display":[{"name":"id","value":"42","display_name":"id"}]},"message":"Allow the probe MCP server to run tool \"delete_record\"?","requestedSchema":{"type":"object","properties":{}}}}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "approval");
        assert_eq!(card["approval"]["kind"], "mcpTool");
        let ids: Vec<_> = card["approval"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            ids,
            [
                "approve",
                "approveSession",
                "approveAlways",
                "decline",
                "cancel"
            ]
        );
        let res = respond_to(&store, &card, json!({"action": "approveSession"}));
        assert_eq!(res["status"], "confirmed", "{res}");
        assert_eq!(
            codex_answer(&received),
            [json!({"action": "accept", "content": {}, "_meta": {"persist": "session"}})]
        );
    }

    #[test]
    #[cfg(unix)]
    fn codex_mcp_form_is_a_question_card_and_keeps_typed_answers() {
        let (_dir, store, live, received) = codex_waiting(
            json!({"id":7,"method":"mcpServer/elicitation/request","params":{"threadId":NATIVE,"turnId":"turn-1","serverName":"probe","mode":"form","_meta":null,"message":"Choose deployment settings","requestedSchema":{"type":"object","properties":{"features":{"type":"array","title":"Features","minItems":1,"items":{"type":"string","enum":["cache","cdn","waf"]}},"notify":{"type":"boolean","title":"Notify team","default":true},"region":{"type":"string","title":"Region","enum":["us-east","eu-west"],"enumNames":["US East","EU West"]},"replicas":{"type":"integer","title":"Replicas","minimum":1,"maximum":5,"default":2}},"required":["region","replicas"]}}}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "questions");
        assert_eq!(card["inputKind"], "elicitation");
        assert_eq!(card["canRespond"], true);
        assert_eq!(card["form"]["serverName"], "probe");
        let questions = card["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 4);
        let id = |title: &str| {
            questions
                .iter()
                .find(|q| q["header"] == title || q["question"] == title)
                .unwrap_or_else(|| panic!("{title} in {questions:?}"))["id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        // A form is not an approval, and an incomplete answer is not sent.
        for extra in [
            json!({"action": "approve"}),
            json!({"action": "answer", "answers": {id("Features"): ["cache"]}}),
        ] {
            assert_eq!(
                respond_to(&store, &card, extra.clone())["status"],
                "blocked",
                "{extra}"
            );
        }
        assert!(codex_answer(&received).is_empty());
        let mut answers = serde_json::Map::new();
        answers.insert(id("Region"), json!("EU West"));
        answers.insert(id("Replicas"), json!("3"));
        answers.insert(id("Features"), json!(["cache", "waf"]));
        answers.insert(id("Notify team"), json!(false));
        let res = respond_to(
            &store,
            &card,
            json!({"action": "answer", "answers": answers}),
        );
        assert_eq!(res["status"], "confirmed", "{res}");
        assert_eq!(
            codex_answer(&received),
            [
                json!({"action":"accept","content":{"region":"eu-west","replicas":3,"features":["cache","waf"],"notify":false}})
            ]
        );

        // A page to open elsewhere is shown, never answered from here.
        let (_dir, store, live, received) = codex_waiting(
            json!({"id":7,"method":"mcpServer/elicitation/request","params":{"threadId":NATIVE,"turnId":"turn-1","serverName":"probe","mode":"url","_meta":null,"message":"Sign in to continue","url":"https://example.com/auth","elicitationId":"e1"}}),
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "readonly");
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["form"]["url"], "https://example.com/auth");
        for extra in [json!({"action": "approve"}), json!({"action": "cancel"})] {
            assert_eq!(respond_to(&store, &card, extra)["status"], "blocked");
        }
        assert!(codex_answer(&received).is_empty());
    }

    #[test]
    fn remote_codex_reply_is_built_from_the_request_the_helper_read() {
        let reply = |verb: Verb, hash: &'static str| Reply {
            verb,
            answers: None,
            text: None,
            reason: None,
            hash: Some(hash),
        };
        let amendment =
            json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["npm","test"]}});
        let read = json!({"promptHash":"h1","approvalRequest":{"id":3,"method":"item/commandExecution/requestApproval","params":{"threadId":NATIVE,"turnId":"t","itemId":"c","command":"npm test","availableDecisions":["accept",amendment.clone(),"cancel"]}}});
        let native = |verb| remote_codex_native(&read, &reply(verb, "h1"), "en");
        assert_eq!(
            native(Verb::Decide("approveAlways".into())).unwrap(),
            Some(json!({"decision": amendment}))
        );
        assert_eq!(
            native(Verb::Decide("decline".into())).unwrap(),
            Some(json!({"decision": "decline"}))
        );
        assert_eq!(
            native(Verb::Cancel).unwrap(),
            Some(json!({"decision": "cancel"}))
        );
        // Not listed, a stale card, or text aimed at an approval: nothing is built.
        assert!(native(Verb::Decide("approveSession".into())).is_err());
        assert!(native(Verb::Answer).is_err());
        assert!(
            remote_codex_native(&read, &reply(Verb::Decide("approve".into()), "h0"), "en").is_err()
        );

        let read = json!({"promptHash":"h2","questionRequest":{"id":4,"method":"item/tool/requestUserInput","params":{"threadId":NATIVE,"turnId":"t","itemId":"c","questions":[{"id":"langs","header":"Languages","question":"Which?","isOther":true,"isSecret":false,"options":[{"label":"Rust","description":""},{"label":"Go","description":""}]}]}}});
        let answers = json!({"langs": "Zig"});
        let mut answer = reply(Verb::Answer, "h2");
        answer.answers = Some(&answers);
        assert_eq!(
            remote_codex_native(&read, &answer, "en").unwrap(),
            Some(json!({"answers": {"langs": {"answers": ["Zig"]}}}))
        );
        assert!(
            remote_codex_native(&read, &reply(Verb::Decide("approve".into()), "h2"), "en").is_err()
        );

        // Nothing structured is pending: text goes the old way, a decision goes nowhere.
        let idle = json!({"promptHash": "h3"});
        assert_eq!(
            remote_codex_native(&idle, &reply(Verb::FollowUp, "h3"), "en").unwrap(),
            None
        );
        assert!(
            remote_codex_native(&idle, &reply(Verb::Decide("approve".into()), "h3"), "en").is_err()
        );
    }

    /// Agents without a reply interface show what they ask (rows as written by Kimi Code
    /// 2.4.1) and never take a decision or text for it, with or without the terminal adapter.
    #[test]
    fn transcript_read_prompts_are_shown_read_only_and_never_answered_through_the_terminal() {
        let (_dir, store, source) = fixture("kimi");
        save_config(
            &store.db,
            &Config {
                allow_terminal: true,
                ..Default::default()
            },
        )
        .unwrap();
        let folder = format!("session_{NATIVE}");
        let agent = std::path::Path::new(&source.path)
            .join("sessions/wd_fixture")
            .join(&folder)
            .join("agents/main");
        std::fs::create_dir_all(&agent).unwrap();
        let wire = agent.join("wire.jsonl");
        let save = |rows: &[Value]| {
            std::fs::write(
                &wire,
                rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
            )
            .unwrap()
        };
        let mut rows = vec![
            json!({"type":"turn.prompt","input":[{"type":"text","text":"Clean up"}],"turnId":0,"time":1}),
            json!({"type":"interaction.request","id":"approval_1","kind":"approval","toolCallId":"c1","request":{"sessionId":folder,"toolName":"Bash","action":"Running: rm -rf probe_dir","display":{"kind":"command","command":"rm -rf probe_dir","cwd":"/fixture"}}}),
        ];
        save(&rows);
        let mut live = controlled(&source, &format!("{folder}:main"), Phase::NeedsInput);
        live.path = wire.to_string_lossy().into();
        live.input_request = "approval_1".into();
        live.input_kind = "approval".into();
        watch(&store, &live);
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "approval");
        assert_eq!(card["inputKind"], "approval");
        assert_eq!(card["adapter"], "focusOnly");
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["approval"]["toolName"], "Bash");
        assert_eq!(card["approval"]["command"], "rm -rf probe_dir");
        assert_eq!(card["approval"]["kind"], "command");
        assert_eq!(card["approval"]["cwd"], "/fixture");
        assert_eq!(card["approval"]["decisions"], json!([]));
        assert_eq!(card["promptSource"], "question");
        assert!(!card["readonlyReason"].as_str().unwrap().is_empty());
        for extra in [
            json!({"action": "approve"}),
            json!({"action": "decline"}),
            json!({"action": "answer", "text": "y"}),
        ] {
            let mut params = extra;
            params["capabilities"] = json!({"localTerminal": true});
            let res = respond_to(&store, &card, params);
            assert_eq!(res["status"], "blocked", "{res}");
            assert!(res.get("terminalJob").is_none());
        }

        rows.push(json!({"type":"interaction.resolved","id":"approval_1","response":{"decision":"approved"}}));
        rows.push(json!({"type":"interaction.request","id":"question_1","kind":"question","toolCallId":"c2","request":{"questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"Warm"},{"label":"Blue"}]},{"question":"Which fruits?","header":"Fruit","multiSelect":true,"options":[{"label":"Apple"}]}]}}));
        save(&rows);
        live.input_request = "question_1".into();
        live.input_kind = "question".into();
        watch(&store, &live);
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "questions");
        assert_eq!(card["inputKind"], "question");
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["adapter"], "focusOnly");
        assert_eq!(card["questions"][0]["question"], "Which color?");
        assert_eq!(card["questions"][0]["options"][0]["label"], "Red");
        assert_eq!(card["questions"][1]["multiSelect"], true);
        let mut params = json!({"action": "answer", "text": "1", "answers": {"Color": "Red"}});
        params["capabilities"] = json!({"localTerminal": true});
        let res = respond_to(&store, &card, params);
        assert_eq!(res["status"], "blocked", "{res}");
        assert!(res.get("terminalJob").is_none());
    }

    /// A question tool whose options no reader can list still opens a menu in the terminal.
    #[test]
    fn question_wait_without_readable_options_is_read_only_and_says_so() {
        let (_dir, store, source) = fixture("cursor");
        save_config(
            &store.db,
            &Config {
                allow_terminal: true,
                ..Default::default()
            },
        )
        .unwrap();
        let caps = Capabilities::from_params(&json!({"capabilities": {"localTerminal": true}}));
        let mut live = controlled(&source, NATIVE, Phase::NeedsInput);
        live.input_request = "ask-1".into();
        live.input_kind = "question".into();
        let target = watch(&store, &live);
        assert_eq!(
            terminal_gate(
                &source,
                &target,
                Some(&content::Excerpt::default()),
                true,
                caps
            ),
            Err(TerminalBlock::Structured)
        );
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "readonly");
        assert_eq!(card["inputKind"], "question");
        assert_eq!(card["canRespond"], false);
        assert!(
            card["readonlyReason"]
                .as_str()
                .unwrap()
                .contains("选项内容暂不可读取")
        );
        let mut params = json!({"action": "answer", "text": "1"});
        params["capabilities"] = json!({"localTerminal": true});
        params["promptHash"] = json!(text_prompt_hash(&target, ""));
        let res = respond_to(&store, &card, params);
        assert_eq!(res["status"], "blocked", "{res}");
        assert!(res.get("terminalJob").is_none());
    }

    #[test]
    fn a_remote_read_that_failed_is_not_an_empty_card() {
        assert!(remote_read_ok(
            &json!({"excerpt": {"text": "", "question": ""}})
        ));
        assert!(!remote_read_ok(
            &json!({"status": "blocked", "reason": "x", "receipt": {}})
        ));
    }

    /// The remote helper reports what it read in the same shape as the local readers.
    #[test]
    fn remote_transcript_prompts_render_like_local_ones() {
        let excerpt: content::Excerpt = serde_json::from_value(json!({
            "text": "", "question": "shell\nrm -rf probe_dir", "userText": "", "userAt": 0,
            "structuredApproval": {"toolName": "shell", "reason": "Remove it",
                "command": "rm -rf probe_dir", "details": "", "kind": "command"}
        }))
        .unwrap();
        let mut payload = empty_payload("s");
        assert!(set_transcript_prompt(&mut payload, &excerpt, "en"));
        assert_eq!(payload["kind"], "approval");
        assert_eq!(payload["inputKind"], "approval");
        assert_eq!(payload["canRespond"], false);
        assert_eq!(payload["adapter"], "focusOnly");
        assert_eq!(payload["approval"]["command"], "rm -rf probe_dir");
        assert_eq!(payload["approval"]["decisions"], json!([]));

        let excerpt: content::Excerpt = serde_json::from_value(json!({
            "structuredQuestions": [{"id": "color", "question": "Which color?",
                "multiSelect": false, "options": [{"label": "red"}, {"label": "blue"}]}]
        }))
        .unwrap();
        let mut payload = empty_payload("s");
        assert!(set_transcript_prompt(&mut payload, &excerpt, "en"));
        assert_eq!(payload["kind"], "questions");
        assert_eq!(payload["canRespond"], false);
        assert_eq!(payload["questions"][0]["options"][1]["label"], "blue");
        assert!(!payload["readonlyReason"].as_str().unwrap().is_empty());

        let mut payload = empty_payload("s");
        assert!(!set_transcript_prompt(
            &mut payload,
            &content::Excerpt::default(),
            "en"
        ));
        assert_eq!(payload["kind"], "none");
    }

    /// A CodeBuddy approval read from its transcript still offers the popup approval hook.
    #[test]
    fn codebuddy_transcript_approval_shows_the_command_and_offers_the_decision_hook() {
        let (_dir, store, source) = fixture("codebuddy");
        let project = std::path::Path::new(&source.path).join("projects/fixture");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{NATIVE}.jsonl"));
        let rows = [
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":"Clean"}],"timestamp":5,"sessionId":NATIVE}),
            json!({"type":"function_call","callId":"c1","name":"Bash","arguments":"{\"command\":\"rm -rf probe_dir\"}","sessionId":NATIVE}),
        ];
        std::fs::write(
            &path,
            rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        let mut live = make_session(&source, NATIVE, Phase::NeedsInput);
        live.path = path.to_string_lossy().into();
        live.input_request = "approval:Bash".into();
        live.input_kind = "approval".into();
        watch(&store, &live);
        let card = inspect_card_of(&store, &live);
        assert_eq!(card["kind"], "approval");
        assert_eq!(card["canRespond"], false);
        assert_eq!(card["approval"]["toolName"], "Bash");
        assert_eq!(card["approval"]["command"], "rm -rf probe_dir");
        assert_eq!(card["needsClaudeApprovals"], true);
        assert_eq!(card["quickEnable"], "approvals");
    }

    /// A question the user dismissed stays in the transcript of the interrupted turn. The
    /// follow-up card is bound to the session state alone, and so is the reply check.
    #[test]
    fn terminal_follow_up_hash_ignores_a_question_left_unanswered_in_the_transcript() {
        let (_dir, store, source) = fixture("claude");
        let transcript = std::path::Path::new(&source.path).join("session.jsonl");
        let rows = [
            json!({"type":"assistant","sessionId":NATIVE,"timestamp":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":[
                {"type":"tool_use","id":"toolu_q","name":"AskUserQuestion","input":{"questions":[
                    {"question":"Which color?","header":"Color","options":[{"label":"Red"},{"label":"Blue"}]}]}}]}}),
            json!({"type":"user","sessionId":NATIVE,"timestamp":"2026-01-01T00:00:01Z","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"toolu_q","is_error":true,"content":"rejected"}]}}),
        ];
        std::fs::write(
            &transcript,
            rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        save_config(
            &store.db,
            &Config {
                allow_terminal: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut live = make_session(&source, NATIVE, Phase::Interrupted);
        live.path = transcript.to_string_lossy().into();
        live.turn_id.clear();
        let target = watch(&store, &live);
        let excerpt = control::local_excerpt(&source, &target);
        assert!(excerpt.question.contains("Which color?"), "stale question");
        let follow = text_prompt_hash(&target, "");
        assert_eq!(terminal_prompt_hash(&target, &excerpt), follow);
        // Where this machine offers a terminal adapter the card carries the same hash.
        let card = inspect_card_of(&store, &live);
        if card["kind"] == "followup" {
            assert_eq!(card["promptHash"], follow);
        }
        // A waiting session stays bound to the question that was shown.
        let mut waiting = live.clone();
        waiting.phase = Phase::NeedsInput;
        waiting.input_request = "idle".into();
        waiting.input_kind = "notification".into();
        let target = watch(&store, &waiting);
        assert_eq!(
            terminal_prompt_hash(&target, &excerpt),
            text_prompt_hash(&target, &excerpt.question)
        );
        assert_ne!(
            terminal_prompt_hash(&target, &excerpt),
            text_prompt_hash(&target, "")
        );
    }

    #[test]
    fn finished_text_is_read_again_only_when_the_record_changes() {
        let (_dir, store, source) = fixture("claude");
        let transcript = std::path::Path::new(&source.path).join("session.jsonl");
        let write = |text: &str| {
            std::fs::write(
                &transcript,
                json!({"type":"assistant","sessionId":NATIVE,"timestamp":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string() + "\n",
            )
            .unwrap();
        };
        write("First reply");
        let mut live = make_session(&source, NATIVE, Phase::Complete);
        live.id = "finished-text-cache".into();
        live.path = transcript.to_string_lossy().into();
        live.turn_id.clear();
        let target = watch(&store, &live);
        let cached = || {
            FINISHED_TEXT
                .lock()
                .unwrap()
                .iter()
                .find(|e| e.0 == "finished-text-cache")
                .map(|e| e.3.clone())
        };
        assert_eq!(finished_text(&source, &target), "First reply");
        assert_eq!(cached().as_deref(), Some("First reply"));
        // Same state and files: the remembered text is served without a read.
        FINISHED_TEXT
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|e| e.0 == "finished-text-cache")
            .for_each(|e| e.3 = "remembered".into());
        assert_eq!(finished_text(&source, &target), "remembered");
        write("A longer second reply");
        assert_eq!(finished_text(&source, &target), "A longer second reply");
        let mut moved = target.clone();
        moved.state_key.push_str(":next");
        FINISHED_TEXT
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|e| e.0 == "finished-text-cache")
            .for_each(|e| e.3 = "remembered".into());
        assert_eq!(finished_text(&source, &moved), "A longer second reply");
        // Without a record file nothing is remembered.
        let mut bare = target.clone();
        bare.session.id = "finished-text-bare".into();
        bare.path.clear();
        bare.log_paths.clear();
        finished_text(&source, &bare);
        assert!(
            !FINISHED_TEXT
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.0 == "finished-text-bare")
        );
    }
}
