use super::{Command, Config, Session, codex, configuration, content, db, random_id};
use crate::{
    models::{Settings, Source, now},
    process,
    sessions::{self, Phase},
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

const REMOTE: &str = include_str!("../../../../scripts/session_channel.py");

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Outcome {
    pub status: String,
    pub reason: String,
    pub receipt: Value,
}
impl Outcome {
    pub fn blocked(reason: &str) -> Self {
        Self {
            status: "blocked".into(),
            reason: reason.into(),
            receipt: json!({}),
        }
    }
    pub fn uncertain(reason: &str) -> Self {
        Self {
            status: "uncertain".into(),
            reason: reason.into(),
            receipt: json!({}),
        }
    }
    pub fn submitted(reason: &str, receipt: Value) -> Self {
        Self {
            status: "submitted".into(),
            reason: reason.into(),
            receipt,
        }
    }
    pub fn confirmed(reason: &str, receipt: Value) -> Self {
        Self {
            status: "confirmed".into(),
            reason: reason.into(),
            receipt,
        }
    }
}
pub(super) fn status_label(status: &str) -> &str {
    match status {
        "queued" => "等待提交",
        "blocked" => "待处理",
        "automation" => "等待桌面操作",
        "dispatching" => "正在提交",
        "submitted" => "已提交，待确认",
        "confirmed" => "已确认",
        "uncertain" => "结果待核对",
        "cancelled" => "已取消",
        _ => "未知状态",
    }
}
pub(super) fn capability(target: &Session, config: &Config) -> &'static str {
    if matches!(target.session.provider.as_str(), "codex" | "chatgpt-work") {
        "原生会话接口"
    } else if config.allow_terminal && target.process.is_some() {
        "按终端能力接入"
    } else {
        "通知与待处理指令"
    }
}

pub(super) fn source(settings: &Settings, target: &Session) -> Result<Source> {
    let mut source = settings
        .sources
        .iter()
        .find(|s| {
            s.id == target.session.source_id
                && (s.provider == target.session.provider
                    || s.provider == "codex" && target.session.provider == "chatgpt-work")
                && s.host_id.as_deref().unwrap_or("local") == target.session.machine_id
                && settings.source_enabled(s)
        })
        .cloned()
        .context("会话来源已暂停或移除，指令已保留")?;
    if source.path.is_empty() {
        source.path = crate::contracts::provider(&source.provider)
            .map_or("", |p| p.default_path)
            .into();
    }
    Ok(source)
}

fn remote(
    settings: &Settings,
    source: &Source,
    target: &Session,
    action: &str,
    text: &str,
    allow_terminal: bool,
    prompt: Option<&str>,
) -> Result<Value> {
    let host = settings
        .hosts
        .iter()
        .find(|h| Some(&h.id) == source.host_id.as_ref() && h.enabled)
        .context("目标主机已暂停或移除")?;
    let loader = "import json,sys;p=json.load(sys.stdin);exec(compile(p['script'],'<aieyes-channel>','exec'),{'__name__':'__main__','REQUEST':p['request']})";
    let command =
        crate::ssh::command(host, &format!("exec python3 -c {}", process::quote(loader)))?;
    let input = json!({"script":REMOTE,"request":{"action":action,"source":source,"target":target,"text":text,"allowTerminal":allow_terminal,"promptHash":prompt,"language":settings.appearance.language}});
    let bytes = process::run(
        command,
        serde_json::to_vec(&input)?,
        Duration::from_secs(20),
    )?;
    let value = bytes
        .split(|b| *b == b'\n')
        .rev()
        .find(|l| !l.is_empty())
        .context("远程会话控制无响应")?;
    Ok(serde_json::from_slice(value)?)
}

fn saved_prompt(store: &Store, target: &Session) -> Result<Option<String>> {
    let value: Option<String> = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("channel.prompt:{}", target.session.id)],
            |r| r.get(0),
        )
        .optional()?;
    let value: Value = value
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(Value::Null);
    Ok((value["expected"].as_str() == Some(&target.state_key))
        .then(|| value["hash"].as_str().map(str::to_owned))
        .flatten())
}

