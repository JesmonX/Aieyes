//! Bring the terminal or IDE that owns a local live session to the front.
//!
//! The core walks the agent's process ancestors (with a start-time check against pid reuse); the
//! native desktop thread route wins; otherwise a full workspace path can disambiguate windows.
//! Activation without evidence of a specific window is reported at application granularity.
use serde_json::{Value, json};

#[tauri::command]
pub async fn focus_session(
    pid: Option<u32>,
    process_start: Option<u64>,
    provider: Option<String>,
    native_session: Option<String>,
    client: Option<String>,
    workspace: Option<String>,
    machine_id: Option<String>,
) -> Value {
    if machine_id.as_deref().is_some_and(|id| id != "local") {
        return json!({"outcome":"unsupported"});
    }
    tauri::async_runtime::spawn_blocking(move || {
        let link = aieyes_core::sessions::session_deep_link(
            provider.as_deref().unwrap_or(""),
            client.as_deref().unwrap_or(""),
            native_session.as_deref().unwrap_or(""),
        );
        if let Some(url) = link {
            if platform::open_session(&url) {
                return json!({"outcome":"session","name":"Codex"});
            }
            if pid.is_none() {
                return json!({"outcome":"failed","name":"Codex"});
            }
        }
        let Some(pid) = pid.filter(|p| *p > 0) else {
            return json!({"outcome":"noWindow"});
        };
        focus_process(
            pid,
            process_start.unwrap_or(0),
            workspace.as_deref().unwrap_or(""),
        )
    })
    .await
    .unwrap_or_else(|_| json!({"outcome":"failed"}))
}

fn focus_process(pid: u32, process_start: u64, workspace: &str) -> Value {
    let target = aieyes_core::sessions::focus_target(pid, process_start);
    if target["status"] != "found" {
        return json!({"outcome":"exited"});
    }
    let ancestors: Vec<(u32, String, u64)> = target["ancestors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                u32::try_from(a["pid"].as_u64()?).ok()?,
                a["name"].as_str().unwrap_or("").to_string(),
                a["start"].as_u64().unwrap_or(0),
            ))
        })
        .collect();
    // Re-check the agent right before activation: the pid must still name the same process.
    let observed_start = ancestors.first().map_or(process_start, |a| a.2);
    if !aieyes_core::sessions::process_matches(pid, observed_start) {
        return json!({"outcome":"exited"});
    }
    platform::focus(&ancestors, workspace)
}

/// A basename is never enough to distinguish two checkouts with the same folder name.
#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn workspace_in_title(title: &str, workspace: &str) -> bool {
    if workspace.len() < 2 || workspace.len() > 4096 || workspace.chars().any(char::is_control) {
        return false;
    }
    let normalize = |s: &str| {
        let s = s.replace('\\', "/");
        if cfg!(target_os = "windows") {
            s.to_lowercase()
        } else {
            s
        }
    };
    let title = normalize(title);
    let workspace = normalize(workspace);
    let workspace = workspace.trim_end_matches('/');
    if workspace.is_empty()
        || !(workspace.starts_with('/') || workspace.as_bytes().get(1) == Some(&b':'))
    {
        return false;
    }
    title.match_indices(workspace).any(|(start, _)| {
        let before = title[..start].chars().next_back();
        let after = title[start + workspace.len()..].chars().next();
        before.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '"' | '\''))
            && after.is_none_or(|c| c.is_whitespace() || matches!(c, '/' | ')' | ']' | '"' | '\''))
    })
}

