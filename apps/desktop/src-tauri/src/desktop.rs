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
#[cfg(not(target_os = "linux"))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "windows")]
use tauri::tray::MouseButtonState;
#[cfg(not(target_os = "linux"))]
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

#[cfg(not(target_os = "linux"))]
const TRAY_ID: &str = "aieyes-status";
const HIDE_MENU_ID: &str = "hide";
/// Transparent pet surface in logical pixels on every platform.
const BALL_HEIGHT: f64 = 168.0;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn entry_size(app: &AppHandle) -> (f64, f64) {
    app.try_state::<Desktop>()
        .and_then(|desktop| {
            desktop.state.lock().ok().map(|state| {
                let scale = aieyes_core::pets::normalized_scale(state.preferences.pet_scale);
                (
                    144.0 * scale,
                    aieyes_core::pets::height(&aieyes_core::pets::project(&state.snapshot)) * scale,
                )
            })
        })
        .unwrap_or((144.0, BALL_HEIGHT))
}
/// Panel size in logical pixels; height is clamped to the monitor work area.
const PANEL_WIDTH: f64 = 450.0;
const PANEL_HEIGHT: f64 = 720.0;
/// Distance to a work-area edge that makes a dragged ball snap, in logical pixels.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const SNAP_DISTANCE: f64 = 48.0;
/// Gap between the ball and the opened panel, in logical pixels.
const PANEL_GAP: f64 = 8.0;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    #[serde(alias = "auto", alias = "floating")]
    Pet,
    Tray,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Preferences {
    mode: Mode,
    position: Option<Position>,
    show_count: bool,
    hidden: bool,
    pet_scale: f64,
    pet_uniform_timing: bool,
    main_material: String,
    floating_material: String,
    main_window: Option<MainWindowPreferences>,
    background_hint_shown: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            position: None,
            show_count: false,
            hidden: false,
            pet_scale: 1.0,
            pet_uniform_timing: false,
            main_material: "default".into(),
            floating_material: "default".into(),
            main_window: None,
            background_hint_shown: false,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainWindowPreferences {
    width: f64,
    height: f64,
    maximized: bool,
    position: Option<Position>,
}
struct ShellState {
    accent: String,
    preferences: Preferences,
    snapshot: Snapshot,
    pet_asset: Value,
    floating: bool,
    visible: bool,
    hidden: bool,
    panel: bool,
    panel_page: String,
    pin: bool,
    reason: String,
    page: String,
    dirty: bool,
    native_interacting: bool,
    panel_generation: u64,
    panel_epoch: u64,
    recovery: String,
}
#[cfg(not(target_os = "windows"))]
impl ShellState {
    fn can_dismiss_panel(&self) -> bool {
        self.panel && !self.pin && !self.native_interacting
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn panel_generation(app: &AppHandle) -> (u64, u64) {
    app.state::<Desktop>()
        .state
        .lock()
        .map(|s| (s.panel_generation, s.panel_epoch))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
pub(crate) fn outside_click(app: &AppHandle, x: i32, y: i32, generation: (u64, u64)) {
    #[cfg(debug_assertions)]
    eprintln!(
        "Linux outside click: x={x} y={y} generation={generation:?} current={:?}",
        panel_generation(app)
    );
    if generation != panel_generation(app) {
        return;
    }
    if !app
        .state::<Desktop>()
        .state
        .lock()
        .is_ok_and(|s| s.panel && !s.native_interacting)
    {
        return;
    }
    let inside = |left: i32, top: i32, width: u32, height: u32| {
        x >= left
            && y >= top
            && i64::from(x) < i64::from(left) + i64::from(width)
            && i64::from(y) < i64::from(top) + i64::from(height)
    };
    if let Some(pet) = app.try_state::<crate::native_pet::NativePet>()
        && let Ok((left, top, width, height, _)) = pet.geometry()
        && inside(left, top, width, height)
    {
        return;
    }
    if let Some(panel) = app.get_webview_window("floating")
        && let (Ok(position), Ok(size)) = (panel.outer_position(), panel.outer_size())
        && inside(position.x, position.y, size.width, size.height)
    {
        return;
    }
    // Even a pinned panel must stop trying to acquire focus after the user
    // deliberately clicks elsewhere.
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.panel_epoch = state.panel_epoch.wrapping_add(1);
    }
    let _ = request_native_panel(app, PanelRequest::Dismiss);
}
struct WindowMaterial {
    kind: &'static str,
    #[cfg(target_os = "windows")]
    applied: Option<(usize, bool, bool, String)>,
}
impl Default for WindowMaterial {
    fn default() -> Self {
        Self {
            kind: "opaque",
            #[cfg(target_os = "windows")]
            applied: None,
        }
    }
}
#[derive(Default)]
struct Materials {
    main: WindowMaterial,
    floating: WindowMaterial,
}

/// Windows notification-icon clicks open the panel like a system flyout.
#[cfg(target_os = "windows")]
#[derive(Default)]
struct TrayClicks {
    /// Placement and activation for the next panel change; consumed by it.
    anchor: Option<(i32, i32, u32, u32)>,
    focus: bool,
    /// Last known icon rect, for opening the panel from the tray menu.
    icon: Option<(i32, i32, u32, u32)>,
    /// The open panel came from the tray and holds keyboard focus.
    activated: bool,
    /// Pressing the icon dismisses an open panel through the outside-click hook
    /// before the button is released; that release must not reopen it.
    closed: Option<std::time::Instant>,
    /// The trailing release of a double click must not reopen the panel.
    double: Option<std::time::Instant>,
}
#[cfg(target_os = "windows")]
const TRAY_CLICK_GRACE: Duration = Duration::from_millis(500);

pub struct Desktop {
    path: PathBuf,
    state: Mutex<ShellState>,
    #[cfg(target_os = "windows")]
    tray_clicks: Mutex<TrayClicks>,
    #[cfg(not(target_os = "linux"))]
    menu: Mutex<Menu<tauri::Wry>>,
    menu_state: Mutex<Option<(bool, bool, bool)>>,
    sampling: AtomicBool,
    tray_available: AtomicBool,
    materials: Mutex<Materials>,
    #[cfg(target_os = "linux")]
    _tray: Mutex<Option<crate::linux_tray::LinuxTrayHandle>>,
    // Tauri removes a tray icon when its last handle is dropped. Keep the
    // original handle for the lifetime of the app; the resource table clone is
    // not sufficient on the AppIndicator backend.
    #[cfg(not(target_os = "linux"))]
    _tray: Mutex<Option<TrayIcon<tauri::Wry>>>,
}

// Never hide the only usable entry point when tray support is absent.
fn use_floating(mode: Mode, _windows: bool, tray_available: bool) -> bool {
    !tray_available || mode == Mode::Pet
}

/// Hiding the only entry point would strand the app; a usable tray is required.
fn can_hide(floating: bool, tray_available: bool) -> bool {
    floating && tray_available
}

#[cfg(any(not(any(target_os = "windows", target_os = "linux")), test))]
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
    // A tall panel cannot fit above/below a pet_window midway down a small screen.
    // Prefer the free horizontal side before clamping into the pet_window itself.
    if below + panel_height > y + height as i32 && above < y {
        let right = ball_pos.x + ball.0 + gap;
        let left_side = ball_pos.x - panel_width - gap;
        let side = if left_side >= x {
            Some(left_side)
        } else if right + panel_width <= x + width as i32 {
            Some(right)
        } else {
            None
        };
        if let Some(side) = side {
            return Position {
                x: side,
                y: (ball_pos.y + ball.1 / 2 - panel_height / 2)
                    .clamp(y, (y + height as i32 - panel_height).max(y)),
            };
        }
    }
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
#[cfg(any(target_os = "windows", target_os = "linux", test))]
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
thread_local! {
    /// The main window's move/size loop: (inside the loop, DPI changed during it).
    static MAIN_SIZE_MOVE: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
}

/// Dragging across monitors changes DPI inside the move/size loop. Fitting the window to the new
/// monitor there would pull it away from the cursor, so the fit waits for the loop to end.
#[cfg(target_os = "windows")]
fn fit_main_after_size_move(hwnd: windows::Win32::Foundation::HWND, app: AppHandle) {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE},
        },
    };
    unsafe extern "system" fn subclass(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        match msg {
            WM_ENTERSIZEMOVE => MAIN_SIZE_MOVE.set((true, false)),
            WM_EXITSIZEMOVE if MAIN_SIZE_MOVE.replace((false, false)).1 => {
                // SAFETY: `data` is the AppHandle leaked below, alive for the process.
                let app = unsafe { &*(data as *const AppHandle) };
                if let Some(main) = app.get_webview_window("main") {
                    let _ = fit_main_window(&main);
                }
            }
            WM_EXITSIZEMOVE => MAIN_SIZE_MOVE.set((false, false)),
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
    // The main window is hidden on close, never destroyed, so the handle lives as long as the app.
    let data = Box::into_raw(Box::new(app)) as usize;
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass), 1, data);
    }
}

