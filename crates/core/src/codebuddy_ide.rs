//! CodeBuddy IDE 4.12.x request aggregates. The extension stores history in
//! CodeBuddyExtension/Data/<user>/<ide>/<uid>/history/<workspace>/<conversation>.
//! Only index request metadata is decoded; message files and bodies are ignored.
use crate::{
    models::*,
    store::{Store, expand},
    usage,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

pub(crate) fn is_path(path: &Path) -> bool {
    path.components()
        .any(|p| p.as_os_str() == "CodeBuddyExtension")
}

pub(crate) fn directory(path: &str) -> Option<PathBuf> {
    let root = expand(path);
    (is_path(&root) && root.is_dir()).then_some(root)
}

pub(crate) fn paths() -> Vec<PathBuf> {
    let home = crate::store::home();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/CodeBuddyExtension")
    } else if cfg!(windows) {
        home.join("AppData/Local/CodeBuddyExtension")
    } else {
        home.join(".local/share/CodeBuddyExtension")
    };
    history_roots(&base).unwrap_or_default()
}

fn history_roots(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Ok(vec![]);
    }
    if root.file_name().is_some_and(|n| n == "history") {
        return Ok(vec![root.to_owned()]);
    }
    let mut roots = vec![];
    let mut walk = walkdir::WalkDir::new(root)
        .max_depth(6)
        .follow_links(false)
        .into_iter();
    let mut visited = 0;
    while let Some(entry) = walk.next() {
        let entry = entry?;
        visited += 1;
        ensure!(visited <= MAX_FILES, "CodeBuddy IDE 历史目录过多");
        if !entry.file_type().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name == "history" {
            roots.push(entry.path().to_owned());
            walk.skip_current_dir();
        } else if entry.depth() > 0
            && (name.starts_with('.')
                || matches!(
                    name.as_ref(),
                    "Cache" | "Logs" | "Public" | "messages" | "file-tree" | "check-point"
                ))
        {
            walk.skip_current_dir();
        }
    }
    roots.sort();
    Ok(roots)
}

/// The directory is named from authenticationService.account.uid, independently
/// of whichever account is currently signed in. No credential is read here.
pub(crate) fn user_id(root: &Path) -> Option<String> {
    if !is_path(root) || root.file_name()? != "history" {
        return None;
    }
    let user = root.parent()?.file_name()?.to_str()?;
    (!user.is_empty() && user.len() <= 256 && !matches!(user, "default" | "Public" | "anonymous"))
        .then(|| user.to_owned())
}

