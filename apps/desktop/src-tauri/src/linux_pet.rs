//! GTK pet surface. Animation never resizes/repaints a WebKit window.
use aieyes_core::pets::{PetState, frame, next_frame_delay, normalized_scale};
use gtk::{cairo, gdk, glib, prelude::*};
use serde_json::Value;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

thread_local! {
    static SURFACE: RefCell<Option<Rc<RefCell<Surface>>>> = const { RefCell::new(None) };
}

pub struct NativePet {
    app: AppHandle,
    stopped: Arc<AtomicBool>,
}
struct Surface {
    window: gtk::ApplicationWindow,
    area: gtk::DrawingArea,
    info: Value,
    state: PetState,
    frames: Vec<cairo::ImageSurface>,
    asset: String,
    start: Instant,
    timer: Option<glib::SourceId>,
    visible: bool,
    press: Option<(f64, f64)>,
    dragged: bool,
}

/// Select the compatibility backend before GTK or worker threads start. Explicit
/// user backend choices win, and an unavailable X server never prevents startup.
pub fn configure_backend() {
    if std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        && std::env::var_os("DISPLAY").is_some()
        && x11rb::connect(None).is_ok()
    {
        // main calls this before creating the Tauri runtime or spawning threads.
        unsafe {
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }
}

/// A user explicitly opened this panel. Coordinate GTK activation with X11's
/// keyboard focus, including WMs that decline activation of a skip-taskbar window.
pub fn focus_panel(window: &gtk::ApplicationWindow) {
    let time = gtk::current_event_time();
    window.present_with_time(time);
    let Some(surface) = window.window() else {
        return;
    };
    let Ok(surface) = surface.downcast::<gdkx11::X11Window>() else {
        return;
    };
    surface.upcast_ref::<gdk::Window>().display().sync();
    use x11rb::{
        connection::Connection,
        protocol::xproto::{ClientMessageEvent, ConnectionExt as _, EventMask, InputFocus},
    };
    if let Ok((connection, screen)) = x11rb::connect(None) {
        let id = surface.xid() as u32;
        let root = connection.setup().roots[screen].root;
        if let Ok(cookie) = connection.intern_atom(false, b"_NET_ACTIVE_WINDOW")
            && let Ok(atom) = cookie.reply()
        {
            let event = ClientMessageEvent::new(32, id, atom.atom, [1, time, 0, 0, 0]);
            let _ = connection.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            );
        }
        let _ = connection.set_input_focus(InputFocus::PARENT, id, x11rb::CURRENT_TIME);
        let _ = connection.flush();
    }
}

fn with_surface<T>(f: impl FnOnce(&Rc<RefCell<Surface>>) -> T) -> Option<T> {
    SURFACE.with(|slot| slot.borrow().as_ref().map(f))
}
fn dispatch(app: &AppHandle, action: &'static str) {
    if cfg!(debug_assertions) && std::env::var_os("AIEYES_LINUX_TRACE").is_some() {
        eprintln!("Linux pet action: {action}");
    }
    if let Err(error) = crate::desktop::action(app, action) {
        let _ = app.emit("desktop:error", error);
    }
}