/// The page reveals the main window once its shell is laid out, instead of the window appearing
/// as an empty frame while WebView2 loads. Only the first reveal counts: a later page reload must
/// not reopen a window the user has closed.
#[cfg(target_os = "windows")]
static MAIN_REVEALED: AtomicBool = AtomicBool::new(false);
/// A frame painted while the window was hidden is not reused once it shows, so the page's reveal
/// shows the window cloaked (DWM composes it off screen) and the next painted frame uncloaks it.
#[cfg(target_os = "windows")]
static MAIN_CLOAKED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "windows")]
fn reveal_main(app: &AppHandle, cloaked: bool) {
    if MAIN_REVEALED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if cloaked && let Ok(hwnd) = window.hwnd() {
        cloak(windows::Win32::Foundation::HWND(hwnd.0 as _), true);
        MAIN_CLOAKED.store(true, Ordering::SeqCst);
        // Uncloak even if the page never reports a painted frame.
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || uncloak_main(&handle));
        });
    }
    let _ = window.show();
}

#[cfg(target_os = "windows")]
fn uncloak_main(app: &AppHandle) {
    if MAIN_CLOAKED.swap(false, Ordering::SeqCst)
        && let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok())
    {
        cloak(windows::Win32::Foundation::HWND(hwnd.0 as _), false);
    }
}

#[cfg(target_os = "windows")]
fn cloak(hwnd: windows::Win32::Foundation::HWND, cloaked: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let value = i32::from(cloaked);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        );
    }
}

/// `painted: false` once the page shell is laid out (shows the window, cloaked); `true` after a
/// frame painted in the shown window (uncloaks it).
#[tauri::command]
pub fn desktop_main_ready(app: AppHandle, painted: bool) {
    #[cfg(target_os = "windows")]
    if painted {
        uncloak_main(&app);
    } else {
        reveal_main(&app, true);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (app, painted);
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

#[cfg(target_os = "windows")]
fn apply_window_material(window: &tauri::WebviewWindow) -> bool {
    let Some(desktop) = window.app_handle().try_state::<Desktop>() else {
        return false;
    };
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    let dark = window.theme().ok() == Some(tauri::Theme::Dark);
    let effects = crate::material::effects_enabled();
    let requested = {
        let Ok(state) = desktop.state.lock() else {
            return false;
        };
        if window.label() == "main" {
            state.preferences.main_material.clone()
        } else {
            state.preferences.floating_material.clone()
        }
    };
    let identity = (hwnd.0 as usize, dark, effects, requested.clone());
    {
        let Ok(materials) = desktop.materials.lock() else {
            return false;
        };
        let current = if window.label() == "main" {
            &materials.main
        } else {
            &materials.floating
        };
        if current.applied.as_ref() == Some(&identity) {
            return false;
        }
    }
    let kind = crate::material::apply(
        windows::Win32::Foundation::HWND(hwnd.0 as _),
        dark,
        effects,
        window.label() == "main",
        &requested,
    );
    if let Ok(mut materials) = desktop.materials.lock() {
        let current = if window.label() == "main" {
            &mut materials.main
        } else {
            &mut materials.floating
        };
        current.kind = kind;
        current.applied = Some(identity);
    }
    true
}

/// Called by application appearance and Windows setting-change broadcasts.
/// Cache the HWND/theme/accessibility tuple to avoid a WM_THEMECHANGED feedback loop.
#[cfg(target_os = "windows")]
pub(crate) fn refresh_materials(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut changed = false;
        for label in ["main", "floating"] {
            if let Some(window) = handle.get_webview_window(label) {
                changed |= apply_window_material(&window);
            }
        }
        if changed {
            emit_shell(&handle);
        }
    });
}

