//! AskUserQuestion and opt-in tool approval replies use Claude's PermissionRequest hook contract,
//! MCP forms its Elicitation hook contract.
//! Qoder CLI ships the same tools, transcript and hook contracts, so it shares this path.
//! CodeBuddy and ZCode take the allow / deny part of the PermissionRequest contract only.
//! The observer hooks remain metadata-only; these separate hooks are opt-in (Feishu or Desktop Interaction).
//! A held request lives in `channel_claude_requests` until its helper delivers the reply; the
//! tool input and the answer are erased at that moment.
use super::{Session, codex, configuration, content, control::Outcome};
use crate::{
    models::{Source, now},
    session_interaction::contract::Decision,
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

mod card;
pub(in crate::channels) mod elicitation;
mod watch;

pub(crate) use card::approval_card;

const MARKER: &str = "--aieyes-claude-question";
const PREVIEW_BUDGET: usize = 8000;
pub(crate) const PASSTHROUGH: &str = "__aieyes_passthrough__";
const DESKTOP_QUESTION_TTL: i64 = 45;
const DESKTOP_APPROVAL_TTL: i64 = 15;
/// Hook input larger than this is left to the agent's own dialog.
const MAX_STDIN: u64 = 32 * 1024 * 1024;
/// What one request may keep in the request table.
const STORE_BUDGET: usize = 256 * 1024;
/// Seconds a request is held before it falls back to the agent's own dialog.
const QUESTION_HOLD: u64 = 86390;
const APPROVAL_HOLD: u64 = 1800;
const SEQUENTIAL_HOLD: u64 = 120;

#[derive(Debug, Clone)]
pub(crate) struct PendingClaude {
    pub id: String,
    pub kind: String,
    pub tool_name: String,
    pub call_id: String,
    pub question_hash: String,
    /// What a card can show. Long values of an approval are cut (`meta.truncated`).
    pub input: Value,
    /// `provider`, `cwd`, `suggestions`, `truncated`, `exists`; recorded by the hook helper.
    pub meta: Value,
    pub pid: u32,
    pub process_start: u64,
    /// Seconds; when the hook registered the request.
    pub at: i64,
}

impl PendingClaude {
    /// The agent that registered the request.
    pub fn provider(&self) -> &str {
        self.meta["provider"].as_str().unwrap_or("claude")
    }
}

/// What an agent's decision hook can carry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Profile {
    /// `AskUserQuestion` is answered through `updatedInput`.
    questions: bool,
    /// MCP forms are answered through an `Elicitation` hook.
    elicitation: bool,
    /// An allow may carry `updatedPermissions`.
    suggestions: bool,
    /// A deny may carry `interrupt`.
    interrupt: bool,
    /// The agent shows its own dialog while a `PermissionRequest` hook runs. Otherwise the
    /// terminal shows nothing until the hook returns, so a request is only held briefly.
    /// An `Elicitation` hook always runs before the agent's own form (see `hold_secs`).
    concurrent: bool,
}

impl Profile {
    const CLAUDE: Self = Self {
        questions: true,
        elicitation: true,
        suggestions: true,
        interrupt: true,
        concurrent: true,
    };
}

fn profile(provider: &str) -> Option<Profile> {
    let basic = Profile {
        questions: false,
        elicitation: false,
        suggestions: false,
        interrupt: false,
        concurrent: false,
    };
    match provider {
        "claude" | "qoder" | "qoder-cn" => Some(Profile::CLAUDE),
        // ZCode's schema accepts `interrupt` on a deny, but nothing in ZCode reads it.
        "codebuddy" | "zcode" => Some(basic),
        _ => None,
    }
}

/// Agents whose CLI answers `AskUserQuestion` / tool permissions through a `PermissionRequest` hook.
pub(crate) fn hooked(provider: &str) -> bool {
    matches!(provider, "claude" | "qoder" | "qoder-cn")
}

/// Agents whose tool approvals can be answered through a decision hook.
pub(crate) fn approval_hooked(provider: &str) -> bool {
    profile(provider).is_some()
}

/// The CLI configuration directory, or `None` for a source that is not one (Qoder desktop).
pub(crate) fn home(source: &Source) -> Option<std::path::PathBuf> {
    let home = expand(if source.path.is_empty() {
        crate::contracts::provider(&source.provider).map_or("", |p| p.default_path)
    } else {
        &source.path
    });
    (source.provider == "claude"
        || crate::qoder_desktop::database(&source.path).is_none() && home.join("projects").is_dir())
    .then_some(home)
}

pub(crate) fn question_hash(input: &Value) -> String {
    let mut questions = input["questions"].clone();
    if let Some(items) = questions.as_array_mut() {
        for q in items {
            q["multiSelect"] = json!(q["multiSelect"] == true);
        }
    }
    crate::models::hash(&questions.to_string())
}

