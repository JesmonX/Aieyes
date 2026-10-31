//! Cline's current session envelopes and the Cline/Roo extension task format.
//! Keep the projection deliberately small: message text is only decoded for the
//! extension's JSON usage counters, never retained in a row or cursor.
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

const LIMIT: usize = 20_000;

pub(crate) fn directories(provider: &str, root: &Path) -> Vec<PathBuf> {
    if !matches!(provider, "cline" | "roo" | "kilo") || root.is_file() {
        return vec![];
    }
    let names: &[&str] = if provider == "cline" {
        &["sessions", "tasks"]
    } else {
        &["tasks"]
    };
    let mut dirs = vec![root.to_owned()];
    for base in [root.to_owned(), root.join("data")] {
        for name in names {
            dirs.push(base.join(name));
        }
    }
    dirs.retain(|p| p.is_dir());
    dirs.sort();
    dirs.dedup();
    dirs
}

pub(crate) fn files(provider: &str, root: &Path) -> Vec<PathBuf> {
    let supported = |p: &Path| {
        p.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name == "history_item.json"
                || name == "ui_messages.json"
                || (provider == "cline" && name.ends_with(".messages.json"))
        })
    };
    if root.is_file() && supported(root) {
        return vec![root.to_owned()];
    }
    let mut files = Vec::new();
    // The index records absolute transcript paths even when the session and
    // database directory overrides point to different locations.
    if provider == "cline"
        && let Some(db_path) = root.to_str().and_then(crate::sessions::cline_database)
        && let Ok(db) = crate::sqlite_ro::open_ro(&db_path)
        && let Ok(mut query) = db.prepare(
            "SELECT messages_path FROM sessions WHERE messages_path IS NOT NULL LIMIT 20000",
        )
        && let Ok(paths) = query.query_map([], |r| r.get::<_, String>(0))
    {
        files.extend(
            paths
                .filter_map(|p| p.ok())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute() && p.is_file() && supported(p)),
        );
    }
    for dir in directories(provider, root) {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_ok_and(|t| t.is_dir())
                    && !e.file_name().to_string_lossy().starts_with('.')
            })
            .map(|e| e.path())
            .collect();
        children.push(dir);
        children.sort();
        for child in children {
            let mut messages: Vec<_> = if provider == "cline" {
                std::fs::read_dir(&child)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().ends_with(".messages.json"))
                    })
                    .collect()
            } else {
                vec![]
            };
            if !messages.is_empty() {
                messages.sort();
                files.extend(messages);
            }
            // A summary includes ui_messages.json, including compacted requests.
            else if let Some(file) = ["history_item.json", "ui_messages.json"]
                .iter()
                .map(|name| child.join(name))
                .find(|p| p.is_file())
            {
                files.push(file);
            }
            if files.len() >= LIMIT {
                files.sort();
                files.dedup();
                files.truncate(LIMIT);
                return files;
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty() && s.len() <= 4096)
}

fn counter(value: &Value) -> Result<u64> {
    if value.is_null() {
        return Ok(0);
    }
    value
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("任务 Token 计数无效"))
}

fn token_row(value: &Value, names: [&str; 4]) -> Result<Value> {
    let values = names
        .map(|name| counter(&value[name]))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        values
            .iter()
            .try_fold(0u64, |sum, n| sum.checked_add(*n))
            .is_some(),
        "任务 Token 计数溢出"
    );
    Ok(json!({"input":values[0],"output":values[1],"cacheRead":values[2],"cacheWrite":values[3]}))
}

fn task_id(path: Option<&Path>) -> Option<&str> {
    path?
        .parent()?
        .file_name()?
        .to_str()
        .filter(|s| !s.is_empty())
}

