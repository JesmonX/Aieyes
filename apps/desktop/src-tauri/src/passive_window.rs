//! Keep mouse interaction with the floating panel from activating the application.
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, MA_NOACTIVATE, SW_SHOWNOACTIVATE, SetWindowLongPtrW,
            ShowWindow, WM_MOUSEACTIVATE, WM_NCDESTROY, WS_EX_NOACTIVATE,
        },
    },
};
const SUBCLASS_ID: usize = 0xA1E;

pub fn configure(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0);
    }
}

pub fn show(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if message == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(window_proc), id);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}
