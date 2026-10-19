use super::*;
use toml::{Table, Value as Toml};

fn selected(config: &Toml) -> Option<&Toml> {
    config
        .get("models")?
        .get(config.get("default_model")?.as_str()?)
}

pub(super) fn public(path: &std::path::Path, config: &Toml, revision: &str) -> Value {
    let model = selected(config);
    let provider = model
        .and_then(|m| m.get("provider"))
        .and_then(Toml::as_str)
        .and_then(|id| config.get("providers")?.get(id));
    let key = provider
        .and_then(|p| p.get("api_key"))
        .and_then(Toml::as_str)
        .unwrap_or("");
    json!({"configPath":path,"revision":revision,
        "baseUrl":provider.and_then(|p|p.get("base_url")).and_then(Toml::as_str).unwrap_or(""),
        "model":model.and_then(|m|m.get("model")).and_then(Toml::as_str).unwrap_or(""),
        "contextWindow":model.and_then(|m|m.get("max_context_size")).and_then(Toml::as_integer),
        "hasApiKey":!key.is_empty()&&key!="not-required","noKey":key=="not-required"})
}

pub(super) fn prepare(
    mut config: Toml,
    params: &Value,
) -> Result<(Toml, String, String, String, bool)> {
    // Reuse only our own entry's key; importing the currently selected provider
    // would silently copy credentials belonging to another client account.
    let provider = config.get("providers").and_then(|p| p.get(PROVIDER));
    let (base, model, key, no_key) = connection(
        params,
        provider
            .and_then(|p| p.get("base_url"))
            .and_then(Toml::as_str)
            .unwrap_or(""),
        provider
            .and_then(|p| p.get("api_key"))
            .and_then(Toml::as_str)
            .unwrap_or(""),
    )?;
    let context = params["contextWindow"]
        .as_u64()
        .context("请输入模型上下文窗口大小")?;
    ensure!(
        (1024..=16_777_216).contains(&context),
        "上下文窗口应为 1024 至 16777216 Token"
    );
    let root = config.as_table_mut().context("原生配置应为 TOML 表")?;
    for name in ["providers", "models"] {
        let table = root
            .entry(name)
            .or_insert_with(|| Toml::Table(Table::new()))
            .as_table_mut()
            .context("原生模型配置格式不支持")?;
        if let Some(old) = table.get(PROVIDER) {
            let ours = if name == "providers" {
                old.get("type").and_then(Toml::as_str) == Some("openai")
            } else {
                old.get("provider").and_then(Toml::as_str) == Some(PROVIDER)
            };
            ensure!(ours, "原生配置已使用 aieyes-managed 标识；未覆盖");
        }
    }
    root.get_mut("providers")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert(
            PROVIDER.into(),
            Toml::try_from(json!({"type":"openai","base_url":base,"api_key":key}))?,
        );
    root.get_mut("models").unwrap().as_table_mut().unwrap().insert(PROVIDER.into(),Toml::try_from(json!({"provider":PROVIDER,"model":model,"max_context_size":context,"capabilities":["tool_use"]}))?);
    root.insert("default_model".into(), Toml::String(PROVIDER.into()));
    Ok((config, base, model, key, no_key))
}
