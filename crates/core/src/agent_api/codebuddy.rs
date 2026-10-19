//! CodeBuddy resolves custom model capabilities from models.json, separately
//! from settings.json. Keep each model's endpoint and key in one atomic file.
use super::*;

pub(super) struct Models {
    path: PathBuf,
    value: Value,
    pub revision: String,
    original: Option<Vec<u8>>,
}

impl Models {
    pub fn load(settings: &std::path::Path) -> Result<Self> {
        let path = settings.with_file_name("models.json");
        files::safe(&path)?;
        let original = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 1024 * 1024, "CodeBuddy 模型配置文件过大");
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => anyhow::bail!("无法读取 CodeBuddy 模型配置"),
        };
        let revision = original
            .as_ref()
            .map_or_else(|| "missing".into(), |b| digest(b));
        let value = match &original {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|_| anyhow::anyhow!("CodeBuddy 模型配置不是有效 JSON；未覆盖配置"))?,
            None => json!({"models":[]}),
        };
        ensure!(
            value.is_array() || value.is_object(),
            "CodeBuddy 模型配置格式不支持"
        );
        ensure!(
            value.is_array() || value["models"].is_null() || value["models"].is_array(),
            "CodeBuddy models 配置应为列表"
        );
        ensure!(
            value.is_array()
                || value["availableModels"].is_null()
                || value["availableModels"]
                    .as_array()
                    .is_some_and(|v| v.iter().all(Value::is_string)),
            "CodeBuddy availableModels 配置应为模型名称列表"
        );
        Ok(Self {
            path,
            value,
            revision,
            original,
        })
    }

    pub fn annotate(&self, mut public: Value) -> Value {
        public["nativeRevision"] = json!(self.revision);
        public
    }

    pub fn prepare(&self, base: &str, model: &str, key: &str, effort: &str) -> Result<Value> {
        let mut value = self.value.clone();
        if value.is_object() && value["models"].is_null() {
            value["models"] = json!([]);
        }
        let models = if value.is_array() {
            value.as_array_mut().unwrap()
        } else {
            value["models"].as_array_mut().unwrap()
        };
        ensure!(
            !models
                .iter()
                .any(|m| m["id"] == model && m["aieyesManaged"] != true),
            "CodeBuddy 已存在同名自定义模型；请使用其他模型或先在客户端调整，未覆盖配置"
        );
        let removed: Vec<String> = models
            .iter()
            .filter(|m| m["aieyesManaged"] == true)
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        let mut managed = models
            .iter()
            .find(|m| m["id"] == model && m["aieyesManaged"] == true)
            .cloned()
            .unwrap_or_else(|| json!({}));
        ensure!(
            managed["reasoning"].is_null() || managed["reasoning"].is_object(),
            "CodeBuddy 模型推理配置格式不支持"
        );
        let fields = json!({"id":model,"name":"Aieyes API","aieyesManaged":true,"url":format!("{base}/chat/completions"),"apiKey":key,"supportsReasoning":effort!="disabled"});
        managed
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        if managed["supportsToolCall"].is_null() {
            managed["supportsToolCall"] = json!(true);
        }
        managed["reasoning"]["defaultEffort"] =
            json!(if effort == "disabled" { "low" } else { effort });
        managed["reasoning"]["supportedEfforts"] =
            json!(["minimal", "low", "medium", "high", "xhigh", "max"]);
        managed["reasoning"]["canDisableThinking"] = json!(true);
        models.retain(|m| m["aieyesManaged"] != true);
        models.push(managed);
        if let Some(available) = value
            .get_mut("availableModels")
            .and_then(Value::as_array_mut)
            .filter(|v| !v.is_empty())
        {
            available.retain(|v| !removed.iter().any(|id| v == id));
            if !available.iter().any(|v| v == model) {
                available.push(json!(model));
            }
        }
        Ok(value)
    }

    pub fn check(&self, settings: &std::path::Path) -> Result<()> {
        ensure!(
            Self::load(settings)?.revision == self.revision,
            "CodeBuddy 模型配置已改变，请重新读取后保存"
        );
        Ok(())
    }

    pub fn save(
        &self,
        settings: &std::path::Path,
        settings_bytes: &[u8],
        next: &Value,
    ) -> Result<Option<PathBuf>> {
        let backup = if let Some(original) = &self.original {
            let dir = self.path.parent().unwrap().join(".aieyes/api-backups");
            files::private_dir(&dir)?;
            let path = dir.join(format!("codebuddy-models-{}.json", files::new_id()));
            files::atomic_bytes(&path, original)?;
            Some(path)
        } else {
            None
        };
        let bytes = serde_json::to_vec(next)?;
        files::atomic_bytes(&self.path, &bytes)?;
        if let Err(error) = files::atomic_bytes(settings, settings_bytes) {
            // A native watcher may have rewritten the catalog. Never roll over
            // its changes, including encrypted credentials.
            if Self::load(settings).is_ok_and(|v| v.revision == digest(&bytes)) {
                if let Some(original) = &self.original {
                    files::atomic_bytes(&self.path, original)?;
                } else {
                    std::fs::remove_file(&self.path)?;
                }
            }
            return Err(error);
        }
        Ok(backup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_foreign_models_restrictions_and_exact_private_backup() {
        for array in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let settings = temp.path().canonicalize().unwrap().join("settings.json");
            let other = json!({"id":"other","apiKey":"PRIVATE_OTHER","extra":42});
            let owned = json!({"id":"previous","aieyesManaged":true});
            let original = if array {
                json!([other, owned])
            } else {
                json!({"models":[other,owned],"availableModels":["other","previous"],"extra":{"keep":true}})
            };
            let bytes = serde_json::to_vec_pretty(&original).unwrap();
            files::atomic_bytes(&settings.with_file_name("models.json"), &bytes).unwrap();
            let catalog = Models::load(&settings).unwrap();
            let next = catalog
                .prepare("https://fixture.test/v1", "qwen", "PRIVATE_NEW", "minimal")
                .unwrap();
            let entries = if array {
                next.as_array().unwrap()
            } else {
                next["models"].as_array().unwrap()
            };
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0], other);
            assert_eq!(
                entries[1]["url"],
                "https://fixture.test/v1/chat/completions"
            );
            assert_eq!(entries[1]["reasoning"]["defaultEffort"], "minimal");
            assert!(!catalog.annotate(json!({})).to_string().contains("PRIVATE"));
            if !array {
                assert_eq!(next["availableModels"], json!(["other", "qwen"]));
                assert_eq!(next["extra"], original["extra"]);
            }
            let backup = catalog.save(&settings, b"{}", &next).unwrap().unwrap();
            assert_eq!(std::fs::read(backup).unwrap(), bytes);
            assert!(catalog.check(&settings).is_err());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(settings.with_file_name("models.json"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn refuses_foreign_name_collisions_and_invalid_catalogs() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().canonicalize().unwrap().join("settings.json");
        let path = settings.with_file_name("models.json");
        files::atomic(&path, &json!({"models":[{"id":"qwen","apiKey":"PRIVATE"}]})).unwrap();
        let error = Models::load(&settings)
            .unwrap()
            .prepare("https://fixture.test/v1", "qwen", "key", "low")
            .unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        for invalid in [
            json!({"models":{}}),
            json!({"availableModels":[5]}),
            json!(false),
        ] {
            files::atomic(&path, &invalid).unwrap();
            assert!(Models::load(&settings).is_err());
        }
    }

    #[test]
    fn catalog_rolls_back_when_settings_cannot_be_replaced() {
        for existing in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let settings = temp.path().canonicalize().unwrap().join("settings.json");
            std::fs::create_dir(&settings).unwrap();
            let path = settings.with_file_name("models.json");
            let original = b"{\"models\":[]}\n";
            if existing {
                files::atomic_bytes(&path, original).unwrap();
            }
            let catalog = Models::load(&settings).unwrap();
            let next = catalog
                .prepare("https://fixture.test/v1", "qwen", "PRIVATE", "low")
                .unwrap();
            assert!(catalog.save(&settings, b"{}", &next).is_err());
            if existing {
                assert_eq!(std::fs::read(path).unwrap(), original);
            } else {
                assert!(!path.exists());
            }
        }
    }
}
