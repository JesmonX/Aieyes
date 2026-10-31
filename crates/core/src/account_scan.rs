//! Explicit discovery. Native secrets are read only inside the core and are never
//! serialized in settings, progress or results. File references remain on their
//! owning machine; discovering an account never changes history attribution.
use crate::{Engine, accounts, contracts, models::*, store::expand};
use anyhow::{Context, Result, ensure};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(test)]
mod copilot_tests;
#[cfg(test)]
mod gemini_tests;
mod omp;
mod remote;

const LIMIT: u64 = 1024 * 1024;
const MAX_FILES: usize = 128;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct NativeConnection {
    pub id: String,
    pub provider: String,
    pub machine_id: String,
    pub machine_fingerprint: String,
    pub source_id: Option<String>,
    pub source_path: String,
    pub root: String,
    pub config_path: String,
    pub selector: String,
    pub kind: String,
    pub account_key: Option<String>,
    pub identity_key: Option<String>,
    pub email: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub has_credential: bool,
    pub status: String,
    pub note: String,
    pub checked_at: i64,
    /// Provider-scoped server identity (`openai:`, `anthropic:`, `google:`, `github:`),
    /// hashed except for GitHub's public numeric id; merges a login across sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_identity: Option<String>,
    /// When a saved token was first seen expired. The native client usually
    /// refreshes it on next launch, so this is only shown after 24 hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expired_at: Option<i64>,
}
/// Seconds an expired or unreadable credential is tolerated before it is `stale`.
pub(crate) const STALE_AFTER: i64 = 24 * 3600;
pub(crate) fn stale_note(provider: &str) -> String {
    let name = contracts::provider(provider).map_or(provider, |p| p.name);
    format!("打开 {name} 一次即可刷新")
}
#[derive(Clone)]
struct Candidate {
    connection: NativeConnection,
    fingerprint: String,
    /// In memory only, for zero-config quota reads; never stored or sent to the UI.
    secret: String,
    /// Plan the client recorded next to its login (Cursor `stripeMembershipType`).
    plan: String,
    /// Upstream login routing, in memory only; never stored as a reference.
    upstream_auth: Option<Value>,
}
struct Document {
    root: String,
    path: String,
    bytes: Vec<u8>,
}
#[derive(Default)]
struct Loaded {
    documents: Vec<Document>,
    errors: Vec<String>,
    roots: Vec<String>,
    aliases: std::collections::BTreeMap<String, String>,
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn public_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}
fn url(value: &str) -> String {
    reqwest::Url::parse(value)
        .ok()
        .filter(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
        })
        .map(|u| u.to_string())
        .unwrap_or_default()
}
fn ptr(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}
fn spec(provider: &str) -> (&'static str, Vec<&'static str>) {
    match provider {
        "codex" => (
            "CODEX_HOME",
            vec![
                "auth.json",
                "config.toml",
                ".aieyes/accounts/*/auth.json",
                ".aieyes/accounts/*/metadata.json",
                ".aieyes/state/home.json",
            ],
        ),
        "claude" => (
            "CLAUDE_CONFIG_DIR",
            vec![".credentials.json", ".claude.json", "settings.json"],
        ),
        "antigravity" => ("", vec!["antigravity-oauth-token", "settings.json"]),
        "kimi" => (
            "KIMI_CODE_HOME",
            vec![
                "config.toml",
                "credentials.json",
                "auth.json",
                "oauth/*.json",
                "credentials/*.json",
            ],
        ),
        "grok" => ("GROK_HOME", vec!["config.toml", "auth.json"]),
        "opencode" => (
            "XDG_DATA_HOME",
            vec!["auth.json", "opencode.json", "opencode.jsonc"],
        ),
        "qoder" => (
            "QODER_CONFIG_DIR",
            vec!["settings.json", "auth.json", "credentials.json"],
        ),
        "codebuddy" => (
            "CODEBUDDY_CONFIG_DIR",
            vec![
                "settings.json",
                "models.json",
                ".credentials.json",
                "auth.json",
            ],
        ),
        "zcode" => (
            "",
            vec![
                "v2/provider_config.json",
                "v2/credentials.json",
                "auth.json",
                "credentials.json",
            ],
        ),
        "pi" => ("PI_CODING_AGENT_DIR", vec!["auth.json", "models.json"]),
        "omp" => (
            "PI_CODING_AGENT_DIR",
            vec!["agent.db", "auth.json", "models.yml", "models.yaml"],
        ),
        "deepseek-harness" => (
            "DSH_HOME",
            vec![
                "cordis.patch.yml",
                "profiles/*/cordis.patch.yml",
                ".credentials.yaml",
                ".env",
            ],
        ),
        "cursor" => ("CURSOR_CONFIG_DIR", vec!["cli-config.json", "auth.json"]),
        // VS Code-style user data; the login sits in the main process' storage.json.
        "trae" | "trae-cn" => ("", vec!["User/globalStorage/storage.json"]),
        // Identity only: the desktop token is encrypted and never read. The
        // account snapshot is WorkBuddy's own non-secret record of the current
        // login (removed on logout); the database only serves older clients.
        "workbuddy" => (
            "",
            vec!["storage/skeleton/account-snapshot.json", "workbuddy.db"],
        ),
        // JSONC; a token is present only when the CLI was allowed to store it in
        // plain text. The `copilot-cli` keychain item is never read.
        "copilot" => ("COPILOT_HOME", vec!["config.json"]),
        // Plain-text JSON shared by the Cline CLI and the new extension. Read only:
        // Cline refreshes it under its own lock.
        "cline" => ("CLINE_DATA_DIR", vec!["settings/providers.json"]),
        "kilo" => ("XDG_DATA_HOME", vec!["auth.json"]),
        // Kiro keeps its logins outside `~/.kiro`; see `extra_paths`.
        _ => ("", vec![]),
    }
}
/// Whether a provider has any credential file to look at.
fn has_files(provider: &str) -> bool {
    !spec(provider).1.is_empty() || !extra_paths(provider).is_empty()
}
fn default_root(provider: &str, remote: bool) -> Result<String> {
    if matches!(provider, "opencode" | "kilo") && !remote {
        return Ok(std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".local/share"))
            .join(provider)
            .to_string_lossy()
            .into());
    }
    if provider == "codex" && !remote {
        return Ok(std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".codex"))
            .to_string_lossy()
            .into());
    }
    crate::provider_paths::default_path(provider, remote)
}
fn patterns(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut paths = vec![root.to_owned()];
    for part in pattern.split('/') {
        paths = paths
            .into_iter()
            .flat_map(|p| {
                if part.contains('*') {
                    let (prefix, suffix) = part.split_once('*').unwrap();
                    let mut found: Vec<_> = std::fs::read_dir(p)
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            let n = e.file_name();
                            let n = n.to_string_lossy();
                            !n.starts_with('.') && n.starts_with(prefix) && n.ends_with(suffix)
                        })
                        .map(|e| e.path())
                        .collect();
                    found.sort();
                    found.truncate(MAX_FILES);
                    found
                } else {
                    vec![p.join(part)]
                }
            })
            .take(MAX_FILES)
            .collect();
    }
    paths
}
fn extra_paths(provider: &str) -> Vec<String> {
    let home = crate::store::home();
    match provider {
        "cline" => std::env::var_os("CLINE_PROVIDER_SETTINGS_PATH")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(|p| p.to_string_lossy().into_owned())
            .into_iter()
            .collect(),
        "kilo" => {
            let config = std::env::var_os("KILO_CONFIG_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| {
                    std::env::var_os("XDG_CONFIG_HOME")
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute())
                        .unwrap_or_else(|| home.join(".config"))
                        .join("kilo")
                });
            [
                "config.json",
                "kilo.json",
                "kilo.jsonc",
                "opencode.json",
                "opencode.jsonc",
            ]
            .iter()
            .map(|name| config.join(name))
            .chain(
                std::env::var_os("KILO_CONFIG")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute()),
            )
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
        }
        "claude" => vec![home.join(".claude.json").to_string_lossy().into()],
        "antigravity" => vec![
            home.join(".gemini/antigravity-oauth-token")
                .to_string_lossy()
                .into(),
        ],
        "opencode" => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".config"));
            vec![
                config
                    .join("opencode/opencode.json")
                    .to_string_lossy()
                    .into(),
                config
                    .join("opencode/opencode.jsonc")
                    .to_string_lossy()
                    .into(),
            ]
        }
        "zcode" => {
            let base = std::env::var_os("ZCODE_DATA_BASE_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(home)
                .join(".zcode/v2");
            vec![
                std::env::var_os("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| base.join("provider_config.json"))
                    .to_string_lossy()
                    .into(),
                base.join("credentials.json").to_string_lossy().into(),
            ]
        }
        "copilot" => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".config"));
            let gh = std::env::var_os("GH_CONFIG_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| config.join("gh"));
            vec![
                config
                    .join("github-copilot/apps.json")
                    .to_string_lossy()
                    .into(),
                config
                    .join("github-copilot/hosts.json")
                    .to_string_lossy()
                    .into(),
                // Only when gh stores its token in plain text; `gh auth token` is never run.
                gh.join("hosts.yml").to_string_lossy().into(),
            ]
        }
        "kiro" => {
            // kiro-cli's own database (read-only, never refreshed) and the IDE token.
            let data = if cfg!(target_os = "macos") {
                home.join("Library/Application Support")
            } else if cfg!(windows) {
                std::env::var_os("APPDATA")
                    .map_or_else(|| home.join("AppData/Roaming"), PathBuf::from)
            } else {
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| home.join(".local/share"))
            };
            vec![
                data.join("kiro-cli/data.sqlite3").to_string_lossy().into(),
                home.join(".aws/sso/cache/kiro-auth-token.json")
                    .to_string_lossy()
                    .into(),
            ]
        }
        "cursor" => {
            let base = if cfg!(target_os = "macos") {
                home.join("Library/Application Support")
            } else if cfg!(windows) {
                std::env::var_os("APPDATA")
                    .map_or_else(|| home.join("AppData/Roaming"), PathBuf::from)
            } else {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| home.join(".config"))
            };
            vec![
                base.join("Cursor/User/globalStorage/state.vscdb")
                    .to_string_lossy()
                    .into(),
            ]
        }
        _ => vec![],
    }
}
/// Login rows other apps keep in SQLite, exposed as a small JSON document.
/// The remote scanner sends the same projection, never a database file.
fn sqlite_db(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    name == "state.vscdb"
        || name == "opencode.db"
        || name == "agent.db"
        || name == "workbuddy.db"
        || name == "data.sqlite3"
}
fn read_sqlite(root: &str, path: &Path) -> Result<Document> {
    let db = crate::sqlite_ro::open_ro(path).context("原生配置无法读取")?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let value = if name == "state.vscdb" {
        cursor_state(&db)?
    } else if name == "workbuddy.db" {
        workbuddy_identity(&db, path)?
    } else if name == "data.sqlite3" {
        kiro_auth(&db)?
    } else if name == "agent.db" {
        omp::credentials(&db)?
    } else {
        opencode_credentials(&db)?
    };
    Ok(Document {
        root: root.into(),
        path: path.to_string_lossy().into(),
        bytes: serde_json::to_vec(&value)?,
    })
}
/// Cursor IDE: `ItemTable` keys `cursorAuth/*`.
fn cursor_state(db: &rusqlite::Connection) -> Result<Value> {
    let mut auth = serde_json::Map::new();
    let mut stmt = db
        .prepare("SELECT key,value FROM ItemTable WHERE key IN ('cursorAuth/accessToken','cursorAuth/cachedEmail','cursorAuth/stripeMembershipType')")
        .context("Cursor 登录状态格式不支持")?;
    let rows = stmt.query_map([], |r| {
        let key: String = r.get(0)?;
        let value = match r.get_ref(1)? {
            rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => {
                String::from_utf8_lossy(t).into_owned()
            }
            _ => String::new(),
        };
        Ok((key, value))
    })?;
    for row in rows {
        let (key, value) = row?;
        let value = value.trim().trim_matches('"').to_owned();
        // Cursor 3 may seal the token with the OS keychain (`cursor-safe-storage:v1:`).
        // That key is never read: such a login keeps its email but has no quota here.
        if key == "cursorAuth/accessToken" && value.starts_with("cursor-safe-storage:") {
            auth.insert("encrypted".into(), json!(true));
            continue;
        }
        if !value.is_empty() && value.len() <= 32768 {
            auth.insert(key.trim_start_matches("cursorAuth/").into(), json!(value));
        }
    }
    Ok(json!({ "cursorAuth": auth }))
}
/// WorkBuddy before the account snapshot existed: the user of the most recent
/// visible session. Only `sessions.user_id` is read; `edge-sync-mapping-v4.db`,
/// `device-id` and the encrypted `local_storage` token are never opened. A
/// client that keeps `storage/skeleton` names its login there, and a missing
/// snapshot then means signed out, so old sessions do not name a current login.
fn workbuddy_identity(db: &rusqlite::Connection, path: &Path) -> Result<Value> {
    use rusqlite::OptionalExtension;
    if !crate::opencode::table_exists(db, "sessions")?
        || path
            .parent()
            .is_some_and(|root| root.join("storage/skeleton").is_dir())
    {
        return Ok(json!({}));
    }
    let user: Option<String> = db
        .query_row(
            "SELECT user_id FROM sessions WHERE deleted_at IS NULL AND user_id<>'' ORDER BY COALESCE(last_activity_at,updated_at) DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .context("WorkBuddy 会话格式不支持")?;
    Ok(json!({ "workbuddy": { "userId": user } }))
}
/// kiro-cli: the OIDC (`odic`, sic) or social token in `auth_kv` and the
/// profile ARN in `state`. Opened read-only; the rotating refresh token is
/// never used, so it is dropped here.
fn kiro_auth(db: &rusqlite::Connection) -> Result<Value> {
    use rusqlite::OptionalExtension;
    let read = |table: &str, key: &str| -> Result<Value> {
        if !crate::opencode::table_exists(db, table)? {
            return Ok(Value::Null);
        }
        let raw: Option<String> = db
            .query_row(
                &format!("SELECT CAST(value AS TEXT) FROM {table} WHERE key=?1"),
                [key],
                |r| r.get(0),
            )
            .optional()
            .context("Kiro 登录状态格式不支持")?;
        raw.map(|r| serde_json::from_str(&r).context("Kiro 登录状态格式不支持"))
            .unwrap_or(Ok(Value::Null))
    };
    let mut out = serde_json::Map::new();
    for (name, key) in [
        ("odic", "kirocli:odic:token"),
        ("social", "kirocli:social:token"),
    ] {
        let mut token = read("auth_kv", key)?;
        if let Some(t) = token.as_object_mut() {
            t.retain(|k, _| !k.to_ascii_lowercase().contains("refresh"));
            out.insert(name.into(), token);
        }
    }
    let profile = read("state", "api.codewhisperer.profile")?;
    out.insert("profileArn".into(), profile["arn"].clone());
    Ok(json!({ "kiro": out }))
}
/// Seconds since the epoch from a number (s or ms) or an RFC 3339 string.
fn epoch(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_f64() {
        let n = n as i64;
        return Some(if n > 10_000_000_000 { n / 1000 } else { n });
    }
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}
/// Kiro login rows: kiro-cli's database and the IDE's `kiro-auth-token.json`.
/// The credential is `{accessToken, profileArn}`; the API region comes from the
/// ARN. kiro-cli keeps the ARN in `state` (social tokens also carry it); the IDE
/// keeps it in the token, or for IAM Identity Center in its own `profile.json`.
fn kiro_rows(machine: &str, doc: &Document, v: &Value) -> Vec<Candidate> {
    let (selector, entry, token, arn) = if let Some(k) = v.get("kiro") {
        let tokens: Vec<_> = ["odic", "social"]
            .into_iter()
            .map(|n| (n, &k[n]))
            .filter(|(_, t)| !kiro_token(t).is_empty())
            .collect();
        // A leftover token of an earlier sign-in method must not shadow the current one.
        let Some((name, token)) = tokens
            .iter()
            .max_by_key(|(_, t)| {
                let expires = kiro_expiry(t);
                (
                    expires.is_none_or(|e| e > now()),
                    epoch(&t["updated_at"])
                        .or_else(|| epoch(&t["updatedAt"]))
                        .or_else(|| epoch(&t["issued_at"]))
                        .or_else(|| epoch(&t["issuedAt"]))
                        .or(expires)
                        .unwrap_or(0),
                )
            })
            .copied()
        else {
            return vec![];
        };
        let arn = [
            &k["profileArn"],
            &token["profile_arn"],
            &token["profileArn"],
        ]
        .into_iter()
        .map(text)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_owned();
        (format!("/auth_kv/{name}"), name, token, arn)
    } else {
        let arn = Some(text(&v["profileArn"]).to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| (machine == "local").then(kiro_ide_profile_arn).flatten())
            .unwrap_or_default();
        ("/kiro-auth-token".to_owned(), "ide", v, arn)
    };
    let access = kiro_token(token);
    if access.is_empty() {
        return vec![];
    }
    let secret =
        json!({"accessToken":access,"profileArn":arn,"expiresAt":kiro_expiry(token)}).to_string();
    let mut row = candidate("kiro", machine, doc, &selector, "login", &secret);
    // Tokens rotate on every refresh; the login itself is the profile plus how
    // it signed in, which kiro-cli and the IDE describe the same way.
    let kind = kiro_login_kind(token, entry);
    row.fingerprint = hash(&json!([selector, arn, kind]).to_string());
    if !arn.is_empty() {
        let id = format!("kiro:{}", hash(&json!([arn, kind]).to_string()));
        row.connection.identity_key = Some(id.clone());
        // CLI and IDE logins of the same profile are one quota target.
        row.connection.upstream_identity = Some(id);
    }
    row.connection.note = "已识别 Kiro 登录".into();
    // Never stale: kiro-cli refreshes its own login when Aieyes falls back to
    // `/usage`; an IDE login refreshes when the IDE opens.
    row.connection.token_expired_at = kiro_expiry(token).filter(|t| *t <= now());
    vec![row]
}
fn kiro_token(token: &Value) -> &str {
    token["access_token"]
        .as_str()
        .or(token["accessToken"].as_str())
        .unwrap_or("")
}
fn kiro_expiry(token: &Value) -> Option<i64> {
    epoch(&token["expires_at"]).or_else(|| epoch(&token["expiresAt"]))
}
/// Builder ID's start URL in kiro-cli's OIDC token.
const KIRO_BUILDER_ID: &str = "https://view.awsapps.com/start";
/// How a Kiro login signed in, normalized across kiro-cli (`provider`
/// google/github, `start_url`) and the IDE (`provider` Google/Github/BuilderId/
/// Enterprise). Two people with the same sign-in method and default profile
/// cannot be told apart locally; the token itself is opaque.
fn kiro_login_kind(token: &Value, entry: &str) -> String {
    let provider = text(&token["provider"]).to_ascii_lowercase();
    let start = [text(&token["start_url"]), text(&token["startUrl"])]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .trim_end_matches('/');
    match provider.as_str() {
        "google" | "github" => format!("social:{provider}"),
        "builderid" | "internal" => provider,
        "enterprise" => "idc".into(),
        "externalidp" => "external".into(),
        _ if start == KIRO_BUILDER_ID => "builderid".into(),
        _ if !start.is_empty() => "idc".into(),
        _ => entry.into(),
    }
}
/// The profile the Kiro IDE selected, kept in its extension storage
/// (`User/globalStorage/kiro.kiroagent/profile.json`, Kiro IDE 1.4.62).
fn kiro_ide_profile_arn() -> Option<String> {
    let home = crate::store::home();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map_or_else(|| home.join("AppData/Roaming"), PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
    };
    kiro_profile_arn(&base.join("Kiro/User/globalStorage/kiro.kiroagent/profile.json"))
}
fn kiro_profile_arn(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return None;
    }
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(text(&v["arn"]).to_owned()).filter(|a| a.starts_with("arn:") && a.len() <= 256)
}
/// OpenCode 2: the credential each integration currently uses, shaped like
/// auth.json entries. The selection mirrors OpenCode's own: the `active` row,
/// otherwise the newest (rows imported from auth.json carry no flag). `value`
/// is plain JSON: `{type:"oauth",access,refresh,expires,metadata}`,
/// `{type:"key",key}` or `{type:"external"}` (no secret here).
fn opencode_credentials(db: &rusqlite::Connection) -> Result<Value> {
    let mut out = serde_json::Map::new();
    if !crate::opencode::table_exists(db, "credential")? {
        return Ok(Value::Object(out));
    }
    let mut stmt = db.prepare(
        "SELECT integration_id,value FROM credential WHERE integration_id IS NOT NULL AND integration_id<>'' ORDER BY integration_id,active DESC,time_created DESC,id DESC LIMIT 256",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (name, raw) = row?;
        if out.contains_key(&name) || out.len() >= 64 {
            continue;
        }
        let value = match serde_json::from_str::<Value>(&raw) {
            Ok(v) if v.is_object() => v,
            _ if !raw.trim().is_empty() => json!({"type":"api","key":raw.trim()}),
            _ => continue,
        };
        out.insert(name, value);
    }
    Ok(Value::Object(out))
}
/// The key the Trae IDEs (`authProviderId` `icube.cloudide`) keep their login under.
const TRAE_LOGIN: &str = "iCubeAuthInfo://icube.cloudide";
/// Trae / Trae CN login in `storage.json`. Current builds wrap it in an
/// app-encrypted envelope that is never decrypted: the row then only records
/// that a login exists, and the official usage page is the fallback. A
/// plain-text login yields `{token, host}` for the usage read; its refresh
/// token is never used and nothing is written back.
fn trae_rows(provider: &str, machine: &str, doc: &Document, v: &Value) -> Vec<Candidate> {
    let raw = &v[TRAE_LOGIN];
    let login = match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_default(),
        _ => return vec![],
    };
    let selector = format!("/{}", ptr(TRAE_LOGIN));
    let token = text(&login["token"]);
    if token.is_empty() {
        if raw.as_str().is_none_or(|s| s.trim().is_empty()) {
            return vec![];
        }
        let mut row = candidate(provider, machine, doc, &selector, "login", "");
        // The envelope changes on every token refresh; the login itself does not.
        row.fingerprint = hash(&selector);
        row.connection.note = "Trae 登录已加密保存；额度请在官方用量页查看".into();
        return vec![row];
    }
    let host = ["host", "apiHost", "api_host"]
        .iter()
        .map(|k| text(&login[*k]))
        .find(|h| !h.is_empty())
        .unwrap_or("");
    let secret = json!({"token":token,"host":host}).to_string();
    let mut row = candidate(provider, machine, doc, &selector, "login", &secret);
    let user = [&login["userId"], &login["user_id"]]
        .into_iter()
        .map(|v| match v {
            Value::Number(n) => n.to_string(),
            v => text(v).to_owned(),
        })
        .find(|u| !u.is_empty())
        .unwrap_or_default();
    if !user.is_empty() {
        // Trae and Trae CN are separate account systems.
        let id = format!("{provider}:{}", hash(&user));
        row.connection.identity_key = Some(id.clone());
        row.connection.upstream_identity = Some(id);
        row.fingerprint = hash(&json!([selector, user]).to_string());
    }
    row.connection.email = public_text(text(&login["account"]["email"]));
    row.connection.note = "已识别 Trae 登录".into();
    let expires = epoch(&login["expiredAt"]).or_else(|| epoch(&login["expired_at"]));
    row.connection.token_expired_at = expires.filter(|t| *t <= now());
    vec![row]
}
fn read_file(root: &str, path: &Path) -> Result<Document> {
    if path.is_dir()
        && let Some(user) = crate::codebuddy_ide::user_id(path)
    {
        return Ok(Document {
            root: root.into(),
            path: path.to_string_lossy().into(),
            bytes: json!({"codebuddyHistoryUserId":user})
                .to_string()
                .into_bytes(),
        });
    }
    if sqlite_db(path) {
        return read_sqlite(root, path);
    }
    let metadata = std::fs::metadata(path).context("原生配置无法读取")?;
    ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "原生配置不是普通文件或超过 1 MiB"
    );
    let mut bytes = vec![];
    std::fs::File::open(path)
        .context("原生配置无法读取")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .context("原生配置无法读取")?;
    ensure!(bytes.len() <= LIMIT as usize, "原生配置超过 1 MiB");
    // A size change during the read means the owner is still writing it.
    ensure!(bytes.len() as u64 == metadata.len(), "原生配置正在写入");
    Ok(Document {
        root: root.into(),
        path: path.to_string_lossy().into(),
        bytes,
    })
}
fn load(provider: &str, machine: &str, settings: &Settings) -> Result<Loaded> {
    load_inner(provider, machine, settings, None)
}
fn scan_roots(
    provider: &str,
    machine: &str,
    settings: &Settings,
    reference: Option<&NativeConnection>,
) -> Result<Vec<String>> {
    let remote = machine != "local";
    let default = match reference {
        Some(r) => r.root.clone(),
        None => default_root(provider, remote)?,
    };
    let mut roots: Vec<String> = settings
        .sources
        .iter()
        .filter(|s| {
            s.provider == provider
                && s.host_id.as_deref().unwrap_or("local") == machine
                && s.machine_binding == MachineBinding::Bound
                && s.subscription.is_none()
                && !s.path.is_empty()
        })
        .map(|s| {
            if provider == "codex" {
                crate::codex_auth::home_path(&s.path)
            } else {
                s.path.clone()
            }
        })
        .collect();
    if reference.is_some() {
        roots.clear();
    }
    roots.push(default);
    if reference.is_none()
        && let Some(spec) = contracts::provider(provider)
        && !spec.default_path.is_empty()
    {
        roots.push(spec.default_path.into());
    }
    roots.sort();
    roots.dedup();
    let (env, _) = spec(provider);
    if !remote
        && reference.is_none()
        && !env.is_empty()
        && let Some(value) = std::env::var_os(env).filter(|s| !s.is_empty())
    {
        let mut path = PathBuf::from(value);
        ensure!(path.is_absolute(), "原生目录环境变量应为绝对路径");
        if matches!(provider, "opencode" | "kilo") {
            path.push(provider);
        }
        roots.push(path.to_string_lossy().into());
    }
    if remote {
        return Ok(roots);
    }
    if provider == "omp" && reference.is_none() {
        // Current OMP stores agent.db under its data root, while legacy auth
        // and models stay in the selected profile's configuration directory.
        roots.push(
            crate::provider_paths::omp_config_dir()?
                .to_string_lossy()
                .into_owned(),
        );
    }
    if provider == "kimi" && reference.is_none() {
        roots.extend(
            crate::kimi_work::paths()
                .into_iter()
                .map(|(root, _)| root.to_string_lossy().into_owned()),
        );
    }
    if provider == "codebuddy" && reference.is_none() {
        roots.extend(
            crate::codebuddy_ide::paths()
                .into_iter()
                .map(|root| root.to_string_lossy().into_owned()),
        );
    }
    let roots: Vec<String> = roots
        .into_iter()
        .map(|s| {
            std::fs::canonicalize(expand(&s))
                .unwrap_or_else(|_| expand(&s))
                .to_string_lossy()
                .into()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(roots)
}

fn load_inner(
    provider: &str,
    machine: &str,
    settings: &Settings,
    reference: Option<&NativeConnection>,
) -> Result<Loaded> {
    let remote = machine != "local";
    let roots = scan_roots(provider, machine, settings, reference)?;
    let (env, mut files) = spec(provider);
    if !remote
        && provider == "cline"
        && reference.is_none()
        && let Some(path) =
            std::env::var_os("CLINE_PROVIDER_SETTINGS_PATH").filter(|p| !p.is_empty())
    {
        ensure!(
            Path::new(&path).is_absolute(),
            "原生目录环境变量应为绝对路径"
        );
        files.clear();
    }
    if remote {
        let host = settings
            .hosts
            .iter()
            .find(|h| h.id == machine)
            .context("机器已移除")?;
        let mut request = json!({"provider":provider,"roots":roots,"env":env,"patterns":files,"primary":default_root(provider,true)?});
        if provider == "codebuddy" {
            request["codebuddyIdeScript"] = json!(crate::ssh::CODEBUDDY_IDE_SCRIPT);
        }
        if let Some(r) = reference {
            request["env"] = json!("");
            request["patterns"] = json!([]);
            request["primary"] = json!(r.root);
            request["pathsOnly"] = json!(reference_paths(r));
        }
        let encoded = STANDARD.encode(serde_json::to_vec(&request)?);
        let script = include_str!("../../../scripts/remote_account_scan.py");
        let command = crate::ssh::command(
            host,
            &format!("exec python3 - {}", crate::process::quote(&encoded)),
        )?;
        let output =
            crate::process::run(command, script.as_bytes().to_vec(), Duration::from_secs(35))
                .map_err(|_| {
                    anyhow::anyhow!("SSH 扫描失败，请检查连接、Python 3 和配置读取权限")
                })?;
        ensure!(output.len() <= 12 * 1024 * 1024, "远端配置总量超过扫描限制");
        let result: Value =
            serde_json::from_slice(&output).map_err(|_| anyhow::anyhow!("SSH 扫描协议无效"))?;
        let mut loaded = Loaded {
            roots: result["roots"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            ..Default::default()
        };
        for row in result["documents"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_FILES)
        {
            let bytes = STANDARD
                .decode(text(&row["data"]))
                .map_err(|_| anyhow::anyhow!("SSH 配置编码无效"))?;
            ensure!(bytes.len() <= LIMIT as usize, "原生配置超过 1 MiB");
            loaded.documents.push(Document {
                root: text(&row["root"]).into(),
                path: text(&row["path"]).into(),
                bytes,
            });
        }
        loaded.aliases = serde_json::from_value(result["aliases"].clone()).unwrap_or_default();
        loaded.errors = result["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        if provider == "copilot" {
            loaded.documents.sort_by_key(|d| copilot_priority(&d.path));
        }
        return Ok(loaded);
    }
    let mut loaded = Loaded {
        roots: roots.clone(),
        ..Default::default()
    };
    let mut paths = BTreeSet::new();
    for root in &roots {
        if provider == "codebuddy" && crate::codebuddy_ide::user_id(Path::new(root)).is_some() {
            paths.insert((root.clone(), PathBuf::from(root)));
        }
        for pattern in files.iter().filter(|_| reference.is_none()) {
            for path in patterns(Path::new(root), pattern) {
                paths.insert((root.clone(), path));
            }
        }
        if provider == "opencode" && reference.is_none() {
            paths.insert((root.clone(), Path::new(root).join("opencode.db")));
        }
    }
    let primary = expand(
        &reference
            .map(|r| Ok(r.root.clone()))
            .unwrap_or_else(|| default_root(provider, false))?,
    )
    .to_string_lossy()
    .into_owned();
    for path in reference
        .map(reference_paths)
        .unwrap_or_else(|| extra_paths(provider))
    {
        let anchor = if reference.is_none()
            && provider == "claude"
            && Path::new(&path) == crate::store::home().join(".claude.json")
        {
            let root = crate::store::home().join(".claude");
            std::fs::canonicalize(&root)
                .unwrap_or(root)
                .to_string_lossy()
                .into_owned()
        } else {
            primary.clone()
        };
        paths.insert((anchor, PathBuf::from(path)));
    }
    if provider == "kimi" && reference.is_none() {
        for (root, config) in crate::kimi_work::paths() {
            paths.insert((root.to_string_lossy().into_owned(), config));
        }
    }
    for (root, path) in paths.into_iter().take(MAX_FILES) {
        if !path.exists() {
            continue;
        }
        match read_file(&root, &path) {
            Ok(doc) => loaded.documents.push(doc),
            Err(error) => loaded.errors.push(format!("{}：{error}", path.display())),
        }
    }
    if provider == "copilot" {
        loaded.documents.extend(copilot_tokens(&primary, reference));
        loaded.documents.sort_by_key(|d| copilot_priority(&d.path));
    }
    if provider == "kilo" {
        for (name, replaces_auth) in [("KILO_AUTH_CONTENT", true), ("KILO_CONFIG_CONTENT", false)] {
            let path = format!("env:{name}");
            if reference.is_none_or(|r| r.config_path == path)
                && let Ok(value) = std::env::var(name)
                && !value.trim().is_empty()
            {
                ensure!(value.len() <= LIMIT as usize, "原生配置超过 1 MiB");
                if replaces_auth {
                    loaded.documents.retain(|d| !d.path.ends_with("auth.json"));
                }
                loaded.documents.push(Document {
                    root: primary.clone(),
                    path,
                    bytes: value.into_bytes(),
                });
            }
        }
    }
    // The macOS keychain is read only after the one-time consent (claudeAuth.keychain).
    Ok(loaded)
}

/// Copilot tokens that are not files: the first non-empty token variable (the
/// CLI's own precedence) and the token Aieyes was granted by device-code login.
/// Neither is written anywhere; the document exists only in memory. The generic
/// `GH_TOKEN` / `GITHUB_TOKEN` count only once Copilot has run here (its home
/// exists), so a token kept for other GitHub tools does not become an account.
fn copilot_tokens(root: &str, reference: Option<&NativeConnection>) -> Vec<Document> {
    let wanted = |path: &str| reference.is_none_or(|r| r.config_path == path);
    let mut documents = vec![];
    if let Some((name, token)) = copilot_environment(root, |name| std::env::var(name).ok()) {
        let path = format!("env:{name}");
        if wanted(&path) {
            documents.push(Document {
                root: root.into(),
                path,
                bytes: json!({"token":token}).to_string().into_bytes(),
            });
        }
    }
    if crate::copilot_auth::enabled()
        && wanted(COPILOT_GRANTED)
        && let Ok(token) = crate::credentials::read_agent(crate::copilot_auth::REFERENCE)
    {
        documents.push(Document {
            root: root.into(),
            path: COPILOT_GRANTED.into(),
            bytes: json!({"token":token}).to_string().into_bytes(),
        });
    }
    documents
}
fn copilot_environment(
    root: &str,
    read: impl Fn(&str) -> Option<String>,
) -> Option<(&'static str, String)> {
    let (name, token) = COPILOT_ENV.iter().find_map(|name| {
        read(name)
            .filter(|token| !token.trim().is_empty())
            .map(|token| (*name, token.trim().to_owned()))
    })?;
    if (name != "COPILOT_GITHUB_TOKEN" && !Path::new(root).is_dir())
        || token.len() > 32768
        || token.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some((name, token))
}
const OPENROUTER: &str = "https://openrouter.ai/api/";
/// Cline `providers.json`: the Cline account login and each provider's key.
fn cline_rows(machine: &str, doc: &Document, v: &Value) -> Vec<Candidate> {
    let mut rows = vec![];
    for (name, entry) in v["providers"].as_object().into_iter().flatten() {
        let settings = &entry["settings"];
        let selector = format!("/providers/{}/settings", ptr(name));
        let provider = settings["provider"].as_str().unwrap_or(name);
        if provider == "cline" {
            let auth = &settings["auth"];
            let token = auth["accessToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(settings["apiKey"].as_str())
                .or(auth["apiKey"].as_str())
                .unwrap_or("");
            if token.is_empty() {
                continue;
            }
            let before = rows.len();
            login(
                &mut rows,
                "cline",
                machine,
                doc,
                &if auth["accessToken"].is_string() {
                    format!("{selector}/auth")
                } else {
                    selector.clone()
                },
                &json!({"accessToken":token,"accountId":auth["accountId"],"expiresAt":auth["expiresAt"]}),
            );
            for row in &mut rows[before..] {
                if row.connection.status != "stale" {
                    row.connection.note = "已识别 Cline 账户".into();
                }
            }
            continue;
        }
        let key = settings["apiKey"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(settings["auth"]["apiKey"].as_str())
            .unwrap_or("");
        let base = match text(&settings["baseUrl"]) {
            "" if provider == "openrouter" => "https://openrouter.ai/api/v1",
            base => base,
        };
        api(
            &mut rows,
            "cline",
            machine,
            doc,
            &selector,
            settings,
            base,
            key,
            vec![text(&settings["model"]).into()],
        );
    }
    rows
}
const COPILOT_ENV: [&str; 3] = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];
pub(crate) const COPILOT_GRANTED: &str = "aieyes:copilot-github";
fn copilot_env(path: &str) -> bool {
    path.strip_prefix("env:")
        .is_some_and(|name| COPILOT_ENV.contains(&name))
}
fn copilot_priority(path: &str) -> u8 {
    if let Some(index) = path
        .strip_prefix("env:")
        .and_then(|name| COPILOT_ENV.iter().position(|value| *value == name))
    {
        index as u8
    } else if path == COPILOT_GRANTED {
        4
    } else {
        3
    }
}
/// Not a file on disk: no file signature, and never sent to another machine.
fn virtual_path(path: &str) -> bool {
    path.starts_with("env:") || path.starts_with("aieyes:")
}
/// Claude on macOS keeps the default directory's login in the Keychain, not in a file.
fn keychain_account(machine: &str, doc: &Document) -> bool {
    cfg!(target_os = "macos")
        && machine == "local"
        && Path::new(&doc.path) == crate::store::home().join(".claude.json")
        && crate::claude_keychain::default_dir(Path::new(&doc.root))
        && !Path::new(&doc.root).join(".credentials.json").exists()
}
/// Copilot / GitHub token rows. github.com only: the quota endpoint is api.github.com.
fn copilot_rows(machine: &str, doc: &Document) -> Result<Vec<Candidate>> {
    let path = doc.path.replace('\\', "/");
    let v: Value = if path.ends_with("config.json") {
        // Valid but unsupported/unsigned JSONC is not logged in. A torn file
        // is different: propagate its error to the shared snapshot/grace path.
        serde_json::from_slice(&jsonc(&doc.bytes))
            .map_err(|_| anyhow::anyhow!("原生 JSON 格式不支持"))?
    } else {
        parse(doc)?
    };
    let mut found: Vec<(String, String, String)> = vec![];
    if virtual_path(&doc.path) {
        if !copilot_env(&doc.path) && !(machine == "local" && doc.path == COPILOT_GRANTED) {
            return Ok(vec![]);
        }
        found.push((String::new(), text(&v["token"]).into(), String::new()));
    } else if path.ends_with("config.json") {
        let last = v["last_logged_in_user"]["login"]
            .as_str()
            .or(v["lastLoggedInUser"]["login"].as_str())
            .unwrap_or("");
        for key in ["copilot_tokens", "copilotTokens"] {
            for (name, token) in v[key].as_object().into_iter().flatten() {
                // Keys look like `https://github.com:login`.
                let Some(login) = name
                    .strip_prefix("https://github.com:")
                    .or_else(|| name.strip_prefix("github.com:"))
                else {
                    continue;
                };
                found.push((
                    format!("/{key}/{}", ptr(name)),
                    text(token).into(),
                    login.into(),
                ));
            }
        }
        if found.len() > 1 && !last.is_empty() {
            found.retain(|(_, _, login)| login.eq_ignore_ascii_case(last));
        }
    } else {
        // apps.json (`github.com:<app id>`), legacy hosts.json and gh hosts.yml (`github.com`).
        for (name, entry) in v.as_object().into_iter().flatten() {
            if name != "github.com" && !name.starts_with("github.com:") {
                continue;
            }
            found.push((
                format!("/{}", ptr(name)),
                text(&entry["oauth_token"]).into(),
                text(&entry["user"]).into(),
            ));
        }
    }
    // SSH environment secrets stay on the owning host. Its projection carries
    // only a synthetic token and the hash used by the pinned quota transport.
    let remote_fingerprint = if machine != "local" && copilot_env(&doc.path) {
        let fingerprint = text(&v["aieyesTokenFingerprint"]);
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(vec![]);
        }
        Some(fingerprint)
    } else {
        None
    };
    Ok(found
        .into_iter()
        .filter(|(_, token, _)| {
            !token.is_empty()
                && token.len() <= 32768
                && !token.chars().any(|c| c.is_whitespace() || c.is_control())
        })
        .map(|(selector, token, login)| {
            let mut row = candidate("copilot", machine, doc, &selector, "login", &token);
            if let Some(fingerprint) = remote_fingerprint {
                row.fingerprint = fingerprint.into();
            }
            // A cached username is display metadata, never a stable account.
            // Only a successful `/user` read can assign the numeric identity.
            row.connection.email = public_text(&login);
            row.connection.note = if token.starts_with("ghp_") {
                "经典 ghp_ 令牌不支持 Copilot 额度".into()
            } else if virtual_path(&doc.path) {
                "已识别 GitHub 令牌".into()
            } else {
                "已识别 GitHub 登录".into()
            };
            row
        })
        .collect())
}
fn bind_github_identity(row: &mut NativeConnection, identity: &crate::upstream::Identity) {
    row.identity_key = Some(format!("copilot:{}", identity.subject));
    row.upstream_identity = Some(identity.upstream.clone());
    row.email = public_text(&identity.email);
}
fn github_connection(row: &NativeConnection) -> bool {
    row.kind == "login" && (row.provider == "copilot" || upstream_kind(row) == Some("copilot"))
}
fn numeric_github(row: &NativeConnection) -> bool {
    row.upstream_identity
        .as_deref()
        .and_then(crate::upstream::github_id)
        .is_some()
}
/// Files, env and Aieyes grants all require GitHub's stable numeric subject.
/// Classic `ghp_` tokens cannot read Copilot quota and are never probed.
fn needs_github_identity(c: &Candidate) -> bool {
    c.connection.provider == "copilot"
        && c.connection.kind == "login"
        && !c.secret.is_empty()
        && !c.secret.starts_with("ghp_")
}
/// The login an earlier scan resolved for this exact token: same connection and
/// same fingerprint. A changed token is resolved again, never inherited.
fn reuse_verified_identity(
    store: &crate::store::Store,
    settings: &Settings,
    c: &mut Candidate,
) -> bool {
    use rusqlite::OptionalExtension;
    let Some(old) = settings
        .native_connections
        .iter()
        .find(|r| r.id == c.connection.id && r.identity_key.is_some())
    else {
        return false;
    };
    if github_connection(&c.connection) && !numeric_github(old) {
        return false;
    }
    if upstream_kind(&c.connection) == Some("gemini")
        && !old
            .upstream_identity
            .as_deref()
            .is_some_and(|id| id.starts_with("google-gemini-cli:"))
    {
        return false;
    }
    let stored: Option<String> = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("native-fingerprint:{}", old.id)],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if stored.as_deref() != Some(c.fingerprint.as_str()) {
        return false;
    }
    c.connection.identity_key = old.identity_key.clone();
    c.connection.upstream_identity = old.upstream_identity.clone();
    c.connection.email = old.email.clone();
    true
}
/// Verifies Copilot tokens with `/user`, once per unchanged credential. Legacy
/// hashes are upgraded on the next successful scan. Failure preserves proven
/// history but cannot assign a new token based only on a username in a file.
fn copilot_identities(
    store: &crate::store::Store,
    settings: &Settings,
    candidates: &mut [Candidate],
    lookup: impl Fn(&str) -> Result<crate::upstream::Identity>,
) {
    for c in candidates
        .iter_mut()
        .filter(|c| c.connection.machine_id == "local" && needs_github_identity(c))
    {
        if reuse_verified_identity(store, settings, c) {
            continue;
        }
        match lookup(&c.secret) {
            Ok(identity) => bind_github_identity(&mut c.connection, &identity),
            Err(error) => {
                retain_github_identity(store, settings, c);
                c.connection.note = error.to_string();
            }
        }
    }
}

fn retain_github_identity(store: &crate::store::Store, settings: &Settings, c: &mut Candidate) {
    let old = settings
        .native_connections
        .iter()
        .find(|old| old.id == c.connection.id && old.identity_key.is_some());
    if let Some(old) = old
        && let Some(fingerprint) = stored_fingerprint(store, &old.id)
    {
        copy_identity(c, old);
        // A changed, unverified credential cannot inherit this fingerprint at
        // read time. Retaining it here keeps its historical account intact.
        c.fingerprint = fingerprint;
    } else {
        c.connection.identity_key = None;
        c.connection.upstream_identity = None;
    }
}

/// Upgrade in place before reconciliation mistakes a username rename for an
/// account switch. Only the same credential proves a legacy-to-numeric alias;
/// matching names alone is unsafe because GitHub usernames can be reused.
fn migrate_github_identity(
    store: &crate::store::Store,
    settings: &mut Settings,
    candidate: &Candidate,
) {
    let row = &candidate.connection;
    if !github_connection(row) || !numeric_github(row) || row.identity_key.is_none() {
        return;
    }
    if settings
        .accounts
        .iter()
        .any(|account| account.provider == row.provider && account.identity_key == row.identity_key)
    {
        return;
    }
    let previous = settings.native_connections.iter().find_map(|old| {
        if old.provider != row.provider
            || !github_connection(old)
            || numeric_github(old)
            || old.identity_key.is_none()
            || stored_fingerprint(store, &old.id).as_deref() != Some(&candidate.fingerprint)
        {
            return None;
        }
        let key = old.account_key.as_ref()?;
        settings.accounts.iter().position(|account| {
            accounts::key(account) == *key
                && account.provider == row.provider
                && account.identity_key == old.identity_key
                && account
                    .upstream_identity
                    .as_deref()
                    .and_then(crate::upstream::github_id)
                    .is_none()
        })
    });
    let Some(index) = previous else { return };
    let account = &mut settings.accounts[index];
    let key = accounts::key(account);
    account.identity_key = row.identity_key.clone();
    account.upstream_identity = row.upstream_identity.clone();
    // Account.id, source account IDs, usage and quota history stay untouched.
    for old in &mut settings.native_connections {
        if old.account_key.as_ref() == Some(&key)
            && old.provider == row.provider
            && stored_fingerprint(store, &old.id).as_deref() == Some(&candidate.fingerprint)
        {
            old.identity_key = row.identity_key.clone();
            old.upstream_identity = row.upstream_identity.clone();
            old.email = row.email.clone();
        }
    }
}

fn prepaid_account(row: &NativeConnection) -> bool {
    (row.provider == "cline" && row.kind == "login")
        || (row.provider == "kilo" && row.selector == "/kilo")
}
fn needs_prepaid_identity(c: &Candidate) -> bool {
    c.connection.machine_id == "local"
        && !c.secret.is_empty()
        && c.connection.status != "stale"
        && prepaid_account(&c.connection)
}

/// A Kilo `accountId` selects an organization, not a user. Resolve the user's
/// profile once per credential; repeated scans reuse only an exact fingerprint.
fn prepaid_identities(
    store: &crate::store::Store,
    settings: &Settings,
    candidates: &mut [Candidate],
    lookup: impl Fn(&str, &str) -> Result<AccountIdentity>,
) {
    for c in candidates.iter_mut().filter(|c| needs_prepaid_identity(c)) {
        if reuse_verified_identity(store, settings, c) {
            continue;
        }
        match lookup(&c.connection.provider, &c.secret) {
            Ok(identity) => {
                c.connection.identity_key = Some(identity.key);
                c.connection.email = public_text(&identity.email);
                c.connection.note = "原生客户端已确认账户身份；授权有效性以查询结果为准".into();
            }
            Err(error) => c.connection.note = error.to_string(),
        }
    }
}

// JSONC is parsed without evaluating strings, comments or JavaScript.
fn jsonc(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut i = 0;
    let mut string = false;
    let mut escape = false;
    while i < out.len() {
        if string {
            if escape {
                escape = false;
            } else if out[i] == b'\\' {
                escape = true;
            } else if out[i] == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        if out[i] == b'"' {
            string = true;
            i += 1;
            continue;
        }
        if out[i..].starts_with(b"//") {
            while i < out.len() && out[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if out[i..].starts_with(b"/*") {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i + 1 < out.len() && !out[i..].starts_with(b"*/") {
                out[i] = b' ';
                i += 1;
            }
            if i + 1 < out.len() {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
            }
        } else {
            i += 1;
        }
    }
    string = false;
    escape = false;
    for i in 0..out.len() {
        if string {
            if escape {
                escape = false;
            } else if out[i] == b'\\' {
                escape = true;
            } else if out[i] == b'"' {
                string = false;
            }
            continue;
        }
        if out[i] == b'"' {
            string = true;
            continue;
        }
        if out[i] == b','
            && out[i + 1..]
                .iter()
                .find(|c| !c.is_ascii_whitespace())
                .is_some_and(|c| b"}]".contains(c))
        {
            out[i] = b' ';
        }
    }
    out
}
fn parse(doc: &Document) -> Result<Value> {
    let value = if doc.path.ends_with(".env") {
        // Node util.parseEnv-style literals: no shell or variable expansion.
        let s = std::str::from_utf8(&doc.bytes).context("原生配置编码无效")?;
        let mut values = serde_json::Map::new();
        let mut lines = s.lines();
        while let Some(line) = lines.next() {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty()
                || !key.bytes().enumerate().all(|(i, c)| {
                    c == b'_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit()
                })
            {
                continue;
            }
            let value = value.trim_start();
            let quote = value
                .chars()
                .next()
                .filter(|c| matches!(c, '\'' | '"' | '\u{60}'));
            let value = if let Some(quote) = quote {
                let mut rest = value[1..].to_owned();
                while !rest.contains(quote) {
                    let Some(next) = lines.next() else { break };
                    rest.push('\n');
                    rest.push_str(next);
                }
                let Some(end) = rest.find(quote) else {
                    continue;
                };
                let value = rest[..end].to_owned();
                if quote == '"' {
                    value.replace("\\n", "\n").replace("\\r", "\r")
                } else {
                    value
                }
            } else {
                value.split('#').next().unwrap_or("").trim().to_owned()
            };
            values.insert(key.into(), Value::String(value));
        }
        Value::Object(values)
    } else if doc.path.ends_with(".toml") {
        let s = std::str::from_utf8(&doc.bytes).context("原生配置编码无效")?;
        serde_json::to_value(
            toml::from_str::<toml::Value>(s)
                .map_err(|_| anyhow::anyhow!("原生 TOML 格式不支持"))?,
        )?
    } else if doc.path.ends_with(".yml") || doc.path.ends_with(".yaml") {
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&doc.bytes)
            .map_err(|_| anyhow::anyhow!("原生 YAML 格式不支持"))?;
        fn plain(v: serde_yaml_ng::Value) -> Value {
            match v {
                serde_yaml_ng::Value::Tagged(_) => Value::Null,
                serde_yaml_ng::Value::Sequence(a) => {
                    Value::Array(a.into_iter().map(plain).collect())
                }
                serde_yaml_ng::Value::Mapping(m) => Value::Object(
                    m.into_iter()
                        .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), plain(v))))
                        .collect(),
                ),
                v => serde_json::to_value(v).unwrap_or(Value::Null),
            }
        }
        plain(value)
    } else {
        let bytes = if doc.path.ends_with(".jsonc") {
            jsonc(&doc.bytes)
        } else {
            doc.bytes.clone()
        };
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("原生 JSON 格式不支持"))?
    };
    ensure!(
        value.is_object() || value.is_array(),
        "原生配置应为对象或列表"
    );
    Ok(value)
}
fn candidate(
    provider: &str,
    machine: &str,
    doc: &Document,
    selector: &str,
    kind: &str,
    secret: &str,
) -> Candidate {
    let id = hash(&json!([provider, machine, doc.path, selector, kind]).to_string());
    Candidate {
        connection: NativeConnection {
            id,
            provider: provider.into(),
            machine_id: machine.into(),
            root: doc.root.clone(),
            config_path: doc.path.clone(),
            selector: selector.into(),
            kind: kind.into(),
            has_credential: !secret.is_empty(),
            status: "observed".into(),
            note: "已识别原生配置".into(),
            checked_at: now(),
            ..Default::default()
        },
        fingerprint: hash(secret),
        secret: secret.into(),
        plan: String::new(),
        upstream_auth: None,
    }
}
#[allow(clippy::too_many_arguments)]
fn api(
    rows: &mut Vec<Candidate>,
    provider: &str,
    machine: &str,
    doc: &Document,
    selector: &str,
    route: &Value,
    base: &str,
    key: &str,
    models: Vec<String>,
) {
    if base.is_empty() && key.is_empty() {
        return;
    }
    let mut row = candidate(provider, machine, doc, selector, "api", key);
    row.connection.base_url = url(base);
    row.connection.models = models
        .into_iter()
        .map(|s| public_text(&s))
        .filter(|s| !s.is_empty())
        .take(128)
        .collect();
    row.connection.note = if key.is_empty() {
        "原生 API 配置；凭据由客户端解析"
    } else {
        "原生 API 连接；Key 保留在原配置"
    }
    .into();
    if route.get("type") == Some(&json!("oauth")) {
        return;
    }
    rows.push(row);
}
fn login(
    rows: &mut Vec<Candidate>,
    provider: &str,
    machine: &str,
    doc: &Document,
    selector: &str,
    value: &Value,
) {
    let token = [
        "access_token",
        "accessToken",
        "access",
        "token",
        "key",
        "refresh_token",
        "refreshToken",
        "refresh",
    ]
    .into_iter()
    .find_map(|k| value[k].as_str().filter(|s| !s.is_empty()));
    let Some(token) = token else {
        return;
    };
    let mut row = candidate(provider, machine, doc, selector, "login", token);
    let identity = claims(text(&value["id_token"]));
    let access = claims(token);
    row.connection.email = public_text(
        value["email"]
            .as_str()
            .or(identity["email"].as_str())
            .or(access["email"].as_str())
            .unwrap_or(""),
    );
    if provider == "codex" {
        // Use the existing user + workspace identity exactly, including its namespace.
        if let Ok(id) = crate::codex_auth::files::identity(value) {
            row.connection.upstream_identity = Some(format!("openai:{}", id.key));
            row.connection.identity_key = Some(id.key);
            row.connection.email = public_text(&id.email);
        }
    } else if provider == "antigravity" {
        if matches!(
            text(&identity["iss"]),
            "accounts.google.com" | "https://accounts.google.com"
        ) && (identity["email_verified"] == true || identity["email_verified"] == "true")
            && !text(&identity["sub"]).is_empty()
        {
            row.connection.identity_key = Some(format!("google:{}", hash(text(&identity["sub"]))));
            row.connection.upstream_identity = row.connection.identity_key.clone();
        }
    } else if provider == "kimi" {
        // Coding OAuth subjects are not a documented replacement for /me.user_id.
        // Resolve both opaque and JWT tokens to the same identity as Kimi Work.
    } else if provider == "cline" {
        // The persisted Cline account ID is the /users/me ID; no profile name
        // or organization is part of the personal wallet's identity.
        row.connection.identity_key = value["accountId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|id| format!("cline:{}", hash(id)));
    } else if provider == "kilo" && selector == "/kilo" {
        // OAuth accountId is the selected organization. GET /profile identifies
        // the user, including opaque and manually pasted Kilo tokens.
        row.connection.identity_key = None;
    } else {
        // Only provider-scoped subject/account IDs, never email or token hashes.
        let subject = value["accountId"]
            .as_str()
            .or(value["account_id"].as_str())
            .or(value["userId"].as_str())
            .or(identity["sub"].as_str())
            .or(access["sub"].as_str());
        if let Some(id) = subject.filter(|s| !s.is_empty()) {
            row.connection.identity_key = Some(format!(
                "{provider}:{}",
                hash(
                    &json!([
                        selector,
                        identity["iss"],
                        access["iss"],
                        id,
                        value["organizationId"],
                        identity["organization_id"],
                        access["organization_id"]
                    ])
                    .to_string()
                )
            ));
        }
    }
    let expires = value["expires_at"]
        .as_i64()
        .or(value["expiresAt"].as_i64())
        .or(value["expires"].as_i64())
        .or(access["exp"].as_i64());
    // The native client refreshes its own token on next launch; stay quiet for a day.
    if let Some(at) = expires
        .map(|t| if t > 10_000_000_000 { t / 1000 } else { t })
        .filter(|t| *t <= now())
    {
        row.connection.token_expired_at = Some(at);
        if now() - at >= STALE_AFTER {
            row.connection.status = "stale".into();
            row.connection.note = stale_note(provider);
        }
    }
    rows.push(row);
}
fn model_names(route: &Value) -> Vec<String> {
    route["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.as_str()
                .or(v["id"].as_str())
                .or(v["model"].as_str())
                .map(str::to_owned)
        })
        .chain(route["model"].as_str().map(str::to_owned))
        .collect()
}
/// Cursor's membership as the IDE caches it (`stripeMembershipType`).
fn cursor_plan(membership: &str) -> String {
    match membership.trim().to_ascii_lowercase().as_str() {
        "" => String::new(),
        "free" => "Cursor Free".into(),
        "free_trial" => "Cursor Pro Trial".into(),
        "pro" => "Cursor Pro".into(),
        "pro_plus" | "pro-plus" => "Cursor Pro+".into(),
        "ultra" => "Cursor Ultra".into(),
        "business" | "team" | "teams" => "Cursor Teams".into(),
        "enterprise" => "Cursor Enterprise".into(),
        other => public_text(other).chars().take(64).collect(),
    }
}
fn extract(provider: &str, machine: &str, doc: &Document) -> Result<Vec<Candidate>> {
    let mut rows = extract_rows(provider, machine, doc)?;
    for row in &mut rows {
        upstream(provider, doc, row);
    }
    Ok(rows)
}
/// Server identities shared with another agent's own login, so the same
/// upstream account seen through several clients is queried once.
fn upstream(provider: &str, doc: &Document, row: &mut Candidate) {
    let c = &mut row.connection;
    if provider == "cursor" && c.kind == "login" {
        // IDE and CLI logins of one Cursor user are one account.
        let sub = text(&claims(&row.secret)["sub"]).to_owned();
        if !sub.is_empty() {
            c.identity_key = Some(format!("cursor:{}", hash(&sub)));
            c.upstream_identity = c.identity_key.clone();
        }
    }
    // OpenCode / Pi logins for Anthropic, ChatGPT or Copilot: the service's own
    // token, expiry rules and (when the token carries it) server identity.
    let Some(kind) = (c.kind == "login")
        .then(|| crate::upstream::kind(provider, &c.selector))
        .flatten()
    else {
        return;
    };
    let Ok(v) = parse(doc) else {
        return;
    };
    let entry = &v[c.selector.trim_start_matches('/')];
    let Some(token) = crate::upstream::token(kind, entry) else {
        // e.g. GitHub Enterprise: listed, never queried.
        row.secret.clear();
        row.fingerprint = hash("");
        c.has_credential = false;
        return;
    };
    // Generic JWT subjects and cached emails are not the upstream account.
    c.identity_key = None;
    c.upstream_identity = None;
    row.upstream_auth = Some(crate::upstream::auth(kind, &token, entry));
    row.fingerprint = hash(&row.upstream_auth.as_ref().unwrap().to_string());
    c.token_expired_at = crate::upstream::expired_at(kind, entry);
    c.status = "observed".into();
    c.note = crate::upstream::note(kind).into();
    if let Some(at) = c.token_expired_at
        && now() - at >= STALE_AFTER
    {
        c.status = "stale".into();
        c.note = stale_note(provider);
    }
    if let Some(id) = crate::upstream::local_identity(kind, &token, entry) {
        c.identity_key = Some(format!(
            "{provider}:{}",
            hash(&format!("{kind}\0{}", id.subject))
        ));
        c.upstream_identity = Some(id.upstream);
        if c.email.is_empty() {
            c.email = public_text(&id.email);
        }
    }
    row.secret = token;
}
fn extract_rows(provider: &str, machine: &str, doc: &Document) -> Result<Vec<Candidate>> {
    if crate::subscriptions::trae::family(provider) {
        return Ok(trae_rows(provider, machine, doc, &parse(doc)?));
    }
    if provider == "copilot" {
        return copilot_rows(machine, doc);
    }
    if provider == "cline" {
        let value = parse(doc)?;
        if value["providers"].is_object() {
            return Ok(cline_rows(machine, doc, &value));
        }
    }
    if provider == "kiro" {
        return Ok(kiro_rows(machine, doc, &parse(doc)?));
    }
    let v = match parse(doc) {
        // Pi rewrites auth.json in place; an unreadable one means not logged in.
        Err(_) if matches!(provider, "pi" | "omp") && doc.path.ends_with("auth.json") => {
            return Ok(vec![]);
        }
        v => v?,
    };
    if provider == "grok" && doc.path.ends_with("auth.json") {
        let entries: Vec<_> = v
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(scope, entry)| {
                (scope.starts_with("https://auth.x.ai::")
                    || scope.as_str() == "https://accounts.x.ai/sign-in")
                    && entry["key"].as_str().is_some_and(|s| !s.is_empty())
            })
            .collect();
        let oidc: Vec<_> = entries
            .iter()
            .filter(|(scope, _)| scope.starts_with("https://auth.x.ai::"))
            .map(|(_, v)| *v)
            .collect();
        let all: Vec<_> = entries.iter().map(|(_, v)| *v).collect();
        let latest = crate::subscriptions::grok_latest(if oidc.is_empty() { &all } else { &oidc });
        let mut rows = vec![];
        for (scope, entry) in entries {
            let mut value = entry.clone();
            if let Some(id) = entry["user_id"].as_str() {
                value["userId"] = json!(id);
            }
            login(
                &mut rows,
                provider,
                machine,
                doc,
                &format!("/{}", ptr(scope)),
                &value,
            );
            if let Some(row) = rows.last_mut() {
                row.connection.base_url = "https://cli-chat-proxy.grok.com/v1".into();
                if let Some(expiry) =
                    crate::subscriptions::grok_expiry(entry).filter(|t| *t <= now())
                {
                    row.connection.token_expired_at = Some(expiry);
                    if now() - expiry >= STALE_AFTER {
                        row.connection.status = "stale".into();
                        row.connection.note = stale_note(provider);
                    }
                }
                if latest.is_none_or(|latest| !std::ptr::eq(latest, entry)) {
                    row.connection.status = "remembered".into();
                    row.connection.note = "已记住的 Grok 登录；监测账户可手动选择".into();
                }
            }
        }
        return Ok(rows);
    }
    let mut rows = vec![];
    if provider == "codebuddy"
        && let Some(user) = v["codebuddyHistoryUserId"]
            .as_str()
            .filter(|s| !s.is_empty())
    {
        let mut row = candidate(provider, machine, doc, "/history/user_id", "login", "");
        row.connection.identity_key = Some(format!("codebuddy:{}", hash(user)));
        row.connection.status = "remembered".into();
        row.fingerprint = hash(user);
        row.connection.note = "已识别 CodeBuddy IDE 历史账户；余额需登录查看".into();
        rows.push(row);
        return Ok(rows);
    }
    if provider == "kimi" && doc.path.replace('\\', "/").ends_with("daimon/config.json") {
        // Work's web token is not a Kimi Code OAuth token. Keep identity only;
        // when the CLI has the same user, its quota source serves this account.
        if let Some(user) = v["credentials"]["kimiWeb"]["userId"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            let mut row = candidate(provider, machine, doc, "/credentials/kimiWeb", "login", "");
            row.connection.identity_key = Some(format!("kimi:{}", hash(user)));
            row.connection.upstream_identity = row.connection.identity_key.clone();
            row.fingerprint = hash(user);
            row.connection.note = "Kimi Work 与 Kimi Code 共用账户额度".into();
            rows.push(row);
        }
        return Ok(rows);
    }
    if provider == "cursor" && doc.path.ends_with("state.vscdb") {
        let auth = &v["cursorAuth"];
        let login_value = json!({"accessToken":auth["accessToken"],"email":auth["cachedEmail"]});
        login(
            &mut rows,
            provider,
            machine,
            doc,
            "/cursorAuth",
            &login_value,
        );
        if rows.is_empty() && auth["encrypted"] == true {
            // A sealed token: show who is signed in; opening Cursor does not change it.
            let mut row = candidate(provider, machine, doc, "/cursorAuth", "login", "");
            row.connection.email = public_text(text(&auth["cachedEmail"]));
            row.connection.note = "Cursor 登录令牌已加密保存；额度请在 Cursor 中查看".into();
            rows.push(row);
        }
        let plan = cursor_plan(text(&auth["stripeMembershipType"]));
        for row in &mut rows {
            row.plan = plan.clone();
        }
        return Ok(rows);
    }
    if provider == "workbuddy" && doc.path.ends_with("account-snapshot.json") {
        let primary = &v["primary"];
        let uid = text(&primary["uid"]);
        if !uid.is_empty() && uid.len() <= 256 {
            let mut row = candidate(provider, machine, doc, "/primary", "login", "");
            row.fingerprint = hash(uid);
            row.connection.identity_key = Some(format!("workbuddy:{}", hash(uid)));
            row.connection.email = public_text(text(&primary["nickname"]));
            // `ultimate` / `exclusive` are WorkBuddy's enterprise editions.
            row.connection.note = if matches!(text(&primary["type"]), "ultimate" | "exclusive") {
                "已识别 WorkBuddy 企业账户；余额需登录查看"
            } else {
                "已识别 WorkBuddy 个人账户；余额需登录查看"
            }
            .into();
            rows.push(row);
        }
        return Ok(rows);
    }
    if provider == "workbuddy" && doc.path.ends_with("workbuddy.db") {
        if let Some(user) = v["workbuddy"]["userId"].as_str().filter(|s| !s.is_empty()) {
            // No credential is copied; the row only remembers which account signed in.
            let mut row = candidate(provider, machine, doc, "/sessions/user_id", "login", "");
            row.fingerprint = hash(user);
            row.connection.identity_key = Some(format!("workbuddy:{}", hash(user)));
            row.connection.note = "已识别 WorkBuddy 账户；余额需登录查看".into();
            rows.push(row);
        }
        return Ok(rows);
    }
    if provider == "codex" && doc.path.ends_with("auth.json") {
        if let Some(key) = v["OPENAI_API_KEY"].as_str().filter(|s| !s.is_empty()) {
            api(
                &mut rows,
                provider,
                machine,
                doc,
                "/OPENAI_API_KEY",
                &v,
                "",
                key,
                vec![],
            );
        }
        if v["tokens"].is_object() {
            let mut auth = v.clone();
            auth["access_token"] = v["tokens"]["access_token"].clone();
            login(&mut rows, provider, machine, doc, "", &auth);
        }
        return Ok(rows);
    }
    if provider == "claude" {
        login(
            &mut rows,
            provider,
            machine,
            doc,
            "/claudeAiOauth",
            &v["claudeAiOauth"],
        );
        // The macOS login is in the Keychain, which only quota queries read after the
        // one-time consent (claude_keychain). The account itself needs no secret.
        if keychain_account(machine, doc)
            && let Some(uuid) = v["oauthAccount"]["accountUuid"]
                .as_str()
                .filter(|s| !s.is_empty())
        {
            let mut row = candidate(provider, machine, doc, "/oauthAccount", "login", "");
            row.fingerprint = hash(uuid);
            row.connection.note = "已识别 Claude Code 账户；登录保存在钥匙串".into();
            rows.push(row);
        }
        let env = &v["env"];
        let key = env["ANTHROPIC_AUTH_TOKEN"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(text(&env["ANTHROPIC_API_KEY"]));
        api(
            &mut rows,
            provider,
            machine,
            doc,
            "/env",
            env,
            env["ANTHROPIC_BASE_URL"]
                .as_str()
                .unwrap_or("https://api.anthropic.com"),
            key,
            vec![text(&env["ANTHROPIC_MODEL"]).into()],
        );
        // settings without either a credential or explicit endpoint are not connections.
        if key.is_empty() && env["ANTHROPIC_BASE_URL"].is_null() {
            rows.retain(|r| r.connection.kind != "api");
        }
    }
    if provider == "antigravity" {
        login(&mut rows, provider, machine, doc, "", &v);
    }
    if doc.path.ends_with("auth.json")
        || provider == "kilo" && doc.path == "env:KILO_AUTH_CONTENT"
        || doc.path.ends_with("opencode.db")
        || provider == "omp" && doc.path.ends_with("agent.db")
        || doc.path.ends_with("credentials.json")
        || doc.path.ends_with("cli-config.json")
        || provider == "kimi"
            && (doc.path.replace('\\', "/").contains("/oauth/")
                || doc.path.replace('\\', "/").contains("/credentials/"))
    {
        if provider != "claude" {
            login(&mut rows, provider, machine, doc, "", &v);
        }
        if let Some(map) = v.as_object() {
            for (name, value) in map {
                let selector = format!("/{}", ptr(name));
                if value.is_object() {
                    if value["type"] == "oauth"
                        || name.starts_with("https://auth.x.ai")
                        || name == "https://accounts.x.ai/sign-in"
                    {
                        if provider == "kilo" && name == "kilo" && text(&value["access"]).is_empty()
                        {
                            continue;
                        }
                        login(&mut rows, provider, machine, doc, &selector, value);
                    } else if value["type"] == "api"
                        || provider == "opencode" && value["type"] == "key"
                    {
                        // `key` is the OpenCode 2 credential form of an API key.
                        api(
                            &mut rows,
                            provider,
                            machine,
                            doc,
                            &selector,
                            value,
                            text(&value["baseUrl"]),
                            value["key"].as_str().unwrap_or(text(&value["apiKey"])),
                            vec![],
                        );
                    }
                }
            }
        }
    }
    if provider == "codebuddy" {
        let env = &v["env"];
        api(
            &mut rows,
            provider,
            machine,
            doc,
            "/env",
            env,
            text(&env["CODEBUDDY_BASE_URL"]),
            text(&env["CODEBUDDY_API_KEY"]),
            vec![text(&env["CODEBUDDY_MODEL"]).into()],
        );
        for (i, model) in v
            .as_array()
            .or(v["models"].as_array())
            .into_iter()
            .flatten()
            .enumerate()
        {
            api(
                &mut rows,
                provider,
                machine,
                doc,
                &format!("{}/{}", if v.is_array() { "" } else { "/models" }, i),
                model,
                text(&model["url"]),
                text(&model["apiKey"]),
                vec![text(&model["id"]).into()],
            );
        }
    }
    for section in ["providers", "provider", "model", "model_providers"] {
        if let Some(map) = v[section].as_object() {
            for (name, route) in map {
                if !route.is_object() {
                    continue;
                }
                let options = route.get("options").unwrap_or(route);
                let base = options["baseURL"]
                    .as_str()
                    .or(options["baseUrl"].as_str())
                    .or(options["base_url"].as_str())
                    .unwrap_or("");
                let base = if provider == "kilo" && name == "openrouter" && base.is_empty() {
                    "https://openrouter.ai/api/v1"
                } else {
                    base
                };
                let key = options["apiKey"]
                    .as_str()
                    .or(options["api_key"].as_str())
                    .unwrap_or("");
                let mut models = model_names(route);
                if provider == "kilo"
                    && let Some(known) = route["models"].as_object()
                {
                    models.extend(known.keys().cloned());
                }
                if provider == "kimi"
                    && let Some(map) = v["models"].as_object()
                {
                    models.extend(
                        map.values()
                            .filter(|m| m["provider"] == *name)
                            .filter_map(|m| m["model"].as_str().map(str::to_owned)),
                    );
                }
                if provider == "codex" {
                    models.extend(v["model"].as_str().map(str::to_owned));
                }
                api(
                    &mut rows,
                    provider,
                    machine,
                    doc,
                    &format!("/{}/{}", section, ptr(name)),
                    route,
                    base,
                    key,
                    models,
                );
            }
        }
    }
    if provider == "zcode" {
        for (i, rule) in v["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            api(
                &mut rows,
                provider,
                machine,
                doc,
                &format!("/config/providerConfigRules/providerRules/{i}"),
                rule,
                text(&rule["config"]["api"]["baseUrl"]),
                text(&rule["config"]["access"]["apiKey"]),
                vec![],
            );
        }
    }
    if provider == "zcode"
        && doc
            .path
            .replace('\\', "/")
            .ends_with("/v2/credentials.json")
    {
        zcode_login(&mut rows, machine, doc, &v)?;
    }
    if provider == "deepseek-harness" && doc.path.ends_with(".credentials.yaml") {
        ensure!(
            v["version"] == 1 || v.as_object().is_some_and(|v| v.is_empty()),
            "DeepSeek 凭据版本不支持，请在 dsh 中重新登录"
        );
        if let Some(map) = v["records"].as_object() {
            for (name, record) in map
                .iter()
                .filter(|(name, _)| name.as_str() == "deepseek-account-platform/default")
            {
                if record["kind"] == "grant"
                    && record["payload"]["version"] == 1
                    && record["payload"]["token"].is_string()
                {
                    let payload = &record["payload"];
                    let selector = format!("/records/{}", ptr(name));
                    let before = rows.len();
                    login(&mut rows, provider, machine, doc, &selector, payload);
                    if let Some(row) = rows.get_mut(before) {
                        row.connection.base_url = url(text(&payload["issuer"]));
                    }
                }
            }
        }
        if let Some(key) = v["refs"]["DEEPSEEK_API_KEY"].as_str() {
            api(
                &mut rows,
                provider,
                machine,
                doc,
                "/refs/DEEPSEEK_API_KEY",
                &v["refs"],
                DEEPSEEK_API,
                key,
                vec![],
            );
        }
        return Ok(rows);
    }
    if provider == "deepseek-harness" && doc.path.ends_with(".env") {
        if let Some(key) = v["DEEPSEEK_API_KEY"].as_str().filter(|s| !s.is_empty()) {
            let base = v["DEEPSEEK_BASE_URL"].as_str().unwrap_or(DEEPSEEK_API);
            api(
                &mut rows,
                provider,
                machine,
                doc,
                "/DEEPSEEK_API_KEY",
                &v,
                base,
                key,
                vec![],
            );
        }
        return Ok(rows);
    }
    if provider == "deepseek-harness" {
        for (i, plugin) in v.as_array().into_iter().flatten().enumerate() {
            if let Some(map) = plugin["config"]["providers"].as_object() {
                for (name, route) in map {
                    api(
                        &mut rows,
                        provider,
                        machine,
                        doc,
                        &format!("/{i}/config/providers/{}", ptr(name)),
                        route,
                        text(&route["baseURL"]),
                        text(&route["headers"]["Authorization"])
                            .strip_prefix("Bearer ")
                            .unwrap_or(""),
                        model_names(route),
                    );
                }
            }
        }
    }
    Ok(rows)
}

