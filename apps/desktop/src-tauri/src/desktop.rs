use aieyes_core::{
    sessions::{Phase, SessionMonitor, Snapshot},
    store::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};

const TRAY_ID: &str = "aieyes-status";
const HIDE_MENU_ID: &str = "hide";
/// Linux WebView capsule size in logical pixels; Windows uses its own native window.
const BALL_SIZE: f64 = 144.0;
const BALL_HEIGHT: f64 = 44.0;
/// Panel size in logical pixels; height is clamped to the monitor work area.
const PANEL_WIDTH: f64 = 450.0;
const PANEL_HEIGHT: f64 = 720.0;
/// Distance to a work-area edge that makes a dragged ball snap, in logical pixels.
const SNAP_DISTANCE: f64 = 48.0;
/// Gap between the ball and the opened panel, in logical pixels.
const PANEL_GAP: f64 = 8.0;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Auto,
    Tray,
    Floating,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Preferences {
    mode: Mode,
    position: Option<Position>,
    show_count: bool,
}
struct ShellState {
    accent: String,
    preferences: Preferences,
    snapshot: Snapshot,
    floating: bool,
    visible: bool,
    hidden: bool,
    panel: bool,
    panel_page: String,
    panel_anchor: Option<Position>,
    pin: bool,
    reason: String,
    page: String,
    dirty: bool,
    native_interacting: bool,
    panel_generation: u64,
    recovery: String,
}
#[cfg(not(target_os = "windows"))]
impl ShellState {
    fn can_dismiss_panel(&self) -> bool {
        self.panel && !self.pin && !self.native_interacting
    }
}
pub struct Desktop {
    path: PathBuf,
    state: Mutex<ShellState>,
    menu: Menu<tauri::Wry>,
    status_item: MenuItem<tauri::Wry>,
    hide_item: MenuItem<tauri::Wry>,
    tray_available: AtomicBool,
    material: &'static str,
}

// Never hide the only usable entry point when tray support is absent.
fn use_floating(mode: Mode, windows: bool, tray_available: bool) -> bool {
    !tray_available || mode == Mode::Floating || (mode == Mode::Auto && windows)
}

/// Hiding the only entry point would strand the app; a usable tray is required.
fn can_hide(floating: bool, tray_available: bool) -> bool {
    floating && tray_available
}

fn snap_rect(
    position: Position,
    area: (i32, i32, u32, u32),
    ball: (i32, i32),
    threshold: i32,
) -> Position {
    let (x, y, width, height) = area;
    let right = x + width as i32 - ball.0;
    let bottom = y + height as i32 - ball.1;
    let mut snapped = position;
    if position.x - x <= threshold {
        snapped.x = x;
    } else if right - position.x <= threshold {
        snapped.x = right;
    }
    if position.y - y <= threshold {
        snapped.y = y;
    } else if bottom - position.y <= threshold {
        snapped.y = bottom;
    }
    clamp_rect(snapped, x, y, width, height, ball)
}

fn panel_rect(
    ball_pos: Position,
    ball: (i32, i32),
    panel: (i32, i32),
    area: (i32, i32, u32, u32),
    gap: i32,
) -> Position {
    let (x, y, width, height) = area;
    let (panel_width, panel_height) = panel;
    let mut left = if ball_pos.x + ball.0 / 2 >= x + width as i32 / 2 {
        ball_pos.x + ball.0 - panel_width
    } else {
        ball_pos.x
    };
    let below = ball_pos.y + ball.1 + gap;
    let above = ball_pos.y - panel_height - gap;
    let mut top = if ball_pos.y + ball.1 / 2 < y + height as i32 / 2 {
        below
    } else {
        above
    };
    if top + panel_height > y + height as i32 {
        top = above;
    }
    if top < y {
        top = below;
    }
    left = left.clamp(x, (x + width as i32 - panel_width).max(x));
    top = top.clamp(y, (y + height as i32 - panel_height).max(y));
    Position { x: left, y: top }
}

#[cfg(test)]
fn snap_position(p: Position, a: (i32, i32, u32, u32), b: i32, t: i32) -> Position {
    snap_rect(p, a, (b, b), t)
}
#[cfg(test)]
fn panel_position(p: Position, b: i32, s: (i32, i32), a: (i32, i32, u32, u32), g: i32) -> Position {
    panel_rect(p, (b, b), s, a, g)
}
#[cfg(test)]
fn clamp_position(p: Position, x: i32, y: i32, w: u32, h: u32, b: i32) -> Position {
    clamp_rect(p, x, y, w, h, (b, b))
}
fn panel_size(scale: f64, work_height: Option<u32>) -> (u32, u32) {
    let width = (PANEL_WIDTH * scale).round() as u32;
    let height = (PANEL_HEIGHT * scale).round() as u32;
    let margin = (24.0 * scale).round() as u32;
    let height = work_height.map_or(height, |work| {
        height.min(work.saturating_sub(margin).max(1))
    });
    (width, height)
}

/// Work areas and positions use physical desktop coordinates; the user's size
/// is logical so a 150% or 200% display must not be treated as a 1x desktop.
#[cfg(any(target_os = "windows", test))]
fn main_window_bounds(
    pos: Position,
    requested: (f64, f64),
    area: (i32, i32, u32, u32),
    scale: f64,
) -> (Position, (u32, u32)) {
    let (x, y, width, height) = area;
    let fitted_width = (requested.0 * scale).round().max(1.0) as u32;
    let fitted_height = (requested.1 * scale).round().max(1.0) as u32;
    let size = (fitted_width.min(width), fitted_height.min(height));
    let pos = Position {
        x: pos.x.clamp(x, x.saturating_add((width - size.0) as i32)),
        y: pos.y.clamp(y, y.saturating_add((height - size.1) as i32)),
    };
    (pos, size)
}

