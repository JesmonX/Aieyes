use super::*;
use toml::{Table, Value as Toml};

pub(super) fn public(path: &std::path::Path, config: &Toml, revision: &str) -> Value {
    let model = config.get("model").and_then(|m| m.get(PROVIDER));
    let string = |field: &str| {
        model
            .and_then(|m| m.get(field))
            .and_then(Toml::as_str)
            .unwrap_or("")
    };
    let key = string("api_key");
    json!({"configPath":path,"revision":revision,"baseUrl":string("base_url"),"model":string("model"),
        "contextWindow":model.and_then(|m|m.get("context_window")).and_then(Toml::as_integer),
        "maxOutputTokens":model.and_then(|m|m.get("max_completion_tokens")).and_then(Toml::as_integer),
        "hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required"})
}
pub(super) fn prepare(
    mut config: Toml,
    params: &Value,
) -> Result<(Toml, String, String, String, bool)> {
    let old = config.get("model").and_then(|m| m.get(PROVIDER));
    if let Some(old) = old {
        ensure!(
            old.get("name").and_then(Toml::as_str) == Some("Aieyes API"),
            "原生配置已使用 aieyes-managed 标识；未覆盖"
        );
    }
    let (base, model, key, no_key) = connection(
        params,
        old.and_then(|m| m.get("base_url"))
            .and_then(Toml::as_str)
            .unwrap_or(""),
        old.and_then(|m| m.get("api_key"))
            .and_then(Toml::as_str)
            .unwrap_or(""),
    )?;
    ensure!(
        !key.contains('$'),
        "Grok 会展开 Key 中的 $ 变量；请在原生客户端使用 env_key 配置此类 Key"
    );
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
    let root = config.as_table_mut().context("原生配置应为 TOML 表")?;
    for field in ["model", "models"] {
        ensure!(
            root.entry(field)
                .or_insert_with(|| Toml::Table(Table::new()))
                .is_table(),
            "Grok 模型配置格式不支持"
        );
    }
    root.get_mut("model").unwrap().as_table_mut().unwrap().insert(PROVIDER.into(),Toml::try_from(json!({"name":"Aieyes API","model":model,"base_url":base,"api_key":key,"api_backend":"chat_completions","context_window":context,"max_completion_tokens":output,"supports_backend_search":false}))?);
    root.get_mut("models")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("default".into(), Toml::String(PROVIDER.into()));
    // Native title generation otherwise keeps its built-in grok-* model ID
    // while reusing the custom endpoint, producing 404s on compatible APIs.
    root.get_mut("models")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .entry("session_summary")
        .or_insert_with(|| Toml::String(PROVIDER.into()));
    Ok((config, base, model, key, no_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_api_model_keeps_other_models_and_global_preferences() {
        let old:Toml=toml::from_str("[models]\ndefault='other'\nweb_search='other'\n[model.other]\napi_key='PRIVATE_OTHER'\nmodel='other-model'\n[ui]\ntheme='dark'\n").unwrap();
        let params = json!({"baseUrl":"https://fixture.test/v1","model":"qwen","contextWindow":261000,"maxOutputTokens":2048,"noKey":true});
        let (next, _, _, _, _) = prepare(old.clone(), &params).unwrap();
        assert_eq!(next["model"]["other"], old["model"]["other"]);
        assert_eq!(next["ui"], old["ui"]);
        assert_eq!(next["models"]["web_search"].as_str(), Some("other"));
        assert_eq!(next["models"]["default"].as_str(), Some(PROVIDER));
        assert_eq!(next["models"]["session_summary"].as_str(), Some(PROVIDER));
        assert_eq!(
            next["model"][PROVIDER]["api_key"].as_str(),
            Some("not-required")
        );
        assert!(next["model"][PROVIDER].get("env_key").is_none());
        assert!(
            !public(std::path::Path::new("config.toml"), &next, "revision")
                .to_string()
                .contains("PRIVATE_OTHER")
        );
        let mut invalid = params;
        invalid["noKey"] = json!(false);
        invalid["apiKey"] = json!("key-$ENV");
        assert!(prepare(next, &invalid).is_err());
        let mut configured = old;
        configured["models"].as_table_mut().unwrap().insert(
            "session_summary".into(),
            Toml::String("other-summary".into()),
        );
        invalid["noKey"] = json!(true);
        assert_eq!(
            prepare(configured, &invalid).unwrap().0["models"]["session_summary"].as_str(),
            Some("other-summary")
        );
    }
}