const DEEPSEEK_API: &str = "https://api.deepseek.com";
/// Moonshot open-platform keys in Kimi's config.toml can read the account balance.
fn moonshot_host(base: &str) -> Option<&'static str> {
    let host = reqwest::Url::parse(base).ok()?.host_str()?.to_owned();
    match host.as_str() {
        "api.moonshot.cn" => Some("moonshot-cn"),
        "api.moonshot.ai" => Some("moonshot-global"),
        _ => None,
    }
}
/// Hosts whose Coding Plan key or login can query the matching region's quota.
pub(crate) fn zcode_region(base: &str) -> Option<&'static str> {
    let host = base
        .split("://")
        .nth(1)
        .unwrap_or(base)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    if host == "z.ai" || host.ends_with(".z.ai") {
        Some("global")
    } else if host == "bigmodel.cn" || host.ends_with(".bigmodel.cn") {
        Some("cn")
    } else {
        None
    }
}
/// The v2 store has encrypted flat keys; the legacy spelling is still readable.
fn zcode_login(
    rows: &mut Vec<Candidate>,
    machine: &str,
    doc: &Document,
    raw: &Value,
) -> Result<()> {
    let current = raw.get("zcodejwttoken").is_some()
        || raw.get("oauth:active_provider").is_some()
        || raw
            .as_object()
            .is_some_and(|v| v.keys().any(|k| k.starts_with("account-provider:")));
    let v = if current {
        crate::zcode_credentials::project(raw, machine == "local")?
    } else {
        raw.clone()
    };
    // New standalone manual Coding Plan credentials live in this same store.
    for family in ["zai", "bigmodel"] {
        let provider = format!("account:{family}-individual-coding-plan");
        let identity = text(&v[format!("account-provider:{provider}:identity")]);
        if identity.is_empty() || v[format!("account-provider:{provider}:auth-source")] != "manual"
        {
            continue;
        }
        // The current CLI uses a key-<hash> account namespace for manual keys.
        if !identity.starts_with("key-")
            || !identity
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            continue;
        }
        let key = format!("account-provider:coding-plan:{provider}:account:{identity}:api-key");
        if let Some(secret) = v[&key].as_str().filter(|s| !s.is_empty()) {
            let base = if family == "zai" {
                "https://api.z.ai/api/coding/paas/v4"
            } else {
                "https://open.bigmodel.cn/api/coding/paas/v4"
            };
            api(
                rows,
                "zcode",
                machine,
                doc,
                &format!("/{}", ptr(&key)),
                &Value::Null,
                base,
                secret,
                vec![],
            );
        }
    }
    let Some(token) = v["zcodejwttoken"]
        .as_str()
        .or(v["zcodeJwtToken"].as_str())
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    let active = text(&v["oauth:active_provider"]);
    let info = if current {
        let key = match active {
            "zai" => "oauth:zai:user_info",
            "bigmodel" => "oauth:bigmodel:user_info",
            _ => return Ok(()),
        };
        v[key]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .unwrap_or_default()
    } else {
        v["zaiUserInfo"].clone()
    };
    let jwt = claims(token);
    // BigModel's native profile ID scopes the selected organization/project.
    let scoped = (active == "bigmodel")
        .then(|| info["id"].as_str())
        .flatten();
    let id = scoped
        .or(info["user_id"]
            .as_str()
            .or(info["rawProfile"]["user_id"].as_str()))
        .or(info["userId"].as_str())
        .map(str::to_owned)
        .or(info["userId"].as_i64().map(|n| n.to_string()))
        .or(info["id"].as_str().map(str::to_owned))
        .or(info["id"].as_i64().map(|n| n.to_string()));
    let mut value = json!({
        "token": token,
        "email": info["email"].as_str().or(info["rawProfile"]["email"].as_str())
            .or(jwt["email"].as_str()).unwrap_or(""),
    });
    if let Some(id) = id {
        value["userId"] = json!(id);
    }
    let before = rows.len();
    login(
        rows,
        "zcode",
        machine,
        doc,
        if current {
            "/zcodejwttoken"
        } else {
            "/zcodeJwtToken"
        },
        &value,
    );
    // The region follows the login provider (`zcode login zai|bigmodel`).
    let hint = [
        &v["oauth:active_provider"],
        &v["provider"],
        &v["loginProvider"],
        &info["provider"],
        &info["loginProvider"],
        &jwt["iss"],
    ]
    .into_iter()
    .filter_map(|v| v.as_str())
    .map(str::to_ascii_lowercase)
    .find_map(|s| {
        if s.contains("bigmodel") {
            Some("https://open.bigmodel.cn")
        } else if s == "zai" || s.contains("z.ai") {
            Some("https://api.z.ai")
        } else {
            None
        }
    });
    if let (Some(row), Some(base)) = (rows.get_mut(before), hint) {
        row.connection.base_url = base.into();
        // The selector/cipher spelling does not define an account.
        if let Some(id) = value["userId"].as_str().or(jwt["sub"].as_str()) {
            row.connection.identity_key = Some(format!(
                "zcode:{}",
                hash(&json!([zcode_region(base), id]).to_string())
            ));
        }
    }
    Ok(())
}

