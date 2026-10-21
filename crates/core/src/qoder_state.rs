//! Recover Qoder session totals from the encrypted `state.json` sidecar.
//!
//! Qoder writes request rows to `<project>/<session>.jsonl`, but older builds
//! zeroed the token counters unless `QODER_EXPOSE_TOKEN_USAGE=true` was present
//! when the process started. The same session keeps an encrypted cumulative
//! usage record under `<project>/<session>/state.json`. We decrypt only that
//! usage segment locally; the key never leaves memory and is never persisted.
use crate::{
    models::*,
    pricing,
    store::{Store, expand},
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

// Minified binding names differ across CLI versions and operating systems.
// AES-GCM authentication selects the matching key from 32-byte Buffer literals.
const STATE_KEY_MARKER: &[u8] = b"Buffer.from([";
const SEGMENT: &str = "s0";

pub(crate) fn model_display_name(model: &str) -> Option<&'static str> {
    // Names observed in Qoder's SessionStart metadata, not inferred vendor models.
    match model {
        "dfmodel" => Some("DeepSeek-Flash"),
        "qfmodel" => Some("Qwen3.8-Flash"),
        "Qoder" => Some("Qoder（主模型待识别）"),
        _ => None,
    }
}

fn main_models(store: &Store, source: &Source) -> Result<BTreeMap<String, String>> {
    let mut models = BTreeMap::new();
    // Parser cursors retain main-chain metadata even when no usage event could
    // be emitted. Both collectors replay old cursors on the parser upgrade.
    let mut query = store.db.prepare(
        "SELECT state FROM files WHERE source_id=?1
         UNION ALL SELECT state FROM remote_cursors WHERE source_id=?1",
    )?;
    for row in query.query_map([&source.id], |r| r.get::<_, String>(0))? {
        let state: ParseState = serde_json::from_str(&row?)?;
        if !state.session_id.is_empty() && !state.qoder_main_model.is_empty() {
            models
                .entry(state.session_id)
                .or_insert(state.qoder_main_model);
        }
    }
    Ok(models)
}

#[derive(Debug, Default)]
pub(crate) struct Report {
    pub changed: u64,
    pub residuals: u64,
    pub issues: Vec<Value>,
}

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS qoder_state(
           source_id TEXT NOT NULL,
           path TEXT NOT NULL,
           session_id TEXT NOT NULL,
           signature TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           tokens TEXT NOT NULL,
           PRIMARY KEY(source_id,path)
         );
         CREATE INDEX IF NOT EXISTS qoder_state_session ON qoder_state(source_id,session_id);",
    )?;
    Ok(())
}

