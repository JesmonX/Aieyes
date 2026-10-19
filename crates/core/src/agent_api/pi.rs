//! Pi keeps provider credentials in models.json; OMP uses models.yml.
//! Default model selection lives in a second, independently versioned file.
use super::*;
use std::path::Path;

struct Config {
    path: PathBuf,
    value: Value,
    original: Option<Vec<u8>>,
    revision: String,
}
impl Config {
    fn load(path: PathBuf) -> Result<Self> {
        files::safe(&path)?;
        let original = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 1024 * 1024, "原生配置文件过大");
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => anyhow::bail!("无法读取原生配置文件"),
        };
        let value: Value = match &original {
            Some(bytes) if path.extension().is_some_and(|e| e == "json") => {
                serde_json::from_slice(bytes)
                    .map_err(|_| anyhow::anyhow!("原生配置不是有效 JSON；未覆盖配置"))?
            }
            Some(bytes) => serde_yaml_ng::from_slice(bytes)
                .map_err(|_| anyhow::anyhow!("原生配置不是有效 YAML；未覆盖配置"))?,
            None => json!({}),
        };
        ensure!(value.is_object(), "原生配置应为对象");
        let revision = original
            .as_ref()
            .map_or_else(|| "missing".into(), |b| digest(b));
        Ok(Self {
            path,
            value,
            original,
            revision,
        })
    }
    fn encoded(&self, value: &Value) -> Result<Vec<u8>> {
        Ok(if self.path.extension().is_some_and(|e| e == "json") {
            serde_json::to_vec_pretty(value)?
        } else {
            serde_yaml_ng::to_string(value)?.into_bytes()
        })
    }
    fn check(&self) -> Result<()> {
        ensure!(
            Self::load(self.path.clone())?.revision == self.revision,
            "原生模型或默认设置已改变，请重新读取后保存"
        );
        Ok(())
    }
    fn backup(&self, agent: &str) -> Result<Option<PathBuf>> {
        let Some(bytes) = &self.original else {
            return Ok(None);
        };
        let dir = self
            .path
            .parent()
            .context("配置路径无效")?
            .join(".aieyes/api-backups");
        files::private_dir(&dir)?;
        let path = dir.join(format!(
            "{agent}-{}-{}",
            files::new_id(),
            self.path.file_name().unwrap().to_string_lossy()
        ));
        files::atomic_bytes(&path, bytes)?;
        Ok(Some(path))
    }
}

fn yaml_path(root: &Path, name: &str) -> PathBuf {
    let yml = root.join(format!("{name}.yml"));
    if yml.exists() || !root.join(format!("{name}.yaml")).exists() {
        yml
    } else {
        root.join(format!("{name}.yaml"))
    }
}

fn public(models: &Config, defaults: &Config, omp: bool) -> Value {
    let provider = &models.value["providers"][PROVIDER];
    let selected = if omp {
        defaults.value["modelRoles"]["default"]
            .as_str()
            .and_then(|s| s.strip_prefix(&format!("{PROVIDER}/")))
    } else if defaults.value["defaultProvider"] == PROVIDER {
        defaults.value["defaultModel"].as_str()
    } else {
        None
    };
    let model = provider["models"]
        .as_array()
        .and_then(|a| a.iter().find(|v| selected.is_some_and(|s| v["id"] == s)))
        .unwrap_or(&Value::Null);
    let key = provider["apiKey"].as_str().unwrap_or("");
    json!({"configPath":models.path,"revision":models.revision,"nativeRevision":defaults.revision,
        "baseUrl":provider["baseUrl"].as_str().unwrap_or(""),"model":selected.unwrap_or(""),
        "contextWindow":model["contextWindow"],"maxOutputTokens":model["maxTokens"],
        "hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required",
        "effort":if model["reasoning"]==false {"disabled"} else {defaults.value["defaultThinkingLevel"].as_str().unwrap_or("low")}})
}

fn prepare(
    models: &Value,
    defaults: &Value,
    params: &Value,
    omp: bool,
) -> Result<(Value, Value, String, String, String, bool)> {
    let old = &models["providers"][PROVIDER];
    let (base, model, key, no_key) = connection(
        params,
        old["baseUrl"].as_str().unwrap_or(""),
        old["apiKey"].as_str().unwrap_or(""),
    )?;
    ensure!(
        !key.starts_with(['!', '$']),
        "请输入实际 API Key；此表单不执行配置中的命令或变量"
    );
    let context = params["contextWindow"]
        .as_u64()
        .context("请输入模型上下文窗口大小")?;
    let output = params["maxOutputTokens"]
        .as_u64()
        .context("请输入最大输出 Token 数")?;
    ensure!(
        (1024..=16_777_216).contains(&context) && output > 0 && output <= context,
        "模型上下文或最大输出 Token 数无效"
    );
    let effort = params["effort"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("low");
    ensure!(
        ["disabled", "low", "medium", "high", "xhigh", "max"].contains(&effort),
        "推理强度不支持"
    );
    let enabled = effort != "disabled";
    let mut models = models.clone();
    ensure!(
        models["providers"].is_null() || models["providers"].is_object(),
        "providers 应为对象"
    );
    let mut entry = json!({"id":model,"name":model,"reasoning":enabled,"input":["text"],"contextWindow":context,"maxTokens":output,
        "compat":{"supportsStore":false,"supportsDeveloperRole":false,"supportsReasoningEffort":enabled,"maxTokensField":"max_tokens","thinkingFormat":"openai"}});
    if omp {
        entry["compat"]["supportsReasoningParams"] = json!(enabled);
        if enabled {
            entry["thinking"] = json!({"mode":"effort","efforts":["low","medium","high","xhigh","max"],"defaultLevel":effort});
        }
    }
    // Native cost defaults are only a client display estimate. Aieyes prices
    // requests independently and never treats these zero defaults as a balance.
    entry["cost"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0});
    models["providers"][PROVIDER] =
        json!({"baseUrl":base,"apiKey":key,"api":"openai-completions","models":[entry]});
    let mut defaults = defaults.clone();
    if omp {
        ensure!(
            defaults["modelRoles"].is_null() || defaults["modelRoles"].is_object(),
            "modelRoles 应为对象"
        );
        defaults["modelRoles"]["default"] = json!(format!("{PROVIDER}/{model}"));
    } else {
        defaults["defaultProvider"] = json!(PROVIDER);
        defaults["defaultModel"] = json!(model);
    }
    defaults["defaultThinkingLevel"] = json!(if enabled { effort } else { "off" });
    Ok((models, defaults, base, model, key, no_key))
}

