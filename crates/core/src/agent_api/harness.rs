use super::*;
use serde_yaml_ng::Value as Yaml;

const LLM: &str = "llm-pi-ai";
const DEFAULT: &str = "agent-default-model";
fn static_plugins(tree: &Yaml) -> Result<()> {
    ensure!(
        tree.as_sequence()
            .is_some_and(|rows| rows.iter().all(|row| matches!(row, Yaml::Mapping(_))
                && matches!(row["id"], Yaml::String(_) | Yaml::Null))),
        "Harness 插件列表或标识使用动态标签，无法确认模型覆盖顺序；请先在客户端整理为静态配置"
    );
    Ok(())
}
fn entry<'a>(tree: &'a Yaml, id: &str) -> Option<&'a Yaml> {
    tree.as_sequence()?
        .iter()
        .rev()
        .find(|v| v["id"].as_str() == Some(id))
}
fn selected(tree: &Yaml) -> Option<&Yaml> {
    let config = &entry(tree, LLM)?["config"];
    config["providers"].get(PROVIDER)
}
fn key(route: &Yaml) -> &str {
    route["headers"]["Authorization"]
        .as_str()
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
}
pub(super) fn public(path: &std::path::Path, tree: &Yaml, revision: &str) -> Value {
    let route = selected(tree).unwrap_or(&Yaml::Null);
    let model = route["models"]
        .as_sequence()
        .and_then(|v| v.first())
        .unwrap_or(&Yaml::Null);
    let secret = key(route);
    json!({"configPath":path,"revision":revision,"baseUrl":route["baseURL"].as_str().unwrap_or(""),"model":model["id"].as_str().unwrap_or(""),
        "contextWindow":model["contextWindow"].as_u64(),"maxOutputTokens":model["maxTokens"].as_u64(),"hasApiKey":!secret.is_empty()&&secret!="not-required","noKey":secret=="not-required"})
}

