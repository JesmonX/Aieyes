//! On-demand desktop interaction for sessions waiting for user input.
//! Reuses the verified session adapters (Codex app-server, OpenCode loopback HTTP,
//! Claude/Qoder PermissionRequest hooks, bounded transcript readers, tmux and macOS terminal)
//! independently of the Feishu chat channel. Prompts are inspected on demand and never archived.
use crate::{
    channels::{Session, claude, codex, content, control, opencode},
    models::{Source, now},
    sessions::{LiveSession, Phase, Snapshot},
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicI64, Ordering};

const CONFIG_KEY: &str = "desktop.interaction.v1";
const HEARTBEAT_KEY: &str = "interaction.heartbeat";
const RETENTION: i64 = 86_400;
const MAX_INPUT: usize = 8192;
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
    pub allow_terminal: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_popup: false,
            claude_questions: false,
            claude_approvals: false,
            allow_terminal: false,
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

/// Called by the session monitor lane. Stores metadata-only control identity; never reads prompts.
pub fn observe(store: &Store, snapshot: &Snapshot) -> Result<()> {
    let at = now();
    let cfg = config(&store.db)?;
    if cfg.enabled && at - LAST_HEARTBEAT.load(Ordering::Relaxed) >= 4 {
        touch_heartbeat(store);
    }
    if cfg.enabled
        && (cfg.claude_questions || cfg.claude_approvals)
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
    }
    tx.commit()?;
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

fn status(store: &Store) -> Result<Value> {
    let cfg = config(&store.db)?;
    let feishu = crate::channels::configuration(&store.db)?;
    Ok(json!({
        "enabled": cfg.enabled,
        "autoPopup": cfg.auto_popup,
        "claudeQuestions": cfg.claude_questions,
        "claudeApprovals": cfg.claude_approvals,
        "allowTerminal": cfg.allow_terminal || feishu.allow_terminal,
        "desktopAllowTerminal": cfg.allow_terminal,
        "feishuPaired": feishu.enabled && feishu.owner.is_some(),
        "hooks": claude::hook_summary(store),
    }))
}

fn configure(store: &Store, input: &Value) -> Result<Value> {
    let mut cfg = config(&store.db)?;
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
    if let Some(v) = input["allowTerminal"].as_bool() {
        cfg.allow_terminal = v;
    }
    save_config(&store.db, &cfg)?;
    if cfg.enabled {
        touch_heartbeat(store);
    }
    let _ = claude::sync_hooks(store);
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

fn normalize_codex_approval(request: &Value, lang: &str) -> Value {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let method = request["method"].as_str().unwrap_or("");
    let command = p["command"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            p["command"].as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        })
        .unwrap_or_else(|| {
            if method == "item/fileChange/requestApproval" {
                tr("文件修改")
            } else {
                String::new()
            }
        });
    let reason = p["reason"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if method == "item/fileChange/requestApproval" {
                tr("请求批准文件修改")
            } else {
                tr("请求批准命令执行")
            }
        });
    let allowed: Vec<&str> = p["availableDecisions"]
        .as_array()
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| vec!["accept", "decline"]);
    let mut decisions = Vec::new();
    if allowed.contains(&"accept") {
        decisions.push(json!({"id":"approve","label":tr("允许本次"),"style":"primary"}));
    }
    if allowed.contains(&"decline") {
        decisions.push(json!({"id":"decline","label":tr("拒绝"),"style":"danger"}));
    }
    if allowed.contains(&"cancel") {
        decisions.push(json!({"id":"cancel","label":tr("取消"),"style":"secondary"}));
    }
    let mut details = String::new();
    if let Some(cwd) = p["cwd"].as_str().filter(|s| !s.is_empty()) {
        details.push_str(cwd);
    }
    json!({
        "toolName": if method == "item/fileChange/requestApproval" { "FileChange" } else { "Command" },
        "reason": content::clip(&reason, 800),
        "command": content::clip(&command, 4000),
        "details": content::clip(&details, 1000),
        "decisions": decisions,
    })
}