fn machine_enabled(settings: &Settings, machine: &str) -> bool {
    if machine == "local" {
        settings.local_device_enabled
    } else {
        settings.hosts.iter().any(|h| h.id == machine && h.enabled)
    }
}
fn agent_enabled(settings: &Settings, provider: &str) -> bool {
    let agent = contracts::provider(provider)
        .map(|p| p.agent_id)
        .unwrap_or(provider);
    settings
        .agents
        .iter()
        .any(|a| a.enabled && contracts::provider(&a.provider).is_some_and(|p| p.agent_id == agent))
}
fn selected(settings: &Settings, provider: &str, machine: &str) -> bool {
    settings
        .agents
        .iter()
        .any(|a| a.provider == provider && a.machine_ids.iter().any(|id| id == machine))
}
fn host_signature(settings: &Settings, machine: &str) -> Value {
    settings
        .hosts
        .iter()
        .find(|h| h.id == machine)
        .map(|h| {
            json!([
                h.target,
                h.port,
                h.username,
                h.identity_file,
                h.auth_mode,
                h.password_ref,
                h.shell
            ])
        })
        .unwrap_or(Value::Null)
}

impl Engine {
    pub(crate) fn scan_accounts(&mut self, params: Value) -> Result<Value> {
        let only: Option<BTreeSet<(String, String)>> = params["targets"].as_array().map(|rows| {
            rows.iter()
                .filter_map(|r| Some((r[0].as_str()?.to_owned(), r[1].as_str()?.to_owned())))
                .collect()
        });
        // Legacy extension histories can exist without a readable login. They
        // are source directories only, never evidence of the active account.
        {
            let tx = self.store.write_transaction()?;
            let mut settings = self.store.settings()?;
            let mut changed = false;
            for provider in ["cline", "kilo", "roo"] {
                if !machine_enabled(&settings, "local")
                    || !agent_enabled(&settings, provider)
                    || !selected(&settings, provider, "local")
                {
                    continue;
                }
                if only
                    .as_ref()
                    .is_some_and(|targets| !targets.contains(&(provider.into(), "local".into())))
                {
                    continue;
                }
                // A bad path override is reported by that provider's load; it
                // must not abort an unrelated target's automatic scan.
                let Ok(paths) = crate::provider_paths::discovery_paths(provider) else {
                    continue;
                };
                for path in paths {
                    let root = expand(&path);
                    if !root.exists()
                        || settings.sources.iter().any(|s| {
                            s.provider == provider && s.host_id.is_none() && expand(&s.path) == root
                        })
                    {
                        continue;
                    }
                    settings.sources.push(Source {
                        id: format!(
                            "scan-{}",
                            &hash(&format!("{provider}:local:{}", root.display()))[..24]
                        ),
                        name: format!("{} · 本机", crate::providers::display_name(provider)),
                        provider: provider.into(),
                        path: root.to_string_lossy().into_owned(),
                        ..Default::default()
                    });
                    changed = true;
                }
            }
            if changed {
                self.store.save_settings(&settings)?;
            }
            tx.commit()?;
        }
        let initial = self.store.settings()?;
        // Every enabled agent on every enabled machine; nothing to do is not an error.
        // Detection is automatic (launch and file changes); there is no manual scope.
        let mut targets = vec![];
        let machines: Vec<String> = std::iter::once("local".to_string())
            .chain(initial.hosts.iter().map(|h| h.id.clone()))
            .filter(|m| machine_enabled(&initial, m))
            .collect();
        for p in contracts::PROVIDERS.iter().filter(|p| {
            p.machines
                && p.id != "custom"
                && p.id != "deepseek"
                && has_files(p.id)
                && agent_enabled(&initial, p.id)
        }) {
            for machine in &machines {
                let target = (p.id.to_string(), machine.clone());
                if only.as_ref().is_none_or(|o| o.contains(&target)) {
                    targets.push(target);
                }
            }
        }
        targets.sort_by_key(|(provider, machine)| {
            (machine != "local", provider.clone(), machine.clone())
        });
        targets.dedup();
        let mut results = vec![];
        let mut failed_machines = std::collections::BTreeMap::new();
        let mut added_accounts = 0;
        let mut added_connections = 0;
        for (provider, machine) in targets {
            crate::operations::check()?;
            self.progress(&format!(
                "扫描 {} · {}",
                contracts::provider(&provider).unwrap().name,
                if machine == "local" {
                    "本机"
                } else {
                    &machine
                }
            ));
            let _lock = crate::file_lock::history(
                &self.store.root,
                &format!("account-scan:{provider}:{machine}"),
            )?;
            let before = self.store.settings()?;
            let host_revision = host_signature(&before, &machine);
            if failed_machines.get(&machine) == Some(&host_revision) {
                results.push(json!({"provider":provider,"machineId":machine,"status":"skipped","message":"此机器的 SSH 连接已失败，修复连接后会自动识别"}));
                continue;
            }
            let loaded = match load(&provider, &machine, &before) {
                Ok(v) => v,
                Err(error) => {
                    if machine != "local" {
                        failed_machines.insert(machine.clone(), host_revision);
                    }
                    results.push(json!({"provider":provider,"machineId":machine,"status":"failed","message":error.to_string()}));
                    continue;
                }
            };
            let mut candidates = vec![];
            // A file that is unreadable or half written keeps its last connection
            // (and so its last quota snapshot) instead of reporting a failure.
            let mut retained = BTreeSet::new();
            let known = |path: &str| {
                before.native_connections.iter().any(|r| {
                    r.provider == provider && r.machine_id == machine && r.config_path == path
                })
            };
            for error in &loaded.errors {
                let path = error.split_once('：').map_or("", |(p, _)| p);
                if !path.is_empty() && known(path) {
                    retained.insert(path.to_owned());
                    results.push(json!({"provider":provider,"machineId":machine,"status":"retained","configPath":path}));
                } else {
                    results.push(json!({"provider":provider,"machineId":machine,"status":"failed","message":error}));
                }
            }
            for doc in &loaded.documents {
                match extract(&provider, &machine, doc) {
                    Ok(mut rows) => candidates.append(&mut rows),
                    Err(_) if known(&doc.path) => {
                        retained.insert(doc.path.clone());
                        results.push(json!({"provider":provider,"machineId":machine,"status":"retained","configPath":doc.path}));
                    }
                    Err(error) => results.push(json!({"provider":provider,"machineId":machine,"status":"failed","configPath":doc.path,"message":error.to_string()})),
                }
            }
            // A native identity request has no model prompt and never copies a key.
            enrich_identities(&provider, &loaded.documents, &mut candidates);
            if provider == "copilot" && machine == "local" {
                let proxy = before.proxy.clone();
                copilot_identities(&self.store, &before, &mut candidates, |token| {
                    crate::copilot_auth::github_identity(&proxy, token)
                });
            }
            kimi_bases(&provider, &loaded.documents, &mut candidates);
            probe_identities(&self.store, &before, &mut candidates, true);
            if matches!(provider.as_str(), "cline" | "kilo") && machine == "local" {
                prepaid_identities(&self.store, &before, &mut candidates, |provider, token| {
                    crate::subscriptions::prepaid_identity(provider, &before, token)
                });
            }
            remote::probe_scan_identities(&self.store, &before, &mut candidates);
            if matches!(provider.as_str(), "qoder" | "antigravity") {
                let mut identity_roots: Vec<_> = loaded
                    .roots
                    .iter()
                    .filter(|r| machine != "local" || Path::new(r).is_dir())
                    .cloned()
                    .collect();
                if provider == "antigravity" && identity_roots.is_empty() {
                    let source = Source {
                        provider: provider.clone(),
                        host_id: (machine != "local").then(|| machine.clone()),
                        ..Default::default()
                    };
                    if let Some(root) = crate::agy_identity::ide_root(&source, &before) {
                        identity_roots.push(root);
                    }
                }
                for root in &identity_roots {
                    let source = Source {
                        provider: provider.clone(),
                        path: root.clone(),
                        host_id: (machine != "local").then(|| machine.clone()),
                        ..Default::default()
                    };
                    let identity = if provider == "qoder" {
                        crate::subscriptions::qoder::inspect(&source, &before, "").ok()
                    } else {
                        crate::agy_identity::read(&source, &before).identity
                    };
                    if let Some(identity) = identity {
                        let doc = Document {
                            root: root.clone(),
                            path: format!("native-cli:{provider}:{root}"),
                            bytes: vec![],
                        };
                        let mut row =
                            candidate(&provider, &machine, &doc, "", "login", &identity.key);
                        if provider == "antigravity" {
                            row.connection.upstream_identity = Some(identity.key.clone());
                        }
                        row.connection.identity_key = Some(identity.key);
                        row.connection.email = public_text(&identity.email);
                        row.connection.status = "observed".into();
                        row.connection.note =
                            "原生客户端已确认账户身份；授权有效性以查询结果为准".into();
                        candidates.push(row);
                    }
                }
            }
            // Preserve disconnected/unreadable references, but don't keep claiming
            // the last successful scan describes the current file.
            let found: BTreeSet<_> = candidates.iter().map(|c| c.connection.id.clone()).collect();
            let _critical = crate::operations::critical()?;
            let tx = self.store.write_transaction()?;
            let mut latest = self.store.settings()?;
            if !agent_enabled(&latest, &provider)
                || !machine_enabled(&latest, &machine)
                || source_signature(&before, &provider, &machine)
                    != source_signature(&latest, &provider, &machine)
                || host_signature(&before, &machine) != host_signature(&latest, &machine)
            {
                results.push(json!({"provider":provider,"machineId":machine,"status":"skipped","message":"识别期间机器或 Agent 配置已变化，稍后会自动重试"}));
                continue;
            }
            let mut kept = vec![];
            for old in latest.native_connections.iter_mut().filter(|r| {
                r.provider == provider && r.machine_id == machine && !found.contains(&r.id)
            }) {
                if retained.contains(&old.config_path) {
                    kept.push(old.id.clone());
                    let since = self.failed_since(&old.id)?;
                    if now() - since >= STALE_AFTER && old.status != "remembered" {
                        old.status = "stale".into();
                        old.note = stale_note(&provider);
                    }
                    continue;
                }
                if old.status != "remembered" {
                    old.status = "remembered".into();
                    old.note = "历史账号；原生配置已移除或已切换账户".into();
                    old.checked_at = now();
                }
            }
            for id in latest
                .native_connections
                .iter()
                .filter(|r| r.provider == provider && r.machine_id == machine)
                .map(|r| r.id.clone())
                .filter(|id| !kept.contains(id))
            {
                self.store.db.execute(
                    "DELETE FROM kv WHERE key=?1",
                    [format!("native-failed-since:{id}")],
                )?;
            }
            for candidate in &candidates {
                migrate_github_identity(&self.store, &mut latest, candidate);
            }
            let mut seen = BTreeSet::new();
            for candidate in candidates {
                let mut row = candidate.connection;
                row.machine_fingerprint = hash(&host_signature(&latest, &machine).to_string());
                if !seen.insert(row.id.clone()) {
                    continue;
                }
                let old = latest
                    .native_connections
                    .iter()
                    .find(|r| r.id == row.id)
                    .cloned();
                let existing = old.is_some();
                // A different login in the same file: remember the previous account.
                if let Some(old) = old.filter(|o| {
                    o.identity_key.is_some()
                        && o.identity_key != row.identity_key
                        && o.account_key.is_some()
                }) {
                    let id = hash(&json!([old.id, old.identity_key]).to_string());
                    if !latest.native_connections.iter().any(|r| r.id == id) {
                        latest.native_connections.push(NativeConnection {
                            id,
                            status: "remembered".into(),
                            note: "历史账号；原生客户端已切换账户".into(),
                            checked_at: now(),
                            ..old
                        });
                    }
                }
                // Switching back: the live connection replaces its remembered copy.
                if row.identity_key.is_some() {
                    let back = hash(&json!([row.id, row.identity_key]).to_string());
                    latest.native_connections.retain(|r| r.id != back);
                }
                // Unverified credentials remain visible even when they cannot yet
                // be safely assigned to an account.
                if let Some(identity) = &row.identity_key {
                    let reference = saved_profile_reference(&row, &loaded.documents);
                    let account = latest
                        .accounts
                        .iter()
                        .position(|a| {
                            a.provider == provider && a.identity_key.as_ref() == Some(identity)
                        })
                        .or_else(|| {
                            latest.accounts.iter().position(|a| {
                                a.provider == provider
                                    && a.identity_key.is_none()
                                    && reference.as_ref().is_some_and(|r| a.has_profile(r))
                            })
                        });
                    let index = if let Some(i) = account {
                        i
                    } else {
                        let id = format!("scan-{}", crate::codex_auth::files::new_id());
                        latest.accounts.push(Account {
                            id,
                            name: if row.email.is_empty() {
                                accounts::default_name(&provider)
                            } else {
                                row.email.clone()
                            },
                            provider: provider.clone(),
                            identity_key: Some(identity.clone()),
                            pending_name: row.email.is_empty(),
                            ..Default::default()
                        });
                        added_accounts += 1;
                        latest.accounts.len() - 1
                    };
                    let account = &mut latest.accounts[index];
                    account.identity_key = Some(identity.clone());
                    if row.upstream_identity.is_some() {
                        account.upstream_identity = row.upstream_identity.clone();
                    }
                    if account.pending_name && !row.email.is_empty() {
                        account.name = row.email.clone();
                        account.pending_name = false;
                    }
                    row.account_key = Some(accounts::key(account));
                }
                if !existing {
                    added_connections += 1;
                }
                results.push(json!({"provider":provider,"machineId":machine,"connectionId":row.id,"kind":row.kind,"status":if existing{"existing"}else{"added"},"credentialStatus":row.status,"accountKey":row.account_key,"configPath":row.config_path,"message":row.note}));
                self.store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![format!("native-fingerprint:{}",row.id),candidate.fingerprint])?;
                if let Some(i) = latest
                    .native_connections
                    .iter()
                    .position(|r| r.id == row.id)
                {
                    latest.native_connections[i] = row;
                } else {
                    latest.native_connections.push(row);
                }
            }
            if !found.is_empty() {
                if let Some(a) = latest.agents.iter_mut().find(|a| a.provider == provider)
                    && !a.machine_ids.contains(&machine)
                {
                    a.machine_ids.push(machine.clone());
                }
                // Add only a real discovered history root. Never assign its past
                // events to the account that happened to be logged in today.
                for root in &loaded.roots {
                    let exists = if machine == "local" {
                        Path::new(root).is_dir()
                    } else {
                        latest.native_connections.iter().any(|r| {
                            r.provider == provider && r.machine_id == machine && r.root == *root
                        })
                    };
                    if exists
                        && !latest.sources.iter().any(|s| {
                            s.provider == provider
                                && s.host_id.as_deref().unwrap_or("local") == machine
                                && source_root_matches(
                                    s,
                                    root,
                                    machine == "local",
                                    &loaded.aliases,
                                    true,
                                )
                                && s.subscription.is_none()
                        })
                    {
                        latest.sources.push(Source {
                            id: format!(
                                "scan-{}",
                                &hash(&format!("{provider}:{machine}:{root}"))[..24]
                            ),
                            name: format!(
                                "{} · {}",
                                if provider == "kimi" && crate::kimi_work::is_home(Path::new(root))
                                {
                                    "Kimi Work"
                                } else if provider == "codebuddy"
                                    && crate::codebuddy_ide::is_path(Path::new(root))
                                {
                                    "CodeBuddy IDE"
                                } else {
                                    contracts::provider(&provider).unwrap().name
                                },
                                if machine == "local" {
                                    "本机"
                                } else {
                                    &machine
                                }
                            ),
                            provider: provider.clone(),
                            path: root.clone(),
                            host_id: (machine != "local").then(|| machine.clone()),
                            ..Default::default()
                        });
                    }
                }
            } else {
                results.push(json!({"provider":provider,"machineId":machine,"status":"empty","message":if provider=="cursor"{"未找到可识别的 CLI 登录；GUI 私有凭据需在客户端管理"}else if provider=="claude"{"未登录或使用 API key"}else{"未找到可识别的原生账户或 API 配置"}}));
            }
            let source_links: Vec<_> = latest
                .native_connections
                .iter()
                .filter(|r| r.provider == provider && r.machine_id == machine)
                .map(|r| {
                    (
                        r.id.clone(),
                        latest
                            .sources
                            .iter()
                            .find(|s| {
                                s.provider == provider
                                    && s.host_id.as_deref().unwrap_or("local") == machine
                                    && s.machine_binding == MachineBinding::Bound
                                    && source_root_matches(
                                        s,
                                        &r.root,
                                        machine == "local",
                                        &loaded.aliases,
                                        true,
                                    )
                            })
                            .map(|s| s.id.clone()),
                    )
                })
                .collect();
            for (id, source_id) in source_links {
                if let Some(r) = latest.native_connections.iter_mut().find(|r| r.id == id) {
                    r.source_path = source_id
                        .as_ref()
                        .and_then(|id| latest.sources.iter().find(|s| s.id == *id))
                        .map(|s| s.path.clone())
                        .unwrap_or_default();
                    r.source_id = source_id;
                }
            }
            if provider == "codex" {
                bind_saved_codex(&mut latest, &machine, &loaded);
            }
            if provider == "codebuddy" {
                // This history directory itself names its owning uid. Unlike a
                // shared CLI log directory, its past usage has proven ownership.
                for source in latest.sources.iter_mut().filter(|s| {
                    s.provider == "codebuddy"
                        && s.account_id.is_empty()
                        && s.host_id.as_deref().unwrap_or("local") == machine
                }) {
                    if let Some(user) = crate::codebuddy_ide::user_id(Path::new(&source.path)) {
                        let identity = format!("codebuddy:{}", hash(&user));
                        if let Some(account) = latest.accounts.iter().find(|a| {
                            a.provider == "codebuddy"
                                && a.identity_key.as_deref() == Some(&identity)
                        }) {
                            source.account_id = account.id.clone();
                        }
                    }
                }
            }
            self.store.save_auth_settings(&latest)?;
            tx.commit()?;
        }
        let settings = self.store.settings()?;
        Ok(
            json!({"settings":settings,"addedAccounts":added_accounts,"addedConnections":added_connections,"results":results}),
        )
    }
    /// First time this connection's file was seen unreadable, recorded once.
    fn failed_since(&self, id: &str) -> Result<i64> {
        let key = format!("native-failed-since:{id}");
        self.store.db.execute(
            "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
            rusqlite::params![key, now().to_string()],
        )?;
        let value: String =
            self.store
                .db
                .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))?;
        Ok(value.parse().unwrap_or_else(|_| now()))
    }
    pub(crate) fn remove_native_connection(&mut self, params: Value) -> Result<Value> {
        let ids: std::collections::BTreeSet<String> = if let Some(ids) = params.get("connectionIds")
        {
            serde_json::from_value(ids.clone()).context("请选择原生连接")?
        } else {
            [params["connectionId"]
                .as_str()
                .context("请选择原生连接")?
                .to_owned()]
            .into()
        };
        ensure!(!ids.is_empty(), "请选择原生连接");
        let tx = self.store.write_transaction()?;
        let mut settings = self.store.settings()?;
        let before = settings.native_connections.len();
        settings.native_connections.retain(|r| !ids.contains(&r.id));
        for id in &ids {
            for prefix in ["native-fingerprint", "native-failed-since", "native-mtime"] {
                self.store
                    .db
                    .execute("DELETE FROM kv WHERE key=?1", [format!("{prefix}:{id}")])?;
            }
        }
        let removed = before - settings.native_connections.len();
        self.store.save_auth_settings(&settings)?;
        tx.commit()?;
        Ok(json!({"settings":self.store.settings()?,"removed":removed}))
    }
}