pub fn setup(app: &mut tauri::App, root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // The tray menu and native pet are drawn before a page reports the appearance (desktop_appearance);
    // start them in the saved language and theme instead of Chinese and the system theme.
    if let Ok(settings) = Store::open(root).and_then(|store| store.settings()) {
        crate::localization::set_language(&settings.appearance.language);
        #[cfg(target_os = "linux")]
        crate::linux_theme::apply(app.handle(), &settings.appearance.theme);
        #[cfg(not(target_os = "linux"))]
        match settings.appearance.theme.as_str() {
            "dark" => app.handle().set_theme(Some(tauri::Theme::Dark)),
            "light" => app.handle().set_theme(Some(tauri::Theme::Light)),
            _ => {}
        }
    }
    let path = root.join("desktop.json");
    let preferences: Preferences = std::fs::read(&path)
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    let menu = build_context_menu(app.handle(), false, false, false, false)?;
    app.manage(Desktop {
        path,
        state: Mutex::new(ShellState {
            accent: "system".into(),
            hidden: preferences.hidden,
            preferences,
            snapshot: Snapshot::default(),
            pet_asset: Value::Null,
            floating: false,
            visible: false,
            panel: false,
            panel_page: "agent".into(),
            pin: false,
            reason: String::new(),
            page: "agent".into(),
            dirty: false,
            native_interacting: false,
            panel_generation: 0,
            panel_epoch: 0,
            recovery: String::new(),
        }),
        #[cfg(target_os = "windows")]
        tray_clicks: Mutex::default(),
        #[cfg(not(target_os = "linux"))]
        menu: Mutex::new(menu),
        menu_state: Mutex::new(None),
        sampling: AtomicBool::new(false),
        tray_available: AtomicBool::new(false),
        materials: Mutex::new(Materials::default()),
        _tray: Mutex::new(None),
    });
    #[cfg(target_os = "linux")]
    if !crate::linux_bridge::child()
        && let Some(window) = app.get_webview_window("main")
    {
        let saved = app
            .state::<Desktop>()
            .state
            .lock()
            .ok()
            .and_then(|s| s.preferences.main_window);
        let monitors = window.available_monitors().unwrap_or_default();
        let monitor = saved
            .and_then(|p| p.position)
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
            .or_else(|| window.primary_monitor().ok().flatten())
            .or_else(|| monitors.first().cloned());
        if let Some(monitor) = monitor {
            let area = monitor.work_area();
            let scale = monitor.scale_factor();
            let requested = saved
                .map(|s| (s.width.clamp(760.0, 3840.0), s.height.clamp(480.0, 2160.0)))
                .unwrap_or((1120.0, 800.0));
            let position = saved.and_then(|s| s.position).unwrap_or(Position {
                x: area.position.x,
                y: area.position.y,
            });
            let (position, size) = main_window_bounds(
                position,
                requested,
                (
                    area.position.x,
                    area.position.y,
                    area.size.width,
                    area.size.height,
                ),
                scale,
            );
            let _ = window.set_min_size(Some(tauri::LogicalSize::new(
                760.0_f64.min(area.size.width as f64 / scale),
                480.0_f64.min(area.size.height as f64 / scale),
            )));
            let _ = window.set_size(PhysicalSize::new(size.0, size.1));
            // Compositors own Wayland placement; an X11 position is never
            // replayed into a Wayland session or outside a remaining monitor.
            if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("wayland") && saved.is_some() {
                let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
            }
        }
        if saved.is_some_and(|s| s.maximized) {
            let _ = window.maximize();
        }
    }
    #[cfg(target_os = "linux")]
    let separate = crate::linux_bridge::needed();
    #[cfg(not(target_os = "linux"))]
    let separate = false;
    if !separate {
        create_floating(app.handle(), 0)?;
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if !separate {
        let position = app
            .state::<Desktop>()
            .state
            .lock()
            .ok()
            .and_then(|s| s.preferences.position)
            .map(|p| (p.x, p.y));
        app.manage(
            crate::native_pet::NativePet::start(app.handle().clone(), position)
                .map_err(std::io::Error::other)?,
        );
        if let Some(main) = app.get_webview_window("main") {
            configure_webview(&main);
        }
    }
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::child() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if separate {
        crate::linux_bridge::start(app.handle())?;
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
        let mut pet_revision = None;
        let mut pet_asset = Value::Null;
        loop {
            let preferences = store
                .settings()
                .map(|s| s.session_sounds)
                .unwrap_or_default();
            let snapshot = match store.settings() {
                Ok(settings) => monitor.read(&settings, std::time::SystemTime::now()),
                Err(_) => Snapshot {
                    unavailable: true,
                    ..Snapshot::default()
                },
            };
            let current = store
                .db
                .query_row(
                    "SELECT value FROM kv WHERE key='runtime.revision:pets'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            if current != pet_revision || pet_asset.is_null() {
                let mut next =
                    aieyes_core::pet_import::current(&store, false).unwrap_or(Value::Null);
                if next["error"].is_string() && !pet_asset["pet"].is_null() {
                    next["pet"] = pet_asset["pet"].clone();
                }
                if !next["pet"].is_null() || pet_asset.is_null() {
                    pet_asset = next;
                }
                pet_revision = store
                    .db
                    .query_row(
                        "SELECT value FROM kv WHERE key='runtime.revision:pets'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .ok();
            }
            let pet_asset = pet_asset.clone();
            let supported = if tick.is_multiple_of(15) {
                Some(tray_supported())
            } else {
                None
            };
            let sampling = store
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM estimates WHERE status IN ('active','pending'))",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap_or(false);
            let app = handle.clone();
            if handle
                .run_on_main_thread(move || {
                    app.state::<Desktop>()
                        .sampling
                        .store(sampling, Ordering::Relaxed);
                    crate::session_sounds::update(&app, &snapshot, &preferences);
                    update(&app, snapshot, pet_asset, supported);
                    refresh_menu(&app);
                })
                .is_err()
            {
                break;
            }
            tick = tick.wrapping_add(1);
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("main") {
        let _ = fit_main_window(&window);
        if let Ok(hwnd) = window.hwnd() {
            let hwnd = windows::Win32::Foundation::HWND(hwnd.0 as _);
            fit_main_after_size_move(hwnd, app.handle().clone());
            crate::caption_hit::install(hwnd, app.handle().clone());
        }
        // desktop_main_ready normally reveals the window; this covers a page that never loads.
        let app = app.handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            reveal_main(&app, false);
        });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn tray_supported() -> bool {
    true
}
#[cfg(target_os = "linux")]
fn tray_supported() -> bool {
    crate::linux_tray::host_available()
}

fn create_floating(app: &AppHandle, generation: u64) -> tauri::Result<()> {
    if let Ok(mut materials) = app.state::<Desktop>().materials.lock() {
        materials.floating = WindowMaterial::default();
    }
    let builder =
        WebviewWindowBuilder::new(app, "floating", WebviewUrl::App("floating.html".into()))
            .title(crate::localization::label("Aieyes 面板"))
            .inner_size(
                if cfg!(any(windows, target_os = "linux")) {
                    PANEL_WIDTH
                } else {
                    144.0
                },
                if cfg!(any(windows, target_os = "linux")) {
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
            .focused(cfg!(target_os = "linux"))
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
    // Keep Escape in the focused webview: nested menus/dialogs consume it
    // first, then floating.js explicitly closes even a pinned panel. A native
    // key handler would bypass that hierarchy and consume the event twice.
    #[cfg(target_os = "windows")]
    if let Ok(hwnd) = window.hwnd() {
        // Recreating the renderer may call this from a worker; configure on its UI thread.
        let handle = hwnd.0 as usize;
        let app = app.clone();
        window.run_on_main_thread(move || {
            crate::passive_window::configure(windows::Win32::Foundation::HWND(handle as *mut _));
            refresh_materials(&app);
        })?;
    }
    Ok(())
}
fn configure_webview(_window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    let _ = _window.with_webview(|view| unsafe {
        use webview2_com::{
            ContextMenuRequestedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_11,
        };
        use windows::core::Interface;
        let Ok(core) = view.controller().CoreWebView2() else {
            return;
        };
        if let Ok(settings) = core.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
            #[cfg(not(debug_assertions))]
            let _ = settings.SetAreDevToolsEnabled(false);
        }
        // The page cancels `contextmenu` outside editable fields and selected text
        // (webview-guards.js); the menus that remain keep only editing commands.
        if let Ok(core) = core.cast::<ICoreWebView2_11>() {
            let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                args.map_or(Ok(()), |args| keep_edit_commands(&args))
            }));
            let mut token = 0;
            let _ = core.add_ContextMenuRequested(&handler, &mut token);
        }
    });
}
#[cfg(target_os = "windows")]
fn keep_edit_commands(
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ContextMenuRequestedEventArgs,
) -> windows::core::Result<()> {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
        },
        take_pwstr,
    };
    // Unlocalized WebView2 item names (the English label in lower camel case).
    const EDIT: [&str; 10] = [
        "undo",
        "redo",
        "cut",
        "copy",
        "paste",
        "pasteAndMatchStyle",
        "pasteAsPlainText",
        "selectAll",
        "emoji",
        "spellCheck",
    ];
    unsafe {
        let items = args.MenuItems()?;
        let mut count = 0;
        items.Count(&mut count)?;
        // (index, separator) of the entries that stay; separators only between commands.
        let mut kept: Vec<(u32, bool)> = Vec::new();
        for index in 0..count {
            let item = items.GetValueAtIndex(index)?;
            let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
            let mut name = windows::core::PWSTR::null();
            item.Kind(&mut kind)?;
            item.Name(&mut name)?;
            let name = take_pwstr(name);
            if kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR {
                if kept.last().is_some_and(|&(_, separator)| !separator) {
                    kept.push((index, true));
                }
            } else if EDIT.contains(&name.as_str()) {
                kept.push((index, false));
            }
        }
        if kept.last().is_some_and(|&(_, separator)| separator) {
            kept.pop();
        }
        for index in (0..count).rev() {
            if !kept.iter().any(|&(kept, _)| kept == index) {
                items.RemoveValueAtIndex(index)?;
            }
        }
        if kept.is_empty() {
            args.SetHandled(true)?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) fn native_interaction(app: &AppHandle, active: bool) {
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.native_interacting = active;
    }
}
#[cfg(target_os = "linux")]
pub(crate) struct DialogInteraction(AppHandle);
#[cfg(target_os = "linux")]
impl DialogInteraction {
    pub fn new(app: &AppHandle) -> Self {
        native_interaction(app, true);
        Self(app.clone())
    }
}
#[cfg(target_os = "linux")]
impl Drop for DialogInteraction {
    fn drop(&mut self) {
        native_interaction(&self.0, false);
    }
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) fn pet_moved(app: &AppHandle, x: i32, y: i32) {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::child() {
        crate::linux_bridge::request(app, "helper_pet_moved", json!({"x":x,"y":y}));
        return;
    }
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
#[cfg(any(target_os = "windows", target_os = "linux", test))]
#[derive(Clone, Copy)]
enum PanelRequest {
    Set(bool),
    Toggle,
    /// Passive dismissal from outside clicks or focus loss; pinning blocks it.
    Dismiss,
    Restore(u64),
}
#[cfg(any(target_os = "windows", target_os = "linux", test))]
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
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn set_native_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    request_native_panel(app, PanelRequest::Set(open))
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn request_native_panel(app: &AppHandle, request: PanelRequest) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::child() && !matches!(request, PanelRequest::Restore(_)) {
        crate::linux_bridge::request(
            app,
            "desktop_action",
            json!({"action":match request {PanelRequest::Dismiss=>"dismiss-panel",PanelRequest::Toggle=>"toggle-panel",PanelRequest::Set(true)=>"open-panel",_=>"close-panel"}}),
        );
        return Ok(());
    }
    let app = app.clone();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let owner = app.clone();
    // Tauri executes inline on the UI thread. Resolve toggles/dismissals here,
    // not before enqueueing, so interleaved tray and pet_window input stays ordered.
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
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn set_native_panel_on_ui(app: &AppHandle, open: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::active(app) {
        if !crate::linux_bridge::send(app, "panel", json!(open)) {
            if open {
                let _ = activate_main(app);
            }
            return Ok(());
        }
        if let Ok(mut state) = app.state::<Desktop>().state.lock() {
            state.panel = open;
            state.panel_epoch = state.panel_epoch.wrapping_add(1);
        }
        emit_shell(app);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    let was_open = app
        .state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .panel;
    #[cfg(target_os = "linux")]
    if cfg!(debug_assertions) && std::env::var_os("AIEYES_LINUX_TRACE").is_some() {
        eprintln!("Linux panel: {was_open} -> {open}");
    }
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
    // Tray requests apply to this change only, so a later pet click is never
    // placed at, or focused like, a stale tray click.
    #[cfg(target_os = "windows")]
    let (tray_anchor, tray_focus) = {
        let desktop = app.state::<Desktop>();
        let mut clicks = desktop
            .tray_clicks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (clicks.anchor.take(), std::mem::take(&mut clicks.focus))
    };
    #[cfg(target_os = "linux")]
    let tray_anchor: Option<(i32, i32, u32, u32)> = if app
        .state::<Desktop>()
        .state
        .lock()
        .is_ok_and(|s| !s.visible)
    {
        window.primary_monitor().ok().flatten().map(|monitor| {
            let area = monitor.work_area();
            (
                area.position.x + area.size.width as i32 - 24,
                area.position.y + area.size.height as i32 - 24,
                1,
                1,
            )
        })
    } else {
        None
    };
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let tray_anchor: Option<(i32, i32, u32, u32)> = None;
    if open {
        let (x, y, ball_width, ball_height) = match tray_anchor {
            Some(anchor) => anchor,
            None => {
                let pet_window = app
                    .try_state::<crate::native_pet::NativePet>()
                    .ok_or("悬浮入口不可用")?;
                let (x, y, width, height, _pet_scale) = pet_window.geometry()?;
                (x, y, width, height)
            }
        };
        let monitors = window.available_monitors().map_err(|e| e.to_string())?;
        let monitor = monitors
            .into_iter()
            .find(|m| {
                // The notification area lies outside the work area, on the taskbar.
                let (position, size) = if tray_anchor.is_some() {
                    (*m.position(), *m.size())
                } else {
                    let a = m.work_area();
                    (a.position, a.size)
                };
                let (cx, cy) = if tray_anchor.is_some() {
                    (x + ball_width as i32 / 2, y + ball_height as i32 / 2)
                } else {
                    (x, y)
                };
                cx >= position.x
                    && cy >= position.y
                    && cx < position.x + size.width as i32
                    && cy < position.y + size.height as i32
            })
            .or(window.primary_monitor().map_err(|e| e.to_string())?)
            .ok_or("屏幕不可用")?;
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
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
    #[cfg(target_os = "windows")]
    {
        let handle = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
        crate::passive_window::set_visible(
            windows::Win32::Foundation::HWND(handle as *mut _),
            open,
        )?;
        if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
            pet_window.watch_panel(open.then_some(handle));
        }
        // A tray click grants this process foreground rights, so the panel takes
        // keyboard focus like a system flyout; pet clicks keep it passive.
        let activated = open && tray_focus && window.set_focus().is_ok();
        let desktop = app.state::<Desktop>();
        let was_open = desktop.state.lock().is_ok_and(|s| s.panel);
        let mut clicks = desktop
            .tray_clicks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        clicks.activated = activated;
        if !open && was_open {
            clicks.closed = Some(std::time::Instant::now());
        }
    }
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.panel != open {
            state.panel_epoch = state.panel_epoch.wrapping_add(1);
        }
        state.panel = open;
    }
    #[cfg(target_os = "linux")]
    {
        if open {
            use gtk::prelude::*;
            let native = window.gtk_window().map_err(|e| e.to_string())?;
            native.set_accept_focus(true);
            window.show().map_err(|e| e.to_string())?;
            if !was_open {
                // Use the activating click's timestamp; CurrentTime can be
                // rejected by focus-stealing prevention on X11 window managers.
                crate::native_pet::focus_window(&native);
                window
                    .with_webview(|view| view.inner().grab_focus())
                    .map_err(|e| e.to_string())?;
                let activation = panel_generation(app);
                let handle = app.clone();
                // A prior unmap's WM focus restoration can arrive after the
                // activating click. Retry once after that transaction settles.
                gtk::glib::timeout_add_local_once(Duration::from_millis(80), move || {
                    if activation != panel_generation(&handle) {
                        return;
                    }
                    if !handle
                        .state::<Desktop>()
                        .state
                        .lock()
                        .is_ok_and(|s| s.panel && !s.native_interacting)
                    {
                        return;
                    }
                    if let Some(window) = handle.get_webview_window("floating") {
                        if let Ok(native) = window.gtk_window() {
                            crate::native_pet::focus_window(&native);
                        }
                        let _ = window.with_webview(|view| view.inner().grab_focus());
                    }
                });
            }
        } else {
            window.hide().map_err(|e| e.to_string())?;
        }
    }
    emit_shell(app);
    Ok(())
}

/// Recreate the renderer, never the core or the native pet_window. The menu callback
/// must return before Windows starts constructing another WebView2 controller.
fn refresh_floating(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::active(app) {
        crate::linux_bridge::send(app, "refresh", Value::Null);
        return Ok(());
    }
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
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
        pet_window.watch_panel(None);
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
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                request_native_panel(&app, PanelRequest::Restore(generation))?;
            }
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn repair_panel_geometry(app: &AppHandle) -> Result<(), String> {
    let open = app
        .state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .panel;
    if !open {
        return Ok(());
    }
    let window = app.get_webview_window("floating").ok_or("面板不可用")?;
    if let Some(monitor) = window.current_monitor().map_err(|e| e.to_string())? {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let (width, height) = panel_size(scale, Some(area.size.height));
        let width = width.min(
            area.size
                .width
                .saturating_sub((24.0 * scale).round() as u32)
                .max(1),
        );
        let actual = window.inner_size().map_err(|e| e.to_string())?;
        if actual.width.abs_diff(width) > 1 || actual.height.abs_diff(height) > 1 {
            window
                .set_size(PhysicalSize::new(width, height))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    repair_panel_geometry(&app)?;
    emit_shell(&app);
    Ok(())
}

fn tray_present(app: &AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        app.try_state::<Desktop>()
            .and_then(|desktop| {
                desktop
                    ._tray
                    .lock()
                    .ok()
                    .map(|tray| tray.as_ref().is_some_and(|tray| !tray.is_closed()))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        app.tray_by_id(TRAY_ID).is_some()
    }
}

#[cfg(not(target_os = "linux"))]
fn tray_set_visible(app: &AppHandle, visible: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(visible);
    }
}

fn tray_set_summary(app: &AppHandle, summary: String) {
    let summary = crate::localization::text(&summary);
    #[cfg(target_os = "linux")]
    {
        let desktop = app.state::<Desktop>();
        if let Ok(tray) = desktop._tray.lock()
            && let Some(tray) = &*tray
        {
            tray.set_summary(summary);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(format!("Aieyes · {summary}")));
            let _ = tray.set_title(Some(&summary));
        }
    }
}

fn tray_set_icon(app: &AppHandle, phase: Option<Phase>, dark: bool) {
    #[cfg(target_os = "linux")]
    {
        let desktop = app.state::<Desktop>();
        if let Ok(tray) = desktop._tray.lock()
            && let Some(tray) = &*tray
        {
            tray.set_icon(phase, dark);
        }
    }
    #[cfg(not(target_os = "linux"))]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        #[cfg(target_os = "windows")]
        let image = {
            let _ = dark;
            windows_tray_icon(phase)
        };
        #[cfg(not(target_os = "windows"))]
        let image = icon(phase, dark);
        let _ = tray.set_icon(Some(image));
    }
}

#[cfg(not(target_os = "linux"))]
fn tray_event(app: &AppHandle, event: TrayIconEvent) {
    match event {
        #[cfg(target_os = "windows")]
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            rect,
            ..
        } => {
            let desktop = app.state::<Desktop>();
            let mut clicks = desktop
                .tray_clicks
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let icon = tray_rect(&rect);
            clicks.icon = Some(icon);
            let recent = |at: Option<std::time::Instant>| {
                at.is_some_and(|at| at.elapsed() < TRAY_CLICK_GRACE)
            };
            if recent(clicks.double) || recent(clicks.closed) {
                return;
            }
            clicks.anchor = Some(icon);
            clicks.focus = true;
            drop(clicks);
            let _ = request_native_panel(app, PanelRequest::Toggle);
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            #[cfg(target_os = "windows")]
            {
                app.state::<Desktop>()
                    .tray_clicks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .double = Some(std::time::Instant::now());
            }
            let _ = show_main(app, "agent");
        }
        #[cfg(target_os = "windows")]
        TrayIconEvent::Click { rect, .. } => {
            app.state::<Desktop>()
                .tray_clicks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .icon = Some(tray_rect(&rect));
        }
        _ => {}
    }
}
#[cfg(target_os = "windows")]
fn tray_rect(rect: &tauri::Rect) -> (i32, i32, u32, u32) {
    // Notification-area geometry is already in physical pixels.
    let position = rect.position.to_physical::<i32>(1.0);
    let size = rect.size.to_physical::<u32>(1.0);
    (
        position.x,
        position.y,
        size.width.max(1),
        size.height.max(1),
    )
}
/// Opens the panel at the notification icon with keyboard focus (tray menu entry).
#[cfg(target_os = "windows")]
fn open_tray_panel(app: &AppHandle) -> Result<(), String> {
    {
        let desktop = app.state::<Desktop>();
        let mut clicks = desktop
            .tray_clicks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Without icon geometry yet, the panel follows the pet position.
        clicks.anchor = clicks.icon;
        clicks.focus = true;
    }
    request_native_panel(app, PanelRequest::Set(true))
}

