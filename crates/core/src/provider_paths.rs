//! Native client roots used when discovering or creating a local source.
//! Existing sources stay attached to their explicitly stored path.
use anyhow::{Context, Result, ensure};
use std::{ffi::OsString, path::PathBuf};

pub(crate) fn default_path(provider: &str, remote: bool) -> Result<String> {
    resolve(provider, remote, |key| std::env::var_os(key))
}

/// Work is an installed desktop form of Codex, not a separate account or CLI.
pub(crate) fn chatgpt_work_installed() -> bool {
    cfg!(target_os = "macos")
        && chatgpt_work_installed_in(&[
            PathBuf::from("/Applications"),
            crate::store::home().join("Applications"),
        ])
}

fn chatgpt_work_installed_in(roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| root.join("ChatGPT.app").is_dir())
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
    if provider == "roo" && (remote || !cfg!(target_os = "macos")) {
        return Ok(roo_root(remote, &mut env));
    }
    if provider == "cline"
        && !remote
        && env("CLINE_DATA_DIR").is_none_or(|p| p.is_empty())
        && let Some(config) = env("CLINE_DIR").filter(|p| !p.is_empty())
    {
        let root = PathBuf::from(config);
        ensure!(
            root.is_absolute(),
            "CLINE_DIR 应为绝对目录；请设置有效路径或手动选择采集目录"
        );
        return Ok(root.join("data").to_string_lossy().into_owned());
    }
    if provider == "kilo"
        && !remote
        && let Some(database) = env("KILO_DB").filter(|p| !p.is_empty())
    {
        let database = PathBuf::from(database);
        if database.is_absolute() {
            return Ok(database.to_string_lossy().into_owned());
        }
        let data = env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::store::home().join(".local/share"));
        ensure!(
            data.is_absolute(),
            "XDG_DATA_HOME 应为绝对目录；请设置有效路径或手动选择采集目录"
        );
        return Ok(data
            .join("kilo")
            .join(database)
            .to_string_lossy()
            .into_owned());
    }
    // Trae keeps VS Code's user-data layout under its product name: the
    // contract's macOS folder, or `%APPDATA%\Trae` / `%APPDATA%\Trae CN`.
    if matches!(provider, "trae" | "trae-cn")
        && !remote
        && cfg!(windows)
        && let Some(appdata) = env("APPDATA").filter(|s| !s.is_empty())
    {
        return Ok(PathBuf::from(appdata)
            .join(spec.name)
            .to_string_lossy()
            .into_owned());
    }
    let variable = match provider {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "kimi" => Some("KIMI_CODE_HOME"),
        "pi" | "omp" => Some("PI_CODING_AGENT_DIR"),
        "deepseek-harness" => Some("DSH_HOME"),
        "codebuddy" => Some("CODEBUDDY_CONFIG_DIR"),
        "qoder" => Some("QODER_CONFIG_DIR"),
        "qoder-cn" => Some("QODERCN_CONFIG_DIR"),
        "grok" => Some("GROK_HOME"),
        "opencode" | "kilo" => Some("XDG_DATA_HOME"),
        "copilot" => Some("COPILOT_HOME"),
        "cline" => Some("CLINE_DATA_DIR"),
        // kiro-cli 2.36 keeps sessions and settings under `$KIRO_HOME` (default `~/.kiro`).
        "kiro" => Some("KIRO_HOME"),
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
        if matches!(provider, "opencode" | "kilo") {
            path.push(provider);
        }
        return path
            .to_str()
            .map(str::to_owned)
            .context("原生数据目录编码不支持，请手动选择采集目录");
    }
    Ok(spec.default_path.into())
}