#[cfg(target_os = "windows")]
fn fit_main_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    // The window manager owns maximized/fullscreen bounds and Snap placement.
    if window.is_maximized()? || window.is_fullscreen()? {
        return Ok(());
    }
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let area = monitor.work_area();
    let scale = window.scale_factor()?;
    let original_pos = window.outer_position()?;
    let original_size = window.outer_size()?;
    let logical = original_size.to_logical::<f64>(scale);
    let (pos, size) = main_window_bounds(
        Position {
            x: original_pos.x,
            y: original_pos.y,
        },
        (logical.width, logical.height),
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        scale,
    );
    window.set_min_size(Some(tauri::LogicalSize::new(
        640.0_f64.min(f64::from(area.size.width) / scale),
        440.0_f64.min(f64::from(area.size.height) / scale),
    )))?;
    if size != (original_size.width, original_size.height) {
        window.set_size(PhysicalSize::new(size.0, size.1))?;
    }
    if (pos.x, pos.y) != (original_pos.x, original_pos.y) {
        window.set_position(PhysicalPosition::new(pos.x, pos.y))?;
    }
    Ok(())
}

fn main_window_material(_app: &tauri::App) -> &'static str {
    #[cfg(target_os = "windows")]
    if let Some(window) = _app.get_webview_window("main") {
        let dark = window.theme().ok().map(|theme| theme == tauri::Theme::Dark);
        // The direct API reports unsupported Windows versions; Tauri's effects list
        // does not fall through on errors. Keep the web surface opaque on failure.
        if window_vibrancy::apply_mica(&window, dark).is_ok() {
            return "mica";
        }
    }
    "opaque"
}

