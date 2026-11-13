use super::{Session, db::MAX_INPUT};
use crate::{models::Source, store::expand};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_TAIL: u64 = 2 * 1024 * 1024;

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Excerpt {
    pub text: String,
    pub question: String,
    pub user_text: String,
    pub user_at: i64,
}

pub(super) fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit.saturating_sub("\n…（内容已截断）".len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（内容已截断）", &text[..end])
}

fn time(v: &Value) -> i64 {
    let v = v
        .get("timestamp")
        .or_else(|| v.get("time"))
        .or_else(|| v.get("at"));
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .map(|n| if n < 10_000_000_000 { n * 1000 } else { n })
            .unwrap_or(0),
        Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|t| t.timestamp_millis())
            .unwrap_or(0),
        _ => 0,
    }
}

pub(super) fn text_blocks(v: &Value) -> String {
    if let Some(text) = v.as_str() {
        return text.to_owned();
    }
    v.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|p| {
                    if matches!(
                        p["type"].as_str(),
                        Some("text" | "output_text" | "input_text" | "Text")
                    ) {
                        p["text"].as_str()
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub(super) fn questions(value: &Value) -> String {
    let Some(items) = value.as_array() else {
        return String::new();
    };
    items
        .iter()
        .enumerate()
        .map(|(index, q)| {
            let id = q["id"]
                .as_str()
                .or_else(|| q["header"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| (index + 1).to_string());
            let title = q["question"]
                .as_str()
                .or_else(|| q["title"].as_str())
                .unwrap_or("");
            let options = q["options"]
                .as_array()
                .map(|opts| {
                    opts.iter()
                        .enumerate()
                        .map(|(i, o)| {
                            format!(
                                "  {}. {}{}",
                                i + 1,
                                o["label"].as_str().or_else(|| o.as_str()).unwrap_or(""),
                                o["description"]
                                    .as_str()
                                    .filter(|s| !s.is_empty())
                                    .map(|s| format!(" — {s}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            format!(
                "{id}: {title}{}",
                if options.is_empty() {
                    String::new()
                } else {
                    format!("\n{options}")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Select display text, never reasoning, tool results, arbitrary JSON or a previous turn.
pub(super) fn parse(values: impl IntoIterator<Item = Value>, target: &Session) -> Excerpt {
    let mut out = Excerpt::default();
    let mut turn = String::new();
    let mut pending_call = String::new();
    let codex = matches!(target.session.provider.as_str(), "codex" | "chatgpt-work");
    for v in values {
        if record_session(&v).is_some_and(|id| id != target.session.native_session)
            || v["isSidechain"] == true
            || v["is_sidechain"] == true
            || v["agentId"].as_str().is_some_and(|id| !id.is_empty())
        {
            continue;
        }
        let at = time(&v);
        let p = v.get("payload").unwrap_or(&v);
        if codex
            && (v["type"] == "turn_context"
                || v["type"] == "event_msg" && p["type"] == "task_started")
        {
            turn = p["turn_id"].as_str().unwrap_or("").into();
        }
        let explicit_turn = p["turn_id"]
            .as_str()
            .or_else(|| p["turnId"].as_str())
            .unwrap_or(&turn);
        if codex && !target.session.turn_id.is_empty() && explicit_turn.is_empty() {
            continue;
        }
        if !target.session.turn_id.is_empty()
            && !explicit_turn.is_empty()
            && explicit_turn != target.session.turn_id
        {
            continue;
        }
        if target.session.turn_started_at > 0
            && (at == 0 || at < target.session.turn_started_at as i64)
        {
            continue;
        }
        if target
            .session
            .turn_ended_at
            .is_some_and(|end| at > 0 && at > end as i64)
        {
            continue;
        }
        if codex {
            if v["type"] == "retained_context" && p["questions"].is_array() {
                out.question = questions(&p["questions"]);
                pending_call = p["call_id"].as_str().unwrap_or("").into();
            }
            if v["type"] == "response_item"
                && p["type"] == "function_call"
                && p["name"]
                    .as_str()
                    .is_some_and(|n| n.ends_with("request_user_input"))
                && let Some(args) = p["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
            {
                out.question = questions(&args["questions"]);
                pending_call = p["call_id"].as_str().unwrap_or("").into();
            }
            if v["type"] == "response_item"
                && p["type"] == "function_call_output"
                && !pending_call.is_empty()
                && p["call_id"].as_str() == Some(&pending_call)
            {
                out.question.clear();
                pending_call.clear();
            }
            if v["type"] == "response_item" && p["type"] == "message" {
                if p["role"] == "assistant" && p["phase"] == "final" {
                    out.text = text_blocks(&p["content"]);
                }
                if p["role"] == "user" {
                    out.user_text = text_blocks(&p["content"]);
                    out.user_at = at;
                }
            }
            if v["type"] == "event_msg" && p["type"] == "user_message" {
                out.user_text = p["message"].as_str().unwrap_or("").into();
                out.user_at = at;
            }
            if v["type"] == "event_msg" && p["type"] == "agent_message" && p["phase"] == "final" {
                out.text = p["message"].as_str().unwrap_or("").into();
            }
        } else {
            let message = v.get("message").filter(|m| m.is_object()).unwrap_or(&v);
            let role = message["role"]
                .as_str()
                .or_else(|| v["type"].as_str())
                .unwrap_or("");
            if role == "assistant" {
                let text = text_blocks(&message["content"]);
                let tool = message["content"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|b| matches!(b["type"].as_str(), Some("tool_use" | "toolCall")))
                });
                if !text.is_empty()
                    && !tool
                    && !matches!(message["phase"].as_str(), Some("commentary" | "analysis"))
                {
                    out.text = text;
                }
                if let Some(blocks) = message["content"].as_array() {
                    for block in blocks {
                        if block["type"] == "tool_use" && block["name"] == "AskUserQuestion" {
                            out.question = questions(&block["input"]["questions"]);
                            pending_call = block["id"].as_str().unwrap_or("").into();
                        }
                    }
                }
            } else if role == "user" {
                let text = text_blocks(&message["content"]);
                if !text.is_empty() {
                    out.user_text = text;
                    out.user_at = at;
                }
                if message["content"].as_array().is_some_and(|a| {
                    a.iter().any(|b| {
                        b["type"] == "tool_result"
                            && b["tool_use_id"].as_str() == Some(&pending_call)
                    })
                }) {
                    out.question.clear();
                    pending_call.clear();
                }
            }
        }
    }
    out.text = clip(&out.text, 10000);
    out.question = clip(&out.question, 10000);
    out.user_text = clip(&out.user_text, MAX_INPUT);
    out
}

fn record_session(v: &Value) -> Option<&str> {
    if v["type"] == "session_meta" {
        return v["payload"]["id"].as_str();
    }
    if v["type"] == "session" {
        return v["id"].as_str();
    }
    v["sessionId"].as_str().or_else(|| v["session_id"].as_str())
}

/// Validate the root session before reading text. A matching filename is insufficient for Codex.
fn records(path: &Path, root: &Path, target: &Session) -> Result<(Vec<Value>, bool)> {
    let path = path.canonicalize().context("会话记录不可用")?;
    let root = root.canonicalize().context("会话来源目录不可用")?;
    ensure!(
        path.starts_with(&root) && path.is_file(),
        "会话记录不属于此来源"
    );
    let mut file = File::open(&path)?;
    let size = file.metadata()?.len();
    let mut header = Vec::new();
    (&mut file).take(65536).read_to_end(&mut header)?;
    let head: Vec<Value> = header
        .split_inclusive(|b| *b == b'\n')
        .filter(|l| l.ends_with(b"\n"))
        .filter_map(|l| serde_json::from_slice(l).ok())
        .collect();
    let identity = head.iter().find_map(record_session);
    let codex = matches!(target.session.provider.as_str(), "codex" | "chatgpt-work");
    let named = !codex
        && path.components().any(|p| {
            p.as_os_str().to_str().is_some_and(|name| {
                name == target.session.native_session
                    || name.strip_suffix(".jsonl") == Some(&target.session.native_session)
            })
        });
    ensure!(
        identity == Some(&target.session.native_session) || identity.is_none() && named,
        "会话记录身份不符"
    );
    let offset = size.saturating_sub(MAX_TAIL);
    file.seek(SeekFrom::Start(offset))?;
    let mut data = Vec::new();
    file.take(MAX_TAIL).read_to_end(&mut data)?;
    let data = if offset > 0 {
        data.splitn(2, |b| *b == b'\n').nth(1).unwrap_or_default()
    } else {
        &data
    };
    // A partial final JSONL record is not committed content.
    let records = data
        .split_inclusive(|b| *b == b'\n')
        .filter(|l| l.ends_with(b"\n"))
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .collect();
    Ok((records, data.ends_with(b"\n")))
}

fn paths(source: &Source, target: &Session) -> (PathBuf, Vec<String>) {
    let root = expand(&source.path);
    let root = if root.is_file() {
        root.parent().unwrap_or(&root).to_path_buf()
    } else {
        root
    };
    let mut paths = Vec::new();
    if !target.path.is_empty() {
        paths.push(target.path.clone());
    }
    for path in &target.log_paths {
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    (root, paths)
}

pub(super) fn local(source: &Source, target: &Session) -> Excerpt {
    let (root, paths) = paths(source, target);
    for path in paths.iter().take(8) {
        if let Ok((rows, _)) = records(Path::new(path), &root, target) {
            // Never append a child or a second transcript to this turn.
            let next = parse(rows, target);
            if !next.text.is_empty() || !next.question.is_empty() || next.user_at > 0 {
                return next;
            }
        }
    }
    Excerpt::default()
}

/// Queue has no compare-and-swap option. Require current, committed identity and turn evidence.
pub(super) fn verify_codex_record(source: &Source, target: &Session) -> Result<()> {
    ensure!(!target.session.turn_id.is_empty(), "原会话缺少轮次标识");
    let (root, paths) = paths(source, target);
    for path in paths.iter().take(8) {
        if let Ok((rows, complete)) = records(Path::new(path), &root, target) {
            ensure!(complete, "原会话记录尚未写入完成");
            let latest = rows
                .iter()
                .rev()
                .find(|v| {
                    v["type"] == "turn_context"
                        || v["type"] == "event_msg" && v["payload"]["type"] == "task_started"
                })
                .context("缺少原会话的当前轮次记录")?;
            ensure!(
                latest["payload"]["turn_id"].as_str() == Some(&target.session.turn_id),
                "原会话已进入其他轮次"
            );
            if let Some(workspace) = &target.session.workspace {
                let cwd = rows.iter().rev().find_map(|v| {
                    if matches!(v["type"].as_str(), Some("turn_context" | "session_meta")) {
                        v["payload"]["cwd"].as_str()
                    } else {
                        None
                    }
                });
                ensure!(cwd == Some(workspace), "原会话工作目录已改变或无法核对");
            }
            return Ok(());
        }
    }
    anyhow::bail!("原会话记录不可用")
}