fn local_codex(store: &Store, source: &Source, target: &Session, text: &str) -> Outcome {
    let root = expand(&source.path);
    if let Ok(mut client) = codex::Client::open(&root) {
        let thread = match client.attach(target, target.session.phase == Phase::NeedsInput) {
            Ok(thread) => thread,
            Err(e) => return Outcome::blocked(&e.to_string()),
        };
        if target.session.phase == Phase::NeedsInput {
            let request = match client.pending(target) {
                Ok(Some(r)) => r,
                _ => return Outcome::blocked("原会话没有可回答的请求，请核对原窗口"),
            };
            match saved_prompt(store, target) {
                Ok(Some(hash)) if hash == codex::fingerprint(&request) => {
                    return client.answer(target, &request, text);
                }
                _ => return Outcome::blocked("问题已改变或尚未同步，请回复最新的问题通知"),
            }
        }
        if thread["status"]["type"] == "idle" {
            if matches!(text.trim(), "/approve" | "/decline" | "/cancel-request") {
                return Outcome::blocked("当前没有审批请求，请回复具体问题通知");
            }
            return client.followup(target, text);
        }
        if thread["status"]["activeFlags"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            return Outcome::blocked("原会话正在等待回答，请回复最新的问题通知");
        }
    }
    if target.session.phase == Phase::NeedsInput {
        return Outcome::blocked("无法连接原会话的提问接口，回答已保留");
    }
    if !codex::valid_id(&target.session.native_session) {
        return Outcome::blocked("原生会话标识无效");
    }
    if matches!(text.trim(), "/approve" | "/decline" | "/cancel-request") {
        return Outcome::blocked("当前没有可确认的审批请求");
    }
    let mut probe = process::cli_command(&source.codex_binary);
    probe.env("CODEX_HOME", &root).args(["queue", "--help"]);
    let help = process::run(probe, vec![], Duration::from_secs(4));
    if !help.is_ok_and(|v| String::from_utf8_lossy(&v).contains("--thread")) {
        return Outcome::blocked("此 Codex 版本没有既有会话排队接口");
    }
    if content::verify_codex_record(source, target).is_err() {
        return Outcome::blocked("原会话记录已改变或无法核对，指令已保留");
    }
    let mut command = process::cli_command(&source.codex_binary);
    command.env("CODEX_HOME", &root).args([
        "queue",
        "--thread",
        &target.session.native_session,
        "--message",
        text,
        "--resume",
    ]);
    match process::run(command, vec![], Duration::from_secs(12)) {
        Ok(_) => Outcome::submitted(
            "已排队，等待原会话恢复；排队成功不代表已开始执行",
            json!({"adapter":"codexQueue"}),
        ),
        Err(_) => Outcome::uncertain("排队结果无法确认，请核对原会话；系统不会重发"),
    }
}

