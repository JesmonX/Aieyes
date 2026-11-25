//! AskUserQuestion and opt-in tool approval replies use Claude's PermissionRequest hook contract.
//! Qoder CLI ships the same tool, transcript and hook contract, so it shares this path.
//! The observer hooks remain metadata-only; this separate hook is opt-in (Feishu or Desktop Interaction).
use super::{Session, codex, configuration, content, control::Outcome};
use crate::{
    models::{Source, now},
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const MARKER: &str = "--aieyes-claude-question";
const PREVIEW_BUDGET: usize = 8000;
pub(crate) const PASSTHROUGH: &str = "__aieyes_passthrough__";
const DESKTOP_QUESTION_TTL: i64 = 45;
const DESKTOP_APPROVAL_TTL: i64 = 15;

#[derive(Debug, Clone)]
pub(crate) struct PendingClaude {
    pub id: String,
    pub kind: String,
    pub tool_name: String,
    pub call_id: String,
    pub question_hash: String,
    pub input: Value,
    pub pid: u32,
    pub process_start: u64,
}

/// Agents whose CLI answers `AskUserQuestion` / tool permissions through a `PermissionRequest` hook.
pub(crate) fn hooked(provider: &str) -> bool {
    matches!(provider, "claude" | "qoder" | "qoder-cn")
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
        kind TEXT NOT NULL DEFAULT 'question', tool_name TEXT NOT NULL DEFAULT 'AskUserQuestion');
        CREATE INDEX IF NOT EXISTS channel_claude_pending ON channel_claude_requests(epoch,source_id,native,question_hash);")?;
    let mut stmt = db.prepare("PRAGMA table_info(channel_claude_requests)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    if !cols.iter().any(|c| c == "kind") {
        db.execute(
            "ALTER TABLE channel_claude_requests ADD COLUMN kind TEXT NOT NULL DEFAULT 'question'",
            [],
        )?;
    }
    if !cols.iter().any(|c| c == "tool_name") {
        db.execute(
            "ALTER TABLE channel_claude_requests ADD COLUMN tool_name TEXT NOT NULL DEFAULT 'AskUserQuestion'",
            [],
        )?;
    }
    db.execute(
        "CREATE INDEX IF NOT EXISTS channel_claude_active ON channel_claude_requests(source_id,native,kind,consumed)",
        [],
    )?;
    Ok(())
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
            json!({"id":(i+1).to_string(),"question":q["question"],"options":options,"multiSelect":q["multiSelect"]==true})
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

pub(crate) fn approval_decision_payload(allow: bool, reason: Option<&str>) -> Value {
    if allow {
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow"
                }
            }
        })
    } else {
        let msg = reason
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| content::clip(s, 500));
        let mut decision = json!({"behavior": "deny"});
        if let Some(message) = msg {
            decision["message"] = json!(message);
        }
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": decision
            }
        })
    }
}

pub(crate) fn pending_request(
    store: &Store,
    source_id: &str,
    native: &str,
) -> Result<Option<PendingClaude>> {
    let row = store
        .db
        .query_row(
            "SELECT id,kind,tool_name,call_id,question_hash,input,pid,process_start \
             FROM channel_claude_requests \
             WHERE source_id=?1 AND native=?2 AND consumed=0 AND response IS NULL AND heartbeat>?3 \
             ORDER BY at DESC LIMIT 1",
            params![source_id, native, now() - 5],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get::<_, String>(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()?;
    let Some((id, kind, tool_name, call_id, question_hash, raw, pid, process_start)) = row else {
        return Ok(None);
    };
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    Ok(Some(PendingClaude {
        id,
        kind,
        tool_name,
        call_id,
        question_hash,
        input,
        pid,
        process_start,
    }))
}

pub(crate) fn answer_pending_question(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    answers_map: Option<&Value>,
    text: Option<&str>,
) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        ensure!(
            target.process
                == Some(crate::sessions::ControlProcess {
                    pid: req.pid,
                    start: req.process_start,
                })
                && crate::sessions::process_matches(req.pid, req.process_start),
            "原会话进程已退出或身份已改变"
        );
        let response = updated_input_structured(&req.input, answers_map, text)?;
        let changed = store.db.execute(
            "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",
            params![response.to_string(), req.id, now() - 5],
        )?;
        ensure!(changed == 1, "原会话的问题已结束或已被其他端处理");
        let confirmed = wait_consumed(store, &req.id, Duration::from_millis(1200));
        if confirmed {
            Ok(Outcome::confirmed(
                "原会话已接收回答",
                json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id}),
            ))
        } else {
            Ok(Outcome::submitted(
                "回答已提交，等待原会话确认",
                json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id}),
            ))
        }
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

