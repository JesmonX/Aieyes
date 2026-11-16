//! AskUserQuestion replies use Claude's PermissionRequest updatedInput contract.
//! Qoder CLI ships the same tool, transcript and hook contract, so it shares this path.
//! The observer hooks remain metadata-only; this separate hook is channel opt-in.
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

/// Agents whose CLI answers `AskUserQuestion` through a `PermissionRequest` hook.
pub(super) fn hooked(provider: &str) -> bool {
    matches!(provider, "claude" | "qoder" | "qoder-cn")
}

/// The CLI configuration directory, or `None` for a source that is not one (Qoder desktop).
fn home(source: &Source) -> Option<std::path::PathBuf> {
    let home = expand(if source.path.is_empty() {
        crate::contracts::provider(&source.provider).map_or("", |p| p.default_path)
    } else {
        &source.path
    });
    (source.provider == "claude"
        || crate::qoder_desktop::database(&source.path).is_none() && home.join("projects").is_dir())
    .then_some(home)
}

pub(super) fn question_hash(input: &Value) -> String {
    let mut questions = input["questions"].clone();
    if let Some(items) = questions.as_array_mut() {
        for q in items {
            q["multiSelect"] = json!(q["multiSelect"] == true);
        }
    }
    crate::models::hash(&questions.to_string())
}

pub(super) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS channel_claude_requests (
        id TEXT PRIMARY KEY, epoch TEXT NOT NULL, source_id TEXT NOT NULL, native TEXT NOT NULL,
        question_hash TEXT NOT NULL, call_id TEXT NOT NULL, input TEXT NOT NULL, pid INTEGER NOT NULL, process_start INTEGER NOT NULL,
        at INTEGER NOT NULL, heartbeat INTEGER NOT NULL, response TEXT, consumed INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS channel_claude_pending ON channel_claude_requests(epoch,source_id,native,question_hash);")?;
    Ok(())
}

pub(super) fn request(target: &Session, excerpt: &content::Excerpt) -> Option<Value> {
    let raw = excerpt.claude_request.as_ref()?;
    let items = raw["input"]["questions"].as_array()?;
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
        json!({"method":"aieyes/claudeQuestion","params":{"threadId":target.session.native_session,"turnId":target.session.turn_id,"itemId":raw["callId"],"questions":questions}}),
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
        let row:Option<(String,u32,u64,String)>=store.db.query_row("SELECT id,pid,process_start,input FROM channel_claude_requests WHERE epoch=?1 AND source_id=?2 AND native=?3 AND question_hash=?4 AND call_id=?5 AND consumed=0 AND response IS NULL AND heartbeat>?6 ORDER BY at DESC LIMIT 1",params![c.epoch,source.id,target.session.native_session,hash,call,now()-4],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
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

/// Reconcile only the owned question hook. Preserve all unrelated hooks and permissions.
pub(super) fn install(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled || c.owner.is_none() {
        return Ok(());
    }
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
                rows.iter().any(|r| {
                    r["matcher"] == "AskUserQuestion"
                        && r["hooks"].as_array().is_some_and(|hs| {
                            hs.iter().any(|h| h["command"].as_str() == Some(&command))
                        })
                })
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
        rows.push(json!({"matcher":"AskUserQuestion","hooks":[{"type":"command","command":command,"timeout":86400}]}));
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

/// Entrypoint used only by the narrow AskUserQuestion hook, before service startup.
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
            input["tool_name"] == "AskUserQuestion"
                && input["hook_event_name"] == "PermissionRequest",
            "not a question"
        );
        let native = input["session_id"]
            .as_str()
            .filter(|s| codex::valid_id(s))
            .context("missing native id")?;
        let raw = &input["tool_input"];
        let q = raw["questions"].as_array().context("missing questions")?;
        ensure!(!q.is_empty() && q.len() <= 4, "invalid questions");
        let store = Store::open(root)?;
        let c = configuration(&store.db)?;
        ensure!(c.enabled && c.owner.is_some(), "disabled");
        let settings = store.settings()?;
        let installed = settings
            .sources
            .iter()
            .find(|s| s.id == *source_id && hooked(&s.provider) && s.host_id.is_none())
            .context("source unavailable")?;
        let (pid, start, kind) = crate::session_hooks::ancestry(&installed.provider);
        ensure!(
            pid > 0 && start > 0 && kind != Some(crate::session_events::HEADLESS),
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
        ensure!(service_alive(&store, &c.epoch), "service unavailable");
        let id = super::random_id()?;
        let hash = question_hash(raw);
        let session_id = crate::models::hash(&format!("local:{}:{native}", installed.provider));
        let until_observed = Instant::now() + Duration::from_secs(5);
        // Several sources may share one configuration directory; the answer is looked up
        // under the source that observed the session, not the one that installed the hook.
        let (source_id, call) = loop {
            if let Some(target) = super::db::session(&store.db, &c.epoch, &session_id)?
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
        store.db.execute("INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",params![id,c.epoch,source_id,native,hash,call,raw.to_string(),pid,start,now()])?;
        let until = Instant::now() + Duration::from_secs(86390);
        let mut checked = Instant::now() - Duration::from_secs(1);
        while Instant::now() < until {
            if checked.elapsed() >= Duration::from_secs(1) {
                let current = configuration(&store.db)?;
                let settings = store.settings()?;
                if !service_alive(&store, &c.epoch)
                    || current.epoch != c.epoch
                    || !current.enabled
                    || !settings
                        .sources
                        .iter()
                        .any(|s| s.id == source_id && settings.source_enabled(s))
                {
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
                store.db.execute("UPDATE channel_claude_requests SET consumed=1,heartbeat=?1 WHERE id=?2 AND consumed=0",params![now(),id])?;
                println!("{response}");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
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
