//! Native API configuration. Keys stay in the client's private config file;
//! inspection and RPC results contain only a presence flag.
use crate::{Engine, codex_auth::files, models::*, network, store::expand};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, time::Duration};

const PROVIDER: &str = "aieyes-managed";
mod claude;
mod codebuddy;
mod grok;
mod harness;
mod harness_yaml;
mod kimi;
mod pi;
mod qoder;
mod zcode;

enum Document {
    Json(Value),
    Toml(toml::Value),
    Yaml(serde_yaml_ng::Value),
}
impl Document {
    fn encoded(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Json(value) => serde_json::to_vec(value)?,
            Self::Toml(value) => toml::to_string_pretty(value)?.into_bytes(),
            Self::Yaml(value) => harness_yaml::encode(value)?,
        })
    }
    fn public(&self, agent: &str, path: &std::path::Path, revision: &str) -> Value {
        match self {
            Self::Json(value) => public(agent, path, value, revision),
            Self::Toml(value) => {
                if agent == "grok" {
                    grok::public(path, value, revision)
                } else {
                    kimi::public(path, value, revision)
                }
            }
            Self::Yaml(value) => harness::public(path, value, revision),
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn path(source: &Source, params: &Value, validate: bool) -> Result<PathBuf> {
    let path = if let Some(p) = params["configPath"].as_str().filter(|p| !p.is_empty()) {
        expand(p)
    } else if source.provider == "deepseek-harness" {
        std::env::var_os("DSH_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".dsh"))
            .join("profiles/headless/cordis.patch.yml")
    } else if matches!(source.provider.as_str(), "kimi" | "grok") {
        std::env::var_os(if source.provider == "grok" {
            "GROK_HOME"
        } else {
            "KIMI_CODE_HOME"
        })
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            crate::store::home().join(if source.provider == "grok" {
                ".grok"
            } else {
                ".kimi-code"
            })
        })
        .join("config.toml")
    } else if source.provider == "claude" {
        std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".claude"))
            .join("settings.json")
    } else if source.provider == "zcode" {
        std::env::var_os("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                std::env::var_os("ZCODE_DATA_BASE_DIR")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(crate::store::home)
                    .join(".zcode/v2/provider_config.json")
            })
    } else if source.provider == "qoder" {
        std::env::var_os("QODER_CONFIG_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".qoder"))
            .join("settings.json")
    } else if source.provider == "codebuddy" {
        std::env::var_os("CODEBUDDY_CONFIG_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".codebuddy"))
            .join("settings.json")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".config"))
            .join("opencode/opencode.json")
    };
    if !validate {
        return Ok(path);
    }
    let extension = if source.provider == "deepseek-harness" {
        "yml"
    } else if matches!(source.provider.as_str(), "kimi" | "grok") {
        "toml"
    } else {
        "json"
    };
    ensure!(
        path.is_absolute() && path.extension().is_some_and(|e| e == extension),
        "请选择原生配置的绝对 .{extension} 路径"
    );
    files::safe(&path)?;
    if matches!(source.provider.as_str(), "qoder" | "codebuddy") {
        ensure!(
            path.file_name().is_some_and(|n| n == "settings.json"),
            "请选择客户端配置目录中的 settings.json"
        );
    }
    if source.provider == "opencode" {
        ensure!(
            path.exists() || !path.with_extension("jsonc").exists(),
            "同目录存在 JSONC 配置，请先在 OpenCode 中整理为 JSON；未覆盖配置"
        );
    }
    Ok(path)
}

fn read_document(path: &std::path::Path) -> Result<(Document, String, Option<Vec<u8>>)> {
    let bytes = match std::fs::File::open(path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, "原生配置文件过大");
            bytes
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let empty = if path.extension().is_some_and(|e| e == "yml") {
                Document::Yaml(serde_yaml_ng::Value::Sequence(vec![]))
            } else if path.extension().is_some_and(|e| e == "toml") {
                Document::Toml(toml::Value::Table(Default::default()))
            } else {
                Document::Json(json!({}))
            };
            return Ok((empty, "missing".into(), None));
        }
        Err(_) => anyhow::bail!("无法读取原生配置文件"),
    };
    let revision = digest(&bytes);
    if path.extension().is_some_and(|e| e == "yml") {
        let value = harness_yaml::parse(&bytes)?;
        ensure!(value.is_sequence(), "Harness 补丁应为 YAML 列表");
        return Ok((Document::Yaml(value), revision, Some(bytes)));
    }
    if path.extension().is_some_and(|e| e == "toml") {
        let value: toml::Value = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| toml::from_str(s).ok())
            .context("原生配置不是有效 TOML；未覆盖配置")?;
        ensure!(value.is_table(), "原生配置应为 TOML 表");
        return Ok((Document::Toml(value), revision, Some(bytes)));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("原生配置不是有效 JSON；未覆盖配置"))?;
    ensure!(value.is_object(), "原生配置应为 JSON 对象");
    Ok((Document::Json(value), revision, Some(bytes)))
}
#[cfg(test)]
fn read(path: &std::path::Path) -> Result<(Value, String)> {
    let (Document::Json(value), revision, _) = read_document(path)? else {
        anyhow::bail!("Expected JSON")
    };
    Ok((value, revision))
}

