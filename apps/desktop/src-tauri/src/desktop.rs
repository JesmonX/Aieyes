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
/// Floating window edge in logical pixels; the panel expands the same window.
const BALL_SIZE: f64 = 88.0;
/// Panel size in logical pixels; height is clamped to the monitor work area.
const PANEL_WIDTH: f64 = 420.0;
const PANEL_HEIGHT: f64 = 640.0;
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
}
struct ShellState {
    preferences: Preferences,
    snapshot: Snapshot,
    floating: bool,
    visible: bool,
    hidden: bool,
    panel: bool,
    pin: bool,
    reason: String,
    page: String,
    dirty: bool,
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

fn snap_position(
    position: Position,
    area: (i32, i32, u32, u32),
    ball: i32,
    threshold: i32,
) -> Position {
    let (x, y, width, height) = area;
    let right = x + width as i32 - ball;
    let bottom = y + height as i32 - ball;
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
    clamp_position(snapped, x, y, width, height, ball)
}

fn panel_position(
    ball_pos: Position,
    ball: i32,
    panel: (i32, i32),
    area: (i32, i32, u32, u32),
    gap: i32,
) -> Position {
    let (x, y, width, height) = area;
    let (panel_width, panel_height) = panel;
    let mut left = if ball_pos.x + ball / 2 >= x + width as i32 / 2 {
        ball_pos.x + ball - panel_width
    } else {
        ball_pos.x
    };
    let below = ball_pos.y + ball + gap;
    let above = ball_pos.y - panel_height - gap;
    let mut top = if ball_pos.y + ball / 2 < y + height as i32 / 2 {
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

fn panel_size(scale: f64, work_height: Option<u32>) -> (u32, u32) {
    let width = (PANEL_WIDTH * scale).round() as u32;
    let height = (PANEL_HEIGHT * scale).round() as u32;
    let margin = (24.0 * scale).round() as u32;
    let height = work_height.map_or(height, |work| {
        height.min(work.saturating_sub(margin).max(1))
    });
    (width, height)
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
    let hide = MenuItem::with_id(app, "hide", "暂时隐藏悬浮球", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Aieyes", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_item,
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
            preferences,
            snapshot: Snapshot::default(),
            floating: false,
            visible: false,
            hidden: false,
            panel: false,
            pin: false,
            reason: String::new(),
            page: "agent".into(),
            dirty: false,
        }),
        menu,
        status_item,
        hide_item: hide,
        tray_available: AtomicBool::new(false),
        material,
    });
    // The native Swift app remains the macOS entry point; opaque mode permits Tauri development there.
    let floating =
        WebviewWindowBuilder::new(app, "floating", WebviewUrl::App("floating.html".into()))
            .title("Aieyes 会话状态")
            .inner_size(88.0, 88.0)
            .resizable(false)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false);
    #[cfg(not(target_os = "macos"))]
    let floating = floating.transparent(true);
    floating.build()?;
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