pub fn setup(app: &mut tauri::App, root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let material = main_window_material(app);
    let path = root.join("desktop.json");
    let preferences = std::fs::read(&path)
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default();
    let status_item = MenuItem::with_id(app, "status", "正在读取会话状态…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "打开 Aieyes", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let auto = MenuItem::with_id(
        app,
        "auto",
        "跟随系统（Windows 悬浮球 / Linux 状态栏）",
        true,
        None::<&str>,
    )?;
    let floating = MenuItem::with_id(app, "floating", "使用悬浮球", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset-position", "重置悬浮球位置", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, HIDE_MENU_ID, "暂时隐藏悬浮球", false, None::<&str>)?;
    let refresh = MenuItem::with_id(
        app,
        "refresh-floating",
        "重建面板（用于界面无响应）",
        true,
        None::<&str>,
    )?;
    let toggle = MenuItem::with_id(app, "toggle-panel", "打开／收起面板", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "toggle-pin", "固定／取消固定面板", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Aieyes", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &toggle,
            &pin,
            &refresh,
            &open,
            &settings,
            &auto,
            &floating,
            &reset,
            &hide,
            &quit,
        ],
    )?;
    app.manage(Desktop {
        path,
        state: Mutex::new(ShellState {
            accent: "indigo".into(),
            preferences,
            snapshot: Snapshot::default(),
            floating: false,
            visible: false,
            hidden: false,
            panel: false,
            panel_page: "agent".into(),
            panel_anchor: None,
            pin: false,
            reason: String::new(),
            page: "agent".into(),
            dirty: false,
            native_interacting: false,
            panel_generation: 0,
            recovery: String::new(),
        }),
        menu,
        status_item,
        hide_item: hide,
        tray_available: AtomicBool::new(false),
        material,
    });
    create_floating(app.handle(), 0)?;
    #[cfg(target_os = "windows")]
    {
        let position = app
            .state::<Desktop>()
            .state
            .lock()
            .ok()
            .and_then(|s| s.preferences.position)
            .map(|p| (p.x, p.y));
        app.manage(
            crate::capsule::Capsule::start(app.handle().clone(), position)
                .map_err(std::io::Error::other)?,
        );
        if let Some(main) = app.get_webview_window("main") {
            configure_webview(&main);
        }
    }
    let _ = restore_position(app.handle());
    apply_mode(app.handle()).map_err(std::io::Error::other)?;
    app.on_menu_event(|app, event| {
        if let Err(error) = action(app, event.id.as_ref()) {
            let _ = app.emit("desktop:error", error);
        }
    });
    // Independent connection: live status is not blocked by SSH, imports or quota requests.
    let store = Store::open(root)?;
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let mut monitor = SessionMonitor::default();
        let mut tick = 0u32;
        loop {
            let snapshot = match store.settings() {
                Ok(settings) => monitor.read(&settings.sources, std::time::SystemTime::now()),
                Err(_) => Snapshot {
                    unavailable: true,
                    ..Snapshot::default()
                },
            };
            let supported = if tick.is_multiple_of(3) {
                Some(tray_supported())
            } else {
                None
            };
            let app = handle.clone();
            if handle
                .run_on_main_thread(move || update(&app, snapshot, supported))
                .is_err()
            {
                break;
            }
            tick = tick.wrapping_add(1);
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("main") {
        let _ = fit_main_window(&window);
        window.show()?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn tray_supported() -> bool {
    true
}
#[cfg(target_os = "linux")]
fn tray_supported() -> bool {
    // libappindicator panics without its dynamic library. Probe before building a tray.
    let library = [
        "libayatana-appindicator3.so.1",
        "libappindicator3.so.1",
        "libayatana-appindicator3.so",
        "libappindicator3.so",
    ]
    .iter()
    .any(|name| unsafe { libloading::Library::new(name).is_ok() });
    if !library {
        return false;
    }
    // A watcher can exist with no registered host (GNOME without an indicator extension).
    let mut command = std::process::Command::new("gdbus");
    command.args([
        "call",
        "--session",
        "--dest",
        "org.kde.StatusNotifierWatcher",
        "--object-path",
        "/StatusNotifierWatcher",
        "--method",
        "org.freedesktop.DBus.Properties.Get",
        "org.kde.StatusNotifierWatcher",
        "IsStatusNotifierHostRegistered",
    ]);
    aieyes_core::process::run(command, vec![], Duration::from_secs(2))
        .is_ok_and(|out| String::from_utf8_lossy(&out).trim() == "(<true>,)")
}

fn create_floating(app: &AppHandle, generation: u64) -> tauri::Result<()> {
    let builder =
        WebviewWindowBuilder::new(app, "floating", WebviewUrl::App("floating.html".into()))
            .title("Aieyes 面板")
            .inner_size(
                if cfg!(windows) {
                    PANEL_WIDTH
                } else {
                    BALL_SIZE
                },
                if cfg!(windows) {
                    PANEL_HEIGHT
                } else {
                    BALL_HEIGHT
                },
            )
            .resizable(false)
            .decorations(false)
            .shadow(cfg!(windows))
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .zoom_hotkeys_enabled(false)
            .initialization_script(format!("window.AIEYES_SHELL_GENERATION = {generation};"))
            .on_navigation(|url| {
                matches!(url.scheme(), "tauri" | "http" | "https")
                    && matches!(url.host_str(), Some("tauri.localhost" | "localhost"))
            })
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.transparent(true);
    let window = builder.build()?;
    configure_webview(&window);
    #[cfg(target_os = "windows")]
    if let Ok(hwnd) = window.hwnd() {
        // Recreating the renderer may call this from a worker; configure on its UI thread.
        let handle = hwnd.0 as usize;
        window.run_on_main_thread(move || {
            crate::passive_window::configure(windows::Win32::Foundation::HWND(handle as *mut _));
        })?;
    }
    Ok(())
}
fn configure_webview(_window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    let _ = _window.with_webview(|view| unsafe {
        if let Ok(core) = view.controller().CoreWebView2()
            && let Ok(settings) = core.Settings()
        {
            let _ = settings.SetAreDefaultContextMenusEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
            #[cfg(not(debug_assertions))]
            let _ = settings.SetAreDevToolsEnabled(false);
        }
    });
}

#[cfg(target_os = "windows")]
pub(crate) fn native_interaction(app: &AppHandle, active: bool) {
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.native_interacting = active;
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn capsule_moved(app: &AppHandle, x: i32, y: i32) {
    let open = if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.preferences.position = Some(Position { x, y });
        state.dirty = true;
        state.panel
    } else {
        false
    };
    // Re-anchor an open panel after a drag without blocking the native message pump.
    if open {
        let app = app.clone();
        std::thread::spawn(move || {
            let generation = app
                .state::<Desktop>()
                .state
                .lock()
                .map(|s| s.panel_generation)
                .unwrap_or(0);
            let _ = request_native_panel(&app, PanelRequest::Restore(generation));
        });
    }
}
#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy)]
enum PanelRequest {
    Set(bool),
    Toggle,
    Dismiss,
    Restore(u64),
}
#[cfg(any(target_os = "windows", test))]
impl PanelRequest {
    fn resolve(self, open: bool, pinned: bool, interacting: bool, generation: u64) -> Option<bool> {
        match self {
            Self::Set(value) => Some(value),
            Self::Toggle => Some(!open),
            Self::Dismiss => (open && !pinned && !interacting).then_some(false),
            Self::Restore(expected) => (expected == generation).then_some(open),
        }
    }
}
#[cfg(target_os = "windows")]
fn set_native_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    request_native_panel(app, PanelRequest::Set(open))
}
#[cfg(target_os = "windows")]
fn request_native_panel(app: &AppHandle, request: PanelRequest) -> Result<(), String> {
    let app = app.clone();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let owner = app.clone();
    // Tauri executes inline on the UI thread. Resolve toggles/dismissals here,
    // not before enqueueing, so interleaved tray and capsule input stays ordered.
    owner
        .run_on_main_thread(move || {
            let result = (|| {
                let desired = {
                    let desktop = app.state::<Desktop>();
                    let state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
                    request.resolve(
                        state.panel,
                        state.pin,
                        state.native_interacting,
                        state.panel_generation,
                    )
                };
                if let Some(open) = desired {
                    set_native_panel_on_ui(&app, open)?;
                }
                Ok(())
            })();
            let _ = send.send(result);
        })
        .map_err(|e| e.to_string())?;
    receive.recv().map_err(|e| e.to_string())?
}
#[cfg(target_os = "windows")]
fn set_native_panel_on_ui(app: &AppHandle, open: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window("floating") else {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.recovery == "refreshing" {
            state.panel = open;
            drop(state);
            emit_shell(app);
            return Ok(());
        }
        return Err("面板不可用，可右键刷新悬浮窗".into());
    };
    if open {
        let capsule = app
            .try_state::<crate::capsule::Capsule>()
            .ok_or("悬浮入口不可用")?;
        let (x, y, ball_width, ball_height, scale) = capsule.geometry()?;
        let monitors = window.available_monitors().map_err(|e| e.to_string())?;
        let monitor = monitors
            .into_iter()
            .find(|m| {
                let a = m.work_area();
                x >= a.position.x
                    && y >= a.position.y
                    && x < a.position.x + a.size.width as i32
                    && y < a.position.y + a.size.height as i32
            })
            .or(window.primary_monitor().map_err(|e| e.to_string())?)
            .ok_or("屏幕不可用")?;
        let area = monitor.work_area();
        let (width, height) = panel_size(scale, Some(area.size.height));
        let width = width.min(area.size.width.saturating_sub((24.0 * scale) as u32).max(1));
        let pos = panel_rect(
            Position { x, y },
            (ball_width as i32, ball_height as i32),
            (width as i32, height as i32),
            (
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
            ),
            (PANEL_GAP * scale) as i32,
        );
        window
            .set_position(PhysicalPosition::new(pos.x, pos.y))
            .map_err(|e| e.to_string())?;
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
    }
    let handle = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    crate::passive_window::set_visible(windows::Win32::Foundation::HWND(handle as *mut _), open)?;
    if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
        capsule.watch_panel(open.then_some(handle));
    }
    app.state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .panel = open;
    emit_shell(app);
    Ok(())
}

/// Recreate the renderer, never the core or the native capsule. The menu callback
/// must return before Windows starts constructing another WebView2 controller.
fn refresh_floating(app: &AppHandle) -> Result<(), String> {
    let (generation, _open) = {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.recovery == "refreshing" {
            return Ok(());
        }
        state.panel_generation += 1;
        state.recovery = "refreshing".into();
        (state.panel_generation, state.panel)
    };
    emit_shell(app);
    #[cfg(target_os = "windows")]
    if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
        capsule.watch_panel(None);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            if let Some(window) = app.get_webview_window("floating") {
                window.destroy().map_err(|e| e.to_string())?;
            }
            // Wait for Tauri to remove the destroyed window's label before reuse.
            for _ in 0..100 {
                if app.get_webview_window("floating").is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            create_floating(&app, generation).map_err(|e| e.to_string())?;
            #[cfg(target_os = "windows")]
            {
                request_native_panel(&app, PanelRequest::Restore(generation))?;
            }
            #[cfg(not(target_os = "windows"))]
            {
                app.state::<Desktop>()
                    .state
                    .lock()
                    .map_err(|_| "显示状态不可用")?
                    .panel = false;
                restore_position(&app).map_err(|e| e.to_string())?;
                set_panel(&app, _open)?;
                let visible = app
                    .state::<Desktop>()
                    .state
                    .lock()
                    .map_err(|_| "显示状态不可用")?
                    .visible;
                if visible && let Some(window) = app.get_webview_window("floating") {
                    window.show().map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            recovery_failed(&app, generation, &error);
            return;
        }
        std::thread::sleep(Duration::from_secs(10));
        let waiting = app
            .state::<Desktop>()
            .state
            .lock()
            .is_ok_and(|s| s.panel_generation == generation && s.recovery == "refreshing");
        if waiting {
            recovery_failed(&app, generation, "面板恢复超时，可再次刷新悬浮窗");
        }
    });
    Ok(())
}
fn recovery_failed(app: &AppHandle, generation: u64, error: &str) {
    if let Ok(mut state) = app.state::<Desktop>().state.lock()
        && state.panel_generation == generation
    {
        state.recovery = "failed".into();
    }
    emit_shell(app);
    let _ = app.emit("desktop:error", error);
}
#[tauri::command]
pub fn desktop_panel_ready(app: AppHandle, generation: u64) -> Result<(), String> {
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if generation != state.panel_generation {
            return Ok(());
        }
        state.recovery.clear();
    }
    emit_shell(&app);
    Ok(())
}

