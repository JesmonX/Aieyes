//! Whole-file JSON task history: Cline CLI / new extension sessions
//! (`sessions/<id>/<id>.messages.json`), Roo Code tasks
//! (`tasks/<id>/history_item.json`) and trae-cli trajectories
//! (`trajectories/*.json`). Only token counters, model ids and timestamps are
//! read; prompts and replies never enter the Aieyes database.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
pub(crate) mod json_tasks;

/// A single task file larger than this is reported, not read.
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

pub(crate) fn applies(source: &Source) -> bool {
    // A Trae source never walks the IDE's data folder (its chats are encrypted):
    // without a trajectories directory it simply has no history.
    crate::subscriptions::trae::family(&source.provider)
        || matches!(source.provider.as_str(), "cline" | "roo" | "kilo")
            && !json_tasks::files(&source.provider, &expand(&source.path)).is_empty()
}

fn directory(provider: &str, root: &Path) -> Option<PathBuf> {
    let name = if provider == "cline" {
        "sessions"
    } else if crate::subscriptions::trae::family(provider) {
        "trajectories"
    } else {
        "tasks"
    };
    if root.file_name().is_some_and(|n| n == name) && root.is_dir() {
        return Some(root.to_owned());
    }
    Some(root.join(name)).filter(|p| p.is_dir())
}

/// Each task directory holds exactly one file Aieyes reads.
pub(crate) fn files(provider: &str, root: &Path) -> Vec<PathBuf> {
    if matches!(provider, "cline" | "roo" | "kilo") {
        return json_tasks::files(provider, root);
    }
    let Some(dir) = directory(provider, root) else {
        return vec![];
    };
    if crate::subscriptions::trae::family(provider) {
        // trae-cli writes one file per run (`trajectory_<time>.json`, or `-t`).
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().is_some_and(|e| e == "json")
                    && p.file_name()
                        .is_some_and(|n| !n.to_string_lossy().starts_with('.'))
            })
            .take(MAX_FILES)
            .collect();
        files.sort();
        return files;
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if id.starts_with('.') {
                return None;
            }
            let file = if provider == "cline" {
                e.path().join(format!("{id}.messages.json"))
            } else {
                e.path().join("history_item.json")
            };
            file.is_file().then_some(file)
        })
        .take(MAX_FILES)
        .collect();
    files.sort();
    files
}

pub(crate) fn read(path: &Path) -> Result<Value> {
    let meta = std::fs::metadata(path)?;
    ensure!(meta.len() <= MAX_FILE, "任务文件过大，已跳过");
    // Clients rewrite these files in place; a torn read is retried next scan.
    serde_json::from_slice(&std::fs::read(path)?).context("任务文件格式无效或正在写入")
}

fn count(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

/// A trajectory timestamp: trae-cli writes the machine's local time without an
/// offset (`datetime.now().isoformat()`).
fn local_time(value: &str) -> Option<i64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(t.timestamp());
    }
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()?
        .and_local_timezone(chrono::Local)
        .earliest()
        .map(|t| t.timestamp())
}

/// Normalized usage rows for one task file.
pub(crate) fn rows(provider: &str, v: &Value) -> Vec<Value> {
    if crate::subscriptions::trae::family(provider) {
        // One record per model call; `interactive` rewrites the file for each
        // task, and `start_time` names the task.
        let Some(task) = v["start_time"].as_str().filter(|s| !s.is_empty()) else {
            return vec![];
        };
        return v["llm_interactions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|call| {
                let at = call["timestamp"].as_str()?;
                let usage = &call["response"]["usage"];
                let model = call["model"]
                    .as_str()
                    .or(v["model"].as_str())
                    .filter(|s| !s.is_empty())?;
                Some(json!({"id":format!("call:{at}"),"sessionId":format!("trae:{task}"),
                    "timestamp":local_time(at)?,"model":model,
                    "tokens":{"input":count(&usage["input_tokens"]),"output":count(&usage["output_tokens"]),
                        "cacheRead":count(&usage["cache_read_input_tokens"]),
                        "cacheWrite":count(&usage["cache_creation_input_tokens"]),
                        "reasoning":count(&usage["reasoning_tokens"])}}))
            })
            .collect();
    }
    json_tasks::rows(provider, v, None).unwrap_or_default()
}