fn public(agent: &str, path: &std::path::Path, config: &Value, revision: &str) -> Value {
    if agent == "qoder" {
        return qoder::public(path, config, revision);
    }
    if agent == "claude" {
        return claude::public(path, config, revision);
    }
    if agent == "zcode" {
        return zcode::public(path, config, revision);
    }
    if agent == "codebuddy" {
        let key = config["env"]["CODEBUDDY_API_KEY"].as_str().unwrap_or("");
        return json!({"configPath":path,"revision":revision,"baseUrl":config["env"]["CODEBUDDY_BASE_URL"].as_str().unwrap_or(""),"model":config["env"]["CODEBUDDY_MODEL"].as_str().filter(|s|!s.trim().is_empty()).or_else(||config["model"].as_str()).unwrap_or(""),"hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required","effort":if config["alwaysThinkingEnabled"]==false {"disabled"} else {config["reasoningEffort"].as_str().unwrap_or("")}});
    }
    let provider = &config["provider"][PROVIDER];
    let key = provider["options"]["apiKey"].as_str().unwrap_or("");
    json!({"configPath":path,"revision":revision,"baseUrl":provider["options"]["baseURL"].as_str().unwrap_or(""),
        "model":config["model"].as_str().and_then(|s|s.strip_prefix(&format!("{PROVIDER}/"))).unwrap_or(""),
        "hasApiKey":!key.is_empty() && key!="not-required","noKey":key=="not-required"})
}

fn connection(
    params: &Value,
    old_base: &str,
    old_key: &str,
) -> Result<(String, String, String, bool)> {
    let base = params["baseUrl"]
        .as_str()
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let url = reqwest::Url::parse(&base).context("API 地址无效")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "API 地址应为不含凭据和查询参数的 HTTP(S) Base URL"
    );
    let model = params["model"].as_str().unwrap_or("").trim().to_owned();
    ensure!(
        !model.is_empty() && model.len() <= 200 && !model.chars().any(char::is_control),
        "请输入有效模型 ID"
    );
    let no_key = params["noKey"].as_bool().unwrap_or(false);
    let supplied = params["apiKey"].as_str().unwrap_or("");
    let key = if no_key {
        "not-required".to_owned()
    } else if !supplied.is_empty() {
        supplied.to_owned()
    } else {
        ensure!(
            old_base == base,
            "更换 API 地址时请重新输入 Key，或选择无需 Key"
        );
        ensure!(
            !old_key.is_empty() && old_key != "not-required",
            "请输入 API Key，或选择无需 Key"
        );
        old_key.to_owned()
    };
    ensure!(
        key.len() <= 32768 && !key.contains(['\r', '\n', '\0']),
        "API Key 格式无效"
    );
    ensure!(
        !key.starts_with("{env:") && !key.starts_with("{file:"),
        "已有 Key 使用原生变量引用，请重新输入 Key，或选择无需 Key"
    );
    Ok((base, model, key, no_key))
}

fn prepare(mut config: Value, params: &Value) -> Result<(Value, String, String, String, bool)> {
    ensure!(
        config["provider"].is_null() || config["provider"].is_object(),
        "原生 provider 配置格式不支持"
    );
    let old = &config["provider"][PROVIDER];
    ensure!(
        old.is_null() || old["name"] == "Aieyes API",
        "原生配置已使用 aieyes-managed 标识；未覆盖"
    );
    let (base, model, key, no_key) = connection(
        params,
        old["options"]["baseURL"].as_str().unwrap_or(""),
        old["options"]["apiKey"].as_str().unwrap_or(""),
    )?;
    for field in ["enabled_providers", "disabled_providers"] {
        if let Some(value) = config.get_mut(field) {
            let list = value
                .as_array_mut()
                .context("原生 Provider 过滤配置格式不支持")?;
            ensure!(
                list.iter().all(Value::is_string),
                "原生 Provider 过滤配置格式不支持"
            );
            list.retain(|value| value != PROVIDER);
            if field == "enabled_providers" {
                list.push(json!(PROVIDER));
            }
        }
    }
    // Dedicated entry only. Other providers, hooks, permissions and options survive.
    config["provider"][PROVIDER] = json!({"name":"Aieyes API","npm":"@ai-sdk/openai-compatible","options":{"baseURL":base,"apiKey":key},"models":{&model:{"name":model}}});
    config["model"] = json!(format!("{PROVIDER}/{model}"));
    Ok((config, base, model, key, no_key))
}