fn create_tray(app: &AppHandle, desktop: &Desktop) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(None, false))
        .tooltip("Aieyes")
        .menu(&desktop.menu)
        // Linux uses the native menu; tray click events are not emitted there.
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = show_main(tray.app_handle(), "agent");
            }
        })
        .build(app)?;
    Ok(())
}

fn update(app: &AppHandle, snapshot: Snapshot, supported: Option<bool>) {
    let desktop = app.state::<Desktop>();
    let mut new_tray = false;
    if let Some(supported) = supported {
        new_tray = supported && app.tray_by_id(TRAY_ID).is_none();
        let available = supported && create_tray(app, &desktop).is_ok();
        desktop.tray_available.store(available, Ordering::Relaxed);
    }
    let summary = snapshot.summary();
    let (text_changed, phase_changed) = desktop
        .state
        .lock()
        .map(|state| {
            (
                state.snapshot.summary() != summary,
                state.snapshot.phase() != snapshot.phase(),
            )
        })
        .unwrap_or((true, true));
    let _ = desktop.status_item.set_text(format!("Aieyes · {summary}"));
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if text_changed || new_tray {
            let _ = tray.set_tooltip(Some(format!("Aieyes · {summary}")));
            let _ = tray.set_title(Some(&summary));
        }
        if phase_changed || new_tray {
            let _ = tray.set_icon(Some(icon(
                snapshot.phase(),
                app.get_webview_window("main")
                    .and_then(|w| w.theme().ok())
                    .is_some_and(|t| t == tauri::Theme::Dark),
            )));
        }
    }
    if let Ok(mut state) = desktop.state.lock() {
        state.snapshot = snapshot;
    }
    if let Err(error) = apply_mode(app) {
        let _ = app.emit("desktop:error", error);
    }
    if let Ok(info) = desktop_info(app.clone()) {
        #[cfg(target_os = "windows")]
        if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
            capsule.status(info.clone());
        }
        let _ = app.emit("desktop:status", info);
    }
    save(app);
}

fn apply_mode(app: &AppHandle) -> Result<(), String> {
    let desktop = app.state::<Desktop>();
    let available = desktop.tray_available.load(Ordering::Relaxed);
    let (floating, hidden, changed) = {
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        let floating = use_floating(state.preferences.mode, cfg!(windows), available);
        let hidden = state.hidden;
        let visible = floating && !hidden;
        let changed = visible != state.visible;
        state.reason = if !available {
            "未检测到可用状态栏，已使用悬浮球".into()
        } else if hidden {
            "悬浮球已暂时隐藏".into()
        } else {
            String::new()
        };
        state.floating = floating;
        state.visible = visible;
        (floating, hidden, changed)
    };
    if changed && (!floating || hidden) {
        set_panel(app, false)?;
    }
    #[cfg(target_os = "windows")]
    if changed && let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
        capsule.visible(floating && !hidden);
    }
    #[cfg(not(target_os = "windows"))]
    if changed && let Some(window) = app.get_webview_window("floating") {
        let result = if floating && !hidden {
            // Some Wayland compositors refuse absolute placement; the window must still open.
            let _ = restore_position(app);
            window.show()
        } else {
            window.hide()
        };
        if let Err(error) = result {
            let _ = show_main(app, "agent");
            return Err(error.to_string());
        }
    }
    // Keep a secondary tray entry on Windows, and always keep one while the ball is hidden.
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(available && (!floating || cfg!(windows) || hidden));
    }
    refresh_menu(app);
    Ok(())
}