/// The three extensions use VS Code's globalStorage layout. Discover existing
/// editor roots by their on-disk extension ID, including non-default editors.
pub(crate) fn discovery_paths(provider: &str) -> Result<Vec<String>> {
    let mut paths = vec![default_path(provider, false)?];
    if crate::qoder_family::is_qoder(provider) {
        let desktop = crate::qoder_family::desktop_root(provider);
        if desktop.join("main.sqlite").is_file() {
            paths.push(desktop.to_string_lossy().into_owned());
        }
    }
    let extension = match provider {
        "cline" => "saoudrizwan.claude-dev",
        "roo" => "rooveterinaryinc.roo-cline",
        "kilo" => "kilocode.kilo-code",
        _ => return Ok(paths),
    };
    let home = crate::store::home();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
    };
    paths.extend(
        extension_paths(&base, extension)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned()),
    );
    for server in [".vscode-server", ".vscode-server-insiders"] {
        let path = home
            .join(server)
            .join("data/User/globalStorage")
            .join(extension);
        if path.join("tasks").is_dir() {
            paths.push(path.to_string_lossy().into_owned());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn extension_paths(base: &std::path::Path, extension: &str) -> Vec<PathBuf> {
    let mut editors: Vec<_> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    editors.sort();
    editors.truncate(256);
    editors
        .into_iter()
        .map(|p| p.join("User/globalStorage").join(extension))
        .filter(|p| p.join("tasks").is_dir())
        .collect()
}

/// VS Code globalStorage of the Roo Code extension. Remote machines are Linux.
fn roo_root(remote: bool, env: &mut impl FnMut(&str) -> Option<OsString>) -> String {
    const STORAGE: &str = "Code/User/globalStorage/rooveterinaryinc.roo-cline";
    if !remote
        && cfg!(windows)
        && let Some(appdata) = env("APPDATA").filter(|s| !s.is_empty())
    {
        return PathBuf::from(appdata)
            .join(STORAGE)
            .to_string_lossy()
            .into_owned();
    }
    format!("~/.config/{STORAGE}")
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
    fn work_installation_detects_application_bundles_without_a_codex_cli() {
        let temp = tempfile::tempdir().unwrap();
        let roots = [
            temp.path().join("Applications"),
            temp.path().join("user/Applications"),
        ];
        assert!(!chatgpt_work_installed_in(&roots));
        for root in &roots {
            std::fs::create_dir_all(root).unwrap();
            let bundle = root.join("ChatGPT.app");
            std::fs::write(&bundle, "not an application bundle").unwrap();
            assert!(!chatgpt_work_installed_in(&roots));
            std::fs::remove_file(&bundle).unwrap();
            std::fs::create_dir(&bundle).unwrap();
            assert!(chatgpt_work_installed_in(&roots));
            std::fs::remove_dir(&bundle).unwrap();
            assert!(!chatgpt_work_installed_in(&roots));
        }
    }

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
            ("kilo", "XDG_DATA_HOME"),
            ("copilot", "COPILOT_HOME"),
            ("cline", "CLINE_DATA_DIR"),
            ("kiro", "KIRO_HOME"),
        ] {
            let local = resolve(provider, false, |actual| {
                if provider == "kilo" && actual == "KILO_DB" {
                    return None;
                }
                assert_eq!(actual, key);
                Some(root.clone().into_os_string())
            })
            .unwrap();
            assert_eq!(
                PathBuf::from(local),
                if matches!(provider, "opencode" | "kilo") {
                    root.join(provider)
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
    fn roo_storage_follows_the_platform_and_remote_is_linux() {
        let linux = "~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline";
        assert_eq!(resolve("roo", true, |_| None).unwrap(), linux);
        let local = resolve("roo", false, |_| None).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                local,
                crate::contracts::provider("roo").unwrap().default_path
            );
        } else if !cfg!(windows) {
            assert_eq!(local, linux);
        }
    }

    #[test]
    fn trae_user_data_uses_the_product_name() {
        for (provider, name) in [("trae", "Trae"), ("trae-cn", "Trae CN")] {
            let contract = crate::contracts::provider(provider).unwrap().default_path;
            assert_eq!(contract, format!("~/Library/Application Support/{name}"));
            assert_eq!(resolve(provider, true, |_| None).unwrap(), contract);
            let local = resolve(provider, false, |key| {
                assert_eq!(key, "APPDATA");
                Some("C:\\Users\\u\\AppData\\Roaming".into())
            })
            .unwrap();
            if cfg!(windows) {
                assert_eq!(
                    PathBuf::from(local),
                    PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join(name)
                );
            } else {
                assert_eq!(local, contract);
            }
        }
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
                home.join(".other-omp")
                    .join("profiles")
                    .join("work")
                    .join("agent")
            }
            .to_str()
            .unwrap()
        );
        assert_eq!(
            omp_root(false, |k| variables.get(k).cloned(), home).unwrap(),
            home.join(".other-omp")
                .join("profiles")
                .join("work")
                .join("agent")
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

    #[test]
    fn cline_config_root_and_kilo_database_overrides_match_native_storage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let resolve_with = |provider: &str, vars: Vec<(&str, PathBuf)>| {
            resolve(provider, false, |key| {
                vars.iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, path)| path.clone().into_os_string())
            })
        };
        assert_eq!(
            resolve_with("cline", vec![("CLINE_DIR", root.join("config"))]).unwrap(),
            root.join("config/data").to_string_lossy()
        );
        assert_eq!(
            resolve_with(
                "cline",
                vec![
                    ("CLINE_DIR", root.join("config")),
                    ("CLINE_DATA_DIR", root.join("data"))
                ]
            )
            .unwrap(),
            root.join("data").to_string_lossy()
        );
        assert!(resolve_with("cline", vec![("CLINE_DIR", PathBuf::from("relative"))]).is_err());
        assert_eq!(
            resolve_with("kilo", vec![("KILO_DB", root.join("selected.db"))]).unwrap(),
            root.join("selected.db").to_string_lossy()
        );
        assert_eq!(
            resolve_with(
                "kilo",
                vec![
                    ("KILO_DB", PathBuf::from("channel.db")),
                    ("XDG_DATA_HOME", root.to_owned())
                ]
            )
            .unwrap(),
            root.join("kilo/channel.db").to_string_lossy()
        );
        assert!(
            resolve_with(
                "kilo",
                vec![
                    ("KILO_DB", PathBuf::from("channel.db")),
                    ("XDG_DATA_HOME", PathBuf::from("relative"))
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn discovers_extension_tasks_in_existing_editors_without_login_files() {
        let temp = tempfile::tempdir().unwrap();
        let extension = "saoudrizwan.claude-dev";
        let expected = temp
            .path()
            .join("Nondefault Editor/User/globalStorage")
            .join(extension);
        std::fs::create_dir_all(expected.join("tasks")).unwrap();
        std::fs::create_dir_all(
            temp.path()
                .join("Other Editor/User/globalStorage")
                .join(extension),
        )
        .unwrap();
        assert_eq!(extension_paths(temp.path(), extension), vec![expected]);
    }
}