fn prepare_codebuddy(
    mut config: Value,
    params: &Value,
) -> Result<(Value, String, String, String, bool)> {
    ensure!(
        config["env"].is_null() || config["env"].is_object(),
        "原生 env 配置格式不支持"
    );
    ensure!(
        config["apiKeyHelper"].as_str().is_none_or(|s| s.is_empty())
            && config["env"]["CODEBUDDY_AUTH_TOKEN"]
                .as_str()
                .is_none_or(|s| s.is_empty()),
        "原生配置启用了优先级更高的登录令牌或 apiKeyHelper；请先在 CodeBuddy 中调整认证方式"
    );
    let (base, model, key, no_key) = connection(
        params,
        config["env"]["CODEBUDDY_BASE_URL"].as_str().unwrap_or(""),
        config["env"]["CODEBUDDY_API_KEY"].as_str().unwrap_or(""),
    )?;
    ensure!(
        !base.contains("${") && !key.contains("${"),
        "CodeBuddy API 地址和 Key 不支持环境变量引用，请输入实际值"
    );
    config["env"]["CODEBUDDY_BASE_URL"] = json!(base);
    config["env"]["CODEBUDDY_API_KEY"] = json!(key);
    config["env"]["CODEBUDDY_MODEL"] = json!(model);
    config["model"] = json!(model);
    let effort = codebuddy_effort(&config, params)?;
    config["alwaysThinkingEnabled"] = json!(effort != "disabled");
    if effort != "disabled" {
        config["reasoningEffort"] = json!(effort);
    }
    Ok((config, base, model, key, no_key))
}

fn verify(
    agent: &str,
    base: &str,
    model: &str,
    key: &str,
    no_key: bool,
    proxy: &ProxyConfig,
    options: &Value,
) -> Result<()> {
    let client = network::client_with_policy(
        proxy,
        network::ClientPolicy {
            redirects: 0,
            timeout: Duration::from_secs(60),
            ..Default::default()
        },
    )?;
    let mut body = json!({"model":model,"messages":[{"role":"user","content":"Reply exactly AIEYES_API_OK."}],"max_tokens":if matches!(agent,"claude"|"codebuddy"){256}else{64},"stream":false});
    let effort = if matches!(agent, "pi" | "omp") && options["effort"] == "disabled" {
        String::new()
    } else if agent == "codebuddy" {
        let level = codebuddy_effort(&json!({}), options)?;
        if level == "disabled" {
            String::new()
        } else {
            level
        }
    } else {
        effort(options)?
    };
    if !effort.is_empty() {
        if agent == "claude" {
            body["output_config"] = json!({"effort":effort});
        } else if matches!(agent, "zcode" | "codebuddy" | "pi" | "omp") {
            body["reasoning_effort"] = json!(effort);
        }
    }
    let mut request = client
        .post(if agent == "claude" {
            format!("{base}/v1/messages")
        } else {
            format!("{base}/chat/completions")
        })
        .json(&body);
    if agent == "claude" {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if !no_key {
        if agent == "claude" && options["authMode"].as_str().unwrap_or("api-key") == "api-key" {
            request = request.header("x-api-key", key);
        } else {
            request = request.bearer_auth(key);
        }
        if agent == "codebuddy" {
            request = request.header("x-api-key", key);
        }
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("API 连接失败，请检查地址、网络和代理"))?;
    ensure!(
        response.status().is_success(),
        "API 验证失败（HTTP {}）",
        response.status().as_u16()
    );
    let mut body = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| anyhow::anyhow!("API 响应读取失败"))?;
    ensure!(body.len() <= 1024 * 1024, "API 响应过大");
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| anyhow::anyhow!("API 未返回有效协议 JSON"))?;
    if agent == "claude" {
        ensure!(
            value["content"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v["type"] == "text"
                    && v["text"].as_str().is_some_and(|s| !s.trim().is_empty()))),
            "API 未返回有效的 Anthropic 模型回复"
        );
        return Ok(());
    }
    ensure!(
        value["choices"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c["message"]["content"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty()))),
        "API 未返回有效的模型回复"
    );
    Ok(())
}

fn effort(params: &Value) -> Result<String> {
    let level = params["effort"].as_str().unwrap_or("");
    ensure!(
        ["", "low", "medium", "high", "xhigh", "max"].contains(&level),
        "推理强度不支持"
    );
    Ok(level.into())
}

fn codebuddy_effort(config: &Value, params: &Value) -> Result<String> {
    ensure!(
        params["effort"].is_null() || params["effort"].is_string(),
        "推理强度不支持"
    );
    let level = params["effort"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if config["alwaysThinkingEnabled"] == false {
                "disabled"
            } else {
                config["reasoningEffort"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("low")
            }
        });
    ensure!(
        [
            "disabled", "minimal", "low", "medium", "high", "xhigh", "max"
        ]
        .contains(&level),
        "推理强度不支持"
    );
    Ok(level.into())
}

