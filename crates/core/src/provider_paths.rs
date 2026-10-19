//! Native client roots used when discovering or creating a local source.
//! Existing sources stay attached to their explicitly stored path.
use anyhow::{Context, Result, ensure};
use std::{ffi::OsString, path::PathBuf};

pub(crate) fn default_path(provider: &str, remote: bool) -> Result<String> {
    resolve(provider, remote, |key| std::env::var_os(key))
}

fn resolve(
    provider: &str,
    remote: bool,
    mut env: impl FnMut(&str) -> Option<OsString>,
) -> Result<String> {
    let spec = crate::contracts::provider(provider).context("不支持的 Agent")?;
    if provider == "omp" && !remote {
        return omp_root(true, env, &crate::store::home());
    }
    let variable = match provider {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "kimi" => Some("KIMI_CODE_HOME"),
        "pi" | "omp" => Some("PI_CODING_AGENT_DIR"),
        "deepseek-harness" => Some("DSH_HOME"),
        "codebuddy" => Some("CODEBUDDY_CONFIG_DIR"),
        "qoder" => Some("QODER_CONFIG_DIR"),
        "grok" => Some("GROK_HOME"),
        "opencode" => Some("XDG_DATA_HOME"),
        _ => None,
    };
    if !remote
        && let Some(variable) = variable
        && let Some(value) = env(variable).filter(|s| !s.is_empty())
    {
        let mut path = PathBuf::from(value);
        ensure!(
            path.is_absolute(),
            "{variable} 应为绝对目录；请设置有效路径或手动选择采集目录"
        );
        if provider == "opencode" {
            path.push("opencode");
        }
        return path
            .to_str()
            .map(str::to_owned)
            .context("原生数据目录编码不支持，请手动选择采集目录");
    }
    Ok(spec.default_path.into())
}

pub(crate) fn omp_config_dir() -> Result<PathBuf> {
    Ok(crate::store::expand(&omp_root(
        false,
        |key| std::env::var_os(key),
        &crate::store::home(),
    )?))
}