pub(crate) fn answer_pending_approval(
    store: &Store,
    req: &PendingClaude,
    target: &Session,
    allow: bool,
    reason: Option<&str>,
) -> Outcome {
    let attempt = (|| -> Result<Outcome> {
        ensure!(
            target.process
                == Some(crate::sessions::ControlProcess {
                    pid: req.pid,
                    start: req.process_start,
                })
                && crate::sessions::process_matches(req.pid, req.process_start),
            "原会话进程已退出或身份已改变"
        );
        let response = approval_decision_payload(allow, reason);
        let changed = store.db.execute(
            "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",
            params![response.to_string(), req.id, now() - 5],
        )?;
        ensure!(changed == 1, "原会话的审批请求已结束或已失效");
        let confirmed = wait_consumed(store, &req.id, Duration::from_millis(1200));
        let msg = if allow {
            if confirmed {
                "已允许本次操作"
            } else {
                "已提交允许，等待原会话确认"
            }
        } else if confirmed {
            "已拒绝本次操作"
        } else {
            "已提交拒绝，等待原会话确认"
        };
        if confirmed {
            Ok(Outcome::confirmed(
                msg,
                json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id,"decision":if allow{"allow"}else{"deny"}}),
            ))
        } else {
            Ok(Outcome::submitted(
                msg,
                json!({"adapter":"claudeHook","requestId":req.id,"callId":req.call_id,"decision":if allow{"allow"}else{"deny"}}),
            ))
        }
    })();
    attempt.unwrap_or_else(|e| Outcome::blocked(&e.to_string()))
}

