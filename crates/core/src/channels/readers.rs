//! Bounded readers for agents whose history is not a Claude or Codex transcript.
//! They make a pending question or approval and the final reply readable. These agents have
//! no verified answer interface, so a reply still goes through the terminal adapters.
//!
//! Record formats were observed with Kimi Code 2.4.1, Grok CLI 1.0.62 and 1.0.92, Copilot CLI 1.0.105 and
//! CodeBuddy Code 2.187.0. A row that does not match is skipped, never guessed at.
use super::{
    Session,
    content::{self, Excerpt, clip, text_blocks},
    db::MAX_INPUT,
};
use crate::{models::Source, sessions::Phase, store::expand};
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_TAIL: usize = 2 * 1024 * 1024;

/// `None` when this source has no dedicated reader or its record is unavailable.
pub(crate) fn excerpt(source: &Source, target: &Session) -> Option<Excerpt> {
    let waiting = target.session.phase == Phase::NeedsInput;
    let native = &target.session.native_session;
    let root = expand(&source.path);
    // Kimi names a session `<folder>:<agent>`; its record is found by that derived name only.
    if source.provider == "kimi" {
        return Some(bounded(kimi(kimi_rows(&root, target)?, native, waiting)));
    }
    // The native identifier becomes a path component or a query parameter.
    if native.is_empty()
        || native.len() > 128
        || !native
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return None;
    }
    let out = match source.provider.as_str() {
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
        "grok" => {
            let events = named(target, "events.jsonl")?;
            let updates = events.with_file_name("updates.jsonl");
            grok(
                content::records(&events, &root, target).ok()?.0,
                content::records(&updates, &root, target)
                    .map(|r| r.0)
                    .unwrap_or_default(),
                waiting,
            )
        }
        "copilot" => {
            let events = named(target, "events.jsonl")?;
            let rows = content::records(&events, &root, target).ok()?.0;
            copilot(rows, waiting, &|call| copilot_field_order(&events, call))
        }
        "codebuddy" | "workbuddy" => {
            let base = if root.is_file() {
                root.parent()?
            } else {
                &root
            };
            let path = transcript(base, target)?;
            codebuddy(
                content::records(&path, base, target).ok()?.0,
                waiting,
                target.session.input_kind == "approval",
            )
        }
        _ => return None,
    };
    Some(bounded(out))
}

fn bounded(mut out: Excerpt) -> Excerpt {
    out.text = clip(&out.text, 10000);
    out.question = clip(&out.question, 10000);
    out.user_text = clip(&out.user_text, MAX_INPUT);
    out
}

/// The session's own record files, the active one first.
fn candidates(target: &Session) -> impl Iterator<Item = &Path> {
    std::iter::once(&target.path)
        .chain(&target.log_paths)
        .filter(|p| !p.is_empty())
        .take(8)
        .map(Path::new)
}

/// The record named `file` inside this session's own folder.
fn named(target: &Session, file: &str) -> Option<PathBuf> {
    candidates(target)
        .find(|p| {
            p.file_name().is_some_and(|n| n == file)
                && p.parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    == Some(&target.session.native_session)
        })
        .map(Path::to_path_buf)
}

/// CodeBuddy and WorkBuddy name a transcript after its session: `projects/<project>/<id>.jsonl`.
/// WorkBuddy sessions come from its database, so the transcript is looked up by that name.
fn transcript(root: &Path, target: &Session) -> Option<PathBuf> {
    let native = &target.session.native_session;
    let own = |p: &Path| {
        p.extension().is_some_and(|e| e == "jsonl")
            && p.file_stem().and_then(|n| n.to_str()) == Some(native)
    };
    if let Some(path) = candidates(target).find(|p| own(p)) {
        return Some(path.to_path_buf());
    }
    std::fs::read_dir(root.join("projects"))
        .ok()?
        .filter_map(|e| e.ok())
        .take(4096)
        .map(|e| e.path().join(format!("{native}.jsonl")))
        .find(|p| p.is_file())
}

/// A bounded tail of committed JSONL rows from a record below `root`. The caller has already
/// established whose record it is.
fn tail_rows(path: &Path, root: &Path) -> Option<Vec<Value>> {
    let path = path.canonicalize().ok()?;
    if !path.starts_with(root.canonicalize().ok()?) || !path.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let offset = size.saturating_sub(MAX_TAIL as u64);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut data = Vec::new();
    file.take(MAX_TAIL as u64).read_to_end(&mut data).ok()?;
    let data = if offset > 0 {
        data.splitn(2, |b| *b == b'\n').nth(1).unwrap_or_default()
    } else {
        &data
    };
    Some(
        data.split_inclusive(|b| *b == b'\n')
            .filter(|l| l.ends_with(b"\n"))
            .filter_map(|l| serde_json::from_slice(l).ok())
            .collect(),
    )
}

