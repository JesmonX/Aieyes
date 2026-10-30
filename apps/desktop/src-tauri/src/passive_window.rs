//! Show without stealing focus, but allow deliberate interaction to activate the panel.
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOPMOST,
            IsWindowVisible, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SWP_SHOWWINDOW, SendMessageW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOWPOS,
            WM_NCACTIVATE, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        },
    },
};

/// A tool window never gets a taskbar or Alt+Tab entry. `skip_taskbar` alone only removes the tab once,
/// and Explorer adds it back each time the panel is shown.
pub fn configure(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let style = style & !((WS_EX_NOACTIVATE.0 | WS_EX_APPWINDOW.0) as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_TOOLWINDOW.0 as isize);
        let _ = SetWindowSubclass(hwnd, Some(overlay_proc), 1, 0);
    }
}

/// Both directions use the same native API. Mixing native ShowWindow with a
/// cached framework hide can leave an invisible WebView intercepting the desktop.
/// Must run on the window's UI thread.
pub fn set_visible(hwnd: HWND, open: bool) -> Result<(), String> {
    unsafe {
        if open {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .map_err(|error| error.to_string())?;
        } else {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        if IsWindowVisible(hwnd).as_bool() != open {
            return Err("无法更改面板窗口可见性".into());
        }
    }
    if open {
        draw_active(hwnd);
    }
    Ok(())
}

/// Keep a visible overlay in the topmost band before Windows commits a z-order change.
/// Repairing it in WM_WINDOWPOSCHANGED leaves a composited frame behind the active app.
///
/// # Safety
/// `lparam` must contain a valid WINDOWPOS pointer from WM_WINDOWPOSCHANGING.
pub unsafe fn preserve_topmost(hwnd: HWND, lparam: LPARAM) {
    if lparam.0 == 0 || !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return;
    }
    let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
    if position.flags.contains(SWP_NOZORDER) {
        return;
    }
    let after = position.hwndInsertAfter;
    let target_topmost = after != HWND_BOTTOM
        && after != HWND_NOTOPMOST
        && !after.is_invalid()
        && unsafe { GetWindowLongPtrW(after, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 };
    if after != HWND_TOPMOST && !target_topmost {
        position.hwndInsertAfter = HWND_TOPMOST;
        position.flags |= SWP_NOACTIVATE;
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_WINDOWPOSCHANGING {
        unsafe {
            preserve_topmost(hwnd, lparam);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// DWM replaces an inactive window's Acrylic with a flat fallback colour, and the panel opens without
/// taking keyboard focus. Draw its frame as active instead; tao tracks focus separately (WM_SETFOCUS),
/// so this emits no focus event.
pub fn draw_active(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{Win32::UI::WindowsAndMessaging::*, core::w};

    #[test]
    fn overlay_retains_topmost_without_focus_and_hidden_window_stays_hidden() {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Aieyes overlay test"),
                WS_POPUP,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            struct Cleanup(HWND);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    unsafe {
                        let _ = DestroyWindow(self.0);
                    }
                }
            }
            let _cleanup = Cleanup(hwnd);
            configure(hwnd);
            let foreground = GetForegroundWindow();
            set_visible(hwnd, true).unwrap();
            for after in [HWND_NOTOPMOST, HWND_BOTTOM] {
                SetWindowPos(
                    hwnd,
                    Some(after),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .unwrap();
                assert_ne!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
                    0,
                    "Topmost must survive synchronously, before a later message or repaint"
                );
                assert_eq!(GetForegroundWindow(), foreground);
            }
            set_visible(hwnd, false).unwrap();
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .unwrap();
            assert!(!IsWindowVisible(hwnd).as_bool());
            set_visible(hwnd, true).unwrap();
            assert_ne!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
                0
            );
        }
    }
}