fn signature(path: &Path) -> String {
    std::fs::metadata(path)
        .map(|m| {
            format!(
                "task-json-v3:{}:{}",
                m.len(),
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|t| t.as_nanos())
                    .unwrap_or(0)
            )
        })
        .unwrap_or_default()
}

pub fn scan(store: &Store, settings: &Settings, source: &Source) -> Result<Value> {
    scan_mode(store, settings, source, false)
}

pub(crate) fn rescan(store: &Store, settings: &Settings, source: &Source) -> Result<Value> {
    scan_mode(store, settings, source, true)
}

fn scan_mode(store: &Store, settings: &Settings, source: &Source, full: bool) -> Result<Value> {
    let paths = files(&source.provider, &expand(&source.path));
    let overlapping = json_tasks::overlapping_legacy_tasks(&source.provider, &paths);
    let prices = store.prices()?;
    let (mut count, mut read_bytes, mut failed) = (0, 0u64, 0);
    let mut issues = Vec::new();
    for path in &paths {
        let key = path.to_string_lossy();
        let exclude_subagents = overlapping.contains(path);
        if exclude_subagents {
            issues.push(json!({"path":path.file_name().unwrap_or_default().to_string_lossy(),"code":"overlappingSubagentSummary",
                "message":"子代理汇总与独立后代记录并存；已统计可核实请求，未叠加无法逐条核对的汇总。"}));
        }
        let signature = format!(
            "{}:{}:{}:{}",
            signature(path),
            settings.precise_counting,
            settings.counting_generation,
            exclude_subagents
        );
        if !full
            && store
                .cursor(&source.id, &key)?
                .is_some_and(|(s, _, _)| s == signature)
        {
            continue;
        }
        let rows = match read(path).and_then(|value| {
            if matches!(source.provider.as_str(), "cline" | "roo" | "kilo") {
                json_tasks::rows_with_policy(
                    &source.provider,
                    &value,
                    Some(path),
                    exclude_subagents,
                )
            } else {
                Ok(rows(&source.provider, &value))
            }
        }) {
            Ok(v) => v,
            Err(error) => {
                failed += 1;
                issues.push(json!({"path":path.file_name().unwrap_or_default().to_string_lossy(),"code":"fileRead","message":error.to_string()}));
                continue;
            }
        };
        read_bytes += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let tx = store.write_transaction()?;
        let current = crate::import::current_settings(store, settings, source)?;
        for row in rows {
            if let Some(event) = usage::parse(source, &mut ParseState::default(), &row)
                && store.put_event(&event, &prices, &current)?
            {
                count += 1;
            }
        }
        store.set_cursor(&source.id, &key, &signature, 0, &ParseState::default())?;
        tx.commit()?;
    }
    let mut status = json!({"updatedAt":now(),"newEvents":count,"files":paths.len(),"readBytes":read_bytes,
        "partial":!issues.is_empty(),"issues":issues,"failedFiles":failed,"format":format!("{}-tasks",source.provider)});
    if failed > 0 && failed == paths.len() {
        status.as_object_mut().unwrap().remove("updatedAt");
        status["error"] = json!("任务文件读取失败");
        status["failedAt"] = json!(now());
    }
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &status)?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cline_messages_yield_one_request_per_assistant_reply_without_content() {
        // Shape written by Cline CLI 3.0.74 (`cline --json`).
        let v = json!({"version":1,"sessionId":"1793319873210_b0tf2","messages":[
            {"id":"u","role":"user","content":[{"type":"text","text":"PRIVATE"}],"ts":1793319873305_i64},
            {"id":"msg_diDuNU_V","role":"assistant","content":[{"type":"text","text":"PRIVATE"}],"ts":1793319874086_i64,
             "modelInfo":{"id":"Qwen3.8-27B","provider":"openai-compatible"},
             "metrics":{"inputTokens":5547,"outputTokens":23,"cacheReadTokens":0,"cacheWriteTokens":0}}]});
        let rows = rows("cline", &v);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "msg:msg_diDuNU_V");
        assert_eq!(rows[0]["timestamp"], 1793319874);
        assert_eq!(rows[0]["tokens"]["input"], 5547);
        assert!(!rows[0].to_string().contains("PRIVATE"));
    }

    #[test]
    fn roo_history_item_is_one_cumulative_task_record() {
        // Fields per Roo Code 3.54.0 `historyItemSchema`.
        let v = json!({"id":"child","rootTaskId":"root","parentTaskId":"root","number":2,"ts":1793319874086_i64,
            "task":"PRIVATE","tokensIn":10,"tokensOut":5,"cacheReads":3,"totalCost":0.1,"apiConfigName":"default"});
        let rows = rows("roo", &v);
        assert_eq!(rows[0]["sessionId"], "root");
        assert_eq!(rows[0]["model"], "unknown");
        assert_eq!(rows[0]["tokens"]["cacheWrite"], 0);
        assert!(!rows[0].to_string().contains("PRIVATE"));
        assert!(super::rows("roo", &json!({"ts":1})).is_empty());
    }

    #[test]
    fn trae_trajectory_yields_one_request_per_model_call_without_content() {
        // Shape written by trae-agent 0.1.0 (`trae-cli run`, Qwen3.8-27B over an
        // OpenAI-compatible endpoint): naive local timestamps.
        let call = |at: &str, input: u64| {
            json!({"timestamp":at,"provider":"openai","model":"Qwen3.8-27B",
                "input_messages":[{"role":"user","content":"PRIVATE"}],
                "response":{"content":"PRIVATE","model":"Qwen3.8-27B","finish_reason":"completed",
                    "usage":{"input_tokens":input,"output_tokens":132,"cache_creation_input_tokens":0,"cache_read_input_tokens":null,"reasoning_tokens":7}}})
        };
        let v = json!({"task":"PRIVATE","start_time":"2026-10-31T01:24:38.728865","end_time":"2026-10-31T01:24:43.497592",
            "provider":"openai","model":"Qwen3.8-27B","max_steps":4,
            "llm_interactions":[call("2026-10-31T01:24:40.217263",3025),call("2026-10-31T01:24:41.100000",3200),{"timestamp":"bad"}],
            "agent_steps":[{"state":"completed"}],"success":true,"final_result":"PRIVATE"});
        let rows = rows("trae", &v);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "call:2026-10-31T01:24:40.217263");
        assert_eq!(rows[0]["sessionId"], "trae:2026-10-31T01:24:38.728865");
        assert_eq!(rows[0]["model"], "Qwen3.8-27B");
        assert_eq!(rows[0]["tokens"]["input"], 3025);
        assert_eq!(rows[0]["tokens"]["cacheRead"], 0);
        assert_eq!(rows[0]["tokens"]["reasoning"], 7);
        let local =
            chrono::NaiveDateTime::parse_from_str("2026-10-31T01:24:40", "%Y-%m-%dT%H:%M:%S")
                .unwrap()
                .and_local_timezone(chrono::Local)
                .earliest()
                .unwrap()
                .timestamp();
        assert_eq!(rows[0]["timestamp"], local);
        assert!(!serde_json::to_string(&rows).unwrap().contains("PRIVATE"));
        assert!(
            super::rows(
                "trae-cn",
                &json!({"llm_interactions":[call("2026-10-31T01:24:40",1)]})
            )
            .is_empty()
        );
    }

    #[test]
    fn task_files_are_found_per_task_directory() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("sessions/abc");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("abc.messages.json"), "{}").unwrap();
        std::fs::write(session.join("abc.json"), "{}").unwrap();
        let tasks = temp.path().join("tasks/t1");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(tasks.join("history_item.json"), "{}").unwrap();
        std::fs::write(temp.path().join("tasks/_index.json"), "{}").unwrap();
        assert_eq!(
            files("cline", temp.path()),
            [
                session.join("abc.messages.json"),
                tasks.join("history_item.json")
            ]
        );
        assert_eq!(files("roo", temp.path()), [tasks.join("history_item.json")]);
        // trae-cli: run files directly in `trajectories/`; Trae sources never fall
        // back to walking the IDE's data folder.
        let trajectories = temp.path().join("trajectories");
        std::fs::create_dir_all(trajectories.join("nested")).unwrap();
        std::fs::write(trajectories.join("trajectory_20261031_012437.json"), "{}").unwrap();
        std::fs::write(trajectories.join("notes.txt"), "").unwrap();
        assert_eq!(
            files("trae", temp.path()),
            [trajectories.join("trajectory_20261031_012437.json")]
        );
        assert_eq!(files("trae", &trajectories).len(), 1);
        let source = |path: &Path| Source {
            provider: "trae".into(),
            path: path.to_string_lossy().into(),
            ..Default::default()
        };
        assert!(applies(&source(&temp.path().join("missing"))));
        assert!(files("trae", &temp.path().join("missing")).is_empty());
    }
}