/// A legacy batch has no child request IDs. If explicitly related child files
/// coexist, their counters are usable but the batch cannot safely be added.
pub(crate) fn overlapping_legacy_tasks(provider: &str, paths: &[PathBuf]) -> HashSet<PathBuf> {
    if provider != "cline" {
        return HashSet::new();
    }
    let legacy: HashMap<_, _> = paths
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == "ui_messages.json")
        })
        .filter_map(|path| Some((task_id(Some(path))?.to_owned(), path.clone())))
        .collect();
    if legacy.is_empty() {
        return HashSet::new();
    }
    let mut parents = HashMap::new();
    for path in paths.iter().filter(|path| {
        !path
            .file_name()
            .is_some_and(|name| name == "ui_messages.json")
    }) {
        let companion = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".messages.json"))
            .map(|name| path.with_file_name(format!("{name}.json")))
            .filter(|path| path.is_file());
        let Ok(value) = super::read(companion.as_deref().unwrap_or(path)) else {
            continue;
        };
        let id = text(&value["session_id"])
            .or_else(|| text(&value["sessionId"]))
            .or_else(|| text(&value["id"]));
        let parent = text(&value["parent_session_id"])
            .or_else(|| text(&value["origin"]["parentThreadId"]))
            .or_else(|| text(&value["parentTaskId"]))
            .or_else(|| text(&value["rootTaskId"]));
        if let (Some(id), Some(parent)) = (id, parent) {
            parents.insert(id.to_owned(), parent.to_owned());
        }
    }
    let mut related = HashSet::new();
    for parent in parents.values() {
        let mut current = parent.as_str();
        let mut seen = HashSet::new();
        for _ in 0..32 {
            if !seen.insert(current) {
                break;
            }
            if let Some(path) = legacy.get(current) {
                related.insert(path.clone());
            }
            let Some(parent) = parents.get(current) else {
                break;
            };
            current = parent;
        }
    }
    related
        .into_iter()
        .filter(|path| {
            super::read(path).is_ok_and(|value| {
                value.as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["type"] == "say" && message["say"] == "subagent_usage"
                    })
                })
            })
        })
        .collect()
}

fn summary(value: &Value) -> Result<Vec<Value>> {
    let id = text(&value["id"]).ok_or_else(|| anyhow::anyhow!("任务 ID 缺失"))?;
    let ts = value["ts"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow::anyhow!("任务时间无效"))?;
    ensure!(
        value["tokensIn"].is_number() && value["tokensOut"].is_number(),
        "任务用量缺失"
    );
    // apiConfigName identifies a profile, never a model.
    let session = text(&value["rootTaskId"])
        .or_else(|| text(&value["parentTaskId"]))
        .unwrap_or(id);
    Ok(vec![
        json!({"id":format!("task:{id}"),"sessionId":session,"timestamp":ts/1000,"model":"unknown",
        "tokens":token_row(value,["tokensIn","tokensOut","cacheReads","cacheWrites"])?}),
    ])
}