pub(crate) fn available(settings: &Settings, row: &NativeConnection) -> bool {
    row.machine_fingerprint == hash(&host_signature(settings, &row.machine_id).to_string())
        && machine_enabled(settings, &row.machine_id)
        && agent_enabled(settings, &row.provider)
        && selected(settings, &row.provider, &row.machine_id)
        && row.status != "remembered"
        && row.source_id.as_ref().is_none_or(|id| {
            settings.sources.iter().any(|s| {
                s.id == *id
                    && s.path == row.source_path
                    && s.provider == row.provider
                    && s.host_id.as_deref().unwrap_or("local") == row.machine_id
                    && settings.source_enabled(s)
            })
        })
}
pub(crate) fn status_rows(settings: &Settings) -> Vec<Value> {
    settings.native_connections.iter().filter_map(|r|{
        let key=r.account_key.as_ref()?;
        if !settings.accounts.iter().any(|a|accounts::key(a)==*key){return None;}
        Some(json!({"accountKey":key,"sourceId":format!("native-{}",r.id),"nativeConnectionId":r.id,"machineId":r.machine_id,"current":if r.status=="observed"&&available(settings,r){Some(true)}else{None},"credential":if r.machine_fingerprint==hash(&host_signature(settings,&r.machine_id).to_string()){Some(r.has_credential&&r.status!="remembered")}else{None},"checkedAt":r.checked_at,"note":r.note,"status":r.status,"error":if r.status=="stale"{Some(&r.note)}else{None}}))
    }).collect()
}
pub(crate) fn quota_sources(settings: &Settings, account: &Account) -> Vec<Source> {
    let mut sources = settings
        .native_connections
        .iter()
        .filter(|r| {
            r.account_key.as_deref() == Some(&accounts::key(account))
                && available(settings, r)
                && r.status == "observed"
                && (r.kind == "login"
                    && matches!(
                        r.provider.as_str(),
                        "codex" | "claude" | "antigravity" | "qoder"
                    )
                    || native_subscription(r).is_some()
                    || upstream_kind(r).is_some())
                && !shadowed(settings, r)
                && !(r.provider == "codex"
                    && r.config_path
                        .replace('\\', "/")
                        .contains("/.aieyes/accounts/"))
        })
        .map(|r| {
            let mut source = r
                .source_id
                .as_ref()
                .and_then(|id| settings.sources.iter().find(|s| s.id == *id))
                .cloned()
                .unwrap_or_default();
            source.id = format!("native-{}", r.id);
            source.name = account.name.clone();
            source.provider = r.provider.clone();
            source.account_id = account.id.clone();
            source.path = r.root.clone();
            source.host_id = (r.machine_id != "local").then(|| r.machine_id.clone());
            source.codex_home_id = None;
            source.quota_command.clear();
            source.subscription = if r.provider == "qoder" {
                Some(crate::subscriptions::SubscriptionConfig {
                    mode: "cli".into(),
                    native_config_dir: r.root.clone(),
                    ..Default::default()
                })
            } else {
                native_subscription(r)
            };
            let priority = native_priority(r);
            (
                r.upstream_identity.clone(),
                r.token_expired_at.is_some(),
                priority,
                source,
            )
        })
        .collect::<Vec<_>>();
    sources.sort_by_key(|(_, expired, priority, s)| {
        let preferred = account.quota_source_id.as_ref().is_some_and(|id| {
            *id == s.id
                || settings
                    .native_connections
                    .iter()
                    .any(|r| s.id == format!("native-{}", r.id) && r.source_id.as_ref() == Some(id))
        });
        (*expired, !preferred, *priority, s.host_id.is_some())
    });
    // The same server login seen through several native files or machines is one target.
    let mut seen = BTreeSet::new();
    sources
        .into_iter()
        .filter(|(upstream, _, _, _)| upstream.as_ref().is_none_or(|u| seen.insert(u.clone())))
        .map(|(_, _, _, source)| source)
        .collect()
}

/// Native credentials that query a subscription endpoint directly with the key
/// left in the client's own file (read at query time, never copied).
/// SSH references use the same adapter through the owning host transport.
fn native_subscription(r: &NativeConnection) -> Option<crate::subscriptions::SubscriptionConfig> {
    if !r.has_credential
        || r.machine_id != "local"
            && virtual_path(&r.config_path)
            && !(r.provider == "copilot" && copilot_env(&r.config_path))
    {
        return None;
    }
    let mode = match (r.provider.as_str(), r.kind.as_str()) {
        ("zcode", _) => zcode_region(&r.base_url)?,
        ("trae" | "trae-cn", "login") => "usage",
        ("grok", "login") if r.base_url == "https://cli-chat-proxy.grok.com/v1" => "subscription",
        ("deepseek-harness", "login")
            if r.base_url.trim_end_matches('/') == "https://platform.deepseek.com" =>
        {
            "account"
        }
        ("cursor", "login") => "api2",
        ("copilot", "login") => "github",
        ("cline", "login") => "account",
        ("kiro", "login") => "usage",
        // `kilo` in auth.json: device login (oauth) or a pasted Kilo token (api).
        ("kilo", "login" | "api") if r.selector == "/kilo" => "profile",
        ("kilo", "api") if r.selector == "/openrouter" || r.base_url.starts_with(OPENROUTER) => {
            "openrouter"
        }
        ("cline", "api") if r.base_url.starts_with(OPENROUTER) => "openrouter",
        ("opencode", "api") if r.selector == "/opencode-go" => "go",
        ("kimi", "api") => moonshot_host(&r.base_url)?,
        // Kimi Code's OAuth login reads its own region's usage; never refreshed here.
        ("kimi", "login") => {
            if r.base_url == KIMI_GLOBAL {
                "global"
            } else {
                "cn"
            }
        }
        ("deepseek-harness", "api")
            if r.base_url.trim_end_matches('/').trim_end_matches("/v1") == DEEPSEEK_API =>
        {
            "api"
        }
        _ => return None,
    };
    Some(crate::subscriptions::SubscriptionConfig {
        mode: mode.into(),
        native_config_dir: r.root.clone(),
        ..Default::default()
    })
}
/// Another client's login for Anthropic, ChatGPT or Copilot (see `upstream`).
/// Read with the token from the client's file on its owning device.
fn upstream_kind(r: &NativeConnection) -> Option<&'static str> {
    if r.kind != "login" || !r.has_credential {
        return None;
    }
    crate::upstream::kind(&r.provider, &r.selector)
}
/// A server account is read once: where it is signed in natively (Claude Code,
/// Codex, Copilot), otherwise through the first client that holds it.
fn shadowed(settings: &Settings, r: &NativeConnection) -> bool {
    shadowing(settings, r).is_some()
}
fn native_priority(r: &NativeConnection) -> u8 {
    if r.provider == "copilot" {
        copilot_priority(&r.config_path)
    } else if r.provider == "omp"
        && Path::new(&r.config_path)
            .file_name()
            .is_some_and(|name| name == "agent.db")
    {
        0
    } else {
        1
    }
}
fn shadowing<'a>(settings: &'a Settings, r: &NativeConnection) -> Option<&'a NativeConnection> {
    let (Some(upstream), Some(_)) = (&r.upstream_identity, upstream_kind(r)) else {
        return None;
    };
    settings.native_connections.iter().find(|o| {
        o.id != r.id
            && o.upstream_identity.as_ref() == Some(upstream)
            && o.has_credential
            && o.token_expired_at.is_none()
            && settings.accounts.iter().any(|a| {
                a.quota_enabled
                    && !a.archived
                    && o.account_key.as_deref() == Some(&accounts::key(a))
            })
            && o.status == "observed"
            && available(settings, o)
            && (upstream_kind(o).is_none()
                || (
                    o.token_expired_at.is_some(),
                    o.provider.as_str(),
                    native_priority(o),
                    o.id.as_str(),
                ) < (
                    r.token_expired_at.is_some(),
                    r.provider.as_str(),
                    native_priority(r),
                    r.id.as_str(),
                ))
    })
}
/// Why an account has no quota target of its own: its only login is another
/// client's copy of an account that is already read elsewhere.
pub(crate) fn shadow_notice(settings: &Settings, account: &Account) -> Option<String> {
    let key = accounts::key(account);
    let owner = settings
        .native_connections
        .iter()
        .filter(|r| r.account_key.as_deref() == Some(&key) && r.status == "observed")
        .find_map(|r| shadowing(settings, r))?;
    let name = contracts::provider(&owner.provider).map_or(owner.provider.as_str(), |p| p.name);
    Some(format!("与 {name} 中的登录为同一账户，额度只在那里查询"))
}
/// A fallback key must belong to this login, not merely to the same region.
fn read_native_subscription(
    settings: &Settings,
    row: &NativeConnection,
    source: &Source,
    secret: &str,
) -> Result<QuotaSnapshot> {
    let first = crate::subscriptions::read_native(source, settings, secret);
    if row.provider != "zcode" || row.kind != "login" || first.is_ok() {
        return first;
    }
    let region = source.subscription.as_ref().map(|c| c.mode.clone());
    let loaded = load(&row.provider, &row.machine_id, settings)?;
    let key = loaded
        .documents
        .iter()
        .filter_map(|d| extract(&row.provider, &row.machine_id, d).ok())
        .flatten()
        .find(|c| {
            c.connection.kind == "api"
                && !c.secret.is_empty()
                && c.connection.root == row.root
                && row.identity_key.is_some()
                && c.connection.identity_key == row.identity_key
                && zcode_region(&c.connection.base_url).map(str::to_owned) == region
        });
    match key {
        Some(key) => crate::subscriptions::read_native(source, settings, &key.secret),
        None => first,
    }
}

