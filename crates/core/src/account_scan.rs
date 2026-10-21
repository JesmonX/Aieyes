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
}
#[derive(Clone)]
struct Candidate {
    connection: NativeConnection,
    fingerprint: String,
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
            vec!["v2/provider_config.json", "auth.json", "credentials.json"],
        ),
        "pi" => ("PI_CODING_AGENT_DIR", vec!["auth.json", "models.json"]),
        "omp" => (
            "PI_CODING_AGENT_DIR",
            vec!["auth.json", "models.yml", "models.yaml"],
        ),
        "deepseek-harness" => (
            "DSH_HOME",
            vec!["cordis.patch.yml", "profiles/*/cordis.patch.yml"],
        ),
        "cursor" => ("CURSOR_CONFIG_DIR", vec!["cli-config.json", "auth.json"]),
        _ => ("", vec![]),
    }
}
fn default_root(provider: &str, remote: bool) -> Result<String> {
    if provider == "opencode" && !remote {
        return Ok(std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".local/share"))
            .join("opencode")
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
        "zcode" => vec![
            std::env::var_os("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| {
                    std::env::var_os("ZCODE_DATA_BASE_DIR")
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute())
                        .unwrap_or(home)
                        .join(".zcode/v2/provider_config.json")
                })
                .to_string_lossy()
                .into(),
        ],
        _ => vec![],
    }
}
fn read_file(root: &str, path: &Path) -> Result<Document> {
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
    Ok(Document {
        root: root.into(),
        path: path.to_string_lossy().into(),
        bytes,
    })
}
fn load(provider: &str, machine: &str, settings: &Settings) -> Result<Loaded> {
    load_inner(provider, machine, settings, None)
}
fn load_inner(
    provider: &str,
    machine: &str,
    settings: &Settings,
    reference: Option<&NativeConnection>,
) -> Result<Loaded> {
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
    let (env, files) = spec(provider);
    if !remote
        && reference.is_none()
        && !env.is_empty()
        && let Some(value) = std::env::var_os(env).filter(|s| !s.is_empty())
    {
        let mut path = PathBuf::from(value);
        ensure!(path.is_absolute(), "原生目录环境变量应为绝对路径");
        if provider == "opencode" {
            path.push("opencode");
        }
        roots.push(path.to_string_lossy().into());
    }
    if remote {
        let host = settings
            .hosts
            .iter()
            .find(|h| h.id == machine)
            .context("机器已移除")?;
        let mut request = json!({"provider":provider,"roots":roots,"env":env,"patterns":files,"primary":default_root(provider,true)?});
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
        return Ok(loaded);
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
    let mut loaded = Loaded {
        roots: roots.clone(),
        ..Default::default()
    };
    let mut paths = BTreeSet::new();
    for root in &roots {
        for pattern in files.iter().filter(|_| reference.is_none()) {
            for path in patterns(Path::new(root), pattern) {
                paths.insert((root.clone(), path));
            }
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
    for (root, path) in paths.into_iter().take(MAX_FILES) {
        if !path.exists() {
            continue;
        }
        match read_file(&root, &path) {
            Ok(doc) => loaded.documents.push(doc),
            Err(error) => loaded.errors.push(format!("{}：{error}", path.display())),
        }
    }
    #[cfg(target_os = "macos")]
    if provider == "claude"
        && roots
            .iter()
            .any(|r| Path::new(r).is_dir() && Path::new(r) == crate::store::home().join(".claude"))
        && !loaded
            .documents
            .iter()
            .any(|d| d.path.ends_with(".credentials.json"))
    {
        let mut command = std::process::Command::new("/usr/bin/security");
        command.args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ]);
        if let Ok(bytes) = crate::process::run(command, vec![], Duration::from_secs(5))
            && bytes.len() <= LIMIT as usize
        {
            loaded.documents.push(Document {
                root: crate::store::home()
                    .join(".claude")
                    .to_string_lossy()
                    .into(),
                path: "keychain:Claude Code-credentials".into(),
                bytes,
            });
        }
    }
    Ok(loaded)
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
    let value = if doc.path.ends_with(".toml") {
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
            status: "pending".into(),
            note: "已发现原生配置，登录有效性待验证".into(),
            checked_at: now(),
            ..Default::default()
        },
        fingerprint: hash(secret),
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
        "原生 API 配置；凭据由客户端解析，待验证"
    } else {
        "原生 API 连接；Key 保留在原配置，待验证"
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
        }
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
    if expires.is_some_and(|t| {
        if t > 10_000_000_000 {
            t / 1000 <= now()
        } else {
            t <= now()
        }
    }) {
        row.connection.status = "expired".into();
        row.connection.note = "凭据已过期，请在原生客户端登录后重新扫描".into();
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
fn extract(provider: &str, machine: &str, doc: &Document) -> Result<Vec<Candidate>> {
    let v = parse(doc)?;
    let mut rows = vec![];
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
                        login(&mut rows, provider, machine, doc, &selector, value);
                    } else if value["type"] == "api" {
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
                let key = options["apiKey"]
                    .as_str()
                    .or(options["api_key"].as_str())
                    .unwrap_or("");
                let mut models = model_names(route);
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
        let initial = self.store.settings()?;
        let scope = text(&params["scope"]);
        ensure!(
            matches!(scope, "machine" | "agent"),
            "请选择机器扫描或 Agent 扫描"
        );
        let mut targets = vec![];
        if scope == "machine" {
            let machine = params["machineId"].as_str().context("请选择机器")?;
            ensure!(
                machine_enabled(&initial, machine),
                "请先开启此机器的 Agent 采集"
            );
            for p in contracts::PROVIDERS
                .iter()
                .filter(|p| p.id != "custom" && p.id != "deepseek" && agent_enabled(&initial, p.id))
            {
                targets.push((p.id.to_string(), machine.to_string()));
            }
        } else {
            let provider = params["provider"].as_str().context("请选择 Agent")?;
            let agent = contracts::provider(provider)
                .context("不支持的 Agent")?
                .agent_id;
            ensure!(agent_enabled(&initial, provider), "请先启用此 Agent");
            for p in contracts::PROVIDERS
                .iter()
                .filter(|p| p.agent_id == agent && p.id != "deepseek" && p.id != "custom")
            {
                for a in initial.agents.iter().filter(|a| a.provider == p.id) {
                    for machine in &a.machine_ids {
                        if machine_enabled(&initial, machine) {
                            targets.push((p.id.to_string(), machine.clone()));
                        }
                    }
                }
            }
        }
        ensure!(
            !targets.is_empty(),
            "没有已启用的扫描目标，请先选择机器并开启 Agent 采集"
        );
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
                results.push(json!({"provider":provider,"machineId":machine,"status":"skipped","message":"此机器的 SSH 连接已失败，请修复连接后重新扫描"}));
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
            for error in &loaded.errors {
                results.push(json!({"provider":provider,"machineId":machine,"status":"failed","message":error}));
            }
            for doc in &loaded.documents {
                match extract(&provider,&machine,doc){Ok(mut rows)=>candidates.append(&mut rows),Err(error)=>results.push(json!({"provider":provider,"machineId":machine,"status":"failed","configPath":doc.path,"message":error.to_string()}))}
            }
            // A native identity request has no model prompt and never copies a key.
            enrich_identities(&provider, &loaded.documents, &mut candidates);
            if matches!(provider.as_str(), "qoder" | "antigravity") {
                for root in loaded
                    .roots
                    .iter()
                    .filter(|r| machine != "local" || Path::new(r).is_dir())
                {
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
                || (scope == "agent" && !selected(&latest, &provider, &machine))
            {
                results.push(json!({"provider":provider,"machineId":machine,"status":"skipped","message":"扫描期间机器或 Agent 配置已变化，请重新扫描"}));
                continue;
            }
            for old in latest.native_connections.iter_mut().filter(|r| {
                r.provider == provider && r.machine_id == machine && !found.contains(&r.id)
            }) {
                old.status = "unavailable".into();
                old.note = "原生配置已移除、变化或暂不可读，请重新扫描".into();
                old.checked_at = now();
            }
            let mut seen = BTreeSet::new();
            for candidate in candidates {
                let mut row = candidate.connection;
                row.machine_fingerprint = hash(&host_signature(&latest, &machine).to_string());
                if !seen.insert(row.id.clone()) {
                    continue;
                }
                let old = latest.native_connections.iter().find(|r| r.id == row.id);
                let existing = old.is_some();
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
                                contracts::provider(&provider).unwrap().name,
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
                results.push(json!({"provider":provider,"machineId":machine,"status":"empty","message":if provider=="cursor"{"未找到可识别的 CLI 登录；GUI 私有凭据需在客户端管理"}else{"未找到可识别的原生账户或 API 配置"}}));
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
            self.store.save_auth_settings(&latest)?;
            tx.commit()?;
        }
        let settings = self.store.settings()?;
        Ok(
            json!({"settings":settings,"addedAccounts":added_accounts,"addedConnections":added_connections,"results":results}),
        )
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
            self.store.db.execute(
                "DELETE FROM kv WHERE key=?1",
                [format!("native-fingerprint:{id}")],
            )?;
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
        && row.status != "unavailable"
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
        Some(json!({"accountKey":key,"sourceId":format!("native-{}",r.id),"nativeConnectionId":r.id,"machineId":r.machine_id,"current":if r.status=="observed"&&available(settings,r){Some(true)}else{None},"credential":if r.machine_fingerprint==hash(&host_signature(settings,&r.machine_id).to_string()){Some(r.has_credential&&r.status!="unavailable")}else{None},"checkedAt":r.checked_at,"note":r.note,"error":if matches!(r.status.as_str(),"expired"|"unavailable"){Some(&r.note)}else{None}}))
    }).collect()
}
pub(crate) fn quota_sources(settings: &Settings, account: &Account) -> Vec<Source> {
    let mut sources = settings
        .native_connections
        .iter()
        .filter(|r| {
            r.account_key.as_deref() == Some(&accounts::key(account))
                && r.kind == "login"
                && available(settings, r)
                && r.status != "expired"
                && matches!(
                    r.provider.as_str(),
                    "codex" | "claude" | "antigravity" | "qoder"
                )
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
            source.provider = r.provider.clone();
            source.name = account.name.clone();
            source.account_id = account.id.clone();
            source.path = r.root.clone();
            source.host_id = (r.machine_id != "local").then(|| r.machine_id.clone());
            source.codex_home_id = None;
            source.quota_command.clear();
            source.subscription =
                (r.provider == "qoder").then(|| crate::subscriptions::SubscriptionConfig {
                    mode: "cli".into(),
                    native_config_dir: r.root.clone(),
                    ..Default::default()
                });
            source
        })
        .collect::<Vec<_>>();
    sources.sort_by_key(|s| {
        let preferred = account.quota_source_id.as_ref().is_some_and(|id| {
            *id == s.id
                || settings
                    .native_connections
                    .iter()
                    .any(|r| s.id == format!("native-{}", r.id) && r.source_id.as_ref() == Some(id))
        });
        (!preferred, s.host_id.is_some())
    });
    sources
}

pub(crate) fn has_quota_target(settings: &Settings, account: &Account) -> bool {
    !quota_sources(settings, account).is_empty()
}
pub(crate) fn read_native_quota(
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
            "原生登录账户已变化，请重新扫描"
        );
    } else {
        let candidate = reload_candidate(settings, row)?;
        ensure!(
            candidate.connection.identity_key == row.identity_key,
            "原生登录账户已变化，请重新扫描"
        );
        let expected: String = store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key=?1",
                [format!("native-fingerprint:{}", row.id)],
                |r| r.get(0),
            )
            .context("请重新扫描此凭据")?;
        ensure!(
            candidate.connection.identity_key.is_some() || candidate.fingerprint == expected,
            "凭据已变化，请重新扫描"
        );
        ensure!(
            candidate.connection.status != "expired",
            "原生登录已过期，请重新登录后扫描"
        );
    }
    let snapshot = crate::quota::read(source, settings)?;
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
        let current = reload_candidate(settings, row)?;
        ensure!(
            current.connection.identity_key == row.identity_key,
            "查询期间原生账户身份已变化"
        );
    }
    Ok(snapshot)
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
            Some((
                d.root.clone(),
                format!(
                    "claude:{}",
                    hash(&json!([a["accountUuid"].as_str()?, a["organizationUuid"]]).to_string())
                ),
                public_text(text(&a["emailAddress"])),
            ))
        })
        .collect();
    for candidate in candidates
        .iter_mut()
        .filter(|c| c.connection.kind == "login")
    {
        if let Some((_, identity, email)) = identities
            .iter()
            .find(|(root, _, _)| *root == candidate.connection.root)
        {
            candidate.connection.identity_key = Some(identity.clone());
            candidate.connection.email = email.clone();
        }
    }
}
fn reload_candidate(settings: &Settings, row: &NativeConnection) -> Result<Candidate> {
    let loaded = load_inner(&row.provider, &row.machine_id, settings, Some(row))?;
    let mut candidates = loaded
        .documents
        .iter()
        .find(|d| d.path == row.config_path)
        .map(|d| extract(&row.provider, &row.machine_id, d))
        .transpose()?
        .unwrap_or_default();
    enrich_identities(&row.provider, &loaded.documents, &mut candidates);
    candidates
        .into_iter()
        .find(|c| c.connection.id == row.id)
        .context("原生凭据已移除，请重新扫描")
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

impl Engine {
    pub(crate) fn refresh_native_connections(&mut self, params: &Value) -> Result<()> {
        let settings = self.store.settings()?;
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
            if params["dueOnly"] == true && (0..300).contains(&(now() - row.checked_at)) {
                continue;
            }
            crate::operations::check()?;
            let mut next = row.clone();
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
                identity
                    .map(|i| {
                        ensure!(
                            Some(i.key) == row.identity_key,
                            "原生登录身份已变化，请重新扫描"
                        );
                        Ok(())
                    })
                    .and_then(|r| r)
            } else {
                reload_candidate(&settings, row).and_then(|c| {
                    ensure!(
                        c.connection.identity_key == row.identity_key,
                        "原生登录身份已变化，请重新扫描"
                    );
                    if row.identity_key.is_none() {
                        let expected: String = self.store.db.query_row(
                            "SELECT value FROM kv WHERE key=?1",
                            [format!("native-fingerprint:{}", row.id)],
                            |r| r.get(0),
                        )?;
                        ensure!(expected == c.fingerprint, "原生凭据已变化，请重新扫描");
                    }
                    next.status = c.connection.status;
                    next.note = c.connection.note;
                    Ok(())
                })
            };
            if let Err(error) = observation {
                next.status = "unavailable".into();
                next.note = error.to_string();
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
        Ok(())
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
            status: "pending".into(),
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
    }
    fn doc(path: &str, value: Value) -> Document {
        Document {
            root: "/native".into(),
            path: path.into(),
            bytes: serde_json::to_vec(&value).unwrap(),
        }
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
        assert_eq!(rows[0].connection.status, "expired");
        assert!(rows[0].connection.identity_key.is_none());
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
}