fn omp_root(
    data: bool,
    mut env: impl FnMut(&str) -> Option<OsString>,
    home: &std::path::Path,
) -> Result<String> {
    let profile = env("OMP_PROFILE")
        .or_else(|| env("PI_PROFILE"))
        .unwrap_or_default();
    let profile = profile.to_str().context("OMP Profile 编码无效")?.trim();
    let profile = if profile == "default" { "" } else { profile };
    if !profile.is_empty() {
        let reserved = profile.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            profile.len() <= 64 && profile.as_bytes()[0].is_ascii_lowercase()
                || profile.len() <= 64 && profile.as_bytes()[0].is_ascii_digit(),
            "OMP Profile 名称无效"
        );
        ensure!(
            profile
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
                && !profile.ends_with('.')
                && !["CON", "PRN", "AUX", "NUL"].contains(&reserved.as_str())
                && !(reserved.len() == 4
                    && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
                    && reserved.as_bytes()[3].is_ascii_digit()),
            "OMP Profile 名称无效"
        );
    }
    let config = env("PI_CONFIG_DIR")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".omp".into());
    let config = PathBuf::from(config);
    ensure!(
        !config.is_absolute()
            && !config
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
        "PI_CONFIG_DIR 应为用户主目录内的相对配置目录"
    );
    let root = if profile.is_empty() {
        home.join(&config)
    } else {
        home.join(&config).join("profiles").join(profile)
    };
    let default = root.join("agent");
    let agent = if profile.is_empty() {
        env("PI_CODING_AGENT_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| default.clone())
    } else {
        default.clone()
    };
    ensure!(
        agent.is_absolute(),
        "PI_CODING_AGENT_DIR 应为绝对目录；请设置有效路径或手动选择采集目录"
    );
    if data
        && agent == default
        && cfg!(any(target_os = "linux", target_os = "macos"))
        && let Some(xdg) = env("XDG_DATA_HOME").filter(|s| !s.is_empty())
    {
        let xdg = PathBuf::from(xdg);
        ensure!(xdg.is_absolute(), "XDG_DATA_HOME 应为绝对目录");
        let app = if profile.is_empty() {
            xdg.join("omp")
        } else {
            xdg.join("omp/profiles").join(profile)
        };
        if app.is_dir() {
            return app
                .to_str()
                .map(str::to_owned)
                .context("OMP 数据目录编码无效");
        }
    }
    if agent == home.join(".omp/agent") {
        return Ok("~/.omp/agent".into());
    }
    agent
        .to_str()
        .map(str::to_owned)
        .context("OMP 配置目录编码无效")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_overrides_match_client_roots_and_never_change_ssh_or_codex_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("配置目录");
        for (provider, key) in [
            ("claude", "CLAUDE_CONFIG_DIR"),
            ("kimi", "KIMI_CODE_HOME"),
            ("pi", "PI_CODING_AGENT_DIR"),
            ("deepseek-harness", "DSH_HOME"),
            ("codebuddy", "CODEBUDDY_CONFIG_DIR"),
            ("qoder", "QODER_CONFIG_DIR"),
            ("grok", "GROK_HOME"),
            ("opencode", "XDG_DATA_HOME"),
        ] {
            let local = resolve(provider, false, |actual| {
                assert_eq!(actual, key);
                Some(root.clone().into_os_string())
            })
            .unwrap();
            assert_eq!(
                PathBuf::from(local),
                if provider == "opencode" {
                    root.join("opencode")
                } else {
                    root.clone()
                }
            );
            assert_eq!(
                resolve(provider, true, |_| panic!(
                    "SSH must not read local environment"
                ))
                .unwrap(),
                crate::contracts::provider(provider).unwrap().default_path
            );
        }
        assert_eq!(
            resolve("codex", false, |_| panic!("Codex scope is unchanged")).unwrap(),
            "~/.codex"
        );
        assert_eq!(
            resolve("zcode", false, |_| panic!(
                "ZCODE_DATA_BASE_DIR relocates provider config, not CLI usage or hooks"
            ))
            .unwrap(),
            "~/.zcode"
        );
    }

    #[test]
    fn omp_profiles_and_xdg_keep_config_separate_from_native_session_data() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let data = home.join("data");
        std::fs::create_dir_all(data.join("omp/profiles/work")).unwrap();
        let variables = std::collections::HashMap::from([
            ("OMP_PROFILE", OsString::from("work")),
            ("PI_CONFIG_DIR", OsString::from(".other-omp")),
            ("PI_CODING_AGENT_DIR", home.join("ignored").into_os_string()),
            ("XDG_DATA_HOME", data.clone().into_os_string()),
        ]);
        assert_eq!(
            omp_root(true, |k| variables.get(k).cloned(), home).unwrap(),
            if cfg!(any(target_os = "linux", target_os = "macos")) {
                data.join("omp/profiles/work")
            } else {
                home.join(".other-omp/profiles/work/agent")
            }
            .to_str()
            .unwrap()
        );
        assert_eq!(
            omp_root(false, |k| variables.get(k).cloned(), home).unwrap(),
            home.join(".other-omp/profiles/work/agent")
                .to_str()
                .unwrap()
        );
        assert_eq!(
            omp_root(
                true,
                |k| (k == "PI_CODING_AGENT_DIR").then(|| home.join("custom").into_os_string()),
                home
            )
            .unwrap(),
            home.join("custom").to_str().unwrap()
        );
        assert!(
            omp_root(
                true,
                |k| (k == "OMP_PROFILE").then(|| "../escape".into()),
                home
            )
            .is_err()
        );
    }
    #[test]
    fn invalid_relative_roots_do_not_silently_read_another_default_profile() {
        assert!(resolve("kimi", false, |_| Some("relative-profile".into())).is_err());
        assert_eq!(
            resolve("kimi", false, |_| Some("".into())).unwrap(),
            "~/.kimi-code"
        );
        assert!(resolve("unknown", false, |_| None).is_err());
    }
}