pub(super) struct Native {
    tree: Yaml,
    pub revision: String,
}
impl Native {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        ensure!(
            path.file_name().is_some_and(|n| n == "cordis.patch.yml"),
            "请选择 Harness Profile 的 cordis.patch.yml"
        );
        let profile_dir = path.parent().context("Profile 目录无效")?;
        let profiles = profile_dir.parent().context("Profile 目录无效")?;
        ensure!(
            profiles.file_name().is_some_and(|n| n == "profiles"),
            "配置路径应位于 DSH_HOME/profiles/<Profile>/cordis.patch.yml"
        );
        let home = profiles.parent().context("Harness 目录无效")?;
        let profile = profile_dir
            .file_name()
            .and_then(|n| n.to_str())
            .context("Profile 名称无效")?;
        ensure!(
            !profile.starts_with('-')
                && profile.len() <= 80
                && profile
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Profile 名称不支持"
        );
        files::safe(&profile_dir.join("package.json"))?;
        ensure!(
            profile_dir.join("package.json").is_file(),
            "请先在 Harness 中安装或初始化此 Profile"
        );
        // The home layer is applied AFTER the profile. Never claim the saved
        // profile is effective when a later layer owns these same plugins.
        let global = home.join("cordis.patch.yml");
        files::safe(&global)?;
        if global.exists() {
            let (Document::Yaml(value), _, _) = read_document(&global)? else {
                anyhow::bail!("全局 Harness 配置不可用")
            };
            static_plugins(&value)?;
            fn conflicts(value: &Yaml) -> bool {
                match value {
                    Yaml::Sequence(rows) => rows.iter().any(conflicts),
                    Yaml::Mapping(map) => {
                        matches!(
                            map.get(Yaml::String("id".into())).and_then(Yaml::as_str),
                            Some(LLM | DEFAULT)
                        ) || matches!(
                            map.get(Yaml::String("name".into())).and_then(Yaml::as_str),
                            Some(
                                "@deepseek-ai/dsh-llm-pi-ai"
                                    | "@deepseek-ai/dsh-agent-default-model"
                            )
                        ) || map.values().any(conflicts)
                    }
                    _ => false,
                }
            }
            ensure!(
                !conflicts(&value),
                "Harness 全局补丁覆盖了模型配置，请先在 Harness 中整理全局与 Profile 的模型设置"
            );
        }
        let mut command = crate::process::cli_command("dsh");
        command
            .args(["--profile", profile, "--dump-config"])
            .env("DSH_HOME", home);
        let bytes =
            crate::process::run(command, vec![], Duration::from_secs(20)).map_err(|_| {
                anyhow::anyhow!("无法读取 Harness 原生合成配置，请检查 dsh 程序和 Profile")
            })?;
        ensure!(bytes.len() <= 2 * 1024 * 1024, "Harness 合成配置过大");
        let tree = harness_yaml::parse(&bytes)?;
        static_plugins(&tree)?;
        for (id, name) in [
            (LLM, "@deepseek-ai/dsh-llm-pi-ai"),
            (DEFAULT, "@deepseek-ai/dsh-agent-default-model"),
        ] {
            ensure!(
                entry(&tree, id).is_some_and(|v| v["name"].as_str() == Some(name)),
                "此 Profile 未提供受支持的 {id} 插件"
            );
        }
        Ok(Self {
            tree,
            revision: digest(&bytes),
        })
    }
    pub fn public(&self, path: &std::path::Path, revision: &str) -> Value {
        let mut value = public(path, &self.tree, revision);
        value["nativeRevision"] = json!(self.revision);
        value
    }
    pub fn prepare(
        &self,
        mut patches: Yaml,
        params: &Value,
    ) -> Result<(Yaml, String, String, String, bool)> {
        let old = selected(&patches).unwrap_or(&Yaml::Null);
        if params["apiKey"].as_str().is_none_or(str::is_empty) && params["noKey"] != true {
            ensure!(
                !matches!(old["headers"]["Authorization"], Yaml::Tagged(_)),
                "已有 Key 使用 Harness 动态标签，请重新输入实际 Key"
            );
        }
        let (base, model, key, no_key) =
            connection(params, old["baseURL"].as_str().unwrap_or(""), key(old))?;
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
        let mut llm = entry(&self.tree, LLM).unwrap()["config"].clone();
        if llm.is_null() {
            llm = Yaml::Mapping(Default::default());
        }
        ensure!(
            matches!(llm, Yaml::Mapping(_)),
            "llm-pi-ai config 使用动态表达式；请先在 Harness 中整理为静态配置"
        );
        if llm["providers"].is_null() {
            llm["providers"] = Yaml::Mapping(Default::default());
        }
        ensure!(
            matches!(llm["providers"], Yaml::Mapping(_)),
            "Provider 列表使用动态表达式，无法安全合并"
        );
        if !llm["providers"][PROVIDER].is_null() {
            ensure!(
                llm["providers"][PROVIDER]["displayName"].as_str() == Some("Aieyes API"),
                "原生配置已使用 aieyes-managed 标识；未覆盖"
            );
        }
        // Custom OpenAI routes accept Authorization headers without requiring
        // apiKeyEnv. This avoids mutating a second credential document.
        llm["providers"][PROVIDER] = serde_yaml_ng::to_value(
            json!({"displayName":"Aieyes API","api":"openai-completions","baseURL":base,"headers":{"Authorization":format!("Bearer {key}")},"models":[{"id":model,"contextWindow":context,"maxTokens":output}]}),
        )?;
        let mut default = entry(&self.tree, DEFAULT).unwrap()["config"].clone();
        if default.is_null() {
            default = Yaml::Mapping(Default::default());
        }
        ensure!(
            matches!(default, Yaml::Mapping(_)),
            "默认模型 config 使用动态表达式，无法安全合并"
        );
        default["provider"] = Yaml::String(PROVIDER.into());
        default["model"] = Yaml::String(model.clone());
        let rows = patches.as_sequence_mut().context("Harness 补丁应为列表")?;
        for (id, config) in [(LLM, llm), (DEFAULT, default)] {
            if let Some(row) = rows.iter_mut().rev().find(|v| v["id"].as_str() == Some(id)) {
                row["config"] = config;
                row["disabled"] = Yaml::Bool(false);
            } else {
                let mut row = serde_yaml_ng::to_value(json!({"id":id,"disabled":false}))?;
                row["config"] = config;
                rows.push(row);
            }
        }
        Ok((patches, base, model, key, no_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dynamic_plugin_identity_is_rejected_but_unrelated_config_expressions_are_preserved() {
        for input in [
            b"- !!js ({id:'llm-pi-ai'})\n".as_slice(),
            b"- id: !!js 'modelPluginId'\n".as_slice(),
        ] {
            assert!(static_plugins(&harness_yaml::parse(input).unwrap()).is_err());
        }
        let tree = harness_yaml::parse(
            b"- id: session-persistence-jsonl\n  config: {root: !!js dshHomePath('sessions')}\n",
        )
        .unwrap();
        static_plugins(&tree).unwrap();
        assert!(matches!(&tree[0]["config"]["root"], Yaml::Tagged(_)));
    }
    #[test]
    fn merged_native_config_retains_other_routes_tags_and_session_persistence() {
        let tree:Yaml=serde_yaml_ng::from_str("- id: llm-pi-ai\n  name: '@deepseek-ai/dsh-llm-pi-ai'\n  config:\n    providers:\n      existing:\n        baseURL: https://existing.test/v1\n        headers: {Authorization: 'Bearer PRIVATE_OTHER'}\n- id: agent-default-model\n  name: '@deepseek-ai/dsh-agent-default-model'\n  config: {temperature: 0.5}\n").unwrap();
        let old = harness_yaml::parse(
            b"- id: session-persistence-jsonl\n  config:\n    root: !!js dshHomePath('sessions')\n",
        )
        .unwrap();
        let native = Native {
            tree,
            revision: "revision".into(),
        };
        let (next,_,_,_,_)=native.prepare(old.clone(),&json!({"baseUrl":"https://custom.test/v1","model":"custom","contextWindow":261000,"maxOutputTokens":2048,"noKey":true})).unwrap();
        assert_eq!(
            entry(&next, "session-persistence-jsonl"),
            entry(&old, "session-persistence-jsonl")
        );
        assert_eq!(entry(&next,LLM).unwrap()["config"]["providers"]["existing"]["headers"]["Authorization"].as_str(),Some("Bearer PRIVATE_OTHER"));
        assert_eq!(
            entry(&next, DEFAULT).unwrap()["config"]["temperature"].as_f64(),
            Some(0.5)
        );
        assert!(matches!(
            &entry(&next, "session-persistence-jsonl").unwrap()["config"]["root"],
            Yaml::Tagged(_)
        ));
        let bytes = harness_yaml::encode(&next).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("!!js"));
        let roundtrip = harness_yaml::parse(&bytes).unwrap();
        assert_eq!(roundtrip, next);
        let public = public(std::path::Path::new("cordis.patch.yml"), &next, "revision");
        assert!(!public.to_string().contains("PRIVATE_OTHER"));
        assert_eq!(public["maxOutputTokens"], 2048);
    }
}