pub(crate) fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = vec![];
    let mut visited = 0;
    for history in history_roots(root)? {
        for entry in walkdir::WalkDir::new(history)
            .max_depth(3)
            .follow_links(false)
        {
            let entry = entry?;
            visited += 1;
            ensure!(visited <= MAX_FILES * 5, "CodeBuddy IDE 历史文件过多");
            if entry.depth() == 3
                && entry.file_type().is_file()
                && entry.file_name() == "index.json"
            {
                files.push(entry.path().to_owned());
                ensure!(files.len() <= MAX_FILES, "CodeBuddy IDE 历史文件过多");
            }
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Deserialize, Default)]
pub(crate) struct History {
    #[serde(default)]
    requests: Vec<Request>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Request {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    state: Value,
    #[serde(default)]
    started_at: Value,
    #[serde(default)]
    usage: Value,
}

pub(crate) fn read(path: &Path) -> Result<History> {
    let before = std::fs::metadata(path)?;
    ensure!(
        before.is_file() && before.len() <= MAX_FILE,
        "CodeBuddy IDE 历史文件过大"
    );
    let value = serde_json::from_reader(std::fs::File::open(path)?.take(MAX_FILE + 1))
        .map_err(|_| anyhow::anyhow!("CodeBuddy IDE 记录格式无效或正在写入"))?;
    let after = std::fs::metadata(path)?;
    ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "CodeBuddy IDE 历史正在写入"
    );
    Ok(value)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn project(session: &str, request: &Request) -> Result<Option<Value>> {
    let u = &request.usage;
    if u.is_null() {
        return Ok(None);
    }
    ensure!(u.is_object(), "invalid usage");
    let id = request
        .id
        .as_str()
        .filter(|s| valid_id(s))
        .ok_or_else(|| anyhow::anyhow!("invalid request"))?;
    let at = request
        .started_at
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow::anyhow!("missing request time"))?;
    let count = |key: &str| -> Result<u64> {
        match u.get(key) {
            None | Some(Value::Null) => Ok(0),
            Some(value) => value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("invalid counter")),
        }
    };
    // Usage.inputTokens includes both cache categories (verified in Usage.add
    // and _executeSingleStep); lastTokens is context occupancy, not a delta.
    let input = count("inputTokens")?;
    let output = count("outputTokens")?;
    let read = count("cacheTokens")?;
    let write = count("cachedWriteTokens")?;
    let uncached = input
        .checked_sub(read)
        .and_then(|n| n.checked_sub(write))
        .ok_or_else(|| anyhow::anyhow!("invalid cache counters"))?;
    let total = input
        .checked_add(output)
        .ok_or_else(|| anyhow::anyhow!("counter overflow"))?;
    ensure!(
        u.get("totalTokens")
            .is_none_or(|v| v.is_null() || v.as_u64() == Some(total)),
        "invalid total"
    );
    let credit = u["credit"].as_f64().filter(|n| n.is_finite() && *n > 0.);
    if total == 0 && credit.is_none() {
        return Ok(None);
    }
    let mut row = json!({"type":"ide-request-usage","id":format!("ide-request:{id}"),
        "sessionId":format!("ide:{session}"),"requestId":id,"timestamp":at / 1000,"model":"unknown",
        "tokens":{"input":uncached,"output":output,"cacheRead":read,"cacheWrite":write},
        "inheritance":{"origin":format!("codebuddy-ide:{id}:{at}"),"inherited":false}});
    if let Some(credit) = credit {
        row["nativeCredits"] = json!({"credits":credit});
    }
    Ok(Some(row))
}

pub(crate) fn rows(path: &Path, history: &History) -> (Vec<Value>, usize) {
    let Some(session) = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .filter(|s| valid_id(s))
    else {
        return (vec![], 1);
    };
    let mut rows = vec![];
    let mut invalid = 0;
    for request in &history.requests {
        match project(session, request) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(_) => invalid += 1,
        }
    }
    (rows, invalid)
}

pub(crate) fn parse(source: &Source, state: &mut ParseState, row: &Value) -> Option<UsageEvent> {
    let mut event = usage::parse_normalized(source, state, row)?;
    event.turn_id = row["requestId"].as_str().unwrap_or("").into();
    event.attribution = "codebuddy-ide-request".into();
    event.usage_version = usage::parser_version("codebuddy");
    Some(event)
}

fn signature(path: &Path) -> String {
    std::fs::metadata(path)
        .map(|m| {
            format!(
                "codebuddy-ide-v1:{}:{}",
                m.len(),
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_nanos())
            )
        })
        .unwrap_or_default()
}