/// Kiro: the newest unexpired token of the same login (kiro-cli or IDE). When
/// every token has expired and kiro-cli holds this login, `kiro-cli /usage` runs
/// at most every 5 min (it refreshes kiro-cli's own token); the session it
/// leaves behind is recorded as Aieyes' own. An IDE-only login waits for the IDE.
fn kiro_snapshot(
    store: &crate::store::Store,
    settings: &Settings,
    row: &NativeConnection,
    source: &Source,
    secret: &str,
    expired: bool,
) -> Result<QuotaSnapshot> {
    let logins: Vec<_> = if row.identity_key.is_some() {
        load(&row.provider, &row.machine_id, settings)
            .map(|loaded| {
                loaded
                    .documents
                    .iter()
                    .filter_map(|d| extract(&row.provider, &row.machine_id, d).ok())
                    .flatten()
                    .filter(|c| c.connection.identity_key == row.identity_key)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };
    if let Some(fresh) = logins
        .iter()
        .filter(|c| c.connection.token_expired_at.is_none())
        .max_by_key(|c| {
            serde_json::from_str::<Value>(&c.secret)
                .ok()
                .and_then(|v| kiro_expiry(&v))
                .unwrap_or(0)
        })
    {
        return crate::subscriptions::read_native(source, settings, &fresh.secret);
    }
    if !expired {
        return crate::subscriptions::read_native(source, settings, secret);
    }
    let cli = |selector: &str| selector.starts_with("/auth_kv/");
    ensure!(
        row.machine_id == "local"
            && (cli(&row.selector) || logins.iter().any(|c| cli(&c.connection.selector))),
        "{}",
        crate::subscriptions::kiro::RELOGIN
    );
    use crate::native_sessions::kiro;
    kiro_cli_usage(
        store,
        settings,
        source,
        "kiro-cli",
        &kiro::home(),
        &kiro::probe_dir(),
    )
}
fn kiro_cli_usage(
    store: &crate::store::Store,
    settings: &Settings,
    source: &Source,
    program: &str,
    kiro_home: &Path,
    probe: &Path,
) -> Result<QuotaSnapshot> {
    use crate::{native_sessions::kiro as sessions, subscriptions::kiro};
    const KEY: &str = "kiro-cli-usage-at";
    // Atomically claim the polling slot: manual and background refreshes can
    // arrive through different Engine connections at the same time.
    let claimed = store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE CAST(kv.value AS INTEGER)<=?3",
        rusqlite::params![KEY, now().to_string(), now() - kiro::CLI_FLOOR],
    )?;
    ensure!(claimed == 1, "Kiro 登录已过期，稍后由 kiro-cli 重新查询");
    // Marks older than two days name sessions no longer listed.
    store.db.execute(
        "DELETE FROM kv WHERE key LIKE 'internal-session:kiro:%' AND CAST(value AS INTEGER)<?1",
        [now() - 2 * 86400],
    )?;
    std::fs::create_dir_all(probe).context("无法准备 Kiro 查询目录")?;
    let files = || {
        sessions::directory(&kiro_home.to_string_lossy())
            .map(|dir| sessions::files(&dir))
            .unwrap_or_default()
            .into_iter()
            .map(|(path, _)| path)
            .collect::<BTreeSet<_>>()
    };
    let before = files();
    store.mark_internal_session("kiro", &probe.to_string_lossy())?;
    let output = kiro::run_cli(program, probe, settings);
    // Record only sessions created in Aieyes' own working directory.
    for path in files().difference(&before) {
        // A user can start their own session while the probe is running.
        // A new file alone does not mean it belongs to Aieyes.
        if let Some(id) = sessions::session_id_in_directory(path, probe) {
            store.mark_internal_session("kiro", &id)?;
        }
    }
    kiro::parse_cli(source, &output?)
}
/// The last stored snapshot of `account` that has content and no error.
pub(crate) fn last_snapshot(
    store: &crate::store::Store,
    account: &Account,
) -> Option<QuotaSnapshot> {
    use rusqlite::OptionalExtension;
    let raw: String = store
        .db
        .query_row(
            "SELECT payload FROM quotas WHERE account_key=?1",
            [accounts::key(account)],
            |r| r.get(0),
        )
        .optional()
        .ok()??;
    let mut q: QuotaSnapshot = serde_json::from_str(&raw).ok()?;
    if !q.has_values() {
        return None;
    }
    // Store preserves the last measured values when a later read fails. That
    // failure does not invalidate the cached wallet/window itself.
    q.error = None;
    Some(q)
}
pub(crate) fn has_quota_target(settings: &Settings, account: &Account) -> bool {
    crate::quota::local_credits(&account.provider) || !quota_sources(settings, account).is_empty()
}
pub(crate) fn read_native_quota(
    store: &crate::store::Store,
    settings: &Settings,
    account: &Account,
    source: &Source,
) -> Result<QuotaSnapshot> {
    let result = read_native_quota_inner(store, settings, account, source);
    match result {
        // An unreadable state.vscdb or a rejected token: the IDE refreshes it on launch.
        Err(e)
            if source.provider == "cursor"
                && !matches!(
                    e.to_string().as_str(),
                    "原生登录账户已变化，正在重新识别" | "查询期间原生账户身份已变化"
                ) =>
        {
            Err(anyhow::anyhow!("{}", stale_note("cursor")))
        }
        r => r,
    }
}
fn read_native_quota_inner(
    store: &crate::store::Store,
    settings: &Settings,
    account: &Account,
    source: &Source,
) -> Result<QuotaSnapshot> {
    let row = settings
        .native_connections
        .iter()
        .find(|r| {
            source.id == format!("native-{}", r.id)
                && r.account_key.as_deref() == Some(&accounts::key(account))
        })
        .context("原生连接已移除")?;
    if row.machine_id != "local" && !matches!(row.provider.as_str(), "qoder" | "antigravity") {
        return remote::read(store, settings, account, source, row);
    }
    let mut secret = String::new();
    let mut expired = false;
    let mut plan = String::new();
    let mut upstream_auth = None;
    if row.provider == "qoder" || row.provider == "antigravity" {
        let identity = if row.provider == "qoder" {
            crate::subscriptions::qoder::inspect(
                source,
                settings,
                &accounts::pre_command(settings, Some(account), source),
            )?
        } else {
            crate::agy_identity::read(source, settings)
                .identity
                .context("原生账户身份待确认")?
        };
        ensure!(
            row.identity_key.as_deref() == Some(&identity.key),
            "原生登录账户已变化，正在重新识别"
        );
    } else {
        let candidate = reload_verified(store, settings, row)?;
        ensure!(
            candidate.connection.identity_key == row.identity_key,
            "原生登录账户已变化，正在重新识别"
        );
        secret = candidate.secret.clone();
        expired = candidate.connection.token_expired_at.is_some();
        plan = candidate.plan.clone();
        upstream_auth = candidate.upstream_auth.clone();
        let expected: String = store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key=?1",
                [format!("native-fingerprint:{}", row.id)],
                |r| r.get(0),
            )
            .context("原生凭据尚未识别")?;
        ensure!(
            candidate.connection.identity_key.is_some() || candidate.fingerprint == expected,
            "凭据已变化，正在重新识别"
        );
        ensure!(
            candidate.connection.status != "stale",
            "{}",
            stale_note(&row.provider)
        );
    }
    if crate::subscriptions::trae::family(&row.provider)
        && let Some(recent) = trae_recent(store, account, row, expired)?
    {
        return Ok(recent);
    }
    let mut snapshot = if row.provider == "kiro" {
        kiro_snapshot(store, settings, row, source, &secret, expired)?
    } else if let Some(kind) = upstream_kind(row) {
        crate::upstream::read(
            &store.root.join("upstream"),
            settings,
            source,
            kind,
            upstream_auth.as_ref().context("上游登录凭据不可用")?,
            row.upstream_identity.as_deref(),
        )?
    } else if native_subscription(row).is_some() {
        read_native_subscription(settings, row, source, &secret)?
    } else {
        crate::quota::read(source, settings)?
    };
    if snapshot.plan.is_none() && !plan.is_empty() {
        snapshot.plan = Some(plan);
    }
    if prepaid_account(row) || row.provider == "copilot" {
        ensure!(
            snapshot.identity.as_ref().map(|i| &i.key) == row.identity_key.as_ref(),
            "查询期间原生账户身份已变化"
        );
    }
    // The public identity read guards attribution without storing or switching auth.
    if row.provider == "qoder" || row.provider == "antigravity" {
        let identity = snapshot
            .identity
            .as_ref()
            .context("额度响应未确认账户身份")?;
        ensure!(
            row.identity_key.as_deref() == Some(&identity.key),
            "查询期间原生账户身份已变化"
        );
    } else {
        let current = reload_verified(store, settings, row)?;
        ensure!(
            current.connection.identity_key == row.identity_key,
            "查询期间原生账户身份已变化"
        );
    }
    Ok(snapshot)
}

/// Trae: one `ide_user_ent_usage` call per login every ten minutes, whatever
/// asks for it; a call in between returns the snapshot it produced. An expired
/// token is left for the IDE to renew.
fn trae_recent(
    store: &crate::store::Store,
    account: &Account,
    row: &NativeConnection,
    expired: bool,
) -> Result<Option<QuotaSnapshot>> {
    ensure!(!expired, "{}", stale_note(&row.provider));
    let key = format!("trae-usage-at:{}", row.id);
    let last: i64 = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let floor = crate::quota_schedule::floor(&row.provider) as i64;
    if (0..floor).contains(&(now() - last)) {
        return last_snapshot(store, account)
            .map(Some)
            .context("Trae 额度每 10 分钟最多查询一次，稍后自动刷新");
    }
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, now().to_string()],
    )?;
    Ok(None)
}
fn same_root(a: &str, b: &str, local: bool) -> bool {
    if local {
        std::fs::canonicalize(expand(a)).unwrap_or_else(|_| expand(a))
            == std::fs::canonicalize(expand(b)).unwrap_or_else(|_| expand(b))
    } else {
        a.trim_end_matches('/') == b.trim_end_matches('/')
    }
}
fn source_signature(settings: &Settings, provider: &str, machine: &str) -> Value {
    json!(settings.sources.iter().filter(|s|s.provider==provider&&s.host_id.as_deref().unwrap_or("local")==machine).map(|s|json!([s.id,s.path,s.enabled,s.machine_binding,s.codex_binary,s.agy_binary,s.subscription])).collect::<Vec<_>>())
}
fn enrich_identities(provider: &str, documents: &[Document], candidates: &mut [Candidate]) {
    if provider != "claude" {
        return;
    }
    let identities: Vec<_> = documents
        .iter()
        .filter_map(|d| {
            let v = parse(d).ok()?;
            let a = &v["oauthAccount"];
            let uuid = a["accountUuid"].as_str()?;
            Some((
                d.root.clone(),
                format!(
                    "claude:{}",
                    hash(&json!([uuid, a["organizationUuid"]]).to_string())
                ),
                format!("anthropic:{}", hash(uuid)),
                public_text(text(&a["emailAddress"])),
            ))
        })
        .collect();
    for candidate in candidates
        .iter_mut()
        .filter(|c| c.connection.kind == "login")
    {
        if let Some((_, identity, upstream, email)) = identities
            .iter()
            .find(|(root, ..)| *root == candidate.connection.root)
        {
            candidate.connection.identity_key = Some(identity.clone());
            candidate.connection.upstream_identity = Some(upstream.clone());
            candidate.connection.email = email.clone();
        } else if candidate.connection.status == "observed" {
            candidate.connection.note = "未登录或使用 API key".into();
        }
    }
}
fn reload_candidate(
    store: &crate::store::Store,
    settings: &Settings,
    row: &NativeConnection,
) -> Result<Candidate> {
    let loaded = load_inner(&row.provider, &row.machine_id, settings, Some(row))?;
    let mut candidates = loaded
        .documents
        .iter()
        .find(|d| d.path == row.config_path)
        .map(|d| extract(&row.provider, &row.machine_id, d))
        .transpose()?
        .unwrap_or_default();
    enrich_identities(&row.provider, &loaded.documents, &mut candidates);
    let mut candidate = candidates
        .into_iter()
        .find(|c| c.connection.id == row.id)
        .context("原生凭据已移除")?;
    // Same token, same numeric GitHub user: no lookup while checking the file.
    if needs_github_identity(&candidate) || needs_prepaid_identity(&candidate) {
        reuse_verified_identity(store, settings, &mut candidate);
    }
    Ok(candidate)
}
/// `reload_candidate` plus the server identity a scan asked the service for
/// (see `probe_identities`). A changed token whose identity cannot be confirmed
/// now stays unassigned, so its quota is never attributed to the old account.
fn reload_verified(
    store: &crate::store::Store,
    settings: &Settings,
    row: &NativeConnection,
) -> Result<Candidate> {
    let mut c = reload_candidate(store, settings, row)?;
    if c.connection.base_url.is_empty() {
        c.connection.base_url = row.base_url.clone();
    }
    probe_identities(store, settings, std::slice::from_mut(&mut c), false);
    Ok(c)
}

const KIMI_CN: &str = "https://api.kimi.com/coding/v1";
const KIMI_GLOBAL: &str = "https://api.kimi.ai/coding/v1";
/// Kimi Code's region for its OAuth login, resolved like the CLI: the managed
/// provider's endpoints in config.toml, then the install-channel `region`
/// marker; mainland China otherwise. Kept as the login's API base.
fn kimi_bases(provider: &str, documents: &[Document], candidates: &mut [Candidate]) {
    if provider != "kimi" {
        return;
    }
    for c in candidates
        .iter_mut()
        .filter(|c| c.connection.kind == "login")
    {
        c.connection.base_url = kimi_base(documents, &c.connection).into();
    }
}
fn kimi_base(documents: &[Document], row: &NativeConnection) -> &'static str {
    let host = |s: &str| {
        reqwest::Url::parse(s)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default()
    };
    for d in documents
        .iter()
        .filter(|d| d.root == row.root && d.path.ends_with("config.toml"))
    {
        let Ok(v) = parse(d) else {
            continue;
        };
        let managed = &v["providers"]["managed:kimi-code"];
        if !managed.is_object() {
            continue;
        }
        let oauth = &managed["oauth"];
        let global = [
            &managed["base_url"],
            &managed["baseUrl"],
            &oauth["oauth_host"],
            &oauth["oauthHost"],
        ]
        .into_iter()
        .any(|v| matches!(host(text(v)).as_str(), "api.kimi.ai" | "auth.kimi.ai"));
        return if global { KIMI_GLOBAL } else { KIMI_CN };
    }
    // The marker is read only on this device; remote logins are not queried here.
    if row.machine_id == "local"
        && std::fs::read_to_string(Path::new(&row.root).join("region"))
            .is_ok_and(|s| s.trim() == "global")
    {
        return KIMI_GLOBAL;
    }
    KIMI_CN
}

/// Which service to ask for the server identity of an opaque login token.
fn probe_kind(c: &NativeConnection) -> Option<&'static str> {
    if c.kind != "login" {
        return None;
    }
    if c.provider == "kimi" {
        return Some("kimi");
    }
    crate::upstream::kind(&c.provider, &c.selector).filter(|k| *k != "codex")
}
fn stored_fingerprint(store: &crate::store::Store, id: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("native-fingerprint:{id}")],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
}
/// Answers by token fingerprint: the identity, or when a request last failed.
type Probed = std::collections::HashMap<String, Result<crate::upstream::Identity, i64>>;
static PROBED: std::sync::OnceLock<std::sync::Mutex<Probed>> = std::sync::OnceLock::new();
/// A token the service rejected (usually expired until its client runs) is
/// not asked about again for a while; a refreshed token is asked at once.
const PROBE_RETRY: i64 = 1800;
/// Logins whose server account is known only by asking the service: an
/// Anthropic or GitHub login kept by OpenCode / Pi, or a Kimi Code token.
/// An answer is reused while the token is unchanged (token fingerprints in
/// memory, then the last scan's reference), so rescans of a busy OpenCode
/// database do not repeat the request. During a scan a failed request keeps
/// the previous account and fingerprint, so a later read asks again.
fn probe_identities(
    store: &crate::store::Store,
    settings: &Settings,
    candidates: &mut [Candidate],
    keep_on_failure: bool,
) {
    for c in candidates.iter_mut() {
        let Some(kind) = probe_kind(&c.connection) else {
            continue;
        };
        if c.connection.identity_key.is_some()
            || c.secret.is_empty()
            || c.connection.machine_id != "local"
        {
            continue;
        }
        let key = hash(&format!("{}\0{}", c.connection.id, c.fingerprint));
        let cache = PROBED.get_or_init(Default::default);
        let cached = cache.lock().ok().and_then(|m| m.get(&key).cloned());
        let old = settings.native_connections.iter().find(|r| {
            r.id == c.connection.id
                && r.identity_key.is_some()
                && (kind != "kimi" || r.upstream_identity == r.identity_key)
                && (kind != "gemini"
                    || r.upstream_identity
                        .as_deref()
                        .is_some_and(|id| id.starts_with("google-gemini-cli:")))
        });
        let stored = stored_fingerprint(store, &c.connection.id);
        let identity = match cached {
            Some(Ok(id)) => Some(id),
            None if old.is_some_and(|old| kind != "copilot" || numeric_github(old))
                && stored.as_deref() == Some(c.fingerprint.as_str()) =>
            {
                copy_identity(c, old.unwrap());
                continue;
            }
            Some(Err(at)) if now() - at < PROBE_RETRY => None,
            _ => {
                let answer = if kind == "gemini" {
                    c.upstream_auth
                        .as_ref()
                        .context("上游登录凭据不可用")
                        .and_then(|auth| {
                            crate::upstream::gemini_identity_using(auth, |request| {
                                request.send(&settings.proxy)
                            })
                        })
                } else {
                    crate::upstream::probe(
                        kind,
                        &c.secret,
                        &settings.proxy,
                        if c.connection.base_url.is_empty() {
                            KIMI_CN
                        } else {
                            &c.connection.base_url
                        },
                    )
                }
                .map_err(|_| now());
                if let Ok(mut m) = cache.lock() {
                    if m.len() >= 256 {
                        m.clear();
                    }
                    m.insert(key, answer.clone());
                }
                answer.ok()
            }
        };
        match identity {
            Some(id) => {
                c.connection.identity_key = Some(crate::upstream::client_identity(
                    &c.connection.provider,
                    &c.connection.selector,
                    kind,
                    &id.subject,
                ));
                if kind == "kimi" {
                    c.connection.identity_key = Some(id.upstream.clone());
                }
                c.connection.upstream_identity = Some(id.upstream);
                if !id.email.is_empty() {
                    c.connection.email = public_text(&id.email);
                }
            }
            None if keep_on_failure => {
                if let (Some(old), Some(stored)) = (old, stored) {
                    copy_identity(c, old);
                    c.fingerprint = stored;
                }
            }
            None => {}
        }
    }
}
fn copy_identity(c: &mut Candidate, old: &NativeConnection) {
    c.connection.identity_key = old.identity_key.clone();
    c.connection.upstream_identity = old.upstream_identity.clone();
    if !old.email.is_empty() {
        c.connection.email = old.email.clone();
    }
}

fn bind_saved_codex(settings: &mut Settings, machine: &str, loaded: &Loaded) {
    let documents = &loaded.documents;
    // Existing, self-consistent managed profiles can be linked without copying
    // auth, enabling storage, launching a CLI or changing the daily login.
    for marker in documents.iter().filter(|d| {
        d.path
            .replace('\\', "/")
            .ends_with("/.aieyes/state/home.json")
    }) {
        let Ok(value) = parse(marker) else {
            continue;
        };
        let home = text(&value["homeId"]);
        if crate::codex_auth::files::id(home).is_err() {
            continue;
        }
        let Some(source) = settings.sources.iter_mut().find(|s| {
            s.provider == "codex"
                && s.host_id.as_deref().unwrap_or("local") == machine
                && source_root_matches(s, &marker.root, machine == "local", &loaded.aliases, false)
        }) else {
            continue;
        };
        if !source.account_id.is_empty()
            || !source.quota_command.is_empty()
            || source.codex_home_id.as_ref().is_some_and(|id| id != home)
        {
            continue;
        }
        source.codex_home_id = Some(home.into());
        let source_id = source.id.clone();
        for metadata in documents.iter().filter(|d| {
            d.root == marker.root && d.path.replace('\\', "/").ends_with("/metadata.json")
        }) {
            let Ok(profile) = parse(metadata) else {
                continue;
            };
            let profile_id = text(&profile["id"]);
            if crate::codex_auth::files::id(profile_id).is_err() {
                continue;
            }
            let expected = format!(
                "{}/.aieyes/accounts/{profile_id}/metadata.json",
                marker.root.replace('\\', "/").trim_end_matches('/')
            );
            if metadata.path.replace('\\', "/") != expected {
                continue;
            }
            let Some(connection) = settings.native_connections.iter().find(|r| {
                r.provider == "codex"
                    && r.machine_id == machine
                    && r.config_path.replace('\\', "/")
                        == expected.replace("metadata.json", "auth.json")
                    && r.identity_key.as_deref() == profile["identity"]["key"].as_str()
            }) else {
                continue;
            };
            let reference = format!("{home}:{profile_id}");
            if settings.accounts.iter().any(|a| {
                a.has_profile(&reference)
                    && connection.account_key.as_deref() != Some(&accounts::key(a))
            }) {
                continue;
            }
            let Some(account) = settings
                .accounts
                .iter_mut()
                .find(|a| connection.account_key.as_deref() == Some(&accounts::key(a)))
            else {
                continue;
            };
            if !account.has_profile(&reference) {
                account.connections.push(AccountConnection {
                    source_id: source_id.clone(),
                    profile_id: Some(reference.clone()),
                });
            }
            if account.quota_profile_id.is_none() {
                account.quota_profile_id = Some(reference);
            }
        }
    }
}

/// File names and WAL changes matter too: a new login may never touch the main DB.
fn file_signature(paths: &[PathBuf]) -> String {
    let mut paths: BTreeSet<_> = paths.iter().cloned().collect();
    for path in paths.clone() {
        if sqlite_db(&path) {
            paths.insert(PathBuf::from(format!("{}-wal", path.to_string_lossy())));
        }
    }
    hash(
        &paths
            .iter()
            .map(|p| {
                std::fs::metadata(p).map_or(json!([p, null]), |m| {
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos() as u64);
                    json!([p, modified, m.len()])
                })
            })
            .collect::<Value>()
            .to_string(),
    )
}
/// Watch the same default, configured and discovered roots that discovery reads.
fn watched_files(provider: &str, settings: &Settings) -> Vec<PathBuf> {
    let Ok(roots) = scan_roots(provider, "local", settings, None) else {
        return vec![];
    };
    let mut paths: Vec<_> = extra_paths(provider)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    for root in roots {
        let root = Path::new(&root);
        for pattern in spec(provider).1 {
            paths.extend(patterns(root, pattern));
        }
        if provider == "opencode" {
            paths.push(root.join("opencode.db"));
        }
    }
    match provider {
        "kimi" => {
            paths.extend(
                crate::kimi_work::paths()
                    .into_iter()
                    .map(|(_, config)| config),
            );
        }
        "codebuddy" => paths.extend(crate::codebuddy_ide::paths()),
        _ => {}
    }
    if provider == "antigravity" {
        paths.extend(
            crate::agy_identity::ide_paths()
                .into_iter()
                .flat_map(|path| {
                    let wal = PathBuf::from(format!("{}-wal", path.display()));
                    [path, wal]
                }),
        );
    }
    paths
}