fn create_tray(app: &AppHandle, desktop: &Desktop) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let mut stored = desktop._tray.lock().unwrap_or_else(|e| e.into_inner());
        if stored.as_ref().is_some_and(|tray| !tray.is_closed()) {
            return Ok(());
        }
        // ksni closes its service when the session bus disappears. Retaining
        // that handle must not make an unusable tray suppress the pet fallback.
        *stored = None;
        let tray = crate::linux_tray::start(app)?;
        *stored = Some(tray);
        drop(stored);
        // The replacement starts with default menu values. Replay the current
        // hide/restore actions even if the shell's state has not changed.
        *desktop.menu_state.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        if app.tray_by_id(TRAY_ID).is_some() {
            return Ok(());
        }
        let menu = desktop
            .menu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let tray = TrayIconBuilder::with_id(TRAY_ID)
            .icon({
                #[cfg(target_os = "windows")]
                let image = windows_tray_icon(None);
                #[cfg(not(target_os = "windows"))]
                let image = icon(None, false);
                image
            })
            .tooltip("Aieyes")
            .menu(&menu)
            // Windows opens notification-area flyouts on left click and menus on
            // right click; the macOS menu bar shows the menu on any click.
            .show_menu_on_left_click(cfg!(not(target_os = "windows")))
            .on_tray_icon_event(|tray, event| tray_event(tray.app_handle(), event))
            .build(app)
            .map_err(|error| error.to_string())?;
        *desktop._tray.lock().unwrap_or_else(|e| e.into_inner()) = Some(tray);
        Ok(())
    }
}

fn semantic(snapshot: &Snapshot) -> Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    if let Some(rows) = value["sources"].as_array_mut() {
        for row in rows {
            row.as_object_mut().map(|object| object.remove("checkedAt"));
        }
    }
    value
}

