//! Custom providers are created by the installed CLI's BYOK control API.
//! Only the documented default model selection is written by Aieyes itself.
use super::*;

pub(super) fn public(path: &std::path::Path, config: &Value, revision: &str) -> Value {
    let route = &config["providers"][PROVIDER];
    let model = &route["models"][0];
    let key = route["apiKey"].as_str().unwrap_or("");
    json!({"configPath":path,"revision":revision,"baseUrl":route["baseUrl"].as_str().unwrap_or(""),"model":model["model"].as_str().unwrap_or(""),"contextWindow":model["contextWindow"].as_u64(),"maxOutputTokens":model["maxOutputTokens"].as_u64(),"hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required"})
}

pub(super) struct Prepared {
    pub input: Value,
    pub base: String,
    pub model: String,
    pub key: String,
    pub no_key: bool,
    pub existing: bool,
}

pub(super) fn prepare(old: &Value, params: &Value) -> Result<Prepared> {
    ensure!(
        old["providers"].is_null() || old["providers"].is_object(),
        "Qoder Providers 配置格式不支持"
    );
    ensure!(
        old["model"].is_null() || old["model"].is_object(),
        "Qoder 默认模型配置格式不支持"
    );
    let route = &old["providers"][PROVIDER];
    ensure!(
        route.is_null() || route["displayName"] == "Aieyes API",
        "原生配置已使用 aieyes-managed 标识；未覆盖"
    );
    let (base, model, key, no_key) = connection(
        params,
        route["baseUrl"].as_str().unwrap_or(""),
        route["apiKey"].as_str().unwrap_or(""),
    )?;
    let context = params["contextWindow"]
        .as_u64()
        .context("请输入模型上下文窗口大小")?;
    let output = params["maxOutputTokens"]
        .as_u64()
        .context("请输入最大输出 Token 数")?;
    ensure!(
        (1024..=16_777_216).contains(&context) && output > 0 && output <= context,
        "上下文或最大输出 Token 数无效"
    );
    let input = json!({"providerId":PROVIDER,"displayName":"Aieyes API","baseUrl":base,"apiKey":key,"type":"openai-compatible","protocol":"openai","authType":"bearer","models":[{"model":model,"displayName":model,"contextWindow":context,"maxOutputTokens":output,"capabilities":{"tools":true,"vision":false}}]});
    Ok(Prepared {
        input,
        base,
        model,
        key,
        no_key,
        existing: !route.is_null(),
    })
}

fn control(path: &std::path::Path, request: Value, proxy: &ProxyConfig) -> Result<Value> {
    let root = path.parent().context("Qoder 配置目录无效")?;
    let args = [
        "--print",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--no-session-persistence",
        "--setting-sources",
        "user",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--config-dir",
        root.to_str().context("Qoder 配置目录编码无效")?,
    ];
    let mut command = crate::process::cli_command_with_pre_command(
        "qodercli",
        &args,
        if cfg!(unix) { ":" } else { "" },
        "",
    );
    command.current_dir(crate::store::home());
    network::apply_env(&mut command, proxy);
    let mut request = request;
    request["subtype"] = request["type"].clone();
    let input = format!(
        "{}\n{}\n",
        json!({"type":"control_request","request_id":"aieyes-init","request":{"type":"initialize","subtype":"initialize"}}),
        json!({"type":"control_request","request_id":"aieyes-byok","request":request})
    );
    let bytes = crate::process::run(command, input.into_bytes(), Duration::from_secs(45)).map_err(
        |_| {
            anyhow::anyhow!(
                "Qoder CLI API 操作失败；请先在该配置目录完成 qodercli login，并检查网络"
            )
        },
    )?;
    response(&bytes)
}

fn response(bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= 2 * 1024 * 1024, "Qoder API 响应过大");
    let mut supported = false;
    let mut result = None;
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(row) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if row["type"] != "control_response" {
            continue;
        }
        let response = &row["response"];
        if response["request_id"] == "aieyes-init" {
            supported = response["subtype"] == "success"
                && response["response"]["capabilities"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|v| v == "byok_config_management_v1"));
        } else if response["request_id"] == "aieyes-byok" {
            ensure!(
                result.is_none() && response["subtype"] == "success",
                "Qoder 原生 API 操作失败；请检查登录、配置和 CLI 版本后重新读取"
            );
            result = Some(response["response"].clone());
        }
    }
    ensure!(
        supported,
        "当前 Qoder CLI 未确认 BYOK 管理能力；请升级并在该配置目录完成登录"
    );
    result.context("Qoder CLI 未返回 API 操作结果，请检查登录")
}

