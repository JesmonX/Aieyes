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
#[path = "pet_sprite.rs"]
mod pet;
const PET_TIMER: usize = 42;
const WAKE: u32 = WM_APP + 41;
const RESTORE_TOPMOST: u32 = WM_APP + 42;
pub const WIDTH: f64 = 144.0;
pub const HEIGHT: f64 = 168.0;
enum Command {
    Status(Value),
    Visible(bool),
    Position(Option<(i32, i32)>),
    WatchPanel(Option<usize>),
    Stop,
}
pub struct NativePet {
    hwnd: usize,
    tx: mpsc::Sender<Command>,
}
struct PetFont {
    collection: *mut GpFontCollection,
    family: *mut GpFontFamily,
    font: *mut GpFont,
}
impl PetFont {
    unsafe fn new() -> Self {
        let mut result = Self {
            collection: null_mut(),
            family: null_mut(),
            font: null_mut(),
        };
        let bytes = include_bytes!("../../web/fonts/HarmonyOS_Sans_SC_Regular.ttf");
        if GdipNewPrivateFontCollection(&mut result.collection) == GDI_OK
            && GdipPrivateAddMemoryFont(
                result.collection,
                bytes.as_ptr().cast(),
                bytes.len() as i32,
            ) == GDI_OK
        {
            GdipCreateFontFamilyFromName(
                w!("HarmonyOS Sans SC"),
                result.collection,
                &mut result.family,
            );
        }
        if result.family.is_null() {
            GdipCreateFontFamilyFromName(w!("Microsoft YaHei UI"), null_mut(), &mut result.family);
        }
        if result.family.is_null() {
            GdipGetGenericFontFamilySansSerif(&mut result.family);
        }
        if !result.family.is_null() {
            GdipCreateFont(
                result.family,
                13.0,
                FontStyleRegular.0,
                UnitPixel,
                &mut result.font,
            );
        }
        result
    }
}
impl Drop for PetFont {
    fn drop(&mut self) {
        unsafe {
            if !self.font.is_null() {
                GdipDeleteFont(self.font);
            }
            if !self.family.is_null() {
                GdipDeleteFontFamily(self.family);
            }
            if !self.collection.is_null() {
                GdipDeletePrivateFontCollection(&mut self.collection);
            }
        }
    }
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
    panel: Cell<Option<usize>>,
    mouse_hook: Cell<Option<HHOOK>>,
    menu_active: Cell<bool>,
    tooltip_text: RefCell<Vec<u16>>,
    font: PetFont,
    pet_sprite: RefCell<pet::Sprite>,
    pet_start: Cell<std::time::Instant>,
    pet_state: RefCell<aieyes_core::pets::PetState>,
    timer_deadline: Cell<Option<std::time::Instant>>,
    painted_frame: Cell<Option<u32>>,
    renderer: RefCell<pet::Renderer>,
}
impl NativePet {
    pub fn start(app: AppHandle, position: Option<(i32, i32)>) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (ready, wait) = mpsc::sync_channel(1);
        let (actions, commands) = mpsc::channel::<String>();
        let worker = app.clone();
        std::thread::Builder::new().name("aieyes-pet-actions".into()).spawn(move || {
            for action in commands {
                if let Err(error)=crate::desktop::action(&worker,&action) {
                    let _=worker.emit("desktop:error",&error);
                    if let Some(pet)=worker.try_state::<NativePet>() {
                        pet.status(serde_json::json!({"summary":error,"phase":"interrupted","recovery":"failed"}));
                    }
                }
            }
        }).map_err(|e|e.to_string())?;
        std::thread::Builder::new()
            .name("aieyes-pet".into())
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
                    lpszClassName: w!("AieyesNativePet"),
                    hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
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
                    panel: Cell::new(None),
                    mouse_hook: Cell::new(None),
                    menu_active: Cell::new(false),
                    tooltip_text: RefCell::new(Vec::new()),
                    font: PetFont::new(),
                    pet_sprite: RefCell::new(pet::Sprite::empty()),
                    pet_start: Cell::new(std::time::Instant::now()),
                    pet_state: RefCell::new(aieyes_core::pets::project(&Default::default())),
                    timer_deadline: Cell::new(None),
                    painted_frame: Cell::new(None),
                    renderer: RefCell::new(pet::Renderer::default()),
                });
                let result = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    class.lpszClassName,
                    w!("Aieyes · 单击打开面板，右键菜单"),
                    WS_POPUP,
                    0,
                    0,
                    144,
                    168,
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
                        HOOK_SURFACE.with(|slot| slot.set(&*surface));
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
                if let Some(hook) = surface.mouse_hook.take() {
                    let _ = UnhookWindowsHookEx(hook);
                }
                HOOK_SURFACE.with(|slot| slot.set(std::ptr::null()));
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
    pub fn watch_panel(&self, hwnd: Option<usize>) {
        self.send(Command::WatchPanel(hwnd));
    }
    pub fn stop(&self) {
        self.send(Command::Stop);
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
thread_local! {
    static HOOK_SURFACE: Cell<*const Surface> = const { Cell::new(std::ptr::null()) };
}

// Runs on the pet message thread. Never wait for the renderer or consume
// input here: the original click must still reach the other application.
unsafe extern "system" fn outside_click(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0
        && matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        HOOK_SURFACE.with(|slot| {
            if let Some(surface) = slot.get().as_ref()
                && let Some(panel) = surface.panel.get()
                && !surface.menu_active.get()
            {
                let point = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
                let target = WindowFromPoint(point);
                let root = GetAncestor(target, GA_ROOT);
                let owner = GetAncestor(target, GA_ROOTOWNER);
                let pet = surface
                    .app
                    .try_state::<NativePet>()
                    .map(|c| c.hwnd)
                    .unwrap_or(0);
                if ![root.0 as usize, owner.0 as usize]
                    .iter()
                    .any(|h| *h == panel || *h == pet)
                {
                    dispatch(surface, "dismiss-panel");
                }
            }
        });
    }
    CallNextHookEx(None, code, wparam, lparam)
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
unsafe fn animate(hwnd: HWND) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Surface;
    if pointer.is_null() {
        return;
    }
    let surface = &*pointer;
    let now = std::time::Instant::now();
    let delay = aieyes_core::pets::next_frame_delay(
        &surface.pet_state.borrow(),
        surface.pet_start.get().elapsed().as_millis() as u64,
        !IsWindowVisible(hwnd).as_bool() || pet::reduced_motion(),
    );
    if let Some(delay) = delay {
        let delay = delay.clamp(1, u32::MAX as u64) as u32;
        let due = now + std::time::Duration::from_millis(u64::from(delay));
        if surface.timer_deadline.get().is_some_and(|old| {
            old.saturating_duration_since(due)
                .max(due.saturating_duration_since(old))
                < std::time::Duration::from_millis(2)
        }) {
            return;
        }
        surface
            .timer_deadline
            .set((SetTimer(Some(hwnd), PET_TIMER, delay, None) != 0).then_some(due));
    } else if surface.timer_deadline.take().is_some() {
        let _ = KillTimer(Some(hwnd), PET_TIMER);
    }
}
unsafe fn pet_height(hwnd: HWND) -> f64 {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Surface;
    if pointer.is_null() {
        return HEIGHT;
    }
    aieyes_core::pets::height(&(&*pointer).pet_state.borrow())
}
unsafe fn pet_scale(hwnd: HWND) -> f64 {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Surface;
    if pointer.is_null() {
        return 1.0;
    }
    aieyes_core::pets::normalized_scale(
        (&*pointer).info.borrow()["petScale"]
            .as_f64()
            .unwrap_or(1.0),
    )
}
unsafe fn is_topmost(hwnd: HWND) -> bool {
    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0
}
unsafe fn raise_pet(hwnd: HWND, surface: &Surface, show: bool) {
    let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    if show {
        flags |= SWP_SHOWWINDOW;
    }
    if let Err(error) = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags) {
        let _ = surface.app.emit("desktop:error", &error.to_string());
    }
}
unsafe fn reshape(hwnd: HWND) {
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED.0) as isize);
    let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0 * pet_scale(hwnd);
    let _ = SetWindowPos(
        hwnd,
        None,
        0,
        0,
        (WIDTH * scale).round() as i32,
        (pet_height(hwnd) * scale).round() as i32,
        SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
    );
    SetWindowRgn(hwnd, None, true);
    animate(hwnd);
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
    let rows = crate::context_menu::entries(true, true, info["trayAvailable"] == true, false);
    for (i, row) in rows.iter().enumerate() {
        if row.action == "separator" {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            continue;
        }
        let label: Vec<u16> = row.label.encode_utf16().chain(Some(0)).collect();
        let flags = if row.enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(label.as_ptr()));
    }
    crate::desktop::native_interaction(&surface.app, true);
    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    surface.menu_active.set(true);
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
    surface.menu_active.set(false);
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    crate::desktop::native_interaction(&surface.app, false);
    if selected > 0
        && let Some(row) = rows.get(selected as usize - 1)
    {
        dispatch(surface, row.action);
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
                        let changed = surface.info.borrow()["pet"]["key"] != info["pet"]["key"];
                        if surface.info.borrow()["petAsset"]["spritePath"]
                            != info["petAsset"]["spritePath"]
                        {
                            surface.pet_sprite.replace(
                                pet::Sprite::load(&info["petAsset"])
                                    .unwrap_or_else(pet::Sprite::empty),
                            );
                        }
                        let previous_height = pet_height(hwnd);
                        let previous_scale = pet_scale(hwnd);
                        let state = serde_json::from_value(info["pet"].clone())
                            .unwrap_or_else(|_| aieyes_core::pets::project(&Default::default()));
                        surface.pet_state.replace(state);
                        surface.info.replace(info);
                        if pet_height(hwnd) != previous_height || pet_scale(hwnd) != previous_scale
                        {
                            reshape(hwnd);
                            let mut rect = RECT::default();
                            let _ = GetWindowRect(hwnd, &mut rect);
                            place(hwnd, Some((rect.left, rect.top)), false);
                        }
                        if changed {
                            surface.pet_start.set(std::time::Instant::now());
                        }
                        animate(hwnd);
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
                        if show {
                            raise_pet(hwnd, surface, true);
                        } else {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                        }
                        animate(hwnd);
                    }
                    Command::Position(position) => {
                        place(hwnd, position, false);
                        reshape(hwnd);
                        save_position(hwnd, surface);
                    }
                    Command::WatchPanel(panel) => {
                        surface.panel.set(panel);
                        if panel.is_some() && surface.mouse_hook.get().is_none() {
                            match SetWindowsHookExW(
                                WH_MOUSE_LL,
                                Some(outside_click),
                                GetModuleHandleW(None).ok().map(Into::into),
                                0,
                            ) {
                                Result::Ok(hook) => surface.mouse_hook.set(Some(hook)),
                                Err(error) => {
                                    let _ = surface.app.emit("desktop:error", &error.to_string());
                                }
                            }
                        } else if panel.is_none()
                            && let Some(hook) = surface.mouse_hook.take()
                        {
                            let _ = UnhookWindowsHookEx(hook);
                        }
                    }
                    Command::Stop => {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_WINDOWPOSCHANGED => {
            // A shell/window-management operation can demote the pet after
            // creation. Defer recovery until that operation has completed.
            // Do not raise hidden pets or reorder already-topmost peers.
            if IsWindowVisible(hwnd).as_bool() && !is_topmost(hwnd) {
                let _ = PostMessageW(Some(hwnd), RESTORE_TOPMOST, WPARAM(0), LPARAM(0));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        RESTORE_TOPMOST => {
            if IsWindowVisible(hwnd).as_bool() && !is_topmost(hwnd) {
                raise_pet(hwnd, surface, false);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == PET_TIMER => {
            let _ = KillTimer(Some(hwnd), PET_TIMER);
            surface.timer_deadline.set(None);
            if IsWindowVisible(hwnd).as_bool() {
                let frame = aieyes_core::pets::frame(
                    &surface.pet_state.borrow(),
                    surface.pet_start.get().elapsed().as_millis() as u64,
                    pet::reduced_motion(),
                );
                if surface.painted_frame.get() != Some(frame) {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            animate(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            animate(hwnd);
            crate::desktop::refresh_materials(&surface.app);
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
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
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
        crate::desktop::pet_moved(&surface.app, rect.left, rect.top);
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
// Animation uses only the imported atlas.
unsafe fn paint(hwnd: HWND, surface: &Surface) {
    let mut ps = PAINTSTRUCT::default();
    BeginPaint(hwnd, &mut ps);
    let info = surface.info.borrow();
    let state = surface.pet_state.borrow();
    let frame = aieyes_core::pets::frame(
        &state,
        surface.pet_start.get().elapsed().as_millis() as u64,
        pet::reduced_motion(),
    );
    pet::paint(
        hwnd,
        &surface.pet_sprite.borrow(),
        &state,
        frame,
        surface.font.font,
        &info,
        &mut surface.renderer.borrow_mut(),
    );
    surface.painted_frame.set(Some(frame));
    let _ = EndPaint(hwnd, &ps);
}