pub(crate) fn handoff_request(store: &Store, req_id: &str) -> Result<bool> {
    let changed = store.db.execute(
        "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL AND consumed=0 AND heartbeat>?3",
        params![PASSTHROUGH, req_id, now() - 5],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    Ok(wait_consumed(store, req_id, Duration::from_millis(1200)) || true)
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

/// Reconcile only the owned interaction hook. Preserve all unrelated hooks and permissions.
pub(crate) fn sync_hooks(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    let dc = crate::session_interaction::config(&store.db)?;
    let feishu_wants = c.enabled && c.owner.is_some();
    let desktop_wants_q = dc.enabled && (dc.claude_questions || dc.claude_approvals);
    let desktop_wants_a = dc.enabled && dc.claude_approvals;
    let want = feishu_wants || desktop_wants_q;
    let matcher = if desktop_wants_a {
        "*"
    } else {
        "AskUserQuestion"
    };

    let settings = store.settings()?;
    let mut seen = std::collections::HashSet::new();
    for source in settings
        .sources
        .iter()
        .filter(|s| hooked(&s.provider) && s.host_id.is_none())
    {
        if !settings.source_enabled(source) {
            continue;
        }
        let Some(home) = home(source) else {
            continue;
        };
        if !seen.insert(home.clone()) {
            continue;
        }
        let path = home.join("settings.json");
        if !home.is_dir() || fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        if !want {
            // Only touch settings.json if we previously installed an owned entry.
            let Ok(original) = fs::read(&path) else {
                continue;
            };
            if !String::from_utf8_lossy(&original).contains(MARKER) {
                continue;
            }
            let folder = home.join(".aieyes");
            fs::create_dir_all(&folder)?;
            let lock = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(folder.join("session-hooks.lock"))?;
            let _lock = crate::file_lock::FileLock::try_exclusive(lock)?;
            let original = fs::read(&path)?;
            let Ok(mut value) = serde_json::from_slice::<Value>(&original) else {
                continue;
            };
            if let Some(rows) = value["hooks"]["PermissionRequest"].as_array_mut() {
                for r in rows.iter_mut() {
                    if let Some(hs) = r["hooks"].as_array_mut() {
                        hs.retain(|h| !h["command"].as_str().is_some_and(|s| s.contains(MARKER)));
                    }
                }
                rows.retain(|r| r["hooks"].as_array().is_none_or(|h| !h.is_empty()));
                let tmp = folder.join("channel-settings.pending");
                private_write(&tmp, &serde_json::to_vec_pretty(&value)?)?;
                fs::rename(tmp, path)?;
            }
            continue;
        }
        let folder = home.join(".aieyes");
        fs::create_dir_all(&folder)?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(folder.join("session-hooks.lock"))?;
        let _lock = crate::file_lock::FileLock::try_exclusive(lock)?;
        let original = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => b"{}".to_vec(),
            Err(e) => return Err(e.into()),
        };
        let mut value: Value = serde_json::from_slice(&original)?;
        ensure!(
            value.is_object() && (value["hooks"].is_null() || value["hooks"].is_object()),
            "hooks 配置格式无效，原配置已保留"
        );
        let exe = crate::credentials::helper()?;
        let bytes = fs::read(&exe)?;
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let helper = folder.join(format!(
            "aieyes-question-{digest}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let command = format!(
            "{} {} {} {}",
            crate::process::quote(&helper.to_string_lossy()),
            MARKER,
            crate::process::quote(&store.root.to_string_lossy()),
            crate::process::quote(&source.id)
        );
        if value["hooks"]["PermissionRequest"]
            .as_array()
            .is_some_and(|rows| {
                let mut owned_count = 0usize;
                let mut exact = false;
                for r in rows {
                    if let Some(hs) = r["hooks"].as_array() {
                        for h in hs {
                            if h["command"].as_str().is_some_and(|s| s.contains(MARKER)) {
                                owned_count += 1;
                                if r["matcher"] == matcher
                                    && h["command"].as_str() == Some(&command)
                                {
                                    exact = true;
                                }
                            }
                        }
                    }
                }
                owned_count == 1 && exact
            })
        {
            continue;
        }
        if !helper.exists() {
            let tmp = helper.with_extension("tmp");
            fs::copy(&exe, &tmp)?;
            fs::rename(tmp, &helper)?;
        }
        if value["hooks"].is_null() {
            value["hooks"] = json!({});
        }
        if value["hooks"]["PermissionRequest"].is_null() {
            value["hooks"]["PermissionRequest"] = json!([]);
        }
        let rows = value["hooks"]["PermissionRequest"]
            .as_array_mut()
            .context("无法读取会话来源")?;
        for r in rows.iter_mut() {
            if let Some(hs) = r["hooks"].as_array_mut() {
                hs.retain(|h| !h["command"].as_str().is_some_and(|s| s.contains(MARKER)));
            }
        }
        rows.retain(|r| r["hooks"].as_array().is_none_or(|h| !h.is_empty()));
        rows.push(json!({"matcher":matcher,"hooks":[{"type":"command","command":command,"timeout":86400}]}));
        let backup = folder.join(format!(
            "channel-settings-before-{}.json",
            &crate::models::hash(&String::from_utf8_lossy(&original))[..16]
        ));
        if !backup.exists() {
            private_write(&backup, &original)?;
        }
        let tmp = folder.join("channel-settings.pending");
        private_write(&tmp, &serde_json::to_vec_pretty(&value)?)?;
        fs::rename(tmp, path)?;
    }
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
        .filter(|s| hooked(&s.provider) && s.host_id.is_none() && settings.source_enabled(s))
    {
        let Some(home) = home(source) else {
            continue;
        };
        if !seen.insert(home.clone()) {
            continue;
        }
        let path = home.join("settings.json");
        let mode = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| {
                v["hooks"]["PermissionRequest"].as_array().and_then(|arr| {
                    arr.iter().find_map(|r| {
                        let has = r["hooks"].as_array().is_some_and(|hs| {
                            hs.iter()
                                .any(|h| h["command"].as_str().is_some_and(|s| s.contains(MARKER)))
                        });
                        has.then(|| {
                            r["matcher"]
                                .as_str()
                                .unwrap_or("AskUserQuestion")
                                .to_owned()
                        })
                    })
                })
            })
            .unwrap_or_else(|| "none".into());
        rows.push(json!({
            "sourceId": source.id,
            "provider": source.provider,
            "path": path.to_string_lossy(),
            "mode": mode
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

/// Entrypoint used by the PermissionRequest interaction hook, before service startup.
pub fn run_if_requested() -> bool {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(i) = args.iter().position(|a| a == MARKER) else {
        return false;
    };
    let _ = (|| -> Result<()> {
        let root = Path::new(args.get(i + 1).context("missing root")?);
        let source_id = args.get(i + 2).context("missing source")?;
        let mut bytes = Vec::new();
        std::io::stdin().take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "input too large");
        let input: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            input["hook_event_name"] == "PermissionRequest",
            "not a permission request"
        );
        let tool_name = input["tool_name"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .context("missing tool_name")?;
        let is_question = tool_name == "AskUserQuestion";
        let native = input["session_id"]
            .as_str()
            .filter(|s| codex::valid_id(s))
            .context("missing native id")?;
        let raw = &input["tool_input"];
        if is_question {
            let q = raw["questions"].as_array().context("missing questions")?;
            ensure!(!q.is_empty() && q.len() <= 4, "invalid questions");
        }
        let store = Store::open(root)?;
        let c = configuration(&store.db)?;
        let dc = crate::session_interaction::config(&store.db)?;
        let feishu_alive = c.enabled && c.owner.is_some() && service_alive(&store, &c.epoch);
        let desktop_q_alive = dc.enabled
            && (dc.claude_questions || dc.claude_approvals)
            && desktop_alive(&store, DESKTOP_QUESTION_TTL);
        let desktop_a_alive =
            dc.enabled && dc.claude_approvals && desktop_alive(&store, DESKTOP_APPROVAL_TTL);
        if is_question {
            ensure!(feishu_alive || desktop_q_alive, "disabled");
        } else {
            ensure!(desktop_a_alive, "approvals disabled");
        }
        let settings = store.settings()?;
        let installed = settings
            .sources
            .iter()
            .find(|s| s.id == *source_id && hooked(&s.provider) && s.host_id.is_none())
            .context("source unavailable")?;
        let (pid, start, kind_verdict) = crate::session_hooks::ancestry(&installed.provider);
        ensure!(
            pid > 0 && start > 0 && kind_verdict != Some(crate::session_events::HEADLESS),
            "not an interactive agent process"
        );
        // A copied hook must never bind a different source's native session.
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
        let id = super::random_id()?;
        let session_id = crate::models::hash(&format!("local:{}:{native}", installed.provider));
        let (observed_source_id, call, hash, req_kind) = if is_question {
            let hash = question_hash(raw);
            let until_observed = Instant::now() + Duration::from_secs(5);
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
                            && self::home(s).as_ref() == Some(&home)
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
                ensure!(Instant::now() < until_observed, "question not observed");
                std::thread::sleep(Duration::from_millis(100));
            };
            (sid, call, hash, "question")
        } else {
            let sid = crate::session_interaction::session(&store.db, &session_id)
                .ok()
                .flatten()
                .and_then(|target| {
                    settings
                        .sources
                        .iter()
                        .find(|s| {
                            s.id == target.session.source_id
                                && s.provider == installed.provider
                                && s.host_id.is_none()
                                && settings.source_enabled(s)
                                && self::home(s).as_ref() == Some(&home)
                        })
                        .map(|s| s.id.clone())
                })
                .unwrap_or_else(|| installed.id.clone());
            let hash = approval_hash(tool_name, raw);
            let call = input["tool_use_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .map(str::to_owned)
                .unwrap_or_else(|| hash.clone());
            (sid, call, hash, "approval")
        };

        store.db.execute(
            "INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat,kind,tool_name) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,?11,?12)",
            params![
                id,
                c.epoch,
                observed_source_id,
                native,
                hash,
                call,
                raw.to_string(),
                pid,
                start,
                now(),
                req_kind,
                tool_name
            ],
        )?;

        let max_wait = if is_question { 86390 } else { 1800 };
        let until = Instant::now() + Duration::from_secs(max_wait);
        let mut checked = Instant::now() - Duration::from_secs(1);
        while Instant::now() < until {
            if checked.elapsed() >= Duration::from_secs(1) {
                let current = configuration(&store.db)?;
                let dc = crate::session_interaction::config(&store.db)?;
                let settings = store.settings()?;
                let source_ok = settings
                    .sources
                    .iter()
                    .any(|s| s.id == observed_source_id && settings.source_enabled(s));
                let still_active = if is_question {
                    let feishu_ok = current.enabled
                        && current.epoch == c.epoch
                        && current.owner.is_some()
                        && service_alive(&store, &c.epoch);
                    let desktop_ok = dc.enabled
                        && (dc.claude_questions || dc.claude_approvals)
                        && desktop_alive(&store, DESKTOP_QUESTION_TTL);
                    source_ok && (feishu_ok || desktop_ok)
                } else {
                    source_ok
                        && dc.enabled
                        && dc.claude_approvals
                        && desktop_alive(&store, DESKTOP_APPROVAL_TTL)
                };
                if !still_active {
                    break;
                }
                store.db.execute(
                    "UPDATE channel_claude_requests SET heartbeat=?1 WHERE id=?2",
                    params![now(), id],
                )?;
                checked = Instant::now();
            }
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
                store.db.execute(
                    "UPDATE channel_claude_requests SET consumed=1,heartbeat=?1 WHERE id=?2 AND consumed=0",
                    params![now(), id],
                )?;
                if response != PASSTHROUGH {
                    println!("{response}");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(120));
        }
        store.db.execute(
            "DELETE FROM channel_claude_requests WHERE id=?1 AND response IS NULL",
            [id],
        )?;
        Ok(())
    })();
    true
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