fn cline(value: &Value) -> Result<Vec<Value>> {
    let session = text(&value["sessionId"]).ok_or_else(|| anyhow::anyhow!("Cline 会话 ID 缺失"))?;
    let messages = value["messages"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Cline 会话格式无效"))?;
    let mut rows = Vec::new();
    for m in messages
        .iter()
        .filter(|m| m["role"] == "assistant" && m["metrics"].is_object())
    {
        let id = text(&m["id"]).ok_or_else(|| anyhow::anyhow!("Cline 请求 ID 缺失"))?;
        let ts = m["ts"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(|| anyhow::anyhow!("Cline 请求时间无效"))?;
        let model = text(&m["modelInfo"]["id"]).unwrap_or("unknown");
        rows.push(json!({"id":format!("msg:{id}"),"sessionId":session,"timestamp":ts/1000,"model":model,
            "tokens":token_row(&m["metrics"],["inputTokens","outputTokens","cacheReadTokens","cacheWriteTokens"])?,
            // Native message IDs and timestamps survive copied transcripts.
            "inheritance":{"origin":format!("cline-message:{id}:{ts}"),"inherited":false}}));
    }
    Ok(rows)
}

fn extension(
    messages: &[Value],
    path: Option<&Path>,
    exclude_subagents: bool,
) -> Result<Vec<Value>> {
    let id = task_id(path).ok_or_else(|| anyhow::anyhow!("扩展任务目录缺失"))?;
    let mut usage = Vec::new();
    let mut current = None;
    let mut suppressed_at = 0;
    for message in messages {
        if message["type"] != "say" {
            continue;
        }
        let kind = message["say"].as_str().unwrap_or("");
        if ![
            "api_req_started",
            "api_req_finished",
            "deleted_api_reqs",
            "subagent_usage",
        ]
        .contains(&kind)
        {
            continue;
        }
        let Some(raw) = message["text"].as_str() else {
            continue;
        };
        let data: Value =
            serde_json::from_str(raw).map_err(|_| anyhow::anyhow!("扩展任务用量格式无效"))?;
        ensure!(data.is_object(), "扩展任务用量格式无效");
        if kind == "api_req_finished" {
            if let Some(index) = current.take() {
                let (value, _, _): &mut (Value, i64, String) = &mut usage[index];
                value
                    .as_object_mut()
                    .unwrap()
                    .extend(data.as_object().unwrap().clone());
            }
            continue;
        }
        let ts = message["ts"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(|| anyhow::anyhow!("扩展任务时间无效"))?;
        if exclude_subagents && kind == "subagent_usage" {
            suppressed_at = suppressed_at.max(ts);
            current = None;
            continue;
        }
        let model = if kind == "api_req_started" {
            text(&message["modelInfo"]["modelId"]).unwrap_or("unknown")
        } else {
            "unknown"
        };
        current = (kind == "api_req_started").then_some(usage.len());
        usage.push((data, ts, model.to_owned()));
    }
    let mut totals = [0u64; 4];
    let mut models = BTreeSet::new();
    let mut at = suppressed_at;
    for (data, ts, model) in usage {
        // A placeholder api_req_started is not a zero-token request.
        if !["tokensIn", "tokensOut", "cacheReads", "cacheWrites"]
            .iter()
            .any(|key| data.get(key).is_some())
        {
            continue;
        }
        let row = token_row(
            &data,
            ["tokensIn", "tokensOut", "cacheReads", "cacheWrites"],
        )?;
        for (i, key) in ["input", "output", "cacheRead", "cacheWrite"]
            .iter()
            .enumerate()
        {
            totals[i] = totals[i]
                .checked_add(row[key].as_u64().unwrap())
                .ok_or_else(|| anyhow::anyhow!("任务 Token 计数溢出"))?;
        }
        at = at.max(ts);
        models.insert(model);
    }
    if at == 0 {
        return Ok(vec![]);
    }
    ensure!(
        totals
            .iter()
            .try_fold(0u64, |sum, n| sum.checked_add(*n))
            .is_some(),
        "任务 Token 计数溢出"
    );
    let model = if models.len() == 1 {
        models.into_iter().next().unwrap()
    } else {
        "unknown".into()
    };
    // Compaction replaces old request rows with deleted_api_reqs. A stable,
    // cumulative task row replaces prior totals instead of charging both.
    Ok(vec![
        json!({"id":format!("task:{id}"),"sessionId":id,"timestamp":at/1000,"model":model,
        "tokens":{"input":totals[0],"output":totals[1],"cacheRead":totals[2],"cacheWrite":totals[3]}}),
    ])
}

pub(crate) fn rows(provider: &str, value: &Value, path: Option<&Path>) -> Result<Vec<Value>> {
    rows_with_policy(provider, value, path, false)
}

pub(crate) fn rows_with_policy(
    provider: &str,
    value: &Value,
    path: Option<&Path>,
    exclude_subagents: bool,
) -> Result<Vec<Value>> {
    if let Some(messages) = value.as_array() {
        let mut rows = extension(messages, path, exclude_subagents)?;
        if provider == "cline" {
            for row in &mut rows {
                row["clineTaskUsageVersion"] = json!(if exclude_subagents { 3 } else { 2 });
            }
        }
        return Ok(rows);
    }
    if provider == "cline" && value["messages"].is_array() {
        return cline(value);
    }
    if value.is_object() && value.get("tokensIn").is_some() {
        return summary(value);
    }
    bail!("不支持此任务文件格式")
}

pub(crate) fn phase(value: &Value) -> &'static str {
    if let Some(status) = value["status"].as_str() {
        return match status {
            "idle" | "completed" => "complete",
            "running" | "active" | "delegated" => "start",
            "pending" => "PermissionRequest",
            "failed" => "failed",
            "cancelled" => "interrupted",
            _ => "unknown",
        };
    }
    let Some(messages) = value.as_array().or_else(|| value["messages"].as_array()) else {
        return "unknown";
    };
    for message in messages.iter().rev() {
        let ask = message["ask"].as_str().unwrap_or("");
        if ["completion_result", "resume_completed_task"].contains(&ask) {
            return "complete";
        }
        if ask == "api_req_failed" {
            return "failed";
        }
        if [
            "followup",
            "plan_mode_respond",
            "act_mode_respond",
            "command",
            "tool",
            "use_mcp_server",
        ]
        .contains(&ask)
        {
            return "PermissionRequest";
        }
        match message["say"].as_str().unwrap_or("") {
            "completion_result" | "api_req_finished" => return "complete",
            "error" => return "failed",
            "api_req_started" => {
                let data: Value = serde_json::from_str(message["text"].as_str().unwrap_or("{}"))
                    .unwrap_or_default();
                return match data["cancelReason"].as_str() {
                    Some("user_cancelled") => "interrupted",
                    Some(_) => "failed",
                    None if data.get("cost").is_some() => "complete",
                    None => "start",
                };
            }
            "tool" | "command" => return "tool",
            _ => {}
        }
        match message["role"].as_str() {
            Some("user") => return "start",
            Some("assistant") if message["metrics"].is_object() => return "complete",
            _ => {}
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_usage_combines_old_pairs_and_replaces_compacted_totals() {
        let path = Path::new("tasks/task-one/ui_messages.json");
        let m = |kind: &str, value: Value, ts| json!({"type":"say","say":kind,"ts":ts,"text":value.to_string(),"modelInfo":{"modelId":"real-model"}});
        let rows = rows(
            "cline",
            &json!([
                m("api_req_started", json!({"request":"PRIVATE"}), 1000),
                m(
                    "api_req_finished",
                    json!({"tokensIn":10,"tokensOut":3,"cacheReads":2}),
                    1001
                ),
                m(
                    "api_req_started",
                    json!({"tokensIn":20,"tokensOut":4}),
                    2000
                )
            ]),
            Some(path),
        )
        .unwrap();
        assert_eq!(rows[0]["tokens"]["input"], 30);
        assert_eq!(rows[0]["model"], "real-model");
        let compacted = super::rows(
            "cline",
            &json!([
                m(
                    "deleted_api_reqs",
                    json!({"tokensIn":10,"tokensOut":3,"cacheReads":2}),
                    1000
                ),
                m(
                    "api_req_started",
                    json!({"tokensIn":20,"tokensOut":4}),
                    2000
                )
            ]),
            Some(path),
        )
        .unwrap();
        assert_eq!(compacted[0]["id"], rows[0]["id"]);
        assert_eq!(compacted[0]["tokens"], rows[0]["tokens"]);
        assert_eq!(compacted[0]["model"], "unknown");
        assert!(!rows[0].to_string().contains("PRIVATE"));
    }

    #[test]
    fn finds_cline_subagents_and_one_extension_record_per_task() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for name in [
            "sessions/root/root.messages.json",
            "sessions/root/agent.messages.json",
            "sessions/root/agent__task.messages.json",
            "tasks/task/ui_messages.json",
            "tasks/task/history_item.json",
        ] {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "{}").unwrap();
        }
        let paths = files("cline", root);
        assert_eq!(paths.len(), 4);
        assert!(!paths.iter().any(|p| p.ends_with("ui_messages.json")));
        assert_eq!(files("kilo", root).len(), 1);
    }

    #[test]
    fn summary_parent_fallback_never_uses_profile_as_model() {
        let rows = rows("roo",&json!({"id":"child","parentTaskId":"parent","ts":1000,"tokensIn":1,"tokensOut":2,"apiConfigName":"PRIVATE"}),None).unwrap();
        assert_eq!(rows[0]["sessionId"], "parent");
        assert_eq!(rows[0]["model"], "unknown");
        assert!(!rows[0].to_string().contains("PRIVATE"));
    }
}
