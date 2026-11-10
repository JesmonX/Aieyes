//! Bring the terminal or IDE that owns a local live session to the front.
//!
//! The core walks the agent's process ancestors (with a start-time check against pid reuse); the
//! nearest ancestor that owns a visible top-level window is the target. One such window is the
//! exact window; several mean only the application could be activated. No new permission is used.
use serde_json::{Value, json};

#[tauri::command]
pub fn focus_session(pid: u32, process_start: u64) -> Value {
    let target = aieyes_core::sessions::focus_target(pid, process_start);
    if target["status"] != "found" {
        return json!({"outcome":"exited"});
    }
    let ancestors: Vec<(u32, String)> = target["ancestors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                u32::try_from(a["pid"].as_u64()?).ok()?,
                a["name"].as_str().unwrap_or("").to_string(),
            ))
        })
        .collect();
    // Re-check the agent right before activation: the pid must still name the same process.
    if !aieyes_core::sessions::process_matches(pid, process_start) {
        return json!({"outcome":"exited"});
    }
    platform::focus(&ancestors)
}

#[cfg(target_os = "windows")]
mod platform {
    use serde_json::{Value, json};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetWindow, GetWindowLongPtrW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
        ShowWindow, WS_EX_TOOLWINDOW,
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
    pub fn focus(ancestors: &[(u32, String)]) -> Value {
        for (pid, name) in ancestors {
            let found = windows(*pid);
            let Some(&hwnd) = found.first() else { continue };
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
            return json!({"outcome": if found.len() == 1 { "window" } else { "application" },"name":name});
        }
        json!({"outcome":"noWindow"})
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use serde_json::{Value, json};
    use std::process::Command;
    /// X11 window managers expose windows with their pid through `wmctrl -lp`; Wayland does not.
    pub fn focus(ancestors: &[(u32, String)]) -> Value {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return json!({"outcome":"unsupported"});
        }
        let Ok(output) = Command::new("wmctrl").arg("-lp").output() else {
            return json!({"outcome":"unsupported"});
        };
        if !output.status.success() {
            return json!({"outcome":"unsupported"});
        }
        let list = String::from_utf8_lossy(&output.stdout);
        for (pid, name) in ancestors {
            let found: Vec<&str> = list
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let id = parts.next()?;
                    let _desktop = parts.next()?;
                    (parts.next()? == pid.to_string()).then_some(id)
                })
                .collect();
            let Some(id) = found.first() else { continue };
            let activated = Command::new("wmctrl")
                .args(["-ia", id])
                .status()
                .is_ok_and(|s| s.success());
            if !activated {
                return json!({"outcome":"failed","name":name});
            }
            return json!({"outcome": if found.len() == 1 { "window" } else { "application" },"name":name});
        }
        json!({"outcome":"noWindow"})
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use serde_json::{Value, json};
    /// macOS uses the native SwiftUI app; the Tauri build there has no window enumeration.
    pub fn focus(_ancestors: &[(u32, String)]) -> Value {
        json!({"outcome":"unsupported"})
    }
}