fn normalize_opencode_approval(request: &Value, lang: &str) -> Value {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    json!({
        "toolName": "Permission",
        "reason": content::clip(p["reason"].as_str().unwrap_or(&tr("原会话请求确认")), 800),
        "command": content::clip(p["command"].as_str().unwrap_or(""), 4000),
        "details": "",
        "decisions": [
            {"id":"approve","label":tr("允许本次"),"style":"primary"},
            {"id":"decline","label":tr("拒绝"),"style":"danger"},
        ]
    })
}

fn normalize_claude_approval(req: &claude::PendingClaude, lang: &str) -> Value {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let input = &req.input;
    let command = input["command"]
        .as_str()
        .or_else(|| input["file_path"].as_str())
        .or_else(|| input["path"].as_str())
        .or_else(|| input["url"].as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    let reason = input["description"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} · {}", req.tool_name, tr("请求执行权限")));
    let details = if command.is_empty() {
        serde_json::to_string_pretty(input).unwrap_or_default()
    } else if req.tool_name == "Edit" || req.tool_name == "Write" {
        input["new_string"]
            .as_str()
            .or_else(|| input["content"].as_str())
            .map(|s| content::clip(s, 1200))
            .unwrap_or_default()
    } else {
        String::new()
    };
    json!({
        "toolName": req.tool_name,
        "reason": content::clip(&reason, 800),
        "command": content::clip(&command, 4000),
        "details": content::clip(&details, 2000),
        "decisions": [
            {"id":"approve","label":tr("允许本次"),"style":"primary"},
            {"id":"decline","label":tr("拒绝"),"style":"danger"},
        ]
    })
}

fn normalize_questions(request: &Value) -> Vec<Value> {
    let method = request["method"].as_str().unwrap_or("");
    let allow_custom_default = matches!(
        method,
        "aieyes/claudeQuestion"
            | "aieyes/asyncQuestion"
            | "item/tool/requestUserInput"
            | "tool/requestUserInput"
    );
    request["params"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, q)| {
            let id = q["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| (i + 1).to_string());
            let options = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    let label = o["label"]
                        .as_str()
                        .or_else(|| o.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let mut item = json!({"label": content::clip(&label, 300)});
                    if let Some(desc) = o["description"].as_str().filter(|s| !s.is_empty()) {
                        item["description"] = json!(content::clip(desc, 600));
                    }
                    if let Some(prev) = o["preview"].as_str().filter(|s| !s.trim().is_empty()) {
                        item["preview"] = json!(content::clip(prev, 2500));
                    }
                    if !o["value"].is_null() {
                        item["value"] = o["value"].clone();
                    }
                    item
                })
                .collect::<Vec<_>>();
            let allow_custom = if method == opencode::FORM {
                options.is_empty() || q["custom"] == true
            } else {
                allow_custom_default
            };
            json!({
                "id": id,
                "key": q["key"].as_str().unwrap_or(""),
                "question": content::clip(q["question"].as_str().unwrap_or(""), 1600),
                "kind": q["kind"].as_str().unwrap_or("string"),
                "multiSelect": q["multiSelect"] == true,
                "allowCustom": allow_custom,
                "options": options,
            })
        })
        .collect()
}

fn base_payload(target: &Session, lang: &str) -> Value {
    json!({
        "sessionId": target.session.id,
        "stateKey": target.state_key,
        "promptHash": "",
        "kind": "none",
        "adapter": "focusOnly",
        "canRespond": false,
        "canHandoff": false,
        "needsClaudeHook": false,
        "requestId": "",
        "title": target.title(lang),
        "approval": Value::Null,
        "questions": [],
        "textPrompt": "",
        "readonlyReason": "",
    })
}

fn can_terminal_respond(target: &Session, allow_terminal: bool) -> Option<&'static str> {
    if !allow_terminal {
        return None;
    }
    if now() - target.observed_at > 20 {
        return None;
    }
    if target.session.subagent_count > 0 && target.session.phase == Phase::NeedsInput {
        return None;
    }
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

