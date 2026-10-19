use super::*;
fn rule(config: &Value) -> Option<&Value> {
    config["config"]["providerConfigRules"]["providerRules"]
        .as_array()?
        .iter()
        .find(|v| v["providerId"] == PROVIDER)
}
pub(super) fn public(path: &std::path::Path, config: &Value, revision: &str) -> Value {
    let rule = rule(config).unwrap_or(&Value::Null);
    let key = rule["config"]["access"]["apiKey"].as_str().unwrap_or("");
    let selected = &config["config"]["defaultModelSelection"];
    let own = selected["providerId"] == PROVIDER;
    let model_rule = config["config"]["modelConfigRules"]["providerModelRules"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|v| v["providerId"] == PROVIDER && v["modelId"] == selected["modelId"])
        });
    json!({"configPath":path,"revision":revision,"baseUrl":rule["config"]["api"]["baseUrl"].as_str().unwrap_or(""),"model":if own{selected["modelId"].as_str().unwrap_or("")}else{""},
        "hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required","effort":if own{selected["options"]["reasoningLevel"].as_str().unwrap_or("")}else{""},
        "contextWindow":model_rule.and_then(|r|r["config"]["properties"]["contextWindow"].as_u64()),"maxOutputTokens":model_rule.and_then(|r|r["config"]["optionSpecs"]["maxOutputTokens"]["max"].as_u64())})
}
pub(super) fn prepare(
    mut config: Value,
    params: &Value,
) -> Result<(Value, String, String, String, bool)> {
    ensure!(
        config.as_object().is_some_and(|v| v
            .keys()
            .all(|k| ["schemaVersion", "config"].contains(&k.as_str()))),
        "ZCode Provider 文件包含未知顶层字段；未覆盖配置"
    );
    if let Some(native) = config["config"].as_object() {
        ensure!(
            native.keys().all(|k| [
                "providerOrder",
                "providerConfigRules",
                "modelConfigRules",
                "defaultModelSelection"
            ]
            .contains(&k.as_str())),
            "ZCode Provider 配置包含未知字段；未覆盖配置"
        );
    }
    ensure!(
        config["schemaVersion"].is_null() || config["schemaVersion"] == 1,
        "ZCode Provider 配置版本不支持"
    );
    let old = rule(&config).unwrap_or(&Value::Null);
    ensure!(
        old.is_null() || old["providerName"] == "Aieyes API",
        "原生配置已使用 aieyes-managed 标识；未覆盖"
    );
    let (base, model, key, no_key) = connection(
        params,
        old["config"]["api"]["baseUrl"].as_str().unwrap_or(""),
        old["config"]["access"]["apiKey"].as_str().unwrap_or(""),
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
    let effort = effort(params)?;
    for pointer in [
        "/config",
        "/config/providerConfigRules",
        "/config/modelConfigRules",
    ] {
        if config.pointer(pointer).is_none_or(Value::is_null) {
            match pointer {
                "/config" => config["config"] = json!({}),
                "/config/providerConfigRules" => {
                    config["config"]["providerConfigRules"] = json!({})
                }
                _ => config["config"]["modelConfigRules"] = json!({}),
            }
        }
        ensure!(
            config.pointer(pointer).is_some_and(Value::is_object),
            "ZCode Provider 配置格式不支持"
        );
    }
    let native = &mut config["config"];
    for key in ["providerModelRules", "manualProviderModelRules"] {
        let rows = &mut native["modelConfigRules"][key];
        if rows.is_null() {
            *rows = json!([]);
        }
        ensure!(rows.is_array(), "ZCode 模型规则格式不支持");
    }
    ensure!(
        !native["modelConfigRules"]["manualProviderModelRules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["providerId"] == PROVIDER),
        "Aieyes Provider 存在手动模型覆盖，请先在 ZCode 中整理后保存"
    );
    let providers = &mut native["providerConfigRules"]["providerRules"];
    if providers.is_null() {
        *providers = json!([]);
    }
    let providers = providers
        .as_array_mut()
        .context("ZCode Provider 规则格式不支持")?;
    providers.retain(|v| v["providerId"] != PROVIDER);
    providers.push(json!({"providerId":PROVIDER,"providerName":"Aieyes API","config":{"group":"standard-personal","access":{"type":"api-key","apiKey":key},"api":{"type":"openai-chat-completions","baseUrl":base},"personalModelIds":[model],"modelOrder":[model]}}));
    let models = native["modelConfigRules"]["providerModelRules"]
        .as_array_mut()
        .unwrap();
    models.retain(|v| v["providerId"] != PROVIDER);
    models.push(json!({"providerId":PROVIDER,"modelId":model,"config":{"properties":{"contextWindow":context},"optionSpecs":{"reasoningLevel":{"values":["low","medium","high","xhigh","max"],"map":if effort.is_empty(){"{}"}else{"{\"reasoning_effort\": reasoningLevel}"}},"maxOutputTokens":{"max":output,"map":"{\"max_tokens\": maxOutputTokens}"}}}}));
    // CLI 0.16.9 allows only reasoningLevel in selection options. The output
    // limit belongs to the model optionSpecs; extra fields invalidate the file.
    native["defaultModelSelection"] = json!({"providerId":PROVIDER,"modelId":model,"options":{}});
    if !effort.is_empty() {
        native["defaultModelSelection"]["options"]["reasoningLevel"] = json!(effort);
    }
    config["schemaVersion"] = json!(1);
    Ok((config, base, model, key, no_key))
}