pub(super) fn call(
    engine: &Engine,
    source: &Source,
    settings: &Settings,
    method: &str,
    mut params: Value,
) -> Result<Value> {
    let omp = source.provider == "omp";
    let location = json!([
        source.provider,
        source.path,
        source.host_id,
        source.machine_binding
    ]);
    let remembered = format!("agent-api-path:{}", source.id);
    if params["configPath"].as_str().is_none_or(str::is_empty) {
        let previous: Option<String> = engine
            .store
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [&remembered], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(previous) = previous
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(|v| v["source"] == location)
        {
            params["configPath"] = previous["configPath"].clone();
        }
    }
    let path = if let Some(path) = params["configPath"].as_str().filter(|p| !p.is_empty()) {
        expand(path)
    } else {
        let root = if omp {
            crate::provider_paths::omp_config_dir()?
        } else {
            expand(&crate::provider_paths::default_path(
                &source.provider,
                false,
            )?)
        };
        if omp {
            yaml_path(&root, "models")
        } else {
            root.join("models.json")
        }
    };
    if method == "agentApi.inspect" && params["locationOnly"] == true {
        return Ok(
            json!({"configPath":path,"revision":"","baseUrl":"","model":"","hasApiKey":false,"noKey":false}),
        );
    }
    ensure!(
        path.is_absolute()
            && path.file_name().is_some_and(|name| if omp {
                name == "models.yml" || name == "models.yaml"
            } else {
                name == "models.json"
            }),
        "请选择客户端原生 models 配置文件的绝对路径"
    );
    let root = path.parent().context("配置路径无效")?;
    if omp {
        ensure!(
            path.exists() || !root.join("models.json").exists(),
            "存在尚未迁移的 OMP models.json，请先启动 OMP 完成原生迁移"
        );
        ensure!(
            path.file_name().is_none_or(|n| n != "models.yaml")
                || !root.join("models.yml").exists(),
            "models.yml 优先于 models.yaml，请选择当前生效的配置"
        );
    }
    let models = Config::load(path.clone())?;
    let defaults = Config::load(if omp {
        yaml_path(root, "config")
    } else {
        root.join("settings.json")
    })?;
    if method == "agentApi.inspect" {
        return Ok(public(&models, &defaults, omp));
    }
    ensure!(method == "agentApi.save", "未知 API 操作");
    ensure!(
        params["revision"] == models.revision && params["nativeRevision"] == defaults.revision,
        "原生模型或默认设置已改变，请重新读取后保存"
    );
    let (next_models, next_defaults, base, model, key, no_key) =
        prepare(&models.value, &defaults.value, &params, omp)?;
    let mut options = params.clone();
    if options["effort"].as_str().is_none_or(str::is_empty) {
        options["effort"] = json!("low");
    }
    verify(
        &source.provider,
        &base,
        &model,
        &key,
        no_key,
        source.proxy.as_ref().unwrap_or(&settings.proxy),
        &options,
    )?;
    let tx = engine.store.write_transaction()?;
    ensure!(
        engine
            .store
            .settings()?
            .sources
            .iter()
            .find(|s| s.id == source.id)
            .is_some_and(|s| serde_json::to_value(s).ok() == serde_json::to_value(source).ok()),
        "验证期间来源配置已改变，请重新读取后保存"
    );
    std::fs::create_dir_all(root)?;
    let _lock = files::lock(root)?;
    models.check()?;
    defaults.check()?;
    let models_backup = models.backup(&source.provider)?;
    let defaults_backup = defaults.backup(&source.provider)?;
    let model_bytes = models.encoded(&next_models)?;
    files::atomic_bytes(&path, &model_bytes)?;
    if let Err(error) = files::atomic_bytes(&defaults.path, &defaults.encoded(&next_defaults)?) {
        if Config::load(path.clone()).is_ok_and(|v| v.revision == digest(&model_bytes)) {
            if let Some(original) = &models.original {
                files::atomic_bytes(&path, original)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        return Err(error);
    }
    let mut result = public(
        &Config::load(path.clone())?,
        &Config::load(defaults.path)?,
        omp,
    );
    result["backupPath"] = json!(models_backup);
    result["settingsBackupPath"] = json!(defaults_backup);
    result["verified"] = json!(true);
    engine.store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![remembered,json!({"source":location,"configPath":path}).to_string()])?;
    tx.commit()?;
    Ok(result)
}