impl NativePet {
    pub fn start(app: AppHandle, position: Option<(i32, i32)>) -> Result<Self, String> {
        let application = app
            .get_webview_window("main")
            .ok_or("主窗口不可用")?
            .gtk_window()
            .map_err(|e| e.to_string())?
            .application()
            .ok_or("GTK 应用不可用")?;
        let window = gtk::ApplicationWindow::new(&application);
        window.set_title("Aieyes 桌宠");
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_keep_above(true);
        window.set_accept_focus(false);
        window.set_focus_on_map(false);
        window.set_app_paintable(true);
        if let Some(screen) = gtk::prelude::WidgetExt::screen(&window)
            && let Some(visual) = screen.rgba_visual()
        {
            window.set_visual(Some(&visual));
        }
        let css = gtk::CssProvider::new();
        css.load_from_data(b"window { background-color: transparent; box-shadow: none; }")
            .map_err(|e| e.to_string())?;
        window
            .style_context()
            .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let area = gtk::DrawingArea::new();
        area.set_size_request(144, 168);
        area.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK,
        );
        window.add(&area);
        let surface = Rc::new(RefCell::new(Surface {
            window: window.clone(),
            area: area.clone(),
            info: Value::Null,
            state: aieyes_core::pets::project(&Default::default()),
            frames: vec![],
            asset: String::new(),
            start: Instant::now(),
            timer: None,
            visible: false,
            press: None,
            dragged: false,
        }));
        SURFACE.with(|slot| *slot.borrow_mut() = Some(surface.clone()));
        let weak = Rc::downgrade(&surface);
        area.connect_draw(move |_, context| {
            if let Some(surface) = weak.upgrade() {
                let _ = paint(&surface.borrow(), context);
            }
            glib::Propagation::Stop
        });
        let handle = app.clone();
        area.connect_button_press_event(move |_, event| {
            if cfg!(debug_assertions) && std::env::var_os("AIEYES_LINUX_TRACE").is_some() {
                eprintln!("Linux pet press: {} {:?}", event.button(), event.root());
            }
            if event.button() == 3 {
                popup_menu(&handle, Some(event));
            } else if event.button() == 1 {
                with_surface(|s| {
                    let mut s = s.borrow_mut();
                    s.press = Some(event.root());
                    s.dragged = false;
                });
            }
            glib::Propagation::Stop
        });
        area.connect_motion_notify_event(move |_, event| {
            with_surface(|s| {
                let mut s = s.borrow_mut();
                if let Some((x, y)) = s.press {
                    let (next_x, next_y) = event.root();
                    if !s.dragged && (next_x - x).hypot(next_y - y) >= 5.0 {
                        s.dragged = true;
                        s.window
                            .begin_move_drag(1, next_x as i32, next_y as i32, event.time());
                    }
                }
            });
            glib::Propagation::Stop
        });
        let handle = app.clone();
        area.connect_button_release_event(move |_, event| {
            if cfg!(debug_assertions) && std::env::var_os("AIEYES_LINUX_TRACE").is_some() {
                eprintln!("Linux pet release: {} {:?}", event.button(), event.root());
            }
            if event.button() == 1 {
                let dragged = with_surface(|s| {
                    let mut s = s.borrow_mut();
                    let pressed = s.press.take().is_some();
                    (pressed, s.dragged)
                });
                if let Some((true, false)) = dragged {
                    dispatch(&handle, "toggle-panel");
                } else if let Some((true, true)) = dragged {
                    with_surface(|s| position_surface(&s.borrow(), None, true));
                }
            }
            glib::Propagation::Stop
        });
        let handle = app.clone();
        window.connect_configure_event(move |window, _| {
            let (x, y) = window.position();
            let scale = window.scale_factor();
            crate::desktop::pet_moved(&handle, x * scale, y * scale);
            false
        });
        window.connect_delete_event(|_, _| glib::Propagation::Stop);
        position_surface(&surface.borrow(), position, false);
        let stopped = Arc::new(AtomicBool::new(false));
        if gdk::Display::default().is_some_and(|d| d.type_().name().contains("X11")) {
            watch_outside(app.clone(), stopped.clone());
        }
        Ok(Self { app, stopped })
    }
    pub fn status(&self, info: Value) {
        let _ = self.app.run_on_main_thread(move || {
            with_surface(|surface| {
                let mut s = surface.borrow_mut();
                if let Ok(next) = serde_json::from_value::<PetState>(info["pet"].clone()) {
                    if s.state.key != next.key
                        || s.state.frame_durations_ms != next.frame_durations_ms
                    {
                        s.start = Instant::now();
                    }
                    s.state = next;
                }
                let asset = info["petAsset"]["spritePath"].as_str().unwrap_or("");
                if asset != s.asset
                    && let Some(sprite) = crate::pet_frames::Sprite::load(&info["petAsset"])
                {
                    let width = sprite.cell_width as i32;
                    let height = sprite.cell_height as i32;
                    let decoded = sprite
                        .frames
                        .into_iter()
                        .map(|pixels| {
                            cairo::ImageSurface::create_for_data(
                                pixels,
                                cairo::Format::ARgb32,
                                width,
                                height,
                                width * 4,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>();
                    if let Ok(frames) = decoded {
                        s.frames = frames;
                        s.asset = asset.into();
                    }
                }
                s.info = info;
                let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
                let size = (
                    (144.0 * scale).round() as i32,
                    (aieyes_core::pets::height(&s.state) * scale).round() as i32,
                );
                if s.area.size_request() != size {
                    s.area.set_size_request(size.0, size.1);
                    s.window.resize(size.0, size.1);
                }
                s.area.set_tooltip_text(Some(&format!(
                    "Aieyes · {}\n单击打开面板 · 拖动移动 · 右键菜单",
                    s.state.label
                )));
                s.area.queue_draw();
                drop(s);
                schedule(surface);
            });
        });
    }
    pub fn visible(&self, visible: bool) {
        let _ = self.app.run_on_main_thread(move || {
            with_surface(|surface| {
                {
                    let mut s = surface.borrow_mut();
                    s.visible = visible;
                    if visible {
                        s.window.show_all();
                    } else {
                        s.window.hide();
                    }
                }
                schedule(surface);
            });
        });
    }
    pub fn position(&self, position: Option<(i32, i32)>) {
        let _ = self.app.run_on_main_thread(move || {
            with_surface(|s| position_surface(&s.borrow(), position, false));
        });
    }
    pub fn snap(&self) {
        let _ = self.app.run_on_main_thread(|| {
            with_surface(|s| position_surface(&s.borrow(), None, true));
        });
    }
    pub fn geometry(&self) -> Result<(i32, i32, u32, u32, f64), String> {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        self.app
            .run_on_main_thread(move || {
                let result = with_surface(|s| {
                    let s = s.borrow();
                    let (x, y) = s.window.position();
                    let (w, h) = s.window.size();
                    let scale = s.window.scale_factor();
                    (
                        x * scale,
                        y * scale,
                        (w * scale) as u32,
                        (h * scale) as u32,
                        f64::from(scale),
                    )
                })
                .ok_or_else(|| "桌宠不可用".to_string());
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        receive.recv().map_err(|e| e.to_string())?
    }
    pub fn watch_panel(&self, _handle: Option<usize>) {}
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.app.run_on_main_thread(|| {
            SURFACE.with(|slot| {
                if let Some(s) = slot.borrow_mut().take() {
                    let mut s = s.borrow_mut();
                    if let Some(timer) = s.timer.take() {
                        timer.remove();
                    }
                    s.window.close();
                }
            });
        });
    }
}

fn position_surface(s: &Surface, saved: Option<(i32, i32)>, snap: bool) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let (w, h) = s.area.size_request();
    let scale = s.window.scale_factor();
    let current = s.window.position();
    let monitor = saved
        .and_then(|(x, y)| display.monitor_at_point(x / scale, y / scale))
        .or_else(|| {
            s.window
                .window()
                .and_then(|w| display.monitor_at_window(&w))
        })
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0));
    let Some(monitor) = monitor else { return };
    let a = monitor.workarea();
    let (mut x, mut y) = saved
        .map(|(x, y)| (x / scale, y / scale))
        .unwrap_or_else(|| {
            if snap {
                current
            } else {
                (a.x() + a.width() - w - 24, a.y() + (a.height() - h) / 2)
            }
        });
    let right = (a.x() + a.width() - w).max(a.x());
    let bottom = (a.y() + a.height() - h).max(a.y());
    x = x.clamp(a.x(), right);
    y = y.clamp(a.y(), bottom);
    if snap {
        if x - a.x() <= 48 {
            x = a.x();
        } else if right - x <= 48 {
            x = right;
        }
        if y - a.y() <= 48 {
            y = a.y();
        } else if bottom - y <= 48 {
            y = bottom;
        }
    }
    s.window.move_(x, y);
}
fn reduced() -> bool {
    gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
}
fn schedule(surface: &Rc<RefCell<Surface>>) {
    let mut s = surface.borrow_mut();
    if let Some(timer) = s.timer.take() {
        timer.remove();
    }
    let delay = if s.visible {
        next_frame_delay(&s.state, s.start.elapsed().as_millis() as u64, reduced())
    } else {
        None
    };
    if let Some(delay) = delay {
        let weak = Rc::downgrade(surface);
        s.timer = Some(glib::timeout_add_local_once(
            Duration::from_millis(delay.max(1)),
            move || {
                if let Some(surface) = weak.upgrade() {
                    {
                        let mut s = surface.borrow_mut();
                        s.timer = None;
                        s.area.queue_draw();
                    }
                    schedule(&surface);
                }
            },
        ));
    }
}
fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    cr.close_path();
}
fn paint(s: &Surface, cr: &cairo::Context) -> Result<(), cairo::Error> {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint()?;
    cr.set_operator(cairo::Operator::Over);
    let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
    cr.scale(scale, scale);
    let index = frame(&s.state, s.start.elapsed().as_millis() as u64, reduced()) as usize;
    if let Some(image) = s.frames.get(index) {
        cr.save()?;
        cr.translate(6.0, 0.0);
        cr.scale(
            132.0 / f64::from(image.width()),
            143.0 / f64::from(image.height()),
        );
        cr.set_source_surface(image, 0.0, 0.0)?;
        cr.source().set_filter(cairo::Filter::Bilinear);
        cr.paint()?;
        cr.restore()?;
    }
    if s.info["showCount"] == true
        && let Some(count) = s.info["activeCount"].as_u64().filter(|n| *n > 0)
    {
        let text = if count > 99 {
            "99+".into()
        } else {
            count.to_string()
        };
        let layout = s.area.create_pango_layout(Some(&text));
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Sans 9")));
        let (width, height) = layout.pixel_size();
        let width = f64::from(width + 10);
        rounded(cr, 144.0 - width, 18.0, width, 20.0, 10.0);
        cr.set_source_rgb(0.30, 0.35, 0.85);
        cr.fill()?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(149.0 - width, 18.0 + (20.0 - f64::from(height)) / 2.0);
        pangocairo::functions::show_layout(cr, &layout);
    }
    let dark = gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
    let layout = s.area.create_pango_layout(Some(&s.state.label));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Sans 9")));
    layout.set_alignment(gtk::pango::Alignment::Center);
    let (width, height) = layout.pixel_size();
    let w = f64::from(width + 24).min(140.0);
    let h = f64::from(height + 6);
    let x = (144.0 - w) / 2.0;
    let y = aieyes_core::pets::height(&s.state) - h - 4.0;
    rounded(cr, x, y, w, h, 10.0);
    if dark {
        cr.set_source_rgb(0.125, 0.153, 0.208);
    } else {
        cr.set_source_rgb(0.965, 0.973, 0.988);
    }
    cr.fill()?;
    let rgb = crate::phase_colors::rgb(s.info["phase"].as_str().unwrap_or("idle"), dark);
    cr.set_source_rgb(
        f64::from(rgb[0]) / 255.0,
        f64::from(rgb[1]) / 255.0,
        f64::from(rgb[2]) / 255.0,
    );
    cr.arc(x + 8.0, y + h / 2.0, 2.5, 0.0, std::f64::consts::TAU);
    cr.fill()?;
    if dark {
        cr.set_source_rgb(0.93, 0.95, 0.99);
    } else {
        cr.set_source_rgb(0.14, 0.19, 0.29);
    }
    cr.move_to(x + 16.0, y + 3.0);
    pangocairo::functions::show_layout(cr, &layout);
    Ok(())
}

