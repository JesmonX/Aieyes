//! Infer authentication without launching a CLI, contacting a provider, or retaining secrets.
use std::{path::Path, time::SystemTime};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Usage {
    #[default]
    Account,
    IndependentApi,
    UnknownProfile,
}

fn environment<'a>(env: &'a [u8], key: &str) -> Option<&'a str> {
    env.split(|b| *b == 0).find_map(|entry| {
        let (name, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
        (name == key && !value.trim().is_empty()).then_some(value)
    })
}

fn merge(base: &mut toml::Value, layer: toml::Value) {
    if let (Some(base), Some(layer)) = (base.as_table_mut(), layer.as_table()) {
        for (key, value) in layer {
            if let Some(old) = base.get_mut(key) {
                merge(old, value.clone());
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = layer;
    }
}

fn read_config(path: &Path, started: Option<SystemTime>) -> Option<Option<toml::Value>> {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(None),
        Err(_) => return None,
    };
    let metadata = file.metadata().ok()?;
    // Profiles are launch selections; don't infer from a changed profile file.
    // Base settings are also rewritten by Codex itself, so callers may read their
    // current values without treating unrelated edits as an unknown profile.
    if metadata.len() > 1024 * 1024
        || started.is_some_and(|time| {
            metadata
                .modified()
                .ok()
                .is_none_or(|modified| modified > time + std::time::Duration::from_secs(1))
        })
    {
        return None;
    }
    let mut text = String::new();
    file.by_ref()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() > 1024 * 1024 {
        return None;
    }
    toml::from_str(&text).ok().map(Some)
}

fn override_value(config: &mut toml::Value, raw: &str) -> Option<()> {
    let (key, value) = raw.split_once('=')?;
    // Parse dotted/quoted TOML keys independently, then replace the exact target.
    let path: toml::Value = toml::from_str(&format!("{} = 0", key.trim())).ok()?;
    let mut keys = Vec::new();
    let mut cursor = &path;
    while let Some(table) = cursor.as_table() {
        if table.len() != 1 {
            return None;
        }
        let (key, value) = table.iter().next()?;
        keys.push(key.clone());
        cursor = value;
    }
    let value = toml::from_str::<toml::Value>(&format!("value = {}", value.trim()))
        .ok()
        .and_then(|mut v| v.as_table_mut()?.remove("value"))
        .unwrap_or_else(|| toml::Value::String(value.trim().into()));
    let (last, parents) = keys.split_last()?;
    let mut cursor = config;
    for key in parents {
        if !cursor.is_table() {
            *cursor = toml::Value::Table(Default::default());
        }
        cursor = cursor
            .as_table_mut()?
            .entry(key.clone())
            .or_insert_with(|| toml::Value::Table(Default::default()));
    }
    if !cursor.is_table() {
        *cursor = toml::Value::Table(Default::default());
    }
    cursor.as_table_mut()?.insert(last.clone(), value);
    Some(())
}

fn classify(config: &toml::Value, env: &[u8]) -> Usage {
    let provider = config
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    // A URL override, an unrelated env key, or provider definitions that aren't
    // selected do not establish independence from the signed-in account.
    if provider == "openai" {
        return Usage::Account;
    }
    let Some(provider) = config
        .get("model_providers")
        .and_then(|v| v.get(provider))
        .and_then(toml::Value::as_table)
    else {
        return Usage::UnknownProfile;
    };
    if provider
        .get("requires_openai_auth")
        .and_then(toml::Value::as_bool)
        == Some(true)
    {
        return Usage::Account;
    }
    if provider
        .get("requires_openai_auth")
        .is_some_and(|v| !v.is_bool())
    {
        return Usage::UnknownProfile;
    }
    let endpoint = provider
        .get("base_url")
        .and_then(toml::Value::as_str)
        .and_then(|s| reqwest::Url::parse(s).ok())
        .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    let token = provider
        .get("experimental_bearer_token")
        .and_then(toml::Value::as_str)
        .is_some_and(|v| !v.trim().is_empty());
    let key = provider
        .get("env_key")
        .and_then(toml::Value::as_str)
        .and_then(|key| environment(env, key))
        .is_some();
    if endpoint && (token || key) {
        Usage::IndependentApi
    } else {
        Usage::UnknownProfile
    }
}

pub(super) fn inspect(args: &[String], env: &[u8], started: Option<SystemTime>) -> Usage {
    inspect_with_system(args, env, started, Path::new("/etc/codex/config.toml"))
}

fn inspect_with_system(
    args: &[String],
    env: &[u8],
    started: Option<SystemTime>,
    system: &Path,
) -> Usage {
    let mut profile = None;
    let mut overrides = Vec::new();
    let mut index = 1;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if arg == "--profile" || arg == "-p" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Usage::UnknownProfile;
            };
            profile = Some(value.as_str());
        } else if let Some(value) = arg
            .strip_prefix("--profile=")
            .or_else(|| arg.strip_prefix("-p").filter(|s| !s.is_empty()))
        {
            profile = Some(value.strip_prefix('=').unwrap_or(value));
        } else if arg == "--config" || arg == "-c" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Usage::UnknownProfile;
            };
            overrides.push(value.as_str());
        } else if let Some(value) = arg
            .strip_prefix("--config=")
            .or_else(|| arg.strip_prefix("-c").filter(|s| !s.is_empty()))
        {
            overrides.push(value.strip_prefix('=').unwrap_or(value));
        }
        index += 1;
    }
    let fallback = if profile.is_some() {
        Usage::UnknownProfile
    } else {
        Usage::Account
    };
    let Some(home) = environment(env, "CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| environment(env, "HOME").map(|s| Path::new(s).join(".codex")))
    else {
        return fallback;
    };
    if !home.is_absolute() {
        return fallback;
    }
    let mut config = toml::Value::Table(Default::default());
    let mut incomplete_base = false;
    for path in [system.to_path_buf(), home.join("config.toml")] {
        let Some(layer) = read_config(&path, None) else {
            // A stable profile may prove auth even when a base file is unreadable,
            // but only if it explicitly overrides every authentication choice.
            incomplete_base = true;
            config = toml::Value::Table(Default::default());
            continue;
        };
        if let Some(layer) = layer {
            merge(&mut config, layer);
        }
    }
    let mut alternatives = Vec::new();
    if let Some(profile) = profile {
        if profile.is_empty()
            || !profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Usage::UnknownProfile;
        }
        let Some(file) = read_config(&home.join(format!("{profile}.config.toml")), started) else {
            return Usage::UnknownProfile;
        };
        let legacy = config.get("profiles").and_then(|v| v.get(profile)).cloned();
        if file.is_none() && legacy.is_none() {
            return Usage::UnknownProfile;
        }
        // If both generations of profile format exist, only exclude when both
        // prove independent auth. Don't guess the running CLI's version.
        for layer in [file, legacy].into_iter().flatten() {
            let mut alternative = config.clone();
            merge(&mut alternative, layer);
            alternatives.push(alternative);
        }
    } else {
        alternatives.push(config);
    }
    let mut answer = None;
    for mut config in alternatives {
        for raw in &overrides {
            if override_value(&mut config, raw).is_none() {
                return fallback;
            }
        }
        let usage = classify(&config, env);
        // App-server hosts can inject credentials at runtime. Without an explicit
        // profile, lack of an independent credential is not a reason to exempt or
        // block a normal app process from the existing close-and-switch flow.
        if usage == Usage::UnknownProfile && profile.is_none() {
            return fallback;
        }
        if incomplete_base {
            let selected = config.get("model_provider").and_then(toml::Value::as_str);
            let explicit_independent = selected
                .and_then(|id| {
                    config
                        .get("model_providers")?
                        .get(id)?
                        .get("requires_openai_auth")
                })
                .and_then(toml::Value::as_bool)
                == Some(false);
            if !(usage == Usage::IndependentApi && explicit_independent
                || selected == Some("openai"))
            {
                return fallback;
            }
        }
        if answer.is_some_and(|old| old != usage) {
            return Usage::UnknownProfile;
        }
        answer = Some(usage);
    }
    answer.unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn shared_profile_and_override_cases() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../scripts/fixtures/codex-process-auth.json"
        ))
        .unwrap();
        for case in cases {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().canonicalize().unwrap();
            std::fs::write(home.join("config.toml"), case["config"].as_str().unwrap()).unwrap();
            if case["changedBase"] == true {
                std::fs::File::open(home.join("config.toml"))
                    .unwrap()
                    .set_times(
                        std::fs::FileTimes::new()
                            .set_modified(SystemTime::now() + std::time::Duration::from_secs(5)),
                    )
                    .unwrap();
            }
            for (name, text) in case["profiles"].as_object().unwrap() {
                std::fs::write(
                    home.join(format!("{name}.config.toml")),
                    text.as_str().unwrap(),
                )
                .unwrap();
            }
            let mut env = format!("CODEX_HOME={}\0", home.display());
            for (key, value) in case["env"].as_object().unwrap() {
                env.push_str(&format!("{key}={}\0", value.as_str().unwrap()));
            }
            let args: Vec<String> = serde_json::from_value(case["args"].clone()).unwrap();
            let result = inspect_with_system(
                &args,
                env.as_bytes(),
                Some(SystemTime::now()),
                &home.join("system.toml"),
            );
            let actual = match result {
                Usage::Account => "account",
                Usage::IndependentApi => "independentApi",
                Usage::UnknownProfile => "unknownProfile",
            };
            assert_eq!(
                actual,
                case["expected"].as_str().unwrap(),
                "case {}",
                case["name"]
            );
        }
    }

    #[test]
    fn changed_profile_cannot_prove_running_authentication() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::write(home.join("api.config.toml"), "model_provider='gateway'\n[model_providers.gateway]\nbase_url='https://fixture.invalid/v1'\nexperimental_bearer_token='FIXTURE_SECRET'\nrequires_openai_auth=false").unwrap();
        let args = vec!["codex".into(), "--profile".into(), "api".into()];
        let env = format!("CODEX_HOME={}\0", home.display());
        assert!(
            inspect_with_system(
                &args,
                env.as_bytes(),
                Some(SystemTime::UNIX_EPOCH),
                &home.join("system.toml")
            ) == Usage::UnknownProfile
        );
    }
}