pub(crate) fn scan(
    store: &Store,
    settings: &Settings,
    source: &Source,
    full: bool,
) -> Result<Value> {
    let paths = files(&expand(&source.path))?;
    let prices = store.prices()?;
    let (mut count, mut bytes, mut invalid) = (0, 0u64, 0);
    let mut issues = vec![];
    for path in &paths {
        crate::operations::check()?;
        let key = path.to_string_lossy();
        let mark = format!(
            "{}:{}:{}",
            signature(path),
            settings.precise_counting,
            settings.counting_generation
        );
        if !full
            && store
                .cursor(&source.id, &key)?
                .is_some_and(|(s, _, _)| s == mark)
        {
            continue;
        }
        let history = match read(path) {
            Ok(value) => value,
            Err(error) => {
                issues.push(json!({"code":"fileRead","path":key,"message":error.to_string()}));
                continue;
            }
        };
        bytes += std::fs::metadata(path).map_or(0, |m| m.len());
        let (rows, bad) = rows(path, &history);
        invalid += bad;
        let tx = store.write_transaction()?;
        let current = crate::import::current_settings(store, settings, source)?;
        for row in rows {
            if let Some(event) = usage::parse(source, &mut ParseState::default(), &row)
                && store.put_event(&event, &prices, &current)?
            {
                count += 1;
            }
        }
        if bad == 0 {
            store.set_cursor(&source.id, &key, &mark, 0, &ParseState::default())?;
        }
        tx.commit()?;
    }
    if invalid > 0 {
        issues.push(json!({"code":"invalidRecord","message":"部分 CodeBuddy IDE 请求缺少时间或有效用量，已保留其他计量并等待下次重试。","count":invalid}));
    }
    let status = json!({"updatedAt":now(),"newEvents":count,"files":paths.len(),"readBytes":bytes,"malformedRecords":invalid,"partial":!issues.is_empty(),"issues":issues,"format":"codebuddy-ide"});
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    store.source_status(&source.id, &status)?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(status)
}