fn update(app: &AppHandle, snapshot: Snapshot, pet_asset: Value, supported: Option<bool>) {
    let desktop = app.state::<Desktop>();
    let mut new_tray = false;
    let previous_tray = desktop.tray_available.load(Ordering::Relaxed);
    if let Some(supported) = supported {
        new_tray = supported && !tray_present(app);
        let tray_result = supported.then(|| create_tray(app, &desktop));
        let created = matches!(tray_result, Some(Ok(())));
        let available = supported && created;
        desktop.tray_available.store(available, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        {
            if let Some(Err(error)) = &tray_result {
                eprintln!("Linux tray creation failed: {error}");
            }
            eprintln!(
                "Linux tray: supported={supported} created={created} available={available} present={}",
                tray_present(app)
            );
        }
    }
    let unchanged = desktop.state.lock().is_ok_and(|state| {
        state.pet_asset == pet_asset && semantic(&state.snapshot) == semantic(&snapshot)
    });
    if unchanged && !new_tray && previous_tray == desktop.tray_available.load(Ordering::Relaxed) {
        return;
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
    if text_changed || new_tray {
        tray_set_summary(app, summary.clone());
    }
    if phase_changed || new_tray {
        tray_set_icon(
            app,
            snapshot.phase(),
            app.get_webview_window("main")
                .and_then(|w| w.theme().ok())
                .is_some_and(|t| t == tauri::Theme::Dark),
        );
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let previous_size = entry_size(app);
    if let Ok(mut state) = desktop.state.lock() {
        state.snapshot = snapshot;
        state.pet_asset = pet_asset;
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    if previous_size != entry_size(app)
        && desktop
            .state
            .lock()
            .map(|state| !state.panel)
            .unwrap_or(false)
        && let Some(window) = app.get_webview_window("floating")
    {
        let (width, height) = entry_size(app);
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        let _ = clamp_ball(app);
    }
    if let Err(error) = apply_mode(app) {
        let _ = app.emit("desktop:error", error);
    }
    if let Ok(info) = desktop_info(app.clone()) {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
            pet_window.status(info.clone());
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
        #[cfg(target_os = "linux")]
        let floating = floating && !crate::linux_bridge::failed(app);
        let hidden = state.hidden && available;
        let visible = floating && !hidden;
        let changed = visible != state.visible;
        #[cfg(target_os = "linux")]
        let helper_failed = crate::linux_bridge::failed(app);
        #[cfg(not(target_os = "linux"))]
        let helper_failed = false;
        state.reason = if helper_failed {
            "悬浮入口不可用，已返回主窗口".into()
        } else if !available {
            "未检测到可用状态栏，已使用桌宠".into()
        } else if hidden {
            "桌宠已暂时隐藏".into()
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
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if changed && let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
        pet_window.visible(floating && !hidden);
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
    // Linux's StatusNotifierItem stays Active; its pet visibility belongs only
    // to the menu. Background updates must not overwrite Hide/Show pet labels.
    #[cfg(not(target_os = "linux"))]
    tray_set_visible(
        app,
        available && (!floating || cfg!(any(windows, target_os = "linux")) || hidden),
    );
    refresh_menu(app);
    Ok(())
}

#[tauri::command]
pub fn desktop_appearance(
    app: AppHandle,
    theme: String,
    accent: String,
    language: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let preference = theme.clone();
    let resolved_theme = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        "system" => None,
        _ => return Err("无效的主题".into()),
    };
    if !["system", "indigo", "blue", "teal", "purple"].contains(&accent.as_str()) {
        return Err("无效的强调色".into());
    }
    app.state::<Desktop>()
        .state
        .lock()
        .map_err(|_| "显示状态不可用")?
        .accent = accent;
    #[cfg(target_os = "linux")]
    let theme = {
        use gtk::prelude::*;
        crate::linux_theme::apply(&app, &preference);
        Some(
            if gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme()) {
                tauri::Theme::Dark
            } else {
                tauri::Theme::Light
            },
        )
    };
    #[cfg(not(target_os = "linux"))]
    let theme = {
        app.set_theme(resolved_theme);
        resolved_theme
    };
    #[cfg(target_os = "linux")]
    let _ = resolved_theme;
    if let Some(language) = language
        && crate::localization::set_language(&language)
    {
        *app.state::<Desktop>()
            .menu_state
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        refresh_menu(&app);
        let summary = app
            .state::<Desktop>()
            .state
            .lock()
            .map(|state| state.snapshot.summary())
            .unwrap_or_default();
        tray_set_summary(&app, summary);
    }
    for window in app.webview_windows().values() {
        let _ = window.set_theme(theme);
        if window.label() == "floating" {
            let _ = window.set_title(crate::localization::label("Aieyes 面板"));
        }
    }
    #[cfg(target_os = "windows")]
    refresh_materials(&app);
    emit_shell(&app);
    Ok(())
}
/// Windows 10 keeps square windows (material.rs); the web pane follows so no transparent corner shows.
fn window_corners() -> &'static str {
    #[cfg(target_os = "windows")]
    if !crate::material::rounded_corners() {
        return "square";
    }
    "round"
}
#[tauri::command]
pub fn desktop_info(app: AppHandle) -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::child()
        && let Some(info) = crate::linux_bridge::cached(&app)
    {
        return Ok(info);
    }
    let desktop = app.state::<Desktop>();
    #[cfg(target_os = "linux")]
    let helper_failed = crate::linux_bridge::failed(&app);
    #[cfg(not(target_os = "linux"))]
    let helper_failed = false;
    let state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
    let materials = desktop.materials.lock().map_err(|_| "材质状态不可用")?;
    let mut pet = aieyes_core::pets::project(&state.snapshot);
    if state.preferences.pet_uniform_timing {
        pet.frame_durations_ms = vec![100; usize::from(pet.frames)];
    }
    Ok(json!({
        "materials": {"main": materials.main.kind, "floating": materials.floating.kind},
        "materialPreferences": {"main": state.preferences.main_material, "floating": state.preferences.floating_material},
        "platform": std::env::consts::OS, "material": materials.main.kind, "mode": state.preferences.mode,
        "windowCorners": window_corners(),
        "hourCycle": crate::desktop_clock::hour_cycle(),
        "language": crate::localization::language(),
        "dark": app.get_webview_window("main").and_then(|w| w.theme().ok()) == Some(tauri::Theme::Dark),
        "effectiveMode": if helper_failed { "main" } else if state.floating { "pet" } else { "tray" },
        "trayAvailable": desktop.tray_available.load(Ordering::Relaxed),
        "reason": crate::localization::text(&state.reason),
        "petEnabled": state.floating,
        "petScale": aieyes_core::pets::normalized_scale(state.preferences.pet_scale),
        "petAsset": state.pet_asset["pet"], "petError": state.pet_asset["error"],
        "petUniformTiming": state.preferences.pet_uniform_timing,
        "pet": pet,
        "summary": crate::localization::text(&state.snapshot.summary()), "phase": state.snapshot.phase(), "activeCount": state.snapshot.active_count(),
        "sources": state.snapshot.sources, "sessions": state.snapshot.sessions, "unavailable": state.snapshot.unavailable, "page": state.page,
        "hidden": state.hidden, "panelOpen": state.panel, "panelPinned": state.pin,
        "showCount": state.preferences.show_count, "panelPage": state.panel_page, "nativePet": cfg!(any(windows, target_os = "linux")),
        "recovery": state.recovery,
        "accent": state.accent,
    }))
}
#[tauri::command]
pub fn desktop_materials(app: AppHandle, main: String, floating: String) -> Result<Value, String> {
    if !cfg!(target_os = "windows") {
        return Err("此系统不支持 Windows 窗口材质".into());
    }
    if [&main, &floating]
        .iter()
        .any(|v| !["default", "mica", "acrylic", "opaque"].contains(&v.as_str()))
    {
        return Err("不支持的窗口材质".into());
    }
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        let previous = state.preferences.clone();
        state.preferences.main_material = main;
        state.preferences.floating_material = floating;
        if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
            state.preferences = previous;
            return Err(format!("显示偏好保存失败：{error}"));
        }
    }
    #[cfg(target_os = "windows")]
    for label in ["main", "floating"] {
        if let Some(window) = app.get_webview_window(label) {
            apply_window_material(&window);
        }
    }
    emit_shell(&app);
    desktop_info(app)
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
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    if let Some(window) = app.get_webview_window("floating") {
        let panel = app.state::<Desktop>().state.lock().is_ok_and(|s| s.panel);
        if !panel {
            let (w, h) = entry_size(&app);
            window
                .set_size(tauri::LogicalSize::new(w, h))
                .map_err(|e| e.to_string())?;
            restore_position(&app).map_err(|e| e.to_string())?;
        }
    }
    let info = desktop_info(app.clone())?;
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
        pet_window.status(info.clone());
    }
    let _ = app.emit("desktop:status", &info);
    Ok(info)
}
#[tauri::command]
pub fn desktop_pet_uniform_timing(app: AppHandle, enabled: bool) -> Result<Value, String> {
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        let previous = state.preferences.pet_uniform_timing;
        state.preferences.pet_uniform_timing = enabled;
        if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
            state.preferences.pet_uniform_timing = previous;
            return Err(error.to_string());
        }
    }
    emit_shell(&app);
    desktop_info(app)
}
#[tauri::command]
pub fn desktop_pet_scale(app: AppHandle, scale: f64) -> Result<Value, String> {
    if !scale.is_finite()
        || !(aieyes_core::pets::MIN_SCALE..=aieyes_core::pets::MAX_SCALE).contains(&scale)
    {
        return Err("桌宠缩放范围为 50%–200%".into());
    }
    {
        let desktop = app.state::<Desktop>();
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        let previous = state.preferences.pet_scale;
        state.preferences.pet_scale = scale;
        if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
            state.preferences.pet_scale = previous;
            return Err(format!("缩放偏好保存失败：{error}"));
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    if let Some(window) = app.get_webview_window("floating") {
        let panel = app.state::<Desktop>().state.lock().is_ok_and(|s| s.panel);
        if !panel {
            let (width, height) = entry_size(&app);
            window
                .set_size(tauri::LogicalSize::new(width, height))
                .map_err(|e| e.to_string())?;
            // Use the current position so changing size does not jump back to an old drag position.
            clamp_ball(&app).map_err(|e| e.to_string())?;
        }
    }
    emit_shell(&app);
    desktop_info(app)
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
    let filter = detail_view(&view)?;
    show_main(&app, filter["page"].as_str().unwrap_or("agent"))?;
    app.emit_to("main", "desktop:detail-view", filter)
        .map_err(|e| e.to_string())
}

fn detail_view(view: &Value) -> Result<Value, String> {
    let mut filter = serde_json::Map::new();
    for key in ["provider", "sourceId", "accountKey", "model"] {
        if let Some(value) = view[key].as_str() {
            filter.insert(key.into(), json!(value));
        }
    }
    filter.insert(
        "page".into(),
        json!(match view["page"].as_str() {
            Some("analysis") => "analysis",
            Some("servers") => "servers",
            _ => "agent",
        }),
    );
    if let Some(days) = view["days"].as_u64() {
        filter.insert(
            "days".into(),
            json!(if [1, 7, 30, 90, 365].contains(&days) {
                days
            } else {
                1
            }),
        );
    }
    if let Some(cost) = view["cost"].as_bool() {
        filter.insert("cost".into(), json!(cost));
    }
    if let Some(value) = view.get("dateRange") {
        let range: Option<aieyes_core::models::DateRange> =
            serde_json::from_value(value.clone()).map_err(|_| "日期范围无效")?;
        filter.insert("dateRange".into(), json!(range));
    }
    Ok(Value::Object(filter))
}

pub(crate) fn action(app: &AppHandle, action: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::action(app, action) {
        return Ok(());
    }
    match action {
        value if value.starts_with("edit-account:") => show_main(app, value),
        value if value.starts_with("settings-target:") => {
            let target: Value = serde_json::from_str(&value["settings-target:".len()..])
                .map_err(|_| "设置入口无效")?;
            if !target.is_object() {
                return Err("设置入口无效".into());
            }
            show_main(app, value)
        }
        "focus-panel" => set_panel(app, true),
        "close-panel" => set_panel(app, false),
        #[cfg(target_os = "windows")]
        "tray-panel" => open_tray_panel(app),
        "dismiss-panel" => {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            return request_native_panel(app, PanelRequest::Dismiss);
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            return request_native_panel(app, PanelRequest::Toggle);
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
                pet_window.status(info.clone());
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
        "pet-settings" => show_main(
            app,
            "settings-target:{\"category\":\"general\",\"section\":\"desktop\"}",
        ),
        "prices" => show_main(app, "prices"),
        "add-source" => show_main(app, "add-source"),
        "add-host" => show_main(app, "add-host"),
        "add-quota" => show_main(app, "add-quota"),
        "tray" => desktop_mode(app.clone(), Mode::Tray).map(|_| ()),
        "pet" => desktop_mode(app.clone(), Mode::Pet).map(|_| ()),
        "reset-position" => {
            {
                let desktop = app.state::<Desktop>();
                let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
                state.preferences.position = None;
                state.dirty = true;
            }
            restore_position(app).map_err(|e| e.to_string())
        }
        "menu" => {
            #[cfg(target_os = "linux")]
            {
                let owner = app.clone();
                app.run_on_main_thread(move || crate::native_pet::popup_menu(&owner, None))
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            #[cfg(not(target_os = "linux"))]
            {
                let info = desktop_info(app.clone())?;
                let visible = info["petEnabled"] == true && info["hidden"] != true;
                let menu =
                    build_context_menu(app, true, visible, info["trayAvailable"] == true, false)
                        .map_err(|e| e.to_string())?;
                app.get_webview_window("floating")
                    .ok_or("桌宠不可用")?
                    .popup_menu(&menu)
                    .map_err(|e| e.to_string())
            }
        }
        "open-panel" => set_panel(app, true),
        "sampling" => show_main(app, "sampling"),
        "check-updates" => {
            let page = app
                .state::<Desktop>()
                .state
                .lock()
                .map_err(|_| "显示状态不可用")?
                .page
                .clone();
            show_main(app, &page)?;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = crate::updates::updates_panel_check(app.clone()).await;
                let _ = app.emit("updates:open", ());
                if let Err(error) = result {
                    let _ = app.emit("desktop:error", error);
                }
            });
            Ok(())
        }
        "snap" => snap_ball(app),
        HIDE_MENU_ID => {
            let info = desktop_info(app.clone())?;
            if info["petEnabled"] == true && info["hidden"] != true {
                set_hidden(app, true)
            } else {
                set_hidden(app, false)?;
                desktop_mode(app.clone(), Mode::Pet).map(|_| ())
            }
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
    let window = focus_main(app)?;
    let _ = window.emit("desktop:navigate", page);
    Ok(())
}

/// Brings the main window forward on its current page, keeping any open drafts.
pub(crate) fn activate_main(app: &AppHandle) -> Result<(), String> {
    let _ = set_panel(app, false);
    focus_main(app).map(|_| ())
}

fn focus_main(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
    window
        .show()
        .and_then(|_| window.unminimize())
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    {
        MAIN_REVEALED.store(true, Ordering::SeqCst);
        uncloak_main(app);
        let _ = fit_main_window(&window);
    }
    #[cfg(target_os = "linux")]
    {
        let focus = window.clone();
        window
            .run_on_main_thread(move || {
                use gtk::prelude::*;
                if let Ok(native) = focus.gtk_window() {
                    crate::native_pet::focus_window(&native);
                }
                let _ = focus.with_webview(|view| view.inner().grab_focus());
            })
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "linux"))]
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(window)
}

#[cfg(any(not(any(target_os = "windows", target_os = "linux")), test))]
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
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::active(app) {
        let position = app
            .state::<Desktop>()
            .state
            .lock()
            .ok()
            .and_then(|s| s.preferences.position);
        crate::linux_bridge::send(app, "position", json!(position));
        return Ok(());
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
            let position = app
                .state::<Desktop>()
                .state
                .lock()
                .ok()
                .and_then(|s| s.preferences.position);
            pet_window.position(position.map(|p| (p.x, p.y)));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    restore_web_position(app)
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
        let ball = (entry_size(app).0 * monitor.scale_factor()).ceil() as i32;
        let ball_height = (entry_size(app).1 * monitor.scale_factor()).ceil() as i32;
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

#[cfg(not(target_os = "linux"))]
fn build_context_menu(
    app: &AppHandle,
    pet: bool,
    visible: bool,
    can_hide: bool,
    sampling: bool,
) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    // The pet menu opens the panel at the pet; the Windows tray menu opens it at
    // the notification icon, like a left click.
    #[cfg(target_os = "windows")]
    if !pet {
        menu.append(&MenuItem::with_id(
            app,
            "tray-panel",
            crate::localization::label("打开面板"),
            true,
            None::<&str>,
        )?)?;
    }
    for row in crate::context_menu::entries(pet, visible, can_hide, sampling) {
        if row.action == "separator" {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        } else {
            menu.append(&MenuItem::with_id(
                app,
                row.action,
                row.label,
                row.enabled,
                None::<&str>,
            )?)?;
        }
    }
    Ok(menu)
}
fn refresh_menu(app: &AppHandle) {
    let Some(desktop) = app.try_state::<Desktop>() else {
        return;
    };
    let state = desktop.state.lock().unwrap_or_else(|e| e.into_inner());
    let visible = state.floating && !state.hidden;
    let can_hide = can_hide(
        state.floating,
        desktop.tray_available.load(Ordering::Relaxed),
    );
    let sampling = desktop.sampling.load(Ordering::Relaxed);
    drop(state);
    let signature = (visible, can_hide, sampling);
    let mut previous = desktop.menu_state.lock().unwrap_or_else(|e| e.into_inner());
    if *previous == Some(signature) {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(tray) = desktop._tray.lock()
            && let Some(tray) = &*tray
        {
            tray.set_menu(visible, can_hide, sampling);
        }
        *previous = Some(signature);
    }
    #[cfg(not(target_os = "linux"))]
    if let Ok(menu) = build_context_menu(app, false, visible, can_hide, sampling) {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_menu(Some(menu.clone()));
        }
        *desktop.menu.lock().unwrap_or_else(|e| e.into_inner()) = menu;
        *previous = Some(signature);
    }
}