pub(crate) fn approval_hash(tool_name: &str, input: &Value) -> String {
    crate::models::hash(&json!({"tool": tool_name, "input": input}).to_string())
}

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS channel_claude_requests (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, source_id TEXT NOT NULL, native TEXT NOT NULL,
        question_hash TEXT NOT NULL, call_id TEXT NOT NULL, input TEXT NOT NULL, pid INTEGER NOT NULL, process_start INTEGER NOT NULL,
        at INTEGER NOT NULL, heartbeat INTEGER NOT NULL, response TEXT, consumed INTEGER NOT NULL DEFAULT 0,
        kind TEXT NOT NULL DEFAULT 'question', tool_name TEXT NOT NULL DEFAULT 'AskUserQuestion',
        meta TEXT NOT NULL DEFAULT '{}', delivered_at INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS channel_claude_pending ON channel_claude_requests(epoch,source_id,native,question_hash);")?;
    let columns = || {
        let mut stmt = db.prepare("PRAGMA table_info(channel_claude_requests)")?;
        stmt.query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()
    };
    let migrations = [
        ("kind", "TEXT NOT NULL DEFAULT 'question'"),
        ("tool_name", "TEXT NOT NULL DEFAULT 'AskUserQuestion'"),
        ("meta", "TEXT NOT NULL DEFAULT '{}'"),
        ("delivered_at", "INTEGER NOT NULL DEFAULT 0"),
    ];
    let cols = columns()?;
    if migrations
        .iter()
        .any(|(name, _)| !cols.iter().any(|c| c == name))
    {
        // The app and an installed hook may open the same older database together.
        let tx =
            rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
        let cols = columns()?;
        for (name, definition) in migrations {
            if !cols.iter().any(|c| c == name) {
                tx.execute(
                    &format!("ALTER TABLE channel_claude_requests ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        tx.commit()?;
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS channel_claude_active ON channel_claude_requests(source_id,native,kind,consumed)",
        [],
    )?;
    Ok(())
}

/// Native session and tool-call IDs whose approval wait was settled by the helper's reply.
/// Legacy consumed rows and hash-only requests cannot prove that a tool call was answered.
pub(crate) fn delivered_approvals(
    db: &Connection,
    source_id: &str,
) -> rusqlite::Result<HashMap<String, HashMap<String, u64>>> {
    let mut stmt = db.prepare_cached(
        "SELECT native,call_id,MAX(delivered_at) FROM channel_claude_requests
         WHERE source_id=?1 AND kind='approval' AND consumed=1 AND delivered_at>0
           AND call_id<>'' AND call_id<>question_hash GROUP BY native,call_id",
    )?;
    let mut sessions = HashMap::<String, HashMap<String, u64>>::new();
    for row in stmt.query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (native, call, at) = row?;
        sessions.entry(native).or_default().insert(call, at);
    }
    Ok(sessions)
}

pub(crate) fn request(target: &Session, excerpt: &content::Excerpt) -> Option<Value> {
    let raw = excerpt.claude_request.as_ref()?;
    request_from_input(target, &raw["callId"], &raw["input"])
}

pub(crate) fn request_from_input(target: &Session, call_id: &Value, raw: &Value) -> Option<Value> {
    let items = raw["questions"].as_array()?;
    if items.is_empty() || items.len() > 4 {
        return None;
    }
    // Option previews share one budget so that a card with buttons stays deliverable.
    let previews = items
        .iter()
        .filter_map(|q| q["options"].as_array())
        .flatten()
        .filter(|o| o["preview"].is_string())
        .count();
    let limit = PREVIEW_BUDGET / previews.max(1);
    let questions = items
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut options = q["options"].clone();
            for option in options.as_array_mut().into_iter().flatten() {
                if let Some(text) = option["preview"].as_str().map(|s| content::clip(s, limit)) {
                    option["preview"] = json!(text);
                }
            }
            json!({"id":(i+1).to_string(),"header":q["header"].as_str().unwrap_or(""),"question":q["question"],"options":options,"multiSelect":q["multiSelect"]==true})
        })
        .collect::<Vec<_>>();
    Some(
        json!({"method":"aieyes/claudeQuestion","params":{"threadId":target.session.native_session,"turnId":target.session.turn_id,"itemId":call_id,"questions":questions}}),
    )
}

pub(super) fn updated_input(raw: &Value, text: &str) -> Result<Value> {
    let questions = raw["questions"].as_array().context("原会话问题已不可用")?;
    ensure!(
        !questions.is_empty() && questions.len() <= 4,
        "原会话问题已不可用"
    );
    let mut answers = serde_json::Map::new();
    for (i, q) in questions.iter().enumerate() {
        let reply = if questions.len() == 1 {
            text.trim()
        } else {
            let key = (i + 1).to_string();
            let matches = text
                .lines()
                .filter_map(|s| s.split_once('='))
                .filter(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim())
                .collect::<Vec<_>>();
            ensure!(matches.len() == 1, "请按「问题编号=答案」逐行回答所有问题");
            matches[0]
        };
        ensure!(!reply.is_empty(), "问题回答不能为空");
        let select = |s: &str| {
            s.parse::<usize>()
                .ok()
                .filter(|n| *n > 0)
                .and_then(|n| q["options"].as_array()?.get(n - 1)?["label"].as_str())
                .unwrap_or(s)
                .to_string()
        };
        let answer = if q["multiSelect"] == true {
            reply
                .split([',', '，'])
                .map(|s| select(s.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            select(reply)
        };
        answers.insert(
            q["question"]
                .as_str()
                .context("原会话问题已不可用")?
                .to_string(),
            json!(answer),
        );
    }
    let mut updated = raw.clone();
    updated["answers"] = json!(answers);
    Ok(
        json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":updated}}}),
    )
}

pub(crate) fn updated_input_structured(
    raw: &Value,
    answers_map: Option<&Value>,
    text: Option<&str>,
) -> Result<Value> {
    let Some(map) = answers_map.and_then(Value::as_object) else {
        return updated_input(raw, text.unwrap_or(""));
    };
    let questions = raw["questions"].as_array().context("原会话问题已不可用")?;
    ensure!(
        !questions.is_empty() && questions.len() <= 4,
        "原会话问题已不可用"
    );
    let mut answers = serde_json::Map::new();
    for (i, q) in questions.iter().enumerate() {
        let id = (i + 1).to_string();
        let q_text = q["question"].as_str().context("原会话问题已不可用")?;
        let raw_val = map
            .get(&id)
            .or_else(|| map.get(q_text))
            .context("请回答所有问题后再提交")?;
        let answer = if let Some(arr) = raw_val.as_array() {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        } else if let Some(s) = raw_val.as_str() {
            s.trim().to_owned()
        } else {
            String::new()
        };
        ensure!(!answer.is_empty(), "问题回答不能为空");
        answers.insert(q_text.to_string(), json!(answer));
    }
    let mut updated = raw.clone();
    updated["answers"] = json!(answers);
    Ok(
        json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":updated}}}),
    )
}

/// The question card of a pending question or MCP form; `None` when it cannot be presented.
pub(crate) fn question_card(target: &Session, req: &PendingClaude, lang: &str) -> Option<Value> {
    match req.kind.as_str() {
        "question" => request_from_input(target, &json!(req.call_id), &req.input),
        "elicitation" => elicitation::request(target, req, lang),
        _ => None,
    }
}

/// Unanswered hook requests for one native session, oldest first.
pub(crate) fn pending_requests(
    store: &Store,
    source_id: &str,
    native: &str,
) -> Result<Vec<PendingClaude>> {
    let mut stmt = store.db.prepare(
        "SELECT id,kind,tool_name,call_id,question_hash,input,pid,process_start,at,meta \
         FROM channel_claude_requests \
         WHERE source_id=?1 AND native=?2 AND consumed=0 AND response IS NULL AND heartbeat>?3 \
         ORDER BY at ASC, rowid ASC LIMIT 32",
    )?;
    let rows = stmt
        .query_map(params![source_id, native, now() - 5], |r| {
            Ok(PendingClaude {
                id: r.get(0)?,
                kind: r.get(1)?,
                tool_name: r.get(2)?,
                call_id: r.get(3)?,
                question_hash: r.get(4)?,
                input: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
                pid: r.get(6)?,
                process_start: r.get(7)?,
                at: r.get(8)?,
                meta: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or(Value::Null),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Hand a reply to the hook helper that holds `req`. The row can only be read by the helper that
/// registered it, so the reply cannot reach another prompt; the process check refuses a request
/// whose agent has exited or was replaced. Returns whether the helper has picked it up.
fn submit(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    response: &Value,
    gone: &'static str,
) -> Result<bool> {
    let process = Some(crate::sessions::ControlProcess {
        pid: req.pid,
        start: req.process_start,
    });
    ensure!(
        // The monitor binds no process of its own to agents that run inside a generic runtime;
        // their helper recorded the agent process it runs under.
        (target.process == process || !hooked(req.provider()))
            && req.pid > 0
            && req.process_start > 0
            && crate::sessions::process_matches(req.pid, req.process_start),
        "原会话进程已退出或身份已改变"
    );
    let changed = store.db.execute(
        "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",
        params![response.to_string(), req.id, now() - 5],
    )?;
    ensure!(changed == 1, gone);
    Ok(wait_consumed(store, &req.id, Duration::from_millis(1200)))
}

fn outcome(confirmed: bool, done: &str, sent: &str, receipt: Value) -> Outcome {
    if confirmed {
        Outcome::confirmed(done, receipt)
    } else {
        Outcome::submitted(sent, receipt)
    }
}

pub(crate) fn answer_pending_question(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    answers_map: Option<&Value>,
    text: Option<&str>,
) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        ensure!(req.kind == "question", "原会话问题已不可用");
        let response = updated_input_structured(&req.input, answers_map, text)?;
        let confirmed = submit(
            store,
            req,
            target,
            &response,
            "原会话的问题已结束或已被其他端处理",
        )?;
        Ok(outcome(
            confirmed,
            "原会话已接收回答",
            "回答已提交，等待原会话确认",
            json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id}),
        ))
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

/// What to do with a held MCP form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormReply {
    Accept,
    /// The user declines to provide the information.
    Decline,
    /// The user dismisses the dialog.
    Cancel,
}

pub(crate) fn answer_pending_form(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    reply: FormReply,
    answers_map: Option<&Value>,
    text: Option<&str>,
) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        ensure!(req.kind == "elicitation", "原会话问题已不可用");
        let (response, done, sent) = match reply {
            FormReply::Accept => (
                elicitation::payload(
                    "accept",
                    Some(elicitation::content(&req.input, answers_map, text)?),
                ),
                "原会话已接收回答",
                "回答已提交，等待原会话确认",
            ),
            FormReply::Decline => (
                elicitation::payload("decline", None),
                "已拒绝填写此表单",
                "已提交拒绝，等待原会话确认",
            ),
            FormReply::Cancel => (
                elicitation::payload("cancel", None),
                "已取消此表单",
                "已提交取消，等待原会话确认",
            ),
        };
        let confirmed = submit(
            store,
            req,
            target,
            &response,
            "原会话的问题已结束或已被其他端处理",
        )?;
        Ok(outcome(
            confirmed,
            done,
            sent,
            json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id}),
        ))
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

/// `decision` must come from `approval_card(req)`: its private payload is what gets sent.
pub(crate) fn answer_pending_approval(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    decision: &Decision,
    reason: Option<&str>,
) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        ensure!(req.kind == "approval", "不支持的审批操作");
        let response = card::reply(decision, reason)?;
        let confirmed = submit(
            store,
            req,
            target,
            &response,
            "原会话的审批请求已结束或已失效",
        )?;
        let allow = decision.behavior == "allow";
        // The helper has printed the decision, but the agent may still keep its own dialog.
        let confirmed = confirmed && !(allow && card::allow_may_be_discarded(&req.tool_name));
        let (done, sent) = if allow {
            ("已允许本次操作", "已提交允许，等待原会话确认")
        } else {
            ("已拒绝本次操作", "已提交拒绝，等待原会话确认")
        };
        Ok(outcome(
            confirmed,
            done,
            sent,
            json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id,
                "decision":if allow{"allow"}else{"deny"},"decisionId":decision.id}),
        ))
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

/// `None` when the request is no longer open; otherwise whether the hook has already released it.
pub(crate) fn handoff_request(store: &Store, req_id: &str) -> Result<Option<bool>> {
    let changed = store.db.execute(
        "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",
        params![PASSTHROUGH, req_id, now() - 5],
    )?;
    if changed != 1 {
        return Ok(None);
    }
    Ok(Some(wait_consumed(
        store,
        req_id,
        Duration::from_millis(1200),
    )))
}

fn wait_consumed(store: &Store, req_id: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let done: bool = store
            .db
            .query_row(
                "SELECT consumed=1 FROM channel_claude_requests WHERE id=?1",
                [req_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false);
        if done {
            return true;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    false
}

pub(super) fn answer(store: &Store, source: &Source, target: &Session, text: &str) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        let excerpt = content::local(source, target);
        let request =
            request(target, &excerpt).context("原会话的问题已结束或改变，请回复最新通知")?;
        let prompt: Option<String> = store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key=?1",
                [format!("channel.prompt:{}", target.session.id)],
                |r| r.get(0),
            )
            .optional()?;
        let prompt: Value = prompt
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        ensure!(
            prompt["expected"].as_str() == Some(&target.state_key)
                && prompt["hash"].as_str() == Some(&codex::fingerprint(&request)),
            "问题已改变或尚未同步，请回复最新的问题通知"
        );
        let raw = &excerpt.claude_request.as_ref().unwrap()["input"];
        let hash = question_hash(raw);
        let c = configuration(&store.db)?;
        let call = excerpt.claude_request.as_ref().unwrap()["callId"]
            .as_str()
            .context("原会话问题已不可用")?;
        let row:Option<(String,u32,u64,String)>=store.db.query_row("SELECT id,pid,process_start,input FROM channel_claude_requests WHERE epoch=?1 AND source_id=?2 AND native=?3 AND question_hash=?4 AND call_id=?5 AND kind='question' AND consumed=0 AND response IS NULL AND heartbeat>?6 ORDER BY at DESC LIMIT 1",params![c.epoch,source.id,target.session.native_session,hash,call,now()-4],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (id, pid, start, input) =
            row.context("Claude 问题接口尚未就绪，请在原窗口重新提问后重试")?;
        ensure!(
            target.process == Some(crate::sessions::ControlProcess { pid, start })
                && crate::sessions::process_matches(pid, start),
            "原会话进程已退出或身份已改变"
        );
        let response = updated_input(&serde_json::from_str(&input)?, text)?;
        let changed=store.db.execute("UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",params![response.to_string(),id,now()-4])?;
        ensure!(changed == 1, "原会话的问题已结束或改变，请回复最新通知");
        Ok(Outcome::submitted(
            "回答已提交，等待原会话确认",
            json!({"adapter":"claudeHook","requestId":id,"callId":call}),
        ))
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

pub(crate) fn install(store: &Store) -> Result<()> {
    sync_hooks(store)
}

/// Where an agent keeps its hook configuration.
struct Site {
    home: PathBuf,
    config: PathBuf,
    /// ZCode nests events under `hooks.events` and runs handlers as processes.
    zcode: bool,
}

fn site(source: &Source) -> Option<Site> {
    if hooked(&source.provider) {
        return home(source).map(|home| Site {
            config: home.join("settings.json"),
            home,
            zcode: false,
        });
    }
    profile(&source.provider)?;
    let target = crate::session_hooks::target(source);
    // The CodeBuddy IDE extension keeps history there, not a CLI configuration.
    (!crate::codebuddy_ide::is_path(&target.home)).then(|| Site {
        home: target.home,
        config: target.config,
        zcode: source.provider == "zcode",
    })
}

fn owned(handler: &Value) -> bool {
    handler["command"]
        .as_str()
        .is_some_and(|s| s.contains(MARKER))
        || handler["args"].as_array().is_some_and(|args| {
            args.iter()
                .any(|a| a.as_str().is_some_and(|s| s.contains(MARKER)))
        })
}

/// `(event, matcher, timeout in seconds)` of the hooks this configuration asks for.
fn wanted(
    provider: &str,
    feishu: bool,
    desktop_questions: bool,
    desktop_approvals: bool,
) -> Vec<(&'static str, &'static str, u64)> {
    let mut out = Vec::new();
    let Some(profile) = profile(provider) else {
        return out;
    };
    if profile.questions {
        if feishu || desktop_questions {
            let matcher = if desktop_approvals {
                "*"
            } else {
                "AskUserQuestion"
            };
            // One entry serves questions too, which wait for as long as the user needs.
            out.push(("PermissionRequest", matcher, 86400));
        }
    } else if desktop_approvals {
        out.push(("PermissionRequest", "*", SEQUENTIAL_HOLD + 15));
    }
    if profile.elicitation && desktop_questions {
        // The agent shows its own form only after this hook returns.
        out.push(("Elicitation", "*", SEQUENTIAL_HOLD + 15));
    }
    out
}

/// Replace the owned handlers of a hook configuration with `entries`; everything else is kept.
fn reconcile(
    value: &mut Value,
    zcode: bool,
    entries: &[(&'static str, &'static str, u64)],
    command: &str,
) -> Result<()> {
    ensure!(
        value.is_object() && (value["hooks"].is_null() || value["hooks"].is_object()),
        "hooks 配置格式无效，原配置已保留"
    );
    // ZCode runs hooks only when the user has switched them on; that switch is not ours.
    let entries = if zcode && value["hooks"]["enabled"] != true {
        &[][..]
    } else {
        entries
    };
    let container = if zcode {
        &value["hooks"]["events"]
    } else {
        &value["hooks"]
    };
    ensure!(
        container.is_null() || container.is_object(),
        "hooks 配置格式无效，原配置已保留"
    );
    if container.is_null() && entries.is_empty() {
        return Ok(());
    }
    let events = if zcode {
        &mut value["hooks"]["events"]
    } else {
        &mut value["hooks"]
    };
    if events.is_null() {
        *events = json!({});
    }
    let map = events.as_object_mut().context("无法读取会话来源")?;
    let mut emptied = Vec::new();
    for (event, rows) in map.iter_mut() {
        let Some(rows) = rows.as_array_mut() else {
            continue;
        };
        let before = rows.len();
        rows.retain_mut(|row| {
            let Some(handlers) = row["hooks"].as_array_mut() else {
                return true;
            };
            let had = handlers.len();
            handlers.retain(|h| !owned(h));
            handlers.len() == had || !handlers.is_empty()
        });
        if rows.is_empty() && before > 0 && event != "PermissionRequest" {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        map.remove(&event);
    }
    for (event, matcher, timeout) in entries {
        let handler = if !zcode {
            json!({"type":"command","command":command,"timeout":timeout})
        } else if cfg!(windows) {
            json!({"type":"process","command":"cmd.exe","args":["/d","/s","/c",command],"timeoutMs":timeout * 1000})
        } else {
            json!({"type":"process","command":"/bin/sh","args":["-c",command],"timeoutMs":timeout * 1000})
        };
        let rows = map.entry(*event).or_insert_with(|| json!([]));
        rows.as_array_mut()
            .context("hooks 配置格式无效，原配置已保留")?
            .push(json!({"matcher":matcher,"hooks":[handler]}));
    }
    Ok(())
}

fn hook_command(helper: &Path, root: &Path, source_id: &str, zcode: bool) -> Result<String> {
    let values = [
        helper.to_string_lossy().into_owned(),
        MARKER.to_owned(),
        root.to_string_lossy().into_owned(),
        source_id.to_owned(),
    ];
    // ZCode starts the handler through cmd.exe on Windows; Claude-style hooks run in a POSIX shell.
    if zcode && cfg!(windows) {
        ensure!(
            values.iter().all(|s| !s.contains(['"', '%', '\n', '\r'])),
            "Hook 路径包含不支持的 shell 字符"
        );
        return Ok(values
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(" "));
    }
    Ok(values
        .iter()
        .map(|s| crate::process::quote(s))
        .collect::<Vec<_>>()
        .join(" "))
}

/// The helper is copied next to the configuration so that the hook survives an app update.
struct Helper {
    exe: PathBuf,
    digest: String,
}

impl Helper {
    fn current() -> Result<Self> {
        let exe = crate::credentials::helper()?;
        let bytes = fs::read(&exe)?;
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Ok(Self { exe, digest })
    }
    fn path(&self, folder: &Path) -> PathBuf {
        folder.join(format!(
            "aieyes-question-{}{}",
            self.digest,
            std::env::consts::EXE_SUFFIX
        ))
    }
}

/// Reconcile only the owned interaction hooks. Preserve all unrelated hooks and permissions.
pub(crate) fn sync_hooks(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    let dc = crate::session_interaction::config(&store.db)?;
    let feishu = c.enabled && c.owner.is_some();
    let desktop_questions = dc.enabled && (dc.claude_questions || dc.claude_approvals);
    let desktop_approvals = dc.enabled && dc.claude_approvals;

    let settings = store.settings()?;
    let mut seen = std::collections::HashSet::new();
    let mut helper: Option<Helper> = None;
    let mut failure = None;
    for source in settings
        .sources
        .iter()
        .filter(|s| s.host_id.is_none() && settings.source_enabled(s))
    {
        let Some(site) = site(source) else {
            continue;
        };
        if !seen.insert(site.config.clone()) {
            continue;
        }
        let entries = wanted(
            &source.provider,
            feishu,
            desktop_questions,
            desktop_approvals,
        );
        if let Err(error) = sync_site(store, source, &site, &entries, &mut helper) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn sync_site(
    store: &Store,
    source: &Source,
    site: &Site,
    entries: &[(&'static str, &'static str, u64)],
    helper: &mut Option<Helper>,
) -> Result<()> {
    let path = &site.config;
    if !site.home.is_dir() || fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Ok(());
    }
    let read = || -> Result<Option<Vec<u8>>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            // ZCode's switch lives in the file; without the file there is nothing to honour.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok((!entries.is_empty() && !site.zcode).then(|| b"{}".to_vec()))
            }
            Err(e) => Err(e.into()),
        }
    };
    let folder = site.home.join(".aieyes");
    // New bytes for the configuration, or `None` when it is already as wanted.
    let plan = |original: &[u8], helper: &mut Option<Helper>| -> Result<Option<Vec<u8>>> {
        // Only touch a configuration that has, or should have, an owned entry.
        if entries.is_empty() && !String::from_utf8_lossy(original).contains(MARKER) {
            return Ok(None);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(original) else {
            ensure!(entries.is_empty(), "hooks 配置格式无效，原配置已保留");
            return Ok(None);
        };
        let before = value.clone();
        let command = if entries.is_empty() {
            String::new()
        } else {
            if helper.is_none() {
                *helper = Some(Helper::current()?);
            }
            hook_command(
                &helper.as_ref().unwrap().path(&folder),
                &store.root,
                &source.id,
                site.zcode,
            )?
        };
        reconcile(&mut value, site.zcode, entries, &command)?;
        Ok((value != before)
            .then(|| serde_json::to_vec_pretty(&value))
            .transpose()?)
    };
    let Some(original) = read()? else {
        return Ok(());
    };
    if plan(&original, helper)?.is_none() {
        return Ok(());
    }
    fs::create_dir_all(&folder)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("session-hooks.lock"))?;
    let _lock = crate::file_lock::FileLock::try_exclusive(lock)?;
    // Another writer may have changed the file before the lock was taken.
    let Some(original) = read()? else {
        return Ok(());
    };
    let Some(updated) = plan(&original, helper)? else {
        return Ok(());
    };
    if let Some(helper) = helper.as_ref().filter(|_| !entries.is_empty()) {
        let copy = helper.path(&folder);
        if !copy.exists() {
            let tmp = copy.with_extension("tmp");
            fs::copy(&helper.exe, &tmp)?;
            fs::rename(tmp, &copy)?;
        }
    }
    let backup = folder.join(format!(
        "channel-settings-before-{}.json",
        &crate::models::hash(&String::from_utf8_lossy(&original))[..16]
    ));
    if !backup.exists() {
        private_write(&backup, &original)?;
    }
    let tmp = folder.join("channel-settings.pending");
    private_write(&tmp, &updated)?;
    // An existing configuration keeps its mode; a new one stays private.
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(&tmp, meta.permissions())?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

pub(crate) fn hook_summary(store: &Store) -> Value {
    let Ok(settings) = store.settings() else {
        return json!([]);
    };
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for source in settings
        .sources
        .iter()
        .filter(|s| s.host_id.is_none() && settings.source_enabled(s))
    {
        let Some(site) = site(source) else {
            continue;
        };
        if !seen.insert(site.config.clone()) {
            continue;
        }
        let config = fs::read(&site.config)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        let events = if site.zcode {
            &config["hooks"]["events"]
        } else {
            &config["hooks"]
        };
        let matcher = |event: &str| {
            events[event].as_array().and_then(|rows| {
                rows.iter().find_map(|row| {
                    row["hooks"]
                        .as_array()
                        .is_some_and(|handlers| handlers.iter().any(owned))
                        .then(|| row["matcher"].as_str().unwrap_or("*").to_owned())
                })
            })
        };
        rows.push(json!({
            "sourceId": source.id,
            "provider": source.provider,
            "path": site.config.to_string_lossy(),
            // `none` | `AskUserQuestion` (questions only) | `*` (questions and approvals).
            "mode": matcher("PermissionRequest").unwrap_or_else(|| "none".into()),
            "elicitation": matcher("Elicitation").is_some(),
        }));
    }
    json!(rows)
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

/// Entrypoint used by the interaction hooks, before service startup.
pub fn run_if_requested() -> bool {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(i) = args.iter().position(|a| a == MARKER) else {
        return false;
    };
    // Any failure leaves the request to the agent's own dialog: nothing is printed.
    let _ = (|| -> Result<()> {
        let root = Path::new(args.get(i + 1).context("missing root")?);
        let source_id = args.get(i + 2).context("missing source")?;
        hold(root, source_id)
    })();
    true
}

static NULL: Value = Value::Null;

/// The first of `names` the agent sent; agents differ in snake_case and camelCase.
fn field<'a>(input: &'a Value, names: &[&str]) -> &'a Value {
    names
        .iter()
        .map(|name| &input[*name])
        .find(|value| !value.is_null())
        .unwrap_or(&NULL)
}

fn valid_native(provider: &str, id: &str) -> bool {
    if hooked(provider) {
        return codex::valid_id(id);
    }
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The agent process this helper runs under: `(pid, start, headless verdict)`.
fn agent_process(provider: &str) -> (u32, u64, Option<&'static str>) {
    if hooked(provider) {
        return crate::session_hooks::ancestry(provider);
    }
    // These agents run inside a generic runtime, so the process is recognised by its argv.
    let mut system = sysinfo::System::new();
    let mut pid = sysinfo::Pid::from_u32(std::process::id());
    for _ in 0..8 {
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
        );
        let Some(process) = system.process(pid) else {
            break;
        };
        let argv: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        // The helper and the shell that launched it name the agent's directory too.
        if !argv.iter().any(|a| a.contains(MARKER))
            && crate::session_events::is_program(provider, &argv)
        {
            return (
                pid.as_u32(),
                process.start_time(),
                crate::session_events::headless_argv(provider, &argv),
            );
        }
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    (0, 0, None)
}

/// Whether the agent that started this helper is gone and nobody will read the reply.
fn orphaned(pid: u32, start: u64) -> bool {
    #[cfg(unix)]
    if std::os::unix::process::parent_id() == 1 {
        return true;
    }
    !crate::sessions::process_matches(pid, start)
}

/// Register one hook request and wait until it is answered, released or settled elsewhere.
fn hold(root: &Path, source_id: &str) -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_STDIN + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_STDIN, "input too large");
    let input: Value = serde_json::from_slice(&bytes)?;
    let form_order = elicitation::field_order(&bytes);
    drop(bytes);
    let event = field(&input, &["hook_event_name", "hookEventName"])
        .as_str()
        .unwrap_or("");
    ensure!(
        matches!(event, "PermissionRequest" | "Elicitation"),
        "not an interaction request"
    );
    let store = Store::open(root)?;
    let settings = store.settings()?;
    let installed = settings
        .sources
        .iter()
        .find(|s| s.id == *source_id && s.host_id.is_none() && settings.source_enabled(s))
        .context("source unavailable")?;
    let provider = installed.provider.as_str();
    let profile = profile(provider).context("source unavailable")?;
    let native = field(&input, &["session_id", "sessionId"])
        .as_str()
        .filter(|s| valid_native(provider, s))
        .context("missing native id")?;

    let c = configuration(&store.db)?;
    let dc = crate::session_interaction::config(&store.db)?;
    let feishu_alive = c.enabled && c.owner.is_some() && service_alive(&store, &c.epoch);
    let desktop_q_alive = dc.enabled
        && (dc.claude_questions || dc.claude_approvals)
        && desktop_alive(&store, DESKTOP_QUESTION_TTL);
    let desktop_a_alive =
        dc.enabled && dc.claude_approvals && desktop_alive(&store, DESKTOP_APPROVAL_TTL);
    let desktop_form_alive = dc.enabled
        && (dc.claude_questions || dc.claude_approvals)
        && desktop_alive(&store, DESKTOP_APPROVAL_TTL);

    let tool_name = if event == "Elicitation" {
        "Elicitation"
    } else {
        field(&input, &["tool_name", "toolName"])
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .context("missing tool_name")?
    };
    let raw = field(&input, &["tool_input", "toolInput"]);
    let kind = match (event, tool_name) {
        ("Elicitation", _) => "elicitation",
        (_, "AskUserQuestion") => "question",
        _ => "approval",
    };
    match kind {
        "elicitation" => ensure!(profile.elicitation && desktop_form_alive, "forms disabled"),
        "question" => {
            ensure!(
                profile.questions && (feishu_alive || desktop_q_alive),
                "disabled"
            );
            let q = raw["questions"].as_array().context("missing questions")?;
            ensure!(!q.is_empty() && q.len() <= 4, "invalid questions");
        }
        _ => ensure!(desktop_a_alive, "approvals disabled"),
    }

    let (pid, start, verdict) = agent_process(provider);
    ensure!(
        pid > 0 && start > 0 && verdict != Some(crate::session_events::HEADLESS),
        "not an interactive agent process"
    );
    let session_id = crate::models::hash(&format!("local:{provider}:{native}"));
    // A copied hook must never bind a different source's native session.
    let bound = if hooked(provider) {
        let home = home(installed).context("source unavailable")?;
        let transcript = input["transcript_path"]
            .as_str()
            .map(Path::new)
            .context("missing transcript")?
            .canonicalize()?;
        ensure!(
            transcript.starts_with(home.canonicalize()?)
                && transcript.file_stem().and_then(|s| s.to_str()) == Some(native),
            "source mismatch"
        );
        Some((home, transcript))
    } else {
        // No transcript names the session here, so it must be one the monitor has already
        // seen in this very source.
        let until = Instant::now() + Duration::from_millis(1500);
        loop {
            let known = crate::session_interaction::session(&store.db, &session_id)?.is_some_and(
                |target| {
                    target.session.source_id == installed.id
                        && target.session.provider == provider
                        && target.session.machine_id == "local"
                        && target.session.native_session == native
                },
            );
            if known {
                break None;
            }
            ensure!(Instant::now() < until, "session not observed");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // The same directory can be configured as more than one source; use the one that shows
    // the session.
    let observed_source = || {
        let home = &bound.as_ref()?.0;
        let target = crate::session_interaction::session(&store.db, &session_id)
            .ok()
            .flatten()?;
        settings
            .sources
            .iter()
            .find(|s| {
                s.id == target.session.source_id
                    && s.provider == provider
                    && s.host_id.is_none()
                    && settings.source_enabled(s)
                    && self::home(s).as_ref() == Some(home)
            })
            .map(|s| s.id.clone())
    };
    let named_call = || {
        ["tool_use_id", "call_id", "toolCallId"]
            .iter()
            .find_map(|k| input[*k].as_str())
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .map(str::to_owned)
    };
    let mut meta = json!({
        "provider": provider,
        "cwd": content::clip(input["cwd"].as_str().unwrap_or(""), 1000),
    });

    let transcript = bound.as_ref().map(|(_, transcript)| transcript.as_path());
    // Claude Code keeps its own dialog open while a `PermissionRequest` hook runs; the
    // transcript tells when the request was settled there. A subagent's calls are not in the
    // session's own transcript. An MCP form has no terminal dialog until the hook returns, so
    // there is nothing to follow.
    let followed = transcript.filter(|_| profile.concurrent && input["agent_id"].is_null());
    let (row_source, call, hash, stored, mut watch) = match kind {
        "elicitation" => {
            let stored = elicitation::stored(&input, form_order).context("form not presentable")?;
            let hash = crate::models::hash(&stored.to_string());
            let call = input["elicitation_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .map(str::to_owned)
                .unwrap_or_else(|| hash.clone());
            (
                observed_source().unwrap_or_else(|| installed.id.clone()),
                call,
                hash,
                stored,
                None,
            )
        }
        "question" => {
            let (home, _) = bound.as_ref().context("source unavailable")?;
            ensure!(raw.to_string().len() <= STORE_BUDGET, "question too large");
            let watch = followed.and_then(|transcript| {
                watch::Watch::open(
                    transcript,
                    watch::Subject::Question {
                        hash: question_hash(raw),
                    },
                )
            });
            let hash = question_hash(raw);
            // The chat channel matches its reply by the transcript's call ID, so it waits for the
            // session row. The desktop card matches by request row and must not lose a question
            // that arrives before the next session poll (new session, first message of a turn).
            let until_observed = Instant::now()
                + if feishu_alive {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(600)
                };
            let (sid, call) = loop {
                let candidate = if feishu_alive {
                    super::db::session(&store.db, &c.epoch, &session_id)?
                } else {
                    None
                }
                .or_else(|| {
                    crate::session_interaction::session(&store.db, &session_id)
                        .ok()
                        .flatten()
                });
                if let Some(target) = candidate
                    && let Some(source) = settings.sources.iter().find(|s| {
                        s.id == target.session.source_id
                            && s.provider == installed.provider
                            && s.host_id.is_none()
                            && settings.source_enabled(s)
                            && self::home(s).as_ref() == Some(home)
                    })
                {
                    let excerpt = content::local(source, &target);
                    if let Some(request) = excerpt.claude_request
                        && question_hash(&request["input"]) == hash
                        && let Some(call) = request["callId"].as_str().filter(|s| !s.is_empty())
                    {
                        break (source.id.clone(), call.to_string());
                    }
                }
                if Instant::now() >= until_observed {
                    ensure!(desktop_q_alive, "question not observed");
                    // Same binding as an approval: the transcript path proved the source above.
                    break (installed.id.clone(), named_call().unwrap_or_default());
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            (sid, call, hash, raw.clone(), watch)
        }
        _ => {
            // The hash covers the whole input; what is stored is only what a card can show.
            let hash = approval_hash(tool_name, raw);
            let mut budget = STORE_BUDGET;
            let (stored, truncated) = card::display_copy(raw, &mut budget);
            meta["truncated"] = json!(truncated);
            if profile.suggestions {
                meta["suggestions"] = json!(card::suggestions(&input["permission_suggestions"]));
            }
            if tool_name == "Write"
                && let Some(path) = raw["file_path"].as_str().map(Path::new)
                && path.is_absolute()
            {
                meta["exists"] = json!(path.exists());
            }
            let watch =
                followed.and_then(|transcript| watch::Watch::open_tool(transcript, tool_name, raw));
            (
                observed_source().unwrap_or_else(|| installed.id.clone()),
                named_call().unwrap_or_default(),
                hash,
                stored,
                watch,
            )
        }
    };
    let call = if call.is_empty() {
        watch
            .as_ref()
            .and_then(|w| w.call_id())
            .map(str::to_owned)
            .unwrap_or_else(|| hash.clone())
    } else {
        call
    };

    let id = super::random_id()?;
    store.db.execute(
        "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name,meta) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,?11,?12,?13)",
        params![
            id,
            c.epoch,
            row_source,
            native,
            hash,
            call,
            stored.to_string(),
            pid,
            start,
            now(),
            kind,
            tool_name,
            meta.to_string()
        ],
    )?;
    drop(stored);

    let approval_tool = (kind == "approval").then_some(tool_name);
    let until = Instant::now() + Duration::from_secs(hold_secs(kind, profile));
    // Whether an answer that arrives as the wait ends may still be handed to the agent.
    let mut deliver = true;
    let mut checked = Instant::now() - Duration::from_secs(1);
    while Instant::now() < until {
        let response: Option<String> = store
            .db
            .query_row(
                "SELECT response FROM channel_claude_requests WHERE id=?1 AND consumed=0",
                [&id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(response) = response {
            release(
                &store.db,
                &id,
                &response,
                approval_tool,
                raw,
                true,
                &mut io::stdout().lock(),
            )?;
            return Ok(());
        }
        if checked.elapsed() >= Duration::from_secs(1) {
            let current = configuration(&store.db)?;
            let dc = crate::session_interaction::config(&store.db)?;
            let settings = store.settings()?;
            let source_ok = settings
                .sources
                .iter()
                .any(|s| s.id == row_source && settings.source_enabled(s));
            let desktop = |ttl| {
                dc.enabled
                    && (dc.claude_questions || dc.claude_approvals)
                    && desktop_alive(&store, ttl)
            };
            let still_active = source_ok
                && match kind {
                    "question" => {
                        current.enabled
                            && current.epoch == c.epoch
                            && current.owner.is_some()
                            && service_alive(&store, &c.epoch)
                            || desktop(DESKTOP_QUESTION_TTL)
                    }
                    "elicitation" => desktop(DESKTOP_APPROVAL_TTL),
                    _ => dc.claude_approvals && desktop(DESKTOP_APPROVAL_TTL),
                };
            if !still_active {
                break;
            }
            // The terminal has decided, or the agent is gone: a late answer must not be printed.
            if orphaned(pid, start) || watch.as_mut().is_some_and(watch::Watch::settled) {
                deliver = false;
                break;
            }
            store.db.execute(
                "UPDATE channel_claude_requests SET heartbeat=?1 WHERE id=?2",
                params![now(), id],
            )?;
            checked = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    give_up(
        &store.db,
        &id,
        approval_tool,
        raw,
        deliver,
        &mut io::stdout().lock(),
    )
}

/// How long the helper holds a request, in seconds.
fn hold_secs(kind: &str, profile: Profile) -> u64 {
    match kind {
        "question" => QUESTION_HOLD,
        // Claude Code and Qoder run the `Elicitation` hook before they show their own form, so
        // an unanswered card would hide the form for as long as the helper waits.
        "elicitation" => SEQUENTIAL_HOLD,
        // The agent shows nothing in its terminal until this hook returns.
        _ if !profile.concurrent => SEQUENTIAL_HOLD,
        _ => APPROVAL_HOLD,
    }
}

/// Write the reply and close the request. Record a delivery timestamp when the reply
/// settles the approval. Claude may leave the terminal dialog open after an MCP allow.
/// The row keeps its identity for receipts, not the prompt or the answer.
fn release(
    db: &Connection,
    id: &str,
    response: &str,
    approval_tool: Option<&str>,
    raw: &Value,
    deliver: bool,
    out: &mut impl Write,
) -> Result<()> {
    let mut delivered_at = 0;
    let result = if deliver && response != PASSTHROUGH {
        let (reply, settles_approval) = match approval_tool.and_then(|tool| {
            serde_json::from_str::<Value>(response)
                .ok()
                .map(|value| (tool, value))
        }) {
            Some((tool, value)) => {
                let settles = match value["hookSpecificOutput"]["decision"]["behavior"].as_str() {
                    Some("deny") => true,
                    Some("allow") => !card::allow_may_be_discarded(tool),
                    _ => false,
                };
                (card::finish(value, raw).to_string(), settles)
            }
            None => (response.to_owned(), false),
        };
        // Sample before writing so a new wait initiated after this reply stays visible.
        let at = if settles_approval {
            chrono::Utc::now().timestamp_millis()
        } else {
            0
        };
        writeln!(out, "{reply}")
            .and_then(|()| out.flush())
            .inspect(|()| delivered_at = at)
    } else {
        Ok(())
    };
    db.execute(
        "UPDATE channel_claude_requests SET consumed=1,heartbeat=?1,input='{}',meta='{}',response='',delivered_at=?2 WHERE id=?3 AND consumed=0",
        params![now(), delivered_at, id],
    )?;
    result?;
    Ok(())
}

/// The helper stops waiting. Removing the open row decides the race with an answer written at
/// the same moment: if nothing was removed, that answer is in the row and is released like
/// any other, so the card is confirmed and no answered row is left behind.
fn give_up(
    db: &Connection,
    id: &str,
    approval_tool: Option<&str>,
    raw: &Value,
    deliver: bool,
    out: &mut impl Write,
) -> Result<()> {
    let removed = db.execute(
        "DELETE FROM channel_claude_requests WHERE id=?1 AND response IS NULL",
        [id],
    )?;
    if removed > 0 {
        return Ok(());
    }
    let late: Option<String> = db
        .query_row(
            "SELECT response FROM channel_claude_requests WHERE id=?1 AND consumed=0",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    match late {
        Some(response) => release(db, id, &response, approval_tool, raw, deliver, out),
        None => Ok(()),
    }
}

fn service_alive(store: &Store, epoch: &str) -> bool {
    let raw: Option<String> = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='channel.connection'",
            [],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    raw.and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .is_some_and(|v| v["epoch"] == epoch && v["at"].as_i64().is_some_and(|n| now() - n < 45))
}

pub(crate) fn desktop_alive(store: &Store, max_age: i64) -> bool {
    let raw: Option<String> = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='interaction.heartbeat'",
            [],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    raw.and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .and_then(|v| v["at"].as_i64())
        .is_some_and(|n| now() - n < max_age)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "'/h/.aieyes/aieyes-question-1' --aieyes-claude-question '/root' 'src'";

    #[test]
    fn owned_entries_are_replaced_and_everything_else_is_kept() {
        let mut value = json!({"env": {"KEEP": "1"}, "hooks": {
            "PermissionRequest": [
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "mine.sh"}]},
                {"matcher": "AskUserQuestion", "hooks": [
                    {"type": "command", "command": "/old/aieyes-question-0 --aieyes-claude-question /r s"},
                    {"type": "command", "command": "also-mine.sh"}]},
                {"matcher": "*", "hooks": [{"type": "command", "command": "/old --aieyes-claude-question /r s"}]},
                {"matcher": "Empty", "hooks": []}
            ],
            "Elicitation": [{"matcher": "*", "hooks": [{"type": "command", "command": "/old --aieyes-claude-question /r s"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "x --aieyes-session-hook claude Stop"}]}]
        }});
        let entries = wanted("claude", false, true, true);
        assert_eq!(
            entries,
            [
                ("PermissionRequest", "*", 86400),
                ("Elicitation", "*", SEQUENTIAL_HOLD + 15)
            ]
        );
        reconcile(&mut value, false, &entries, CMD).unwrap();
        let rows = value["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["hooks"][0]["command"], "mine.sh");
        assert_eq!(
            rows[1]["hooks"],
            json!([{"type": "command", "command": "also-mine.sh"}])
        );
        assert_eq!(rows[2]["matcher"], "Empty");
        assert_eq!(
            rows[3],
            json!({"matcher": "*", "hooks": [{"type": "command", "command": CMD, "timeout": 86400}]})
        );
        assert_eq!(value["hooks"]["Elicitation"].as_array().unwrap().len(), 1);
        assert_eq!(value["hooks"]["Elicitation"][0]["hooks"][0]["command"], CMD);
        assert_eq!(value["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(value["env"]["KEEP"], "1");

        // A second pass changes nothing; switching everything off removes only what is owned.
        let stable = value.clone();
        reconcile(&mut value, false, &entries, CMD).unwrap();
        assert_eq!(value, stable);
        reconcile(&mut value, false, &[], "").unwrap();
        assert_eq!(
            value["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(value["hooks"].get("Elicitation").is_none());
        assert!(!value.to_string().contains(MARKER));
        assert_eq!(value["hooks"]["Stop"].as_array().unwrap().len(), 1);

        for bad in [
            json!([]),
            json!({"hooks": []}),
            json!({"hooks": {"PermissionRequest": {}}}),
        ] {
            let mut bad = bad;
            assert!(reconcile(&mut bad, false, &entries, CMD).is_err());
        }
    }

    #[test]
    fn each_agent_gets_only_the_hooks_it_can_answer() {
        // Questions alone never widen the matcher to tool approvals.
        assert_eq!(
            wanted("qoder", false, true, false),
            [
                ("PermissionRequest", "AskUserQuestion", 86400),
                ("Elicitation", "*", SEQUENTIAL_HOLD + 15)
            ]
        );
        // The chat channel answers questions only; forms need the desktop card.
        assert_eq!(
            wanted("claude", true, false, false),
            [("PermissionRequest", "AskUserQuestion", 86400)]
        );
        assert!(wanted("claude", false, false, false).is_empty());
        for provider in ["codebuddy", "zcode"] {
            assert!(wanted(provider, true, true, false).is_empty());
            assert_eq!(
                wanted(provider, false, true, true),
                [("PermissionRequest", "*", SEQUENTIAL_HOLD + 15)]
            );
        }
        assert!(wanted("codex", true, true, true).is_empty());
        assert!(hooked("qoder-cn") && !hooked("codebuddy"));
        assert!(approval_hooked("zcode") && !approval_hooked("cursor"));

        // ZCode: nested events, process handlers, and only while its own switch is on.
        let mut off = json!({"hooks": {"enabled": false}});
        let entries = wanted("zcode", false, true, true);
        reconcile(&mut off, true, &entries, CMD).unwrap();
        assert_eq!(off, json!({"hooks": {"enabled": false}}));
        let mut on = json!({"hooks": {"enabled": true, "events": {"PermissionRequest": [
            {"hooks": [{"type": "process", "command": "/bin/sh", "args": ["-c", "obs --aieyes-session-hook zcode PermissionRequest"], "timeoutMs": 2000}]}
        ]}}});
        reconcile(&mut on, true, &entries, CMD).unwrap();
        let rows = on["hooks"]["events"]["PermissionRequest"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["hooks"][0]["type"], "process");
        assert_eq!(
            rows[1]["hooks"][0]["timeoutMs"],
            (SEQUENTIAL_HOLD + 15) * 1000
        );
        assert!(owned(&rows[1]["hooks"][0]) && !owned(&rows[0]["hooks"][0]));
        // Switched off later: the owned handler goes, the observer stays.
        on["hooks"]["enabled"] = json!(false);
        reconcile(&mut on, true, &entries, CMD).unwrap();
        assert_eq!(
            on["hooks"]["events"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn hook_input_fields_are_read_in_either_spelling() {
        let snake =
            json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash", "session_id": "s"});
        let camel = json!({"hookEventName": "PermissionRequest", "toolName": "Bash", "sessionId": "s", "tool_name": null});
        for input in [&snake, &camel] {
            assert_eq!(
                field(input, &["hook_event_name", "hookEventName"]),
                "PermissionRequest"
            );
            assert_eq!(field(input, &["tool_name", "toolName"]), "Bash");
            assert!(field(input, &["tool_input", "toolInput"]).is_null());
        }
        assert!(valid_native(
            "claude",
            "0f8fad5b-d9cb-469f-a165-70867728950e"
        ));
        assert!(!valid_native("claude", "session-1"));
        assert!(valid_native("zcode", "sess_01HZX.a-b"));
        assert!(!valid_native("zcode", "../x") && !valid_native("codebuddy", ""));
    }

    /// Claude Code and Qoder show an MCP form only after the `Elicitation` hook has returned.
    #[test]
    fn a_form_is_held_briefly_whatever_the_agent() {
        let claude = profile("claude").unwrap();
        let zcode = profile("zcode").unwrap();
        assert_eq!(hold_secs("elicitation", claude), SEQUENTIAL_HOLD);
        assert_eq!(hold_secs("approval", claude), APPROVAL_HOLD);
        assert_eq!(hold_secs("question", claude), QUESTION_HOLD);
        assert_eq!(hold_secs("approval", zcode), SEQUENTIAL_HOLD);
        // ZCode validates `interrupt` on a deny and then ignores it.
        assert!(!zcode.interrupt && claude.interrupt);
    }

    #[test]
    fn an_answer_written_as_the_wait_ends_is_released_not_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let insert = |id: &str, response: Option<&str>| {
            store.db.execute(
                "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name,meta,response) VALUES(?1,'','src','n','h',?1,'{\"command\":\"secret\"}',1,1,?2,?2,'approval','Bash','{\"cwd\":\"/w\"}',?3)",
                params![id, now(), response],
            )
            .unwrap();
        };
        let row = |id: &str| -> Option<(String, String, Option<String>, i64)> {
            store
                .db
                .query_row(
                    "SELECT input,meta,response,consumed FROM channel_claude_requests WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .unwrap()
        };
        let scrubbed = Some(("{}".to_owned(), "{}".to_owned(), Some(String::new()), 1));
        let raw = json!({"command": "secret"});
        let allow = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow"}}})
        .to_string();
        let mut out = Vec::new();

        // Nobody answered: the open row is removed and nothing is printed.
        insert("open", None);
        give_up(&store.db, "open", Some("Bash"), &raw, true, &mut out).unwrap();
        assert!(row("open").is_none());
        assert!(out.is_empty());

        // Answered between the last poll and the end of the wait: delivered and confirmed.
        insert("late", Some(&allow));
        give_up(&store.db, "late", Some("Bash"), &raw, true, &mut out).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&out).unwrap()["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
        assert_eq!(row("late"), scrubbed);
        out.clear();

        // The terminal already decided: the row is closed, the late answer is not printed.
        insert("settled", Some(&allow));
        give_up(&store.db, "settled", Some("Bash"), &raw, false, &mut out).unwrap();
        assert!(out.is_empty());
        assert_eq!(row("settled"), scrubbed);

        // A hand-back is released without output.
        insert("back", Some(PASSTHROUGH));
        give_up(&store.db, "back", Some("Bash"), &raw, true, &mut out).unwrap();
        assert!(out.is_empty());
        assert_eq!(row("back"), scrubbed);

        // The MCP decision is printed, but it may leave a real approval in the terminal.
        insert("mcp", Some(&allow));
        give_up(
            &store.db,
            "mcp",
            Some("mcp__server__tool"),
            &raw,
            true,
            &mut out,
        )
        .unwrap();
        assert!(!out.is_empty());
        assert_eq!(row("mcp"), scrubbed);

        let delivered = delivered_approvals(&store.db, "src").unwrap();
        assert_eq!(delivered["n"].len(), 1);
        assert!(delivered["n"]["late"] > 0);
    }
}
