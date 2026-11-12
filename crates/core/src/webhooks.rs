//! Optional outbound webhooks. Events queue in SQLite and one caller delivers them,
//! at most once per target; failed targets retry with backoff until the event ages out.
use crate::{
    Engine,
    models::{Settings, now},
    sessions::{LiveSession, Phase},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

/// Wire format per target. `json` is the full envelope; the rest fit common receivers.
pub const FORMATS: &[&str] = &["json", "text", "bark", "slack", "wechat", "feishu"];
const MAX_TARGETS: usize = 8;
const MAX_ATTEMPTS: i64 = 5;
const MAX_AGE_SECONDS: i64 = 86_400;
const MAX_PENDING: i64 = 200;
const BATCH: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    QuotaReset,
    SessionNeedsInput,
    SessionComplete,
    SessionFailed,
    ServerAlert,
    Test,
}
impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::QuotaReset => "quotaReset",
            Self::SessionNeedsInput => "sessionNeedsInput",
            Self::SessionComplete => "sessionComplete",
            Self::SessionFailed => "sessionFailed",
            Self::ServerAlert => "serverAlert",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub url: String,
    pub format: String,
    pub enabled: bool,
}
impl Default for Target {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            format: "json".into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub enabled: bool,
    pub targets: Vec<Target>,
    pub quota_reset: bool,
    pub session_needs_input: bool,
    pub session_complete: bool,
    pub session_failed: bool,
    pub server_alert: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            targets: vec![],
            quota_reset: true,
            session_needs_input: true,
            session_complete: true,
            session_failed: true,
            server_alert: true,
        }
    }
}
impl Preferences {
    pub fn active_targets(&self) -> Vec<&Target> {
        if !self.enabled {
            return vec![];
        }
        self.targets
            .iter()
            .filter(|t| t.enabled && !t.url.trim().is_empty())
            .collect()
    }
    /// Whether a kind is both subscribed and deliverable right now.
    pub fn allows(&self, kind: Kind) -> bool {
        let subscribed = match kind {
            Kind::QuotaReset => self.quota_reset,
            Kind::SessionNeedsInput => self.session_needs_input,
            Kind::SessionComplete => self.session_complete,
            Kind::SessionFailed => self.session_failed,
            Kind::ServerAlert => self.server_alert,
            Kind::Test => true,
        };
        subscribed && !self.active_targets().is_empty()
    }
}

