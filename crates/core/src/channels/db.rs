use super::{Config, Owner, config, configuration, random_id};
use crate::{
    file_lock::FileLock,
    models::now,
    sessions::{LiveSession, Phase, Snapshot},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const RETENTION: i64 = 86_400;
pub(super) const MAX_INPUT: usize = 8192;

pub(super) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS channel_sessions (
        id TEXT PRIMARY KEY, short_id TEXT NOT NULL UNIQUE, epoch TEXT NOT NULL,
        state_key TEXT NOT NULL, observed_at INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS channel_events (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, session_id TEXT NOT NULL, payload TEXT NOT NULL, at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS channel_inbox (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, at INTEGER NOT NULL, payload TEXT NOT NULL, processed INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS channel_commands (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, session_id TEXT NOT NULL, expected TEXT NOT NULL,
        body TEXT NOT NULL, reply_to TEXT NOT NULL, at INTEGER NOT NULL,
        status TEXT NOT NULL, reason TEXT NOT NULL DEFAULT '', attempted_at INTEGER, receipt TEXT NOT NULL DEFAULT '{}');
        CREATE INDEX IF NOT EXISTS channel_commands_pending ON channel_commands(epoch,status,at);
        CREATE TABLE IF NOT EXISTS channel_outbox (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, chat_id TEXT NOT NULL, reply_to TEXT NOT NULL,
        session_id TEXT NOT NULL, expected TEXT NOT NULL, body TEXT NOT NULL, at INTEGER NOT NULL,
        sent_id TEXT UNIQUE, attempts INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS channel_outbox_pending ON channel_outbox(epoch,sent_id,next_attempt);
        CREATE INDEX IF NOT EXISTS channel_sessions_recent ON channel_sessions(epoch,observed_at);")?;
    Ok(())
}

pub(super) fn config_lock(store: &Store) -> Result<FileLock> {
    FileLock::try_exclusive(
        std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(store.root.join("channel-config.lock"))?,
    )
    .context("聊天渠道正在更新，请稍后重试")
}

pub(super) fn invalidate(db: &Connection, epoch: &str) -> Result<()> {
    db.execute("UPDATE channel_commands SET status='cancelled',reason='渠道配置已变更' WHERE epoch=?1 AND status IN ('queued','blocked','automation')", [epoch])?;
    db.execute("UPDATE channel_commands SET status='uncertain',reason='渠道配置已变更，请核对上次提交结果；系统不会重发' WHERE epoch=?1 AND status IN ('dispatching','submitted')", [epoch])?;
    db.execute(
        "DELETE FROM channel_outbox WHERE epoch=?1 AND sent_id IS NULL",
        [epoch],
    )?;
    db.execute("DELETE FROM channel_events WHERE epoch=?1", [epoch])?;
    db.execute("DELETE FROM kv WHERE key='channel.selection'", [])?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub key: String,
    pub state_key: String,
    pub observed_at: i64,
    pub session: LiveSession,
    // Deliberately separate from LiveSession's public, metadata-only JSON contract.
    pub path: String,
    pub log_paths: Vec<String>,
    #[serde(default)]
    pub process: Option<crate::sessions::ControlProcess>,
}
impl Session {
    pub fn title(&self, language: &str) -> String {
        let s = &self.session;
        let agent = crate::contracts::provider(&s.provider).map_or(s.provider.as_str(), |p| p.name);
        let machine = if s.machine_id == "local" {
            crate::localization::label(language, "本机")
        } else {
            &s.machine
        };
        let project = s
            .workspace
            .as_deref()
            .unwrap_or("")
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("");
        format!(
            "{} · {agent} · {machine}{}",
            self.key,
            if project.is_empty() {
                String::new()
            } else {
                format!(" · {project}")
            }
        )
    }
}

pub(super) fn observe(store: &Store, snapshot: &Snapshot) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled {
        return Ok(());
    }
    let at = now();
    let tx = store.db.unchecked_transaction()?;
    for s in snapshot
        .sessions
        .iter()
        .filter(|s| !s.background && !s.expired && !s.native_session.is_empty())
    {
        let existing: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT epoch,state_key,observed_at FROM channel_sessions WHERE id=?1",
                [&s.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let state_key = format!(
            "{}:{}:{}",
            s.phase.key(),
            s.turn_id,
            if s.phase == Phase::NeedsInput || s.turn_id.is_empty() {
                s.event_at.max(s.updated_at.saturating_mul(1000))
            } else {
                s.turn_started_at
            }
        );
        let observed_at = snapshot
            .sources
            .iter()
            .find(|h| h.source_id == s.source_id && h.machine_id == s.machine_id)
            .filter(|h| h.error.is_none() && h.state != "connecting")
            .map(|h| h.checked_at as i64)
            .unwrap_or(if s.machine_id == "local" { at } else { 0 });
        let fresh_epoch = existing.as_ref().is_none_or(|e| e.0 != c.epoch);
        let changed = fresh_epoch || existing.as_ref().is_none_or(|e| e.1 != state_key);
        if !changed && existing.as_ref().is_some_and(|e| observed_at - e.2 < 5) {
            continue;
        }
        // Usage members keep the original source/native-id scope before child aggregation.
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
            state_key: state_key.clone(),
            observed_at,
            session: s.clone(),
            path: s.path.clone(),
            log_paths,
            process,
        };
        let payload = serde_json::to_string(&row)?;
        tx.execute("INSERT INTO channel_sessions(id,short_id,epoch,state_key,observed_at,payload) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET epoch=excluded.epoch,state_key=excluded.state_key,observed_at=excluded.observed_at,payload=excluded.payload",
            params![s.id,row.key,c.epoch,state_key,observed_at,payload])?;
        let notify = changed
            && c.owner.is_some()
            && (s.phase == Phase::NeedsInput || !fresh_epoch && s.phase.terminal());
        if notify {
            let id = crate::models::hash(&format!("{}:{}:{state_key}", c.epoch, s.id));
            tx.execute(
                "INSERT OR IGNORE INTO channel_events VALUES(?1,?2,?3,?4,?5)",
                params![id, c.epoch, s.id, payload, at],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn session(db: &Connection, epoch: &str, key: &str) -> Result<Option<Session>> {
    let raw: Option<String> = db.query_row("SELECT payload FROM channel_sessions WHERE epoch=?1 AND (id=?2 OR short_id=?2) AND observed_at>?3", params![epoch,key,now()-RETENTION], |r| r.get(0)).optional()?;
    raw.map(|v| serde_json::from_str(&v).context("无法读取渠道会话"))
        .transpose()
}

pub(super) fn session_list(store: &Store) -> Result<Value> {
    let c = configuration(&store.db)?;
    let settings = store.settings()?;
    let mut query = store.db.prepare("SELECT payload FROM channel_sessions WHERE epoch=?1 AND observed_at>?2 ORDER BY observed_at DESC LIMIT 100")?;
    let mut rows = Vec::new();
    for raw in query.query_map(params![c.epoch, now() - RETENTION], |r| {
        r.get::<_, String>(0)
    })? {
        let row: Session = serde_json::from_str(&raw?)?;
        if settings
            .sources
            .iter()
            .any(|s| s.id == row.session.source_id && settings.source_enabled(s))
        {
            rows.push(json!({"id":row.session.id,"key":row.key,"title":row.title(&settings.appearance.language),"phase":row.session.phase.key(),"lastSeen":row.observed_at,"control":super::control::capability(&row,&c)}));
        }
    }
    Ok(json!({"sessions":rows}))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Command {
    pub id: String,
    pub session_id: String,
    pub expected: String,
    pub body: String,
    pub reply_to: String,
    pub at: i64,
    pub status: String,
    pub reason: String,
    pub attempted_at: Option<i64>,
    pub receipt: Value,
}

pub(super) fn command_rows(
    db: &Connection,
    epoch: &str,
    status: Option<&str>,
) -> Result<Vec<Command>> {
    query_commands(db, epoch, status, false)
}

pub(super) fn pending_commands(db: &Connection, epoch: &str, status: &str) -> Result<Vec<Command>> {
    query_commands(db, epoch, Some(status), true)
}

fn query_commands(
    db: &Connection,
    epoch: &str,
    status: Option<&str>,
    oldest_first: bool,
) -> Result<Vec<Command>> {
    // Apply ordering before the limit so new arrivals cannot hide older pending work.
    let order = if oldest_first { "ASC" } else { "DESC" };
    let mut query = db.prepare(&format!("SELECT id,session_id,expected,body,reply_to,at,status,reason,attempted_at,receipt FROM channel_commands WHERE (?1='' OR epoch=?1) AND (?2 IS NULL OR status=?2) ORDER BY at {order},rowid {order} LIMIT 100"))?;
    let rows = query.query_map(params![epoch, status], |r| {
        Ok(Command {
            id: r.get(0)?,
            session_id: r.get(1)?,
            expected: r.get(2)?,
            body: r.get(3)?,
            reply_to: r.get(4)?,
            at: r.get(5)?,
            status: r.get(6)?,
            reason: r.get(7)?,
            attempted_at: r.get(8)?,
            receipt: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or(Value::Null),
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

pub(super) fn command_list(store: &Store) -> Result<Value> {
    // Local review keeps prior configuration results visible; Feishu only sees its bound epoch.
    Ok(json!({"commands":command_rows(&store.db,"",None)?}))
}

pub(super) fn cancel(store: &Store, params: &Value) -> Result<Value> {
    let c = configuration(&store.db)?;
    let id = params["id"].as_str().context("缺少渠道指令标识")?;
    let changed = store.db.execute("UPDATE channel_commands SET status='cancelled',reason='用户已取消' WHERE epoch=?1 AND id=?2 AND status IN ('queued','blocked','automation')", params![c.epoch,id])?;
    ensure!(changed == 1, "该指令已提交或已结束，不能取消");
    Ok(json!({"cancelled":true}))
}

pub(super) fn retry(store: &Store, params: &Value) -> Result<Value> {
    let c = configuration(&store.db)?;
    ensure!(c.enabled && c.owner.is_some(), "飞书渠道尚未启用或配对");
    let id = params["id"].as_str().context("缺少渠道指令标识")?;
    let row = command_rows(&store.db, &c.epoch, Some("blocked"))?
        .into_iter()
        .find(|r| r.id == id)
        .context("只有尚未提交的指令可以重试")?;
    let target =
        session(&store.db, &c.epoch, &row.session_id)?.context("原会话已不可用，请重新选择会话")?;
    ensure!(
        target.state_key == row.expected,
        "会话已进入其他轮次，请发送新指令"
    );
    store.db.execute("UPDATE channel_commands SET status='queued',reason='' WHERE epoch=?1 AND id=?2 AND status='blocked'", params![c.epoch,id])?;
    crate::runtime::signal(&store.root).wake();
    Ok(json!({"queued":true}))
}

pub(super) fn outbox(
    db: &Connection,
    c: &Config,
    reply_to: &str,
    session: Option<(&str, &str)>,
    text: &str,
) -> Result<()> {
    outbox_raw(
        db,
        c,
        reply_to,
        session,
        &crate::localization::translate(&language(db), text),
    )
}

pub(super) fn language(db: &Connection) -> String {
    db.query_row(
        "SELECT json_extract(value,'$.appearance.language') FROM kv WHERE key='settings'",
        [],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "zh-CN".into())
}

/// Mixed notices include user-authored excerpts; translate only the surrounding system copy.
pub(super) fn outbox_raw(
    db: &Connection,
    c: &Config,
    reply_to: &str,
    session: Option<(&str, &str)>,
    text: &str,
) -> Result<()> {
    let Some(owner) = &c.owner else {
        return Ok(());
    };
    let (session_id, expected) = session.unwrap_or(("", ""));
    db.execute("INSERT INTO channel_outbox(id,epoch,chat_id,reply_to,session_id,expected,body,at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![random_id()?,c.epoch,owner.chat_id,reply_to,session_id,expected,super::content::clip(text,16000),now()])?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Incoming {
    pub id: String,
    pub owner: Owner,
    pub text: String,
    pub parent: String,
}

/// Only authenticated, bounded personal messages cross the transport boundary.
pub(super) fn ingest(store: &Store, epoch: &str, event: &Value) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled
        || c.epoch != epoch
        || event["header"]["app_id"].as_str() != Some(&c.app_id)
        || event["header"]["event_type"] != "im.message.receive_v1"
    {
        return Ok(());
    }
    let m = &event["event"]["message"];
    let sender = &event["event"]["sender"];
    if m["chat_type"] != "p2p" || sender["sender_type"] != "user" || m["message_type"] != "text" {
        return Ok(());
    }
    let time = m["create_time"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
        / 1000;
    if (now() - time).abs() > 300 {
        return Ok(());
    }
    let owner = Owner {
        open_id: sender["sender_id"]["open_id"].as_str().unwrap_or("").into(),
        chat_id: m["chat_id"].as_str().unwrap_or("").into(),
        tenant_key: sender["tenant_key"]
            .as_str()
            .or_else(|| event["header"]["tenant_key"].as_str())
            .unwrap_or("")
            .into(),
    };
    if owner.open_id.is_empty() || owner.chat_id.is_empty() {
        return Ok(());
    }
    if c.owner.as_ref().is_some_and(|bound| bound != &owner) {
        return Ok(());
    }
    let content = m["content"].as_str().unwrap_or("");
    if content.len() > MAX_INPUT * 2 {
        return Ok(());
    }
    let text = serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|v| v["text"].as_str().map(str::to_owned))
        .unwrap_or_default();
    if text.is_empty()
        || text.len() > MAX_INPUT
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return Ok(());
    }
    if c.owner.is_none() && !text.starts_with("/pair ") && !text.starts_with("配对 ") {
        return Ok(());
    }
    let id = m["message_id"].as_str().unwrap_or("");
    if id.is_empty() || id.len() > 256 {
        return Ok(());
    }
    if store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_inbox WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let count: i64 = store.db.query_row(
        "SELECT count(*) FROM channel_inbox WHERE processed=0",
        [],
        |r| r.get(0),
    )?;
    ensure!(count < 500, "渠道消息队列已满");
    let incoming = Incoming {
        id: id.into(),
        owner,
        text,
        parent: m["parent_id"]
            .as_str()
            .filter(|p| !p.is_empty())
            .or_else(|| m["root_id"].as_str())
            .unwrap_or("")
            .into(),
    };
    store.db.execute(
        "INSERT OR IGNORE INTO channel_inbox(id,epoch,at,payload) VALUES(?1,?2,?3,?4)",
        params![id, epoch, now(), serde_json::to_string(&incoming)?],
    )?;
    crate::runtime::signal(&store.root).wake();
    Ok(())
}

pub(super) fn process_inbox(store: &Store) -> Result<()> {
    let _lock = config_lock(store)?;
    let mut c = configuration(&store.db)?;
    if !c.enabled {
        return Ok(());
    }
    let raw: Option<(String,String)> = store.db.query_row("SELECT id,payload FROM channel_inbox WHERE epoch=?1 AND processed=0 ORDER BY rowid LIMIT 1", [&c.epoch], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((id, raw)) = raw else {
        return Ok(());
    };
    let msg: Incoming = serde_json::from_str(&raw)?;
    let tx = store.db.unchecked_transaction()?;
    tx.execute(
        "UPDATE channel_inbox SET processed=1,payload='{}' WHERE id=?1",
        [&id],
    )?;
    if c.owner.is_none() {
        let code = msg
            .text
            .strip_prefix("/pair ")
            .or_else(|| msg.text.strip_prefix("配对 "))
            .unwrap_or("")
            .trim();
        if config::accept_pair(&tx, &mut c, code, msg.owner)? {
            outbox(
                &tx,
                &c,
                &msg.id,
                None,
                "已配对 Aieyes。发送 /sessions 查看会话，引用会话通知回复即可继续。发送 /help 查看用法。",
            )?;
            let mut query = tx.prepare(
                "SELECT payload FROM channel_sessions WHERE epoch=?1 AND observed_at>?2",
            )?;
            for raw in query.query_map(params![c.epoch, now() - RETENTION], |r| {
                r.get::<_, String>(0)
            })? {
                let raw = raw?;
                let row: Session = serde_json::from_str(&raw)?;
                if row.session.phase == Phase::NeedsInput {
                    let event = crate::models::hash(&format!(
                        "{}:{}:{}",
                        c.epoch, row.session.id, row.state_key
                    ));
                    tx.execute(
                        "INSERT OR IGNORE INTO channel_events VALUES(?1,?2,?3,?4,?5)",
                        params![event, c.epoch, row.session.id, raw, now()],
                    )?;
                }
            }
        }
    } else if c.owner.as_ref() == Some(&msg.owner) {
        handle_message(&tx, &c, &msg)?;
    }
    tx.commit()?;
    Ok(())
}

fn handle_message(db: &Connection, c: &Config, msg: &Incoming) -> Result<()> {
    let text = msg.text.trim();
    let language = language(db);
    let tr = |text: &str| crate::localization::translate(&language, text).into_owned();
    if matches!(text, "/help" | "帮助") {
        return outbox(
            db,
            c,
            &msg.id,
            None,
            "/sessions 查看会话\n/use 会话编号 选择当前会话\n/send 会话编号 指令 发送到指定会话\n也可引用会话通知直接回复。\n/status 查看指令状态\n/retry 指令编号 重试尚未提交的指令\n/cancel 指令编号 取消尚未提交的指令\n审批请引用具体请求，回复 /approve 或 /decline。多问题按「问题编号=答案」逐行回复。已提交或结果不确定的指令不会自动重发。",
        );
    }
    if matches!(text, "/sessions" | "会话") {
        let mut query = db.prepare("SELECT payload FROM channel_sessions WHERE epoch=?1 AND observed_at>?2 ORDER BY observed_at DESC LIMIT 30")?;
        let mut lines = Vec::new();
        for raw in query.query_map(params![c.epoch, now() - RETENTION], |r| {
            r.get::<_, String>(0)
        })? {
            let row: Session = serde_json::from_str(&raw?)?;
            lines.push(format!(
                "{} · {} · {}",
                row.title(&language),
                tr(row.session.phase.label()),
                tr(super::control::capability(&row, c))
            ));
        }
        return outbox(
            db,
            c,
            &msg.id,
            None,
            &if lines.is_empty() {
                "尚未发现可用会话。请保持 Aieyes 运行并启用对应来源。".into()
            } else {
                lines.join("\n")
            },
        );
    }
    if text == "/status" {
        let rows = command_rows(db, &c.epoch, None)?;
        let lines = rows
            .iter()
            .take(10)
            .map(|r| {
                format!(
                    "{} · {}{}",
                    r.id,
                    tr(super::control::status_label(&r.status)),
                    if r.reason.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", tr(&r.reason))
                    }
                )
            })
            .collect::<Vec<_>>();
        return outbox(
            db,
            c,
            &msg.id,
            None,
            &if lines.is_empty() {
                "暂无指令".into()
            } else {
                lines.join("\n")
            },
        );
    }
    for (prefix, next) in [("/cancel ", "cancelled"), ("/retry ", "queued")] {
        if let Some(id) = text.strip_prefix(prefix) {
            let id = id.trim();
            let rows = command_rows(db, &c.epoch, None)?;
            let row = rows.iter().find(|r| r.id == id);
            let allowed = row.is_some_and(|r| {
                if next == "cancelled" {
                    matches!(r.status.as_str(), "queued" | "blocked" | "automation")
                } else {
                    r.status == "blocked"
                }
            });
            let current = row
                .map(|r| session(db, &c.epoch, &r.session_id))
                .transpose()?
                .flatten();
            let unchanged = next == "cancelled"
                || row
                    .zip(current.as_ref())
                    .is_some_and(|(r, s)| r.expected == s.state_key);
            if allowed && unchanged {
                db.execute(
                    "UPDATE channel_commands SET status=?1,reason='' WHERE epoch=?2 AND id=?3",
                    params![next, c.epoch, id],
                )?;
            }
            return outbox(
                db,
                c,
                &msg.id,
                None,
                if allowed && unchanged {
                    if next == "queued" {
                        "已重新排队"
                    } else {
                        "已取消尚未提交的指令"
                    }
                } else {
                    "指令已提交、已结束或会话已改变，请查看 /status 后发送新指令。"
                },
            );
        }
    }
    if let Some(key) = text.strip_prefix("/use ") {
        if let Some(row) = session(db, &c.epoch, key.trim())? {
            db.execute("INSERT INTO kv(key,value) VALUES('channel.selection',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json!({"epoch":c.epoch,"sessionId":row.session.id}).to_string()])?;
            return outbox(
                db,
                c,
                &msg.id,
                Some((&row.session.id, &row.state_key)),
                &format!(
                    "已选择 {}。后续消息发送到此会话；引用其他会话的消息会改用被引用的会话。",
                    row.title(&language)
                ),
            );
        }
        return outbox(
            db,
            c,
            &msg.id,
            None,
            "未找到会话，请发送 /sessions 查看编号。",
        );
    }
    if text.starts_with("/pair ") || text.starts_with("配对 ") {
        return outbox(db, c, &msg.id, None, "当前账号已配对。");
    }
    let mut body = msg.text.clone();
    let target: Option<(String, String)> = if let Some(rest) = text.strip_prefix("/send ") {
        match rest.split_once(char::is_whitespace) {
            Some((key, value)) if !value.trim().is_empty() => {
                body = value.trim_start().to_owned();
                session(db, &c.epoch, key)?.map(|s| (s.session.id, s.state_key))
            }
            _ => None,
        }
    } else if !msg.parent.is_empty() {
        db.query_row("SELECT session_id,expected FROM channel_outbox WHERE epoch=?1 AND sent_id=?2 AND session_id<>''", params![c.epoch,msg.parent], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
    } else {
        let raw: Option<String> = db
            .query_row(
                "SELECT value FROM kv WHERE key='channel.selection'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let selection: Value = raw
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or(Value::Null);
        if selection["epoch"].as_str() == Some(&c.epoch) {
            session(db, &c.epoch, selection["sessionId"].as_str().unwrap_or(""))?
                .map(|s| (s.session.id, s.state_key))
        } else {
            None
        }
    };
    let Some((session_id, expected)) = target else {
        return outbox(
            db,
            c,
            &msg.id,
            None,
            "请引用会话通知回复，或先用 /use 会话编号 选择目标。发送 /sessions 查看会话。",
        );
    };
    if matches!(body.trim(), "/approve" | "/decline" | "/cancel-request") {
        let quoted: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM channel_outbox o JOIN kv k ON k.key=?1 WHERE o.epoch=?2 AND o.sent_id=?3 AND o.id=json_extract(k.value,'$.outboxId') AND json_extract(k.value,'$.approval')=1 AND json_extract(k.value,'$.expected')=?4)",params![format!("channel.prompt:{session_id}"),c.epoch,msg.parent,expected],|r|r.get(0))?;
        if !quoted || text.starts_with("/send ") {
            return outbox(
                db,
                c,
                &msg.id,
                None,
                "请引用最新的审批通知，再回复 /approve 或 /decline。",
            );
        }
    }
    if text.starts_with('/')
        && !text.starts_with("/send ")
        && !matches!(text, "/approve" | "/decline" | "/cancel-request")
    {
        return outbox(
            db,
            c,
            &msg.id,
            None,
            "未知的渠道命令。发送 /help 查看用法；要把斜杠指令发给 Agent，请用 /send 会话编号 指令。",
        );
    }
    let count: i64 = db.query_row("SELECT count(*) FROM channel_commands WHERE epoch=?1 AND status IN ('queued','blocked','automation','dispatching','submitted')", [&c.epoch], |r| r.get(0))?;
    if count >= 100 {
        return outbox(
            db,
            c,
            &msg.id,
            None,
            "待处理指令过多，请先查看 /status 并取消不再需要的指令。",
        );
    }
    let id = format!("c{}", &crate::models::hash(&msg.id)[..10]);
    db.execute("INSERT OR IGNORE INTO channel_commands(id,epoch,session_id,expected,body,reply_to,at,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued')", params![id,c.epoch,session_id,expected,body,msg.id,now()])?;
    outbox(
        db,
        c,
        &msg.id,
        Some((&session_id, &expected)),
        &format!("已收到指令 {id}，正在核对原会话。"),
    )
}

pub(super) fn cleanup(db: &Connection) -> Result<()> {
    let before = now() - RETENTION;
    db.execute("DELETE FROM channel_inbox WHERE at<?1", [before])?;
    db.execute("DELETE FROM channel_outbox WHERE at<?1", [before])?;
    db.execute("DELETE FROM channel_events WHERE at<?1", [before])?;
    db.execute("DELETE FROM channel_commands WHERE at<?1", [before])?;
    db.execute(
        "DELETE FROM channel_sessions WHERE observed_at<?1",
        [before],
    )?;
    db.execute("DELETE FROM kv WHERE key LIKE 'channel.prompt:%' AND substr(key,16) NOT IN (SELECT id FROM channel_sessions)", [])?;
    Ok(())
}