#[tauri::command]
pub fn desktop_appearance(app: AppHandle, theme: String, accent: String) -> Result<(), String> {
    let theme = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        "system" => None,
        _ => return Err("无效的主题".into()),
    };
    if !["indigo", "blue", "teal", "purple"].contains(&accent.as_str()) {
        return Err("无效的强调色".into());
    }
    app.state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .accent = accent;
    app.set_theme(theme);
    for window in app.webview_windows().values() {
        let _ = window.set_theme(theme);
    }
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("main") {
        let dark = window.theme().ok().map(|theme| theme == tauri::Theme::Dark);
        let _ = window_vibrancy::apply_mica(&window, dark);
    }
    emit_shell(&app);
    Ok(())
}
#[tauri::command]
pub fn desktop_info(app: AppHandle) -> Result<Value, String> {
    let dark = app
        .get_webview_window("main")
        .and_then(|w| w.theme().ok())
        .is_some_and(|t| t == tauri::Theme::Dark);
    let desktop = app.state::<Desktop>();
    let state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
    Ok(json!({
        "platform": std::env::consts::OS, "material": desktop.material, "mode": state.preferences.mode,
        "effectiveMode": if state.floating { "floating" } else { "tray" },
        "trayAvailable": desktop.tray_available.load(Ordering::Relaxed), "reason": state.reason,
        "summary": state.snapshot.summary(), "phase": state.snapshot.phase(), "activeCount": state.snapshot.active_count(),
        "sessions": state.snapshot.sessions, "unavailable": state.snapshot.unavailable, "page": state.page,
        "hidden": state.hidden, "panelOpen": state.panel, "panelPinned": state.pin,
        "showCount": state.preferences.show_count, "panelPage": state.panel_page, "nativeCapsule": cfg!(windows),
        "recovery": state.recovery, "generation": state.panel_generation,
        "dark": dark,
        "accent": state.accent,
    }))
}
#[tauri::command]
pub fn desktop_mode(app: AppHandle, mode: Mode) -> Result<Value, String> {
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        let previous = state.preferences.mode;
        state.preferences.mode = mode;
        if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
            state.preferences.mode = previous;
            return Err(format!("显示偏好保存失败：{error}"));
        }
    }
    apply_mode(&app)?;
    let info = desktop_info(app.clone())?;
    let _ = app.emit("desktop:status", &info);
    Ok(info)
}
#[tauri::command]
pub fn desktop_panel(app: AppHandle, open: bool) -> Result<Value, String> {
    set_panel(&app, open)?;
    desktop_info(app)
}
#[tauri::command]
pub fn desktop_panel_pin(app: AppHandle, pinned: bool) -> Result<Value, String> {
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        state.pin = pinned;
    }
    emit_shell(&app);
    desktop_info(app)
}
#[tauri::command]
pub fn desktop_panel_page(app: AppHandle, page: String) -> Result<(), String> {
    if !matches!(page.as_str(), "agent" | "servers") {
        return Err("未知面板页面".into());
    }
    app.state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .panel_page = page;
    emit_shell(&app);
    Ok(())
}

#[tauri::command]
pub fn desktop_action(app: AppHandle, action: String) -> Result<(), String> {
    self::action(&app, &action)
}
#[tauri::command]
pub fn desktop_detail(app: AppHandle, view: Value) -> Result<(), String> {
    let mut filter = serde_json::Map::new();
    for key in ["provider", "sourceId", "accountKey", "model"] {
        if let Some(value) = view[key].as_str() {
            filter.insert(key.into(), json!(value));
        }
    }
    filter.insert(
        "page".into(),
        json!(if view["page"] == "servers" {
            "servers"
        } else {
            "agent"
        }),
    );
    filter.insert(
        "days".into(),
        json!(
            view["days"]
                .as_u64()
                .filter(|n| [1, 7, 30, 90, 365].contains(n))
                .unwrap_or(1)
        ),
    );
    filter.insert(
        "cost".into(),
        json!(view["cost"].as_bool().unwrap_or(false)),
    );
    show_main(&app, "agent")?;
    app.emit_to("main", "desktop:detail-view", filter)
        .map_err(|e| e.to_string())
}
pub(crate) fn action(app: &AppHandle, action: &str) -> Result<(), String> {
    match action {
        value if value.starts_with("edit-account:") => show_main(app, value),
        "focus-panel" => set_panel(app, true),
        "dismiss-panel" => {
            #[cfg(target_os = "windows")]
            return request_native_panel(app, PanelRequest::Dismiss);
            #[cfg(not(target_os = "windows"))]
            {
                let close = app
                    .state::<Desktop>()
                    .state
                    .lock()
                    .map_err(|_| "显示状态不可用")?
                    .can_dismiss_panel();
                if close {
                    set_panel(app, false)?;
                }
                Ok(())
            }
        }
        "refresh-floating" => refresh_floating(app),
        "toggle-panel" => {
            #[cfg(target_os = "windows")]
            return request_native_panel(app, PanelRequest::Toggle);
            #[cfg(not(target_os = "windows"))]
            {
                let open = app
                    .state::<Desktop>()
                    .state
                    .lock()
                    .map_err(|_| "显示状态不可用")?
                    .panel;
                set_panel(app, !open)
            }
        }
        "toggle-count" => {
            {
                let desktop = app.state::<Desktop>();
                let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
                let previous = state.preferences.show_count;
                state.preferences.show_count = !previous;
                if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
                    state.preferences.show_count = previous;
                    return Err(format!("显示偏好保存失败：{error}"));
                }
            }
            let info = desktop_info(app.clone())?;
            #[cfg(target_os = "windows")]
            if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
                capsule.status(info.clone());
            }
            let _ = app.emit("desktop:status", info);
            Ok(())
        }
        "toggle-pin" => {
            let pin = app
                .state::<Desktop>()
                .state
                .lock()
                .map_err(|_| "显示状态不可用")?
                .pin;
            desktop_panel_pin(app.clone(), !pin).map(|_| ())
        }
        "open" => show_main(app, "agent"),
        "settings" => show_main(app, "settings"),
        "prices" => show_main(app, "prices"),
        "add-source" => show_main(app, "add-source"),
        "add-host" => show_main(app, "add-host"),
        "add-quota" => show_main(app, "add-quota"),
        "auto" => desktop_mode(app.clone(), Mode::Auto).map(|_| ()),
        "floating" => desktop_mode(app.clone(), Mode::Floating).map(|_| ()),
        "reset-position" => {
            {
                let desktop = app.state::<Desktop>();
                let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
                state.preferences.position = None;
                state.dirty = true;
            }
            restore_position(app).map_err(|e| e.to_string())
        }
        "menu" => app
            .get_webview_window("floating")
            .ok_or("悬浮球不可用")?
            .popup_menu(&app.state::<Desktop>().menu)
            .map_err(|e| e.to_string()),
        "snap" => snap_ball(app),
        HIDE_MENU_ID => {
            let hidden = app
                .state::<Desktop>()
                .state
                .lock()
                .map_err(|_| "显示状态不可用")?
                .hidden;
            set_hidden(app, !hidden)
        }
        "hide-ball" => set_hidden(app, true),
        "show-ball" => set_hidden(app, false),
        "quit" => {
            app.emit_to("main", "desktop:quit-requested", ())
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        "quit-confirmed" => {
            save(app);
            app.exit(0);
            Ok(())
        }
        _ => Err("未知操作".into()),
    }
}
fn show_main(app: &AppHandle, page: &str) -> Result<(), String> {
    let _ = set_panel(app, false);
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.page = page.into();
    }
    let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
    window
        .show()
        .and_then(|_| window.unminimize())
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    let _ = fit_main_window(&window);
    window.set_focus().map_err(|e| e.to_string())?;
    let _ = window.emit("desktop:navigate", page);
    Ok(())
}

