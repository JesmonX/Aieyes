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
pub(crate) fn excerpt(source: &Source, target: &Session) -> Option<Excerpt> {
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

/// Parse a `questions` field that may be either a JSON array or a JSON-encoded string array.
fn parse_questions_array(value: &Value) -> Option<Vec<Value>> {
    if let Some(arr) = value.as_array() {
        return (!arr.is_empty()).then(|| arr.clone());
    }
    value
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.as_array().cloned())
        .filter(|arr| !arr.is_empty())
}

fn normalize_questions_value(items: &[Value]) -> Value {
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let id = q["id"]
                .as_str()
                .or_else(|| q["header"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| (i + 1).to_string());
            let title = q["question"]
                .as_str()
                .or_else(|| q["title"].as_str())
                .unwrap_or("");
            let multi = q["multiSelect"] == true || q["is_multi_select"] == true;
            let options = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    let label = o["label"]
                        .as_str()
                        .or_else(|| o["name"].as_str())
                        .or_else(|| o.as_str())
                        .unwrap_or("");
                    let mut item = json!({"label": label});
                    if let Some(desc) = o["description"].as_str().filter(|s| !s.is_empty()) {
                        item["description"] = json!(desc);
                    }
                    if let Some(prev) = o["preview"].as_str().filter(|s| !s.trim().is_empty()) {
                        item["preview"] = json!(prev);
                    }
                    item
                })
                .collect::<Vec<_>>();
            json!({
                "id": id,
                "question": title,
                "multiSelect": multi,
                "options": options,
            })
        })
        .collect::<Vec<_>>();
    Value::Array(rows)
}

/// Kiro appends `{id,timestamp,payload}`. An approval stays pending until it is resolved.
fn kiro(rows: Vec<Value>, waiting: bool) -> Excerpt {
    let mut out = Excerpt::default();
    let mut pending: Option<(String, String, Value)> = None;
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
                let raw = json!([{"id":id,"question":p["question"],"options":options}]);
                pending = Some((
                    text(&p["toolCallId"]),
                    content::questions(&raw),
                    normalize_questions_value(raw.as_array().unwrap()),
                ));
            }
            "interaction_resolved" => {
                if pending
                    .as_ref()
                    .is_some_and(|(call, _, _)| p["toolCallId"].as_str() == Some(call))
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
        if let Some((_, question, structured)) = pending {
            out.question = question;
            out.structured_questions = Some(structured);
        } else {
            out.question = out.text.clone();
        }
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
        if let Some(items) = parse_questions_array(&args["questions"]) {
            let normalized = normalize_questions_value(&items);
            out.question = content::questions(&normalized);
            out.structured_questions = Some(normalized);
        } else {
            let tool_name = call["name"].as_str().unwrap_or("");
            let command = args["CommandLine"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            let summary = ["toolSummary", "Description"]
                .iter()
                .find_map(|k| args[*k].as_str().filter(|s| !s.is_empty()))
                .unwrap_or("");
            let detail = if !command.is_empty() {
                command.to_owned()
            } else if !summary.is_empty() {
                summary.to_owned()
            } else {
                args.to_string()
            };
            out.question = format!("{tool_name}\n{}", clip(&detail, 4000));
            if !tool_name.is_empty() {
                out.structured_approval = Some(json!({
                    "toolName": tool_name,
                    "reason": if !summary.is_empty() { summary.to_owned() } else { tool_name.to_owned() },
                    "command": clip(if !command.is_empty() { command } else { &detail }, 4000),
                    "details": "",
                }));
            }
        }
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
    let mut question: Option<(String, String, Option<Value>)> = None;
    let mut approval: Option<(String, String, Value)> = None;
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
                    let parsed = parse_questions_array(&args["questions"])
                        .map(|items| normalize_questions_value(&items));
                    let formatted = parsed
                        .as_ref()
                        .map(content::questions)
                        .unwrap_or_else(|| content::questions(&args["questions"]));
                    question = Some((call.0.clone(), formatted, parsed));
                }
            }
            "tool/result" => {
                if question
                    .as_ref()
                    .is_some_and(|(id, _, _)| d["message"]["toolCallId"].as_str() == Some(id))
                {
                    question = None;
                }
            }
            "approval/asked" => {
                let tool_name = text(&d["toolName"]);
                let reason = d["reason"].as_str().unwrap_or("").to_owned();
                let mut lines = vec![tool_name.clone()];
                if !reason.is_empty() {
                    lines.push(reason.clone());
                }
                let cmd = if !call.0.is_empty() && d["callId"].as_str() == Some(&call.0) {
                    let clipped = clip(&call.1, 4000);
                    lines.push(clipped.clone());
                    clipped
                } else {
                    String::new()
                };
                let structured = json!({
                    "toolName": tool_name,
                    "reason": reason,
                    "command": cmd,
                    "details": "",
                });
                approval = Some((text(&d["id"]), lines.join("\n"), structured));
            }
            "approval/decided" => {
                if approval
                    .as_ref()
                    .is_some_and(|(id, _, _)| d["id"].as_str() == Some(id))
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
    if let Some((_, q, structured)) = approval {
        out.question = q;
        out.structured_approval = Some(structured);
    } else if let Some((_, q, structured)) = question {
        out.question = q;
        out.structured_questions = structured;
    }
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
            if let Some(items) = parse_questions_array(&tool["input"]["questions"]) {
                let normalized = normalize_questions_value(&items);
                out.question = content::questions(&normalized);
                out.structured_questions = Some(normalized);
            } else {
                out.question = content::questions(&tool["input"]["questions"]);
            }
        }
    }
    Some(out)
}
