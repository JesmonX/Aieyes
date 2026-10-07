//! Windows' entry point deliberately has no WebView. Its message pump and menu
//! remain available even when the panel's renderer stops responding.
#![allow(unsafe_op_in_unsafe_fn)]
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::{ptr::null_mut, sync::mpsc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Gdi::*, GdiPlus::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::*,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

use std::result::Result::Ok;
use windows::Win32::Graphics::GdiPlus::Ok as GDI_OK;
const WAKE: u32 = WM_APP + 41;
pub const WIDTH: f64 = 144.0;
pub const HEIGHT: f64 = 44.0;
enum Command {
    Status(Value),
    Visible(bool),
    Position(Option<(i32, i32)>),
    Stop,
}
pub struct Capsule {
    hwnd: usize,
    tx: mpsc::Sender<Command>,
}
struct Surface {
    app: AppHandle,
    rx: mpsc::Receiver<Command>,
    info: RefCell<Value>,
    press: Cell<Option<(POINT, RECT)>>,
    dragged: Cell<bool>,
    hover: Cell<bool>,
    dark: Cell<bool>,
    actions: mpsc::Sender<String>,
    tooltip: Cell<Option<HWND>>,
    tooltip_text: RefCell<Vec<u16>>,
}
impl Capsule {
    pub fn start(app: AppHandle, position: Option<(i32, i32)>) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (ready, wait) = mpsc::sync_channel(1);
        let (actions, commands) = mpsc::channel::<String>();
        let worker = app.clone();
        std::thread::Builder::new().name("aieyes-capsule-actions".into()).spawn(move || {
            for action in commands {
                if let Err(error)=crate::desktop::action(&worker,&action) {
                    let _=worker.emit("desktop:error",&error);
                    if let Some(capsule)=worker.try_state::<Capsule>() {
                        capsule.status(serde_json::json!({"summary":error,"phase":"interrupted","recovery":"failed"}));
                    }
                }
            }
        }).map_err(|e|e.to_string())?;
        std::thread::Builder::new()
            .name("aieyes-capsule".into())
            .spawn(move || unsafe {
                let mut token = 0;
                if GdiplusStartup(
                    &mut token,
                    &GdiplusStartupInput {
                        GdiplusVersion: 1,
                        ..Default::default()
                    },
                    null_mut(),
                ) != GDI_OK
                {
                    let _ = ready.send(Err("无法初始化悬浮入口绘图".to_string()));
                    return;
                }
                let instance = GetModuleHandleW(None).unwrap_or_default();
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance.into(),
                    lpszClassName: w!("AieyesNativeCapsule"),
                    hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
                    style: CS_DROPSHADOW,
                    ..Default::default()
                };
                RegisterClassW(&class);
                let mut surface = Box::new(Surface {
                    app,
                    rx,
                    info: RefCell::new(Value::Null),
                    press: Cell::new(None),
                    dragged: Cell::new(false),
                    hover: Cell::new(false),
                    dark: Cell::new(false),
                    actions,
                    tooltip: Cell::new(None),
                    tooltip_text: RefCell::new(Vec::new()),
                });
                let result = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    class.lpszClassName,
                    w!("Aieyes · 单击打开面板，右键菜单"),
                    WS_POPUP,
                    0,
                    0,
                    144,
                    44,
                    None,
                    None,
                    Some(instance.into()),
                    Some((&mut *surface as *mut Surface).cast()),
                );
                match result {
                    Result::Ok(hwnd) => {
                        place(hwnd, position, false);
                        reshape(hwnd);
                        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                            dwICC: ICC_WIN95_CLASSES,
                        });
                        surface.tooltip.set(
                            CreateWindowExW(
                                WS_EX_TOPMOST,
                                TOOLTIPS_CLASSW,
                                None,
                                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                                0,
                                0,
                                0,
                                0,
                                Some(hwnd),
                                None,
                                Some(instance.into()),
                                None,
                            )
                            .ok(),
                        );
                        tooltip(hwnd, &surface, "Aieyes · 单击打开面板，右键菜单", true);
                        let _ = ready.send(Result::Ok(hwnd.0 as usize));
                        let mut message = MSG::default();
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                    }
                }
                // Surface and all GDI+ resources belong to this thread.
                drop(surface);
                GdiplusShutdown(token);
            })
            .map_err(|e| e.to_string())?;
        let hwnd = wait
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())??;
        Ok(Self { hwnd, tx })
    }
    fn send(&self, command: Command) {
        if self.tx.send(command).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(HWND(self.hwnd as _)), WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }
    pub fn status(&self, info: Value) {
        self.send(Command::Status(info));
    }
    pub fn visible(&self, visible: bool) {
        self.send(Command::Visible(visible));
    }
    pub fn position(&self, position: Option<(i32, i32)>) {
        self.send(Command::Position(position));
    }
    pub fn stop(&self) {
        self.send(Command::Stop);
    }
    pub fn owns_focus(&self) -> bool {
        unsafe { GetForegroundWindow().0 as usize == self.hwnd }
    }
    pub fn geometry(&self) -> Result<(i32, i32, u32, u32, f64), String> {
        unsafe {
            let hwnd = HWND(self.hwnd as _);
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
            Ok((
                rect.left,
                rect.top,
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
                GetDpiForWindow(hwnd).max(96) as f64 / 96.0,
            ))
        }
    }
}
fn dispatch(surface: &Surface, action: &str) {
    let _ = surface.actions.send(action.to_owned());
}
unsafe fn tooltip(hwnd: HWND, surface: &Surface, text: &str, add: bool) {
    let Some(tip) = surface.tooltip.get() else {
        return;
    };
    // The tooltip retains this buffer's address until the next update.
    let mut next: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let text_pointer = next.as_mut_ptr();
    let previous = surface.tooltip_text.replace(next);
    let tool = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd,
        uId: hwnd.0 as usize,
        lpszText: PWSTR(text_pointer),
        ..Default::default()
    };
    SendMessageW(
        tip,
        if add {
            TTM_ADDTOOLW
        } else {
            TTM_UPDATETIPTEXTW
        },
        None,
        Some(LPARAM((&tool as *const TTTOOLINFOW) as isize)),
    );
    SendMessageW(tip, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(360)));
    drop(previous);
}
unsafe fn area(hwnd: HWND) -> RECT {
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    monitor.rcWork
}
unsafe fn reshape(hwnd: HWND) {
    let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
    let (width, height) = (
        (WIDTH * scale).round() as i32,
        (HEIGHT * scale).round() as i32,
    );
    let _ = SetWindowPos(
        hwnd,
        None,
        0,
        0,
        width,
        height,
        SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
    );
    let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, height, height);
    if SetWindowRgn(hwnd, Some(region), true) == 0 {
        let _ = DeleteObject(region.into());
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}
unsafe fn place(hwnd: HWND, position: Option<(i32, i32)>, snap: bool) {
    if let Some((x, y)) = position {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
    let work = area(hwnd);
    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let (mut x, mut y) = position.unwrap_or((
        work.right - width - 24,
        work.top + (work.bottom - work.top - height) / 2,
    ));
    let threshold = (48.0 * GetDpiForWindow(hwnd).max(96) as f64 / 96.0) as i32;
    if snap {
        if x - work.left <= threshold {
            x = work.left;
        } else if work.right - width - x <= threshold {
            x = work.right - width;
        }
        if y - work.top <= threshold {
            y = work.top;
        } else if work.bottom - height - y <= threshold {
            y = work.bottom - height;
        }
    }
    x = x.clamp(work.left, (work.right - width).max(work.left));
    y = y.clamp(work.top, (work.bottom - height).max(work.top));
    let _ = SetWindowPos(
        hwnd,
        None,
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
    );
}
unsafe fn native_menu(hwnd: HWND, surface: &Surface) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let info = surface.info.borrow().clone();
    let open = info["panelOpen"].as_bool().unwrap_or(false);
    let pinned = info["panelPinned"].as_bool().unwrap_or(false);
    let rows = [
        (if open { "收起面板" } else { "打开面板" }, "toggle-panel"),
        (
            if pinned {
                "取消固定面板"
            } else {
                "固定面板"
            },
            "toggle-pin",
        ),
        ("重建面板（用于界面无响应）", "refresh-floating"),
        ("打开主窗口", "open"),
        ("设置…", "settings"),
        ("重置位置", "reset-position"),
        ("暂时隐藏", "hide-ball"),
        ("退出 Aieyes", "quit"),
    ];
    for (i, (label, _)) in rows.iter().enumerate() {
        if i == 3 || i == 7 {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
        let flags = if i == 6 && !info["trayAvailable"].as_bool().unwrap_or(false) {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(label.as_ptr()));
    }
    crate::desktop::native_interaction(&surface.app, true);
    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    crate::desktop::native_interaction(&surface.app, false);
    if selected > 0
        && let Some((_, action)) = rows.get(selected as usize - 1)
    {
        dispatch(surface, action);
        if open && *action == "toggle-pin" {
            dispatch(surface, "focus-panel");
        }
    } else if open {
        dispatch(surface, "focus-panel");
    }
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Surface;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let surface = &*pointer;
    match msg {
        WAKE => {
            while let Result::Ok(command) = surface.rx.try_recv() {
                match command {
                    Command::Status(info) => {
                        surface
                            .dark
                            .set(info["dark"].as_bool().unwrap_or(surface.dark.get()));
                        surface.info.replace(info);
                        let title = format!(
                            "Aieyes · {} · 单击打开面板，右键菜单",
                            surface.info.borrow()["summary"].as_str().unwrap_or("")
                        );
                        tooltip(hwnd, surface, &title, false);
                        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
                        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    Command::Visible(show) => {
                        let _ = ShowWindow(hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
                    }
                    Command::Position(position) => {
                        place(hwnd, position, false);
                        reshape(hwnd);
                        save_position(hwnd, surface);
                    }
                    Command::Stop => {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd, surface);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let mut point = POINT::default();
            let mut rect = RECT::default();
            let _ = GetCursorPos(&mut point);
            let _ = GetWindowRect(hwnd, &mut rect);
            surface.press.set(Some((point, rect)));
            surface.dragged.set(false);
            let _ = InvalidateRect(Some(hwnd), None, false);
            crate::desktop::native_interaction(&surface.app, true);
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            surface.hover.set(false);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !surface.hover.replace(true) {
                let mut tracking = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tracking);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            if let Some((start, rect)) = surface.press.get() {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let dx = point.x - start.x;
                let dy = point.y - start.y;
                if surface.dragged.get() || dx.abs() + dy.abs() > 5 {
                    surface.dragged.set(true);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        rect.left + dx,
                        rect.top + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let pressed = surface.press.take().is_some();
            let dragged = surface.dragged.get();
            let _ = ReleaseCapture();
            surface.dragged.set(false);
            let _ = InvalidateRect(Some(hwnd), None, false);
            crate::desktop::native_interaction(&surface.app, false);
            if pressed {
                if dragged {
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut rect);
                    place(hwnd, Some((rect.left, rect.top)), true);
                    save_position(hwnd, surface);
                } else {
                    dispatch(surface, "toggle-panel");
                }
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            surface.press.set(None);
            surface.dragged.set(false);
            let _ = InvalidateRect(Some(hwnd), None, false);
            crate::desktop::native_interaction(&surface.app, false);
            LRESULT(0)
        }
        WM_CONTEXTMENU => {
            native_menu(hwnd, surface);
            LRESULT(0)
        }
        // WS_EX_NOACTIVATE is deliberately not a keyboard focus target.
        // Keyboard users enter through the system tray or the main window.
        WM_DPICHANGED => {
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
            reshape(hwnd);
            place(hwnd, Some((suggested.left, suggested.top)), false);
            save_position(hwnd, surface);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            place(hwnd, Some((rect.left, rect.top)), false);
            reshape(hwnd);
            save_position(hwnd, surface);
            LRESULT(0)
        }
        WM_CLOSE => {
            dispatch(surface, "hide-ball");
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
unsafe fn save_position(hwnd: HWND, surface: &Surface) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() {
        crate::desktop::capsule_moved(&surface.app, rect.left, rect.top);
    }
}
// System contrast colors use COLORREF (BGR); GDI+ takes opaque ARGB.
unsafe fn system_argb(index: SYS_COLOR_INDEX) -> u32 {
    let color = GetSysColor(index);
    0xff000000 | ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
}
unsafe fn high_contrast() -> bool {
    let mut settings = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    SystemParametersInfoW(
        SPI_GETHIGHCONTRAST,
        settings.cbSize,
        Some((&mut settings as *mut HIGHCONTRASTW).cast()),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    )
    .is_ok()
        && settings.dwFlags.contains(HCF_HIGHCONTRASTON)
}
// No timers or breathing animation: the native capsule is always motion-reduced.
unsafe fn paint(hwnd: HWND, surface: &Surface) {
    let contrast = high_contrast();
    let mut ps = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut graphics = null_mut();
    if GdipCreateFromHDC(dc, &mut graphics) == GDI_OK {
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderPrepend);
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipGraphicsClear(
            graphics,
            if contrast {
                system_argb(COLOR_WINDOW)
            } else if surface.press.get().is_some() {
                if surface.dark.get() { 0xff494951 } else { 0xffe4e4e9 }
            } else if surface.hover.get() {
                if surface.dark.get() { 0xff3b3b43 } else { 0xffeeeeF2 }
            } else if surface.dark.get() {
                0xff29292e
            } else {
                0xfff6f6f8
            },
        );
        let mut outline = null_mut();
        let mut pen = null_mut();
        GdipCreatePath(FillModeAlternate, &mut outline);
        GdipAddPathArc(outline, 0.5, 0.5, 43.0, 43.0, 90.0, 180.0);
        GdipAddPathArc(outline, 100.5, 0.5, 43.0, 43.0, 270.0, 180.0);
        GdipClosePathFigure(outline);
        GdipCreatePen1(
            if contrast {
                system_argb(COLOR_WINDOWTEXT)
            } else if surface.dark.get() {
                0xff565660
            } else {
                0xffd5d5dd
            },
            1.0,
            UnitPixel,
            &mut pen,
        );
        GdipDrawPath(graphics, pen, outline);
        if surface.press.get().is_some() { GdipTranslateWorldTransform(graphics, 0.0, 1.0, MatrixOrderPrepend); }
        GdipDeletePen(pen);
        GdipDeletePath(outline);
        let info = surface.info.borrow().clone();
        let phase = info["phase"].as_str().unwrap_or("idle");
        let label = match phase {
            "working" => "进行中", "thinking" => "思考中", "tool" => "执行工具",
            "complete" => "已完成", "interrupted" => "已中断", "unknown" => "待确认", _ => "空闲",
        };
        let [r, g, b] = crate::phase_colors::rgb(phase, surface.dark.get());
        let color = 0xff000000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        let label = if info["recovery"] == "failed" {
            "恢复失败"
        } else if info["recovery"] == "refreshing" {
            "恢复中"
        } else if info["unavailable"] == true {
            "状态不全"
        } else {
            label
        };
        let mut pixels = include_bytes!("../icons/brand.rgba").to_vec();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let mut bitmap = null_mut();
        if GdipCreateBitmapFromScan0(32, 32, 128, 0x26200a, Some(pixels.as_ptr()), &mut bitmap)
            == GDI_OK
        {
            GdipDrawImageRect(graphics, bitmap.cast(), 12.0, 10.0, 24.0, 24.0);
            GdipDisposeImage(bitmap.cast());
        }
        let mut brush = null_mut();
        GdipCreateSolidFill(
            if contrast {
                system_argb(COLOR_WINDOWTEXT)
            } else {
                color
            },
            &mut brush,
        );
        GdipFillEllipse(graphics, brush.cast(), 39.0, 19.0, 6.0, 6.0);
        GdipDeleteBrush(brush.cast());
        let mut family = null_mut();
        let mut font = null_mut();
        let mut format = null_mut();
        GdipCreateFontFamilyFromName(w!("Segoe UI"), null_mut(), &mut family);
        GdipCreateFont(family, 13.0, FontStyleRegular.0, UnitPixel, &mut font);
        GdipCreateStringFormat(0, 0, &mut format);
        GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        GdipCreateSolidFill(
            if contrast {
                system_argb(COLOR_WINDOWTEXT)
            } else if surface.dark.get() {
                0xffefeff2
            } else {
                0xff252528
            },
            &mut brush,
        );
        let text: Vec<u16> = label.encode_utf16().collect();
        GdipDrawString(
            graphics,
            PCWSTR(text.as_ptr()),
            text.len() as i32,
            font,
            &RectF {
                X: 49.0,
                Y: 0.0,
                Width: 61.0,
                Height: 44.0,
            },
            format,
            brush.cast(),
        );
        let count = info["activeCount"].as_u64().unwrap_or(0);
        if count > 0 && info["showCount"].as_bool().unwrap_or(false) {
            let mut badge = null_mut();
            GdipCreateSolidFill(if contrast { system_argb(COLOR_HIGHLIGHT) } else { match (info["accent"].as_str().unwrap_or("indigo"), surface.dark.get()) {
                ("blue",true)=>0xff344961,("blue",false)=>0xffdce9f8,
                ("teal",true)=>0xff314d45,("teal",false)=>0xffdceee8,
                ("purple",true)=>0xff4b3e60,("purple",false)=>0xffeee3fa,
                (_,true)=>0xff40425a,(_,false)=>0xffe4e5f5,
            } }, &mut badge);
            GdipFillEllipse(graphics, badge.cast(), 109.0, 12.0, 25.0, 20.0);
            GdipDeleteBrush(badge.cast());
            if contrast { GdipDeleteBrush(brush.cast()); GdipCreateSolidFill(system_argb(COLOR_HIGHLIGHTTEXT), &mut brush); }
            GdipSetStringFormatAlign(format, StringAlignmentCenter);
            let count = if count > 99 {
                "99+".into()
            } else {
                count.to_string()
            };
            let text: Vec<u16> = count.encode_utf16().collect();
            GdipDrawString(
                graphics,
                PCWSTR(text.as_ptr()),
                text.len() as i32,
                font,
                &RectF {
                    X: 108.0,
                    Y: 0.0,
                    Width: 29.0,
                    Height: 44.0,
                },
                format,
                brush.cast(),
            );
        }
        GdipDeleteBrush(brush.cast());
        GdipDeleteStringFormat(format);
        GdipDeleteFont(font);
        GdipDeleteFontFamily(family);
        GdipDeleteGraphics(graphics);
    }
    let _ = EndPaint(hwnd, &ps);
}
