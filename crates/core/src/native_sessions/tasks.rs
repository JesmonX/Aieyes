//! Metadata-only live state for Cline and legacy extension task files.
use super::Activity;
use crate::task_history::{json_tasks, read};
use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone)]
pub(crate) struct Row {
    id: String,
    parent: String,
    kind: String,
    at: u64,
    interactive: Option<bool>,
    source: String,
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .filter(|s| s.len() <= 4096)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn directory(provider: &str, path: &str) -> Option<PathBuf> {
    let root = crate::store::expand(path);
    (!json_tasks::files(provider, &root).is_empty()).then_some(root)
}

fn manifest(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?.strip_suffix(".messages.json")?;
    Some(path.with_file_name(format!("{name}.json"))).filter(|p| p.is_file())
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|t| t.as_millis() as u64)
        .unwrap_or(0)
}

fn recent(files: &[PathBuf], now: u64) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<_> = files
        .iter()
        .cloned()
        .map(|p| {
            let stamp = modified(&p).max(manifest(&p).map(|m| modified(&m)).unwrap_or(0));
            (p, stamp)
        })
        .filter(|(_, at)| now.saturating_sub(at / 1000) < 86400)
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    files.truncate(256);
    files
}

fn metadata_paths(provider: &str, files: &[PathBuf]) -> Vec<PathBuf> {
    if provider != "cline" {
        return files
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|name| name == "history_item.json")
            })
            .cloned()
            .collect();
    }
    // Cline puts an entire agent tree in one session directory. Agent IDs do
    // not necessarily match filenames, and a parent transcript can be pruned
    // while its small companion manifest remains.
    let directories: BTreeSet<_> = files.iter().filter_map(|p| p.parent()).collect();
    let mut paths = Vec::new();
    for dir in directories {
        let mut next: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .filter(|p| {
                p.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.ends_with(".json")
                        && !name.ends_with(".messages.json")
                        && !matches!(
                            name.as_ref(),
                            "ui_messages.json" | "api_conversation_history.json"
                        )
                })
            })
            .collect();
        next.sort();
        paths.extend(next.into_iter().take(20_000 - paths.len()));
        if paths.len() == 20_000 {
            break;
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn fingerprint(provider: &str, path: &Path) -> String {
    let found = json_tasks::files(provider, path);
    let files = recent(&found, crate::models::now() as u64);
    let mut stamps = Vec::new();
    for p in files
        .iter()
        .map(|(p, _)| p.clone())
        .chain(metadata_paths(provider, &found))
    {
        if let Ok(meta) = std::fs::metadata(&p) {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            stamps.push(format!("{}:{}:{modified}", p.display(), meta.len()));
        }
    }
    stamps.sort();
    format!(
        "{provider}:{}:{}",
        files.len(),
        crate::models::hash(&stamps.join("\n"))
    )
}

fn stamp(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis().max(0) as u64)
}

fn decode(provider: &str, path: &Path, fallback_at: u64) -> Result<Row> {
    let metadata = if provider == "cline" {
        manifest(path)
    } else {
        None
    };
    let value = read(metadata.as_deref().unwrap_or(path))?;
    decode_value(provider, path, &value, fallback_at)
}

fn decode_value(provider: &str, path: &Path, value: &Value, fallback_at: u64) -> Result<Row> {
    let current =
        provider == "cline" && (value["session_id"].is_string() || value["sessionId"].is_string());
    let id = if current {
        text(&value["session_id"]).or_else(|| text(&value["sessionId"]))
    } else {
        text(&value["id"]).or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned()
        })
    };
    anyhow::ensure!(!id.is_empty(), "任务会话 ID 缺失");
    anyhow::ensure!(
        current || value.is_array() || value["tokensIn"].is_number(),
        "任务会话格式无效"
    );
    let parent = if current {
        text(&value["parent_session_id"]).or_else(|| text(&value["origin"]["parentThreadId"]))
    } else {
        text(&value["parentTaskId"]).or_else(|| text(&value["rootTaskId"]))
    };
    let at = if current {
        stamp(&value["updated_at"])
            .or_else(|| stamp(&value["ended_at"]))
            .unwrap_or(fallback_at)
    } else if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(|m| m["ts"].as_u64())
            .max()
            .unwrap_or(fallback_at)
    } else {
        value["ts"].as_u64().unwrap_or(fallback_at)
    };
    Ok(Row {
        id,
        parent,
        kind: json_tasks::phase(value).into(),
        at,
        interactive: current.then(|| value["interactive"].as_bool()).flatten(),
        source: if current {
            text(&value["source"]).or_else(|| text(&value["origin"]["source"]))
        } else {
            String::new()
        },
    })
}