fn unrelated(mut value: Value) -> Value {
    if let Some(providers) = value.get_mut("providers").and_then(Value::as_object_mut) {
        providers.remove(PROVIDER);
    }
    if value
        .get("providers")
        .is_some_and(|p| p.is_null() || p.as_object().is_some_and(|p| p.is_empty()))
    {
        value.as_object_mut().unwrap().remove("providers");
    }
    value
}

impl Prepared {
    pub fn validate(&self, path: &std::path::Path, proxy: &ProxyConfig) -> Result<()> {
        ensure!(
            control(
                path,
                json!({"type":"check_byok_config","config":self.input}),
                proxy
            )?["success"]
                == true,
            "Qoder 原生模型验证失败，请核对端点、Key 和模型"
        );
        Ok(())
    }
    pub fn save(&self, path: &std::path::Path, old: &Value, proxy: &ProxyConfig) -> Result<Value> {
        control(
            path,
            json!({"type":if self.existing {"update_byok_config"} else {"create_byok_config"},"config":self.input}),
            proxy,
        )?;
        let (Document::Json(mut next), _, _) = read_document(path)? else {
            anyhow::bail!("Qoder 原生保存结果无效，请从备份恢复或重新读取")
        };
        // The CLI owns credential storage. Reject unrelated concurrent changes
        // rather than replacing them with an older settings snapshot.
        ensure!(
            unrelated(next.clone()) == unrelated(old.clone()),
            "Qoder Provider 已保存，但其他配置同时改变；默认模型未更新，请重新读取"
        );
        let route = &next["providers"][PROVIDER];
        ensure!(
            route["baseUrl"] == self.base
                && route["apiKey"] == self.key
                && route["models"][0]["model"] == self.model,
            "Qoder 原生保存结果与请求不一致，请重新读取"
        );
        next["model"]["name"] = json!(format!("{PROVIDER}/{}", self.model));
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creating_the_first_provider_accepts_absent_or_null_provider_maps_but_not_other_changes() {
        let old = json!({"permissions":{"bash":"ask"}});
        let next =
            json!({"permissions":{"bash":"ask"},"providers":{PROVIDER:{"apiKey":"private"}}});
        assert_eq!(unrelated(old.clone()), unrelated(next.clone()));
        let mut null = old.clone();
        null["providers"] = Value::Null;
        assert_eq!(unrelated(null), unrelated(next.clone()));
        let mut changed = next;
        changed["permissions"]["bash"] = json!("allow");
        assert_ne!(unrelated(old), unrelated(changed));
    }
    #[test]
    fn managed_input_never_adopts_another_providers_key_and_public_data_is_secret_free() {
        let old = json!({"model":{"name":"other/old","preferences":{"effort":"low"}},"providers":{"other":{"apiKey":"PRIVATE_OTHER"}},"permissions":{"bash":"ask"}});
        let params = json!({"baseUrl":"https://model.test/v1","model":"qwen","noKey":true,"contextWindow":261000,"maxOutputTokens":2048});
        let prepared = prepare(&old, &params).unwrap();
        assert_eq!(prepared.key, "not-required");
        assert!(!prepared.existing);
        assert_eq!(old["providers"]["other"]["apiKey"], "PRIVATE_OTHER");
        assert!(
            !public(std::path::Path::new("settings.json"), &old, "revision")
                .to_string()
                .contains("PRIVATE_OTHER")
        );
        let mut collision = old;
        collision["providers"][PROVIDER] = json!({"displayName":"Someone else"});
        assert!(prepare(&collision, &params).is_err());
    }
    #[test]
    fn unsupported_or_failed_control_messages_do_not_expose_errors_or_accept_unrelated_results() {
        assert!(response(b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-byok\",\"subtype\":\"success\",\"response\":{}}}\n").is_err());
        let error=response(b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-byok\",\"subtype\":\"error\",\"error\":\"PRIVATE_KEY\"}}\n").unwrap_err().to_string();
        assert!(!error.contains("PRIVATE_KEY"));
    }
}