fn clamp_rect(
    pos: Position,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    ball: (i32, i32),
) -> Position {
    Position {
        x: pos
            .x
            .clamp(x, (x.saturating_add(width as i32) - ball.0).max(x)),
        y: pos
            .y
            .clamp(y, (y.saturating_add(height as i32) - ball.1).max(y)),
    }
}
fn restore_position(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "windows")]
    {
        if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
            let position = app
                .state::<Desktop>()
                .state
                .lock()
                .ok()
                .and_then(|s| s.preferences.position);
            capsule.position(position.map(|p| (p.x, p.y)));
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    restore_web_position(app)
}
#[cfg(not(target_os = "windows"))]
fn restore_web_position(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("floating") else {
        return Ok(());
    };
    let saved = app
        .state::<Desktop>()
        .state
        .lock()
        .ok()
        .and_then(|s| s.preferences.position);
    let monitors = window.available_monitors()?;
    let monitor = saved
        .and_then(|pos| {
            monitors
                .iter()
                .find(|m| {
                    let a = m.work_area();
                    pos.x >= a.position.x
                        && pos.y >= a.position.y
                        && pos.x < a.position.x + a.size.width as i32
                        && pos.y < a.position.y + a.size.height as i32
                })
                .cloned()
        })
        .or(window.primary_monitor()?);
    if let Some(monitor) = monitor {
        let a = monitor.work_area();
        let ball = (BALL_SIZE * monitor.scale_factor()).ceil() as i32;
        let ball_height = (BALL_HEIGHT * monitor.scale_factor()).ceil() as i32;
        let default = Position {
            x: a.position.x + a.size.width as i32 - ball - 24,
            y: a.position.y + (a.size.height as i32 - ball_height) / 2,
        };
        let pos = clamp_rect(
            saved.unwrap_or(default),
            a.position.x,
            a.position.y,
            a.size.width,
            a.size.height,
            (ball, ball_height),
        );
        window.set_position(PhysicalPosition::new(pos.x, pos.y))?;
    }
    Ok(())
}

