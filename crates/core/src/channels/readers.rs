//! Bounded readers for agents whose history is not a Claude or Codex transcript.
//! They make a pending question or approval and the final reply readable. These agents have
//! no verified answer interface, so a reply still goes through the terminal adapters.
use super::{
    Session,
    content::{self, Excerpt, clip, text_blocks},
    db::MAX_INPUT,
};
use crate::{models::Source, sessions::Phase, store::expand};
use serde_json::{Value, json};
use std::{io::Read, path::Path};

const MAX_TAIL: usize = 2 * 1024 * 1024;

/// `None` when this source has no dedicated reader or its record is unavailable.
pub(super) fn excerpt(source: &Source, target: &Session) -> Option<Excerpt> {
    let waiting = target.session.phase == Phase::NeedsInput;
    let native = &target.session.native_session;
    // The native identifier becomes a path component or a query parameter.
    if native.is_empty()
        || native.len() > 128
        || !native
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return None;
    }
    let root = expand(&source.path);
    let mut out = match source.provider.as_str() {
        "kiro" => {
            let folder = crate::native_sessions::kiro::directory(&source.path)?;
            let path = std::fs::read_dir(&folder)
                .ok()?
                .filter_map(|e| e.ok())
                .take(4096)
                .map(|e| e.path().join(native).join("messages.jsonl"))
                .find(|p| p.is_file())?;
            kiro(content::records(&path, &folder, target).ok()?.0, waiting)
        }
        "antigravity" => {
            let path = root
                .join("brain")
                .join(native)
                .join(".system_generated/logs/transcript.jsonl");
            antigravity(content::records(&path, &root, target).ok()?.0, waiting)
        }
        "deepseek-harness" => harness(harness_rows(Path::new(&target.path), &root, target)?),
        "qoder" | "qoder-cn" => {
            qoder_desktop(&crate::qoder_desktop::database(&source.path)?, native)?
        }
        _ => return None,
    };
    out.text = clip(&out.text, 10000);
    out.question = clip(&out.question, 10000);
    out.user_text = clip(&out.user_text, MAX_INPUT);
    Some(out)
}

fn millis(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or(0, |t| t.timestamp_millis())
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_owned()
}