fn add_ancestors(provider: &str, files: &[PathBuf], all: &mut HashMap<String, Row>) {
    let mut wanted: HashSet<_> = all
        .values()
        .map(|row| row.parent.clone())
        .filter(|id| !id.is_empty() && !all.contains_key(id))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let mut paths = metadata_paths(provider, files);
    // Usually a parent ID is its manifest name. Prefer those paths, then use a
    // bounded manifest index for agent/task names that differ from session IDs.
    paths.sort_by_key(|path| {
        !path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|id| wanted.contains(id))
    });
    for path in paths {
        if !std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= 1024 * 1024) {
            continue;
        }
        let Ok(value) = read(&path) else {
            continue;
        };
        if provider == "cline"
            && !(value["session_id"].is_string() || value["sessionId"].is_string())
        {
            continue;
        }
        let Ok(row) = decode_value(provider, &path, &value, modified(&path)) else {
            continue;
        };
        let id = row.id.clone();
        all.entry(id.clone()).or_insert(row);
        if wanted.remove(&id) {
            let mut current = all.get(&id).unwrap();
            let mut seen = HashSet::new();
            for _ in 0..32 {
                if current.parent.is_empty() || !seen.insert(current.id.as_str()) {
                    break;
                }
                if let Some(parent) = all.get(&current.parent) {
                    current = parent;
                } else {
                    wanted.insert(current.parent.clone());
                    break;
                }
            }
        }
        if wanted.is_empty() {
            break;
        }
    }
}

trait EmptyFallback {
    fn or_else(self, fallback: impl FnOnce() -> String) -> String;
}
impl EmptyFallback for String {
    fn or_else(self, fallback: impl FnOnce() -> String) -> String {
        if self.is_empty() { fallback() } else { self }
    }
}

fn excluded(row: &Row, all: &HashMap<String, Row>) -> bool {
    let mut current = row;
    let mut seen = HashSet::new();
    for _ in 0..32 {
        if !seen.insert(current.id.as_str()) {
            return false;
        }
        if current.parent.is_empty() {
            return current.interactive == Some(false) && current.source != "subagent";
        }
        let Some(parent) = all.get(&current.parent) else {
            return false;
        };
        current = parent;
    }
    false
}

pub(crate) fn refresh(
    provider: &str,
    path: &Path,
    saved: &mut HashMap<PathBuf, Row>,
    now: u64,
) -> Vec<Activity> {
    let found = json_tasks::files(provider, path);
    let files = recent(&found, now);
    let paths: HashSet<_> = files.iter().map(|(p, _)| p.clone()).collect();
    saved.retain(|path, _| paths.contains(path));
    for (path, at) in files {
        // Preserve the last good metadata while a client rewrites a JSON file.
        if let Ok(row) = decode(provider, &path, at) {
            saved.insert(path, row);
        }
    }
    let mut all: HashMap<_, _> = saved
        .values()
        .map(|row| (row.id.clone(), row.clone()))
        .collect();
    add_ancestors(provider, &found, &mut all);
    activities(saved.values(), &all, now)
}