impl Engine {
    pub(crate) fn agent_api_call(&self, method: &str, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let source = settings
            .sources
            .iter()
            .find(|s| Some(s.id.as_str()) == params["sourceId"].as_str())
            .context("请选择 Agent 数据目录")?;
        ensure!(
            matches!(
                source.provider.as_str(),
                "opencode"
                    | "codebuddy"
                    | "kimi"
                    | "deepseek-harness"
                    | "claude"
                    | "zcode"
                    | "grok"
                    | "qoder"
                    | "pi"
                    | "omp"
            ) && source.subscription.is_none(),
            "此 Agent 的原生 API 配置尚未接入"
        );
        ensure!(
            source.host_id.is_none() && source.machine_binding == MachineBinding::Bound,
            "原生 API 配置目前需要本机目录"
        );
        if matches!(source.provider.as_str(), "pi" | "omp") {
            return pi::call(self, source, &settings, method, params);
        }
        let location = json!([
            source.provider,
            source.path,
            source.host_id,
            source.machine_binding
        ]);
        let remembered_key = format!("agent-api-path:{}", source.id);
        let mut params = params;
        if params["configPath"].as_str().is_none_or(|p| p.is_empty()) {
            let stored: Option<String> = self
                .store
                .db
                .query_row(
                    "SELECT value FROM kv WHERE key=?1",
                    [&remembered_key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(stored) = stored
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .filter(|v| v["source"] == location)
            {
                params["configPath"] = stored["configPath"].clone();
            }
        }
        let location_only = method == "agentApi.inspect" && params["locationOnly"] == true;
        let path = path(source, &params, !location_only)?;
        if location_only {
            return Ok(
                json!({"configPath":path,"revision":"","baseUrl":"","model":"","hasApiKey":false,"noKey":false}),
            );
        }
        let (old, revision, original) = read_document(&path)?;
        let codebuddy = if source.provider == "codebuddy" {
            Some(codebuddy::Models::load(&path)?)
        } else {
            None
        };
        let native = if source.provider == "deepseek-harness" {
            Some(harness::Native::load(&path)?)
        } else {
            None
        };
        if method == "agentApi.inspect" {
            if let Some(native) = native {
                return Ok(native.public(&path, &revision));
            }
            let public = old.public(&source.provider, &path, &revision);
            return Ok(if let Some(models) = &codebuddy {
                models.annotate(public)
            } else {
                public
            });
        }
        ensure!(method == "agentApi.save", "未知 API 操作");
        ensure!(
            params["revision"].as_str() == Some(&revision),
            "原生配置已改变，请重新读取后保存"
        );
        if let Some(native) = &native {
            ensure!(
                params["nativeRevision"].as_str() == Some(&native.revision),
                "Harness 合成配置已改变，请重新读取后保存"
            );
        }
        if let Some(models) = &codebuddy {
            ensure!(
                params["nativeRevision"].as_str() == Some(&models.revision),
                "CodeBuddy 模型配置已改变，请重新读取后保存"
            );
        }
        let mut qoder = None;
        let (mut config, base, model, key, no_key) = match old {
            Document::Yaml(old) => {
                let (next, base, model, key, no_key) = native
                    .as_ref()
                    .context("Harness 配置不可用")?
                    .prepare(old, &params)?;
                (Document::Yaml(next), base, model, key, no_key)
            }
            Document::Toml(old) => {
                let (next, base, model, key, no_key) = if source.provider == "grok" {
                    grok::prepare(old, &params)?
                } else {
                    kimi::prepare(old, &params)?
                };
                (Document::Toml(next), base, model, key, no_key)
            }
            Document::Json(old) => {
                if source.provider == "qoder" {
                    let prepared = qoder::prepare(&old, &params)?;
                    let connection = (
                        Document::Json(old),
                        prepared.base.clone(),
                        prepared.model.clone(),
                        prepared.key.clone(),
                        prepared.no_key,
                    );
                    qoder = Some(prepared);
                    connection
                } else {
                    let (next, base, model, key, no_key) = if source.provider == "codebuddy" {
                        prepare_codebuddy(old, &params)?
                    } else if source.provider == "claude" {
                        claude::prepare(old, &params)?
                    } else if source.provider == "zcode" {
                        zcode::prepare(old, &params)?
                    } else {
                        prepare(old, &params)?
                    };
                    (Document::Json(next), base, model, key, no_key)
                }
            }
        };
        let mut verify_options = params.clone();
        if source.provider == "codebuddy" {
            verify_options["effort"] =
                config.public(&source.provider, &path, &revision)["effort"].clone();
        }
        let codebuddy_next = codebuddy
            .as_ref()
            .map(|models| {
                models.prepare(
                    &base,
                    &model,
                    &key,
                    verify_options["effort"].as_str().unwrap_or("low"),
                )
            })
            .transpose()?;
        verify(
            &source.provider,
            &base,
            &model,
            &key,
            no_key,
            source.proxy.as_ref().unwrap_or(&settings.proxy),
            &verify_options,
        )?;
        if let Some(qoder) = &qoder {
            qoder.validate(&path, source.proxy.as_ref().unwrap_or(&settings.proxy))?;
        }
        if let Some(native) = &native {
            ensure!(
                harness::Native::load(&path)?.revision == native.revision,
                "验证期间 Harness 合成配置已改变，请重新读取后保存"
            );
        }
        // Recheck source and the exact config bytes after the potentially slow call.
        let tx = self.store.write_transaction()?;
        let current = self.store.settings()?;
        ensure!(
            current
                .sources
                .iter()
                .find(|s| s.id == source.id)
                .is_some_and(|s| serde_json::to_value(s).ok() == serde_json::to_value(source).ok()),
            "验证期间来源配置已改变，请重新读取后保存"
        );
        std::fs::create_dir_all(path.parent().context("配置路径无效")?)?;
        let _lock = files::lock(path.parent().unwrap())?;
        ensure!(
            read_document(&path)?.1 == revision,
            "验证期间原生配置已改变，请重新读取后保存"
        );
        if let Some(models) = &codebuddy {
            models.check(&path)?;
        }
        let backup = if let Some(original) = original {
            let dir = path.parent().unwrap().join(".aieyes/api-backups");
            files::private_dir(&dir)?;
            let backup = dir.join(format!(
                "{}-{}.{}",
                source.provider,
                files::new_id(),
                path.extension().unwrap().to_string_lossy()
            ));
            files::atomic_bytes(&backup, &original)?;
            Some(backup)
        } else {
            None
        };
        if let Some(qoder) = &qoder {
            let Document::Json(old) = &config else {
                unreachable!()
            };
            config = Document::Json(qoder.save(
                &path,
                old,
                source.proxy.as_ref().unwrap_or(&settings.proxy),
            )?);
        }
        let bytes = config.encoded()?;
        let models_backup = if let (Some(models), Some(next)) = (&codebuddy, &codebuddy_next) {
            models.save(&path, &bytes, next)?
        } else {
            files::atomic_bytes(&path, &bytes)?;
            None
        };
        let mut result = config.public(&source.provider, &path, &digest(&bytes));
        if codebuddy.is_some() {
            result = codebuddy::Models::load(&path)?.annotate(result);
            result["modelsBackupPath"] = json!(models_backup);
        }
        result["backupPath"] = json!(backup);
        result["verified"] = json!(true);
        self.store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![remembered_key,json!({"source":location,"configPath":path}).to_string()])?;
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    fn engine(root: &std::path::Path) -> Engine {
        let engine = Engine::open(&root.join("data")).unwrap();
        engine
            .store
            .save_settings(&Settings {
                sources: vec![Source {
                    id: "open".into(),
                    provider: "opencode".into(),
                    path: root.to_string_lossy().into(),
                    ..Default::default()
                }],
                proxy: ProxyConfig {
                    mode: "direct".into(),
                    url: String::new(),
                },
                ..Default::default()
            })
            .unwrap();
        engine
    }
    fn serve(
        status: &'static str,
        body: &'static str,
        change: impl FnOnce() + Send + 'static,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..pos]);
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= pos + 4 + length {
                        break;
                    }
                }
            }
            change();
            write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }
    const RESPONSE: &str = r#"{"choices":[{"message":{"content":"AIEYES_API_OK"}}]}"#;