/// What a tool call would run or touch, for an approval card.
fn call_subject(args: &Value) -> &str {
    [
        "command",
        "file_path",
        "filePath",
        "path",
        "target_file",
        "url",
    ]
    .iter()
    .find_map(|k| args[*k].as_str().filter(|s| !s.is_empty()))
    .unwrap_or("")
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
            let multi = q["multiSelect"] == true
                || q["is_multi_select"] == true
                || q["multi_select"] == true;
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
    let mut pending: Vec<&Value> = Vec::new();
    for v in &rows {
        let p = &v["payload"];
        match p["type"].as_str().unwrap_or("") {
            "user" => {
                out.user_text = text(&p["content"]);
                out.user_at = millis(&v["timestamp"]);
                out.text.clear();
                pending.clear();
            }
            "session_start" | "turn_start" => {
                out.text.clear();
                pending.clear();
            }
            // Text before a tool call is progress, not the reply.
            "tool_call" => out.text.clear(),
            "assistant" if p["operationType"] == "Say" => out.text = text(&p["content"]),
            "pending_interaction" => pending.push(p),
            "interaction_resolved" if p["toolCallId"].is_string() => {
                pending.retain(|request| {
                    request["toolCallId"] != p["toolCallId"]
                        || request["consentRound"] != p["consentRound"]
                });
            }
            "turn_end" => pending.clear(),
            _ => {}
        }
    }
    if waiting {
        // Without a pending interaction the model asked in its last message.
        if let Some(p) = pending.last() {
            let options = p["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| json!({"label":o["name"]}))
                .collect::<Vec<_>>();
            let kiro = &p["_meta"]["kiro"];
            let id = kiro["toolId"]
                .as_str()
                .or_else(|| p["interactionType"].as_str())
                .unwrap_or("1");
            let raw = json!([{"id":id,"question":p["question"],"options":options}]);
            out.question = content::questions(&raw);
            if p["interactionType"] == "tool_approval" {
                let consent = &kiro["consent"];
                let resource = clip(consent["resource"].as_str().unwrap_or(""), 4000);
                if !resource.is_empty() {
                    out.question.push('\n');
                    out.question.push_str(&resource);
                }
                out.structured_approval = Some(json!({
                    "toolName": kiro["toolId"].as_str().or_else(|| consent["capability"].as_str()).unwrap_or("Kiro"),
                    "kind": if consent["capability"] == "shell" { "command" } else { "tool" },
                    "reason": clip(p["question"].as_str().unwrap_or(""), 800),
                    "command": resource,
                }));
            } else {
                out.structured_questions = Some(normalize_questions_value(raw.as_array().unwrap()));
            }
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

/// Kimi's `wire.jsonl` of the agent this session is named after.
fn kimi_rows(root: &Path, target: &Session) -> Option<Vec<Value>> {
    let native = &target.session.native_session;
    if native.is_empty() || native.len() > 160 {
        return None;
    }
    let path = candidates(target).find(|p| {
        p.file_name().is_some_and(|n| n == "wire.jsonl")
            && crate::native_agents::session_id(p) == *native
    })?;
    tail_rows(path, root)
}

/// Kimi logs `interaction.request` when it opens an approval or a question menu and
/// `interaction.resolved` with the same identifier when the user has answered it.
fn kimi(rows: Vec<Value>, native: &str, waiting: bool) -> Excerpt {
    let mut out = Excerpt::default();
    let mut pending: Vec<Value> = Vec::new();
    for v in rows {
        match v["type"].as_str().unwrap_or("") {
            "turn.prompt" => {
                out.user_text = text_blocks(&v["input"]);
                out.user_at = v["time"].as_i64().unwrap_or(0);
                out.text.clear();
                pending.clear();
            }
            "context.append_loop_event" => match v["event"]["type"].as_str() {
                Some("content.part") if v["event"]["part"]["type"] == "text" => {
                    out.text = text(&v["event"]["part"]["text"]);
                }
                // Text before a tool call is progress, not the reply.
                Some("tool.call") => out.text.clear(),
                _ => {}
            },
            "interaction.request" => pending.push(v),
            "interaction.resolved" => pending.retain(|p| p["id"] != v["id"]),
            "turn.ended" if v["reason"] != "blocked" => pending.clear(),
            _ => {}
        }
    }
    if !waiting {
        return out;
    }
    let session = native.split(':').next().unwrap_or("");
    // A request that names another session is not this one's prompt.
    let Some(row) = pending.into_iter().next_back().filter(|p| {
        p["request"]["sessionId"]
            .as_str()
            .is_none_or(|id| id == session)
    }) else {
        out.question = out.text.clone();
        return out;
    };
    let request = &row["request"];
    if row["kind"] == "question" {
        if let Some(items) = parse_questions_array(&request["questions"]) {
            let normalized = normalize_questions_value(&items);
            out.question = content::questions(&normalized);
            out.structured_questions = Some(normalized);
        }
    } else if row["kind"] == "approval" {
        let display = &request["display"];
        let tool = request["toolName"].as_str().unwrap_or("Tool");
        let command = call_subject(display);
        let details = match display["operation"].as_str() {
            Some("edit") => format!(
                "- {}\n+ {}",
                display["before"].as_str().unwrap_or(""),
                display["after"].as_str().unwrap_or("")
            ),
            _ => text(&display["content"]),
        };
        let reason = text(&request["action"]);
        out.question = [tool, &reason, command]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        out.structured_approval = Some(json!({
            "toolName": tool,
            "reason": reason,
            "command": clip(command, 4000),
            "details": clip(&details, 2000),
            "kind": match display["kind"].as_str() {
                Some("command") => "command",
                Some("file_io") => "fileChange",
                _ => "tool",
            },
            "cwd": display["cwd"].as_str().unwrap_or(""),
        }));
    }
    out
}

/// Grok keeps lifecycle rows in `events.jsonl` and the tool calls themselves in
/// `updates.jsonl`. A permission row names only its tool, so the command is shown only when
/// exactly one unfinished call of that tool exists.
fn grok(events: Vec<Value>, updates: Vec<Value>, waiting: bool) -> Excerpt {
    let mut out = Excerpt::default();
    // (call, tool, kind, input)
    let mut calls: Vec<(String, String, String, Value)> = Vec::new();
    for v in updates {
        let u = &v["params"]["update"];
        let call = u["toolCallId"].as_str().unwrap_or("");
        match u["sessionUpdate"].as_str().unwrap_or("") {
            "user_message_chunk" => {
                out.user_text = text(&u["content"]["text"]);
                out.user_at = v["timestamp"].as_i64().unwrap_or(0) * 1000;
                out.text.clear();
                calls.clear();
            }
            "agent_message_chunk" => {
                if let Some(chunk) = u["content"]["text"].as_str() {
                    out.text.push_str(chunk);
                }
            }
            "tool_call" if !call.is_empty() => {
                // Text before a tool call is progress, not the reply.
                out.text.clear();
                let meta = &u["_meta"]["x.ai/tool"];
                calls.retain(|c| c.0 != call);
                calls.push((
                    call.to_owned(),
                    text(if meta["name"].is_string() {
                        &meta["name"]
                    } else {
                        &u["title"]
                    }),
                    text(&meta["kind"]),
                    u["rawInput"].clone(),
                ));
            }
            "tool_call_update"
                if matches!(
                    u["status"].as_str(),
                    Some("completed" | "failed" | "cancelled" | "canceled" | "rejected")
                ) =>
            {
                calls.retain(|c| c.0 != call);
            }
            _ => {}
        }
    }
    // The tools with an open permission dialog, oldest first. Rows name only the tool, and two
    // parallel calls of one tool each have their own entry.
    let mut pending: Vec<String> = Vec::new();
    for v in events {
        let tool = v["tool_name"].as_str().unwrap_or("");
        let close = |pending: &mut Vec<String>| {
            if let Some(i) = pending.iter().rposition(|t| t == tool) {
                pending.remove(i);
            }
        };
        match v["type"].as_str().unwrap_or("") {
            "permission_requested" => pending.push(tool.to_owned()),
            "permission_resolved" | "permission_response" | "permission_responded" => {
                if tool.is_empty() {
                    pending.pop();
                } else {
                    close(&mut pending);
                }
            }
            // Another tool finishing says nothing about this one's dialog.
            "tool_completed" | "tool_finished" | "tool_failed" if !tool.is_empty() => {
                close(&mut pending)
            }
            "turn_started" | "turn_ended" => pending.clear(),
            _ => {}
        }
    }
    if !waiting {
        return out;
    }
    if let Some(tool) = pending.pop().filter(|t| !t.is_empty()) {
        let mut same = calls.iter().filter(|c| c.1 == tool);
        let only = match (same.next(), same.next()) {
            (Some(call), None) => Some(call),
            _ => None,
        };
        let input = only.map_or(&Value::Null, |c| &c.3);
        let command = call_subject(input);
        let reason = input["description"].as_str().unwrap_or("");
        out.question = [tool.as_str(), reason, command]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        out.structured_approval = Some(json!({
            "toolName": tool,
            "reason": reason,
            "command": clip(command, 4000),
            "details": "",
            "kind": match only.map(|c| c.2.as_str()) {
                Some("execute") => "command",
                Some("edit" | "write" | "delete" | "move") => "fileChange",
                _ => "tool",
            },
        }));
    } else if let Some(items) = calls
        .iter()
        .rfind(|c| c.1 == "ask_user_question" || c.2 == "ask_user")
        .and_then(|c| parse_questions_array(&c.3["questions"]))
    {
        let normalized = normalize_questions_value(&items);
        out.question = content::questions(&normalized);
        out.structured_questions = Some(normalized);
    } else {
        out.question = out.text.clone();
    }
    out
}

/// Copilot CLI `events.jsonl`: a permission is open between `permission.requested` and
/// `permission.completed`, a question while its `ask_user` tool call runs.
/// `order` gives the field names of an `ask_user` form as its record lists them.
fn copilot(rows: Vec<Value>, waiting: bool, order: &dyn Fn(&str) -> Vec<String>) -> Excerpt {
    let mut out = Excerpt::default();
    let mut permissions: Vec<Value> = Vec::new();
    let mut asks: Vec<(String, Value)> = Vec::new();
    for v in rows {
        let d = &v["data"];
        match v["type"].as_str().unwrap_or("") {
            "user.message" => {
                out.user_text = text(&d["content"]);
                out.user_at = millis(&v["timestamp"]);
                out.text.clear();
                permissions.clear();
                asks.clear();
            }
            "assistant.message" => {
                // Text beside tool requests is progress, not the reply.
                out.text = if d["toolRequests"].as_array().is_none_or(|a| a.is_empty()) {
                    text(&d["content"])
                } else {
                    String::new()
                };
            }
            "tool.execution_start" if d["toolName"] == "ask_user" => {
                asks.push((text(&d["toolCallId"]), d["arguments"].clone()));
            }
            "tool.execution_complete" => {
                asks.retain(|a| d["toolCallId"].as_str() != Some(&a.0));
            }
            "permission.requested" => permissions.push(v["data"].clone()),
            "permission.completed" => permissions.retain(|p| p["requestId"] != d["requestId"]),
            "abort" | "session.abort" | "assistant.abort" | "session.error" => {
                permissions.clear();
                asks.clear();
            }
            _ => {}
        }
    }
    if !waiting {
        return out;
    }
    if let Some(p) = permissions.last() {
        let request = &p["permissionRequest"];
        let kind = request["kind"].as_str().unwrap_or("tool");
        let command = ["fullCommandText", "fileName", "path", "url", "toolName"]
            .iter()
            .find_map(|k| request[*k].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("");
        let reason = request["intention"].as_str().unwrap_or("");
        out.question = [kind, reason, command]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        out.structured_approval = Some(json!({
            "toolName": kind,
            "reason": reason,
            "command": clip(command, 4000),
            "details": clip(request["diff"].as_str().unwrap_or(""), 2000),
            "kind": match kind {
                "shell" => "command",
                "write" => "fileChange",
                _ => "tool",
            },
        }));
    } else if let Some((call, args)) = asks.last() {
        let message = args["message"].as_str().unwrap_or("");
        let fields = args["requestedSchema"]["properties"]
            .as_object()
            .filter(|m| !m.is_empty() && m.len() <= 32);
        // Only a form made of plain choices is shown as one; anything else is its message.
        let items = fields.and_then(|map| {
            // A parsed object is sorted by key; the form is shown in the order it was written.
            let mut listed: Vec<_> = map.iter().collect();
            if listed.len() > 1 {
                let written = order(call);
                listed.sort_by_key(|(key, _)| {
                    written
                        .iter()
                        .position(|name| name == *key)
                        .unwrap_or(usize::MAX)
                });
            }
            listed
                .into_iter()
                .map(|(key, f)| {
                    // Numbers and booleans are shown as written; anything else is not a choice.
                    let options = f["enum"]
                        .as_array()
                        .filter(|o| !o.is_empty())?
                        .iter()
                        .map(|o| match o {
                            Value::String(_) => Some(o.clone()),
                            Value::Number(_) | Value::Bool(_) => Some(json!(o.to_string())),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let title = if map.len() == 1 && !message.is_empty() {
                        message
                    } else {
                        f["title"]
                            .as_str()
                            .or_else(|| f["description"].as_str())
                            .unwrap_or(key)
                    };
                    Some(json!({"id": key, "question": title, "options": options}))
                })
                .collect::<Option<Vec<_>>>()
        });
        if let Some(items) = items {
            let normalized = normalize_questions_value(&items);
            out.question = content::questions(&normalized);
            if fields.is_some_and(|m| m.len() > 1) && !message.is_empty() {
                out.question = format!("{message}\n{}", out.question);
            }
            out.structured_questions = Some(normalized);
        } else {
            out.question = message.to_owned();
        }
    } else {
        out.question = out.text.clone();
    }
    out
}

/// The field names of the `ask_user` form of `call`, in the order its record lists them.
/// Read from the bounded tail of the record that `content::records` has already accepted.
fn copilot_field_order(path: &Path, call: &str) -> Vec<String> {
    use super::claude::elicitation::Keys;
    #[derive(serde::Deserialize)]
    struct Schema {
        #[serde(default)]
        properties: Keys,
    }
    #[derive(serde::Deserialize)]
    struct Arguments {
        #[serde(rename = "requestedSchema")]
        schema: Schema,
    }
    #[derive(serde::Deserialize)]
    struct Data {
        #[serde(rename = "toolCallId", default)]
        call: String,
        arguments: Arguments,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "type")]
        kind: String,
        data: Data,
    }
    let read = || -> Option<Vec<u8>> {
        let mut file = std::fs::File::open(path).ok()?;
        let size = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(size.saturating_sub(MAX_TAIL as u64)))
            .ok()?;
        let mut data = Vec::new();
        file.take(MAX_TAIL as u64).read_to_end(&mut data).ok()?;
        Some(data)
    };
    let Some(data) = read() else {
        return Vec::new();
    };
    data.split(|b| *b == b'\n')
        .rev()
        .filter(|line| line.windows(15).any(|w| w == b"requestedSchema"))
        .filter_map(|line| serde_json::from_slice::<Row>(line).ok())
        .find(|row| row.kind == "tool.execution_start" && row.data.call == call)
        .map(|row| row.data.arguments.schema.properties.0)
        .unwrap_or_default()
}

/// CodeBuddy and WorkBuddy transcripts: a `function_call` row stays unanswered until a result
/// row names its `callId`. `approval` says the session is known to wait on a permission.
fn codebuddy(rows: Vec<Value>, waiting: bool, approval: bool) -> Excerpt {
    let mut out = Excerpt::default();
    // (call, tool, arguments)
    let mut calls: Vec<(String, String, String)> = Vec::new();
    for v in rows {
        // A subagent's rows are not this session's conversation.
        if v["isSidechain"] == true {
            continue;
        }
        match (v["type"].as_str().unwrap_or(""), v["role"].as_str()) {
            ("message", Some("user")) => {
                out.user_text = text_blocks(&v["content"]);
                out.user_at = v["timestamp"].as_i64().unwrap_or(0);
                out.text.clear();
                calls.clear();
            }
            ("message", Some("assistant")) if v["status"] == "completed" => {
                out.text = text_blocks(&v["content"]);
            }
            ("function_call", _) => {
                // Text before a tool call is progress, not the reply.
                out.text.clear();
                calls.push((text(&v["callId"]), text(&v["name"]), text(&v["arguments"])));
            }
            ("function_call_output" | "function_call_result", _) => {
                calls.retain(|c| v["callId"].as_str() != Some(&c.0));
            }
            _ => {}
        }
    }
    if !waiting {
        return out;
    }
    let args = |raw: &str| serde_json::from_str::<Value>(raw).unwrap_or(Value::Null);
    if let Some(items) = calls
        .iter()
        .rfind(|c| c.1 == "AskUserQuestion")
        .and_then(|c| parse_questions_array(&args(&c.2)["questions"]))
    {
        let normalized = normalize_questions_value(&items);
        out.question = content::questions(&normalized);
        out.structured_questions = Some(normalized);
    } else if approval && calls.len() == 1 {
        // With several calls in flight the transcript does not say which one is being asked.
        let (_, tool, raw) = &calls[0];
        let input = args(raw);
        let command = call_subject(&input);
        let reason = input["description"].as_str().unwrap_or("");
        let details = ["content", "new_string"]
            .iter()
            .find_map(|k| input[*k].as_str())
            .unwrap_or("");
        out.question = [tool.as_str(), reason, command]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        out.structured_approval = Some(json!({
            "toolName": tool,
            "reason": reason,
            "command": clip(command, 4000),
            "details": clip(details, 2000),
            "kind": if input["command"].is_string() { "command" } else { "tool" },
        }));
    } else if !approval {
        out.question = out.text.clone();
    }
    out
}