pub(crate) fn remote_cursors(store: &Store, source: &Source) -> Result<Value> {
    let mut query = store
        .db
        .prepare("SELECT path,signature FROM qoder_state WHERE source_id=?1")?;
    let mut cursors = serde_json::Map::new();
    for row in query.query_map([&source.id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (path, signature) = row?;
        cursors.insert(path, json!(signature));
    }
    Ok(Value::Object(cursors))
}

struct Pending {
    path: PathBuf,
    signature: String,
    value: Value,
}

fn discover_local(source: &Source) -> Result<Vec<PathBuf>> {
    let root = expand(&source.path);
    let projects = if root.join("projects").is_dir() {
        root.join("projects")
    } else {
        root
    };
    if !projects.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for project in fs::read_dir(&projects).context("读取 Qoder 项目目录失败")? {
        let project = project?;
        if !project.file_type()?.is_dir() {
            continue;
        }
        for session in fs::read_dir(project.path()).context("读取 Qoder 会话目录失败")? {
            let session = session?;
            if !session.file_type()?.is_dir() {
                continue;
            }
            let path = session.path().join("state.json");
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn collect_pending(store: &Store, source: &Source) -> Result<(Vec<Pending>, Vec<Value>)> {
    let mut pending = Vec::new();
    let mut issues = Vec::new();
    for path in discover_local(source)? {
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(error) => {
                issues.push(json!({"code":"qoderStateRead","message":format!("{}: {error}", path.display())}));
                continue;
            }
        };
        let signature = bytes_hash(&raw);
        let known: Option<String> = store
            .db
            .query_row(
                "SELECT signature FROM qoder_state WHERE source_id=?1 AND path=?2",
                params![source.id, path.to_string_lossy()],
                |r| r.get(0),
            )
            .optional()?;
        if known.as_deref() == Some(&signature) {
            continue;
        }
        match serde_json::from_slice::<Value>(&raw) {
            Ok(value) => pending.push(Pending {
                path,
                signature,
                value,
            }),
            Err(error) => issues.push(json!({
                "code":"qoderStateRead",
                "message":format!("{}: 会话状态格式无效（{error}）", path.display()),
            })),
        }
    }
    Ok((pending, issues))
}

pub(crate) fn sync_local(
    store: &Store,
    settings: &Settings,
    source: &Source,
    prices: &[ModelPrice],
) -> Result<Report> {
    let (pending, mut issues) = collect_pending(store, source)?;
    let keys = if pending.is_empty() {
        Vec::new()
    } else {
        extract_keys(source)
    };
    if !pending.is_empty() && keys.is_empty() {
        issues.push(json!({
            "code":"qoderStateKey",
            "message":"未找到可读取的 qodercli 状态密钥，历史 Token 总量暂未补足；新请求仍可正常统计。",
        }));
    }
    let mut report = apply_pending(store, source, pending, &keys, prices, settings)?;
    report.issues.splice(0..0, issues);
    Ok(report)
}

#[cfg(test)]
fn sync_local_with_keys(
    store: &Store,
    settings: &Settings,
    source: &Source,
    prices: &[ModelPrice],
    keys: &[Vec<u8>],
) -> Result<Report> {
    let (pending, mut issues) = collect_pending(store, source)?;
    let mut report = apply_pending(store, source, pending, keys, prices, settings)?;
    report.issues.splice(0..0, issues.drain(..));
    Ok(report)
}

fn apply_pending(
    store: &Store,
    source: &Source,
    pending: Vec<Pending>,
    keys: &[Vec<u8>],
    prices: &[ModelPrice],
    settings: &Settings,
) -> Result<Report> {
    let mut report = Report::default();
    for item in pending {
        let session_id = item
            .value
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                item.path
                    .parent()
                    .and_then(Path::file_name)
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        if session_id.is_empty() {
            report.issues.push(json!({"code":"qoderStateRead","message":format!("{}: 会话标识缺失", item.path.display())}));
            continue;
        }
        let hashes = project_hashes(&item.path, &item.value, &session_id);
        if hashes.is_empty() {
            report.issues.push(json!({"code":"qoderStateRead","message":format!("{}: 无法确定项目路径", item.path.display())}));
            continue;
        }
        let item_value = match item.value.get("items").and_then(|v| v.get(SEGMENT)) {
            Some(value) if value.get("p").is_some() => value,
            _ => continue,
        };
        let Some(tokens) = decrypt_usage(item_value, &session_id, &hashes, keys) else {
            report.issues.push(json!({"code":"qoderStateDecrypt","message":format!("{}: 会话状态无法解密", item.path.display())}));
            continue;
        };
        let created_at = persisted_timestamp(&item.value["createdAt"])
            .or_else(|| persisted_timestamp(&item.value["updatedAt"]))
            .or_else(|| {
                fs::metadata(&item.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
            })
            .unwrap_or_else(now);
        let updated_at = persisted_timestamp(&item.value["updatedAt"]).unwrap_or(created_at);
        let tx = store.write_transaction()?;
        crate::import::current_settings(store, settings, source)?;
        store.db.execute(
            "INSERT INTO qoder_state(source_id,path,session_id,signature,created_at,updated_at,tokens)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(source_id,path) DO UPDATE SET session_id=excluded.session_id,signature=excluded.signature,created_at=excluded.created_at,updated_at=excluded.updated_at,tokens=excluded.tokens",
            params![
                source.id,
                item.path.to_string_lossy(),
                session_id,
                item.signature,
                created_at,
                updated_at,
                serde_json::to_string(&tokens)?,
            ],
        )?;
        tx.commit()?;
        report.changed += 1;
    }
    let reconciled = reconcile(store, source, prices, settings)?;
    report.residuals = reconciled.residuals;
    report.issues.extend(reconciled.issues);
    Ok(report)
}

pub(crate) fn apply_remote_frame(
    store: &Store,
    settings: &Settings,
    source: &Source,
    frame: &Value,
) -> Result<Report> {
    let mut report = Report::default();
    let keys = decode_keys(frame)?;
    let states = frame["states"]
        .as_array()
        .context("远程 Qoder 状态批次缺少记录")?;
    ensure!(states.len() <= 4096, "远程 Qoder 状态批次过大");
    if !states.is_empty() && keys.is_empty() {
        report.issues.push(json!({
            "code":"qoderStateKey",
            "message":"远程 qodercli 状态密钥不可用，历史 Token 总量暂未补足。",
        }));
        return Ok(report);
    }
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    for state in states {
        let path = state["path"].as_str().context("远程 Qoder 状态路径缺失")?;
        let session_id = state["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("远程 Qoder 会话标识缺失")?;
        let signature = state["signature"]
            .as_str()
            .context("远程 Qoder 状态签名缺失")?;
        let hashes: Vec<String> = state["projectHashes"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if hashes.is_empty() {
            report.issues.push(
                json!({"code":"qoderStateRead","message":format!("{path}: 远程项目路径未知")}),
            );
            continue;
        }
        let Some(tokens) = decrypt_usage(state, session_id, &hashes, &keys) else {
            report.issues.push(json!({"code":"qoderStateDecrypt","message":format!("{path}: 远程会话状态无法解密")}));
            continue;
        };
        let created_at = persisted_timestamp(&state["createdAt"])
            .or_else(|| persisted_timestamp(&state["updatedAt"]))
            .unwrap_or_else(now);
        let updated_at = persisted_timestamp(&state["updatedAt"]).unwrap_or(created_at);
        store.db.execute(
            "INSERT INTO qoder_state(source_id,path,session_id,signature,created_at,updated_at,tokens)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(source_id,path) DO UPDATE SET session_id=excluded.session_id,signature=excluded.signature,created_at=excluded.created_at,updated_at=excluded.updated_at,tokens=excluded.tokens",
            params![
                source.id,
                path,
                session_id,
                signature,
                created_at,
                updated_at,
                serde_json::to_string(&tokens)?,
            ],
        )?;
        report.changed += 1;
    }
    tx.commit()?;
    Ok(report)
}

fn decode_keys(frame: &Value) -> Result<Vec<Vec<u8>>> {
    let mut keys = Vec::new();
    if let Some(rows) = frame["keys"].as_array() {
        for row in rows {
            let raw = row.as_str().context("远程 Qoder 状态密钥格式无效")?;
            let key = STANDARD
                .decode(raw)
                .context("远程 Qoder 状态密钥格式无效")?;
            if key.len() == 32 && !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    Ok(keys)
}

fn decrypt_usage(
    value: &Value,
    session_id: &str,
    hashes: &[String],
    keys: &[Vec<u8>],
) -> Option<Tokens> {
    let nonce = STANDARD.decode(value["n"].as_str()?).ok()?;
    let ciphertext = STANDARD.decode(value["p"].as_str()?).ok()?;
    let tag = STANDARD.decode(value["t"].as_str()?).ok()?;
    if nonce.len() != 12 || tag.len() != 16 || ciphertext.is_empty() {
        return None;
    }
    let mut sealed = ciphertext;
    sealed.extend_from_slice(&tag);
    let segment = value
        .get("segmentKey")
        .and_then(Value::as_str)
        .unwrap_or(SEGMENT);
    for key in keys {
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
        for hash in hashes {
            let aad = aad(session_id, hash, segment);
            let mut plaintext = sealed.clone();
            if let Ok(opened) = key.open_in_place(
                Nonce::assume_unique_for_key(nonce.as_slice().try_into().ok()?),
                Aad::from(aad.as_bytes()),
                &mut plaintext,
            ) && let Ok(value) = serde_json::from_slice::<Value>(opened)
                && let Some(tokens) = usage_tokens(&value)
            {
                return Some(tokens);
            }
        }
    }
    None
}

fn usage_tokens(value: &Value) -> Option<Tokens> {
    let total = &value["total"];
    let input = total["input_tokens"].as_u64()?;
    let cache_read = total["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let cache_write = total["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    Some(Tokens {
        input: input.checked_sub(cache_read)?.checked_sub(cache_write)?,
        output: total["output_tokens"].as_u64().unwrap_or(0),
        cache_read,
        cache_write,
        reasoning: 0,
    })
}

fn aad(session_id: &str, project_hash: &str, segment: &str) -> String {
    let session = serde_json::to_string(session_id).unwrap_or_default();
    let project = serde_json::to_string(project_hash).unwrap_or_default();
    let segment = serde_json::to_string(segment).unwrap_or_default();
    format!("{{\"sessionId\":{session},\"projectHash\":{project},\"segmentKey\":{segment}}}")
}

fn persisted_timestamp(value: &Value) -> Option<i64> {
    timestamp(value)
}

fn bytes_hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn project_hashes(path: &Path, value: &Value, session_id: &str) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    if let Some(rows) = value["workspaceDirectories"].as_array() {
        roots.extend(rows.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    if let Some(project) = path.parent().and_then(Path::parent) {
        let transcript = project.join(format!("{session_id}.jsonl"));
        if let Ok(file) = File::open(transcript) {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(file).lines().take(128).flatten() {
                if let Ok(row) = serde_json::from_str::<Value>(&line)
                    && let Some(cwd) = row.get("cwd").and_then(Value::as_str)
                {
                    roots.push(cwd.to_owned());
                }
            }
        }
        if let Some(name) = project.file_name().and_then(|s| s.to_str())
            && let Some(decoded) = decode_project_dir(name)
        {
            roots.push(decoded);
        }
    }
    let mut hashes = Vec::new();
    for root in roots {
        let hash = project_hash(&root);
        if !hash.is_empty() && !hashes.contains(&hash) {
            hashes.push(hash);
        }
    }
    hashes
}

fn project_hash(root: &str) -> String {
    let path = Path::new(root);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let normalized = normalized
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    bytes_hash(normalized.as_bytes())
}

fn decode_project_dir(name: &str) -> Option<String> {
    let body = name.strip_prefix('-')?;
    if body.is_empty() {
        return None;
    }
    Some(format!("/{}", body.replace('-', "/")))
}

pub(crate) fn reconcile(
    store: &Store,
    source: &Source,
    prices: &[ModelPrice],
    settings: &Settings,
) -> Result<Report> {
    let mut rows = Vec::new();
    {
        let mut query = store.db.prepare(
            "SELECT session_id,path,created_at,tokens FROM qoder_state WHERE source_id=?1",
        )?;
        for row in query.query_map([&source.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            rows.push(row?);
        }
    }
    if rows.is_empty() {
        return Ok(Report::default());
    }
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    let captured = captured_tokens(store, source)?;
    let main_models = main_models(store, source)?;
    let mut report = Report::default();
    for (session_id, path, created_at, raw) in rows {
        let total: Tokens =
            serde_json::from_str(&raw).with_context(|| format!("Qoder 会话状态损坏：{path}"))?;
        let zero = Tokens::default();
        let residual = total.checked_delta(captured.get(&session_id).unwrap_or(&zero));
        match residual.filter(|tokens| tokens.total() > 0) {
            Some(tokens) => {
                let identity = format!("qoder-state:{session_id}");
                let event = UsageEvent {
                    turn_id: String::new(),
                    id: residual_id(source, &session_id),
                    import_identity: Some(identity),
                    source_id: source.id.clone(),
                    account_id: source.account_id.clone(),
                    provider: "qoder".into(),
                    session_id: session_id.clone(),
                    model: main_models
                        .get(&session_id)
                        .cloned()
                        .unwrap_or_else(|| "Qoder".into()),
                    timestamp: created_at,
                    tokens,
                    attribution: "qoder-residual".into(),
                    billing: BillingEvidence {
                        category: "unknown".into(),
                        evidence: "qoder-state-v1".into(),
                    },
                    native_credits: None,
                    raw_tokens: None,
                    interval_start: Some(created_at),
                    interval_evidence: "qoder-state-v1".into(),
                };
                report.changed += u64::from(put_residual(store, &event, prices, settings)?);
            }
            None => {
                delete_residual(store, &session_id, source)?;
            }
        }
    }
    tx.commit()?;
    report.residuals = report.changed;
    Ok(report)
}

fn captured_tokens(store: &Store, source: &Source) -> Result<BTreeMap<String, Tokens>> {
    let mut query = store.db.prepare(
        "SELECT e.payload FROM events e JOIN event_sources es ON es.event_id=e.id WHERE es.source_id=?1",
    )?;
    let mut captured = BTreeMap::<String, Tokens>::new();
    for row in query.query_map([&source.id], |r| r.get::<_, String>(0))? {
        let event: UsageEvent = serde_json::from_str(&row?)?;
        if event.attribution == "qoder-residual" || event.session_id.is_empty() {
            continue;
        }
        captured
            .entry(event.session_id)
            .or_default()
            .add(&event.tokens);
    }
    Ok(captured)
}

fn residual_id(source: &Source, session_id: &str) -> String {
    // The synthetic residual follows the source, not the account namespace:
    // connecting or switching accounts must update the same row instead of
    // leaving an orphaned copy behind.
    crate::usage::event_id(
        "qoder",
        &source.id,
        "",
        &format!("qoder-state:{session_id}"),
    )
}

fn delete_residual(store: &Store, session_id: &str, source: &Source) -> Result<()> {
    let id = residual_id(source, session_id);
    store
        .db
        .execute("DELETE FROM event_sources WHERE event_id=?1", [&id])?;
    store.db.execute("DELETE FROM events WHERE id=?1", [&id])?;
    Ok(())
}

fn put_residual(
    store: &Store,
    event: &UsageEvent,
    prices: &[ModelPrice],
    settings: &Settings,
) -> Result<bool> {
    if event.tokens.total() == 0 {
        store
            .db
            .execute("DELETE FROM event_sources WHERE event_id=?1", [&event.id])?;
        store
            .db
            .execute("DELETE FROM events WHERE id=?1", [&event.id])?;
        return Ok(false);
    }
    let price = pricing::find_price(&event.model, prices, &settings.model_mappings);
    let (cost, covered) = pricing::estimate(&event.tokens, price);
    let changed = store.db.execute(
        "INSERT INTO events(id,provider,account_id,model,stamp,payload,cost,priced_tokens,price)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,account_id=excluded.account_id,model=excluded.model,stamp=excluded.stamp,payload=excluded.payload,cost=excluded.cost,priced_tokens=excluded.priced_tokens,price=excluded.price
         WHERE events.payload IS NOT excluded.payload OR events.cost IS NOT excluded.cost OR events.priced_tokens IS NOT excluded.priced_tokens OR events.price IS NOT excluded.price",
        params![
            event.id,
            event.provider,
            event.account_id,
            event.model,
            event.timestamp,
            serde_json::to_string(event)?,
            cost,
            covered,
            price.map(serde_json::to_string).transpose()?,
        ],
    )?;
    store.db.execute(
        "INSERT OR IGNORE INTO event_sources VALUES(?1,?2)",
        params![event.id, event.source_id],
    )?;
    Ok(changed > 0)
}

type KeySignature = Vec<(PathBuf, u64, Option<std::time::SystemTime>)>;
type KeyCache = Mutex<Option<(KeySignature, Instant, Vec<Vec<u8>>)>>;

fn extract_keys(source: &Source) -> Vec<Vec<u8>> {
    extract_candidate_keys(key_candidates(source))
}

fn extract_candidate_keys(paths: Vec<PathBuf>) -> Vec<Vec<u8>> {
    static CACHE: LazyLock<KeyCache> = LazyLock::new(|| Mutex::new(None));
    let signature: KeySignature = paths
        .iter()
        .filter_map(|path| {
            let metadata = fs::metadata(path).ok()?;
            Some((path.clone(), metadata.len(), metadata.modified().ok()))
        })
        .collect();
    if let Ok(cache) = CACHE.lock()
        && let Some((known, at, keys)) = cache.as_ref()
        && known == &signature
        && at.elapsed() < Duration::from_secs(600)
        && !keys.is_empty()
    {
        return keys.clone();
    }
    let mut keys = Vec::new();
    for path in paths {
        for key in keys_from_file(&path) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((signature, Instant::now(), keys.clone()));
    }
    keys
}

fn key_candidates(source: &Source) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("QODER_CLI_PATH") {
        paths.push(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(if cfg!(windows) {
                "qodercli.exe"
            } else {
                "qodercli"
            });
            if candidate.is_file() {
                paths.push(candidate);
            }
        }
    }
    let home = crate::store::home();
    for candidate in [
        home.join(".local/bin/qodercli"),
        home.join("bin/qodercli"),
        PathBuf::from("/usr/local/bin/qodercli"),
        PathBuf::from("/opt/homebrew/bin/qodercli"),
    ] {
        if candidate.is_file() {
            paths.push(candidate);
        }
    }
    let config = expand(&source.path);
    let bin = config.join("bin/qodercli");
    if let Ok(rows) = fs::read_dir(&bin) {
        for row in rows.flatten() {
            if row.file_type().is_ok_and(|kind| kind.is_file()) {
                paths.push(row.path());
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn keys_from_file(path: &Path) -> Vec<Vec<u8>> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    let mut carry = Vec::new();
    let mut chunk = vec![0u8; 4 * 1024 * 1024];
    while let Ok(read) = file.read(&mut chunk) {
        if read == 0 {
            break;
        }
        let mut data = carry;
        data.extend_from_slice(&chunk[..read]);
        let mut start = 0;
        while let Some(position) = find(&data[start..], STATE_KEY_MARKER) {
            let body = &data[start + position + STATE_KEY_MARKER.len()..];
            if let Some(end) = find(body, b"])") {
                let digits = &body[..end];
                if !digits.is_empty()
                    && digits.iter().all(|b| b.is_ascii_digit() || *b == b',')
                    && let Some(key) = digits
                        .split(|b| *b == b',')
                        .map(|part| std::str::from_utf8(part).ok()?.parse::<u8>().ok())
                        .collect::<Option<Vec<u8>>>()
                    && key.len() == 32
                    && !keys.contains(&key)
                {
                    keys.push(key);
                }
            }
            start += position + STATE_KEY_MARKER.len();
        }
        // The complete 32-byte decimal array may straddle the read boundary,
        // even when its marker was already entirely inside the previous chunk.
        let keep = data.len().saturating_sub(STATE_KEY_MARKER.len() + 128);
        carry = data[keep..].to_vec();
    }
    keys
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::{SecureRandom, SystemRandom};

    fn save_source(store: &Store, source: &Source) -> Settings {
        let settings = Settings {
            sources: vec![source.clone()],
            accounts: if source.account_id.is_empty() {
                vec![]
            } else {
                vec![Account {
                    id: source.account_id.clone(),
                    provider: source.provider.clone(),
                    name: "Fixture account".into(),
                    ..Default::default()
                }]
            },
            hosts: source
                .host_id
                .iter()
                .map(|id| Host {
                    id: id.clone(),
                    name: "Fixture host".into(),
                    target: "fixture.invalid".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        settings
    }

    fn encrypt(key: &[u8], session: &str, project: &str, plaintext: &str) -> Value {
        let hash = project_hash(project);
        let aad = aad(session, &hash, SEGMENT);
        let mut nonce = [0u8; 12];
        SystemRandom::new().fill(&mut nonce).unwrap();
        let sealing = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).unwrap());
        let mut in_out = plaintext.as_bytes().to_vec();
        sealing
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad.as_bytes()),
                &mut in_out,
            )
            .unwrap();
        let tag = in_out.split_off(in_out.len() - 16);
        json!({
            "sessionId": session,
            "createdAt": "2026-10-18T04:29:24.846Z",
            "updatedAt": "2026-10-18T04:29:24.851Z",
            "workspaceDirectories": [project],
            "items": {"s0": {
                "n": STANDARD.encode(nonce),
                "p": STANDARD.encode(in_out),
                "t": STANDARD.encode(tag),
            }},
        })
    }

    #[test]
    fn main_model_history_replays_old_cursors_without_double_counting_auxiliary_usage() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "qoder".into(),
            provider: "qoder".into(),
            path: root.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let settings = save_source(&store, &source);
        let key = [23u8; 32];
        let session = root.path().join("projects/p/session");
        fs::create_dir_all(&session).unwrap();
        fs::create_dir_all(root.path().join("bin/qodercli")).unwrap();
        fs::write(
            root.path().join("bin/qodercli/fixture"),
            format!(
                "Buffer.from([{}])",
                key.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
            ),
        )
        .unwrap();
        let state = encrypt(
            &key,
            "session",
            root.path().to_str().unwrap(),
            r#"{"total":{"input_tokens":110,"cache_read_input_tokens":40,"output_tokens":7},"credits":{"total":0,"byModel":{"dfmodel":{"credits":0},"lite":{"credits":0}}}}"#,
        );
        fs::write(session.join("state.json"), state.to_string()).unwrap();
        let mut main = json!({"type":"assistant","timestamp":"2026-10-18T04:29:24Z","sessionId":"session","message":{"id":"main","model":"dfmodel","usage":{"input_tokens":0,"output_tokens":0,"credits":0.25}}});
        let auxiliary = json!({"type":"assistant","timestamp":"2026-10-18T04:29:25Z","sessionId":"session","isSidechain":true,"message":{"id":"aux","model":"lite","usage":{"input_tokens":10,"output_tokens":2,"credits":0.125}}});
        let transcript = root.path().join("projects/p/session.jsonl");
        fs::write(&transcript, format!("{main}\n{auxiliary}\n")).unwrap();
        sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).unwrap();
        let filter = Filter {
            date_range: Some(DateRange::All),
            ..Default::default()
        };
        assert_eq!(store.dashboard(&filter).unwrap().models[0].key, "Qoder");
        crate::import::scan(&store, &settings, &source).unwrap();
        let dashboard = store.dashboard(&filter).unwrap();
        assert_eq!(dashboard.models.len(), 1);
        assert_eq!(dashboard.models[0].key, "dfmodel");
        assert_eq!(dashboard.model_display_names["dfmodel"], "DeepSeek-Flash");
        assert_eq!(
            (dashboard.summary.total, dashboard.summary.events),
            (117, 2)
        );
        assert_eq!(dashboard.summary.native_credits["qoder"].credits, 0.375);
        assert_eq!(
            store
                .dashboard(&Filter {
                    model: Some("dfmodel".into()),
                    ..filter.clone()
                })
                .unwrap()
                .summary
                .total,
            117
        );
        // An old parser cursor must replay even when both files are unchanged.
        let path = transcript.to_string_lossy();
        let (signature, offset, mut cursor) = store.cursor(&source.id, &path).unwrap().unwrap();
        cursor.parser_version = 9;
        cursor.qoder_main_model.clear();
        store
            .set_cursor(&source.id, &path, &signature, offset, &cursor)
            .unwrap();
        reconcile(&store, &source, &[], &settings).unwrap();
        assert!(
            store
                .dashboard(&filter)
                .unwrap()
                .models
                .iter()
                .any(|m| m.key == "Qoder")
        );
        crate::import::scan(&store, &settings, &source).unwrap();
        assert_eq!(store.dashboard(&filter).unwrap().models.len(), 1);
        let revisions: i64 = store
            .db
            .query_row("SELECT SUM(revision) FROM usage_day_revision", [], |r| {
                r.get(0)
            })
            .unwrap();
        crate::import::scan(&store, &settings, &source).unwrap();
        assert_eq!(
            revisions,
            store
                .db
                .query_row::<i64, _, _>("SELECT SUM(revision) FROM usage_day_revision", [], |r| r
                    .get(0))
                .unwrap()
        );
        main["message"]["usage"]["input_tokens"] = json!(100);
        main["message"]["usage"]["cache_read_input_tokens"] = json!(40);
        main["message"]["usage"]["output_tokens"] = json!(5);
        fs::write(&transcript, format!("{main}\n{auxiliary}\n")).unwrap();
        crate::import::rescan(&store, &settings, &source).unwrap();
        let dashboard = store.dashboard(&filter).unwrap();
        assert_eq!(
            (dashboard.summary.total, dashboard.summary.events),
            (117, 2)
        );
        assert_eq!(dashboard.summary.native_credits["qoder"].credits, 0.375);
        assert_eq!(store.db.query_row::<i64, _, _>("SELECT COUNT(*) FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual'", [], |r| r.get(0)).unwrap(), 0);
    }

    #[test]
    fn residual_is_net_of_cache_and_replaced_without_counting_a_request() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "qoder".into(),
            provider: "qoder".into(),
            path: root.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let key = [7u8; 32];
        let project = root.path().join("workspace");
        fs::create_dir_all(root.path().join("projects/p/session")).unwrap();
        let state = encrypt(
            &key,
            "session",
            project.to_string_lossy().as_ref(),
            r#"{"total":{"input_tokens":100,"cache_read_input_tokens":40,"cache_creation_input_tokens":10,"output_tokens":5}}"#,
        );
        fs::write(
            root.path().join("projects/p/session/state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let report =
            sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).unwrap();
        assert_eq!(report.changed, 1);
        let dashboard = store
            .dashboard(&Filter {
                date_range: Some(DateRange::All),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(dashboard.summary.tokens.input, 50);
        assert_eq!(dashboard.summary.tokens.cache_read, 40);
        assert_eq!(dashboard.summary.tokens.cache_write, 10);
        assert_eq!(dashboard.summary.tokens.output, 5);
        assert_eq!(dashboard.summary.events, 0);
        let residual = store
            .db
            .query_row(
                "SELECT payload FROM events WHERE payload LIKE '%qoder-residual%'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap();
        assert!(residual.contains("qoder-state-v1"));

        let changes = store.db.total_changes();
        let repeat =
            sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).unwrap();
        assert_eq!((repeat.changed, repeat.residuals), (0, 0));
        assert_eq!(
            store.db.total_changes(),
            changes,
            "unchanged scans must not invalidate usage caches"
        );

        fs::write(
            root.path().join("projects/p/session/state.json"),
            serde_json::to_vec(&encrypt(
                &key,
                "session",
                project.to_string_lossy().as_ref(),
                r#"{"total":{"input_tokens":100,"cache_read_input_tokens":40,"cache_creation_input_tokens":10,"output_tokens":5}}"#,
            ))
            .unwrap(),
        )
        .unwrap();
        sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).unwrap();
        let events: u64 = store
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1);

        let signature: String = store
            .db
            .query_row(
                "SELECT signature FROM qoder_state WHERE source_id=?1",
                [&source.id],
                |r| r.get(0),
            )
            .unwrap();
        let mut paused = settings.clone();
        paused.sources[0].enabled = false;
        store.save_settings(&paused).unwrap();
        fs::write(
            root.path().join("projects/p/session/state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).is_err());
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT signature FROM qoder_state WHERE source_id=?1",
                    [&source.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            signature
        );
    }

    #[test]
    fn connecting_an_account_updates_the_same_residual_row() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut source = Source {
            id: "qoder".into(),
            provider: "qoder".into(),
            path: root.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let settings = save_source(&store, &source);
        let key = [3u8; 32];
        let project = root.path().join("workspace");
        fs::create_dir_all(root.path().join("projects/p/session")).unwrap();
        let state = encrypt(
            &key,
            "session",
            project.to_string_lossy().as_ref(),
            r#"{"total":{"input_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":5}}"#,
        );
        fs::write(
            root.path().join("projects/p/session/state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        sync_local_with_keys(&store, &settings, &source, &[], &[key.to_vec()]).unwrap();
        source.account_id = "account-2".into();
        let settings = save_source(&store, &source);
        reconcile(&store, &source, &[], &settings).unwrap();
        let rows: u64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        let account: String = store
            .db
            .query_row(
                "SELECT json_extract(payload,'$.accountId') FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(account, "account-2");
    }

    #[test]
    fn remote_state_frame_is_applied_before_reconcile() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "qoder-remote".into(),
            provider: "qoder".into(),
            path: "~/.qoder".into(),
            host_id: Some("server".into()),
            ..Default::default()
        };
        let settings = save_source(&store, &source);
        let key = [9u8; 32];
        let mut cursor = ParseState::default();
        let hidden = json!({"type":"assistant","timestamp":"2026-10-18T04:29:24Z","sessionId":"session","message":{"id":"main","model":"qfmodel","usage":{"input_tokens":0,"output_tokens":0}}});
        assert!(crate::usage::parse(&source, &mut cursor, &hidden).is_none());
        store
            .db
            .execute(
                "INSERT INTO remote_cursors VALUES(?1,'remote.jsonl','fixture','{}',?2)",
                params![source.id, serde_json::to_string(&cursor).unwrap()],
            )
            .unwrap();
        let project = "/srv/workspace";
        let state = encrypt(
            &key,
            "session",
            project,
            r#"{"total":{"input_tokens":80,"cache_read_input_tokens":30,"cache_creation_input_tokens":0,"output_tokens":10}}"#,
        );
        let item = &state["items"]["s0"];
        let frame = json!({
            "kind":"qoderState",
            "keys":[STANDARD.encode(key)],
            "states":[{
                "path":"/home/user/.qoder/projects/p/session/state.json",
                "sessionId":"session",
                "signature":"signature",
                "createdAt":state["createdAt"],
                "updatedAt":state["updatedAt"],
                "segmentKey":"s0",
                "projectHashes":[project_hash(project)],
                "n":item["n"],
                "p":item["p"],
                "t":item["t"],
            }],
        });
        let report = apply_remote_frame(&store, &settings, &source, &frame).unwrap();
        assert_eq!(report.changed, 1);
        let report = reconcile(&store, &source, &[], &settings).unwrap();
        assert_eq!(report.residuals, 1);
        let raw: String = store
            .db
            .query_row(
                "SELECT payload FROM events WHERE json_extract(payload,'$.attribution')='qoder-residual'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: UsageEvent = serde_json::from_str(&raw).unwrap();
        assert_eq!((event.tokens.input, event.tokens.total()), (50, 90));
        assert_eq!(event.model, "qfmodel");
        assert_eq!(
            store
                .dashboard(&Filter {
                    date_range: Some(DateRange::All),
                    ..Default::default()
                })
                .unwrap()
                .model_display_names["qfmodel"],
            "Qwen3.8-Flash"
        );

        let mut paused = settings.clone();
        paused.sources[0].enabled = false;
        store.save_settings(&paused).unwrap();
        assert!(apply_remote_frame(&store, &settings, &source, &frame).is_err());
        assert!(reconcile(&store, &source, &[], &settings).is_err());
    }

    #[test]
    fn embedded_key_is_found_across_chunk_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("qodercli");
        let key = vec![255u8; 32];
        let encoded = key.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
        let embedded = format!("Zjt=Buffer.from([{encoded}])");
        for offset in [6, STATE_KEY_MARKER.len() + 20, embedded.len() - 1] {
            let mut bytes = vec![b'x'; 4 * 1024 * 1024 - offset];
            bytes.extend_from_slice(embedded.as_bytes());
            fs::write(&path, bytes).unwrap();
            assert_eq!(keys_from_file(&path), vec![key.clone()], "offset {offset}");
        }
    }

    #[test]
    fn project_hash_matches_qoder_normalization() {
        assert_eq!(
            project_hash("/Users/Example/Work"),
            project_hash("/users/example/work")
        );
        assert_eq!(
            decode_project_dir("-Users-Example-Work").as_deref(),
            Some("/Users/Example/Work")
        );
    }

    #[test]
    fn key_cache_tracks_the_binary_for_each_source_and_upgrades() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("cli-a");
        let b = root.path().join("cli-b");
        let write = |path: &Path, byte: u8| {
            let digits = vec![byte.to_string(); 32].join(",");
            fs::write(path, format!("zMt=Buffer.from([{digits}])")).unwrap();
        };
        write(&a, 1);
        write(&b, 2);
        assert_eq!(extract_candidate_keys(vec![a.clone()]), vec![vec![1; 32]]);
        assert_eq!(extract_candidate_keys(vec![b.clone()]), vec![vec![2; 32]]);
        write(&b, 255);
        assert_eq!(extract_candidate_keys(vec![b]), vec![vec![255; 32]]);
    }

    #[test]
    fn qoder_usage_segment_splits_inclusive_input() {
        let usage = json!({"total":{"input_tokens":100,"cache_read_input_tokens":60,"cache_creation_input_tokens":10,"output_tokens":5}});
        assert_eq!(
            usage_tokens(&usage),
            Some(Tokens {
                input: 30,
                output: 5,
                cache_read: 60,
                cache_write: 10,
                reasoning: 0,
            })
        );
    }
}