    #[test]
    fn claude_verifies_native_messages_auth_and_preserves_hooks() {
        for mode in ["api-key", "auth-token"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let engine = engine(&root);
            let mut settings = engine.store.settings().unwrap();
            settings.sources[0].provider = "claude".into();
            engine.store.save_settings(&settings).unwrap();
            let path = root.join("settings.json");
            files::atomic(&path,&json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"existing"}]}]},"env":{"KEEP":"yes"}})).unwrap();
            let old = engine
                .agent_api_call(
                    "agentApi.inspect",
                    json!({"sourceId":"open","configPath":path}),
                )
                .unwrap();
            let (base, task) = serve(
                "200 OK",
                r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"AIEYES_API_OK"}]}"#,
                || {},
            );
            let root_url = base.strip_suffix("/v1").unwrap();
            let result=engine.agent_api_call("agentApi.save",json!({"sourceId":"open","configPath":path,"revision":old["revision"],"baseUrl":base,"model":"qwen","authMode":mode,"apiKey":"PRIVATE_CLAUDE","effort":"low"})).unwrap();
            let request = task.join().unwrap().to_lowercase();
            assert!(request.contains("post /v1/messages"));
            assert!(request.contains("anthropic-version: 2023-06-01"));
            assert!(request.contains("\"output_config\":{\"effort\":\"low\"}"));
            if mode == "api-key" {
                assert!(request.contains("x-api-key: private_claude"));
                assert!(!request.contains("authorization:"));
            } else {
                assert!(request.contains("authorization: bearer private_claude"));
                assert!(!request.contains("x-api-key:"));
            }
            assert_eq!(result["baseUrl"], root_url);
            assert!(!result.to_string().contains("PRIVATE_CLAUDE"));
            let saved = read(&path).unwrap().0;
            assert_eq!(saved["env"]["KEEP"], "yes");
            assert!(saved["hooks"]["Stop"].is_array());
            assert_eq!(saved["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "low");
        }
    }

    #[test]
    fn zcode_rules_keep_other_providers_and_use_dynamic_native_option_mapping() {
        let old = json!({"schemaVersion":1,"config":{"providerConfigRules":{"providerRules":[{"providerId":"other","config":{"access":{"apiKey":"PRIVATE_OTHER"}}}]},"modelConfigRules":{"providerModelRules":[{"providerId":"other","modelId":"qwen","config":{"properties":{"contextWindow":10000}}}],"manualProviderModelRules":[]}}});
        let (saved,_,_,_,_)=zcode::prepare(old.clone(),&json!({"baseUrl":"https://fixture.test/v1","model":"qwen","contextWindow":261000,"maxOutputTokens":2048,"effort":"low","noKey":true})).unwrap();
        assert_eq!(
            saved["config"]["providerConfigRules"]["providerRules"][0],
            old["config"]["providerConfigRules"]["providerRules"][0]
        );
        assert_eq!(
            saved["config"]["modelConfigRules"]["providerModelRules"][0],
            old["config"]["modelConfigRules"]["providerModelRules"][0]
        );
        assert_eq!(
            saved["config"]["defaultModelSelection"]["options"]["reasoningLevel"],
            "low"
        );
        assert!(
            saved["config"]["defaultModelSelection"]["options"]
                .get("maxOutputTokens")
                .is_none()
        );
        let response = zcode::public(
            std::path::Path::new("provider_config.json"),
            &saved,
            "revision",
        );
        assert_eq!(response["maxOutputTokens"], 2048);
        assert_eq!(response["effort"], "low");
        assert!(!response.to_string().contains("PRIVATE_OTHER"));
        let mut future = old;
        future["schemaVersion"] = json!(2);
        assert!(zcode::prepare(future, &json!({})).is_err());
    }

    #[test]
    fn kimi_toml_preserves_other_models_and_exact_backup_without_copying_account_key() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let engine = engine(&root);
        let mut settings = engine.store.settings().unwrap();
        settings.sources[0].provider = "kimi".into();
        engine.store.save_settings(&settings).unwrap();
        let path = root.join("config.toml");
        let original=b"# preserve this exact backup\ndefault_model = 'other'\ncustom_date = 2026-10-18\n[loop_control]\nmax_attempts_per_step = 3\n[providers.account]\ntype = 'kimi'\napi_key = 'PRIVATE_ACCOUNT_KEY'\nbase_url = 'https://api.kimi.com/coding/v1'\n[models.other]\nprovider = 'account'\nmodel = 'kimi-model'\nmax_context_size = 128000\n";
        std::fs::write(&path, original).unwrap();
        let inspected = engine
            .agent_api_call(
                "agentApi.inspect",
                json!({"sourceId":"open","configPath":path}),
            )
            .unwrap();
        assert!(!inspected.to_string().contains("PRIVATE_ACCOUNT_KEY"));
        assert_eq!(inspected["contextWindow"], 128000);
        // The account key cannot be implicitly copied to a new custom model.
        assert!(engine.agent_api_call("agentApi.save",json!({"sourceId":"open","configPath":path,"revision":inspected["revision"],"baseUrl":"https://api.kimi.com/coding/v1","model":"qwen","contextWindow":261000})).is_err());
        let (url, task) = serve("200 OK", RESPONSE, || {});
        let result=engine.agent_api_call("agentApi.save",json!({"sourceId":"open","configPath":path,"revision":inspected["revision"],"baseUrl":url,"model":"qwen","contextWindow":261000,"noKey":true})).unwrap();
        assert!(
            !task
                .join()
                .unwrap()
                .to_lowercase()
                .contains("authorization:")
        );
        let (Document::Toml(saved), _, _) = read_document(&path).unwrap() else {
            panic!("not TOML")
        };
        assert_eq!(
            saved["providers"]["account"]["api_key"].as_str(),
            Some("PRIVATE_ACCOUNT_KEY")
        );
        assert!(saved["custom_date"].is_datetime());
        assert_eq!(
            saved["loop_control"]["max_attempts_per_step"].as_integer(),
            Some(3)
        );
        assert_eq!(
            saved["models"][PROVIDER]["max_context_size"].as_integer(),
            Some(261000)
        );
        assert_eq!(
            std::fs::read(result["backupPath"].as_str().unwrap()).unwrap(),
            original
        );
        assert_eq!(result["contextWindow"], 261000);
        assert_eq!(result["model"], "qwen");
    }

    #[test]
    fn codebuddy_native_config_preserves_permissions_and_rejects_conflicting_auth() {
        let old = json!({"env":{"KEEP":"yes"},"permissions":{"deny":["Bash(rm:*)"]},"model":"old"});
        let params = json!({"baseUrl":"https://fixture.test/v1","model":"qwen","noKey":true});
        let next = prepare_codebuddy(old.clone(), &params).unwrap().0;
        assert_eq!(next["permissions"], old["permissions"]);
        assert_eq!(next["env"]["KEEP"], "yes");
        assert_eq!(next["env"]["CODEBUDDY_API_KEY"], "not-required");
        assert_eq!(next["model"], "qwen");
        assert_eq!(next["env"]["CODEBUDDY_MODEL"], "qwen");
        assert_eq!(next["reasoningEffort"], "low");
        assert_eq!(next["alwaysThinkingEnabled"], true);
        let disabled=prepare_codebuddy(next.clone(),&json!({"baseUrl":"https://fixture.test/v1","model":"qwen","noKey":true,"effort":"disabled"})).unwrap().0;
        assert_eq!(disabled["alwaysThinkingEnabled"], false);
        assert_eq!(
            prepare_codebuddy(disabled, &params).unwrap().0["alwaysThinkingEnabled"],
            false
        );
        assert!(prepare_codebuddy(old.clone(),&json!({"baseUrl":"https://fixture.test/v1","model":"qwen","noKey":true,"effort":"invalid"})).is_err());
        assert!(prepare_codebuddy(old.clone(),&json!({"baseUrl":"https://fixture.test/v1","model":"qwen","apiKey":"${PRIVATE_ENV}"})).is_err());
        let inspected = public(
            "codebuddy",
            std::path::Path::new("settings.json"),
            &next,
            "revision",
        );
        assert_eq!(inspected["noKey"], true);
        assert_eq!(inspected["hasApiKey"], false);
        assert_eq!(inspected["effort"], "low");
        for conflict in [
            json!({"apiKeyHelper":"PRIVATE_SCRIPT"}),
            json!({"env":{"CODEBUDDY_AUTH_TOKEN":"PRIVATE_TOKEN"}}),
        ] {
            let error = prepare_codebuddy(conflict, &params).unwrap_err();
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }

    #[test]
    fn codebuddy_validation_uses_the_effort_that_is_saved_in_native_settings() {
        for selected in [None, Some("disabled"), Some("minimal")] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let engine = engine(&root);
            let mut settings = engine.store.settings().unwrap();
            settings.sources[0].provider = "codebuddy".into();
            engine.store.save_settings(&settings).unwrap();
            let path = root.join("settings.json");
            let original = json!({"permissions":{"deny":["Bash(rm:*)"]},"env":{"KEEP":"yes"}});
            files::atomic(&path, &original).unwrap();
            let initial = engine
                .agent_api_call(
                    "agentApi.inspect",
                    json!({"sourceId":"open","configPath":path}),
                )
                .unwrap();
            let (url, task) = serve("200 OK", RESPONSE, || {});
            let mut params = json!({"sourceId":"open","configPath":path,"revision":initial["revision"],"nativeRevision":initial["nativeRevision"],"baseUrl":url,"model":"qwen","noKey":true});
            if let Some(level) = selected {
                params["effort"] = json!(level);
            }
            let result = engine.agent_api_call("agentApi.save", params).unwrap();
            let request = task.join().unwrap();
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let expected = selected.unwrap_or("low");
            assert_eq!(result["effort"], expected);
            if expected == "disabled" {
                assert!(body.get("reasoning_effort").is_none());
            } else {
                assert_eq!(body["reasoning_effort"], expected);
            }
            let saved = read(&path).unwrap().0;
            assert_eq!(saved["alwaysThinkingEnabled"], expected != "disabled");
            assert_eq!(saved["permissions"], original["permissions"]);
            assert_eq!(saved["env"]["KEEP"], "yes");
            let catalog: Value =
                serde_json::from_slice(&std::fs::read(path.with_file_name("models.json")).unwrap())
                    .unwrap();
            assert_eq!(catalog["models"][0]["id"], "qwen");
            assert_eq!(
                catalog["models"][0]["supportsReasoning"],
                expected != "disabled"
            );
            assert_eq!(
                result["nativeRevision"],
                codebuddy::Models::load(&path).unwrap().revision
            );
        }
    }

    #[test]
    fn codebuddy_rejects_catalog_changes_during_validation_without_writing_settings() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let engine = engine(&root);
        let mut settings = engine.store.settings().unwrap();
        settings.sources[0].provider = "codebuddy".into();
        engine.store.save_settings(&settings).unwrap();
        let path = root.join("settings.json");
        let bytes = b"{\"permissions\":{\"deny\":[\"Bash(rm:*)\"]}}";
        files::atomic_bytes(&path, bytes).unwrap();
        let initial = engine
            .agent_api_call(
                "agentApi.inspect",
                json!({"sourceId":"open","configPath":path}),
            )
            .unwrap();
        let catalog_path = path.with_file_name("models.json");
        let concurrent = json!({"models":[{"id":"native-added","apiKey":"PRIVATE_CONCURRENT"}]});
        let expected = concurrent.clone();
        let changed = catalog_path.clone();
        let (url, task) = serve("200 OK", RESPONSE, move || {
            files::atomic(&changed, &concurrent).unwrap();
        });
        let error = engine.agent_api_call("agentApi.save", json!({"sourceId":"open","configPath":path,"revision":initial["revision"],"nativeRevision":initial["nativeRevision"],"baseUrl":url,"model":"qwen","noKey":true})).unwrap_err();
        task.join().unwrap();
        assert!(!error.to_string().contains("PRIVATE"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(catalog_path).unwrap()).unwrap(),
            expected
        );
        assert!(!root.join(".aieyes/api-backups").exists());
    }

    #[test]
    fn verifies_before_writing_and_preserves_config_with_private_backup() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let engine = engine(&root);
        let path = root.join("opencode.json");
        let original = json!({"permission":{"bash":"ask"},"provider":{"other":{"options":{"apiKey":"PRIVATE_OTHER"}}},"model":"other/original"});
        files::atomic(&path, &original).unwrap();
        let inspected = engine
            .agent_api_call(
                "agentApi.inspect",
                json!({"sourceId":"open","configPath":path}),
            )
            .unwrap();
        assert!(!inspected.to_string().contains("PRIVATE_OTHER"));
        let (url, task) = serve("200 OK", RESPONSE, || {});
        let params = json!({"sourceId":"open","configPath":path,"revision":inspected["revision"],"baseUrl":url,"model":"qwen","apiKey":"PRIVATE_NEW"});
        let result = engine
            .agent_api_call("agentApi.save", params.clone())
            .unwrap();
        assert!(!result.to_string().contains("PRIVATE_NEW"));
        assert_eq!(result["hasApiKey"], true);
        let request = task.join().unwrap();
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer private_new")
        );
        assert!(request.contains("POST /v1/chat/completions"));
        let saved = read(&path).unwrap().0;
        assert_eq!(saved["permission"], original["permission"]);
        assert_eq!(saved["provider"]["other"], original["provider"]["other"]);
        assert_eq!(saved["model"], "aieyes-managed/qwen");
        assert_eq!(
            files::read(&PathBuf::from(result["backupPath"].as_str().unwrap())).unwrap(),
            original
        );
        let before = std::fs::read(&path).unwrap();
        assert!(
            engine
                .agent_api_call("agentApi.save", params)
                .unwrap_err()
                .to_string()
                .contains("已改变")
        );
        assert_eq!(before, std::fs::read(&path).unwrap());
        let remembered = engine
            .agent_api_call("agentApi.inspect", json!({"sourceId":"open"}))
            .unwrap();
        assert_eq!(remembered["configPath"], path.to_string_lossy().as_ref());
        let mut moved = engine.store.settings().unwrap();
        moved.sources[0].path = "/moved-source".into();
        engine.store.save_settings(&moved).unwrap();
        let reset = engine
            .agent_api_call(
                "agentApi.inspect",
                json!({"sourceId":"open","locationOnly":true}),
            )
            .unwrap();
        assert_ne!(reset["configPath"], remembered["configPath"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn failed_http_and_concurrent_native_changes_never_overwrite_configuration() {
        for concurrent in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let engine = engine(&root);
            let path = root.join("opencode.json");
            let inspected = engine
                .agent_api_call(
                    "agentApi.inspect",
                    json!({"sourceId":"open","configPath":path}),
                )
                .unwrap();
            let modified = path.clone();
            let (url, task) = serve(
                if concurrent {
                    "200 OK"
                } else {
                    "401 Unauthorized"
                },
                if concurrent {
                    RESPONSE
                } else {
                    "PRIVATE_ERROR_KEY"
                },
                move || {
                    if concurrent {
                        std::fs::write(modified, b"{\"model\":\"native-change\"}").unwrap();
                    }
                },
            );
            let error=engine.agent_api_call("agentApi.save",json!({"sourceId":"open","configPath":path,"revision":inspected["revision"],"baseUrl":url,"model":"qwen","noKey":true})).unwrap_err();
            let request = task.join().unwrap();
            assert!(!request.to_lowercase().contains("authorization:"));
            assert!(!error.to_string().contains("PRIVATE_ERROR"));
            if concurrent {
                assert_eq!(read(&path).unwrap().0["model"], "native-change");
            } else {
                assert!(!path.exists());
            }
        }
    }

    #[test]
    fn changing_origin_requires_new_key_and_invalid_native_files_are_preserved() {
        let old = json!({"provider":{PROVIDER:{"name":"Aieyes API","options":{"baseURL":"https://first.test/v1","apiKey":"PRIVATE"}}}});
        assert!(
            prepare(
                old.clone(),
                &json!({"baseUrl":"https://second.test/v1","model":"x"})
            )
            .is_err()
        );
        assert!(prepare(old, &json!({"baseUrl":"https://first.test/v1","model":"x"})).is_ok());
        let prepared=prepare(json!({"enabled_providers":["other"],"disabled_providers":["aieyes-managed","blocked"]}),&json!({"baseUrl":"https://first.test/v1","model":"x","noKey":true})).unwrap().0;
        assert_eq!(
            prepared["enabled_providers"],
            json!(["other", "aieyes-managed"])
        );
        assert_eq!(prepared["disabled_providers"], json!(["blocked"]));
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let engine = engine(&root);
        let path = root.join("opencode.json");
        std::fs::write(&path, b"// PRIVATE comment\n{}").unwrap();
        let error = engine
            .agent_api_call(
                "agentApi.inspect",
                json!({"sourceId":"open","configPath":path}),
            )
            .unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        assert_eq!(std::fs::read(&path).unwrap(), b"// PRIVATE comment\n{}");
    }
}