impl Engine {
    /// A never-checked target also needs discovery (for example a newly enabled agent).
    fn signature_changed(&self, key: &str, signature: &str) -> Result<bool> {
        use rusqlite::OptionalExtension;
        let key = format!("native-mtime:{key}");
        let old: Option<String> = self
            .store
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
            .optional()?;
        if old.as_deref() == Some(signature) {
            return Ok(false);
        }
        self.store.db.execute(
            "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, signature],
        )?;
        Ok(true)
    }
    /// Re-detect local agents whose credential files changed since the last check.
    fn changed_local_targets(&self, settings: &Settings) -> Result<BTreeSet<(String, String)>> {
        let mut changed = BTreeSet::new();
        if !machine_enabled(settings, "local") {
            return Ok(changed);
        }
        for p in contracts::PROVIDERS
            .iter()
            .filter(|p| p.machines && has_files(p.id) && agent_enabled(settings, p.id))
        {
            if self.signature_changed(
                &format!("{}:local", p.id),
                &hash(
                    &json!([
                        file_signature(&watched_files(p.id, settings)),
                        source_signature(settings, p.id, "local")
                    ])
                    .to_string(),
                ),
            )? {
                changed.insert((p.id.to_string(), "local".to_string()));
            }
        }
        for row in settings.native_connections.iter().filter(|r| {
            r.machine_id == "local"
                && r.status != "remembered"
                && !r.config_path.starts_with("native-cli:")
                && !r.config_path.starts_with("keychain:")
                && !virtual_path(&r.config_path)
        }) {
            let paths: Vec<_> = reference_paths(row)
                .into_iter()
                .map(PathBuf::from)
                .collect();
            if self.signature_changed(&row.id, &file_signature(&paths))? {
                changed.insert((row.provider.clone(), "local".to_string()));
            }
        }
        Ok(changed)
    }
    /// One bounded metadata request per remote machine, including agents with no
    /// known account yet. Only changed providers need a credential projection.
    fn changed_remote_targets(&self, settings: &Settings) -> Result<BTreeSet<(String, String)>> {
        use rusqlite::OptionalExtension;
        let mut changed = BTreeSet::new();
        let providers: Vec<_> = contracts::PROVIDERS
            .iter()
            .filter(|p| p.machines && has_files(p.id) && agent_enabled(settings, p.id))
            .collect();
        if providers.is_empty() {
            return Ok(changed);
        }
        for host in settings.hosts.iter().filter(|host| host.enabled) {
            crate::operations::check()?;
            let config = hash(
                &json!([
                    host_signature(settings, &host.id),
                    providers
                        .iter()
                        .map(|p| json!([p.id, source_signature(settings, p.id, &host.id)]))
                        .collect::<Vec<_>>()
                ])
                .to_string(),
            );
            let key = format!("native-remote-poll:{}", host.id);
            let previous: Option<String> = self
                .store
                .db
                .query_row("SELECT value FROM kv WHERE key=?1", [&key], |row| {
                    row.get(0)
                })
                .optional()?;
            let previous: Value = previous
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or_default();
            let at = now();
            if previous["config"].as_str() == Some(&config)
                && previous["checkedAt"]
                    .as_i64()
                    .is_some_and(|checked| (0..300).contains(&(at - checked)))
            {
                continue;
            }
            self.store.db.execute(
                "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![key, json!({"checkedAt":at,"config":config}).to_string()],
            )?;
            let mut targets = vec![];
            for provider in &providers {
                let (env, patterns) = spec(provider.id);
                let mut target = json!({
                    "provider":provider.id,
                    "roots":scan_roots(provider.id, &host.id, settings, None)?,
                    "env":env,"patterns":patterns,"primary":default_root(provider.id,true)?
                });
                if provider.id == "codebuddy" {
                    target["codebuddyIdeScript"] = json!(crate::ssh::CODEBUDDY_IDE_SCRIPT);
                }
                targets.push(target);
            }
            let encoded = STANDARD.encode(serde_json::to_vec(
                &json!({"action":"fingerprints","targets":targets}),
            )?);
            let command = crate::ssh::command(
                host,
                &format!("exec python3 - {}", crate::process::quote(&encoded)),
            )?;
            let Ok(output) = crate::process::run(
                command,
                include_str!("../../../scripts/remote_account_scan.py")
                    .as_bytes()
                    .to_vec(),
                Duration::from_secs(35),
            ) else {
                continue;
            };
            if output.len() > 128 * 1024 {
                continue;
            }
            let Ok(result) = serde_json::from_slice::<Value>(&output) else {
                continue;
            };
            for provider in &providers {
                if let Some(signature) = result["fingerprints"][provider.id]["fingerprint"].as_str()
                    && self.signature_changed(&format!("{}:{}", provider.id, host.id), signature)?
                {
                    changed.insert((provider.id.to_owned(), host.id.clone()));
                }
            }
        }
        Ok(changed)
    }
    fn auto_scan_targets(&mut self, targets: BTreeSet<(String, String)>) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        let targets: Vec<_> = targets.into_iter().map(|(p, m)| json!([p, m])).collect();
        self.scan_accounts(json!({"targets":targets}))?;
        Ok(())
    }
    pub(crate) fn refresh_native_connections(&mut self, params: &Value) -> Result<()> {
        if !std::env::var("AIEYES_AUTO_SCAN").is_ok_and(|value| value == "off") {
            let _lock = crate::file_lock::history(&self.store.root, "native-discovery")?;
            let settings = self.store.settings()?;
            let mut changed = self.changed_local_targets(&settings)?;
            changed.extend(self.changed_remote_targets(&settings)?);
            self.auto_scan_targets(changed)?;
            self.changed_local_targets(&self.store.settings()?)?;
        }
        let settings = self.store.settings()?;
        let mut switched = BTreeSet::new();
        for row in settings.native_connections.iter().filter(|r| {
            r.kind == "login"
                && available(&settings, r)
                && params["accountKeys"].as_array().is_none_or(|keys| {
                    r.account_key
                        .as_ref()
                        .is_some_and(|key| keys.contains(&json!(key)))
                })
                && params["sourceIds"]
                    .as_array()
                    .is_none_or(|ids| ids.contains(&json!(format!("native-{}", r.id))))
        }) {
            // Qoder re-checks start qodercli: keep its quota floor here too.
            let every = crate::quota_schedule::floor(&row.provider).max(300) as i64;
            if params["dueOnly"] == true && (0..every).contains(&(now() - row.checked_at)) {
                continue;
            }
            crate::operations::check()?;
            let mut next = row.clone();
            let mut identity_changed = false;
            let observation = if row.config_path.starts_with("native-cli:") {
                let source = Source {
                    provider: row.provider.clone(),
                    path: row.root.clone(),
                    host_id: (row.machine_id != "local").then(|| row.machine_id.clone()),
                    ..Default::default()
                };
                let identity = if row.provider == "qoder" {
                    crate::subscriptions::qoder::inspect(&source, &settings, "")
                } else {
                    crate::agy_identity::read(&source, &settings)
                        .identity
                        .context("原生登录身份暂不可读")
                };
                identity.map(|i| identity_changed = Some(i.key) != row.identity_key)
            } else {
                reload_verified(&self.store, &settings, row).and_then(|c| {
                    identity_changed = c.connection.identity_key != row.identity_key;
                    if row.identity_key.is_none() {
                        let expected: String = self.store.db.query_row(
                            "SELECT value FROM kv WHERE key=?1",
                            [format!("native-fingerprint:{}", row.id)],
                            |r| r.get(0),
                        )?;
                        identity_changed |= expected != c.fingerprint;
                    }
                    if !identity_changed {
                        next.status = c.connection.status;
                        next.note = c.connection.note;
                        next.token_expired_at = c.connection.token_expired_at;
                    }
                    Ok(())
                })
            };
            match observation {
                // A different login is not an error: re-detect and remember the old one.
                Ok(()) if identity_changed => {
                    switched.insert((row.provider.clone(), row.machine_id.clone()));
                    continue;
                }
                Ok(()) => {
                    self.store.db.execute(
                        "DELETE FROM kv WHERE key=?1",
                        [format!("native-failed-since:{}", row.id)],
                    )?;
                }
                // Unreadable or half written: keep the last state for a day.
                Err(_) => {
                    if now() - self.failed_since(&row.id)? >= STALE_AFTER {
                        next.status = "stale".into();
                        next.note = stale_note(&row.provider);
                    }
                }
            }
            next.checked_at = now();
            let tx = self.store.write_transaction()?;
            let mut latest = self.store.settings()?;
            if host_signature(&settings, &row.machine_id)
                != host_signature(&latest, &row.machine_id)
            {
                continue;
            }
            if let Some(current) = latest.native_connections.iter_mut().find(|r| **r == *row) {
                *current = next;
                self.store.save_auth_settings(&latest)?;
            }
            tx.commit()?;
        }
        self.auto_scan_targets(switched)
    }
    /// Detection at engine start and on demand (`accounts.autoScan`).
    pub(crate) fn auto_scan(&mut self, params: Value) -> Result<Value> {
        let _lock = crate::file_lock::history(&self.store.root, "native-discovery")?;
        let result = self.scan_accounts(params)?;
        // Baseline the file signatures so later edits trigger re-detection.
        let settings = self.store.settings()?;
        self.changed_local_targets(&settings)?;
        Ok(result)
    }
}

fn reference_paths(row: &NativeConnection) -> Vec<String> {
    let mut paths = vec![row.config_path.clone()];
    if row.provider == "claude" {
        paths.push(format!("{}/.claude.json", row.root.trim_end_matches('/')));
        if row.config_path.starts_with("keychain:") {
            paths.push(format!(
                "{}/.credentials.json",
                row.root.trim_end_matches('/')
            ));
        }
        let root = row.root.replace('\\', "/");
        if root.ends_with("/.claude") {
            paths.push(format!("{}.json", row.root));
        }
    }
    paths
}

fn saved_profile_reference(row: &NativeConnection, documents: &[Document]) -> Option<String> {
    if row.provider != "codex" {
        return None;
    }
    let root = row.root.replace('\\', "/");
    let path = row.config_path.replace('\\', "/");
    let profile = path
        .strip_prefix(&format!("{}/.aieyes/accounts/", root.trim_end_matches('/')))?
        .strip_suffix("/auth.json")?;
    crate::codex_auth::files::id(profile).ok()?;
    let marker = documents.iter().find(|d| {
        d.root == row.root
            && d.path
                .replace('\\', "/")
                .ends_with("/.aieyes/state/home.json")
    })?;
    let value = parse(marker).ok()?;
    let home = value["homeId"].as_str()?;
    crate::codex_auth::files::id(home).ok()?;
    Some(format!("{home}:{profile}"))
}