pub fn popup_menu(app: &AppHandle, event: Option<&gdk::EventButton>) {
    let info = crate::desktop::desktop_info(app.clone()).unwrap_or_default();
    let menu = gtk::Menu::new();
    for row in crate::context_menu::entries(
        true,
        info["petEnabled"] == true && info["hidden"] != true,
        info["trayAvailable"] == true,
        false,
    ) {
        if row.action == "separator" {
            menu.append(&gtk::SeparatorMenuItem::new());
            continue;
        }
        let item = gtk::MenuItem::with_label(row.label);
        item.set_sensitive(row.enabled);
        let handle = app.clone();
        let action = row.action.to_string();
        item.connect_activate(move |_| {
            if let Err(error) = crate::desktop::action(&handle, &action) {
                let _ = handle.emit("desktop:error", error);
            }
        });
        menu.append(&item);
    }
    crate::desktop::native_interaction(app, true);
    let handle = app.clone();
    menu.connect_selection_done(move |_| crate::desktop::native_interaction(&handle, false));
    menu.show_all();
    if let Some(event) = event {
        menu.popup_at_pointer(Some(event));
    } else {
        menu.popup_at_pointer(None::<&gdk::Event>);
    }
}

fn watch_outside(app: AppHandle, stopped: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        use x11rb::{
            connection::Connection,
            protocol::{
                Event,
                xinput::{self, ConnectionExt as _},
                xproto::ConnectionExt as _,
            },
        };
        let Ok((connection, screen)) = x11rb::connect(None) else {
            return;
        };
        let root = connection.setup().roots[screen].root;
        let Ok(version) = connection.xinput_xi_query_version(2, 0) else {
            return;
        };
        if version.reply().is_err() {
            return;
        }
        let mask = xinput::EventMask {
            deviceid: xinput::Device::ALL_MASTER.into(),
            mask: vec![
                xinput::XIEventMask::RAW_BUTTON_PRESS,
                xinput::XIEventMask::RAW_KEY_PRESS,
            ],
        };
        let Ok(selected) = connection.xinput_xi_select_events(root, &[mask]) else {
            return;
        };
        if selected.check().is_err() {
            return;
        }
        let _ = connection.flush();
        while !stopped.load(Ordering::Relaxed) {
            match connection.wait_for_event() {
                Ok(Event::XinputRawButtonPress(_)) => {
                    if let Ok(cookie) = connection.query_pointer(root)
                        && let Ok(pointer) = cookie.reply()
                    {
                        let app = app.clone();
                        let owner = app.clone();
                        let generation = crate::desktop::panel_generation(&app);
                        let _ = owner.run_on_main_thread(move || {
                            crate::desktop::outside_click(
                                &app,
                                i32::from(pointer.root_x),
                                i32::from(pointer.root_y),
                                generation,
                            )
                        });
                    }
                }
                Ok(Event::XinputRawKeyPress(event)) => {
                    // Escape closes the panel even when the WebKit child did
                    // not receive keyboard focus (common under XWayland).
                    if event.detail == 9 {
                        let app = app.clone();
                        let owner = app.clone();
                        let _ = owner.run_on_main_thread(move || {
                            crate::desktop::escape_pressed(&app);
                        });
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
}