fn inspect(store: &Store, params: &Value) -> Result<Value> {
    let cfg = config(&store.db)?;
    let lang = language(store);
    let tr = |t: &str| crate::localization::translate(&lang, t).into_owned();
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let Some(target) = session(&store.db, session_id)? else {
        return Ok(json!({
            "sessionId": session_id,
            "stateKey": "",
            "promptHash": "",
            "kind": "none",
            "adapter": "focusOnly",
            "canRespond": false,
            "canHandoff": false,
            "needsClaudeHook": false,
            "requestId": "",
            "title": "",
            "approval": Value::Null,
            "questions": [],
            "textPrompt": "",
            "readonlyReason": tr("当前会话已结束或不可用"),
        }));
    };
    let mut payload = base_payload(&target, &lang);
    if !cfg.enabled {
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(tr("桌宠快捷交互已关闭，可在设置中开启"));
        return Ok(payload);
    }
    if target.session.phase != Phase::NeedsInput {
        return Ok(payload);
    }
    let settings = store.settings()?;
    let source = match control::source(&settings, &target) {
        Ok(s) => s,
        Err(error) => {
            payload["kind"] = json!("readonly");
            payload["readonlyReason"] = json!(error.to_string());
            return Ok(payload);
        }
    };
    let allow_terminal = effective_allow_terminal(store, &cfg);

    // 1. Remote SSH session
    if source.host_id.is_some() {
        if let Ok(value) = control::remote(&settings, &source, &target, "read", "", false, None) {
            let excerpt: content::Excerpt =
                serde_json::from_value(value["excerpt"].clone()).unwrap_or_default();
            let prompt_hash = value["promptHash"].as_str().unwrap_or("").to_owned();
            if value["approval"] == true
                && let Some(req) = value.get("approvalRequest").filter(|v| v.is_object())
            {
                payload["kind"] = json!("approval");
                payload["adapter"] = json!("codex");
                payload["canRespond"] = json!(true);
                payload["promptHash"] = json!(prompt_hash);
                payload["approval"] = normalize_codex_approval(req, &lang);
                return Ok(payload);
            }
            if let Some(req) = value.get("questionRequest").filter(|v| v.is_object()) {
                payload["kind"] = json!("questions");
                payload["adapter"] = json!("codex");
                payload["canRespond"] = json!(true);
                payload["promptHash"] = json!(prompt_hash);
                payload["questions"] = json!(normalize_questions(req));
                return Ok(payload);
            }
            let text = value["question"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or(excerpt.question);
            payload["textPrompt"] = json!(text);
            if let Some(adapter) = can_terminal_respond(&target, allow_terminal) {
                payload["kind"] = json!("text");
                payload["adapter"] = json!(adapter);
                payload["canRespond"] = json!(true);
            } else {
                payload["kind"] = json!("readonly");
                payload["readonlyReason"] = json!(if allow_terminal {
                    tr("此远程会话暂无可直连的回传接口，请回到原终端处理")
                } else {
                    tr("此会话需要启用终端适配或在原终端处理")
                });
            }
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(tr("暂时无法读取远程会话的等待内容"));
        return Ok(payload);
    }

    // 2. Local Claude / Qoder CLI pending PermissionRequest hook (approval or AskUserQuestion)
    if claude::hooked(&source.provider)
        && let Ok(Some(req)) =
            claude::pending_request(store, &source.id, &target.session.native_session)
    {
        payload["adapter"] = json!("claudeHook");
        payload["canRespond"] = json!(true);
        payload["canHandoff"] = json!(true);
        payload["requestId"] = json!(req.id);
        if req.kind == "approval" {
            payload["kind"] = json!("approval");
            payload["promptHash"] = json!(req.question_hash);
            payload["approval"] = normalize_claude_approval(&req, &lang);
            return Ok(payload);
        }
        if let Some(question_req) =
            claude::request_from_input(&target, &json!(req.call_id), &req.input)
        {
            payload["kind"] = json!("questions");
            payload["promptHash"] = json!(codex::fingerprint(&question_req));
            payload["questions"] = json!(normalize_questions(&question_req));
            return Ok(payload);
        }
    }

    // 3. Local Codex / ChatGPT Work
    if matches!(source.provider.as_str(), "codex" | "chatgpt-work") {
        let excerpt = content::local(&source, &target);
        let waiting_on_approval = is_codex_approval(&target);
        if !waiting_on_approval && let Some(async_req) = codex::async_request(&target, &excerpt) {
            let can_connect = codex::Client::open(&expand(&source.path)).is_ok();
            payload["promptHash"] = json!(codex::fingerprint(&async_req));
            payload["questions"] = json!(normalize_questions(&async_req));
            if can_connect {
                payload["kind"] = json!("questions");
                payload["adapter"] = json!("codex");
                payload["canRespond"] = json!(true);
            } else {
                payload["kind"] = json!("readonly");
                payload["textPrompt"] = json!(excerpt.question);
                payload["readonlyReason"] =
                    json!(tr("未检测到 Codex 控制接口，请点击跳转到原窗口回答"));
            }
            return Ok(payload);
        }
        if let Ok(mut client) = codex::Client::open(&expand(&source.path))
            && client.attach(&target, true).is_ok()
            && let Ok(Some(request)) = if waiting_on_approval {
                client.pending_approval(&target)
            } else {
                client.pending(&target)
            }
        {
            let hash = codex::fingerprint(&request);
            payload["promptHash"] = json!(hash);
            payload["adapter"] = json!("codex");
            if codex::is_approval(&request) {
                payload["kind"] = json!("approval");
                payload["canRespond"] = json!(true);
                payload["approval"] = normalize_codex_approval(&request, &lang);
                return Ok(payload);
            }
            if matches!(
                request["method"].as_str(),
                Some("item/tool/requestUserInput" | "tool/requestUserInput")
            ) {
                payload["kind"] = json!("questions");
                payload["canRespond"] = json!(true);
                payload["questions"] = json!(normalize_questions(&request));
                return Ok(payload);
            }
        }
        if waiting_on_approval {
            payload["kind"] = json!("approval");
            payload["approval"] = json!({
                "toolName": "Codex",
                "reason": tr("原会话请求确认"),
                "command": "",
                "details": "",
                "decisions": [],
            });
            payload["readonlyReason"] = json!(tr("此终端没有可核验的审批接口，请在原窗口确认"));
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["textPrompt"] = json!(excerpt.question);
        payload["readonlyReason"] = json!(if cfg!(windows) {
            tr("Windows 上暂未暴露 Codex 原生控制接口，请点击跳转到原窗口处理")
        } else {
            tr("原会话暂无可直接提交的请求，请点击跳转到原窗口处理")
        });
        return Ok(payload);
    }

    // 4. Local OpenCode hosted by background service
    if source.provider == "opencode"
        && let Some(request) = opencode::request(&source, &target)
    {
        let hash = codex::fingerprint(&request);
        payload["promptHash"] = json!(hash);
        payload["adapter"] = json!("opencode");
        if request["method"] == opencode::PERMISSION {
            payload["kind"] = json!("approval");
            payload["canRespond"] = json!(true);
            payload["approval"] = normalize_opencode_approval(&request, &lang);
            return Ok(payload);
        }
        if request["method"] == opencode::FORM {
            payload["kind"] = json!("questions");
            payload["canRespond"] = json!(true);
            payload["questions"] = json!(normalize_questions(&request));
            return Ok(payload);
        }
        payload["kind"] = json!("readonly");
        payload["readonlyReason"] = json!(tr("此表单包含条件或外链字段，请回到原窗口处理"));
        return Ok(payload);
    }

    // 5. Fallback readers (Claude without active hook, Kiro, DeepSeek Harness, Antigravity, Qoder Desktop, generic CLI)
    let excerpt = control::local_excerpt(&source, &target);
    if claude::hooked(&source.provider) && excerpt.claude_request.is_some() {
        let structured = claude::request(&target, &excerpt)
            .map(|req| normalize_questions(&req))
            .unwrap_or_default();
        payload["kind"] = json!(if structured.is_empty() {
            "readonly"
        } else {
            "questions"
        });
        payload["questions"] = json!(structured);
        payload["textPrompt"] = json!(excerpt.question);
        payload["needsClaudeHook"] = json!(!(cfg.claude_questions || cfg.claude_approvals));
        payload["readonlyReason"] = json!(if cfg.claude_questions || cfg.claude_approvals {
            tr("此提问在交互 Hook 就绪前发起；请回原窗口回答，或重开会话后即可在弹窗操作")
        } else {
            tr("启用 Claude / Qoder 交互 Hook 后可直接在弹窗选择回答")
        });
        return Ok(payload);
    }

    if let Some(questions) = excerpt.structured_questions.as_ref() {
        let synthetic =
            json!({"method": "aieyes/readonlyQuestions", "params": {"questions": questions}});
        payload["kind"] = json!("questions");
        payload["canRespond"] = json!(false);
        payload["questions"] = json!(normalize_questions(&synthetic));
        payload["textPrompt"] = json!(excerpt.question);
        payload["readonlyReason"] = json!(tr("此 Agent 暂无原生回传接口，请点击跳转到原窗口处理"));
        return Ok(payload);
    }

    if let Some(approval) = excerpt.structured_approval.as_ref() {
        payload["kind"] = json!("approval");
        payload["canRespond"] = json!(false);
        payload["approval"] = json!({
            "toolName": approval["toolName"].as_str().unwrap_or("Tool"),
            "reason": approval["reason"].as_str().unwrap_or(""),
            "command": approval["command"].as_str().unwrap_or(""),
            "details": approval["details"].as_str().unwrap_or(""),
            "decisions": [],
        });
        payload["textPrompt"] = json!(excerpt.question);
        payload["readonlyReason"] = json!(tr("此 Agent 暂无原生回传接口，请点击跳转到原窗口处理"));
        return Ok(payload);
    }

    payload["textPrompt"] = json!(excerpt.question);
    let has_terminal = can_terminal_respond(&target, allow_terminal);
    // Avoid sending raw terminal paste when the agent is clearly waiting on a non-verifiable approval
    // or a desktop GUI without terminal control.
    let desktop_gui = matches!(source.provider.as_str(), "qoder" | "qoder-cn")
        && crate::qoder_desktop::database(&source.path).is_some();
    if !desktop_gui && let Some(adapter) = has_terminal {
        payload["kind"] = json!("text");
        payload["adapter"] = json!(adapter);
        payload["canRespond"] = json!(true);
        return Ok(payload);
    }
    payload["kind"] = json!("readonly");
    payload["readonlyReason"] = json!(
        if claude::hooked(&source.provider) && !cfg.claude_approvals {
            tr("可在设置中开启 Claude / Qoder 工具授权弹窗，或点击跳转到原窗口确认")
        } else if !desktop_gui && !allow_terminal && target.session.pid.is_some() {
            tr("此 Agent 无原生回传接口；可点击跳转到原窗口，或在设置中启用终端适配输入")
        } else {
            tr("此 Agent 暂无原生回传接口，请点击跳转到原窗口处理")
        }
    );
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

fn respond(store: &Store, params: &Value) -> Result<Value> {
    let cfg = config(&store.db)?;
    let lang = language(store);
    ensure!(cfg.enabled, "桌宠快捷交互已关闭");
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let expected_state = params["stateKey"].as_str().unwrap_or("");
    let expected_hash = params["promptHash"].as_str().filter(|s| !s.is_empty());
    let action = params["action"].as_str().unwrap_or("answer");
    let answers = params.get("answers").filter(|v| v.is_object());
    let text = params["text"].as_str();
    let reason = params["reason"].as_str();

    if let Some(t) = text {
        ensure!(
            t.len() <= MAX_INPUT
                && !t
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t')),
            "输入内容无效或过长"
        );
    }

    let Some(target) = session(&store.db, session_id)? else {
        return Ok(outcome_json(
            control::Outcome::blocked("原会话已结束或不可用"),
            &lang,
        ));
    };
    if !expected_state.is_empty() && target.state_key != expected_state {
        return Ok(outcome_json(
            control::Outcome::blocked("会话状态或问题已变化，请刷新后再操作"),
            &lang,
        ));
    }
    let settings = store.settings()?;
    let source = match control::source(&settings, &target) {
        Ok(s) => s,
        Err(error) => {
            return Ok(outcome_json(
                control::Outcome::blocked(&error.to_string()),
                &lang,
            ));
        }
    };
    let allow_terminal = effective_allow_terminal(store, &cfg);

    // 1. Remote SSH session
    if source.host_id.is_some() {
        let formatted = format_remote_reply(action, answers, text)?;
        let out = match control::remote(
            &settings,
            &source,
            &target,
            "send",
            &formatted,
            allow_terminal,
            expected_hash,
        ) {
            Ok(v) => serde_json::from_value::<control::Outcome>(v)
                .unwrap_or_else(|_| control::Outcome::uncertain("远程输入结果无效，请核对原会话")),
            Err(_) => control::Outcome::uncertain("远程连接中断，无法确认提交结果；请核对原会话"),
        };
        return Ok(outcome_json(out, &lang));
    }

    // 2. Local Claude / Qoder CLI pending PermissionRequest hook
    if claude::hooked(&source.provider)
        && let Ok(Some(req)) =
            claude::pending_request(store, &source.id, &target.session.native_session)
    {
        if req.kind == "approval" {
            if expected_hash != Some(req.question_hash.as_str()) {
                return Ok(outcome_json(
                    control::Outcome::blocked("审批请求已变化，请刷新后再操作"),
                    &lang,
                ));
            }
            let allow = match action {
                "approve" | "accept" => true,
                "decline" | "deny" => false,
                _ => {
                    return Ok(outcome_json(
                        control::Outcome::blocked("不支持的审批操作"),
                        &lang,
                    ));
                }
            };
            let out = claude::answer_pending_approval(store, &req, &target, allow, reason);
            return Ok(outcome_json(out, &lang));
        }
        if let Some(qreq) = claude::request_from_input(&target, &json!(req.call_id), &req.input) {
            if expected_hash != Some(codex::fingerprint(&qreq).as_str()) {
                return Ok(outcome_json(
                    control::Outcome::blocked("问题已变化，请刷新后再操作"),
                    &lang,
                ));
            }
            let out = claude::answer_pending_question(store, &req, &target, answers, text);
            return Ok(outcome_json(out, &lang));
        }
    }

    // 3. Local Codex / ChatGPT Work
    if matches!(source.provider.as_str(), "codex" | "chatgpt-work") {
        let out = respond_local_codex(&source, &target, expected_hash, action, answers, text);
        return Ok(outcome_json(out, &lang));
    }

    // 4. Local OpenCode
    if source.provider == "opencode"
        && let Some(out) =
            opencode::execute_structured(&source, &target, expected_hash, action, answers, text)
    {
        return Ok(outcome_json(out, &lang));
    }

    // 5. Local terminal fallback (tmux / macOS terminal automation)
    if matches!(action, "approve" | "decline" | "cancel") {
        return Ok(outcome_json(
            control::Outcome::blocked("此终端没有可核验的审批接口，请在原窗口确认"),
            &lang,
        ));
    }
    let body = text.unwrap_or("").trim();
    if body.is_empty() {
        return Ok(outcome_json(
            control::Outcome::blocked("输入内容不能为空"),
            &lang,
        ));
    }
    if !allow_terminal {
        return Ok(outcome_json(
            control::Outcome::blocked("此会话需要启用终端自动化，或回到原窗口输入"),
            &lang,
        ));
    }
    if now() - target.observed_at > 20 {
        return Ok(outcome_json(
            control::Outcome::blocked("缺少原终端的最新状态，请刷新后重试"),
            &lang,
        ));
    }
    if target.session.subagent_count > 0 && target.session.phase == Phase::NeedsInput {
        return Ok(outcome_json(
            control::Outcome::blocked("等待输入可能来自子 Agent，请回到原窗口确认目标"),
            &lang,
        ));
    }
    if let Some(out) = control::tmux(&target, body) {
        return Ok(outcome_json(out, &lang));
    }
    if cfg!(target_os = "macos")
        && target.session.pid.is_some()
        && target.session.process_start.is_some_and(|n| n > 0)
    {
        return Ok(json!({
            "status": "needsLocalTerminal",
            "message": crate::localization::translate(&lang, "正在向原终端发送输入…"),
            "receipt": {"adapter": "macos"},
            "terminalJob": {
                "text": body,
                "session": target.session,
            }
        }));
    }
    Ok(outcome_json(
        control::Outcome::blocked("未找到可连接的原会话接口或 tmux 分屏，请回到原窗口处理"),
        &lang,
    ))
}

fn respond_local_codex(
    source: &Source,
    target: &Session,
    expected_hash: Option<&str>,
    action: &str,
    answers: Option<&Value>,
    text: Option<&str>,
) -> control::Outcome {
    let root = expand(&source.path);
    let Ok(mut client) = codex::Client::open(&root) else {
        return control::Outcome::blocked("无法连接原会话的提问接口，请回到原窗口处理");
    };
    let waiting_on_approval = is_codex_approval(target);
    if target.session.phase == Phase::NeedsInput
        && !waiting_on_approval
        && let Some(request) = codex::async_request(target, &content::local(source, target))
    {
        if expected_hash != Some(codex::fingerprint(&request).as_str()) {
            return control::Outcome::blocked("问题已改变或尚未同步，请刷新后再操作");
        }
        return client.answer_async_structured(target, &request, answers, text);
    }
    let thread = match client.attach(target, target.session.phase == Phase::NeedsInput) {
        Ok(thread) => thread,
        Err(error) => return control::Outcome::blocked(&error.to_string()),
    };
    if target.session.phase == Phase::NeedsInput {
        let pending = if waiting_on_approval {
            client.pending_approval(target)
        } else {
            client.pending(target)
        };
        let request = match pending {
            Ok(Some(r)) => r,
            _ => return control::Outcome::blocked("原会话没有可回答的请求，请核对原窗口"),
        };
        if expected_hash != Some(codex::fingerprint(&request).as_str()) {
            return control::Outcome::blocked("问题已改变或尚未同步，请刷新后再操作");
        }
        return client.answer_structured(target, &request, action, answers, text);
    }
    if thread["status"]["type"] == "idle" {
        if matches!(action, "approve" | "decline" | "cancel") {
            return control::Outcome::blocked("当前没有审批请求");
        }
        let body = text.unwrap_or("").trim();
        if body.is_empty() {
            return control::Outcome::blocked("输入内容不能为空");
        }
        return client.followup(target, body);
    }
    control::Outcome::blocked("原会话状态已改变，请核对原窗口")
}

fn format_remote_reply(
    action: &str,
    answers: Option<&Value>,
    text: Option<&str>,
) -> Result<String> {
    match action {
        "approve" | "accept" => Ok("/approve".into()),
        "decline" | "deny" => Ok("/decline".into()),
        "cancel" => Ok("/cancel-request".into()),
        _ => {
            if let Some(map) = answers.and_then(Value::as_object) {
                ensure!(!map.is_empty(), "问题回答不能为空");
                if map.len() == 1 {
                    let val = map.values().next().unwrap();
                    return Ok(stringify_answer(val));
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
            let body = text.unwrap_or("").trim();
            ensure!(!body.is_empty(), "输入内容不能为空");
            Ok(body.to_owned())
        }
    }
}

fn stringify_answer(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        s.trim().to_owned()
    } else if let Some(arr) = v.as_array() {
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        String::new()
    }
}

fn handoff(store: &Store, params: &Value) -> Result<Value> {
    let lang = language(store);
    let tr = |t: &str| crate::localization::translate(&lang, t).into_owned();
    let session_id = params["sessionId"].as_str().context("缺少会话标识")?;
    let request_id = params["requestId"].as_str().unwrap_or("");
    let target = session(&store.db, session_id)?;
    let req_id = if !request_id.is_empty() {
        Some(request_id.to_owned())
    } else if let Some(target) = target {
        claude::pending_request(
            store,
            &target.session.source_id,
            &target.session.native_session,
        )?
        .map(|r| r.id)
    } else {
        None
    };
    let Some(req_id) = req_id else {
        return Ok(json!({
            "handedOff": false,
            "message": tr("当前没有可交还终端的挂起请求"),
        }));
    };
    let handed = claude::handoff_request(store, &req_id)?;
    Ok(json!({
        "handedOff": handed,
        "message": if handed {
            tr("已交还给原终端确认")
        } else {
            tr("原请求已结束或已被处理")
        }
    }))
}

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
            for _ in 0..8 {
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
}