fn source_root_matches(
    source: &Source,
    root: &str,
    local: bool,
    aliases: &std::collections::BTreeMap<String, String>,
    overlap: bool,
) -> bool {
    let original = if source.provider == "codex" {
        crate::codex_auth::home_path(&source.path)
    } else {
        source.path.clone()
    };
    let value = aliases.get(&original).unwrap_or(&original);
    if same_root(value, root, local) {
        return true;
    }
    if !overlap {
        return false;
    }
    let canonical = |s: &str| {
        if local {
            std::fs::canonicalize(expand(s))
                .unwrap_or_else(|_| expand(s))
                .to_string_lossy()
                .replace('\\', "/")
        } else {
            s.replace('\\', "/")
        }
    };
    let a = canonical(value);
    let b = canonical(root);
    a.starts_with(&format!("{}/", b.trim_end_matches('/')))
        || b.starts_with(&format!("{}/", a.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_targets_follow_source_pause_location_and_transport_settings() {
        let account = Account {
            id: "account".into(),
            provider: "codex".into(),
            ..Default::default()
        };
        let mut settings = Settings {
            agents: vec![AgentConfiguration {
                provider: "codex".into(),
                enabled: true,
                machine_ids: vec!["local".into()],
            }],
            accounts: vec![account.clone()],
            sources: vec![Source {
                id: "source".into(),
                path: "/native".into(),
                codex_binary: "custom-codex".into(),
                proxy: Some(ProxyConfig {
                    mode: "direct".into(),
                    url: String::new(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let row = NativeConnection {
            id: "ref".into(),
            provider: "codex".into(),
            machine_id: "local".into(),
            machine_fingerprint: hash("null"),
            root: "/native".into(),
            source_id: Some("source".into()),
            source_path: "/native".into(),
            config_path: "/native/auth.json".into(),
            kind: "login".into(),
            account_key: Some(accounts::key(&account)),
            status: "observed".into(),
            ..Default::default()
        };
        settings.native_connections.push(row.clone());
        let targets = quota_sources(&settings, &account);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].codex_binary, "custom-codex");
        assert_eq!(targets[0].proxy.as_ref().unwrap().mode, "direct");
        settings.sources[0].enabled = false;
        assert!(!available(&settings, &row));
        assert!(quota_sources(&settings, &account).is_empty());
        settings.sources[0].enabled = true;
        settings.sources[0].path = "/other-home".into();
        assert!(!available(&settings, &row));
        settings.sources[0].path = "/native".into();
        let mut remembered = row.clone();
        remembered.status = "remembered".into();
        assert!(!available(&settings, &remembered));
        let mut stale = row.clone();
        stale.status = "stale".into();
        settings.native_connections = vec![stale];
        assert!(quota_sources(&settings, &account).is_empty());
    }
    #[test]
    fn the_same_upstream_login_is_one_quota_target() {
        let account = Account {
            id: "account".into(),
            provider: "codex".into(),
            ..Default::default()
        };
        let mut settings = Settings {
            agents: vec![AgentConfiguration {
                provider: "codex".into(),
                enabled: true,
                machine_ids: vec!["local".into()],
            }],
            accounts: vec![account.clone()],
            ..Default::default()
        };
        let row = |id: &str, root: &str, upstream: Option<&str>| NativeConnection {
            id: id.into(),
            provider: "codex".into(),
            machine_id: "local".into(),
            machine_fingerprint: hash("null"),
            root: root.into(),
            config_path: format!("{root}/auth.json"),
            kind: "login".into(),
            account_key: Some(accounts::key(&account)),
            upstream_identity: upstream.map(str::to_owned),
            status: "observed".into(),
            ..Default::default()
        };
        settings.native_connections = vec![
            row("a", "/one", Some("openai:same")),
            row("b", "/two", Some("openai:same")),
            row("c", "/three", Some("openai:other")),
            row("d", "/four", None),
        ];
        let ids: Vec<_> = quota_sources(&settings, &account)
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["native-a", "native-c", "native-d"]);
    }
    #[test]
    fn another_clients_copy_of_a_native_login_is_not_queried_twice() {
        let account = |provider: &str| Account {
            id: provider.into(),
            provider: provider.into(),
            ..Default::default()
        };
        let (claude, opencode, pi) = (account("claude"), account("opencode"), account("pi"));
        let mut settings = Settings {
            agents: ["claude", "opencode", "pi"]
                .map(|p| AgentConfiguration {
                    provider: p.into(),
                    enabled: true,
                    machine_ids: vec!["local".into()],
                })
                .to_vec(),
            accounts: vec![claude.clone(), opencode.clone(), pi.clone()],
            ..Default::default()
        };
        let row = |a: &Account, selector: &str| NativeConnection {
            id: format!("{}-row", a.provider),
            provider: a.provider.clone(),
            machine_id: "local".into(),
            machine_fingerprint: hash("null"),
            root: format!("/{}", a.provider),
            config_path: format!("/{}/auth.json", a.provider),
            selector: selector.into(),
            kind: "login".into(),
            has_credential: true,
            account_key: Some(accounts::key(a)),
            upstream_identity: Some("anthropic:same".into()),
            status: "observed".into(),
            ..Default::default()
        };
        settings.native_connections = vec![
            row(&claude, "/claudeAiOauth"),
            row(&opencode, "/anthropic"),
            row(&pi, "/anthropic"),
        ];
        let ids = |s: &Settings, a: &Account| {
            quota_sources(s, a)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        // Native first: Claude Code's own login is the only target.
        assert_eq!(ids(&settings, &claude), ["native-claude-row"]);
        assert!(ids(&settings, &opencode).is_empty());
        assert!(ids(&settings, &pi).is_empty());
        // The client's card says where the quota is read instead of looking empty.
        assert_eq!(
            shadow_notice(&settings, &opencode).as_deref(),
            Some("与 Claude Code 中的登录为同一账户，额度只在那里查询")
        );
        assert_eq!(shadow_notice(&settings, &claude), None);
        // A native login without readable credentials or with polling disabled
        // must not shadow the other client's working token.
        settings.native_connections[0].has_credential = false;
        assert_eq!(ids(&settings, &opencode), ["native-opencode-row"]);
        settings.native_connections[0].has_credential = true;
        settings.accounts[0].quota_enabled = false;
        assert_eq!(ids(&settings, &opencode), ["native-opencode-row"]);
        settings.accounts[0].quota_enabled = true;
        // Without it, the first client holding the account reads it once.
        settings.native_connections.remove(0);
        assert_eq!(ids(&settings, &opencode), ["native-opencode-row"]);
        assert!(ids(&settings, &pi).is_empty());
        // A different server account, a remote copy or an unread token is never merged.
        settings.native_connections[1].upstream_identity = Some("anthropic:other".into());
        assert_eq!(ids(&settings, &pi), ["native-pi-row"]);
        settings.native_connections[1].has_credential = false;
        assert!(ids(&settings, &pi).is_empty());
    }
    #[test]
    fn opaque_logins_reuse_a_known_identity_and_never_guess_a_new_one() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(root.path()).unwrap();
        let doc = Document {
            root: "/oc".into(),
            path: "/oc/auth.json".into(),
            bytes: json!({"anthropic":{"type":"oauth","access":"sk-ant-oat01-PRIVATE","refresh":"R","expires":4102444800000i64}}).to_string().into_bytes(),
        };
        let fresh = || extract("opencode", "local", &doc).unwrap().remove(0);
        let mut known = fresh().connection;
        known.identity_key = Some("opencode:known".into());
        known.upstream_identity = Some("anthropic:known".into());
        known.email = "dev@example.test".into();
        let settings = Settings {
            native_connections: vec![known.clone()],
            // Unreachable on purpose: a request must never be needed here.
            proxy: ProxyConfig {
                mode: "custom".into(),
                url: "http://127.0.0.1:9".into(),
            },
            ..Default::default()
        };
        let save = |fingerprint: &str| {
            store
                .db
                .execute(
                    "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    rusqlite::params![format!("native-fingerprint:{}", known.id), fingerprint],
                )
                .unwrap();
        };
        // The token the last detection confirmed: no request, same account.
        let mut row = fresh();
        save(&row.fingerprint);
        probe_identities(&store, &settings, std::slice::from_mut(&mut row), false);
        assert_eq!(row.connection.identity_key, known.identity_key);
        assert_eq!(row.connection.upstream_identity, known.upstream_identity);
        // A refreshed token that cannot be confirmed stays unassigned for a read ...
        save("older-token");
        let mut row = fresh();
        probe_identities(&store, &settings, std::slice::from_mut(&mut row), false);
        assert!(row.connection.identity_key.is_none());
        // ... while detection keeps the account and the old fingerprint, so it is asked again.
        let mut row = fresh();
        probe_identities(&store, &settings, std::slice::from_mut(&mut row), true);
        assert_eq!(row.connection.identity_key, known.identity_key);
        assert_eq!(row.fingerprint, "older-token");
        // Remote logins are never sent from this device.
        let mut remote = fresh();
        remote.connection.machine_id = "server".into();
        probe_identities(
            &store,
            &Settings::default(),
            std::slice::from_mut(&mut remote),
            true,
        );
        assert!(remote.connection.identity_key.is_none());
    }
    fn jwt(claims: Value) -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }
    #[test]
    fn cursor_ide_state_db_yields_one_login_shared_with_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let token = jwt(json!({"sub":"auth0|user_fixture","exp":4102444800i64}));
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE ItemTable(key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
            )
            .unwrap();
            for (k, v) in [
                ("cursorAuth/accessToken", token.as_str()),
                ("cursorAuth/cachedEmail", "dev@example.test"),
                ("cursorAuth/stripeMembershipType", "pro"),
                ("cursorAuth/refreshToken", "PRIVATE-REFRESH"),
            ] {
                db.execute("INSERT INTO ItemTable VALUES(?1,?2)", [k, v])
                    .unwrap();
            }
        }
        let d = read_file("/native", &path).unwrap();
        assert!(!String::from_utf8_lossy(&d.bytes).contains("PRIVATE-REFRESH"));
        let ide = extract("cursor", "local", &d).unwrap();
        assert_eq!(ide.len(), 1);
        let row = &ide[0].connection;
        assert_eq!(row.kind, "login");
        assert_eq!(row.email, "dev@example.test");
        assert_eq!(native_subscription(row).unwrap().mode, "api2");
        let cli = extract(
            "cursor",
            "local",
            &doc("cli-config.json", json!({"accessToken":token})),
        )
        .unwrap();
        assert_eq!(cli[0].connection.identity_key, row.identity_key);
        assert_eq!(cli[0].connection.upstream_identity, row.upstream_identity);
        assert!(serde_json::to_string(row).unwrap().find(&token).is_none());
        // The membership the IDE cached becomes the card's plan (never stored).
        assert_eq!(ide[0].plan, "Cursor Pro");
        assert_eq!(cli[0].plan, "");
        // Cursor 3 can seal the token with the OS keychain; that key is never read.
        let sealed = dir.path().join("sealed").join("state.vscdb");
        std::fs::create_dir_all(sealed.parent().unwrap()).unwrap();
        {
            let db = rusqlite::Connection::open(&sealed).unwrap();
            db.execute_batch(
                "CREATE TABLE ItemTable(key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
            )
            .unwrap();
            for (k, v) in [
                (
                    "cursorAuth/accessToken",
                    "cursor-safe-storage:v1:UFJJVkFURQ==",
                ),
                ("cursorAuth/cachedEmail", "dev@example.test"),
            ] {
                db.execute("INSERT INTO ItemTable VALUES(?1,?2)", [k, v])
                    .unwrap();
            }
        }
        let sealed = extract("cursor", "local", &read_file("/native", &sealed).unwrap()).unwrap();
        assert_eq!(sealed.len(), 1);
        assert!(!sealed[0].connection.has_credential && sealed[0].secret.is_empty());
        assert_eq!(sealed[0].connection.email, "dev@example.test");
        assert!(native_subscription(&sealed[0].connection).is_none());
        assert_eq!(cursor_plan("pro_plus"), "Cursor Pro+");
    }
    #[test]
    fn opencode_credential_table_and_unreadable_pi_auth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let access = jwt(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":"ws","chatgpt_user_id":"u"}}),
        );
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE credential(id text PRIMARY KEY, integration_id text, label text NOT NULL, value text NOT NULL, connector_id text, method_id text, active integer, time_created integer NOT NULL, time_updated integer NOT NULL)").unwrap();
            // OpenCode selects the `active` row of an integration, else its newest.
            for (id, integration, value, active, created) in [
                ("1", "openai", json!({"type":"oauth","methodID":"chatgpt-browser","access":access,"refresh":"PRIVATE-R","expires":4102444800000i64,"metadata":{"accountID":"ws"}}).to_string(), Some(1), 1),
                ("2", "deepseek", "PRIVATE-KEY".to_owned(), Some(1), 1),
                ("3", "anthropic", json!({"type":"key","key":"PRIVATE-OLD"}).to_string(), Some(0), 9),
                ("4", "anthropic", json!({"type":"oauth","methodID":"oauth","access":"sk-ant-oat01-PRIVATE","refresh":"PRIVATE-R2","expires":4102444800000i64}).to_string(), Some(1), 2),
                // Imported from auth.json: no flag, the newest wins.
                ("5", "github-copilot", json!({"type":"oauth","methodID":"device","access":"gho_OLD","refresh":"gho_OLD","expires":0}).to_string(), None, 1),
                ("6", "github-copilot", json!({"type":"oauth","methodID":"device","access":"gho_PRIVATE","refresh":"gho_PRIVATE","expires":0}).to_string(), None, 3),
                ("7", "opencode-go", json!({"type":"key","key":"PRIVATE-GO"}).to_string(), Some(1), 1),
                ("8", "claude-agent", json!({"type":"external","methodID":"claude-login"}).to_string(), Some(1), 1),
            ] {
                db.execute("INSERT INTO credential VALUES(?1,?2,'l',?3,NULL,NULL,?4,?5,?5)", rusqlite::params![id, integration, value, active, created]).unwrap();
            }
            db.execute("INSERT INTO credential VALUES('9',NULL,'l','{\"type\":\"key\",\"key\":\"PRIVATE-ORPHAN\"}','legacy','m',1,1,1)", []).unwrap();
        }
        let d = read_file("/native", &path).unwrap();
        let rows = extract("opencode", "local", &d).unwrap();
        let by = |selector: &str| {
            rows.iter()
                .find(|r| r.connection.selector == selector)
                .unwrap_or_else(|| panic!("{selector}"))
        };
        assert_eq!(
            rows.len(),
            5,
            "{:?}",
            rows.iter()
                .map(|r| &r.connection.selector)
                .collect::<Vec<_>>()
        );
        let openai = by("/openai");
        assert_eq!(openai.connection.kind, "login");
        let expected =
            crate::codex_auth::files::identity(&json!({"tokens":{"access_token":openai.secret}}))
                .unwrap();
        assert_eq!(
            openai.connection.upstream_identity,
            Some(format!("openai:{}", expected.key))
        );
        assert_eq!(upstream_kind(&openai.connection), Some("codex"));
        // An `active` OAuth row beats a newer inactive key; the API key is not a login.
        let anthropic = by("/anthropic");
        assert_eq!(
            (
                anthropic.connection.kind.as_str(),
                anthropic.secret.as_str()
            ),
            ("login", "sk-ant-oat01-PRIVATE")
        );
        assert!(
            anthropic.connection.identity_key.is_none(),
            "an opaque token needs the profile request"
        );
        assert_eq!(anthropic.connection.note, "已识别 Claude 订阅登录");
        // `expires: 0` is a GitHub token without expiry, not a day-old stale login.
        let copilot = by("/github-copilot");
        assert_eq!(copilot.secret, "gho_PRIVATE");
        assert_eq!(copilot.connection.status, "observed");
        assert!(copilot.connection.token_expired_at.is_none());
        assert_eq!(
            copilot.fingerprint,
            hash(&copilot.upstream_auth.as_ref().unwrap().to_string())
        );
        assert_eq!(by("/opencode-go").connection.kind, "api");
        assert_eq!(
            native_subscription(&by("/opencode-go").connection)
                .unwrap()
                .mode,
            "go"
        );
        assert_eq!(by("/deepseek").connection.kind, "api");
        let text =
            serde_json::to_string(&rows.iter().map(|r| &r.connection).collect::<Vec<_>>()).unwrap();
        assert!(!text.contains("PRIVATE") && !text.contains("gho_"));
        let broken = Document {
            root: "/native".into(),
            path: "auth.json".into(),
            bytes: b"{half".to_vec(),
        };
        assert!(extract("pi", "local", &broken).unwrap().is_empty());
        assert!(extract("opencode", "local", &broken).is_err());
        // Pi keeps the GitHub token as `refresh` and Codex logins as `openai-codex`.
        let pi = extract(
            "pi",
            "local",
            &doc(
                "auth.json",
                json!({
                    "github-copilot":{"type":"oauth","refresh":"gho_PI","access":"tid=1;exp=1","expires":1},
                    "openai-codex":{"type":"oauth","access":access,"refresh":"R","expires":4102444800000i64,"accountId":"ws"}
                }),
            ),
        )
        .unwrap();
        let copilot = pi
            .iter()
            .find(|r| r.connection.selector == "/github-copilot")
            .unwrap();
        assert_eq!(
            (copilot.secret.as_str(), copilot.connection.status.as_str()),
            ("gho_PI", "observed")
        );
        let codex = pi
            .iter()
            .find(|r| r.connection.selector == "/openai-codex")
            .unwrap();
        assert_eq!(
            codex.connection.upstream_identity,
            openai.connection.upstream_identity
        );
    }
    #[test]
    fn upstream_workspace_switch_and_work_identity_do_not_create_false_matches() {
        let access = jwt(
            json!({"sub":"same-sub", "https://api.openai.com/auth":{"chatgpt_account_id":"default", "chatgpt_user_id":"u"}}),
        );
        let row = |workspace: &str| {
            extract("opencode", "local", &doc("auth.json", json!({"openai":{"type":"oauth","access":access,"metadata":{"accountID":workspace}}}))).unwrap().remove(0)
        };
        let a = row("a");
        let b = row("b");
        assert_ne!(a.connection.identity_key, b.connection.identity_key);
        assert_ne!(
            a.connection.upstream_identity,
            b.connection.upstream_identity
        );
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(
            crate::codex_auth::files::external(b.upstream_auth.as_ref().unwrap()).unwrap()["chatgptAccountId"],
            "b"
        );
        let copilot = extract(
            "pi",
            "local",
            &doc(
                "auth.json",
                json!({"github-copilot":{"type":"oauth","access":access,"refresh":"gho_PRIVATE"}}),
            ),
        )
        .unwrap()
        .remove(0);
        assert!(
            copilot.connection.identity_key.is_none(),
            "derived token subject must not bypass GitHub identity lookup"
        );
        let work = extract("kimi", "local", &doc("daimon/config.json", json!({"credentials":{"kimiWeb":{"accessToken":"PRIVATE-WEB","userId":"kimi-1"},"kimiCode":{"apiKey":"PRIVATE-KEY"}}}))).unwrap();
        assert_eq!(work.len(), 1);
        assert_eq!(
            work[0].connection.identity_key,
            Some(format!("kimi:{}", hash("kimi-1")))
        );
        assert!(work[0].secret.is_empty());
        assert!(native_subscription(&work[0].connection).is_none());
    }
    #[test]
    fn moonshot_key_in_kimi_config_queries_balance_natively() {
        let mut d = doc("config.toml", json!({}));
        d.bytes =
            b"[providers.m]\nbase_url='https://api.moonshot.cn/v1'\napi_key='PRIVATE'\n".to_vec();
        let rows = extract("kimi", "local", &d).unwrap();
        assert_eq!(
            native_subscription(&rows[0].connection).unwrap().mode,
            "moonshot-cn"
        );
        let mut other = rows[0].connection.clone();
        other.base_url = "https://api.kimi.com/coding/v1".into();
        assert!(native_subscription(&other).is_none());
    }

    #[test]
    fn kimi_jwt_subject_is_resolved_to_the_same_server_user_id_as_work() {
        let token = format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD
                .encode(json!({"sub":"a-coding-subject","exp":4102444800i64}).to_string())
        );
        let rows = extract(
            "kimi",
            "local",
            &doc("auth.json", json!({"access_token":token})),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].connection.identity_key.is_none());
        assert_eq!(probe_kind(&rows[0].connection), Some("kimi"));
    }

    #[test]
    fn codebuddy_history_directory_remembers_owner_without_claiming_current_login() {
        let temp = tempfile::tempdir().unwrap();
        let history = temp
            .path()
            .join("CodeBuddyExtension/Data/user/CodeBuddy/user/history");
        std::fs::create_dir_all(&history).unwrap();
        let document = read_file(history.to_str().unwrap(), &history).unwrap();
        let rows = extract("codebuddy", "local", &document).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.connection.identity_key,
            Some(format!("codebuddy:{}", hash("user")))
        );
        assert_eq!(row.connection.status, "remembered");
        assert!(!row.connection.has_credential);
        assert!(row.secret.is_empty());
    }
    #[test]
    fn kimi_login_reads_its_own_regions_usage() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().to_string_lossy().into_owned();
        // `kimi login` writes credentials/<slot>.json in snake case; never refreshed here.
        let mut login = doc(
            &format!("{root}/credentials/kimi-code.json"),
            json!({"access_token":"PRIVATE-ACCESS","refresh_token":"PRIVATE-REFRESH","expires_at":4102444800i64,"token_type":"Bearer"}),
        );
        login.root = root.clone();
        let mut rows = extract("kimi", "local", &login).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].secret, "PRIVATE-ACCESS");
        let config = |text: &str| Document {
            root: root.clone(),
            path: format!("{root}/config.toml"),
            bytes: text.as_bytes().to_vec(),
        };
        let cn = config(
            "[providers.\"managed:kimi-code\"]\ntype='kimi'\nbase_url='https://api.kimi.com/coding/v1'\n[providers.\"managed:kimi-code\".oauth]\nstorage='file'\nkey='oauth/kimi-code'\n",
        );
        kimi_bases("kimi", &[cn], &mut rows);
        assert_eq!(native_subscription(&rows[0].connection).unwrap().mode, "cn");
        let global = config(
            "[providers.\"managed:kimi-code\"]\ntype='kimi'\nbase_url='https://api.kimi.ai/coding/v1'\n[providers.\"managed:kimi-code\".oauth]\nkey='oauth/kimi-code-env-0123456789abcdef'\noauth_host='https://auth.kimi.ai'\n",
        );
        kimi_bases("kimi", &[global], &mut rows);
        assert_eq!(
            native_subscription(&rows[0].connection).unwrap().mode,
            "global"
        );
        // Before any config: the install-channel marker, then mainland China.
        kimi_bases("kimi", &[], &mut rows);
        assert_eq!(rows[0].connection.base_url, KIMI_CN);
        std::fs::write(home.path().join("region"), "global\n").unwrap();
        kimi_bases("kimi", &[], &mut rows);
        assert_eq!(rows[0].connection.base_url, KIMI_GLOBAL);
        // An opaque token is identified by the region's `/me`, never by its hash.
        assert_eq!(probe_kind(&rows[0].connection), Some("kimi"));
        let mut api = rows[0].connection.clone();
        api.kind = "api".into();
        assert_eq!(probe_kind(&api), None);
    }
    fn doc(path: &str, value: Value) -> Document {
        Document {
            root: "/native".into(),
            path: path.into(),
            bytes: serde_json::to_vec(&value).unwrap(),
        }
    }
    #[test]
    fn trae_login_is_read_only_when_stored_in_plain_text() {
        let path = "/native/User/globalStorage/storage.json";
        // Trae 3.6.4 / Trae CN 3.4.11 (installed, not signed in): an app-encrypted
        // envelope (base64 of "tc\x05\x10…"). Never decrypted: the row only says
        // a login exists and carries no credential or quota target.
        let encrypted = doc(
            path,
            json!({"iCubeAuthInfo://icube.cloudide":"dGMFEAAAPRIVATE_ENVELOPE","telemetry.machineId":"m"}),
        );
        for provider in ["trae", "trae-cn"] {
            let rows = extract(provider, "local", &encrypted).unwrap();
            assert_eq!(rows.len(), 1);
            let row = &rows[0].connection;
            assert!(!row.has_credential && row.identity_key.is_none());
            assert_eq!(row.note, "Trae 登录已加密保存；额度请在官方用量页查看");
            assert!(native_subscription(row).is_none());
            assert!(!serde_json::to_string(row).unwrap().contains("PRIVATE"));
        }
        // A token refresh rewrites the envelope; the row stays the same login.
        let refreshed = doc(
            path,
            json!({"iCubeAuthInfo://icube.cloudide":"dGMFEAAAOTHER_ENVELOPE"}),
        );
        assert_eq!(
            extract("trae", "local", &refreshed).unwrap()[0].fingerprint,
            extract("trae", "local", &encrypted).unwrap()[0].fingerprint
        );
        assert!(
            extract(
                "trae",
                "local",
                &doc(path, json!({"telemetry.machineId":"m"}))
            )
            .unwrap()
            .is_empty()
        );

        // Plain-text userInfo (field names from the client's authUtil): token,
        // userId, account and expiredAt; the refresh token is never kept.
        let login = json!({"token":"PRIVATE_JWT","refreshToken":"PRIVATE_REFRESH","userId":"7001","expiredAt":"2099-01-01T00:00:00Z",
            "host":"growsg-normal.trae.ai","account":{"email":"dev@example.com","username":"PRIVATE_PHONE"}});
        let plain = doc(
            path,
            json!({"iCubeAuthInfo://icube.cloudide":login.to_string()}),
        );
        let rows = extract("trae", "local", &plain).unwrap();
        let row = &rows[0];
        assert!(row.connection.has_credential);
        assert_eq!(row.connection.email, "dev@example.com");
        assert_eq!(row.connection.note, "已识别 Trae 登录");
        let identity = row.connection.identity_key.clone().unwrap();
        assert!(identity.starts_with("trae:"));
        assert_eq!(
            row.connection.upstream_identity.as_deref(),
            Some(identity.as_str())
        );
        assert_eq!(row.connection.token_expired_at, None);
        assert_eq!(native_subscription(&row.connection).unwrap().mode, "usage");
        let secret: Value = serde_json::from_str(&row.secret).unwrap();
        assert_eq!(
            secret,
            json!({"token":"PRIVATE_JWT","host":"growsg-normal.trae.ai"})
        );
        assert!(
            !serde_json::to_string(&row.connection)
                .unwrap()
                .contains("PRIVATE")
        );
        // The same login in Trae CN is a different account system.
        let cn = extract(
            "trae-cn",
            "local",
            &doc(path, json!({"iCubeAuthInfo://icube.cloudide":login})),
        )
        .unwrap();
        assert!(
            cn[0]
                .connection
                .identity_key
                .as_deref()
                .unwrap()
                .starts_with("trae-cn:")
        );
        let expired = doc(
            path,
            json!({"iCubeAuthInfo://icube.cloudide":{"token":"t","userId":"7001","expiredAt":"2020-01-01T00:00:00Z"}}),
        );
        assert!(
            extract("trae", "local", &expired).unwrap()[0]
                .connection
                .token_expired_at
                .is_some()
        );
        assert_eq!(spec("trae").1, ["User/globalStorage/storage.json"]);
        assert!(has_files("trae-cn"));
    }
    #[test]
    fn trae_usage_is_queried_at_most_every_ten_minutes() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let account = Account {
            id: "a".into(),
            provider: "trae".into(),
            ..Default::default()
        };
        let row = NativeConnection {
            id: "row".into(),
            provider: "trae".into(),
            ..Default::default()
        };
        // The first read goes out and is recorded; the next one inside the floor
        // reuses the snapshot that read produced instead of calling again.
        assert!(
            trae_recent(&store, &account, &row, false)
                .unwrap()
                .is_none()
        );
        assert!(trae_recent(&store, &account, &row, false).is_err());
        let snapshot = QuotaSnapshot {
            provider: "trae".into(),
            account_id: "a".into(),
            meters: vec![QuotaMeter {
                id: "pack-0-credits".into(),
                unit: "credits".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        store
            .db
            .execute(
                "INSERT INTO quotas VALUES(?1,?2)",
                rusqlite::params![
                    accounts::key(&account),
                    serde_json::to_string(&snapshot).unwrap()
                ],
            )
            .unwrap();
        let recent = trae_recent(&store, &account, &row, false).unwrap().unwrap();
        assert_eq!(recent.meters[0].id, "pack-0-credits");
        store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key='trae-usage-at:row'",
                [(now() - 601).to_string()],
            )
            .unwrap();
        assert!(
            trae_recent(&store, &account, &row, false)
                .unwrap()
                .is_none()
        );
        // An expired token waits for the IDE: no call, and the "open Trae" hint.
        assert_eq!(
            trae_recent(&store, &account, &row, true)
                .unwrap_err()
                .to_string(),
            stale_note("trae")
        );
    }
    #[test]
    fn copilot_tokens_come_from_plain_text_files_and_the_environment_only() {
        let secret = "gho_PRIVATE_NEVER_SERIALIZE";
        // Legacy IDE plugins: apps.json keyed by `github.com:<app id>`; GHE entries ignored.
        let apps = doc(
            "/home/.config/github-copilot/apps.json",
            json!({"github.com:Iv1.fixture":{"user":"Octo-Fixture","oauth_token":secret,"githubAppId":"Iv1.fixture"},
                   "corp.ghe.com:Iv1.fixture":{"user":"other","oauth_token":"gho_other"}}),
        );
        let rows = extract("copilot", "local", &apps).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0].connection;
        assert_eq!(
            (row.kind.as_str(), row.email.as_str()),
            ("login", "Octo-Fixture")
        );
        assert!(
            row.identity_key.is_none(),
            "a cached login is not a numeric id"
        );
        assert!(row.upstream_identity.is_none());
        assert!(!serde_json::to_string(row).unwrap().contains(secret));
        assert_eq!(
            native_subscription(row).map(|c| c.mode),
            Some("github".into())
        );
        // gh hosts.yml when gh stores its token in plain text.
        let hosts = Document {
            root: "/native".into(),
            path: "/home/.config/gh/hosts.yml".into(),
            bytes: format!("github.com:\n    user: octo-fixture\n    oauth_token: {secret}\n    git_protocol: https\n").into_bytes(),
        };
        let rows = extract("copilot", "local", &hosts).unwrap();
        assert_eq!(rows[0].connection.identity_key, row.identity_key);
        // The CLI's JSONC config: no token key means not logged in, not an error.
        let config = Document {
            root: "/native".into(),
            path: "/home/.copilot/config.json".into(),
            bytes: b"// managed\n{\n  \"firstLaunchAt\": \"2026-10-29T22:56:38.881Z\"\n}\n"
                .to_vec(),
        };
        assert!(extract("copilot", "local", &config).unwrap().is_empty());
        let config = Document {
            root: "/native".into(),
            path: "/home/.copilot/config.json".into(),
            bytes: format!("// managed\n{{\"last_logged_in_user\":{{\"host\":\"https://github.com\",\"login\":\"b\"}},\"copilot_tokens\":{{\"https://github.com:a\":\"gho_a\",\"https://github.com:b\":\"{secret}\"}}}}").into_bytes(),
        };
        let rows = extract("copilot", "local", &config).unwrap();
        assert_eq!(rows.len(), 1, "only the CLI's current user");
        assert_eq!(rows[0].connection.email, "b");
        assert!(
            extract(
                "copilot",
                "local",
                &doc("/home/.copilot/config.json", json!([]))
            )
            .unwrap()
            .is_empty()
        );
        // Environment token: no identity is known until `/user` supplies its id.
        let env = Document {
            root: "/native".into(),
            path: "env:GH_TOKEN".into(),
            bytes: json!({"token":"ghp_classic"}).to_string().into_bytes(),
        };
        let rows = extract("copilot", "local", &env).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].connection.identity_key.is_none());
        assert!(rows[0].connection.note.contains("ghp_"));
        assert!(virtual_path(&rows[0].connection.config_path));
        // File-backed remote rows use the same adapter on the owning host.
        let mut remote = row.clone();
        remote.machine_id = "host".into();
        assert_eq!(native_subscription(&remote).unwrap().mode, "github");
        remote.config_path = "env:GH_TOKEN".into();
        assert_eq!(native_subscription(&remote).unwrap().mode, "github");
        remote.config_path = "env:UNRELATED_SECRET".into();
        assert!(native_subscription(&remote).is_none());
        remote.config_path = COPILOT_GRANTED.into();
        assert!(native_subscription(&remote).is_none());
    }
    #[test]
    fn copilot_tokens_without_a_login_are_named_by_github_once_per_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let token = |path: &str, secret: &str| {
            let doc = Document {
                root: "/home/.copilot".into(),
                path: path.into(),
                bytes: json!({"token":secret}).to_string().into_bytes(),
            };
            extract("copilot", "local", &doc).unwrap().remove(0)
        };
        let calls = std::cell::RefCell::new(vec![]);
        let lookup = |secret: &str| {
            calls.borrow_mut().push(secret.to_owned());
            match secret {
                "gho_octo" => Ok(crate::upstream::github_identity(
                    &json!({"id":7,"login":"Octo-Fixture"}),
                )
                .unwrap()),
                _ => Err(anyhow::anyhow!("GitHub 令牌无效或已过期")),
            }
        };
        let mut settings = Settings::default();
        let mut rows = vec![
            token("env:COPILOT_GITHUB_TOKEN", "gho_octo"),
            token(COPILOT_GRANTED, "gho_revoked"),
            token("env:GITHUB_TOKEN", "ghp_classic"),
        ];
        copilot_identities(&store, &settings, &mut rows, lookup);
        assert_eq!(
            *calls.borrow(),
            ["gho_octo", "gho_revoked"],
            "ghp_ is never sent"
        );
        let named = &rows[0].connection;
        assert_eq!(named.email, "Octo-Fixture");
        assert_eq!(named.identity_key.as_deref(), Some("copilot:7"));
        // Same upstream identity as a plain-text login of the same user, so one quota target.
        let apps = doc(
            "/home/.config/github-copilot/apps.json",
            json!({"github.com:Iv1.fixture":{"user":"octo-fixture","oauth_token":"gho_octo"}}),
        );
        let mut file = extract("copilot", "local", &apps).unwrap().remove(0);
        copilot_identities(&store, &settings, std::slice::from_mut(&mut file), lookup);
        assert_eq!(named.upstream_identity, file.connection.upstream_identity);
        assert!(rows[1].connection.identity_key.is_none());
        assert_eq!(rows[1].connection.note, "GitHub 令牌无效或已过期");
        assert!(rows[2].connection.identity_key.is_none());
        assert!(rows[2].connection.note.contains("ghp_"));
        assert!(!serde_json::to_string(named).unwrap().contains("gho_octo"));

        // The scan stored the row and its fingerprint: the same token is not looked up again.
        settings.native_connections.push(named.clone());
        store
            .db
            .execute(
                "INSERT INTO kv(key,value) VALUES(?1,?2)",
                rusqlite::params![
                    format!("native-fingerprint:{}", named.id),
                    rows[0].fingerprint
                ],
            )
            .unwrap();
        calls.borrow_mut().clear();
        let mut again = vec![token("env:COPILOT_GITHUB_TOKEN", "gho_octo")];
        copilot_identities(&store, &settings, &mut again, lookup);
        assert!(calls.borrow().is_empty());
        assert_eq!(again[0].connection.identity_key, named.identity_key);
        // A failed changed token retains the old account only as history. The
        // old fingerprint prevents the new credential from reading its quota.
        let mut changed = vec![token("env:COPILOT_GITHUB_TOKEN", "gho_someone_else")];
        assert!(!reuse_verified_identity(&store, &settings, &mut changed[0]));
        copilot_identities(&store, &settings, &mut changed, lookup);
        assert_eq!(*calls.borrow(), ["gho_someone_else"]);
        assert_eq!(changed[0].connection.identity_key, named.identity_key);
        assert_eq!(changed[0].fingerprint, rows[0].fingerprint);
        let mut current = token("env:COPILOT_GITHUB_TOKEN", "gho_someone_else");
        assert_ne!(current.fingerprint, changed[0].fingerprint);
        assert!(!reuse_verified_identity(&store, &settings, &mut current));
        assert!(current.connection.identity_key.is_none());
    }
    #[test]
    fn workbuddy_identity_comes_from_the_latest_session_without_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbuddy.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE sessions(id TEXT,user_id TEXT,updated_at INTEGER,last_activity_at INTEGER,deleted_at INTEGER);INSERT INTO sessions VALUES('a','old-user',1,1,NULL),('b','new-user',2,5,NULL),('c','gone',9,9,1)").unwrap();
        let doc = read_file("/wb", &path).unwrap();
        let rows = extract("workbuddy", "local", &doc).unwrap();
        assert_eq!(rows.len(), 1);
        let c = &rows[0].connection;
        assert!(!c.has_credential);
        assert_eq!(
            c.identity_key.as_deref(),
            Some(format!("workbuddy:{}", hash("new-user")).as_str())
        );
        assert!(!serde_json::to_string(c).unwrap().contains("new-user"));
        let empty = dir.path().join("empty/workbuddy.db");
        std::fs::create_dir(empty.parent().unwrap()).unwrap();
        rusqlite::Connection::open(&empty)
            .unwrap()
            .execute_batch("CREATE TABLE t(v)")
            .unwrap();
        assert!(
            extract("workbuddy", "local", &read_file("/wb", &empty).unwrap())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn workbuddy_account_snapshot_names_the_current_login_and_edition() {
        let dir = tempfile::tempdir().unwrap();
        let skeleton = dir.path().join("storage/skeleton");
        std::fs::create_dir_all(&skeleton).unwrap();
        let snapshot = skeleton.join("account-snapshot.json");
        // Shape written by WorkBuddy 5.5.6; values are made up.
        std::fs::write(&snapshot, r#"{"primary":{"version":1,"uid":"fixture-user","nickname":"Fixture","type":"ultimate","isAdmin":false,"oneidAccountId":"","savedAt":1}}"#).unwrap();
        let rows = extract("workbuddy", "local", &read_file("/wb", &snapshot).unwrap()).unwrap();
        assert_eq!(rows.len(), 1);
        let c = &rows[0].connection;
        assert!(!c.has_credential);
        assert_eq!(c.email, "Fixture");
        assert_eq!(c.note, "已识别 WorkBuddy 企业账户；余额需登录查看");
        // Same identity as the sessions of that user and an Aieyes login.
        assert_eq!(
            c.identity_key.as_deref(),
            Some(format!("workbuddy:{}", hash("fixture-user")).as_str())
        );
        assert!(!serde_json::to_string(c).unwrap().contains("fixture-user"));
        // With the snapshot directory, old sessions never name a current login:
        // after logout (snapshot removed) the account becomes remembered.
        let db = dir.path().join("workbuddy.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(include_str!("../tests/fixtures/workbuddy/workbuddy.sql"))
            .unwrap();
        assert!(
            extract("workbuddy", "local", &read_file("/wb", &db).unwrap())
                .unwrap()
                .is_empty()
        );
        std::fs::write(
            &snapshot,
            r#"{"primary":{"version":1,"uid":"","type":"personal"}}"#,
        )
        .unwrap();
        assert!(
            extract("workbuddy", "local", &read_file("/wb", &snapshot).unwrap())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn every_native_api_adapter_keeps_keys_out_of_public_metadata() {
        let key = "PRIVATE-NEVER-SERIALIZE";
        let fixtures = vec![
            (
                "claude",
                "settings.json",
                json!({"env":{"ANTHROPIC_BASE_URL":"https://fixture.test/v1","ANTHROPIC_API_KEY":key,"ANTHROPIC_MODEL":"model"}}),
            ),
            (
                "codebuddy",
                "models.json",
                json!({"models":[{"id":"model","url":"https://fixture.test/v1","apiKey":key}]}),
            ),
            (
                "qoder",
                "settings.json",
                json!({"providers":{"original":{"baseUrl":"https://fixture.test/v1","apiKey":key,"models":[{"model":"model"}]}}}),
            ),
            (
                "opencode",
                "opencode.json",
                json!({"provider":{"original":{"options":{"baseURL":"https://fixture.test/v1","apiKey":key}}}}),
            ),
            (
                "pi",
                "models.json",
                json!({"providers":{"original":{"baseUrl":"https://fixture.test/v1","apiKey":key}}}),
            ),
            (
                "zcode",
                "provider_config.json",
                json!({"config":{"providerConfigRules":{"providerRules":[{"providerId":"original","config":{"api":{"baseUrl":"https://fixture.test/v1"},"access":{"apiKey":key}}}]}}}),
            ),
            (
                "codex",
                "auth.json",
                json!({"auth_mode":"apikey","OPENAI_API_KEY":key}),
            ),
        ];
        for (provider, path, v) in fixtures {
            let rows = extract(provider, "local", &doc(path, v)).unwrap();
            assert_eq!(rows.len(), 1, "{provider}");
            assert!(rows[0].connection.has_credential);
            assert!(
                !serde_json::to_string(&rows[0].connection)
                    .unwrap()
                    .contains(key)
            );
        }
        for provider in ["kimi", "grok"] {
            let mut d = doc("config.toml", json!({}));
            d.bytes=if provider=="kimi"{format!("[providers.original]\nbase_url='https://fixture.test/v1'\napi_key='{key}'\n[models.m]\nprovider='original'\nmodel='model'\n")}else{format!("[model.original]\nbase_url='https://fixture.test/v1'\napi_key='{key}'\nmodel='model'\n")}.into_bytes();
            let rows = extract(provider, "local", &d).unwrap();
            assert_eq!(rows.len(), 1);
            assert!(
                !serde_json::to_string(&rows[0].connection)
                    .unwrap()
                    .contains(key)
            );
        }
        for (provider, content) in [
            (
                "omp",
                format!(
                    "providers:\n  original:\n    baseUrl: https://fixture.test/v1\n    apiKey: {key}\n"
                ),
            ),
            (
                "deepseek-harness",
                format!(
                    "- id: llm-pi-ai\n  config:\n    providers:\n      original:\n        baseURL: https://fixture.test/v1\n        headers:\n          Authorization: Bearer {key}\n"
                ),
            ),
        ] {
            let d = Document {
                root: "/native".into(),
                path: "models.yml".into(),
                bytes: content.into_bytes(),
            };
            let rows = extract(provider, "local", &d).unwrap();
            assert_eq!(rows.len(), 1);
            assert!(
                !serde_json::to_string(&rows[0].connection)
                    .unwrap()
                    .contains(key)
            );
        }
    }
    #[test]
    fn multiple_routes_jsonc_and_expired_credentials_are_explicit() {
        let d=Document{root:"/native".into(),path:"opencode.jsonc".into(),bytes:br#"{/* comment */"provider":{"one":{"options":{"baseURL":"https://one.test/v1","apiKey":"one"}},"two":{"options":{"baseURL":"https://two.test/v1","apiKey":"two"}},},}"#.to_vec()};
        assert_eq!(extract("opencode", "local", &d).unwrap().len(), 2);
        let rows=extract("pi","local",&doc("auth.json",json!({"anthropic":{"type":"oauth","access":"PRIVATE","refresh":"PRIVATE-REFRESH","expires":1}}))).unwrap();
        assert_eq!(rows.len(), 1);
        // Expired for more than a day: the native client was not opened since.
        assert_eq!(rows[0].connection.status, "stale");
        assert_eq!(rows[0].connection.token_expired_at, Some(1));
        assert!(rows[0].connection.note.contains("一次即可刷新"));
        assert!(!rows[0].connection.note.contains("扫描"));
        assert!(rows[0].connection.identity_key.is_none());
        // Recently expired: still observed, the client refreshes it on next launch.
        let recent = (now() - 3600) * 1000;
        let rows = extract(
            "pi",
            "local",
            &doc(
                "auth.json",
                json!({"anthropic":{"type":"oauth","access":"PRIVATE","expires":recent}}),
            ),
        )
        .unwrap();
        assert_eq!(rows[0].connection.status, "observed");
        assert_eq!(rows[0].connection.token_expired_at, Some(recent / 1000));
        assert!(
            extract(
                "cursor",
                "local",
                &doc("auth.json", json!({"unknown":"PRIVATE"}))
            )
            .unwrap()
            .is_empty()
        );
        let rows=extract("grok","local",&doc("auth.json",json!({"https://auth.x.ai::one":{"key":"one"},"https://auth.x.ai::two":{"key":"two"}}))).unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].connection.id, rows[1].connection.id);
    }
    #[test]
    fn zcode_login_jwt_infers_region_and_feeds_zero_config_quota() {
        let payload =
            URL_SAFE_NO_PAD.encode(br#"{"sub":"jwt-sub","iss":"https://open.bigmodel.cn"}"#);
        let jwt = format!("e30.{payload}.sig");
        let d = Document {
            root: "/home/u/.zcode".into(),
            path: "/home/u/.zcode/v2/credentials.json".into(),
            bytes: serde_json::to_vec(&json!({"zcodeJwtToken":jwt,"zaiUserInfo":{"userId":42,"email":"user@example.test"}})).unwrap(),
        };
        let rows = extract("zcode", "local", &d).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0].connection;
        assert_eq!(row.kind, "login");
        assert_eq!(row.email, "user@example.test");
        assert!(
            row.identity_key
                .as_deref()
                .is_some_and(|k| k.starts_with("zcode:"))
        );
        assert_eq!(row.base_url, "https://open.bigmodel.cn");
        assert!(!serde_json::to_string(row).unwrap().contains(&jwt));
        assert_eq!(native_subscription(row).unwrap().mode, "cn");
        let mut remote = row.clone();
        remote.machine_id = "host".into();
        assert_eq!(native_subscription(&remote).unwrap().mode, "cn");
        // Coding Plan key rows pick their region from the API host.
        assert_eq!(
            zcode_region("https://api.z.ai/api/coding/paas/v4"),
            Some("global")
        );
        assert_eq!(zcode_region("https://open.bigmodel.cn/api"), Some("cn"));
        assert_eq!(zcode_region("https://z.ai.evil.test"), None);
        // No login provider hint: identity only, no quota target.
        let d = Document {
            bytes: serde_json::to_vec(&json!({"zcodeJwtToken":"opaque","zaiUserInfo":{}})).unwrap(),
            ..d
        };
        let rows = extract("zcode", "local", &d).unwrap();
        assert!(native_subscription(&rows[0].connection).is_none());
    }
    #[test]
    fn zcode_v2_flat_credentials_use_selected_profile_and_keep_manual_keys_private() {
        let jwt = format!(
            "h.{}.s",
            URL_SAFE_NO_PAD.encode(json!({"sub":"42","exp":4102444800_i64}).to_string())
        );
        let value = json!({
            "oauth:active_provider":"zai",
            "zcodejwttoken":jwt,
            "oauth:zai:user_info":json!({"user_id":"42","email":"zai@example.test"}).to_string(),
            "oauth:bigmodel:user_info":json!({"id":"org-project","rawProfile":{"user_id":"42","email":"cn@example.test"}}).to_string(),
            "oauth:zai:refresh_token":"PRIVATE-REFRESH",
            "account-provider:account:zai-individual-coding-plan:identity":"key-abcd",
            "account-provider:account:zai-individual-coding-plan:auth-source":"manual",
            "account-provider:coding-plan:account:zai-individual-coding-plan:account:key-abcd:api-key":"PRIVATE-PLAN"
        });
        let doc = Document {
            root: "/home/u/.zcode".into(),
            path: "/home/u/.zcode/v2/credentials.json".into(),
            bytes: serde_json::to_vec(&value).unwrap(),
        };
        let rows = extract("zcode", "local", &doc).unwrap();
        assert_eq!(rows.len(), 2);
        let login = rows.iter().find(|r| r.connection.kind == "login").unwrap();
        assert_eq!(login.connection.email, "zai@example.test");
        assert_eq!(
            native_subscription(&login.connection).unwrap().mode,
            "global"
        );
        let key = rows.iter().find(|r| r.connection.kind == "api").unwrap();
        assert_eq!(key.secret, "PRIVATE-PLAN");
        assert_eq!(native_subscription(&key.connection).unwrap().mode, "global");
        assert!(
            key.connection.identity_key.is_none(),
            "an API key is not proof of the OAuth account"
        );
        assert!(
            !serde_json::to_string(&rows.iter().map(|r| &r.connection).collect::<Vec<_>>())
                .unwrap()
                .contains("PRIVATE")
        );
        let mut cn = value.clone();
        cn["oauth:active_provider"] = json!("bigmodel");
        let cn = extract(
            "zcode",
            "remote",
            &Document {
                bytes: serde_json::to_vec(&cn).unwrap(),
                ..doc
            },
        )
        .unwrap();
        let cn = cn.iter().find(|r| r.connection.kind == "login").unwrap();
        assert_eq!(cn.connection.email, "cn@example.test");
        assert_ne!(cn.connection.identity_key, login.connection.identity_key);
    }
    #[test]
    fn grok_keeps_each_oidc_account_and_selects_recent_unexpired_credentials() {
        let rows = extract("grok", "local", &doc("auth.json", json!({
            "https://auth.x.ai::old":{"key":"PRIVATE-OLD","user_id":"one","expires_at":"2098-01-01T00:00:00Z"},
            "https://auth.x.ai::new":{"key":"PRIVATE-NEW","user_id":"two","expires_at":"2099-01-01T00:00:00Z"},
            "https://auth.x.ai::expired":{"key":"PRIVATE-EXPIRED","user_id":"three","expires_at":"2001-01-01T00:00:00Z"},
            "https://auth.x.ai.evil.test::foreign":{"key":"PRIVATE-FOREIGN"}
        }))).unwrap();
        assert_eq!(rows.len(), 3);
        let current = rows
            .iter()
            .find(|r| r.connection.status == "observed")
            .unwrap();
        assert_eq!(current.secret, "PRIVATE-NEW");
        assert_eq!(
            native_subscription(&current.connection).unwrap().mode,
            "subscription"
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.connection.status == "remembered")
                .count(),
            2
        );
        assert!(rows.iter().all(|r| r.connection.identity_key.is_some()));
    }
    #[test]
    fn deepseek_dsh_credentials_and_env_are_read_without_storing_keys() {
        let yaml = Document {
            root: "/home/u/.dsh".into(),
            path: "/home/u/.dsh/.credentials.yaml".into(),
            bytes: b"version: 1\nrecords:\n  client-connection/browser-session:\n    kind: grant\n    payload:\n      token: IGNORED\n  deepseek-account-platform/device:\n    kind: grant\n    payload:\n      id: 0d8f\n  deepseek-account-platform/default:\n    kind: grant\n    payload:\n      version: 1\n      token: PRIVATE-ACCOUNT\n      issuer: https://platform.deepseek.com\nrefs:\n  DEEPSEEK_API_KEY: sk-PRIVATE-REF\n".to_vec(),
        };
        let rows = extract("deepseek-harness", "local", &yaml).unwrap();
        assert_eq!(rows.len(), 2);
        let login = rows.iter().find(|r| r.connection.kind == "login").unwrap();
        assert_eq!(
            login.connection.selector,
            "/records/deepseek-account-platform~1default"
        );
        assert_eq!(
            native_subscription(&login.connection).unwrap().mode,
            "account"
        );
        let key = rows.iter().find(|r| r.connection.kind == "api").unwrap();
        assert_eq!(native_subscription(&key.connection).unwrap().mode, "api");
        let env = Document {
            root: "/home/u/.dsh".into(),
            path: "/home/u/.dsh/.env".into(),
            bytes: b"# comment\nexport DEEPSEEK_API_KEY=\"sk-PRIVATE-ENV\"\nOTHER=1\n".to_vec(),
        };
        let rows = extract("deepseek-harness", "local", &env).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].secret, "sk-PRIVATE-ENV");
        let comments = Document {
            bytes:b"DEEPSEEK_API_KEY=ignored\nexport DEEPSEEK_API_KEY=\"sk-PRIVATE-ENV\" # native comment\n".to_vec(),
            root:env.root.clone(), path:env.path.clone(),
        };
        assert_eq!(
            extract("deepseek-harness", "local", &comments).unwrap()[0].secret,
            "sk-PRIVATE-ENV"
        );
        let foreign = Document {
            bytes: String::from_utf8(yaml.bytes.clone())
                .unwrap()
                .replace(
                    "https://platform.deepseek.com",
                    "https://private.example.test",
                )
                .into_bytes(),
            root: yaml.root.clone(),
            path: yaml.path.clone(),
        };
        let foreign = extract("deepseek-harness", "local", &foreign).unwrap();
        assert!(
            native_subscription(
                &foreign
                    .iter()
                    .find(|r| r.connection.kind == "login")
                    .unwrap()
                    .connection
            )
            .is_none()
        );
        // A third-party base URL in .env is not the DeepSeek wallet.
        let other = Document {
            bytes: b"DEEPSEEK_API_KEY=sk-x\nDEEPSEEK_BASE_URL=https://proxy.test/v1\n".to_vec(),
            ..env
        };
        let rows = extract("deepseek-harness", "local", &other).unwrap();
        assert!(native_subscription(&rows[0].connection).is_none());
        for row in extract("deepseek-harness", "local", &yaml).unwrap() {
            assert!(
                !serde_json::to_string(&row.connection)
                    .unwrap()
                    .contains("PRIVATE")
            );
        }
    }
    #[test]
    fn invalid_configuration_errors_do_not_quote_secrets() {
        let d = Document {
            root: "/native".into(),
            path: "config.toml".into(),
            bytes: b"PRIVATE-SECRET = [".to_vec(),
        };
        assert!(
            !extract("kimi", "local", &d)
                .err()
                .unwrap()
                .to_string()
                .contains("PRIVATE")
        );
        assert_eq!(
            url("https://user:PRIVATE@fixture.test/key?secret=PRIVATE"),
            ""
        );
    }
    #[test]
    fn cline_and_kilo_logins_feed_prepaid_quota_without_exposing_tokens() {
        let secret = "workos:PRIVATE_NEVER_SERIALIZE";
        let future = (now() + 86400) * 1000;
        // Layout written by `cline auth` (v3.0.74); the Cline account block per Cline source.
        let providers = doc(
            "/native/settings/providers.json",
            json!({"version":1,"lastUsedProvider":"cline","providers":{
                "cline":{"settings":{"provider":"cline","auth":{"accessToken":secret,"refreshToken":"PRIVATE_REFRESH","expiresAt":future,"accountId":"usr_1"}}},
                "openrouter":{"settings":{"provider":"openrouter","apiKey":"sk-or-PRIVATE","model":"m"}},
                "openai-compatible":{"settings":{"provider":"openai-compatible","apiKey":"dummy","model":"Qwen3.8-27B","baseUrl":"http://10.0.0.1:1/v1"},"tokenSource":"manual"}
            }}),
        );
        let rows = extract("cline", "local", &providers).unwrap();
        assert_eq!(rows.len(), 3);
        let login = rows.iter().find(|r| r.connection.kind == "login").unwrap();
        assert_eq!(login.secret, secret);
        assert_eq!(login.connection.note, "已识别 Cline 账户");
        assert_eq!(
            login.connection.identity_key,
            Some(format!("cline:{}", hash("usr_1")))
        );
        assert!(
            login
                .connection
                .identity_key
                .as_deref()
                .unwrap()
                .starts_with("cline:")
        );
        assert_eq!(
            native_subscription(&login.connection).unwrap().mode,
            "account"
        );
        let modes: Vec<_> = rows
            .iter()
            .map(|r| native_subscription(&r.connection).map(|c| c.mode))
            .collect();
        assert_eq!(
            modes
                .iter()
                .filter(|m| m.as_deref() == Some("openrouter"))
                .count(),
            1
        );
        assert_eq!(modes.iter().filter(|m| m.is_none()).count(), 1);
        let public =
            serde_json::to_string(&rows.iter().map(|r| &r.connection).collect::<Vec<_>>()).unwrap();
        assert!(!public.contains("PRIVATE"));
        // Expired more than a day ago: kept as stale, never refreshed by Aieyes.
        let old = doc(
            "/native/settings/providers.json",
            json!({"providers":{"cline":{"settings":{"auth":{"accessToken":"t","expiresAt":(now() - 3 * 86400) * 1000}}}}}),
        );
        assert_eq!(
            extract("cline", "local", &old).unwrap()[0]
                .connection
                .status,
            "stale"
        );

        // auth.json written by `kilo auth` device login (oauth) and a pasted token (api).
        let auth = doc(
            "/native/auth.json",
            json!({"kilo":{"type":"oauth","access":"PRIVATE_KILO","refresh":"PRIVATE_KILO","expires":future,"accountId":"org_1"},
                   "openrouter":{"type":"api","key":"sk-or-PRIVATE"}}),
        );
        let rows = extract("kilo", "local", &auth).unwrap();
        assert!(
            rows.iter()
                .find(|r| r.connection.selector == "/kilo")
                .unwrap()
                .connection
                .identity_key
                .is_none()
        );
        let modes: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| native_subscription(&r.connection).map(|c| c.mode))
            .collect();
        assert_eq!(
            modes,
            BTreeSet::from(["openrouter".to_string(), "profile".to_string()])
        );
        let api = doc(
            "/native/auth.json",
            json!({"kilo":{"type":"api","key":"PRIVATE_KILO"}}),
        );
        let rows = extract("kilo", "local", &api).unwrap();
        assert_eq!(
            native_subscription(&rows[0].connection).unwrap().mode,
            "profile"
        );
    }

    #[test]
    fn prepaid_identity_uses_profile_and_only_reuses_an_unchanged_credential() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(temp.path()).unwrap();
        let mut settings = Settings::default();
        let auth = |token: &str| {
            doc(
                "/native/auth.json",
                json!({"kilo":{"type":"oauth","access":token,"refresh":"PRIVATE_REFRESH","expires":(now()+86400)*1000,"accountId":"organization-not-user"}}),
            )
        };
        let mut rows = extract("kilo", "local", &auth("PRIVATE_ONE")).unwrap();
        assert!(rows[0].connection.identity_key.is_none());
        let requests = std::cell::Cell::new(0);
        prepaid_identities(&store, &settings, &mut rows, |provider, secret| {
            assert_eq!(provider, "kilo");
            assert_eq!(secret, "PRIVATE_ONE");
            requests.set(requests.get() + 1);
            Ok(AccountIdentity {
                key: format!("kilo:{}", hash("user-id")),
                email: "user@example.test".into(),
                ..Default::default()
            })
        });
        assert_eq!(requests.get(), 1);
        assert_eq!(
            rows[0].connection.identity_key,
            Some(format!("kilo:{}", hash("user-id")))
        );
        settings.native_connections.push(rows[0].connection.clone());
        store
            .db
            .execute(
                "INSERT INTO kv VALUES(?1,?2)",
                rusqlite::params![
                    format!("native-fingerprint:{}", rows[0].connection.id),
                    rows[0].fingerprint
                ],
            )
            .unwrap();
        let mut again = extract("kilo", "local", &auth("PRIVATE_ONE")).unwrap();
        prepaid_identities(&store, &settings, &mut again, |_, _| {
            panic!("same credential must reuse profile identity")
        });
        assert_eq!(
            again[0].connection.identity_key,
            rows[0].connection.identity_key
        );
        let mut changed = extract("kilo", "local", &auth("PRIVATE_TWO")).unwrap();
        prepaid_identities(&store, &settings, &mut changed, |_, _| {
            Err(anyhow::anyhow!("profile lookup unavailable"))
        });
        assert!(changed[0].connection.identity_key.is_none());
        assert!(
            !serde_json::to_string(&again[0].connection)
                .unwrap()
                .contains("PRIVATE")
        );
    }

    #[test]
    fn cline_custom_settings_and_kilo_inline_openrouter_preserve_real_model_and_key_locations() {
        let rows=extract("cline","local",&doc("/custom/provider-state.json",json!({"providers":{
            "cline":{"settings":{"provider":"cline","apiKey":"workos:PRIVATE_MANUAL"}},
            "openrouter":{"settings":{"provider":"openrouter","auth":{"apiKey":"PRIVATE_ROUTER"},"model":"real-model"}}
        }}))).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.secret == "PRIVATE_ROUTER"
            && native_subscription(&r.connection).unwrap().mode == "openrouter"));
        assert!(
            rows.iter()
                .any(|r| r.secret == "workos:PRIVATE_MANUAL" && r.connection.kind == "login")
        );
        let rows=extract("kilo","local",&doc("/config/kilo.json",json!({"provider":{"openrouter":{"options":{"apiKey":"PRIVATE_ROUTER"},"models":{"real-model":{}}}}}))).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].connection.models, ["real-model"]);
        assert_eq!(
            native_subscription(&rows[0].connection).unwrap().mode,
            "openrouter"
        );
        let env = extract(
            "kilo",
            "local",
            &doc(
                "env:KILO_AUTH_CONTENT",
                json!({"kilo":{"type":"api","key":"PRIVATE_ENV"}}),
            ),
        )
        .unwrap();
        assert_eq!(env[0].secret, "PRIVATE_ENV");
        assert!(needs_prepaid_identity(&env[0]));
    }
    /// `data.sqlite3` with the tables kiro-cli 2.36.0 creates.
    fn kiro_db(path: &Path, rows: &[(&str, &str, Value)]) {
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE auth_kv(key TEXT PRIMARY KEY, value TEXT);CREATE TABLE state(key TEXT PRIMARY KEY, value BLOB);CREATE TABLE conversations_v2(key TEXT NOT NULL, conversation_id TEXT NOT NULL, value TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY (key, conversation_id))",
        )
        .unwrap();
        for (table, key, value) in rows {
            db.execute(
                &format!("INSERT INTO {table} VALUES(?1,?2)"),
                [*key, value.to_string().as_str()],
            )
            .unwrap();
        }
    }
    #[test]
    fn kiro_cli_and_ide_logins_are_one_identity_and_refresh_tokens_stay_behind() {
        let dir = tempfile::tempdir().unwrap();
        let arn = "arn:aws:codewhisperer:us-east-1:699475941385:profile/EHGA3GRVQMUK";
        let future = "2099-01-01T00:00:00Z";
        // Builder ID through kiro-cli (`start_url`) and through the IDE (`provider`).
        let path = dir.path().join("data.sqlite3");
        kiro_db(
            &path,
            &[
                (
                    "auth_kv",
                    "kirocli:odic:token",
                    json!({"access_token":"PRIVATE_ACCESS","refresh_token":"PRIVATE_REFRESH","expires_at":future,"region":"us-east-1","start_url":"https://view.awsapps.com/start","oauth_flow":"DeviceCode","scopes":["codewhisperer:completions"]}),
                ),
                (
                    "auth_kv",
                    "kirocli:odic:device-registration",
                    json!({"client_id":"c","client_secret":"PRIVATE_SECRET"}),
                ),
                (
                    "state",
                    "api.codewhisperer.profile",
                    json!({"arn":arn,"profile_name":"Default"}),
                ),
            ],
        );
        let d = read_file("/native", &path).unwrap();
        let raw = String::from_utf8_lossy(&d.bytes);
        assert!(!raw.contains("PRIVATE_REFRESH") && !raw.contains("PRIVATE_SECRET"));
        let cli = extract("kiro", "local", &d).unwrap();
        assert_eq!(cli.len(), 1);
        let row = &cli[0].connection;
        assert_eq!(
            (row.kind.as_str(), row.selector.as_str()),
            ("login", "/auth_kv/odic")
        );
        assert_eq!(row.token_expired_at, None);
        assert_eq!(native_subscription(row).unwrap().mode, "usage");
        let secret: Value = serde_json::from_str(&cli[0].secret).unwrap();
        assert_eq!(
            secret,
            json!({"accessToken":"PRIVATE_ACCESS","profileArn":arn,"expiresAt":epoch(&json!(future))})
        );
        assert!(!serde_json::to_string(row).unwrap().contains("PRIVATE"));
        let ide = extract(
            "kiro",
            "fixture",
            &doc(
                "/home/.aws/sso/cache/kiro-auth-token.json",
                json!({"accessToken":"PRIVATE_IDE","refreshToken":"PRIVATE_REFRESH","expiresAt":future,"clientIdHash":"h","authMethod":"IdC","provider":"BuilderId","region":"us-east-1","profileArn":arn}),
            ),
        )
        .unwrap();
        assert_eq!(ide[0].connection.selector, "/kiro-auth-token");
        assert!(row.identity_key.is_some());
        assert_eq!(ide[0].connection.identity_key, row.identity_key);
        assert_eq!(ide[0].connection.upstream_identity, row.upstream_identity);

        // Social sign-in: kiro-cli `provider: github` and the IDE `Github` agree;
        // a leftover expired OIDC token does not shadow it.
        let path = dir.path().join("social/data.sqlite3");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        kiro_db(
            &path,
            &[
                (
                    "auth_kv",
                    "kirocli:odic:token",
                    json!({"access_token":"OLD","expires_at":"2001-01-01T00:00:00Z","start_url":"https://view.awsapps.com/start"}),
                ),
                (
                    "auth_kv",
                    "kirocli:social:token",
                    json!({"access_token":"NEW","expires_at":future,"profile_arn":arn,"provider":"github"}),
                ),
            ],
        );
        let social = extract("kiro", "local", &read_file("/native", &path).unwrap()).unwrap();
        assert_eq!(social[0].connection.selector, "/auth_kv/social");
        let ide = extract(
            "kiro",
            "fixture",
            &doc("/k/kiro-auth-token.json", json!({"accessToken":"t","authMethod":"social","provider":"Github","profileArn":arn})),
        )
        .unwrap();
        assert_eq!(
            ide[0].connection.identity_key,
            social[0].connection.identity_key
        );
        assert_ne!(social[0].connection.identity_key, row.identity_key);

        // Expired everywhere: still observed (kiro-cli refreshes on the fallback).
        let path = dir.path().join("expired/data.sqlite3");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        kiro_db(
            &path,
            &[(
                "auth_kv",
                "kirocli:odic:token",
                json!({"access_token":"OLD","expires_at":"2001-01-01T00:00:00Z","start_url":"https://example.awsapps.com/start"}),
            )],
        );
        let expired = extract("kiro", "local", &read_file("/native", &path).unwrap()).unwrap();
        assert_eq!(expired[0].connection.status, "observed");
        assert!(expired[0].connection.token_expired_at.is_some());
        // No ARN (an IDE IAM Identity Center login without profile.json): no identity.
        let bare = extract(
            "kiro",
            "fixture",
            &doc(
                "/k/kiro-auth-token.json",
                json!({"accessToken":"t","provider":"Enterprise"}),
            ),
        )
        .unwrap();
        assert_eq!(bare[0].connection.identity_key, None);
        // Logged out: an empty database yields nothing.
        let path = dir.path().join("empty/data.sqlite3");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        kiro_db(&path, &[]);
        assert!(
            extract("kiro", "local", &read_file("/native", &path).unwrap())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn kiro_ide_profile_names_the_arn_of_an_identity_center_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        assert_eq!(kiro_profile_arn(&path), None);
        std::fs::write(
            &path,
            json!({"arn":"arn:aws:codewhisperer:eu-central-1:1:profile/ORG","name":"Org"})
                .to_string(),
        )
        .unwrap();
        assert_eq!(
            kiro_profile_arn(&path).as_deref(),
            Some("arn:aws:codewhisperer:eu-central-1:1:profile/ORG")
        );
        std::fs::write(&path, json!({"arn":"not-an-arn","name":"x"}).to_string()).unwrap();
        assert_eq!(kiro_profile_arn(&path), None);
    }
    #[test]
    fn kiro_chooses_the_newest_valid_token_instead_of_the_first_login_method() {
        let payload = json!({"kiro":{
            "profileArn":"arn:aws:codewhisperer:us-east-1:1:profile/ACCOUNT",
            "odic":{"access_token":"OLDER","expires_at":now()+300},
            "social":{"access_token":"NEWER","expires_at":now()+600,"provider":"github"}
        }});
        let rows = extract("kiro", "fixture", &doc("/k/data.sqlite3", payload)).unwrap();
        assert_eq!(rows[0].connection.selector, "/auth_kv/social");
        assert!(rows[0].secret.contains("NEWER"));
    }
    #[test]
    fn an_expired_ide_only_kiro_login_waits_for_the_ide_instead_of_running_kiro_cli() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let row = NativeConnection {
            id: "kiro".into(),
            provider: "kiro".into(),
            machine_id: "local".into(),
            selector: "/kiro-auth-token".into(),
            kind: "login".into(),
            ..Default::default()
        };
        let error = kiro_snapshot(
            &store,
            &Settings::default(),
            &row,
            &Source::default(),
            "{}",
            true,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), crate::subscriptions::kiro::RELOGIN);
        let ran: Option<String> = store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key='kiro-cli-usage-at'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(ran, None, "kiro-cli was not started");
    }
    #[cfg(unix)]
    #[test]
    fn kiro_cli_fallback_runs_once_per_window_and_its_session_is_aieyes_own() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("data")).unwrap();
        let home = dir.path().join("kiro");
        std::fs::create_dir_all(home.join("sessions/cli")).unwrap();
        std::fs::write(
            home.join("sessions/cli/old.json"),
            r#"{"session_id":"old"}"#,
        )
        .unwrap();
        let probe = dir.path().join("probe");
        store.mark_internal_session("kiro", "stale").unwrap();
        store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key='internal-session:kiro:stale'",
                [(now() - 3 * 86400).to_string()],
            )
            .unwrap();
        // Behaves like kiro-cli 2.36.0: one session file per run, usage on stdout.
        let program = dir.path().join("kiro-cli");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho run >> \"{runs}\"\nprintf '{{\"session_id\":\"probe-run\",\"cwd\":\"%s\"}}' \"$PWD\" > \"{session}\"\nprintf '{{\"session_id\":\"user-run\",\"cwd\":\"/user-work\"}}' > \"{user_session}\"\nprintf 'Estimated Usage | resets on 2026-11-19 | KIRO FREE\\nCredits (3.40 of 50 covered in plan), 6.8%%\\n'\n",
                runs = dir.path().join("runs").display(),
                session = home.join("sessions/cli/probe-run.json").display(),
                user_session = home.join("sessions/cli/user-run.json").display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let q = kiro_cli_usage(
            &store,
            &Settings::default(),
            &Source::default(),
            program.to_str().unwrap(),
            &home,
            &probe,
        )
        .unwrap();
        assert_eq!((q.meters[0].used, q.meters[0].total), (3.4, Some(50.)));
        assert_eq!(q.plan.as_deref(), Some("KIRO FREE"));
        let marks = store.internal_sessions().unwrap();
        assert!(marks.contains(&crate::sessions::internal_session_key("kiro", "probe-run")));
        assert!(marks.contains(&crate::sessions::internal_session_key(
            "kiro",
            &probe.to_string_lossy()
        )));
        assert!(!marks.contains(&crate::sessions::internal_session_key("kiro", "old")));
        assert!(
            !marks.contains(&crate::sessions::internal_session_key("kiro", "user-run")),
            "a concurrent user session is not a quota probe"
        );
        assert!(
            !marks.contains(&crate::sessions::internal_session_key("kiro", "stale")),
            "old marks are pruned"
        );
        let error = kiro_cli_usage(
            &store,
            &Settings::default(),
            &Source::default(),
            program.to_str().unwrap(),
            &home,
            &probe,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Kiro 登录已过期，稍后由 kiro-cli 重新查询"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("runs")).unwrap(),
            "run\n"
        );
    }
}
