//! Show without stealing focus, but allow deliberate interaction to activate the panel.
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE, SendMessageW,
        SetWindowLongPtrW, ShowWindow, WM_NCACTIVATE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};

/// A tool window never gets a taskbar or Alt+Tab entry. `skip_taskbar` alone only removes the tab once,
/// and Explorer adds it back each time the panel is shown.
pub fn configure(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let style = style & !((WS_EX_NOACTIVATE.0 | WS_EX_APPWINDOW.0) as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_TOOLWINDOW.0 as isize);
    }
}

/// Both directions use the same native API. Mixing native ShowWindow with a
/// cached framework hide can leave an invisible WebView intercepting the desktop.
/// Must run on the window's UI thread.
pub fn set_visible(hwnd: HWND, open: bool) -> Result<(), String> {
    unsafe {
        let _ = ShowWindow(hwnd, if open { SW_SHOWNOACTIVATE } else { SW_HIDE });
        if IsWindowVisible(hwnd).as_bool() != open {
            return Err("无法更改面板窗口可见性".into());
        }
    }
    if open {
        draw_active(hwnd);
    }
    Ok(())
}

/// DWM replaces an inactive window's Acrylic with a flat fallback colour, and the panel opens without
/// taking keyboard focus. Draw its frame as active instead; tao tracks focus separately (WM_SETFOCUS),
/// so this emits no focus event.
pub fn draw_active(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
    }
}