fn emit_shell(app: &AppHandle) {
    if let Ok(info) = desktop_info(app.clone()) {
        let _ = app.emit("desktop:panel", info.clone());
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let Some(pet_window) = app.try_state::<crate::native_pet::NativePet>() {
            pet_window.status(info.clone());
        }
        let _ = app.emit("desktop:status", info);
    }
}
/// Expand the floating window into the compact panel, or collapse it back to the ball.
fn set_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        set_native_panel(app, open)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    set_web_panel(app, open)
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn set_web_panel(app: &AppHandle, open: bool) -> Result<(), String> {
    let desktop = app.state::<Desktop>();
    {
        let mut state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
        if state.panel == open {
            return Ok(());
        }
        state.panel = open;
    }
    let window = app.get_webview_window("floating").ok_or("桌宠不可用")?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let ball = (entry_size(app).0 * scale).round() as i32;
    let ball_height = (entry_size(app).1 * scale).round() as i32;
    if open {
        let position = window.outer_position().map_err(|error| error.to_string())?;
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
    }
    emit_shell(app);
    Ok(())
}
/// Preserve the explicit hidden preference; a missing tray still restores an entry point.
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
            return Err("没有其它入口，无法隐藏桌宠".into());
        }
        let previous = state.preferences.hidden;
        state.preferences.hidden = hidden;
        if let Err(error) = write_preferences(&desktop.path, &state.preferences) {
            state.preferences.hidden = previous;
            return Err(error.to_string());
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
    #[cfg(target_os = "linux")]
    {
        if let Some(pet) = app.try_state::<crate::native_pet::NativePet>() {
            pet.snap();
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    position_ball(app, true)
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn clamp_ball(app: &AppHandle) -> Result<(), String> {
    position_ball(app, false)
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn position_ball(app: &AppHandle, snap: bool) -> Result<(), String> {
    let window = app.get_webview_window("floating").ok_or("桌宠不可用")?;
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
    let ball = (entry_size(app).0 * scale).round() as i32;
    let ball_height = (entry_size(app).1 * scale).round() as i32;
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
    let threshold = if snap {
        (SNAP_DISTANCE * scale).round() as i32
    } else {
        0
    };
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
    #[cfg(target_os = "windows")]
    if matches!(
        event,
        tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
    ) && let Ok(hwnd) = window.hwnd()
    {
        crate::material::reshape(windows::Win32::Foundation::HWND(hwnd.0 as _));
    }
    #[cfg(target_os = "linux")]
    if window.label() == "main"
        && matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_)
        )
        && let Some(desktop) = window.app_handle().try_state::<Desktop>()
        && let Ok(mut state) = desktop.state.lock()
    {
        let maximized = window.is_maximized().unwrap_or(false);
        let previous = state.preferences.main_window;
        let size = window
            .inner_size()
            .ok()
            .map(|size| size.to_logical::<f64>(window.scale_factor().unwrap_or(1.0)));
        if let Some(size) = size {
            let (width, height) = if maximized {
                previous
                    .map(|p| (p.width, p.height))
                    .unwrap_or((1100.0, 760.0))
            } else {
                (size.width, size.height)
            };
            let position = if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
                None
            } else if maximized {
                previous.and_then(|p| p.position)
            } else {
                window
                    .outer_position()
                    .ok()
                    .map(|p| Position { x: p.x, y: p.y })
            };
            if width >= 760.0 && height >= 480.0 {
                state.preferences.main_window = Some(MainWindowPreferences {
                    width,
                    height,
                    maximized,
                    position,
                });
                state.dirty = true;
            }
        }
    }
    match event {
        tauri::WindowEvent::ThemeChanged(theme) if window.label() == "main" => {
            #[cfg(target_os = "windows")]
            refresh_materials(window.app_handle());
            let app = window.app_handle();
            let phase = app
                .state::<Desktop>()
                .state
                .lock()
                .ok()
                .and_then(|state| state.snapshot.phase());
            tray_set_icon(app, phase, *theme == tauri::Theme::Dark);
            emit_shell(window.app_handle());
        }
        #[cfg(target_os = "windows")]
        tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "main" => {
            let (moving, _) = MAIN_SIZE_MOVE.get();
            if moving {
                MAIN_SIZE_MOVE.set((true, true));
            } else if let Some(main) = window.app_handle().get_webview_window("main") {
                let _ = fit_main_window(&main);
            }
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if window.label() == "main" {
                // Always make the fallback visible before hiding the details window.
                let app = window.app_handle();
                #[cfg(target_os = "linux")]
                if crate::linux_bridge::active(app) {
                    if crate::linux_bridge::connected(app)
                        || app
                            .state::<Desktop>()
                            .tray_available
                            .load(Ordering::Relaxed)
                    {
                        let _ = window.hide();
                        background_hint(app);
                    }
                    return;
                }
                let ready = if let Some(_ball) = app.get_webview_window("floating") {
                    if app
                        .state::<Desktop>()
                        .state
                        .lock()
                        .map(|s| s.visible)
                        .unwrap_or(true)
                    {
                        #[cfg(any(target_os = "windows", target_os = "linux"))]
                        {
                            if let Some(pet_window) =
                                app.try_state::<crate::native_pet::NativePet>()
                            {
                                pet_window.visible(true);
                            }
                            true
                        }
                        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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
                    #[cfg(target_os = "linux")]
                    background_hint(app);
                }
            } else if window.label() == "floating" {
                let _ = set_panel(window.app_handle(), false);
            }
        }
        tauri::WindowEvent::Moved(pos) if window.label() == "floating" => {
            if let Some(desktop) = window.app_handle().try_state::<Desktop>()
                && let Ok(mut state) = desktop.state.lock()
                // Panel placement must never overwrite the ball's saved position.
                && !cfg!(any(windows, target_os = "linux")) && !state.panel
            {
                state.preferences.position = Some(Position { x: pos.x, y: pos.y });
                state.dirty = true;
            }
        }
        #[cfg(not(target_os = "windows"))]
        tauri::WindowEvent::Focused(false) if window.label() == "floating" => {
            #[cfg(debug_assertions)]
            eprintln!("Linux panel lost focus");
            let app = window.app_handle().clone();
            let generation = panel_generation(&app);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let owner = app.clone();
                let _ = owner.run_on_main_thread(move || {
                    if generation != panel_generation(&app)
                        || app
                            .get_webview_window("floating")
                            .is_some_and(|w| w.is_focused().unwrap_or(false))
                    {
                        return;
                    }
                    let close = app
                        .state::<Desktop>()
                        .state
                        .lock()
                        .is_ok_and(|s| s.can_dismiss_panel());
                    if close {
                        let _ = set_panel(&app, false);
                    }
                });
            });
        }
        // A panel opened from the tray behaves like a system flyout and closes
        // when it loses activation; pinning and native menus still keep it.
        #[cfg(target_os = "windows")]
        tauri::WindowEvent::Focused(false) if window.label() == "floating" => {
            let app = window.app_handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let activated = app
                    .state::<Desktop>()
                    .tray_clicks
                    .lock()
                    .is_ok_and(|clicks| clicks.activated);
                let focused = app
                    .get_webview_window("floating")
                    .is_some_and(|w| w.is_focused().unwrap_or(false));
                if activated && !focused {
                    let _ = request_native_panel(&app, PanelRequest::Dismiss);
                } else if !focused {
                    // A panel that stays open after losing activation keeps its Acrylic.
                    let owner = app.clone();
                    let _ = owner.run_on_main_thread(move || {
                        if let Some(window) = app.get_webview_window("floating")
                            && window.is_visible().unwrap_or(false)
                            && !window.is_focused().unwrap_or(false)
                            && let Ok(hwnd) = window.hwnd()
                        {
                            crate::passive_window::draw_active(windows::Win32::Foundation::HWND(
                                hwnd.0 as *mut _,
                            ));
                        }
                    });
                }
            });
        }
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "floating" => {
            let _ = repair_panel_geometry(window.app_handle());
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
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

#[cfg(target_os = "linux")]
fn background_hint(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    use tauri_plugin_notification::NotificationExt;
    let desktop = app.state::<Desktop>();
    let first_close = desktop.state.lock().is_ok_and(|mut state| {
        let first = !state.preferences.background_hint_shown;
        if first {
            state.preferences.background_hint_shown = true;
            state.dirty = true;
        }
        first
    });
    if !first_close {
        return;
    }
    let body = crate::localization::label("关闭窗口后会继续后台监测，可通过桌宠或托盘重新打开。");
    if app
        .notification()
        .builder()
        .title("Aieyes")
        .body(body)
        .show()
        .is_err()
    {
        app.dialog().message(body).title("Aieyes").show(|_| {});
    }
}
fn write_preferences(path: &Path, preferences: &Preferences) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(preferences)?)
}
pub fn save(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if crate::linux_bridge::child() {
        return;
    }
    if let Some(desktop) = app.try_state::<Desktop>()
        && let Ok(mut state) = desktop.state.lock()
        && state.dirty
        && write_preferences(&desktop.path, &state.preferences).is_ok()
    {
        state.dirty = false;
    }
}
// Windows draws its tray icon at the notification area's size (windows_tray_icon).
#[cfg(any(not(target_os = "windows"), test))]
pub(crate) fn icon(phase: Option<Phase>, dark: bool) -> tauri::image::Image<'static> {
    badged_icon(phase, dark, 32)
}
/// The 32 px brand at `size` physical pixels with the live state as a separate top-right badge (never
/// a recoloring of the brand). The badge keeps a 3.5 px minimum radius so it reads at 16 px.
fn badged_icon(phase: Option<Phase>, dark: bool, size: usize) -> tauri::image::Image<'static> {
    let key = match phase {
        Some(Phase::Working) => "working",
        Some(Phase::Thinking) => "thinking",
        Some(Phase::Tool) => "tool",
        Some(Phase::Complete) => "complete",
        Some(Phase::Interrupted) => "interrupted",
        Some(Phase::Unknown) => "unknown",
        Some(Phase::NeedsInput) => "needsInput",
        Some(Phase::Failed) => "failed",
        None => "idle",
    };
    let [r, g, b] = crate::phase_colors::rgb(key, dark);
    let color = [r, g, b, 255];
    let ring = if dark {
        [236, 240, 248, 255]
    } else {
        [20, 30, 50, 255]
    };
    let mut rgba = scale_brand(include_bytes!("../icons/brand.rgba"), 32, size);
    if phase.is_some() {
        let radius = (size as f64 * 5.0 / 32.0).max(3.5);
        let (cx, cy) = (size as f64 - radius - 1.5, radius + 0.5);
        for y in 0..size {
            for x in 0..size {
                let distance = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                if distance <= radius * radius {
                    let ink = if distance > (radius - 1.0).powi(2) {
                        ring
                    } else {
                        color
                    };
                    rgba[(y * size + x) * 4..(y * size + x) * 4 + 4].copy_from_slice(&ink);
                }
            }
        }
    }
    tauri::image::Image::new_owned(rgba, size as u32, size as u32)
}
/// Area-averaged resize of straight-alpha RGBA, weighting colour by alpha so edges stay clean.
fn scale_brand(source: &[u8], from: usize, to: usize) -> Vec<u8> {
    if from == to {
        return source.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let mut output = vec![0; to * to * 4];
    for y in 0..to {
        for x in 0..to {
            let (x0, x1, y0, y1) = (
                x as f64 * ratio,
                (x + 1) as f64 * ratio,
                y as f64 * ratio,
                (y + 1) as f64 * ratio,
            );
            let (mut sum, mut alpha, mut area) = ([0.0; 3], 0.0, 0.0);
            for sy in y0.floor() as usize..(y1.ceil() as usize).min(from) {
                let wy = y1.min(sy as f64 + 1.0) - y0.max(sy as f64);
                for sx in x0.floor() as usize..(x1.ceil() as usize).min(from) {
                    let weight = (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)) * wy;
                    let pixel = &source[(sy * from + sx) * 4..(sy * from + sx) * 4 + 4];
                    let a = f64::from(pixel[3]) / 255.0 * weight;
                    for (channel, value) in sum.iter_mut().zip(pixel) {
                        *channel += f64::from(*value) * a;
                    }
                    alpha += a;
                    area += weight;
                }
            }
            let target = &mut output[(y * to + x) * 4..(y * to + x) * 4 + 4];
            if alpha > 0.0 {
                for (channel, value) in target.iter_mut().zip(sum) {
                    *channel = (value / alpha).round() as u8;
                }
            }
            target[3] = (alpha / area * 255.0).round() as u8;
        }
    }
    output
}
/// Windows: drawn at the notification area's physical size, coloured for the taskbar's own light or
/// dark setting (Windows 10 defaults to a dark taskbar with light apps), not the app theme.
#[cfg(target_os = "windows")]
fn windows_tray_icon(phase: Option<Phase>) -> tauri::image::Image<'static> {
    use windows::Win32::UI::{
        HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi},
        WindowsAndMessaging::SM_CXSMICON,
    };
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.clamp(16, 64);
    badged_icon(phase, crate::material::taskbar_dark(), size as usize)
}
/// Taskbar theme changes arrive as WM_SETTINGCHANGE, which only the native pet window observes.
#[cfg(target_os = "windows")]
pub(crate) fn refresh_tray_icon(app: &AppHandle) {
    let phase = app
        .state::<Desktop>()
        .state
        .lock()
        .ok()
        .and_then(|state| state.snapshot.phase());
    tray_set_icon(app, phase, false);
}