pub(crate) fn fingerprint(root: &Path) -> String {
    files(root)
        .map(|paths| {
            hash(
                &paths
                    .iter()
                    .map(|p| format!("{}:{}", p.display(), signature(p)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })
        .unwrap_or_default()
}

pub(crate) fn activity(
    root: &Path,
    at: u64,
) -> Result<(Vec<crate::native_sessions::Activity>, Option<String>)> {
    let mut recent: Vec<_> = files(root)?
        .into_iter()
        .filter_map(|path| {
            let modified = std::fs::metadata(&path)
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            (at.saturating_sub(modified / 1000) < 86400 && !path.parent()?.join(".hidden").exists())
                .then_some((path, modified))
        })
        .collect();
    recent.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    let mut rows = vec![];
    let mut error = None;
    for (path, modified) in recent.into_iter().take(256) {
        let history = match read(&path) {
            Ok(history) => history,
            Err(problem) => {
                error = Some(problem.to_string());
                continue;
            }
        };
        let Some(request) = history.requests.last() else {
            continue;
        };
        let Some(id) = request.id.as_str().filter(|s| valid_id(s)) else {
            continue;
        };
        let Some(session) = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .filter(|s| valid_id(s))
        else {
            continue;
        };
        let kind = match request.state.as_str() {
            Some("running") => "start",
            Some("complete") => "complete",
            Some("aborted") => "failed",
            Some("canceled") => "interrupted",
            _ => "unknown",
        };
        rows.push((
            format!("ide:{session}"),
            id.into(),
            kind.into(),
            modified,
            String::new(),
        ));
    }
    Ok((rows, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_rewrites_copies_and_remote_projection_keep_one_charge() {
        let temp = tempfile::tempdir().unwrap();
        let history = temp
            .path()
            .join("CodeBuddyExtension/Data/u1/CodeBuddy/u1/history");
        let conversation = history.join("workspace/conversation");
        std::fs::create_dir_all(&conversation).unwrap();
        let path = conversation.join("index.json");
        let stamp = now() * 1000;
        let record = |input, credit| json!({"messages":[{"message":"PRIVATE"}],"requests":[{"id":"request","state":"complete","startedAt":stamp,"usage":{"inputTokens":input,"outputTokens":20,"totalTokens":input+20,"cacheTokens":30,"cachedWriteTokens":10,"credit":credit}}]});
        std::fs::write(&path, record(100, 0.75).to_string()).unwrap();
        let store = Store::open(&temp.path().join("aieyes")).unwrap();
        let source = Source {
            id: "ide".into(),
            provider: "codebuddy".into(),
            path: history.to_string_lossy().into(),
            account_id: "u1".into(),
            ..Default::default()
        };
        let account = Account {
            id: "u1".into(),
            name: "IDE fixture".into(),
            provider: "codebuddy".into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                accounts: vec![account.clone()],
                ..Default::default()
            })
            .unwrap();
        let settings = store.settings().unwrap();
        assert_eq!(
            crate::import::scan(&store, &settings, &source).unwrap()["newEvents"],
            1
        );
        assert_eq!(
            crate::import::scan(&store, &settings, &source).unwrap()["readBytes"],
            0
        );
        let clone = history.join("workspace/copied");
        std::fs::create_dir_all(&clone).unwrap();
        std::fs::copy(&path, clone.join("index.json")).unwrap();
        crate::import::scan(&store, &settings, &source).unwrap();
        assert_eq!(
            crate::quota::local_credits_meter(&store, &settings, &account)
                .unwrap()
                .unwrap()
                .used,
            0.75
        );
        std::fs::write(&path, record(200, 1.25).to_string()).unwrap();
        crate::import::scan(&store, &settings, &source).unwrap();
        assert_eq!(
            crate::quota::local_credits_meter(&store, &settings, &account)
                .unwrap()
                .unwrap()
                .used,
            1.25
        );
        assert_eq!(activity(&history, now() as u64).unwrap().0.len(), 2);
        std::fs::write(conversation.join(".hidden"), "").unwrap();
        assert_eq!(activity(&history, now() as u64).unwrap().0.len(), 1);
        let payloads: String = store
            .db
            .query_row("SELECT group_concat(payload) FROM events", [], |r| r.get(0))
            .unwrap();
        assert!(!payloads.contains("PRIVATE"));
        assert!(!history.join("settings.json").exists());
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/codebuddy_ide_remote.py");
        let output = std::process::Command::new("python3").args(["-c","import json,pathlib,runpy,sys;m=runpy.run_path(sys.argv[1]);p=pathlib.Path(sys.argv[2]);print(json.dumps(m['rows'](p,m['read'](p)[0])))"]).arg(script).arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let remote: Value = serde_json::from_slice(&output.stdout).unwrap();
        let (local, bad) = rows(&path, &read(&path).unwrap());
        assert_eq!(remote, json!([local, bad]));
    }
    #[test]
    fn request_aggregate_has_disjoint_cache_and_no_private_content_or_context_tokens() {
        let history: History = serde_json::from_value(json!({"messages":[{"message":"PRIVATE"}],"requests":[
            {"id":"r1","state":"complete","startedAt":1793319874086_i64,"messages":["PRIVATE"],"usage":{"inputTokens":100,"outputTokens":20,"totalTokens":120,"cacheTokens":30,"cachedWriteTokens":10,"cachedMissTokens":60,"lastTokens":9999,"credit":0.75}},
            {"id":"r2","state":"running"},
            {"id":"bad","startedAt":1793319874086_i64,"usage":{"inputTokens":1,"cacheTokens":2}}
        ]})).unwrap();
        let (rows, bad) = rows(Path::new("workspace/conversation/index.json"), &history);
        assert_eq!(bad, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]["tokens"],
            json!({"input":60,"output":20,"cacheRead":30,"cacheWrite":10})
        );
        assert_eq!(rows[0]["nativeCredits"]["credits"], 0.75);
        assert_eq!(rows[0]["timestamp"], 1793319874);
        assert!(!rows[0].to_string().contains("PRIVATE"));
        assert!(!rows[0].to_string().contains("9999"));
    }
    #[test]
    fn real_directory_layout_keeps_user_roots_and_ignores_message_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("CodeBuddyExtension");
        let history = root.join("Data/u1/CodeBuddy/u1/history");
        let conversation = history.join("workspace/conversation");
        std::fs::create_dir_all(conversation.join("messages")).unwrap();
        std::fs::write(conversation.join("index.json"), "{\"requests\":[]}").unwrap();
        std::fs::write(conversation.join("messages/private.json"), "PRIVATE").unwrap();
        assert_eq!(
            history_roots(&root).unwrap(),
            std::slice::from_ref(&history)
        );
        assert_eq!(user_id(&history).as_deref(), Some("u1"));
        assert_eq!(files(&root).unwrap(), [conversation.join("index.json")]);
        assert!(
            read(&conversation.join("index.json"))
                .unwrap()
                .requests
                .is_empty()
        );
    }
}
