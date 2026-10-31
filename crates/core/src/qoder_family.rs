//! Product boundaries for the two Qoder clients. Never alias their identities.
pub(crate) struct Spec {
    pub name: &'static str,
    pub binary: &'static str,
    pub root: &'static str,
    pub config_env: &'static str,
    pub app_id: &'static str,
    pub site: &'static str,
}
const GLOBAL: Spec = Spec {
    name: "Qoder",
    binary: "qodercli",
    root: "~/.qoder",
    config_env: "QODER_CONFIG_DIR",
    app_id: "com.qoder.app",
    site: "https://qoder.com",
};
const CN: Spec = Spec {
    name: "Qoder CN",
    binary: "qoderclicn",
    root: "~/.qoder-cn",
    config_env: "QODERCN_CONFIG_DIR",
    app_id: "com.qodercn.app",
    site: "https://qoder.cn",
};
pub(crate) fn is_qoder(provider: &str) -> bool {
    matches!(provider, "qoder" | "qoder-cn")
}
pub(crate) fn spec(provider: &str) -> &'static Spec {
    if provider == "qoder-cn" { &CN } else { &GLOBAL }
}
pub(crate) fn desktop_root(provider: &str) -> std::path::PathBuf {
    let home = crate::store::home();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
    };
    base.join(format!("{}.stable", spec(provider).app_id))
}
pub(crate) fn cli_candidates(provider: &str, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let spec = spec(provider);
    let mut paths = Vec::new();
    if let Some(p) = std::env::var_os(if provider == "qoder-cn" {
        "QODERCN_CLI_PATH"
    } else {
        "QODER_CLI_PATH"
    }) {
        paths.push(p.into());
    }
    let mut dirs: Vec<_> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend([
        crate::store::home().join(".local/bin"),
        crate::store::home().join("bin"),
        "/usr/local/bin".into(),
        "/opt/homebrew/bin".into(),
    ]);
    if cfg!(windows)
        && let Some(p) = std::env::var_os("APPDATA")
    {
        dirs.push(std::path::PathBuf::from(p).join("npm"));
    }
    for dir in dirs {
        for suffix in if cfg!(windows) {
            &[".exe", ".cmd"][..]
        } else {
            &[""][..]
        } {
            paths.push(dir.join(format!("{}{suffix}", spec.binary)));
        }
    }
    let bin = root.join("bin").join(spec.binary);
    paths.push(bin.join(spec.binary));
    if let Ok(entries) = std::fs::read_dir(bin) {
        let mut versions: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        versions.sort();
        versions.reverse();
        paths.extend(versions);
    }
    paths.retain(|p| p.is_file());
    paths
}
pub(crate) fn cli_path(provider: &str) -> Option<std::path::PathBuf> {
    cli_candidates(provider, &crate::store::expand(spec(provider).root))
        .into_iter()
        .next()
}
pub(crate) fn installed_apps(provider: &str) -> Vec<String> {
    let spec = spec(provider);
    if !cfg!(target_os = "macos") {
        return if desktop_root(provider).exists() {
            vec![spec.name.into()]
        } else {
            vec![]
        };
    }
    let roots = [
        std::path::PathBuf::from("/Applications"),
        crate::store::home().join("Applications"),
    ];
    let mut apps: Vec<String> = roots
        .iter()
        .filter_map(|root| {
            let plist = root
                .join(format!("{}.app", spec.name))
                .join("Contents/Info.plist");
            let output = std::process::Command::new("/usr/bin/plutil")
                .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
                .arg(plist)
                .output()
                .ok()?;
            (output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == spec.app_id)
                .then(|| spec.name.into())
        })
        .take(1)
        .collect();
    // Preserve the older global desktop/IDE installation hints.
    if provider == "qoder" {
        for name in ["Qoder IDE", "QoderWork"] {
            if roots
                .iter()
                .any(|root| root.join(format!("{name}.app")).is_dir())
            {
                apps.push(name.into());
            }
        }
    }
    apps
}
pub(crate) fn auth_method(root: &std::path::Path) -> Option<String> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("state.json")).ok()?).ok()?;
    match value["lastLoginMethod"].as_str()? {
        "token" => Some("pat".into()),
        "browser" => Some("browser".into()),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn products_have_distinct_native_roots_and_commands() {
        for (id, binary, env) in [
            ("qoder", "qodercli", "QODER_CONFIG_DIR"),
            ("qoder-cn", "qoderclicn", "QODERCN_CONFIG_DIR"),
        ] {
            let s = spec(id);
            assert_eq!(s.binary, binary);
            assert_eq!(s.config_env, env);
            assert_eq!(crate::contracts::provider(id).unwrap().agent_id, id);
            assert_eq!(crate::contracts::provider(id).unwrap().binary, binary);
            assert_eq!(crate::quota_schedule::floor(id), 600);
        }
        assert_ne!(desktop_root("qoder"), desktop_root("qoder-cn"));
        assert!(!is_qoder("cn-legacy"));
    }
    #[test]
    fn both_native_login_methods_are_metadata_only() {
        let root = tempfile::tempdir().unwrap();
        for (native, expected) in [
            ("token", Some("pat")),
            ("browser", Some("browser")),
            ("other", None),
        ] {
            std::fs::write(
                root.path().join("state.json"),
                serde_json::json!({"lastLoginMethod":native}).to_string(),
            )
            .unwrap();
            assert_eq!(auth_method(root.path()).as_deref(), expected);
        }
    }
}