fn tmux(target: &Session, text: &str) -> Option<Outcome> {
    let pid = target.session.pid?;
    let start = target.session.process_start.filter(|n| *n > 0)?;
    let identity = sessions::focus_target(pid, start);
    if identity["status"] != "found" {
        return Some(Outcome::blocked("原会话进程已退出或身份已改变"));
    }
    let tty = identity["tty"].as_str()?;
    let mut command = process::cli_command("tmux");
    command.args([
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{pane_tty}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}",
    ]);
    let output = process::run(command, vec![], Duration::from_secs(3)).ok()?;
    let value = String::from_utf8_lossy(&output);
    let hits = value
        .lines()
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|p| p.len() == 5 && p[1] == tty)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return None;
    }
    if hits.len() != 1 {
        return Some(Outcome::blocked("无法唯一定位 tmux 分屏"));
    }
    let hit = &hits[0];
    let pane_pid = hit[2].parse::<u64>().ok();
    let ancestor = identity["ancestors"]
        .as_array()
        .is_some_and(|a| a.iter().any(|p| p["pid"].as_u64() == pane_pid));
    if !ancestor || hit[4] != "0" || !agent_foreground(&target.session.provider, hit[3]) {
        return Some(Outcome::blocked(
            "目标分屏的前台进程不是原 Agent，指令已保留",
        ));
    }
    if !target.session.phase.terminal() && target.session.phase != Phase::NeedsInput {
        return Some(Outcome::blocked("原终端仍在运行，请等待本轮结束后重试"));
    }
    let buffer = format!("aieyes-{}", random_id().ok()?);
    let mut load = process::cli_command("tmux");
    load.args(["load-buffer", "-b", &buffer, "-"]);
    if process::run(load, text.as_bytes().to_vec(), Duration::from_secs(3)).is_err() {
        return Some(Outcome::blocked("无法准备 tmux 输入"));
    }
    let outcome = if !sessions::process_matches(pid, start) {
        Outcome::blocked("原会话进程已退出")
    } else {
        let mut paste = process::cli_command("tmux");
        // Targeted literal paste; the message is never shell code or part of the tmux format.
        paste.args([
            "paste-buffer",
            "-p",
            "-d",
            "-b",
            &buffer,
            "-t",
            hit[0],
            ";",
            "send-keys",
            "-t",
            hit[0],
            "Enter",
        ]);
        match process::run(paste, vec![], Duration::from_secs(3)) {
            Ok(_) => Outcome::submitted(
                "已输入原 tmux 分屏，等待会话确认",
                json!({"adapter":"tmux"}),
            ),
            Err(_) => Outcome::uncertain("终端输入结果无法确认，请核对原分屏；系统不会重发"),
        }
    };
    let mut cleanup = process::cli_command("tmux");
    cleanup.args(["delete-buffer", "-b", &buffer]);
    let _ = process::run(cleanup, vec![], Duration::from_secs(1));
    Some(outcome)
}

pub(super) fn agent_foreground(provider: &str, name: &str) -> bool {
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    match provider {
        "claude" => matches!(name.as_str(), "claude"),
        "kimi" => matches!(name.as_str(), "kimi"),
        "pi" => matches!(name.as_str(), "pi"),
        "omp" => matches!(name.as_str(), "omp"),
        "opencode" => name == "opencode",
        "qoder" | "qoder-cn" => matches!(name.as_str(), "qoder" | "qodercli"),
        "codebuddy" => matches!(name.as_str(), "codebuddy" | "c"),
        "antigravity" => matches!(name.as_str(), "agy" | "antigravity"),
        "deepseek" | "deepseek-harness" => matches!(name.as_str(), "dsh" | "deepseek"),
        "zcode" => name == "zcode",
        "copilot" => name == "copilot",
        "gemini" => name == "gemini",
        "kilo" => name == "kilo",
        "cline" => name == "cline",
        "kiro" => name == "kiro-cli",
        _ => false,
    }
}

