//! Windows 11 shows Snap Layouts when the window answers HTMAXBUTTON for the pointer. The caption buttons are
//! drawn by the page inside the WebView2 child window, so the top-level window never sees that hit test. An
//! invisible layered child above the WebView covers the maximize button and answers HTMAXBUTTON instead; it
//! reports hover and press to the page and maximizes or restores on click, like the native button.
use std::cell::RefCell;
use tauri::{AppHandle, Emitter};
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, GetClientRect, HTMAXBUTTON, HWND_TOP, IDC_ARROW,
                IsZoomed, LWA_ALPHA, LoadCursorW, PostMessageW, RegisterClassW, SC_MAXIMIZE,
                SC_RESTORE, SM_CYFRAME, SWP_NOACTIVATE, SetLayeredWindowAttributes, SetWindowPos,
                WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
                WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONDOWN, WM_NCRBUTTONUP, WM_SIZE,
                WM_SYSCOMMAND, WNDCLASSW, WS_CHILD, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP,
                WS_VISIBLE,
            },
        },
    },
    core::w,
};

struct Target {
    hwnd: HWND,
    parent: HWND,
    app: AppHandle,
    hover: bool,
    pressed: bool,
}

thread_local! {
    static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
}

/// Covers the main window's maximize button on Windows 11; earlier versions have no Snap Layouts and keep
/// the page's own button (and its tooltip). Must run on the window's UI thread.
pub fn install(parent: HWND, app: AppHandle) {
    if crate::material::build_number() < 22000 {
        return;
    }
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(Into::into);
        let class = WNDCLASSW {
            lpfnWndProc: Some(target_proc),
            hInstance: instance.unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("AieyesSnapTarget"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // No redirection bitmap: nothing is drawn, the page's button stays visible underneath. Layered child
        // windows need the Windows 8+ compatibility entry in the app manifest (windows-app-manifest.xml).
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP,
            w!("AieyesSnapTarget"),
            w!(""),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            instance,
            None,
        ) else {
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        TARGET.with(|slot| {
            *slot.borrow_mut() = Some(Target {
                hwnd,
                parent,
                app,
                hover: false,
                pressed: false,
            })
        });
        place(parent, hwnd);
        let _ = SetWindowSubclass(parent, Some(follow_parent), 2, 0);
    }
}

/// Over the page's maximize button (46 x 40 CSS px, second from the right). While restored, the top resize
/// strip (Tauri's drag-resize window) stays above it.
fn place(parent: HWND, hwnd: HWND) {
    unsafe {
        let mut client = RECT::default();
        if GetClientRect(parent, &mut client).is_err() {
            return;
        }
        let dpi = GetDpiForWindow(parent);
        let scale = dpi as f64 / 96.0;
        let right = client.right - (46.0 * scale).round() as i32;
        let left = client.right - (92.0 * scale).round() as i32;
        // Minimized: no client area to cover until the next WM_SIZE.
        if left < 0 {
            return;
        }
        let top = if IsZoomed(parent).as_bool() {
            0
        } else {
            GetSystemMetricsForDpi(SM_CYFRAME, dpi)
        };
        let bottom = (40.0 * scale).round() as i32;
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            left,
            top,
            right - left,
            bottom - top,
            SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn follow_parent(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_SIZE {
        let target = TARGET.with(|slot| slot.borrow().as_ref().map(|t| t.hwnd));
        if let Some(target) = target {
            place(hwnd, target);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Updates the state and tells the page, which styles its button as hovered or pressed.
fn update(hover: bool, pressed: bool) {
    let changed = TARGET.with(|slot| {
        let mut slot = slot.borrow_mut();
        let target = slot.as_mut()?;
        if (target.hover, target.pressed) == (hover, pressed) {
            return None;
        }
        (target.hover, target.pressed) = (hover, pressed);
        Some(target.app.clone())
    });
    if let Some(app) = changed {
        let _ = app.emit_to(
            "main",
            "desktop:caption",
            serde_json::json!({"button": "maximize", "hover": hover, "pressed": pressed}),
        );
    }
}

unsafe extern "system" fn target_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = TARGET.with(|slot| slot.borrow().as_ref().map(|t| (t.parent, t.pressed)));
    let Some((parent, pressed)) = state else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_NCHITTEST => LRESULT(HTMAXBUTTON as isize),
        WM_NCMOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE | TME_NONCLIENT,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = unsafe { TrackMouseEvent(&mut track) };
            update(true, pressed);
            // DefWindowProc starts the system's Snap Layouts hover for HTMAXBUTTON.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCMOUSELEAVE => {
            update(false, false);
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // Handled here, not by DefWindowProc, which would draw a classic button and track it modally.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            update(true, true);
            LRESULT(0)
        }
        WM_NCLBUTTONUP => {
            if pressed {
                let command = if unsafe { IsZoomed(parent) }.as_bool() {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                let _ = unsafe {
                    PostMessageW(
                        Some(parent),
                        WM_SYSCOMMAND,
                        WPARAM(command as usize),
                        LPARAM(0),
                    )
                };
            }
            update(true, false);
            LRESULT(0)
        }
        WM_NCRBUTTONDOWN | WM_NCRBUTTONUP => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