pub fn validate(p: &Preferences) -> Result<()> {
    ensure!(p.targets.len() <= MAX_TARGETS, "Webhook 地址最多 8 个");
    let mut ids = std::collections::HashSet::new();
    for target in &p.targets {
        ensure!(!target.id.is_empty(), "Webhook 地址缺少标识");
        ensure!(ids.insert(&target.id), "Webhook 地址标识重复");
        ensure!(
            FORMATS.contains(&target.format.as_str()),
            "无效的 Webhook 格式"
        );
        let url = target.url.trim();
        if url.is_empty() {
            continue;
        }
        let parsed = reqwest::Url::parse(url).context("Webhook 地址格式不正确")?;
        ensure!(
            ["http", "https"].contains(&parsed.scheme()) && parsed.host_str().is_some(),
            "Webhook 地址需要为 HTTP/HTTPS URL"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub event: String,
    pub at: i64,
    pub title: String,
    pub text: String,
    pub data: Value,
}

fn tr(language: &str, source: &str) -> String {
    crate::localization::translate(language, source).into_owned()
}

/// Resetting quota, described with the account and each window's before/after percent.
pub(crate) fn reset_message(
    language: &str,
    event: &crate::quota_notifications::ResetEvent,
) -> (String, String, Value) {
    let title = match event.kind {
        crate::quota_notifications::Kind::Unexpected => {
            tr(language, &format!("{} 额度提前重置", event.name))
        }
        crate::quota_notifications::Kind::Natural => {
            tr(language, &format!("{} 额度已重置", event.name))
        }
    };
    let text = event
        .windows
        .iter()
        .map(|w| {
            format!(
                "{} {:.0}% → {:.0}%",
                tr(language, &w.name),
                w.previous_used_percent,
                w.used_percent
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    (
        title,
        text,
        serde_json::to_value(event).unwrap_or(Value::Null),
    )
}

/// Session phase changes carry identity only: agent, machine, phase and folder name.
pub(crate) fn session_message(
    language: &str,
    kind: Kind,
    session: &LiveSession,
) -> (String, String, Value) {
    let agent = crate::contracts::provider(&session.provider)
        .map_or_else(|| session.provider.clone(), |p| p.name.to_string());
    let title = match kind {
        Kind::SessionNeedsInput => tr(language, &format!("{agent} 等待输入")),
        Kind::SessionComplete => tr(language, &format!("{agent} 已完成")),
        _ => tr(language, &format!("{agent} 执行失败")),
    };
    let machine = if session.machine_id == "local" {
        tr(language, "本机")
    } else if session.machine.is_empty() {
        session.machine_id.clone()
    } else {
        session.machine.clone()
    };
    let project = session
        .workspace
        .as_deref()
        .map(|path| path.trim_end_matches(['/', '\\']))
        .and_then(|path| path.rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let text = match &project {
        Some(project) => format!("{machine} · {project}"),
        None => machine,
    };
    let data = json!({
        "sessionId": session.id,
        "provider": session.provider,
        "agent": agent,
        "machineId": session.machine_id,
        "machine": session.machine,
        "phase": session.phase.key(),
        "project": project,
        "updatedAt": session.updated_at,
    });
    (title, text, data)
}

pub(crate) fn alert_message(
    language: &str,
    event: &crate::host_history::AlertEvent,
) -> (String, String, Value) {
    let title = if event.metric == "memory" {
        tr(language, &format!("{} 内存使用率持续偏高", event.name))
    } else {
        tr(language, &format!("{} CPU 使用率持续偏高", event.name))
    };
    let text = tr(
        language,
        &format!(
            "当前 {:.0}%，已持续 {} 分钟",
            event.value,
            event.duration_seconds / 60
        ),
    );
    (
        title,
        text,
        serde_json::to_value(event).unwrap_or(Value::Null),
    )
}

/// Phase and turn per session; a change into a waiting or terminal phase fires once.
pub(crate) type SessionMark = (Phase, String);

pub(crate) fn session_transitions(
    previous: &mut HashMap<String, SessionMark>,
    sessions: &[LiveSession],
) -> Vec<(Kind, LiveSession)> {
    let mut events = Vec::new();
    for session in sessions {
        let mark = (session.phase, session.turn_id.clone());
        match previous.insert(session.id.clone(), mark.clone()) {
            Some(old) if old != mark => {
                let kind = match session.phase {
                    Phase::NeedsInput => Some(Kind::SessionNeedsInput),
                    Phase::Complete => Some(Kind::SessionComplete),
                    Phase::Failed => Some(Kind::SessionFailed),
                    _ => None,
                };
                // A late turn id on an unchanged phase is metadata, not a new event.
                let new_turn =
                    !old.1.is_empty() && !session.turn_id.is_empty() && old.1 != session.turn_id;
                if let Some(kind) = kind
                    && (old.0 != session.phase || new_turn)
                {
                    events.push((kind, session.clone()));
                }
            }
            _ => {}
        }
    }
    previous.retain(|id, _| sessions.iter().any(|s| &s.id == id));
    events
}

/// Session webhooks for a frontend that reads live sessions itself instead of calling
/// `sessions.list` (the Windows/Linux shell's status thread). Same rules as that handler:
/// one event per change into waiting, complete or failed; background sessions never send.
#[derive(Debug, Default)]
pub struct SessionObserver {
    marks: HashMap<String, SessionMark>,
}
impl SessionObserver {
    /// Queues the transitions in `sessions` that the subscription allows; returns how many.
    pub fn observe(
        &mut self,
        store: &Store,
        settings: &Settings,
        sessions: &[LiveSession],
    ) -> usize {
        let mut queued = 0;
        for (kind, session) in session_transitions(&mut self.marks, sessions) {
            if session.background || !settings.webhooks.allows(kind) {
                continue;
            }
            let (title, text, data) =
                session_message(&settings.appearance.language, kind, &session);
            if store
                .queue_webhook(settings, kind, title, text, data)
                .is_ok()
            {
                queued += 1;
            }
        }
        queued
    }
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS webhook_events(id INTEGER PRIMARY KEY AUTOINCREMENT,payload TEXT NOT NULL,created_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,next_attempt_at INTEGER NOT NULL DEFAULT 0,delivered TEXT NOT NULL DEFAULT '[]');",
    )?;
    Ok(())
}

impl Store {
    /// Queue one event when the subscription allows it; callers pass display-ready text.
    pub(crate) fn queue_webhook(
        &self,
        settings: &Settings,
        kind: Kind,
        title: String,
        text: String,
        data: Value,
    ) -> Result<()> {
        if !settings.webhooks.allows(kind) {
            return Ok(());
        }
        let message = Message {
            event: kind.key().into(),
            at: now(),
            title,
            text,
            data,
        };
        self.db.execute(
            "INSERT INTO webhook_events(payload,created_at) VALUES(?1,?2)",
            params![serde_json::to_string(&message)?, message.at],
        )?;
        self.db.execute(
            "DELETE FROM webhook_events WHERE id NOT IN (SELECT id FROM webhook_events ORDER BY id DESC LIMIT ?1)",
            [MAX_PENDING],
        )?;
        Ok(())
    }
}

fn send(client: &reqwest::blocking::Client, target: &Target, message: &Message) -> Result<()> {
    let line = if message.text.trim().is_empty() {
        message.title.clone()
    } else {
        format!("{}\n{}", message.title, message.text)
    };
    let request = client.post(target.url.trim());
    let request = match target.format.as_str() {
        "text" => request
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line),
        "bark" => {
            request.json(&json!({"title":message.title,"body":message.text,"group":"Aieyes"}))
        }
        "slack" => request.json(&json!({"text":format!("*{}*\n{}", message.title, message.text)})),
        "wechat" => request.json(&json!({"msgtype":"text","text":{"content":line}})),
        "feishu" => request.json(&json!({"msg_type":"text","content":{"text":line}})),
        _ => request.json(&json!({
            "app":"Aieyes","version":env!("CARGO_PKG_VERSION"),"event":message.event,
            "at":message.at,"title":message.title,"text":message.text,"data":message.data})),
    };
    let response = request.send().map_err(|error| {
        anyhow::anyhow!(if error.is_timeout() {
            "连接超时"
        } else {
            "连接失败"
        })
    })?;
    ensure!(
        response.status().is_success(),
        "HTTP {}",
        response.status().as_u16()
    );
    if !matches!(target.format.as_str(), "json" | "text") {
        // Do not echo receiver bodies: they can contain the secret webhook URL.
        let body = response
            .text()
            .context("Webhook response could not be read")?;
        validate_response(&target.format, &body)?;
    }
    Ok(())
}

fn validate_response(format: &str, body: &str) -> Result<()> {
    if format == "slack" {
        ensure!(body.trim() == "ok", "Webhook receiver rejected the message");
        return Ok(());
    }
    let value: Value = serde_json::from_str(body).context("Invalid webhook acknowledgement")?;
    let (code, expected) = match format {
        "wechat" => (value["errcode"].as_i64(), 0),
        "feishu" => (
            value
                .get("code")
                .or_else(|| value.get("StatusCode"))
                .and_then(Value::as_i64),
            0,
        ),
        "bark" => (value["code"].as_i64(), 200),
        _ => return Ok(()),
    };
    ensure!(
        code == Some(expected),
        "Webhook receiver rejected the message (code: {:?})",
        code
    );
    Ok(())
}

fn client(settings: &Settings) -> Result<reqwest::blocking::Client> {
    crate::network::client_with_policy(
        &settings.proxy,
        crate::network::ClientPolicy {
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(12),
            redirects: 3,
            ..Default::default()
        },
    )
}

impl Engine {
    /// Deliver due events; each target receives an event at most once.
    pub(crate) fn dispatch_webhooks(&mut self) -> Result<Value> {
        let settings = self.store.settings()?;
        let stamp = now();
        self.store.db.execute(
            "DELETE FROM webhook_events WHERE created_at < ?1 OR attempts >= ?2",
            params![stamp - MAX_AGE_SECONDS, MAX_ATTEMPTS],
        )?;
        let targets = settings.webhooks.active_targets();
        if targets.is_empty() {
            self.store.db.execute("DELETE FROM webhook_events", [])?;
            return Ok(json!({"delivered":0,"failed":0,"pending":0}));
        }
        let targets: Vec<Target> = targets.into_iter().cloned().collect();
        let rows: Vec<(i64, i64, String, String)> = {
            let mut statement = self.store.db.prepare(
                "SELECT id,attempts,payload,delivered FROM webhook_events WHERE next_attempt_at<=?1 ORDER BY id LIMIT ?2",
            )?;
            statement
                .query_map(params![stamp, BATCH as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<rusqlite::Result<_>>()?
        };
        let client = client(&settings)?;
        let (mut delivered, mut failed) = (0_i64, 0_i64);
        for (id, attempts, payload, done) in rows {
            crate::operations::check()?;
            let message: Message = serde_json::from_str(&payload)?;
            let mut done: Vec<String> = serde_json::from_str(&done).unwrap_or_default();
            let mut failure = false;
            for target in &targets {
                if done.contains(&target.id) {
                    continue;
                }
                crate::operations::check()?;
                match send(&client, target, &message) {
                    Ok(()) => done.push(target.id.clone()),
                    Err(_) => failure = true,
                }
            }
            if failure {
                failed += 1;
                let delay = 60 * 4_i64.pow(u32::try_from(attempts).unwrap_or(0));
                self.store.db.execute(
                    "UPDATE webhook_events SET attempts=attempts+1,next_attempt_at=?2,delivered=?3 WHERE id=?1",
                    params![id, stamp + delay, serde_json::to_string(&done)?],
                )?;
            } else {
                delivered += 1;
                self.store
                    .db
                    .execute("DELETE FROM webhook_events WHERE id=?1", [id])?;
            }
        }
        let pending: i64 =
            self.store
                .db
                .query_row("SELECT COUNT(*) FROM webhook_events", [], |r| r.get(0))?;
        Ok(json!({"delivered":delivered,"failed":failed,"pending":pending}))
    }

    /// Send one sample message to the target being edited; the error text reaches the form.
    pub(crate) fn test_webhook(&mut self, params: &Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let target: Target = serde_json::from_value(
            params
                .get("target")
                .cloned()
                .context("请选择要测试的 Webhook 地址")?,
        )?;
        ensure!(!target.url.trim().is_empty(), "请输入 Webhook 地址");
        let single = Preferences {
            enabled: true,
            targets: vec![target.clone()],
            ..Default::default()
        };
        validate(&single)?;
        let message = Message {
            event: Kind::Test.key().into(),
            at: now(),
            title: tr(&settings.appearance.language, "Aieyes 测试通知"),
            text: tr(&settings.appearance.language, "Webhook 已连接"),
            data: json!({"app":"Aieyes"}),
        };
        send(&client(&settings)?, &target, &message)?;
        Ok(json!({"delivered":true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    fn temp_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (store, dir)
    }

    fn direct_settings() -> Settings {
        Settings {
            proxy: crate::models::ProxyConfig {
                mode: "direct".into(),
                url: String::new(),
            },
            ..Default::default()
        }
    }

    fn settings_with(events: &[Kind], urls: &[&str]) -> Settings {
        let mut settings = direct_settings();
        settings.webhooks.enabled = true;
        settings.webhooks.targets = urls
            .iter()
            .enumerate()
            .map(|(index, url)| Target {
                id: format!("t{index}"),
                name: String::new(),
                url: (*url).into(),
                format: "json".into(),
                enabled: true,
            })
            .collect();
        settings.webhooks.quota_reset = events.contains(&Kind::QuotaReset);
        settings.webhooks.session_needs_input = events.contains(&Kind::SessionNeedsInput);
        settings.webhooks.session_complete = events.contains(&Kind::SessionComplete);
        settings.webhooks.session_failed = events.contains(&Kind::SessionFailed);
        settings.webhooks.server_alert = events.contains(&Kind::ServerAlert);
        settings
    }

    /// Minimal HTTP receiver: answers `status` and reports each request body.
    fn receiver(status: u16, count: usize) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..count {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&buffer);
                    let Some(header_end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let length: usize = text
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .or_else(|| line.strip_prefix("Content-Length: "))
                        })
                        .and_then(|value| value.trim().parse().ok())
                        .unwrap_or(0);
                    if buffer.len() >= header_end + 4 + length {
                        let body = String::from_utf8_lossy(&buffer[header_end + 4..]).into_owned();
                        let _ = tx.send(body);
                        break;
                    }
                }
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                );
            }
        });
        (url, rx)
    }

    fn engine(dir: &tempfile::TempDir) -> Engine {
        Engine::open(dir.path()).unwrap()
    }

    #[test]
    fn queue_respects_the_subscription_and_delivers_json() {
        let (store, dir) = temp_store();
        drop(store);
        let (url, bodies) = receiver(200, 1);
        let settings = settings_with(&[Kind::QuotaReset], &[&url]);
        store_queue(&dir, &settings, Kind::QuotaReset, "T", "B");
        store_queue(&dir, &settings, Kind::SessionComplete, "skip", "");
        let mut engine = engine(&dir);
        let result = engine.call("webhooks.dispatch", json!({})).unwrap();
        assert_eq!(result["delivered"], 1);
        assert_eq!(result["pending"], 0);
        let body: Value = serde_json::from_str(&bodies.recv().unwrap()).unwrap();
        assert_eq!(body["event"], "quotaReset");
        assert_eq!(body["title"], "T");
        assert_eq!(body["app"], "Aieyes");
    }

    /// Persist the subscription, then queue under it, as the engine does.
    fn store_queue(
        dir: &tempfile::TempDir,
        settings: &Settings,
        kind: Kind,
        title: &str,
        text: &str,
    ) {
        let store = Store::open(dir.path()).unwrap();
        store.save_settings(settings).unwrap();
        store
            .queue_webhook(settings, kind, title.into(), text.into(), json!({}))
            .unwrap();
    }

    #[test]
    fn failed_targets_retry_alone_and_disabling_clears_the_queue() {
        let (store, dir) = temp_store();
        drop(store);
        let (good, bodies) = receiver(200, 2);
        let (bad, _) = receiver(500, 2);
        let settings = settings_with(&[Kind::QuotaReset], &[&good, &bad]);
        store_queue(&dir, &settings, Kind::QuotaReset, "T", "");
        let mut engine = engine(&dir);
        // Queuing advances the revision clients watch; a failed attempt does not, so retries stay timed.
        let queued = engine.call("runtime.revisions", json!({})).unwrap()["webhooks"]
            .as_i64()
            .unwrap();
        assert!(queued > 0);
        let first = engine.call("webhooks.dispatch", json!({})).unwrap();
        assert_eq!(first["delivered"], 0);
        assert_eq!(first["failed"], 1);
        assert_eq!(first["pending"], 1);
        assert_eq!(
            engine.call("runtime.revisions", json!({})).unwrap()["webhooks"],
            queued
        );
        // The good target was recorded as delivered; only the failing one retries.
        assert!(bodies.recv().is_ok());
        let mut disabled = settings.clone();
        disabled.webhooks.enabled = false;
        engine.store.save_settings(&disabled).unwrap();
        let second = engine.call("webhooks.dispatch", json!({})).unwrap();
        assert_eq!(second["pending"], 0);
    }

    #[test]
    fn session_transitions_fire_once_per_change() {
        let session = LiveSession {
            id: "s1".into(),
            provider: "codex".into(),
            machine_id: "local".into(),
            phase: Phase::Working,
            turn_id: "t1".into(),
            ..Default::default()
        };
        let mut marks = HashMap::new();
        assert!(session_transitions(&mut marks, std::slice::from_ref(&session)).is_empty());
        let waiting = LiveSession {
            phase: Phase::NeedsInput,
            ..session.clone()
        };
        let events = session_transitions(&mut marks, std::slice::from_ref(&waiting));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, Kind::SessionNeedsInput);
        // The same state does not repeat, and a vanished session is forgotten.
        assert!(session_transitions(&mut marks, &[waiting]).is_empty());
        assert!(session_transitions(&mut marks, &[]).is_empty());
        assert!(marks.is_empty());
    }

    #[test]
    fn session_observer_queues_allowed_transitions_once() {
        let (store, _dir) = temp_store();
        let settings = settings_with(
            &[Kind::SessionNeedsInput],
            &["https://example.invalid/hook"],
        );
        store.save_settings(&settings).unwrap();
        let session = LiveSession {
            id: "s1".into(),
            provider: "codex".into(),
            machine_id: "local".into(),
            phase: Phase::Working,
            turn_id: "t1".into(),
            workspace: Some("/work/alpha".into()),
            ..Default::default()
        };
        let waiting = LiveSession {
            phase: Phase::NeedsInput,
            ..session.clone()
        };
        let complete = LiveSession {
            phase: Phase::Complete,
            ..session.clone()
        };
        let hidden = LiveSession {
            id: "s2".into(),
            background: true,
            ..session.clone()
        };
        let mut observer = SessionObserver::default();
        let first = observer.observe(&store, &settings, &[session.clone(), hidden.clone()]);
        assert_eq!(first, 0);
        let hidden_waiting = LiveSession {
            phase: Phase::NeedsInput,
            ..hidden
        };
        let changed = observer.observe(&store, &settings, &[waiting.clone(), hidden_waiting]);
        assert_eq!(changed, 1, "background sessions never send");
        let repeated = observer.observe(&store, &settings, &[waiting]);
        assert_eq!(repeated, 0, "an unchanged phase does not repeat");
        let unsubscribed = observer.observe(&store, &settings, &[complete]);
        assert_eq!(unsubscribed, 0, "unsubscribed kinds are not queued");
        let payload: String = store
            .db
            .query_row("SELECT payload FROM webhook_events", [], |row| row.get(0))
            .unwrap();
        let message: Message = serde_json::from_str(&payload).unwrap();
        assert_eq!(message.event, "sessionNeedsInput");
        assert_eq!(message.data["project"], "alpha");
    }

    #[test]
    fn receiver_acknowledgements_reject_business_errors_and_malformed_bodies() {
        for (format, good, bad) in [
            ("wechat", r#"{"errcode":0}"#, r#"{"errcode":93000}"#),
            ("feishu", r#"{"code":0}"#, r#"{"code":9499}"#),
            ("bark", r#"{"code":200}"#, r#"{"code":400}"#),
            ("slack", "ok", "invalid_payload"),
        ] {
            assert!(validate_response(format, good).is_ok());
            assert!(validate_response(format, bad).is_err());
            assert!(validate_response(format, "").is_err());
            assert!(validate_response(format, "{}").is_err());
        }
        assert!(validate_response("feishu", r#"{"StatusCode":0}"#).is_ok());
        assert!(validate_response("feishu", r#"{"code":9499,"StatusCode":0}"#).is_err());
    }

    #[test]
    fn validation_keeps_addresses_bounded() {
        let mut preferences = Preferences {
            enabled: true,
            ..Default::default()
        };
        preferences.targets.push(Target {
            id: "a".into(),
            url: "ftp://example.com".into(),
            ..Default::default()
        });
        assert!(validate(&preferences).is_err());
        preferences.targets[0].url = "https://example.com/hook".into();
        assert!(validate(&preferences).is_ok());
        preferences.targets[0].url = String::new();
        assert!(validate(&preferences).is_ok());
    }
}