fn execute(store: &Store, c: &Config, target: &Session, text: &str) -> Outcome {
    let settings = match store.settings() {
        Ok(s) => s,
        Err(_) => return Outcome::blocked("无法读取会话来源"),
    };
    let source = match source(&settings, target) {
        Ok(s) => s,
        Err(e) => return Outcome::blocked(&e.to_string()),
    };
    if source.host_id.is_some() {
        return match remote(
            &settings,
            &source,
            target,
            "send",
            text,
            c.allow_terminal,
            saved_prompt(store, target).ok().flatten().as_deref(),
        ) {
            Ok(v) => serde_json::from_value(v)
                .unwrap_or_else(|_| Outcome::uncertain("远程输入结果无效，请核对原会话")),
            Err(_) => Outcome::uncertain("远程连接中断，无法确认指令是否提交；请核对原会话"),
        };
    }
    if matches!(source.provider.as_str(), "codex" | "chatgpt-work") {
        return local_codex(store, &source, target, text);
    }
    if !c.allow_terminal {
        return Outcome::blocked("此会话需要启用终端自动化，指令已保留");
    }
    if now() - target.observed_at > 20 {
        return Outcome::blocked("缺少原终端的最新状态，指令已保留");
    }
    if target.session.subagent_count > 0 && target.session.phase == Phase::NeedsInput {
        return Outcome::blocked("等待输入可能来自子 Agent，请回到原窗口确认目标");
    }
    if matches!(text.trim(), "/approve" | "/decline" | "/cancel-request") {
        return Outcome::blocked("此终端没有可核验的审批接口，请在原窗口确认");
    }
    if let Some(outcome) = tmux(target, text) {
        return outcome;
    }
    if cfg!(target_os = "macos")
        && target.session.pid.is_some()
        && target.session.process_start.is_some_and(|n| n > 0)
    {
        return Outcome {
            status: "automation".into(),
            reason: "等待已授权的原生终端操作".into(),
            receipt: json!({}),
        };
    }
    Outcome::blocked("未找到可连接的原会话接口或 tmux 分屏，指令已保留")
}