fn refresh_menu(app: &AppHandle) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    if let Ok(state) = desktop.state.lock() {
        let text = if state.hidden {
            "恢复显示悬浮球"
        } else {
            "暂时隐藏悬浮球"
        };
        let _ = desktop.hide_item.set_text(text);
        let enabled = state.hidden
            || can_hide(
                state.floating,
                desktop.tray_available.load(Ordering::Relaxed),
            );
        let _ = desktop.hide_item.set_enabled(enabled);
    }
}
fn emit_shell(app: &AppHandle) {
    if let Ok(info) = desktop_info(app.clone()) {
        let _ = app.emit("desktop:panel", info.clone());
        #[cfg(target_os = "windows")]
        if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
            capsule.status(info.clone());
        }
        let _ = app.emit("desktop:status", info);
    }
}
/// Expand the floating window into the compact panel, or collapse it back to the ball.
fn set_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        set_native_panel(app, open)
    }
    #[cfg(not(target_os = "windows"))]
    set_web_panel(app, open)
}
#[cfg(not(target_os = "windows"))]
fn set_web_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    let desktop = app.state::<Desktop>();
    {
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.panel == open {
            return Ok(());
        }
        state.panel = open;
    }
    let window = app.get_webview_window("floating").ok_or("悬浮球不可用")?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let ball = (BALL_SIZE * scale).round() as i32;
    let ball_height = (BALL_HEIGHT * scale).round() as i32;
    if open {
        let position = window.outer_position().map_err(|error| error.to_string())?;
        if let Ok(mut state) = desktop.state.lock() {
            state.panel_anchor = Some(Position {
                x: position.x,
                y: position.y,
            });
        }
        let monitor = window
            .current_monitor()
            .map_err(|error| error.to_string())?
            .or(window
                .primary_monitor()
                .map_err(|error| error.to_string())?);
        let area = monitor.map(|monitor| {
            let area = monitor.work_area();
            (
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
            )
        });
        let (width, height) = panel_size(scale, area.map(|(_, _, _, height)| height));
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(|error| error.to_string())?;
        if let Some(area) = area {
            let gap = (PANEL_GAP * scale).round() as i32;
            let position = panel_rect(
                Position {
                    x: position.x,
                    y: position.y,
                },
                (ball, ball_height),
                (width as i32, height as i32),
                area,
                gap,
            );
            let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
        }
    } else {
        window
            .set_size(PhysicalSize::new(ball as u32, ball_height as u32))
            .map_err(|error| error.to_string())?;
        let _ = restore_position(app);
        if let Ok(mut state) = desktop.state.lock() {
            state.panel_anchor = None;
        }
    }
    emit_shell(app);
    Ok(())
}
/// Temporarily hide the ball without changing the display mode; restart or the tray restores it.
fn set_hidden(app: &AppHandle, hidden: bool) -> Result<(), String> {
    let desktop = app.state::<Desktop>();
    {
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.hidden == hidden {
            return Ok(());
        }
        if hidden
            && !can_hide(
                state.floating,
                desktop.tray_available.load(Ordering::Relaxed),
            )
        {
            return Err("没有其它入口，无法隐藏悬浮球".into());
        }
        state.hidden = hidden;
    }
    if hidden {
        set_panel(app, false)?;
    }
    apply_mode(app)?;
    emit_shell(app);
    Ok(())
}
/// Snap the ball flush to a nearby work-area edge after a drag.
fn snap_ball(app: &AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("floating").ok_or("悬浮球不可用")?;
    if app
        .state::<Desktop>()
        .state
        .lock()
        .map(|state| state.panel)
        .unwrap_or(true)
    {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let ball = (BALL_SIZE * scale).round() as i32;
    let ball_height = (BALL_HEIGHT * scale).round() as i32;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or(window
            .primary_monitor()
            .map_err(|error| error.to_string())?);
    let Some(area) = monitor.map(|monitor| {
        let area = monitor.work_area();
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        )
    }) else {
        return Ok(());
    };
    let threshold = (SNAP_DISTANCE * scale).round() as i32;
    let snapped = snap_rect(
        Position {
            x: position.x,
            y: position.y,
        },
        area,
        (ball, ball_height),
        threshold,
    );
    if snapped.x != position.x || snapped.y != position.y {
        window
            .set_position(PhysicalPosition::new(snapped.x, snapped.y))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::ThemeChanged(theme) if window.label() == "main" => {
            #[cfg(target_os = "windows")]
            let _ = window_vibrancy::apply_mica(window, Some(*theme == tauri::Theme::Dark));
            let app = window.app_handle();
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                let phase = app
                    .state::<Desktop>()
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.snapshot.phase());
                let _ = tray.set_icon(Some(icon(phase, *theme == tauri::Theme::Dark)));
            }
            emit_shell(window.app_handle());
        }
        #[cfg(target_os = "windows")]
        tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "main" => {
            if let Some(main) = window.app_handle().get_webview_window("main") {
                let _ = fit_main_window(&main);
            }
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if window.label() == "main" {
                // Always make the fallback visible before hiding the details window.
                let app = window.app_handle();
                let ready = if let Some(_ball) = app.get_webview_window("floating") {
                    if app
                        .state::<Desktop>()
                        .state
                        .lock()
                        .map(|s| s.floating && !s.hidden)
                        .unwrap_or(true)
                    {
                        #[cfg(target_os = "windows")]
                        {
                            if let Some(capsule) = app.try_state::<crate::capsule::Capsule>() {
                                capsule.visible(true);
                            }
                            true
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            _ball.show().is_ok()
                        }
                    } else {
                        true
                    }
                } else {
                    false
                };
                if ready {
                    let _ = window.hide();
                }
            } else if window.label() == "floating" {
                let _ = set_panel(window.app_handle(), false);
            }
        }
        tauri::WindowEvent::Moved(pos) if window.label() == "floating" => {
            if let Some(desktop) = window.app_handle().try_state::<Desktop>()
                && let Ok(mut state) = desktop.state.lock()
                // Panel placement must never overwrite the ball's saved position.
                && !cfg!(windows) && !state.panel
            {
                state.preferences.position = Some(Position { x: pos.x, y: pos.y });
                state.dirty = true;
            }
        }
        #[cfg(not(target_os = "windows"))]
        tauri::WindowEvent::Focused(false) if window.label() == "floating" => {
            let app = window.app_handle().clone();
            // Let an owned menu/dialog activation settle before dismissing the Web capsule.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                if app
                    .get_webview_window("floating")
                    .is_some_and(|w| w.is_focused().unwrap_or(false))
                {
                    return;
                }
                let close = app
                    .try_state::<Desktop>()
                    .and_then(|desktop| {
                        desktop
                            .state
                            .lock()
                            .ok()
                            .map(|state| state.can_dismiss_panel())
                    })
                    .unwrap_or(false);
                if close {
                    let _ = set_panel(&app, false);
                }
            });
        }
        tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "floating" => {
            let app = window.app_handle();
            let panel = app
                .try_state::<Desktop>()
                .and_then(|desktop| desktop.state.lock().ok().map(|state| state.panel))
                .unwrap_or(false);
            if !panel {
                let _ = restore_position(app);
            }
        }
        _ => {}
    }
}
fn write_preferences(path: &Path, preferences: &Preferences) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(preferences)?)
}
pub fn save(app: &AppHandle) {
    if let Some(desktop) = app.try_state::<Desktop>()
        && let Ok(mut state) = desktop.state.lock()
        && state.dirty
        && write_preferences(&desktop.path, &state.preferences).is_ok()
    {
        state.dirty = false;
    }
}
fn icon(phase: Option<Phase>, dark: bool) -> tauri::image::Image<'static> {
    let key = match phase {
        Some(Phase::Working) => "working",
        Some(Phase::Thinking) => "thinking",
        Some(Phase::Tool) => "tool",
        Some(Phase::Complete) => "complete",
        Some(Phase::Interrupted) => "interrupted",
        Some(Phase::Unknown) => "unknown",
        None => "idle",
    };
    let [r, g, b] = crate::phase_colors::rgb(key, dark);
    let color = [r, g, b, 255];
    let mut rgba = include_bytes!("../icons/brand.rgba").to_vec();
    // Live state is a separate top-right badge, not a recoloring of the brand.
    if phase.is_some() {
        for y in 0..12 {
            for x in 20..32 {
                let dx = x as f64 - 25.5;
                let dy = y as f64 - 5.5;
                if dx * dx + dy * dy <= 25.0 {
                    let ink = if dx * dx + dy * dy > 16.0 {
                        [20, 30, 50, 255]
                    } else {
                        color
                    };
                    rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4].copy_from_slice(&ink);
                }
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_requests_resolve_at_execution_and_ignore_stale_rebuilds() {
        assert_eq!(
            PanelRequest::Set(true).resolve(false, false, false, 1),
            Some(true)
        );
        assert_eq!(
            PanelRequest::Toggle.resolve(true, false, false, 1),
            Some(false)
        );
        assert_eq!(
            PanelRequest::Dismiss.resolve(true, false, false, 1),
            Some(false)
        );
        assert_eq!(PanelRequest::Dismiss.resolve(true, true, false, 1), None);
        assert_eq!(PanelRequest::Dismiss.resolve(true, false, true, 1), None);
        assert_eq!(PanelRequest::Dismiss.resolve(false, false, false, 1), None);
        assert_eq!(
            PanelRequest::Restore(1).resolve(false, false, false, 1),
            Some(false)
        );
        assert_eq!(
            PanelRequest::Restore(1).resolve(true, false, false, 2),
            None
        );
    }
    #[test]
    fn rectangular_capsules_use_their_height_for_bottom_edges_and_panel_anchors() {
        let area = (-1920, 40, 1920, 1040);
        let position = snap_rect(Position { x: -170, y: 1010 }, area, (144, 44), 48);
        assert_eq!((position.x, position.y), (-144, 1036));
        let panel = panel_rect(Position { x: -1900, y: 60 }, (144, 44), (450, 720), area, 8);
        assert_eq!((panel.x, panel.y), (-1900, 112));
        let narrow = clamp_rect(Position { x: 50, y: 100 }, 0, 0, 100, 40, (144, 44));
        assert_eq!((narrow.x, narrow.y), (0, 0));
    }
    #[test]
    fn tray_reuses_brand_pixels_and_only_overlays_the_status_badge() {
        let brand = icon(None, false);
        let active = icon(Some(Phase::Working), false);
        assert_eq!(brand.rgba(), include_bytes!("../icons/brand.rgba"));
        assert_eq!(brand.rgba().len(), 4096);
        assert_eq!(&brand.rgba()[12 * 32 * 4..], &active.rgba()[12 * 32 * 4..]);
        assert_ne!(brand.rgba(), active.rgba());
    }
    #[test]
    fn platform_bundle_configurations_are_valid() {
        let base: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        serde_json::from_value::<tauri::Config>(base.clone()).unwrap();
        for config in [
            include_str!("../tauri.windows.conf.json"),
            include_str!("../tauri.linux.conf.json"),
        ] {
            // Platform files are overlays, not complete configs (identifier comes from the base).
            let mut merged = base.clone();
            let overlay: Value = serde_json::from_str(config).unwrap();
            merged["bundle"]
                .as_object_mut()
                .unwrap()
                .extend(overlay["bundle"].as_object().unwrap().clone());
            if let Some(app) = overlay.get("app") {
                merged["app"]
                    .as_object_mut()
                    .unwrap()
                    .extend(app.as_object().unwrap().clone());
            }
            let parsed = serde_json::from_value::<tauri::Config>(merged).unwrap();
            if overlay.get("app").is_some() {
                assert!(parsed.app.windows[0].transparent);
                assert!(!parsed.app.windows[0].decorations);
                assert_eq!(parsed.app.windows[0].label, "main");
            }
        }
    }
    #[test]
    fn platform_defaults_and_tray_failure_always_leave_an_entry_point() {
        assert!(use_floating(Mode::Auto, true, true));
        assert!(!use_floating(Mode::Auto, false, true));
        for mode in [Mode::Auto, Mode::Tray, Mode::Floating] {
            for windows in [true, false] {
                assert!(use_floating(mode, windows, false));
            }
        }
        assert!(use_floating(Mode::Floating, false, true));
        assert!(!use_floating(Mode::Tray, true, true));
    }
    #[test]
    fn position_clamps_on_negative_monitors_and_small_work_areas() {
        let p = clamp_position(Position { x: 5000, y: -500 }, -1920, 40, 1920, 1040, 132);
        assert_eq!((p.x, p.y), (-132, 40));
        let p = clamp_position(Position { x: 500, y: 500 }, 0, 0, 64, 64, 88);
        assert_eq!((p.x, p.y), (0, 0));
    }
    #[test]
    fn snapping_sticks_to_near_edges_and_keeps_free_positions() {
        let area = (0, 0, 1920, 1040);
        let ball = 88;
        let p = snap_position(
            Position {
                x: 1920 - ball - 30,
                y: 300,
            },
            area,
            ball,
            48,
        );
        assert_eq!((p.x, p.y), (1920 - ball, 300));
        let p = snap_position(Position { x: 20, y: 10 }, area, ball, 48);
        assert_eq!((p.x, p.y), (0, 0));
        let p = snap_position(Position { x: 800, y: 500 }, area, ball, 48);
        assert_eq!((p.x, p.y), (800, 500));
        let p = snap_position(
            Position { x: -1900, y: 10 },
            (-1920, 0, 1920, 1040),
            ball,
            48,
        );
        assert_eq!((p.x, p.y), (-1920, 0));
    }
    #[test]
    fn panel_opens_toward_room_and_stays_inside_the_work_area() {
        let area = (0, 0, 1920, 1040);
        let ball = 88;
        let panel = (420, 640);
        let p = panel_position(
            Position {
                x: 1920 - ball - 24,
                y: 120,
            },
            ball,
            panel,
            area,
            8,
        );
        assert_eq!(p.x, 1920 - ball - 24 + ball - 420);
        assert_eq!(p.y, 120 + ball + 8);
        let p = panel_position(Position { x: 24, y: 900 }, ball, panel, area, 8);
        assert_eq!((p.x, p.y), (24, 900 - 640 - 8));
        let p = panel_position(
            Position { x: 300, y: 300 },
            ball,
            (420, 460),
            (0, 0, 800, 500),
            8,
        );
        assert_eq!((p.x, p.y), (300, 40));
        assert_eq!(panel_size(1.0, Some(1040)), (450, 720));
        assert_eq!(panel_size(2.0, Some(700)), (900, 652));
    }
    #[test]
    fn hiding_the_ball_requires_another_entry_point() {
        assert!(can_hide(true, true));
        assert!(!can_hide(true, false));
        assert!(!can_hide(false, true));
    }
    #[test]
    fn main_window_fits_small_high_dpi_work_areas_without_enlarging_user_sizes() {
        let (pos, size) = main_window_bounds(
            Position { x: 180, y: 90 },
            (1120.0, 800.0),
            (0, 0, 1366, 708),
            1.5,
        );
        assert_eq!((pos.x, pos.y), (0, 0));
        assert_eq!(size, (1366, 708));
        let (pos, size) = main_window_bounds(
            Position { x: 120, y: 20 },
            (640.0, 440.0),
            (0, 0, 1366, 708),
            1.5,
        );
        assert_eq!((pos.x, pos.y), (120, 20));
        assert_eq!(size, (960, 660));
    }
    #[test]
    fn main_window_rehomes_negative_origin_and_mixed_dpi_bounds() {
        let area = (-1920, 40, 1920, 1000);
        let (pos, size) =
            main_window_bounds(Position { x: 100, y: -100 }, (800.0, 600.0), area, 2.0);
        assert_eq!((pos.x, pos.y), (-1600, 40));
        assert_eq!(size, (1600, 1000));
        let (_, size) = main_window_bounds(Position { x: -1700, y: 80 }, (800.0, 600.0), area, 1.0);
        assert_eq!(size, (800, 600));
    }
}
