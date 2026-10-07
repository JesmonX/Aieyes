//! Show without stealing focus, but allow deliberate interaction to activate the panel.
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE,
        SetWindowLongPtrW, ShowWindow, WS_EX_NOACTIVATE,
    },
};

pub fn configure(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_NOACTIVATE.0 as isize));
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
    Ok(())
}
