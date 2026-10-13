//! Managed API connections share Codex history, with request-level billing evidence.
pub mod gateway;
mod scheduler;
pub(crate) mod usage;
use crate::{Engine, models::*, process, store::Store};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub id: String,
    pub source_id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub effort: String,
    pub proxy: ProxyConfig,
    pub prices: Vec<ModelPrice>,
    pub revision: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivateConnection {
    connection: Connection,
    api_key: String,
    local_token: String,
    #[serde(default)]
    proxy_environment: BTreeMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Runtime {
    port: u16,
    runner: PathBuf,
}
fn new_id() -> String {
    crate::codex_auth::files::new_id()
}
fn identifier(id: &str) -> Result<&str> {
    crate::codex_auth::files::id(id)
}
fn private_dir(root: &Path) -> Result<()> {
    crate::codex_auth::files::private_dir(root)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_value(crate::codex_auth::files::read(
        path,
    )?)?)
}
fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    private_dir(path.parent().context("API 路径无效")?)?;
    crate::codex_auth::files::atomic(path, &serde_json::to_value(value)?)
}
fn api_root(source: &Source) -> Result<PathBuf> {
    let mut home = crate::store::expand(&source.path);
    if home
        .file_name()
        .is_some_and(|v| v == "sessions" || v == "archived_sessions")
    {
        home.pop();
    }
    crate::codex_auth::files::safe(&home)?;
    ensure!(home.is_dir(), "Codex 目录不存在");
    Ok(home.join(".aieyes/api"))
}
fn has_connections(root: &Path) -> bool {
    std::fs::read_dir(root.join("connections")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|s| s == "json"))
    })
}
fn client(
    proxy: &ProxyConfig,
    env: &BTreeMap<String, String>,
) -> Result<reqwest::blocking::Client> {
    let mut builder = crate::network::client_builder(proxy)?
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800))
        .redirect(reqwest::redirect::Policy::none());
    if !matches!(proxy.mode.as_str(), "custom" | "direct") {
        for name in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            if let Some(url) = env.get(name) {
                builder = builder.proxy(reqwest::Proxy::all(url)?);
                break;
            }
        }
    }
    Ok(builder.build()?)
}
fn validate(c: &Connection) -> Result<()> {
    identifier(&c.id)?;
    identifier(&c.revision)?;
    ensure!(
        !c.name.trim().is_empty() && c.name.chars().count() <= 100,
        "请输入 API 名称（最多 100 字）"
    );
    let url = reqwest::Url::parse(&c.base_url).context("API Base URL 无效")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Base URL 应为不含凭据、查询参数的 HTTP(S) 地址"
    );
    ensure!(
        !c.model.is_empty() && c.model.len() <= 200 && !c.model.chars().any(char::is_control),
        "请输入模型 ID"
    );
    ensure!(
        [
            "", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"
        ]
        .contains(&c.effort.as_str()),
        "effort 无效"
    );
    ensure!(
        ["", "system", "custom", "direct"].contains(&c.proxy.mode.as_str()),
        "代理模式无效"
    );
    if c.proxy.mode == "custom" {
        reqwest::Proxy::all(&c.proxy.url).context("代理地址无效")?;
    }
    for p in &c.prices {
        ensure!(
            !p.id.is_empty()
                && [p.input, p.output, p.cache_read, p.cache_write]
                    .into_iter()
                    .flatten()
                    .all(|v| v.is_finite() && v >= 0.0),
            "价格必须为非负数"
        );
    }
    Ok(())
}
pub(crate) fn lock_operations(store: &Store) -> Result<crate::file_lock::FileLock> {
    let root = Path::new(store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("codex-api-operations.lock"))?;
    crate::file_lock::FileLock::try_exclusive(file).context("API 配置正在更新，请稍后重试")
}
fn location(settings: &Settings, source: &Source) -> Value {
    let host = source
        .host_id
        .as_ref()
        .and_then(|id| settings.hosts.iter().find(|h| &h.id == id));
    json!([
        source.provider,
        source.path,
        source.host_id,
        source.quota_pre_command,
        host.map(|h| json!([h.target, h.port, h.username, h.shell, h.pre_command]))
    ])
}
pub(crate) fn check_settings(
    store: &Store,
    old: Option<&Settings>,
    next: &Settings,
) -> Result<Option<crate::file_lock::FileLock>> {
    let changed = old.is_some_and(|old| {
        old.sources.iter().any(|s| {
            next.sources
                .iter()
                .find(|n| n.id == s.id)
                .is_none_or(|n| location(old, s) != location(next, n))
        })
    });
    let guard = changed.then(|| lock_operations(store)).transpose()?;
    let mut ids = std::collections::HashSet::new();
    for connection in &next.codex_apis {
        validate(connection)?;
        ensure!(ids.insert(&connection.id), "API 连接标识重复");
        let source = next
            .sources
            .iter()
            .find(|s| s.id == connection.source_id && s.provider == "codex")
            .context("API 连接仍引用此目录，请先移除 API 连接")?;
        if let Some(old) = old
            && let Some(before) = old.sources.iter().find(|s| s.id == source.id)
            && old.codex_apis.iter().any(|c| c.source_id == source.id)
        {
            ensure!(
                location(old, before) == location(next, source),
                "API 连接仍使用此设备和目录，请先移除连接再修改位置"
            );
        }
    }
    Ok(guard)
}
fn provider(c: &Connection) -> String {
    format!("aieyes_api_{}_{}", c.id, c.revision)
}
fn provider_table(root: &Path, c: &Connection, r: &Runtime) -> toml::Value {
    let mut v = toml::Table::new();
    v.insert("name".into(), toml::Value::String(c.name.clone()));
    v.insert(
        "base_url".into(),
        toml::Value::String(format!("http://127.0.0.1:{}/providers/{}/v1", r.port, c.id)),
    );
    v.insert("wire_api".into(), toml::Value::String("responses".into()));
    v.insert("supports_websockets".into(), toml::Value::Boolean(false));
    v.insert("requires_openai_auth".into(), toml::Value::Boolean(false));
    let mut auth = toml::Table::new();
    auth.insert(
        "command".into(),
        toml::Value::String(r.runner.to_string_lossy().into()),
    );
    auth.insert(
        "args".into(),
        toml::Value::Array(
            vec![
                "--codex-api-token".into(),
                root.to_string_lossy().into(),
                c.id.clone(),
            ]
            .into_iter()
            .map(toml::Value::String)
            .collect(),
        ),
    );
    v.insert("auth".into(), toml::Value::Table(auth));
    toml::Value::Table(v)
}
fn home_from_root(root: &Path) -> Result<&Path> {
    root.parent()
        .and_then(Path::parent)
        .context("API home 无效")
}
fn write_toml(path: &Path, value: &toml::Table) -> Result<()> {
    write_bytes(path, toml::to_string_pretty(value)?.as_bytes())
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::codex_auth::files::safe(path)?;
    let temp = path.with_extension(format!("{}.tmp", new_id()));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp, path)?;
    Ok(())
}
struct FileBackup(Vec<(PathBuf, Option<Vec<u8>>)>);
impl FileBackup {
    fn capture(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            crate::codex_auth::files::safe(&path)?;
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            files.push((path, bytes));
        }
        Ok(Self(files))
    }
    fn restore(&self) -> Result<()> {
        for (path, bytes) in &self.0 {
            if let Some(bytes) = bytes {
                write_bytes(path, bytes)?;
            } else if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}
fn config(root: &Path) -> Result<toml::Table> {
    let path = home_from_root(root)?.join("config.toml");
    crate::codex_auth::files::safe(&path)?;
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text).context("Codex 配置格式无效")?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(e.into()),
    }
}
fn config_revision(root: &Path) -> Result<String> {
    Ok(hash(&toml::to_string(&config(root)?)?))
}
fn launch_command(source: &Source, c: &Connection, resume: bool) -> Result<String> {
    let root = api_root(source)?;
    let home = home_from_root(&root)?.to_string_lossy();
    let binary = if source.codex_binary.trim().is_empty() {
        "codex".into()
    } else {
        crate::store::expand(&source.codex_binary)
            .to_string_lossy()
            .into_owned()
    };
    if cfg!(windows) {
        let quote = |v: &str| format!("'{}'", v.replace('\'', "''"));
        Ok(format!(
            "& {{ $env:CODEX_HOME = {}; & {} --profile {}{} }}",
            quote(&home),
            quote(&binary),
            quote(&provider(c)),
            if resume { " resume" } else { "" }
        ))
    } else {
        Ok(format!(
            "env {} {} --profile {}{}",
            process::quote(&format!("CODEX_HOME={home}")),
            process::quote(&binary),
            process::quote(&provider(c)),
            if resume { " resume" } else { "" }
        ))
    }
}
fn select_api(
    root: &Path,
    values: &mut toml::Table,
    c: &Connection,
    runtime: &Runtime,
) -> Result<()> {
    let name = provider(c);
    remove_provider_tables(values, &c.id);
    values
        .entry("model_providers")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .context("model_providers 格式无效")?
        .insert(name.clone(), provider_table(root, c, runtime));
    values.insert("model_provider".into(), toml::Value::String(name));
    values.insert("model".into(), toml::Value::String(c.model.clone()));
    if c.effort.is_empty() {
        values.remove("model_reasoning_effort");
    } else {
        values.insert(
            "model_reasoning_effort".into(),
            toml::Value::String(c.effort.clone()),
        );
    }
    Ok(())
}
fn remove_provider_tables(values: &mut toml::Table, id: &str) -> bool {
    let Some(providers) = values
        .get_mut("model_providers")
        .and_then(toml::Value::as_table_mut)
    else {
        return false;
    };
    let count = providers.len();
    let prefix = format!("aieyes_api_{id}_");
    providers.retain(|name, _| !name.starts_with(&prefix));
    count != providers.len()
}
fn write_default(root: &Path, values: &toml::Table, expected: &str) -> Result<()> {
    ensure!(
        config_revision(root)? == expected,
        "Codex 配置已变化，请刷新后重试"
    );
    let before = config(root)?;
    let path = root.join("default-history.json");
    let mut history: Vec<Value> = if path.exists() {
        read_json(&path)?
    } else {
        vec![]
    };
    let original = history.clone();
    let clock = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    };
    let auth = auth_snapshot(&before);
    history.push(json!({"startedAt":clock(),"before":auth}));
    atomic(&path, &history)?;
    let written = (|| {
        ensure!(
            config_revision(root)? == expected,
            "Codex 配置已变化，请刷新后重试"
        );
        write_toml(&home_from_root(root)?.join("config.toml"), values)
    })();
    if let Err(error) = written {
        atomic(&path, &original)?;
        return Err(error);
    }
    history.last_mut().unwrap()["completedAt"] = json!(clock());
    if let Err(error) = atomic(&path, &history) {
        // Restore only our own write; never overwrite a later external edit.
        if config_revision(root)? == hash(&toml::to_string(values)?) {
            write_toml(&home_from_root(root)?.join("config.toml"), &before)?;
        }
        atomic(&path, &original)?;
        return Err(error);
    }
    Ok(())
}
fn auth_snapshot(values: &toml::Table) -> Value {
    let mut providers = serde_json::Map::new();
    if let Some(tables) = values
        .get("model_providers")
        .and_then(toml::Value::as_table)
    {
        for (name, table) in tables {
            let mut entry = serde_json::Map::new();
            for key in ["requires_openai_auth", "env_key"] {
                if let Some(value) = table.get(key) {
                    entry.insert(key.into(), serde_json::to_value(value).unwrap());
                }
            }
            if table
                .get("base_url")
                .and_then(toml::Value::as_str)
                .and_then(|s| reqwest::Url::parse(s).ok())
                .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            {
                entry.insert("base_url".into(), json!("https://gateway.invalid"));
            }
            if table
                .get("experimental_bearer_token")
                .and_then(toml::Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                entry.insert("experimental_bearer_token".into(), json!("present"));
            }
            if table
                .get("auth")
                .and_then(|v| v.get("command"))
                .and_then(toml::Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                entry.insert("auth".into(), json!({"command":"present"}));
            }
            providers.insert(name.clone(), Value::Object(entry));
        }
    }
    json!({"model_provider":values.get("model_provider"),"model_providers":providers})
}
fn subscription_backup(values: &toml::Table) -> Value {
    let selected = values
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    let subscribed = selected == "openai"
        || values
            .get("model_providers")
            .and_then(|v| v.get(selected))
            .and_then(|v| v.get("requires_openai_auth"))
            .and_then(toml::Value::as_bool)
            == Some(true);
    if subscribed {
        json!({"model_provider":values.get("model_provider"),"model":values.get("model"),"model_reasoning_effort":values.get("model_reasoning_effort")})
    } else {
        json!({"model_provider":"openai"})
    }
}
fn install_profile(root: &Path, c: &Connection, r: &Runtime) -> Result<()> {
    let name = provider(c);
    let mut values = toml::Table::new();
    values.insert("model_provider".into(), toml::Value::String(name.clone()));
    values.insert("model".into(), toml::Value::String(c.model.clone()));
    if !c.effort.is_empty() {
        values.insert(
            "model_reasoning_effort".into(),
            toml::Value::String(c.effort.clone()),
        );
    }
    let mut providers = toml::Table::new();
    providers.insert(name.clone(), provider_table(root, c, r));
    values.insert("model_providers".into(), toml::Value::Table(providers));
    write_toml(
        &home_from_root(root)?.join(format!("{name}.config.toml")),
        &values,
    )
}
pub fn token(root: &Path, id: &str) -> Result<()> {
    identifier(id)?;
    let c: PrivateConnection = read_json(&root.join("connections").join(format!("{id}.json")))?;
    scheduler::ensure_started(root)?;
    println!("{}", c.local_token);
    Ok(())
}
fn local(source: &Source, action: &str, params: &Value) -> Result<Value> {
    let root = api_root(source)?;
    if action == "ledger" {
        return gateway::records(&root, params["cursor"].as_i64().unwrap_or(-1));
    }
    if action == "status" {
        let revision = config_revision(&root)?;
        let ready = read_json::<Runtime>(&root.join("runtime.json"))
            .is_ok_and(|r| scheduler::healthy(&root, &r));
        return Ok(
            json!({"configRevision":revision,"defaultProvider":config(&root)?.get("model_provider"),"installed":root.join("runtime.json").exists() && has_connections(&root),"ready":ready}),
        );
    }
    private_dir(&root)?;
    private_dir(&root.join("connections"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    let _guard =
        crate::file_lock::FileLock::try_exclusive(lock).context("API 设置正在更新，请稍后重试")?;
    if action == "deploy" {
        let c: Connection = serde_json::from_value(params["connection"].clone())?;
        validate(&c)?;
        let path = root.join("connections").join(format!("{}.json", c.id));
        let old: Option<PrivateConnection> = if path.exists() {
            Some(read_json(&path)?)
        } else {
            None
        };
        ensure!(
            old.as_ref()
                .is_none_or(|v| v.connection.source_id == source.id),
            "API 连接不属于所选设备目录"
        );
        ensure!(
            old.as_ref()
                .map(|v| v.connection.revision.as_str())
                .unwrap_or("")
                == params["baseRevision"].as_str().unwrap_or(""),
            "目标设备 API 设置已变化，请重新读取"
        );
        let is_default = config(&root)?
            .get("model_provider")
            .and_then(toml::Value::as_str)
            .is_some_and(|v| v.starts_with(&format!("aieyes_api_{}_", c.id)));
        if is_default {
            ensure!(
                params["configRevision"].as_str() == Some(config_revision(&root)?.as_str()),
                "Codex 默认配置已变化，请刷新后重试"
            );
        }
        let key = params["apiKey"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| old.as_ref().map(|v| v.api_key.clone()))
            .context("请输入 API Key")?;
        ensure!(!key.contains(['\n', '\r', '\0']), "API Key 无效");
        let token = old
            .as_ref()
            .map(|v| v.local_token.clone())
            .unwrap_or_else(|| format!("{}{}", new_id(), new_id()));
        let backup = FileBackup::capture([
            path.clone(),
            root.join("runtime.json"),
            home_from_root(&root)?.join(format!("{}.config.toml", provider(&c))),
        ])?;
        let registration = scheduler::snapshot(&root)?;
        let private = PrivateConnection {
            connection: c.clone(),
            api_key: key,
            local_token: token,
            proxy_environment: std::env::vars()
                .filter(|(k, _)| {
                    [
                        "HTTP_PROXY",
                        "HTTPS_PROXY",
                        "ALL_PROXY",
                        "http_proxy",
                        "https_proxy",
                        "all_proxy",
                    ]
                    .contains(&k.as_str())
                })
                .collect(),
        };
        let result = (|| {
            let runtime = scheduler::install_runtime(&root)?;
            atomic(&path, &private)?;
            install_profile(&root, &c, &runtime)?;
            scheduler::register(&root, &runtime)?;
            scheduler::ensure_started(&root)?;
            if is_default {
                let mut values = config(&root)?;
                select_api(&root, &mut values, &c, &runtime)?;
                write_default(&root, &values, params["configRevision"].as_str().unwrap())?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            backup
                .restore()
                .context("部署失败且文件回滚未完成，请重新部署")?;
            scheduler::restore(&root, &registration)
                .context("部署失败且服务回滚未完成，请重新部署")?;
            return Err(e);
        }
        return Ok(
            json!({"launchCommand":launch_command(source,&c,false)?,"resumeCommand":launch_command(source,&c,true)?,"configRevision":config_revision(&root)?}),
        );
    }
    if action == "default" {
        ensure!(
            params["configRevision"].as_str() == Some(config_revision(&root)?.as_str()),
            "Codex 配置已变化，请刷新后重试"
        );
        let mut values = config(&root)?;
        let backup = root.join("default-backup.json");
        let keys = ["model_provider", "model", "model_reasoning_effort"];
        if params["id"] == "subscription" {
            if backup.exists() {
                let saved: Value = read_json(&backup)?;
                for key in keys {
                    if let Some(value) = saved.get(key).filter(|v| !v.is_null()) {
                        values.insert(
                            key.into(),
                            serde_json::from_value::<toml::Value>(value.clone())?,
                        );
                    } else {
                        values.remove(key);
                    }
                }
            } else {
                values.insert(
                    "model_provider".into(),
                    toml::Value::String("openai".into()),
                );
            }
        } else {
            let id = identifier(params["id"].as_str().context("API 标识缺失")?)?;
            let c: PrivateConnection =
                read_json(&root.join("connections").join(format!("{id}.json")))?;
            ensure!(
                c.connection.source_id == source.id,
                "API 连接不属于所选设备目录"
            );
            ensure!(
                params["revision"].as_str() == Some(c.connection.revision.as_str()),
                "目标设备 API 设置已变化，请重新读取"
            );
            let r: Runtime = read_json(&root.join("runtime.json"))?;
            if !values
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| v.starts_with("aieyes_api_"))
            {
                atomic(&backup, &subscription_backup(&values))?;
            }
            select_api(&root, &mut values, &c.connection, &r)?;
        }
        write_default(&root, &values, params["configRevision"].as_str().unwrap())?;
        return Ok(json!({"configRevision":config_revision(&root)?}));
    }
    let id = identifier(params["id"].as_str().context("API 标识缺失")?)?;
    let path = root.join("connections").join(format!("{id}.json"));
    let c: PrivateConnection = read_json(&path)?;
    ensure!(
        c.connection.source_id == source.id,
        "API 连接不属于所选设备目录"
    );
    if action == "command" {
        return Ok(
            json!({"launchCommand":launch_command(source,&c.connection,false)?,"resumeCommand":launch_command(source,&c.connection,true)?}),
        );
    }
    if action == "probe" {
        let result = client(&c.connection.proxy, &c.proxy_environment)?
            .get(format!(
                "{}/models",
                c.connection.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&c.api_key)
            .timeout(Duration::from_secs(20))
            .send();
        return match result {
            Ok(response) => Ok(
                json!({"reachable":response.status().is_success(),"httpStatus":response.status().as_u16(),"message":if response.status().is_success(){"API 认证与模型列表接口可用；尚未发起计费调用"}else{"模型列表查询未成功；部分接口不支持此操作"}}),
            ),
            Err(_) => anyhow::bail!("API 连接失败；请检查地址、代理及证书"),
        };
    }
    if action == "remove" {
        ensure!(
            params["revision"].as_str() == Some(c.connection.revision.as_str()),
            "目标设备 API 设置已变化，请重新读取"
        );
        let mut values = config(&root)?;
        let revision = hash(&toml::to_string(&values)?);
        ensure!(
            !values
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| v.starts_with(&format!("aieyes_api_{id}_"))),
            "请先切回订阅或其他默认 API，再移除此连接"
        );
        let config_changed = remove_provider_tables(&mut values, id);
        let home = home_from_root(&root)?;
        let mut files = vec![path];
        for entry in std::fs::read_dir(home)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("aieyes_api_{id}_")) && name.ends_with(".config.toml") {
                files.push(entry.path());
            }
        }
        let backup = FileBackup::capture(files.clone())?;
        let registration = scheduler::snapshot(&root)?;
        let last = std::fs::read_dir(root.join("connections"))?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
            .count()
            == 1;
        let result = (|| -> Result<()> {
            if last {
                scheduler::unregister(&root)?;
            }
            for path in files {
                std::fs::remove_file(path)?;
            }
            if last {
                gateway::interrupt_pending(&root, "connection-removed")?;
            }
            if config_changed {
                write_default(&root, &values, &revision)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            backup.restore()?;
            scheduler::restore(&root, &registration)?;
            return Err(error);
        }
        return Ok(json!({"removed":true}));
    }
    anyhow::bail!("未知 API 操作")
}
fn target(settings: &Settings, source: &Source, action: &str, params: &Value) -> Result<Value> {
    let Some(host_id) = &source.host_id else {
        return local(source, action, params);
    };
    let mut host = settings
        .hosts
        .iter()
        .find(|h| &h.id == host_id && h.enabled)
        .context("服务器不可用")?
        .clone();
    if !source.quota_pre_command.is_empty() {
        host.pre_command = source.quota_pre_command.clone();
    }
    let request = json!({"action":action,"params":params,"sourceId":source.id,"home":source.path,"binary":source.codex_binary});
    let sources = json!({"__init__":include_str!("../../../../scripts/vendor/tomli/__init__.py"),"_types":include_str!("../../../../scripts/vendor/tomli/_types.py"),"_re":include_str!("../../../../scripts/vendor/tomli/_re.py"),"_parser":include_str!("../../../../scripts/vendor/tomli/_parser.py")});
    let script = include_str!("../../../../scripts/remote_codex_api.py");
    // All secrets and scripts travel through stdin, never SSH argv.
    let payload = json!({"request":request,"script":script,"tomli":sources});
    let loader = "import json,sys; p=json.load(sys.stdin); scope={'__name__':'aieyes_api','TOMLI':p['tomli']}; exec(compile(p['script'],'<aieyes-api>','exec'),scope); print(json.dumps(scope['dispatch'](p['request'],p['script'],p['tomli']),ensure_ascii=False))";
    let command = crate::ssh::account_command(
        &host,
        &format!("exec python3 -c {}", process::quote(loader)),
    )?;
    let output = process::run(
        command,
        payload.to_string().into_bytes(),
        Duration::from_secs(60),
    )?;
    let value: Value = serde_json::from_slice(
        output
            .split(|b| *b == b'\n')
            .rev()
            .find(|s| !s.is_empty())
            .context("API 助手没有响应")?,
    )?;
    if let Some(error) = value["error"].as_str() {
        anyhow::bail!("{error}");
    }
    Ok(value)
}
impl Engine {
    pub(crate) fn codex_api_call(&mut self, method: &str, p: Value) -> Result<Value> {
        let action = method.strip_prefix("codexApi.").context("API 操作无效")?;
        let mutation = matches!(action, "save" | "deploy" | "remove" | "default");
        let _guard = mutation.then(|| lock_operations(&self.store)).transpose()?;
        let settings = self.store.settings()?;
        if action == "list" {
            return Ok(json!({"connections":settings.codex_apis}));
        }
        let source_id = p["sourceId"]
            .as_str()
            .or_else(|| p["connection"]["sourceId"].as_str())
            .context("请选择 Codex 数据源")?;
        let source = settings
            .sources
            .iter()
            .find(|s| s.id == source_id && s.provider == "codex")
            .context("Codex 数据源不存在")?;
        if action == "save" || action == "deploy" {
            let mut c: Connection = serde_json::from_value(p["connection"].clone())?;
            if c.id.is_empty() {
                c.id = new_id();
            }
            let old = settings.codex_apis.iter().find(|v| v.id == c.id);
            if let Some(old) = old {
                ensure!(old.source_id == source.id, "更换设备请新建 API 连接");
                ensure!(c.revision == old.revision, "API 设置已变化，请刷新后重试");
            }
            c.revision = new_id()[..12].into();
            c.source_id = source.id.clone();
            validate(&c)?;
            let result = target(
                &settings,
                source,
                "deploy",
                &json!({"connection":c,"apiKey":p["apiKey"],"baseRevision":old.map(|v|v.revision.as_str()).unwrap_or(""),"configRevision":p["configRevision"]}),
            )?;
            let tx = self.store.write_transaction()?;
            let mut next = self.store.settings()?;
            ensure!(
                next.codex_apis
                    .iter()
                    .find(|v| v.id == c.id)
                    .map(|v| &v.revision)
                    == old.map(|v| &v.revision),
                "API 设置已并发变化，请重新读取设备配置"
            );
            next.codex_apis.retain(|v| v.id != c.id);
            next.codex_apis.push(c.clone());
            usage::retain_ledger(&self.store, source)?;
            self.store.save_settings(&next)?;
            tx.commit()?;
            return Ok(
                json!({"settings":self.store.settings()?,"connection":c,"deployment":result}),
            );
        }
        if matches!(action, "default" | "remove" | "command" | "probe") && p["id"] != "subscription"
        {
            let connection = settings
                .codex_apis
                .iter()
                .find(|c| Some(c.id.as_str()) == p["id"].as_str())
                .context("API 连接已移除，请刷新后重试")?;
            ensure!(connection.source_id == source.id, "API 连接不属于所选设备");
            if matches!(action, "default" | "remove") {
                ensure!(
                    p["revision"].as_str() == Some(connection.revision.as_str()),
                    "API 设置已变化，请刷新后重试"
                );
            }
        }
        let result = target(&settings, source, action, &p)?;
        if action == "remove" {
            let tx = self.store.write_transaction()?;
            let mut next = self.store.settings()?;
            next.codex_apis
                .retain(|c| Some(c.id.as_str()) != p["id"].as_str());
            usage::retain_ledger(&self.store, source)?;
            self.store.save_settings(&next)?;
            tx.commit()?;
            let _ = usage::sync(&self.store, &settings, source);
            return Ok(json!({"settings":self.store.settings()?}));
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
