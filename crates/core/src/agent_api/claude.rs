use super::*;

pub(super) fn public(path: &std::path::Path, config: &Value, revision: &str) -> Value {
    let env = &config["env"];
    let token = env["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap_or("");
    let key = if token.is_empty() {
        env["ANTHROPIC_API_KEY"].as_str().unwrap_or("")
    } else {
        token
    };
    json!({"configPath":path,"revision":revision,"baseUrl":env["ANTHROPIC_BASE_URL"].as_str().unwrap_or(""),
        "model":env["ANTHROPIC_MODEL"].as_str().or(config["model"].as_str()).unwrap_or(""),
        "authMode":if token.is_empty(){"api-key"}else{"auth-token"},"hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required",
        "effort":env["CLAUDE_CODE_EFFORT_LEVEL"].as_str().or(config["effortLevel"].as_str()).unwrap_or("")})
}
pub(super) fn prepare(
    mut config: Value,
    params: &Value,
) -> Result<(Value, String, String, String, bool)> {
    ensure!(
        config["env"].is_null() || config["env"].is_object(),
        "原生 env 配置格式不支持"
    );
    ensure!(
        config["apiKeyHelper"].as_str().is_none_or(|s| s.is_empty()),
        "原生配置启用了 apiKeyHelper，请先在 Claude 中调整认证方式"
    );
    for key in [
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    ] {
        ensure!(
            config["env"][key].as_str().is_none_or(|s| s.is_empty()
                || ["0", "false", "no", "off"].contains(&s.to_ascii_lowercase().as_str())),
            "原生配置正在使用云平台路由，请先在 Claude 中调整为 Anthropic 协议"
        );
    }
    ensure!(
        config["env"]["ANTHROPIC_CUSTOM_HEADERS"]
            .as_str()
            .is_none_or(|s| s.is_empty()),
        "原生配置带有自定义请求头，请先在 Claude 中整理后再配置 API"
    );
    let mode = params["authMode"].as_str().unwrap_or("api-key");
    ensure!(["api-key", "auth-token"].contains(&mode), "认证方式不支持");
    let (field, other) = if mode == "auth-token" {
        ("ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY")
    } else {
        ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")
    };
    let mut normalized = params.clone();
    if let Some(base) = params["baseUrl"].as_str() {
        let base = base.trim().trim_end_matches('/');
        normalized["baseUrl"] = json!(base.strip_suffix("/v1").unwrap_or(base));
    }
    let (base, model, key, no_key) = connection(
        &normalized,
        config["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap_or(""),
        config["env"][field].as_str().unwrap_or(""),
    )?;
    config["env"]["ANTHROPIC_BASE_URL"] = json!(base);
    config["env"]["ANTHROPIC_MODEL"] = json!(model);
    config["env"][field] = json!(key);
    config["env"][other] = json!("");
    config["model"] = json!(model);
    let effort = effort(params)?;
    config["env"]["CLAUDE_CODE_EFFORT_LEVEL"] = json!(effort);
    if effort.is_empty() {
        config.as_object_mut().unwrap().remove("effortLevel");
    } else {
        config["effortLevel"] = json!(effort);
    }
    Ok((config, base, model, key, no_key))
}