fn create_tray(app: &AppHandle, desktop: &Desktop) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(None))
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
            let _ = tray.set_icon(Some(icon(snapshot.phase())));
        }
    }
    if let Ok(mut state) = desktop.state.lock() {
        state.snapshot = snapshot;
    }
    if let Err(error) = apply_mode(app) {
        let _ = app.emit("desktop:error", error);
    }
    if let Ok(info) = desktop_info(app.clone()) {
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
pub fn desktop_info(app: AppHandle) -> Result<Value, String> {
    let desktop = app.state::<Desktop>();
    let state = desktop.state.lock().map_err(|_| "显示状态不可用")?;
    Ok(json!({
        "platform": std::env::consts::OS, "material": desktop.material, "mode": state.preferences.mode,
        "effectiveMode": if state.floating { "floating" } else { "tray" },
        "trayAvailable": desktop.tray_available.load(Ordering::Relaxed), "reason": state.reason,
        "summary": state.snapshot.summary(), "phase": state.snapshot.phase(), "activeCount": state.snapshot.active_count(),
        "sessions": state.snapshot.sessions, "unavailable": state.snapshot.unavailable, "page": state.page,
        "hidden": state.hidden, "panelOpen": state.panel, "panelPinned": state.pin,
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
pub fn desktop_action(app: AppHandle, action: String) -> Result<(), String> {
    self::action(&app, &action)
}
fn action(app: &AppHandle, action: &str) -> Result<(), String> {
    match action {
        "open" => show_main(app, "agent"),
        "settings" => show_main(app, "settings"),
        "prices" => show_main(app, "prices"),
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
        "hide-ball" => set_hidden(app, true),
        "show-ball" => set_hidden(app, false),
        "quit" => {
            save(app);
            app.exit(0);
            Ok(())
        }
        _ => Err("未知操作".into()),
    }
}
fn show_main(app: &AppHandle, page: &str) -> Result<(), String> {
    if let Ok(mut state) = app.state::<Desktop>().state.lock() {
        state.page = page.into();
    }
    let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
    window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
        .map_err(|e| e.to_string())?;
    let _ = window.emit("desktop:navigate", page);
    Ok(())
}

fn clamp_position(pos: Position, x: i32, y: i32, width: u32, height: u32, ball: i32) -> Position {
    Position {
        x: pos
            .x
            .clamp(x, (x.saturating_add(width as i32) - ball).max(x)),
        y: pos
            .y
            .clamp(y, (y.saturating_add(height as i32) - ball).max(y)),
    }
}
fn restore_position(app: &AppHandle) -> tauri::Result<()> {
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
        let default = Position {
            x: a.position.x + a.size.width as i32 - ball - 24,
            y: a.position.y + (a.size.height as i32 - ball) / 2,
        };
        let pos = clamp_position(
            saved.unwrap_or(default),
            a.position.x,
            a.position.y,
            a.size.width,
            a.size.height,
            ball,
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
        let _ = app.emit("desktop:status", info);
    }
}
/// Expand the floating window into the compact panel, or collapse it back to the ball.
fn set_panel(app: &AppHandle, open: bool) -> Result<(), String> {
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
            let position = panel_position(
                Position {
                    x: position.x,
                    y: position.y,
                },
                ball,
                (width as i32, height as i32),
                area,
                gap,
            );
            let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
        }
    } else {
        window
            .set_size(PhysicalSize::new(ball as u32, ball as u32))
            .map_err(|error| error.to_string())?;
        let _ = restore_position(app);
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
    let snapped = snap_position(
        Position {
            x: position.x,
            y: position.y,
        },
        area,
        ball,
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
        #[cfg(target_os = "windows")]
        tauri::WindowEvent::ThemeChanged(theme) if window.label() == "main" => {
            let _ = window_vibrancy::apply_mica(window, Some(*theme == tauri::Theme::Dark));
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if window.label() == "main" {
                // Always make the fallback visible before hiding the details window.
                let app = window.app_handle();
                let ready = if let Some(ball) = app.get_webview_window("floating") {
                    if app
                        .state::<Desktop>()
                        .state
                        .lock()
                        .map(|s| s.floating && !s.hidden)
                        .unwrap_or(true)
                    {
                        ball.show().is_ok()
                    } else {
                        true
                    }
                } else {
                    false
                };
                if ready {
                    let _ = window.hide();
                }
            }
        }
        tauri::WindowEvent::Moved(pos) if window.label() == "floating" => {
            if let Some(desktop) = window.app_handle().try_state::<Desktop>()
                && let Ok(mut state) = desktop.state.lock()
                // Panel placement must never overwrite the ball's saved position.
                && !state.panel
            {
                state.preferences.position = Some(Position { x: pos.x, y: pos.y });
                state.dirty = true;
            }
        }
        tauri::WindowEvent::Focused(false) if window.label() == "floating" => {
            let app = window.app_handle();
            let close = app
                .try_state::<Desktop>()
                .and_then(|desktop| {
                    desktop
                        .state
                        .lock()
                        .ok()
                        .map(|state| state.panel && !state.pin)
                })
                .unwrap_or(false);
            if close {
                let _ = set_panel(app, false);
            }
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
fn icon(phase: Option<Phase>) -> tauri::image::Image<'static> {
    let color = match phase {
        Some(Phase::Working) => [40, 164, 150, 255],
        Some(Phase::Thinking) => [161, 104, 221, 255],
        Some(Phase::Tool) => [70, 130, 230, 255],
        Some(Phase::Complete) => [49, 165, 101, 255],
        Some(Phase::Interrupted) => [217, 148, 52, 255],
        _ => [132, 145, 167, 255],
    };
    let mut rgba = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f64 - 15.5;
            let dy = y as f64 - 15.5;
            let eye = dx * dx / 196.0 + dy * dy / 64.0;
            if (0.65..=1.0).contains(&eye) || dx * dx + dy * dy < 18.0 {
                rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4].copy_from_slice(&color);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(panel_size(1.0, Some(1040)), (420, 640));
        assert_eq!(panel_size(2.0, Some(700)), (840, 652));
    }
    #[test]
    fn hiding_the_ball_requires_another_entry_point() {
        assert!(can_hide(true, true));
        assert!(!can_hide(true, false));
        assert!(!can_hide(false, true));
    }
}
