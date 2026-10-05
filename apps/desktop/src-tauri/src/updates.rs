use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FEED: &str = "https://github.com/JesmonX/Aieyes/releases/latest/download/latest.json";
const PUBLIC_KEY: &str = match option_env!("AIEYES_UPDATER_PUBLIC_KEY") {
    Some(key) => key,
    None => "",
};
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Preferences {
    automatic: bool,
    last_check: u64,
    deferred_version: String,
    defer_until: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            last_check: 0,
            deferred_version: String::new(),
            defer_until: 0,
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    current_version: String,
    latest_version: Option<String>,
    notes: String,
    phase: String,
    message: String,
    downloaded: u64,
    total: Option<u64>,
    automatic: bool,
    prompt: bool,
}
impl Status {
    fn busy(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "checking" | "downloading" | "verifying" | "installing"
        )
    }
}
struct Inner {
    status: Status,
    preferences: Preferences,
    update: Option<Update>,
    target: String,
}
pub struct Updates {
    inner: Mutex<Inner>,
    path: PathBuf,
    pub installing: AtomicBool,
}
fn save(state: &Updates, prefs: &Preferences) -> Result<(), String> {
    use std::io::Write;
    let parent = state.path.parent().ok_or("更新配置路径无效")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(prefs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.persist(&state.path).map_err(|e| e.to_string())?;
    Ok(())
}
fn change(app: &AppHandle, apply: impl FnOnce(&mut Inner)) -> Status {
    let service = app.state::<Updates>();
    let mut inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
    apply(&mut inner);
    let status = inner.status.clone();
    drop(inner);
    let _ = app.emit("updates:status", &status);
    status
}
fn require_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("请在主窗口中管理更新".into())
    }
}
fn target() -> Result<String, String> {
    if std::env::consts::ARCH != "x86_64" {
        return Err("当前架构暂无更新安装包".into());
    }
    if cfg!(windows) {
        return Ok("windows-x86_64-nsis".into());
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("APPIMAGE").is_some() {
            return Ok("linux-x86_64-appimage".into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let owned = std::process::Command::new("dpkg-query")
            .arg("-S")
            .arg(exe)
            .output()
            .map_err(|_| "无法识别当前安装格式")?;
        if owned.status.success()
            && String::from_utf8_lossy(&owned.stdout)
                .lines()
                .any(|line| line.starts_with("aieyes:") || line.starts_with("aieyes-desktop:"))
        {
            return Ok("linux-x86_64-deb".into());
        }
    }
    Err("当前不是受支持的已安装应用，请安装正式安装包后更新".into())
}
fn valid_download(url: &tauri::Url, version: &str, target: &str) -> bool {
    let suffix = match target {
        "windows-x86_64-nsis" => "windows-x64.exe",
        "linux-x86_64-appimage" => "linux-x64.AppImage",
        "linux-x86_64-deb" => "linux-x64.deb",
        _ => return false,
    };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path()
            == format!("/JesmonX/Aieyes/releases/download/v{version}/Aieyes-{version}-{suffix}")
}
fn version_message(current: &str, latest: &str) -> Result<Option<String>, String> {
    let current = semver::Version::parse(current).map_err(|_| "当前应用版本无效")?;
    let latest =
        semver::Version::parse(latest.trim_start_matches('v')).map_err(|_| "更新版本无效")?;
    if !latest.pre.is_empty() {
        return Err("稳定更新源返回了预发布版本".into());
    }
    Ok(if latest == current {
        Some(format!("当前已是最新版本 v{current}"))
    } else if latest < current {
        Some(format!("当前版本 v{current} 高于最新稳定版 v{latest}"))
    } else {
        None
    })
}
pub fn setup(app: &AppHandle, root: &std::path::Path) {
    let path = root.join("updates.json");
    let preferences: Preferences = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let status = Status {
        current_version: app.package_info().version.to_string(),
        latest_version: None,
        notes: String::new(),
        phase: "idle".into(),
        message: String::new(),
        downloaded: 0,
        total: None,
        automatic: preferences.automatic,
        prompt: false,
    };
    app.manage(Updates {
        inner: Mutex::new(Inner {
            status,
            preferences,
            update: None,
            target: String::new(),
        }),
        path,
        installing: AtomicBool::new(false),
    });
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        let mut startup = true;
        loop {
            let due = {
                let service = app.state::<Updates>();
                let inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.preferences.automatic
                    && !inner.status.busy()
                    && (startup || now().saturating_sub(inner.preferences.last_check) >= 86400)
            };
            startup = false;
            if due {
                let _ = tauri::async_runtime::block_on(check(&app, false));
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}
async fn check(app: &AppHandle, manual: bool) -> Result<Status, String> {
    {
        let service = app.state::<Updates>();
        let mut inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.busy() {
            return Ok(inner.status.clone());
        }
        inner.status.phase = "checking".into();
        inner.status.prompt = false;
        inner.status.message = "正在检查更新…".into();
        inner.preferences.last_check = now();
        if let Err(error) = save(&service, &inner.preferences) {
            inner.status.phase = "error".into();
            inner.status.message = error.clone();
            return Err(error);
        }
    }
    change(app, |_| {});
    let result: Result<_, String> = async {
        if PUBLIC_KEY.is_empty() {
            return Err("此开发构建未配置更新签名公钥，请安装正式发布版本".into());
        }
        let target = target()?;
        let exit_app = app.clone();
        let mut builder = app
            .updater_builder()
            .on_before_exit(move || crate::desktop::save(&exit_app))
            .pubkey(PUBLIC_KEY)
            .target(&target)
            .endpoints(vec![FEED.parse().map_err(|_| "更新源地址无效")?])
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(30))
            .version_comparator(|_, _| true);
        let app_for_proxy = app.clone();
        let proxy = tauri::async_runtime::spawn_blocking(move || {
            let engines = app_for_proxy.state::<crate::Shared>();
            let mut engine = engines.0.lock().map_err(|_| "核心不可用")?;
            let value = engine
                .call("settings.get", serde_json::json!({}))
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(value["proxy"].clone())
        })
        .await
        .map_err(|e| e.to_string())??;
        match proxy["mode"].as_str() {
            Some("direct") => builder = builder.no_proxy(),
            Some("custom") => {
                builder = builder.proxy(
                    proxy["url"]
                        .as_str()
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| "更新代理地址无效")?,
                )
            }
            _ => {}
        }
        let update = builder
            .build()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("更新源未返回版本信息")?;
        let message = version_message(&app.package_info().version.to_string(), &update.version)?;
        if !valid_download(&update.download_url, &update.version, &target) {
            return Err("更新包地址与版本、平台不匹配".into());
        }
        Ok((update, target, message))
    }
    .await;
    Ok(change(app, |inner| match result {
        Ok((update, target, message)) => {
            inner.status.latest_version = Some(update.version.clone());
            inner.status.notes = update.body.clone().unwrap_or_default();
            inner.status.phase = if message.is_some() {
                "current"
            } else {
                "available"
            }
            .into();
            inner.status.message =
                message.unwrap_or_else(|| format!("发现新版本 v{}", update.version));
            inner.status.prompt = inner.status.phase == "available"
                && (manual
                    || inner.preferences.deferred_version != update.version
                    || now() >= inner.preferences.defer_until);
            inner.update = if inner.status.phase == "available" {
                Some(update)
            } else {
                None
            };
            inner.target = target;
        }
        Err(error) => {
            inner.status.phase = "error".into();
            inner.status.message = error;
            inner.update = None;
        }
    }))
}
#[tauri::command]
pub fn updates_info(app: AppHandle) -> Status {
    app.state::<Updates>()
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status
        .clone()
}
#[tauri::command]
pub async fn updates_check(app: AppHandle, window: tauri::WebviewWindow) -> Result<Status, String> {
    require_main(&window)?;
    check(&app, true).await
}
#[tauri::command]
pub fn updates_preferences(
    app: AppHandle,
    window: tauri::WebviewWindow,
    automatic: bool,
) -> Result<Status, String> {
    require_main(&window)?;
    {
        let service = app.state::<Updates>();
        let mut inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut preferences = inner.preferences.clone();
        preferences.automatic = automatic;
        save(&service, &preferences)?;
        inner.preferences = preferences;
    }
    Ok(change(&app, |inner| inner.status.automatic = automatic))
}
#[tauri::command]
pub fn updates_later(app: AppHandle, window: tauri::WebviewWindow) -> Result<Status, String> {
    require_main(&window)?;
    {
        let service = app.state::<Updates>();
        let mut inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.busy() {
            return Err("更新正在进行".into());
        }
        let mut preferences = inner.preferences.clone();
        preferences.deferred_version = inner.status.latest_version.clone().unwrap_or_default();
        preferences.defer_until = now() + 86400;
        save(&service, &preferences)?;
        inner.preferences = preferences;
    }
    Ok(change(&app, |inner| inner.status.prompt = false))
}
#[tauri::command]
pub async fn updates_install(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Status, String> {
    require_main(&window)?;
    let (mut update, target) = {
        let service = app.state::<Updates>();
        let mut inner = service.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.status.busy() {
            return Err("更新正在进行".into());
        }
        let update = inner.update.clone().ok_or("请先检查更新")?;
        inner.status.phase = "downloading".into();
        inner.status.message = "正在下载更新…".into();
        inner.status.prompt = false;
        inner.status.downloaded = 0;
        inner.status.total = None;
        (update, inner.target.clone())
    };
    update.timeout = Some(Duration::from_secs(900));
    change(&app, |_| {});
    let result: Result<(), String> = async {
        let bytes = update
            .download(
                |length, total| {
                    change(&app, |inner| {
                        inner.status.downloaded += length as u64;
                        inner.status.total = total;
                    });
                },
                || {
                    change(&app, |inner| {
                        inner.status.phase = "verifying".into();
                        inner.status.message = "正在校验更新包…".into();
                    });
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        app.state::<Updates>()
            .installing
            .store(true, Ordering::SeqCst);
        change(&app, |inner| {
            inner.status.phase = "installing".into();
            inner.status.message = if target.ends_with("-deb") {
                "请完成系统授权，正在安装更新…"
            } else {
                "正在安装并重启…"
            }
            .into();
        });
        let installing_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            // Wait for existing mutations to finish; engine_call refuses new ones until installation ends.
            let engines = installing_app.state::<crate::Shared>();
            let _core = engines.0.lock().map_err(|_| "核心不可用")?;
            let _metrics = engines.1.lock().map_err(|_| "采样核心不可用")?;
            #[cfg(target_os = "linux")]
            if target.ends_with("-deb") {
                return install_deb(&bytes);
            }
            update.install(bytes).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(())
    }
    .await;
    app.state::<Updates>()
        .installing
        .store(false, Ordering::SeqCst);
    match result {
        Ok(()) => {
            change(&app, |inner| {
                inner.status.phase = "installed".into();
                inner.status.message = "更新已安装，正在重启…".into();
            });
            app.restart()
        }
        Err(error) => Ok(change(&app, |inner| {
            inner.status.phase = "error".into();
            inner.status.message = format!("更新未完成：{error}");
        })),
    }
}
#[cfg(target_os = "linux")]
fn install_deb(bytes: &[u8]) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::PermissionsExt, process::Command};
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    // apt's unprivileged _apt worker must be able to read this verified public installer.
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    let path = directory.path().join("Aieyes.deb");
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .map_err(|e| e.to_string())?;
    let output = Command::new("/usr/bin/pkexec")
        .args(["/usr/bin/apt-get", "install", "--yes", "--no-remove", "--"])
        .arg(&path)
        .output()
        .map_err(|_| "系统授权组件 pkexec 不可用，请安装 polkit 后重试")?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(126 | 127) => Err("系统授权已取消或未获授权".into()),
        _ => Err(format!(
            "系统包管理器安装失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_versions_do_not_downgrade_or_compare_as_text() {
        assert!(version_message("1.9.0", "v1.10.0").unwrap().is_none());
        assert!(
            version_message("1.0.0", "1.0.0")
                .unwrap()
                .unwrap()
                .contains("最新")
        );
        assert!(
            version_message("2.0.0", "1.0.0")
                .unwrap()
                .unwrap()
                .contains("高于")
        );
        assert!(version_message("1.0.0", "garbage").is_err());
        assert!(version_message("1.0.0", "2.0.0-beta.1").is_err());
    }
    #[test]
    fn artifacts_must_match_repository_version_and_installer() {
        let url: tauri::Url =
            "https://github.com/JesmonX/Aieyes/releases/download/v1.2.3/Aieyes-1.2.3-linux-x64.deb"
                .parse()
                .unwrap();
        assert!(valid_download(&url, "1.2.3", "linux-x86_64-deb"));
        assert!(!valid_download(&url, "1.2.4", "linux-x86_64-deb"));
        assert!(!valid_download(&url, "1.2.3", "linux-x86_64-appimage"));
        assert!(!valid_download(
            &"https://evil.example/package.deb".parse().unwrap(),
            "1.2.3",
            "linux-x86_64-deb"
        ));
    }
    #[test]
    fn old_preferences_enable_daily_checks() {
        let prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert!(prefs.automatic);
        assert_eq!(prefs.defer_until, 0);
    }
}
