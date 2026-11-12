//! 本次更新: the bundled release notes of the running version and whether this launch follows
//! an update (`app.launch`, read once at startup).
use serde_json::{Value, json};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// docs/release-notes/v<version>.md, staged by build.rs; empty when this version has none.
const NOTES: &str = include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"));

pub struct WhatsNew(Mutex<Value>);

fn notes() -> Option<&'static str> {
    Some(NOTES).filter(|text| !text.trim().is_empty())
}

/// The compare link that closes the notes: a line `[text](https://…)`.
fn changelog(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find_map(|line| {
        let url = line
            .strip_prefix('[')?
            .strip_suffix(')')?
            .rsplit_once("](")?
            .1;
        (url.starts_with("https://") && !url.contains(char::is_whitespace)).then_some(url)
    })
}

/// Records this launch once; a failed read shows no banner.
pub fn setup(app: &AppHandle, launch: Result<Value, String>) {
    let launch = launch.unwrap_or_else(|_| json!({"updated": false}));
    app.manage(WhatsNew(Mutex::new(launch)));
}

#[tauri::command]
pub fn whats_new_info(app: AppHandle) -> Value {
    let launch = app
        .try_state::<WhatsNew>()
        .and_then(|state| state.0.lock().ok().map(|v| v.clone()))
        .unwrap_or_else(|| json!({"updated": false}));
    json!({
        "version": app.package_info().version.to_string(),
        "previousVersion": launch["previousVersion"],
        "updated": launch["updated"] == true,
        "notes": notes(),
        "changelog": notes().and_then(changelog),
    })
}

/// Opens the bundled changelog link in the default browser; no URL is accepted from the page.
#[tauri::command]
pub fn whats_new_open_changelog() -> Result<(), String> {
    let url = notes()
        .and_then(changelog)
        .ok_or("此版本没有变更记录链接")?;
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开浏览器：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changelog_link_is_the_closing_https_link() {
        let text = "# Title\n\n- item\n\n[Full changelog / 完整代码变更：v0.2.4 → v0.2.5](https://github.com/JesmonX/Aieyes/compare/v0.2.4...v0.2.5)\n";
        assert_eq!(
            changelog(text),
            Some("https://github.com/JesmonX/Aieyes/compare/v0.2.4...v0.2.5")
        );
        assert_eq!(changelog("[x](javascript:alert(1))"), None);
        assert_eq!(changelog("[x](file:///etc/passwd)"), None);
        assert_eq!(changelog("- [x](https://a.example) item"), None);
    }

    #[test]
    fn this_version_bundles_its_notes() {
        let path = format!(
            "{}/../../../docs/release-notes/v{}.md",
            env!("CARGO_MANIFEST_DIR"),
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            notes().is_some(),
            std::path::Path::new(&path).exists(),
            "build.rs stages the running version's notes"
        );
    }
}