fn activities<'a>(
    candidates: impl Iterator<Item = &'a Row>,
    all: &HashMap<String, Row>,
    now: u64,
) -> Vec<Activity> {
    let mut rows: Vec<_> = candidates
        .filter(|row| now.saturating_sub(row.at / 1000) < 86400 && !excluded(row, all))
        .map(|row| {
            (
                row.id.clone(),
                row.id.clone(),
                row.kind.clone(),
                row.at,
                row.parent.clone(),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(&b.0)));
    rows.truncate(256);
    rows
}

pub(crate) fn cline(db: &rusqlite::Connection, now: u64) -> Result<Vec<Activity>> {
    let since = chrono::DateTime::from_timestamp(now.saturating_sub(86400) as i64, 0)
        .unwrap_or_default()
        .to_rfc3339();
    let columns = "session_id,status,updated_at,COALESCE(parent_session_id,''),interactive,source";
    let decode = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
        let status: String = r.get(1)?;
        let at: String = r.get(2)?;
        Ok(Row {
            id: r.get(0)?,
            parent: r.get(3)?,
            interactive: r.get(4)?,
            source: r.get(5)?,
            kind: json_tasks::phase(&serde_json::json!({"status":status})).into(),
            at: chrono::DateTime::parse_from_rfc3339(&at)
                .map(|t| t.timestamp_millis().max(0) as u64)
                .unwrap_or(0),
        })
    };
    let mut all: HashMap<_, _> = db
        .prepare(&format!(
            "SELECT {columns} FROM sessions WHERE updated_at>?1 ORDER BY updated_at DESC LIMIT 2048"
        ))?
        .query_map([since], decode)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut parents: Vec<_> = all
        .values()
        .map(|r| r.parent.clone())
        .filter(|p| !p.is_empty())
        .collect();
    let mut query = db.prepare(&format!(
        "SELECT {columns} FROM sessions WHERE session_id=?1"
    ))?;
    let mut seen = HashSet::new();
    for _ in 0..32 {
        let mut next = Vec::new();
        for id in parents {
            if all.contains_key(&id) || !seen.insert(id.clone()) {
                continue;
            }
            if let Some(row) = query.query_row([id], decode).optional()? {
                if !row.parent.is_empty() {
                    next.push(row.parent.clone());
                }
                all.insert(row.id.clone(), row);
            }
        }
        if next.is_empty() {
            break;
        }
        parents = next;
    }
    Ok(activities(all.values(), &all, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn legacy_tasks_include_unknown_state_and_preserve_torn_updates() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("tasks/one/ui_messages.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let now = crate::models::now() as u64;
        std::fs::write(
            &file,
            json!([{"type":"say","say":"unexpected","ts":now*1000,"text":"PRIVATE"}]).to_string(),
        )
        .unwrap();
        let mut saved = HashMap::new();
        let rows = refresh("cline", temp.path(), &mut saved, now);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "unknown");
        std::fs::write(&file, "[").unwrap();
        assert_eq!(refresh("cline", temp.path(), &mut saved, now), rows);
    }
    #[test]
    fn file_lineage_loads_old_and_overflow_parent_manifests_without_transcripts() {
        for overflow in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("sessions/tree");
            std::fs::create_dir_all(&root).unwrap();
            let now = crate::models::now() as u64;
            let old = now - if overflow { 100 } else { 90_001 };
            let write = |name: &str,
                         id: &str,
                         parent: &str,
                         interactive: bool,
                         at: u64,
                         transcript: bool| {
                let stamp = chrono::DateTime::from_timestamp(at as i64, 0)
                    .unwrap()
                    .to_rfc3339();
                let path = root.join(format!("{name}.json"));
                std::fs::write(&path,json!({"session_id":id,"parent_session_id":parent,"interactive":interactive,"source":if parent.is_empty(){"cli"}else{"subagent"},"status":"running","updated_at":stamp,"prompt":"PRIVATE"}).to_string()).unwrap();
                let time = std::fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(at));
                std::fs::File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_times(time)
                    .unwrap();
                if transcript {
                    let path = root.join(format!("{name}.messages.json"));
                    // Session inspection must read the manifest, not parse this
                    // deliberately malformed transcript, even for ancestors.
                    std::fs::write(&path, "PRIVATE not JSON").unwrap();
                    std::fs::File::options()
                        .write(true)
                        .open(path)
                        .unwrap()
                        .set_times(time)
                        .unwrap();
                }
            };
            write("z-lead-record", "lead", "", false, old, !overflow);
            write("a-worker__job", "middle", "lead", false, old, true);
            write("descendant", "child", "middle", false, now, true);
            write("orphan", "orphan", "missing", false, now, true);
            if overflow {
                for index in 0..254 {
                    let id = format!("filler-{index:03}");
                    write(&id, &id, "", true, now, true);
                }
            }
            let mut saved = HashMap::new();
            let before = fingerprint("cline", temp.path());
            let rows = refresh("cline", temp.path(), &mut saved, now);
            assert!(rows.iter().any(|row| row.0 == "orphan"));
            assert!(
                !rows
                    .iter()
                    .any(|row| matches!(row.0.as_str(), "lead" | "middle" | "child"))
            );
            write("z-lead-record", "lead", "", true, old, !overflow);
            assert_ne!(before, fingerprint("cline", temp.path()));
            let rows = refresh("cline", temp.path(), &mut saved, now);
            assert!(rows.iter().any(|row| row.0 == "child"));
            assert!(
                !rows
                    .iter()
                    .any(|row| matches!(row.0.as_str(), "lead" | "middle"))
            );
        }
    }
    #[test]
    fn recursive_cline_lineage_excludes_headless_descendants_and_counts_orphans() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sessions(session_id TEXT,status TEXT,updated_at TEXT,parent_session_id TEXT,interactive INTEGER,source TEXT)").unwrap();
        let now = crate::models::now() as u64;
        let at = chrono::DateTime::from_timestamp(now as i64, 0)
            .unwrap()
            .to_rfc3339();
        for (id, parent, interactive, status) in [
            ("root", "", 1, "idle"),
            ("child", "root", 0, "running"),
            ("grandchild", "child", 0, "pending"),
            ("run", "", 0, "completed"),
            ("run-child", "run", 0, "running"),
            ("orphan", "missing", 0, "running"),
        ] {
            db.execute(
                "INSERT INTO sessions VALUES(?1,?2,?3,?4,?5,'cli')",
                rusqlite::params![id, status, at, parent, interactive],
            )
            .unwrap();
        }
        let rows = cline(&db, now).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(
            rows.iter()
                .any(|r| r.0 == "grandchild" && r.2 == "PermissionRequest")
        );
        assert!(rows.iter().any(|r| r.0 == "root" && r.2 == "complete"));
        assert!(rows.iter().any(|r| r.0 == "orphan"));
    }
    #[test]
    fn fingerprint_detects_a_non_newest_file_changed_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let now = crate::models::now() as u64;
        for id in ["a", "b"] {
            let file = temp.path().join(format!("tasks/{id}/history_item.json"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                &file,
                json!({"id":id,"ts":now*1000,"tokensIn":1,"tokensOut":1}).to_string(),
            )
            .unwrap();
        }
        let before = fingerprint("roo", temp.path());
        let file = temp.path().join("tasks/a/history_item.json");
        let meta = std::fs::metadata(&file).unwrap();
        std::fs::write(
            &file,
            json!({"id":"a","ts":now*1000,"tokensIn":100,"tokensOut":1}).to_string(),
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(meta.modified().unwrap()))
            .unwrap();
        assert_ne!(before, fingerprint("roo", temp.path()));
    }
}