#[cfg(target_os = "linux")]
pub(crate) fn helper_status(app: &AppHandle, info: Value) {
    crate::linux_bridge::remember(app, &info);
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.floating = info["petEnabled"] == true;
        state.visible = state.floating && info["hidden"] != true;
        state.hidden = info["hidden"] == true;
        state.pin = info["panelPinned"] == true;
        state.preferences.pet_scale = info["petScale"].as_f64().unwrap_or(1.0);
        state.preferences.pet_uniform_timing = info["petUniformTiming"] == true;
        state.preferences.show_count = info["showCount"] == true;
        state.pet_asset = json!({"pet":info["petAsset"]});
        if let Some(page) = info["panelPage"].as_str() {
            state.panel_page = page.into();
        }
    }
    if let Some(language) = info["language"].as_str() {
        crate::localization::set_language(language);
    }
    app.set_theme(Some(if info["dark"] == true {
        tauri::Theme::Dark
    } else {
        tauri::Theme::Light
    }));
    if let Some(pet) = app.try_state::<crate::native_pet::NativePet>() {
        pet.status(info.clone());
        pet.visible(info["petEnabled"] == true && info["hidden"] != true);
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn helper_control(app: &AppHandle, name: &str, payload: Value) {
    match name {
        "status" => helper_status(app, payload),
        "panel" => {
            let _ = set_native_panel_on_ui(app, payload == true);
        }
        "refresh" => {
            let _ = refresh_floating(app);
        }
        "position" => {
            if let Some(pet) = app.try_state::<crate::native_pet::NativePet>() {
                let pos = serde_json::from_value::<Option<Position>>(payload).unwrap_or(None);
                pet.position(pos.map(|p| (p.x, p.y)));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_status_ignores_check_timestamps_but_keeps_health_and_session_changes() {
        use aieyes_core::sessions::SourceHealth;
        let mut first = Snapshot::default();
        first.sources.push(SourceHealth {
            source_id: "s".into(),
            state: "healthy".into(),
            checked_at: 1,
            ..Default::default()
        });
        let mut next = first.clone();
        next.sources[0].checked_at = 2;
        assert_eq!(semantic(&first), semantic(&next));
        next.sources[0].state = "degraded".into();
        assert_ne!(semantic(&first), semantic(&next));
        next = first.clone();
        next.sources[0].hooks = Some("installed".into());
        assert_ne!(semantic(&first), semantic(&next));
    }
    #[test]
    fn detail_context_preserves_destination_and_optional_filters() {
        assert_eq!(
            detail_view(&json!({"page":"analysis", "provider":"codex", "accountKey":"codex:a"}))
                .unwrap(),
            json!({"page":"analysis", "provider":"codex", "accountKey":"codex:a"})
        );
        assert_eq!(
            detail_view(&json!({"page":"servers"})).unwrap(),
            json!({"page":"servers"})
        );
        assert_eq!(
            detail_view(&json!({"page":"agent", "days":30, "cost":true, "dateRange":null}))
                .unwrap(),
            json!({"page":"agent", "days":30, "cost":true, "dateRange":null})
        );
        assert!(detail_view(&json!({"dateRange":{"kind":"invalid"}})).is_err());
    }

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
        // Escape/close and pet toggles remain explicit actions when pinned.
        assert_eq!(
            PanelRequest::Set(false).resolve(true, true, false, 1),
            Some(false)
        );
        assert_eq!(
            PanelRequest::Toggle.resolve(true, true, false, 1),
            Some(false)
        );
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
    fn rectangular_pet_windows_use_their_height_for_bottom_edges_and_panel_anchors() {
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
        // At 100 % (16 px) the badge stays at least 7 px across instead of shrinking to 2 px.
        let small = badged_icon(Some(Phase::Working), false, 16);
        assert_eq!(small.rgba().len(), 16 * 16 * 4);
        let ink = crate::phase_colors::rgb("working", false);
        let row = &small.rgba()[4 * 16 * 4..5 * 16 * 4];
        let badge = row
            .chunks(4)
            .filter(|p| p[..3] == ink[..] || p[..3] == [20, 30, 50])
            .count();
        assert!(badge >= 7, "badge spans {badge} px at 16 px");
        let plain = badged_icon(None, false, 16);
        assert!(
            plain.rgba().chunks(4).any(|p| p[3] > 200),
            "scaled brand stays opaque inside"
        );
    }
    #[test]
    fn platform_bundle_configurations_are_valid() {
        let base: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        serde_json::from_value::<tauri::Config>(base.clone()).unwrap();
        for config in [
            include_str!("../tauri.windows.conf.json"),
            include_str!("../tauri.linux.conf.json"),
            include_str!("../tauri.macos.conf.json"),
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
        assert_eq!(Mode::default(), Mode::Pet);
        for legacy in ["auto", "floating", "pet"] {
            assert_eq!(
                serde_json::from_value::<Mode>(json!(legacy)).unwrap(),
                Mode::Pet
            );
        }
        for mode in [Mode::Tray, Mode::Pet] {
            assert!(use_floating(mode, false, false));
        }
        assert!(use_floating(Mode::Pet, false, true));
        assert!(!use_floating(Mode::Tray, true, true));
        let preferences: Preferences =
            serde_json::from_value(json!({"mode":"tray","hidden":true})).unwrap();
        assert_eq!(preferences.mode, Mode::Tray);
        assert!(preferences.hidden);
        assert!(
            serde_json::to_value(preferences).unwrap()["hidden"]
                .as_bool()
                .unwrap()
        );
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
    fn tall_panel_uses_free_side_without_covering_pet_window() {
        for scale in [1, 2] {
            let ball = Position {
                x: 856 * scale,
                y: 338 * scale,
            };
            let p = panel_rect(
                ball,
                (144 * scale, 44 * scale),
                (450 * scale, 720 * scale),
                (0, 0, 1024 * scale as u32, 768 * scale as u32),
                8 * scale,
            );
            assert!(p.x + 450 * scale + 8 * scale <= ball.x);
            assert!(p.y >= 0 && p.y + 720 * scale <= 768 * scale);
        }
        let p = panel_rect(
            Position { x: -1000, y: 338 },
            (144, 44),
            (450, 720),
            (-1024, 0, 1024, 768),
            8,
        );
        assert!(p.x >= -1000 + 144 + 8);
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