#[cfg(target_os = "windows")]
mod platform {
    use serde_json::{Value, json};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetWindow, GetWindowLongPtrW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE,
        SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
    };
    struct Search {
        pid: u32,
        windows: Vec<HWND>,
    }
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        let tool = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW.0 != 0;
        let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|h| !h.is_invalid());
        if owner == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() && !tool && !owned {
            search.windows.push(hwnd);
        }
        true.into()
    }
    /// Top-level application windows of a process, most recently active first (Z order).
    fn windows(pid: u32) -> Vec<HWND> {
        let mut search = Search {
            pid,
            windows: vec![],
        };
        let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut search as *mut Search as isize)) };
        search.windows
    }
    pub fn open_session(url: &str) -> bool {
        use windows::{
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            core::PCWSTR,
        };
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(url.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
            .0 as isize
                > 32
        }
    }
    pub fn focus(ancestors: &[(u32, String, u64)], workspace: &str) -> Value {
        for (pid, name, start) in ancestors {
            let found = windows(*pid);
            let matching: Vec<_> = found
                .iter()
                .filter(|hwnd| {
                    let mut title = [0u16; 4096];
                    let n = unsafe { GetWindowTextW(**hwnd, &mut title) };
                    super::workspace_in_title(
                        &String::from_utf16_lossy(&title[..n.max(0) as usize]),
                        workspace,
                    )
                })
                .collect();
            let exact = matching.len() == 1 || found.len() == 1;
            let Some(&hwnd) = (if matching.len() == 1 {
                matching.first().copied()
            } else {
                found.first()
            }) else {
                continue;
            };
            if !aieyes_core::sessions::process_matches(*pid, *start) {
                continue;
            }
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            // SetForegroundWindow can be refused; only the actual foreground window counts.
            if unsafe { GetForegroundWindow() } != hwnd {
                return json!({"outcome":"failed","name":name});
            }
            return json!({"outcome": if exact { "window" } else { "application" },"name":name});
        }
        json!({"outcome":"noWindow"})
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use serde_json::{Value, json};
    use std::process::Command;
    use std::time::{Duration, Instant};
    fn run(program: &str, args: &[&str]) -> Option<String> {
        let mut command = Command::new(program);
        command.args(args);
        let output = aieyes_core::process::run(command, vec![], Duration::from_secs(2)).ok()?;
        String::from_utf8(output).ok()
    }
    pub fn open_session(url: &str) -> bool {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        // A protocol handler must exist. A successful spawn alone does not prove registration.
        if run("xdg-mime", &["query", "default", "x-scheme-handler/codex"])
            .is_none_or(|s| s.trim().is_empty())
        {
            return false;
        }
        aieyes_core::process::open_external(command).is_ok()
    }
    /// X11 window managers expose windows with their pid through `wmctrl -lp`; Wayland does not.
    pub fn focus(ancestors: &[(u32, String, u64)], workspace: &str) -> Value {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return json!({"outcome":"unsupported"});
        }
        let Some(list) = run("wmctrl", &["-lp"]) else {
            return json!({"outcome":"unsupported"});
        };
        for (pid, name, start) in ancestors {
            let found: Vec<(&str, String)> = list
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let id = parts.next()?;
                    let _desktop = parts.next()?;
                    if parts.next()? != pid.to_string() {
                        return None;
                    }
                    parts.next()?; // host
                    Some((id, parts.collect::<Vec<_>>().join(" ")))
                })
                .collect();
            let matching: Vec<_> = found
                .iter()
                .filter(|(_, title)| super::workspace_in_title(title, workspace))
                .collect();
            let selected = if matching.len() == 1 {
                matching.first().copied()
            } else {
                found.first()
            };
            let Some((id, _)) = selected else { continue };
            if !aieyes_core::sessions::process_matches(*pid, *start) {
                continue;
            }
            if run("wmctrl", &["-ia", id]).is_none() {
                return json!({"outcome":"failed","name":name});
            }
            let wanted = u64::from_str_radix(id.trim_start_matches("0x"), 16).ok();
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                let Some(active) = run("xprop", &["-root", "_NET_ACTIVE_WINDOW"]) else {
                    return json!({"outcome":"requested","name":name});
                };
                let active = active
                    .split_whitespace()
                    .last()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());
                if active.is_some() && active == wanted {
                    break;
                }
                if Instant::now() >= deadline {
                    return json!({"outcome":"failed","name":name});
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            return json!({"outcome": if matching.len() == 1 || found.len() == 1 { "window" } else { "application" },"name":name});
        }
        json!({"outcome":"noWindow"})
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use serde_json::{Value, json};
    /// macOS uses the native SwiftUI app; the Tauri build there has no window enumeration.
    pub fn open_session(url: &str) -> bool {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.args(["-b", "com.openai.codex", url]);
        aieyes_core::process::open_external(command).is_ok()
    }
    pub fn focus(_ancestors: &[(u32, String, u64)], _workspace: &str) -> Value {
        json!({"outcome":"unsupported"})
    }
}

#[cfg(test)]
mod tests {
    use super::workspace_in_title;
    #[test]
    fn workspace_matching_requires_a_full_path_and_component_boundaries() {
        assert!(workspace_in_title("Editor — /work/Aieyes", "/work/Aieyes"));
        assert!(workspace_in_title(
            "[/work/Aieyes/src/main.rs]",
            "/work/Aieyes/"
        ));
        assert!(!workspace_in_title("Editor — Aieyes", "/work/Aieyes"));
        assert!(!workspace_in_title("/other/work/Aieyes", "/work/Aieyes"));
        assert!(!workspace_in_title("/work/Aieyes-copy", "/work/Aieyes"));
        assert!(!workspace_in_title("Editor", ""));
        assert!(!workspace_in_title("Editor /", "/"));
    }
}
