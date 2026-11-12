//! System integration on Windows and Linux: launch at login and `aieyes://` links.
//!
//! Launch at login is a per-user entry: the HKCU `Run` value on Windows and an XDG autostart
//! file on Linux. Links are registered per user at startup (HKCU `Software\Classes\aieyes`
//! on Windows, an `x-scheme-handler/aieyes` desktop entry on Linux) and reach the running
//! instance through the single-instance plugin. Any app or web page can open a URL scheme,
//! so a link only navigates: it names one of a fixed set of destinations, and query
//! strings, fragments and every other part are ignored.
use serde_json::{Value, json};
use tauri::AppHandle;

/// Settings › General page keys; the same list and order as `generalPageKeys` in
/// web/settings-index.js (scripts/test-experience-ui.cjs compares them).
pub(crate) const GENERAL_PAGES: [&str; 10] = [
    "appearance",
    "panel",
    "desktop",
    "system",
    "network",
    "refresh",
    "notifications",
    "webhooks",
    "advanced",
    "updates",
];
const PAGES: [&str; 4] = ["agent", "analysis", "servers", "sessions"];
/// Link names for settings categories, mapped to the web settings tabs.
const CATEGORIES: [(&str, &str); 5] = [
    ("integrations", "integrations"),
    ("accounts", "integrations"),
    ("machines", "hosts"),
    ("prices", "prices"),
    ("wakeups", "wakeups"),
];
/// The links shown in Settings › 启动与系统.
pub(crate) const LINKS: [&str; 6] = [
    "aieyes://panel",
    "aieyes://overview",
    "aieyes://analysis",
    "aieyes://servers",
    "aieyes://sessions",
    "aieyes://settings",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Link {
    Panel,
    Page(&'static str),
    Settings {
        category: &'static str,
        section: Option<&'static str>,
    },
}

/// Parses `aieyes://<destination>[/<name>]`; `open/` is accepted as a prefix.
pub(crate) fn parse_link(raw: &str) -> Option<Link> {
    let (scheme, rest) = raw.trim().split_once(':')?;
    if !scheme.eq_ignore_ascii_case("aieyes") {
        return None;
    }
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<String> = path
        .split('/')
        .map(str::to_ascii_lowercase)
        .filter(|part| !part.is_empty() && part != "open")
        .collect();
    let first = parts.first()?;
    let name = parts.get(1).map(String::as_str);
    match first.as_str() {
        "panel" => Some(Link::Panel),
        "settings" => Some(settings_link(name)),
        "overview" => Some(Link::Page("agent")),
        page => PAGES.iter().copied().find(|p| *p == page).map(Link::Page),
    }
}

fn settings_link(name: Option<&str>) -> Link {
    if let Some(&(_, category)) = CATEGORIES.iter().find(|entry| Some(entry.0) == name) {
        return Link::Settings {
            category,
            section: None,
        };
    }
    Link::Settings {
        category: "general",
        section: GENERAL_PAGES.iter().copied().find(|key| Some(*key) == name),
    }
}

/// The first `aieyes:` argument, as Windows and the desktop entry pass the link.
pub(crate) fn link_in<I: IntoIterator<Item = String>>(args: I) -> Option<Link> {
    args.into_iter()
        .filter(|arg| {
            arg.get(..7)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("aieyes:"))
        })
        .find_map(|arg| parse_link(&arg))
}

/// The settings target the web app opens for a settings link.
fn settings_target(category: &str, section: Option<&str>) -> Value {
    match section {
        Some(section) => json!({"category": category, "section": section}),
        None => json!({"category": category}),
    }
}

pub(crate) fn open_link(app: &AppHandle, link: &Link) -> Result<(), String> {
    match link {
        Link::Panel => crate::desktop::action(app, "focus-panel")
            .or_else(|_| crate::desktop::activate_main(app)),
        Link::Page(page) => crate::desktop::show_main(app, page),
        Link::Settings { category, section } => crate::desktop::show_main(
            app,
            &format!("settings-target:{}", settings_target(category, *section)),
        ),
    }
}

/// The program a login entry or link handler starts. An AppImage runs from a temporary mount,
/// so its stable path is `$APPIMAGE`.
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
fn program() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(image) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return Ok(image.into());
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Quotes a path for a desktop entry `Exec` key (Desktop Entry Specification, "The Exec key").
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn exec_quote(path: &str) -> String {
    let mut quoted = String::from('"');
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        if c == '%' {
            quoted.push('%');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn autostart_entry(program: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Aieyes\nComment=Agent usage and SSH server monitor\nExec={}\nIcon=aieyes-desktop\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_quote(program)
    )
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn link_handler_entry(program: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Aieyes\nExec={} %u\nIcon=aieyes-desktop\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/aieyes;\n",
        exec_quote(program)
    )
}

/// An autostart file counts as enabled unless it is hidden or switched off.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn autostart_entry_enabled(text: &str) -> bool {
    !text.lines().map(str::trim).any(|line| {
        line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    fn xdg(variable: &str, fallback: &str) -> Result<PathBuf, String> {
        if let Some(dir) = std::env::var_os(variable).filter(|v| !v.is_empty()) {
            return Ok(dir.into());
        }
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(fallback))
            .ok_or_else(|| "无法确定主目录".to_string())
    }
    fn autostart_file() -> Result<PathBuf, String> {
        Ok(xdg("XDG_CONFIG_HOME", ".config")?
            .join("autostart")
            .join("aieyes.desktop"))
    }
    fn write(path: &PathBuf, text: &str) -> Result<bool, String> {
        if std::fs::read_to_string(path).is_ok_and(|current| current == text) {
            return Ok(false);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        Ok(true)
    }
    pub(super) fn autostart() -> Result<bool, String> {
        match std::fs::read_to_string(autostart_file()?) {
            Ok(text) => Ok(super::autostart_entry_enabled(&text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }
    pub(super) fn set_autostart(enabled: bool) -> Result<(), String> {
        let path = autostart_file()?;
        if enabled {
            let program = super::program()?;
            write(&path, &super::autostart_entry(&program.to_string_lossy())).map(|_| ())
        } else {
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(error.to_string())
                }
                _ => Ok(()),
            }
        }
    }
    /// Keeps an enabled entry pointing at the current program after an update or a move.
    pub(super) fn refresh_autostart() {
        if autostart().unwrap_or(false) {
            let _ = set_autostart(true);
        }
    }
    pub(super) fn register_links() -> Result<(), String> {
        let program = super::program()?;
        let path = xdg("XDG_DATA_HOME", ".local/share")?
            .join("applications")
            .join("aieyes-url-handler.desktop");
        if write(
            &path,
            &super::link_handler_entry(&program.to_string_lossy()),
        )? {
            let _ = std::process::Command::new("xdg-mime")
                .args([
                    "default",
                    "aieyes-url-handler.desktop",
                    "x-scheme-handler/aieyes",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::{PCWSTR, w};

    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

    fn read(key: PCWSTR, name: PCWSTR) -> Result<Option<String>, String> {
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key,
                name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(format!("{status:?}"));
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("{status:?}"));
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Ok(Some(String::from_utf16_lossy(&buffer[..end])))
    }
    fn write(key: PCWSTR, name: PCWSTR, value: &str) -> Result<(), String> {
        // REG_SZ data includes its terminating NUL, also for an empty value.
        let value: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let bytes = (value.len() * 2) as u32;
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key,
                name,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                bytes,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("{status:?}"))
        }
    }
    fn command() -> Result<String, String> {
        Ok(format!("\"{}\"", super::program()?.display()))
    }
    pub(super) fn autostart() -> Result<bool, String> {
        read(RUN, w!("Aieyes")).map(|value| value.is_some())
    }
    pub(super) fn set_autostart(enabled: bool) -> Result<(), String> {
        if enabled {
            write(RUN, w!("Aieyes"), &command()?)
        } else {
            let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("Aieyes")) };
            if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(format!("{status:?}"))
            }
        }
    }
    pub(super) fn refresh_autostart() {
        if let (Ok(Some(current)), Ok(expected)) = (read(RUN, w!("Aieyes")), command())
            && current != expected
        {
            let _ = set_autostart(true);
        }
    }
    pub(super) fn register_links() -> Result<(), String> {
        let open = format!("{} \"%1\"", command()?);
        let command_key = w!("Software\\Classes\\aieyes\\shell\\open\\command");
        if read(command_key, PCWSTR::null())?.as_deref() == Some(open.as_str()) {
            return Ok(());
        }
        let root = w!("Software\\Classes\\aieyes");
        write(root, PCWSTR::null(), "URL:Aieyes")?;
        write(root, w!("URL Protocol"), "")?;
        write(command_key, PCWSTR::null(), &open)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    pub(super) fn autostart() -> Result<bool, String> {
        Err("此系统不支持登录时启动".into())
    }
    pub(super) fn set_autostart(_: bool) -> Result<(), String> {
        Err("此系统不支持登录时启动".into())
    }
    pub(super) fn refresh_autostart() {}
    pub(super) fn register_links() -> Result<(), String> {
        Ok(())
    }
}

/// Startup: register links, refresh an enabled login entry, then follow a link the app was
/// started with (the main window reads the page it was sent to on load).
pub(crate) fn setup(app: &AppHandle) {
    if let Err(error) = platform::register_links() {
        eprintln!("Could not register aieyes:// links: {error}");
    }
    platform::refresh_autostart();
    if let Some(link) = link_in(std::env::args().skip(1)) {
        let _ = open_link(app, &link);
    }
}

/// A second launch: a link navigates, anything else brings the main window forward.
pub(crate) fn second_instance(app: &AppHandle, args: Vec<String>) {
    match link_in(args.into_iter().skip(1)) {
        Some(link) => {
            if open_link(app, &link).is_err() {
                let _ = crate::desktop::activate_main(app);
            }
        }
        None => {
            let _ = crate::desktop::activate_main(app);
        }
    }
}

fn info() -> Value {
    let available = cfg!(any(target_os = "windows", target_os = "linux"));
    let (enabled, error) = match platform::autostart() {
        Ok(enabled) => (enabled, None),
        Err(error) => (false, available.then_some(error)),
    };
    json!({
        "autostart": {"available": available, "enabled": enabled, "error": error},
        "links": LINKS,
    })
}

#[tauri::command]
pub fn system_info() -> Value {
    info()
}

#[tauri::command]
pub fn system_autostart(enabled: bool) -> Result<Value, String> {
    platform::set_autostart(enabled).map_err(|error| format!("登录时启动设置未保存：{error}"))?;
    Ok(info())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_navigate_to_a_fixed_set_of_destinations() {
        assert_eq!(parse_link("aieyes://panel"), Some(Link::Panel));
        assert_eq!(
            parse_link("AIEYES://Open/Sessions/"),
            Some(Link::Page("sessions"))
        );
        assert_eq!(parse_link("aieyes://overview"), Some(Link::Page("agent")));
        assert_eq!(parse_link("aieyes:servers"), Some(Link::Page("servers")));
        assert_eq!(
            parse_link("aieyes://settings/machines"),
            Some(Link::Settings {
                category: "hosts",
                section: None
            })
        );
        assert_eq!(
            parse_link("aieyes://settings/accounts"),
            Some(Link::Settings {
                category: "integrations",
                section: None
            })
        );
        assert_eq!(
            parse_link("aieyes://settings/webhooks"),
            Some(Link::Settings {
                category: "general",
                section: Some("webhooks")
            })
        );
        assert_eq!(
            parse_link("aieyes://settings/unknown"),
            Some(Link::Settings {
                category: "general",
                section: None
            })
        );
        // Query strings and fragments are never read; nothing else is a destination.
        assert_eq!(
            parse_link("aieyes://analysis?provider=codex&delete=1#x"),
            Some(Link::Page("analysis"))
        );
        assert_eq!(
            parse_link("aieyes://settings?section=webhooks"),
            parse_link("aieyes://settings")
        );
        for rejected in [
            "aieyes://",
            "aieyes://open",
            "aieyes://quit",
            "aieyes://user@panel",
            "aieyes://panel%2Fx",
            "https://panel",
            "aieyesx://panel",
            "",
        ] {
            assert_eq!(parse_link(rejected), None, "{rejected}");
        }
        assert_eq!(
            link_in([
                "--flag".to_string(),
                "aieyes://sessions".into(),
                "aieyes://panel".into()
            ]),
            Some(Link::Page("sessions"))
        );
        assert_eq!(link_in(["C:\\aieyes:\\x".to_string()]), None);
        assert_eq!(
            settings_target("general", Some("system")),
            json!({"category":"general","section":"system"})
        );
        assert!(LINKS.iter().all(|link| parse_link(link).is_some()));
    }

    #[test]
    fn desktop_entries_quote_the_program() {
        assert_eq!(exec_quote("/opt/Aieyes/aieyes"), "\"/opt/Aieyes/aieyes\"");
        assert_eq!(exec_quote("/home/a b/$x\"%"), "\"/home/a b/\\$x\\\"%%\"");
        let entry = autostart_entry("/usr/bin/aieyes-desktop");
        assert!(entry.contains("\nExec=\"/usr/bin/aieyes-desktop\"\n"));
        assert!(autostart_entry_enabled(&entry));
        assert!(!autostart_entry_enabled(&format!("{entry}Hidden=true\n")));
        assert!(!autostart_entry_enabled(
            "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n"
        ));
        let handler = link_handler_entry("/usr/bin/aieyes-desktop");
        assert!(handler.contains("\nExec=\"/usr/bin/aieyes-desktop\" %u\n"));
        assert!(handler.contains("\nMimeType=x-scheme-handler/aieyes;\n"));
    }
}
