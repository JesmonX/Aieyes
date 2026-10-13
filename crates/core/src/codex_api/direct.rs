//! Stored API credentials and native Codex provider selection. No metering service is deployed.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    connection: Connection,
    api_key: String,
}

fn root(home: &Path) -> PathBuf {
    home.join(".aieyes/api")
}
fn index(root: &Path, id: &str) -> Result<PathBuf> {
    Ok(root
        .join("direct/current")
        .join(format!("{}.json", identifier(id)?)))
}
fn key_file(root: &Path, id: &str, revision: &str) -> Result<PathBuf> {
    Ok(root.join("direct/keys").join(format!(
        "{}-{}.json",
        identifier(id)?,
        identifier(revision)?
    )))
}
fn latest(root: &Path, id: &str) -> Result<Saved> {
    let path = index(root, id)?;
    if path.exists() {
        let c: Connection = read_json(&path)?;
        read_json(&key_file(root, id, &c.revision)?)
    } else {
        let legacy: PrivateConnection = read_json(
            &root
                .join("connections")
                .join(format!("{}.json", identifier(id)?)),
        )?;
        Ok(Saved {
            connection: legacy.connection,
            api_key: legacy.api_key,
        })
    }
}
fn optional_latest(root: &Path, id: &str) -> Result<Option<Saved>> {
    let pointer = index(root, id)?;
    let legacy = root
        .join("connections")
        .join(format!("{}.json", identifier(id)?));
    crate::codex_auth::files::safe(&pointer)?;
    crate::codex_auth::files::safe(&legacy)?;
    if pointer.exists() || legacy.exists() {
        latest(root, id).map(Some)
    } else {
        Ok(None)
    }
}
fn simple(mut c: Connection) -> Connection {
    c.model.clear();
    c.effort.clear();
    c.proxy = ProxyConfig::default();
    c.prices.clear();
    c
}
fn store(root: &Path, saved: &Saved) -> Result<()> {
    let key = key_file(root, &saved.connection.id, &saved.connection.revision)?;
    let pointer = index(root, &saved.connection.id)?;
    private_dir(key.parent().unwrap())?;
    private_dir(pointer.parent().unwrap())?;
    if key.exists() {
        let old: Saved = read_json(&key)?;
        ensure!(
            serde_json::to_value(&old)? == serde_json::to_value(saved)?,
            "API 修订已存在且内容不同，请重新保存"
        );
    } else {
        atomic(&key, saved)?;
    }
    atomic(&pointer, &saved.connection)
}
fn helper(root: &Path) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let binary = crate::wakeups::runtime_binary()?;
    let bytes = std::fs::read(binary)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let folder = root.join("direct");
    private_dir(&folder)?;
    let path = folder.join(format!(
        "key-reader-{}{}",
        &digest[..16],
        if cfg!(windows) { ".exe" } else { "" }
    ));
    if !path.exists() {
        write_bytes(&path, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(path)
}
fn table(root: &Path, c: &Connection, helper: &Path) -> toml::Value {
    let mut v = toml::Table::new();
    v.insert("name".into(), c.name.clone().into());
    v.insert("base_url".into(), c.base_url.clone().into());
    v.insert("wire_api".into(), "responses".into());
    v.insert("requires_openai_auth".into(), false.into());
    v.insert("supports_websockets".into(), false.into());
    let mut auth = toml::Table::new();
    auth.insert(
        "command".into(),
        helper.to_string_lossy().to_string().into(),
    );
    auth.insert(
        "args".into(),
        toml::Value::Array(
            [
                "--codex-api-key".into(),
                root.to_string_lossy().to_string(),
                c.id.clone(),
                c.revision.clone(),
            ]
            .into_iter()
            .map(toml::Value::String)
            .collect(),
        ),
    );
    v.insert("auth".into(), toml::Value::Table(auth));
    toml::Value::Table(v)
}

pub(crate) fn selected(home: &Path) -> Result<Value> {
    let values = config(&root(home))?;
    let name = values
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    let details = values.get("model_providers").and_then(|p| p.get(name));
    let account = name == "openai"
        || details
            .and_then(|p| p.get("requires_openai_auth"))
            .and_then(toml::Value::as_bool)
            == Some(true);
    if account {
        let apikey = crate::codex_auth::files::read(&home.join("auth.json"))
            .ok()
            .is_some_and(|a| {
                a["auth_mode"] == "apikey"
                    || a["OPENAI_API_KEY"].as_str().is_some_and(|s| !s.is_empty())
            });
        return Ok(json!({"kind":if apikey{"api"}else{"account"},"provider":name}));
    }
    let managed = name
        .strip_prefix("aieyes_api_")
        .and_then(|s| s.rsplit_once('_'));
    Ok(
        json!({"kind":"api","provider":name,"connectionId":managed.map(|v|v.0),"revision":managed.map(|v|v.1),"name":details.and_then(|p|p.get("name")).and_then(toml::Value::as_str)}),
    )
}

pub(crate) fn prepare(home: &Path, target: &Value) -> Result<Value> {
    let root = root(home);
    let mut result = target.clone();
    if target["kind"] == "api" {
        let id = target["connectionId"].as_str().context("请选择 API 连接")?;
        let mut saved = latest(&root, id)?;
        ensure!(
            target["revision"].as_str() == Some(saved.connection.revision.as_str()),
            "API 设置已变化，请重新检查"
        );
        if let Some(source) = target["sourceId"].as_str() {
            ensure!(
                saved.connection.source_id == source,
                "API 连接不属于所选目录"
            );
        }
        ensure!(
            !saved.api_key.is_empty() && !saved.api_key.contains(['\r', '\n', '\0']),
            "API Key 无效"
        );
        validate(&saved.connection)?;
        ensure!(
            config(&root)?
                .get("forced_login_method")
                .and_then(toml::Value::as_str)
                .is_none_or(|v| v == "api"),
            "此 home 限制了登录方式"
        );
        saved.connection = simple(saved.connection);
        store(&root, &saved)?;
        let runner = helper(&root)?;
        result["helper"] = json!(runner);
        result["name"] = json!(saved.connection.name);
    } else {
        ensure!(target["kind"] == "account", "切换目标无效");
    }
    result["configRevision"] = json!(config_revision(&root)?);
    Ok(result)
}

/// Called only under the account switch lease, after the approved process preflight.
pub(crate) fn select(home: &Path, target: &Value) -> Result<String> {
    let root = root(home);
    private_dir(&root)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    let _guard =
        crate::file_lock::FileLock::try_exclusive(file).context("API 设置正在更新，请重新检查")?;
    let expected = target["configRevision"]
        .as_str()
        .context("切换配置基线缺失")?;
    ensure!(
        config_revision(&root)? == expected,
        "Codex 配置已变化，请重新检查"
    );
    let mut values = config(&root)?;
    let backup = root.join("account-selection.json");
    if target["kind"] == "api" {
        let saved = latest(
            &root,
            target["connectionId"].as_str().context("API 标识缺失")?,
        )?;
        ensure!(
            target["revision"].as_str() == Some(saved.connection.revision.as_str()),
            "API 设置已变化，请重新检查"
        );
        let runner = PathBuf::from(target["helper"].as_str().context("API 读钥匙助手缺失")?);
        ensure!(runner.is_file(), "API 读钥匙助手缺失，请重新检查");
        if selected(home)?["kind"] == "account" {
            atomic(
                &backup,
                &json!({"model_provider":values.get("model_provider")}),
            )?;
        }
        let name = provider(&saved.connection);
        values
            .entry("model_providers")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .context("model_providers 格式无效")?
            .insert(name.clone(), table(&root, &saved.connection, &runner));
        values.insert("model_provider".into(), name.into());
    } else {
        let saved: Value = read_json(&backup)
            .or_else(|_| read_json(&root.join("default-backup.json")))
            .unwrap_or(json!({}));
        let provider = saved["model_provider"].as_str().unwrap_or("openai");
        let subscribed = provider == "openai"
            || values
                .get("model_providers")
                .and_then(|p| p.get(provider))
                .and_then(|p| p.get("requires_openai_auth"))
                .and_then(toml::Value::as_bool)
                == Some(true);
        values.insert(
            "model_provider".into(),
            if subscribed { provider } else { "openai" }.into(),
        );
    }
    // Model, reasoning, proxy/network settings and auth.json are deliberately inherited.
    write_default(&root, &values, expected)?;
    config_revision(&root)
}

pub(crate) fn restore(home: &Path, values: &toml::Value, written: &str) -> Result<()> {
    write_default(
        &root(home),
        values.as_table().context("原配置格式无效")?,
        written,
    )
}

pub fn key(root: &Path, id: &str, revision: &str) -> Result<()> {
    let saved: Saved = read_json(&key_file(root, id, revision)?)?;
    ensure!(
        !saved.api_key.is_empty() && !saved.api_key.contains(['\r', '\n', '\0']),
        "API Key 无效"
    );
    println!("{}", saved.api_key);
    Ok(())
}

pub(super) fn local(source: &Source, action: &str, params: &Value) -> Result<Value> {
    let root = api_root(source)?;
    let home = home_from_root(&root)?;
    if action == "status" {
        return Ok(
            json!({"configRevision":config_revision(&root)?,"defaultProvider":config(&root)?.get("model_provider"),"selected":selected(home)?}),
        );
    }
    ensure!(
        action != "default" && action != "command",
        "请通过账号切换检查后切换 API"
    );
    private_dir(&root)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    let _guard =
        crate::file_lock::FileLock::try_exclusive(file).context("API 设置正在更新，请稍后重试")?;
    if action == "deploy" {
        let c = simple(serde_json::from_value(params["connection"].clone())?);
        validate(&c)?;
        let old = optional_latest(&root, &c.id)?;
        ensure!(
            old.as_ref()
                .map(|v| v.connection.revision.as_str())
                .unwrap_or("")
                == params["baseRevision"].as_str().unwrap_or(""),
            "目标设备 API 设置已变化，请重新读取"
        );
        ensure!(
            old.as_ref()
                .is_none_or(|v| v.connection.source_id == source.id),
            "API 连接不属于所选设备目录"
        );
        let key = params["apiKey"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| old.map(|v| v.api_key))
            .context("请输入 API Key")?;
        ensure!(!key.contains(['\r', '\n', '\0']), "API Key 无效");
        store(
            &root,
            &Saved {
                connection: c,
                api_key: key,
            },
        )?;
        return Ok(json!({"saved":true,"configRevision":config_revision(&root)?}));
    }
    let id = identifier(params["id"].as_str().context("API 标识缺失")?)?;
    let saved = latest(&root, id)?;
    ensure!(
        saved.connection.source_id == source.id,
        "API 连接不属于所选设备目录"
    );
    if action == "probe" {
        let result = client(&ProxyConfig::default(), &BTreeMap::new())?
            .get(format!(
                "{}/models",
                saved.connection.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&saved.api_key)
            .timeout(Duration::from_secs(20))
            .send();
        return match result {
            Ok(r) => Ok(
                json!({"reachable":r.status().is_success(),"httpStatus":r.status().as_u16(),"message":if r.status().is_success(){"API 认证可用"}else{"模型列表查询未成功；部分服务不支持此接口"}}),
            ),
            Err(_) => anyhow::bail!("API 连接失败，请检查服务地址和本机网络"),
        };
    }
    if action == "remove" {
        ensure!(
            params["revision"].as_str() == Some(saved.connection.revision.as_str()),
            "API 设置已变化，请刷新后重试"
        );
        ensure!(
            selected(home)?["connectionId"] != id,
            "请先切换到账号或其他 API，再移除此连接"
        );
        ensure!(
            crate::codex_auth::processes::list(home)?.is_empty(),
            "此目录仍有 Codex 会话，退出后再移除 API 连接"
        );
        let legacy = root.join("connections").join(format!("{id}.json"));
        let legacy_ledger = root.join("meter.sqlite").is_file();
        let last_legacy = legacy.exists()
            && std::fs::read_dir(root.join("connections"))?
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|v| v == "json"))
                .count()
                == 1;
        let mut files = vec![index(&root, id)?, legacy];
        let prefix = format!("{id}-");
        if let Ok(entries) = std::fs::read_dir(root.join("direct/keys")) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let saved: Saved = read_json(&entry.path())?;
                    if saved.connection.id == id {
                        files.push(entry.path());
                    }
                }
            }
        }
        for entry in std::fs::read_dir(home)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".config.toml"))
                .is_some_and(|name| belongs_to(name, id))
            {
                files.push(entry.path());
            }
        }
        let backup = FileBackup::capture(files.clone())?;
        let registration = last_legacy
            .then(|| scheduler::snapshot(&root))
            .transpose()?;
        let expected = config_revision(&root)?;
        let mut values = config(&root)?;
        let changed = remove_provider_tables(&mut values, id);
        let result = (|| -> Result<()> {
            if last_legacy {
                scheduler::unregister(&root)?;
            }
            for path in files {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
            if changed {
                write_default(&root, &values, &expected)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            backup.restore()?;
            if let Some(registration) = registration {
                scheduler::restore(&root, &registration)?;
            }
            return Err(error);
        }
        return Ok(json!({"removed":true,"legacyLedger":legacy_ledger}));
    }
    anyhow::bail!("未知 API 操作")
}