/// Kiro appends `{id,timestamp,payload}`. An approval stays pending until it is resolved.
fn kiro(rows: Vec<Value>, waiting: bool) -> Excerpt {
    let mut out = Excerpt::default();
    let mut pending: Option<(String, String)> = None;
    for v in rows {
        let p = &v["payload"];
        match p["type"].as_str().unwrap_or("") {
            "user" => {
                out.user_text = text(&p["content"]);
                out.user_at = millis(&v["timestamp"]);
                out.text.clear();
                pending = None;
            }
            // Text before a tool call is progress, not the reply.
            "turn_start" | "tool_call" => out.text.clear(),
            "assistant" if p["operationType"] == "Say" => out.text = text(&p["content"]),
            "pending_interaction" => {
                let options = p["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| json!({"label":o["name"]}))
                    .collect::<Vec<_>>();
                let id = p["_meta"]["kiro"]["toolId"]
                    .as_str()
                    .or_else(|| p["interactionType"].as_str())
                    .unwrap_or("1");
                pending = Some((
                    text(&p["toolCallId"]),
                    content::questions(
                        &json!([{"id":id,"question":p["question"],"options":options}]),
                    ),
                ));
            }
            "interaction_resolved" => {
                if pending
                    .as_ref()
                    .is_some_and(|(call, _)| p["toolCallId"].as_str() == Some(call))
                {
                    pending = None;
                }
            }
            "turn_end" => pending = None,
            _ => {}
        }
    }
    if waiting {
        // Without a pending interaction the model asked in its last message.
        out.question = pending.map_or_else(|| out.text.clone(), |(_, question)| question);
    }
    out
}

/// Antigravity writes one row per step; the planner's tool calls start in order.
fn antigravity(rows: Vec<Value>, waiting: bool) -> Excerpt {
    let mut out = Excerpt::default();
    let mut calls = Vec::new();
    let mut finished = 0;
    for v in rows {
        match v["type"].as_str().unwrap_or("") {
            "USER_INPUT" => {
                out.user_text = text(&v["content"]);
                out.user_at = millis(&v["created_at"]);
                out.text.clear();
                calls.clear();
                finished = 0;
            }
            "PLANNER_RESPONSE" => {
                calls = v["tool_calls"].as_array().cloned().unwrap_or_default();
                finished = 0;
                out.text = if calls.is_empty() {
                    text(&v["content"])
                } else {
                    String::new()
                };
            }
            // Every started call has one step row; a call awaiting the user has none yet.
            "GENERIC" => finished += 1,
            _ => {}
        }
    }
    if waiting && let Some(call) = calls.get(finished) {
        let args = &call["args"];
        out.question = if args["questions"].is_array() {
            content::questions(&args["questions"])
        } else {
            let detail = ["CommandLine", "toolSummary", "Description"]
                .iter()
                .find_map(|k| args[*k].as_str().filter(|s| !s.is_empty()))
                .map_or_else(|| args.to_string(), str::to_owned);
            format!(
                "{}\n{}",
                call["name"].as_str().unwrap_or(""),
                clip(&detail, 4000)
            )
        };
    }
    out
}

/// DeepSeek Harness logs are zstd frames of JSONL; only a bounded tail is kept in memory.
fn harness_rows(path: &Path, root: &Path, target: &Session) -> Option<Vec<Value>> {
    if path.extension().is_none_or(|e| e != "zstd") {
        return Some(content::records(path, root, target).ok()?.0);
    }
    let path = path.canonicalize().ok()?;
    if !path.starts_with(root.canonicalize().ok()?) {
        return None;
    }
    let mut reader = zstd::stream::read::Decoder::new(std::fs::File::open(path).ok()?).ok()?;
    let mut tail = Vec::new();
    let mut head = Vec::new();
    let mut buffer = vec![0; 65536];
    let mut total = 0usize;
    // A frame still being written ends the readable part.
    while let Ok(n) = reader.read(&mut buffer) {
        if n == 0 || total > 512 * 1024 * 1024 {
            break;
        }
        if head.len() < 65536 {
            head.extend_from_slice(&buffer[..n.min(65536 - head.len())]);
        }
        total += n;
        tail.extend_from_slice(&buffer[..n]);
        if tail.len() > 2 * MAX_TAIL {
            tail.drain(..tail.len() - MAX_TAIL);
        }
    }
    let header: Value = serde_json::from_slice(head.split(|b| *b == b'\n').next()?).ok()?;
    if header["type"] != "session" || header["id"].as_str() != Some(&target.session.native_session)
    {
        return None;
    }
    let data = if total > tail.len() {
        tail.splitn(2, |b| *b == b'\n').nth(1).unwrap_or_default()
    } else {
        &tail
    };
    Some(
        data.split_inclusive(|b| *b == b'\n')
            .filter(|l| l.ends_with(b"\n"))
            .filter_map(|l| serde_json::from_slice(l).ok())
            .collect(),
    )
}

fn harness(rows: Vec<Value>) -> Excerpt {
    let mut out = Excerpt::default();
    let mut question: Option<(String, String)> = None;
    let mut approval: Option<(String, String)> = None;
    let mut call = (String::new(), String::new());
    for v in rows {
        let d = &v["data"];
        match v["type"].as_str().unwrap_or("") {
            "turn/start" => {
                out.text.clear();
                question = None;
                approval = None;
            }
            "user/message" => {
                out.user_text = text_blocks(&d["content"]);
                out.user_at = v["time"].as_i64().unwrap_or(0);
            }
            "assistant/message" => {
                let reply = text_blocks(&d["message"]["content"]);
                if !reply.is_empty() {
                    out.text = reply;
                }
            }
            "tool/call" => {
                // Text before a tool call is progress, not the reply.
                out.text.clear();
                call = (text(&d["callId"]), text(&d["arguments"]));
                if d["name"] == "ask_user_question"
                    && let Ok(args) = serde_json::from_str::<Value>(&call.1)
                {
                    question = Some((call.0.clone(), content::questions(&args["questions"])));
                }
            }
            "tool/result" => {
                if question
                    .as_ref()
                    .is_some_and(|(id, _)| d["message"]["toolCallId"].as_str() == Some(id))
                {
                    question = None;
                }
            }
            "approval/asked" => {
                let mut lines = vec![text(&d["toolName"])];
                if let Some(reason) = d["reason"].as_str().filter(|s| !s.is_empty()) {
                    lines.push(reason.to_owned());
                }
                if !call.0.is_empty() && d["callId"].as_str() == Some(&call.0) {
                    lines.push(clip(&call.1, 4000));
                }
                approval = Some((text(&d["id"]), lines.join("\n")));
            }
            "approval/decided" => {
                if approval
                    .as_ref()
                    .is_some_and(|(id, _)| d["id"].as_str() == Some(id))
                {
                    approval = None;
                }
            }
            "turn/end" => {
                question = None;
                approval = None;
            }
            _ => {}
        }
    }
    out.question = approval.or(question).map(|(_, q)| q).unwrap_or_default();
    out
}

/// Qoder desktop keeps display parts per message; a question is a tool part that has not ended.
fn qoder_desktop(database: &Path, native: &str) -> Option<Excerpt> {
    let db = crate::sqlite_ro::open_ro(database).ok()?;
    let latest = |role: &str| -> Option<(Value, i64)> {
        let (payload, at): (String, rusqlite::types::Value) = db
            .query_row(
                "SELECT payload_json,created_at FROM chat_session_messages WHERE session_id=?1 AND role=?2 AND length(payload_json)<=?3 ORDER BY sequence DESC LIMIT 1",
                rusqlite::params![native, role, 4 * MAX_TAIL as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        let at = match at {
            rusqlite::types::Value::Integer(n) if n > 10_000_000_000 => n,
            rusqlite::types::Value::Integer(n) => n * 1000,
            rusqlite::types::Value::Text(s) => millis(&json!(s)),
            _ => 0,
        };
        Some((serde_json::from_str(&payload).ok()?, at))
    };
    let mut out = Excerpt::default();
    if let Some((user, at)) = latest("user") {
        out.user_text = text_blocks(&user["parts"]);
        out.user_at = at;
    }
    if let Some((assistant, at)) = latest("assistant")
        && at >= out.user_at
    {
        let parts = assistant["parts"].as_array().map_or(&[][..], Vec::as_slice);
        // Text with a parent tool belongs to a subagent.
        let texts = || {
            parts
                .iter()
                .filter(|p| p["type"] == "text" && p["parentToolUseId"].is_null())
        };
        out.text = texts()
            .find(|p| p["id"] == assistant["finalTextId"])
            .or_else(|| texts().next_back())
            .map(|p| text(&p["text"]))
            .unwrap_or_default();
        if let Some(tool) = parts.iter().map(|p| &p["tool"]).rfind(|t| {
            t["name"] == "AskUserQuestion"
                && t["completedAt"].is_null()
                && !matches!(
                    t["status"].as_str(),
                    Some("completed" | "failed" | "error" | "cancelled" | "interrupted")
                )
        }) {
            out.question = content::questions(&tool["input"]["questions"]);
        }
    }
    Some(out)
}