fn record(store: &Store, c: &Config, command: &Command, outcome: &Outcome) -> Result<()> {
    let tx = store.db.unchecked_transaction()?;
    let changed = tx.execute("UPDATE channel_commands SET status=?1,reason=?2,receipt=?3 WHERE epoch=?4 AND id=?5 AND status IN ('dispatching','submitted')", params![outcome.status,outcome.reason,outcome.receipt.to_string(),c.epoch,command.id])?;
    if changed == 1 && configuration(&tx)?.epoch == c.epoch {
        let language = db::language(&tx);
        db::outbox_raw(
            &tx,
            c,
            &command.reply_to,
            Some((&command.session_id, &command.expected)),
            &format!(
                "{} · {}\n{}",
                command.id,
                crate::localization::translate(&language, status_label(&outcome.status)),
                crate::localization::translate(&language, &outcome.reason)
            ),
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn process_command(store: &Store) -> Result<()> {
    let _lock = db::config_lock(store)?;
    let c = configuration(&store.db)?;
    if !c.enabled || c.owner.is_none() {
        return Ok(());
    }
    let Some(command) = db::pending_commands(&store.db, &c.epoch, "queued")?
        .into_iter()
        .next()
    else {
        return Ok(());
    };
    let target = db::session(&store.db, &c.epoch, &command.session_id)?;
    let changed = store.db.execute("UPDATE channel_commands SET status='dispatching',attempted_at=?1 WHERE epoch=?2 AND id=?3 AND status='queued'",params![now(),c.epoch,command.id])?;
    if changed != 1 {
        return Ok(());
    }
    let outcome = if let Some(target) = target {
        if target.state_key != command.expected {
            Outcome::blocked("会话或问题已改变，请回复最新通知或发送新指令")
        } else if configuration(&store.db)?.epoch != c.epoch {
            Outcome::blocked("渠道配置已改变")
        } else {
            execute(store, &c, &target, &command.body)
        }
    } else {
        Outcome::blocked("原会话已不可用，请重新选择会话")
    };
    record(store, &c, &command, &outcome)
}

pub(super) fn process_event(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled || c.owner.is_none() {
        return Ok(());
    }
    let raw: Option<(String, String)> = store
        .db
        .query_row(
            "SELECT id,payload FROM channel_events WHERE epoch=?1 ORDER BY at,rowid LIMIT 1",
            [&c.epoch],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, raw)) = raw else {
        return Ok(());
    };
    let target: Session = serde_json::from_str(&raw)?;
    let settings = store.settings()?;
    let tr = |text: &str| {
        crate::localization::translate(&settings.appearance.language, text).into_owned()
    };
    let source = source(&settings, &target);
    let mut excerpt = content::Excerpt::default();
    let mut question = None;
    let mut prompt_hash = None;
    let mut approval = false;
    if let Ok(source) = source {
        if source.host_id.is_some() {
            if let Ok(value) = remote(&settings, &source, &target, "read", "", false, None) {
                excerpt = serde_json::from_value(value["excerpt"].clone()).unwrap_or_default();
                question = value["question"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                prompt_hash = value["promptHash"].as_str().map(str::to_owned);
                approval = value["approval"] == true;
            }
        } else {
            excerpt = content::local(&source, &target);
            if target.session.phase == Phase::NeedsInput
                && matches!(source.provider.as_str(), "codex" | "chatgpt-work")
                && let Ok(mut client) = codex::Client::open(&expand(&source.path))
                && client.attach(&target, true).is_ok()
                && let Ok(Some(request)) = client.pending(&target)
            {
                question = Some(codex::describe(&request, &settings.appearance.language));
                prompt_hash = Some(codex::fingerprint(&request));
                approval = matches!(
                    request["method"].as_str(),
                    Some(
                        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
                    )
                );
            }
        }
    }
    let current = db::session(&store.db, &c.epoch, &target.session.id)?;
    let stale_question = target.session.phase == Phase::NeedsInput
        && current
            .as_ref()
            .is_none_or(|s| s.state_key != target.state_key);
    let tx = store.db.unchecked_transaction()?;
    if !stale_question && configuration(&tx)?.epoch == c.epoch {
        let body = if target.session.phase == Phase::NeedsInput {
            question
                .filter(|s| !s.is_empty())
                .or_else(|| (!excerpt.question.is_empty()).then_some(excerpt.question))
                .unwrap_or_else(|| tr("原会话等待输入，具体问题暂不可读取。"))
        } else if target.session.phase == Phase::Complete {
            if excerpt.text.is_empty() {
                tr("本轮已完成；此 Agent 未提供可读取的最终回复。")
            } else {
                excerpt.text
            }
        } else {
            tr("可以回复此消息继续原会话或下达下一步指令。")
        };
        let footer = if approval {
            tr("请引用最新的审批通知，再回复 /approve 或 /decline。")
        } else {
            tr(&format!(
                "引用此消息回复，或使用 /send {} 指令。",
                target.key
            ))
        };
        db::outbox_raw(
            &tx,
            &c,
            "",
            Some((&target.session.id, &target.state_key)),
            &format!(
                "{}\n{}\n\n{body}\n\n{footer}",
                target.title(&settings.appearance.language),
                tr(target.session.phase.label())
            ),
        )?;
        if let Some(hash) = prompt_hash {
            let outgoing: String = tx.query_row(
                "SELECT id FROM channel_outbox WHERE rowid=last_insert_rowid()",
                [],
                |r| r.get(0),
            )?;
            tx.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("channel.prompt:{}",target.session.id),json!({"expected":target.state_key,"hash":hash,"approval":approval,"outboxId":outgoing}).to_string()])?;
        }
    }
    tx.execute("DELETE FROM channel_events WHERE id=?1", [id])?;
    tx.commit()?;
    Ok(())
}

pub(super) fn confirm(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled {
        return Ok(());
    }
    let settings = store.settings()?;
    for command in db::pending_commands(&store.db, &c.epoch, "dispatching")? {
        if now() - command.attempted_at.unwrap_or(command.at) > 60 {
            record(
                store,
                &c,
                &command,
                &Outcome::uncertain("提交确认已超时，请核对原会话；系统不会重发"),
            )?;
        }
    }
    for command in db::pending_commands(&store.db, &c.epoch, "submitted")?
        .into_iter()
        .take(5)
    {
        let target = db::session(&store.db, &c.epoch, &command.session_id)?;
        let excerpt = if let Some(target) = target
            && let Ok(source) = source(&settings, &target)
        {
            if source.host_id.is_some() {
                remote(&settings, &source, &target, "read", "", false, None)
                    .ok()
                    .and_then(|v| serde_json::from_value(v["excerpt"].clone()).ok())
                    .unwrap_or_default()
            } else {
                content::local(&source, &target)
            }
        } else {
            // An unavailable target still has to leave the submitted state on timeout.
            content::Excerpt::default()
        };
        let at = command.attempted_at.unwrap_or(command.at);
        if !excerpt.user_text.is_empty()
            && excerpt.user_text.trim() == command.body.trim()
            && excerpt.user_at >= at * 1000
        {
            record(
                store,
                &c,
                &command,
                &Outcome::confirmed("已在原会话记录中确认收到指令", command.receipt.clone()),
            )?;
        } else if now() - at > 60 {
            record(
                store,
                &c,
                &command,
                &Outcome::uncertain("尚未从原会话确认执行结果，请核对原窗口；系统不会重发"),
            )?;
        }
    }
    Ok(())
}

pub(super) fn recover(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    for command in db::pending_commands(&store.db, &c.epoch, "dispatching")? {
        record(
            store,
            &c,
            &command,
            &Outcome::uncertain("上次提交期间应用退出，请核对原会话；系统不会重发"),
        )?;
    }
    Ok(())
}

pub(super) fn take_automation(store: &Store) -> Result<Value> {
    let _lock = db::config_lock(store)?;
    let c = configuration(&store.db)?;
    if !cfg!(target_os = "macos") || !c.enabled || !c.allow_terminal || c.owner.is_none() {
        return Ok(json!({"job":null}));
    }
    let Some(command) = db::pending_commands(&store.db, &c.epoch, "automation")?
        .into_iter()
        .next()
    else {
        return Ok(json!({"job":null}));
    };
    let Some(target) = db::session(&store.db, &c.epoch, &command.session_id)? else {
        store.db.execute("UPDATE channel_commands SET status='blocked',reason='原会话已不可用' WHERE id=?1 AND status='automation'",[command.id])?;
        return Ok(json!({"job":null}));
    };
    let lease = random_id()?;
    let receipt = json!({"adapter":"macos","lease":lease});
    let tx = store.db.unchecked_transaction()?;
    if target.state_key != command.expected
        || now() - target.observed_at > 20
        || source(&store.settings()?, &target).is_err()
    {
        tx.execute("UPDATE channel_commands SET status='blocked',reason='原终端状态已改变' WHERE id=?1 AND status='automation'",[command.id])?;
        tx.commit()?;
        return Ok(json!({"job":null}));
    }
    let count = tx.execute("UPDATE channel_commands SET status='dispatching',attempted_at=?1,receipt=?2 WHERE epoch=?3 AND id=?4 AND status='automation'",params![now(),receipt.to_string(),c.epoch,command.id])?;
    tx.commit()?;
    Ok(if count == 1 {
        json!({"job":{"id":command.id,"lease":lease,"text":command.body,"session":target.session}})
    } else {
        json!({"job":null})
    })
}

pub(super) fn finish_automation(store: &Store, input: &Value) -> Result<Value> {
    let c = configuration(&store.db)?;
    let command = db::command_rows(&store.db, &c.epoch, Some("dispatching"))?
        .into_iter()
        .find(|r| Some(r.id.as_str()) == input["id"].as_str())
        .context("桌面指令已过期")?;
    ensure!(
        command.receipt["lease"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
            && command.receipt["lease"] == input["lease"],
        "桌面指令租约无效"
    );
    let state = input["status"].as_str().unwrap_or("uncertain");
    ensure!(
        matches!(state, "blocked" | "submitted" | "uncertain"),
        "桌面提交状态无效"
    );
    let reason = content::clip(input["reason"].as_str().unwrap_or("请核对原会话"), 500);
    record(
        store,
        &c,
        &command,
        &Outcome {
            status: state.into(),
            reason,
            receipt: json!({"adapter":"macos"}),
        },
    )?;
    Ok(json!({"saved":true}))
}
