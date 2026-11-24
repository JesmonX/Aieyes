//! GTK pet surface. Animation never resizes/repaints a WebKit window.
use aieyes_core::pets::{
    ORBIT_NODE_SIZE, ORBIT_OVERFLOW_PROVIDER, PetOrbitAgent, PetState, canvas_size, frame,
    next_frame_delay, normalized_scale, orbit_padding, orbit_slots,
};
use aieyes_core::quota_ring::Ring;
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
    orbit_agents: Vec<PetOrbitAgent>,
    orbit_hover: Option<usize>,
    orbit_start: Instant,
    applied_pad: (f64, f64),
    frames: Vec<cairo::ImageSurface>,
    asset: String,
    shapes: Vec<cairo::Region>,
    shape_scale: f64,
    shape_key: String,
    start: Instant,
    timer: Option<glib::SourceId>,
    visible: bool,
    press: Option<(f64, f64)>,
    dragged: bool,
}

/// A user explicitly opened this window. Coordinate GTK activation with X11's
/// keyboard focus, including WMs that decline activation of a skip-taskbar window.
pub fn focus_window(window: &gtk::ApplicationWindow) {
    let mut time = gtk::current_event_time();
    let Some(surface) = window.window() else {
        window.present_with_time(time);
        return;
    };
    let Ok(surface) = surface.downcast::<gdkx11::X11Window>() else {
        window.present_with_time(time);
        return;
    };
    if time == 0 {
        // DBusMenu actions have no GTK event timestamp. Reusing the last pet
        // click (or CurrentTime) lets GNOME reject focus after an outside click.
        time = gdkx11::functions::x11_get_server_time(&surface);
    }
    surface.set_user_time(time);
    window.present_with_time(time);
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
        let _ = connection.set_input_focus(InputFocus::PARENT, id, time);
        let _ = connection.flush();
    }
}

fn with_surface<T>(f: impl FnOnce(&Rc<RefCell<Surface>>) -> T) -> Option<T> {
    SURFACE.with(|slot| slot.borrow().as_ref().map(f))
}
fn dispatch(app: &AppHandle, action: &str) {
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
            .or_else(|| app.get_webview_window("floating"))
            .ok_or("主窗口不可用")?
            .gtk_window()
            .map_err(|e| e.to_string())?
            .application()
            .ok_or("GTK 应用不可用")?;
        let window = gtk::ApplicationWindow::new(&application);
        window.set_title(crate::localization::label("Aieyes 桌宠"));
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
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK,
        );
        window.add(&area);
        let surface = Rc::new(RefCell::new(Surface {
            window: window.clone(),
            area: area.clone(),
            info: Value::Null,
            state: aieyes_core::pets::project(&Default::default()),
            orbit_agents: Vec::new(),
            orbit_hover: None,
            orbit_start: Instant::now(),
            applied_pad: (0.0, 0.0),
            frames: vec![],
            asset: String::new(),
            shapes: vec![],
            shape_scale: 0.0,
            shape_key: String::new(),
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
                update_shape(&mut surface.borrow_mut());
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
                let (lx, ly) = event.position();
                let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
                let hovered = crate::pet_orbit::hit_test(lx, ly, scale, &s.orbit_agents);
                if s.orbit_hover != hovered {
                    s.orbit_hover = hovered;
                    let tip = hovered
                        .and_then(|idx| s.orbit_agents.get(idx))
                        .map(crate::pet_orbit::node_tooltip)
                        .unwrap_or_else(|| {
                            crate::localization::text(&format!(
                                "Aieyes · {}\n单击打开面板 · 拖动移动 · 右键菜单",
                                crate::localization::text(&s.state.label)
                            ))
                        });
                    s.area.set_tooltip_text(Some(&tip));
                    s.area.queue_draw();
                }
                if let Some((x, y)) = s.press {
                    let (next_x, next_y) = event.root();
                    if !s.dragged && (next_x - x).hypot(next_y - y) >= 5.0 {
                        s.dragged = true;
                        s.window
                            .begin_move_drag(1, next_x as i32, next_y as i32, event.time());
                        glib::timeout_add_local(Duration::from_millis(60), move || {
                            with_surface(|surface| {
                                let mut s = surface.borrow_mut();
                                if s.press.is_none() || !s.dragged {
                                    return glib::ControlFlow::Break;
                                }
                                let pressed = s
                                    .window
                                    .window()
                                    .and_then(|window| {
                                        window
                                            .display()
                                            .default_seat()
                                            .and_then(|seat| seat.pointer())
                                            .map(|pointer| {
                                                window
                                                    .device_position(&pointer)
                                                    .3
                                                    .contains(gdk::ModifierType::BUTTON1_MASK)
                                            })
                                    })
                                    .unwrap_or(false);
                                if pressed {
                                    glib::ControlFlow::Continue
                                } else {
                                    s.press = None;
                                    position_surface(&s, None, true);
                                    glib::ControlFlow::Break
                                }
                            })
                            .unwrap_or(glib::ControlFlow::Break)
                        });
                    }
                }
            });
            glib::Propagation::Stop
        });
        area.connect_leave_notify_event(move |_, _| {
            with_surface(|s| {
                let mut s = s.borrow_mut();
                if s.orbit_hover.take().is_some() {
                    s.area
                        .set_tooltip_text(Some(&crate::localization::text(&format!(
                            "Aieyes · {}\n单击打开面板 · 拖动移动 · 右键菜单",
                            crate::localization::text(&s.state.label)
                        ))));
                    s.area.queue_draw();
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
                let (lx, ly) = event.position();
                let outcome = with_surface(|s| {
                    let mut s = s.borrow_mut();
                    let pressed = s.press.take().is_some();
                    let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
                    let hit = crate::pet_orbit::hit_test(lx, ly, scale, &s.orbit_agents)
                        .and_then(|idx| s.orbit_agents.get(idx).map(|a| a.provider.clone()));
                    (pressed, s.dragged, hit)
                });
                if let Some((true, false, hit)) = outcome {
                    if let Some(provider) = hit {
                        dispatch(&handle, &format!("orbit-agent:{provider}"));
                    } else {
                        dispatch(&handle, "toggle-panel");
                    }
                } else if let Some((true, true, _)) = outcome {
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
                    if !aieyes_core::pets::same_animation(&s.state, &next) {
                        s.start = Instant::now();
                    }
                    s.state = next;
                }
                let asset = info["petAsset"]["spritePath"].as_str().unwrap_or("");
                if asset != s.asset
                    && let Some(sprite) = crate::pet_frames::Sprite::load(&info["petAsset"])
                {
                    s.start = Instant::now();
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
                        s.shapes.clear();
                        s.shape_key.clear();
                    }
                }
                let next_orbit = crate::pet_orbit::parse_agents(&info);
                if s.orbit_agents.is_empty() && !next_orbit.is_empty() {
                    s.orbit_start = Instant::now();
                }
                if s.orbit_hover.is_some_and(|idx| idx >= next_orbit.len()) {
                    s.orbit_hover = None;
                }
                s.orbit_agents = next_orbit;
                s.info = info;
                let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
                let has_orbit = !s.orbit_agents.is_empty();
                let (logical_w, logical_h) = canvas_size(&s.state, has_orbit);
                let next_pad = orbit_padding(has_orbit);
                let old_pad = s.applied_pad;
                s.applied_pad = next_pad;
                let size = (
                    (logical_w * scale).round() as i32,
                    (logical_h * scale).round() as i32,
                );
                if s.area.size_request() != size {
                    s.area.set_size_request(size.0, size.1);
                    s.window.resize(size.0, size.1);
                    if old_pad != next_pad {
                        let (wx, wy) = s.window.position();
                        let dx = ((next_pad.0 - old_pad.0) * scale).round() as i32;
                        let dy = ((next_pad.1 - old_pad.1) * scale).round() as i32;
                        s.window.move_(wx - dx, wy - dy);
                        position_surface(&s, None, false);
                    }
                }
                s.window
                    .set_title(crate::localization::label("Aieyes 桌宠"));
                s.area
                    .set_tooltip_text(Some(&crate::localization::text(&format!(
                        "Aieyes · {}\n单击打开面板 · 拖动移动 · 右键菜单",
                        crate::localization::text(&s.state.label)
                    ))));
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
        let is_reduced = reduced();
        let frame_delay =
            next_frame_delay(&s.state, s.start.elapsed().as_millis() as u64, is_reduced);
        if !is_reduced && !s.orbit_agents.is_empty() {
            Some(frame_delay.map_or(crate::pet_orbit::PULSE_FRAME_MS, |d| {
                d.min(crate::pet_orbit::PULSE_FRAME_MS)
            }))
        } else {
            frame_delay
        }
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
    let has_orbit = !s.orbit_agents.is_empty();
    let (pad_x, pad_top) = orbit_padding(has_orbit);
    let index = frame(&s.state, s.start.elapsed().as_millis() as u64, reduced()) as usize;
    if let Some(image) = s.frames.get(index) {
        cr.save()?;
        cr.translate(pad_x + 6.0, pad_top);
        cr.scale(
            132.0 / f64::from(image.width()),
            143.0 / f64::from(image.height()),
        );
        cr.set_source_surface(image, 0.0, 0.0)?;
        cr.source().set_filter(cairo::Filter::Best);
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
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
            "HarmonyOS Sans SC 9",
        )));
        let (width, height) = layout.pixel_size();
        let width = f64::from(width + 10);
        rounded(cr, pad_x + 144.0 - width, pad_top + 18.0, width, 20.0, 10.0);
        cr.set_source_rgb(0.30, 0.35, 0.85);
        cr.fill()?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(
            pad_x + 149.0 - width,
            pad_top + 18.0 + (20.0 - f64::from(height)) / 2.0,
        );
        pangocairo::functions::show_layout(cr, &layout);
    }
    let dark = gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
    if has_orbit {
        paint_orbit_nodes(s, cr, dark)?;
    }
    let label = crate::localization::text(&s.state.label);
    let layout = s.area.create_pango_layout(Some(&label));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
        "HarmonyOS Sans SC 9",
    )));
    layout.set_alignment(gtk::pango::Alignment::Center);
    let (width, height) = layout.pixel_size();
    let w = f64::from(width + 24).min(140.0);
    let h = f64::from(height + 6);
    let x = pad_x + (144.0 - w) / 2.0;
    let y = pad_top + aieyes_core::pets::height(&s.state) - h - 4.0;
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

fn paint_orbit_nodes(s: &Surface, cr: &cairo::Context, dark: bool) -> Result<(), cairo::Error> {
    let slots = orbit_slots(s.orbit_agents.len());
    let elapsed_ms = s.orbit_start.elapsed().as_millis() as u64;
    let is_reduced = reduced();
    let selected = s.info["orbitAgent"].as_str();

    for (index, (agent, slot)) in s.orbit_agents.iter().zip(slots.iter()).enumerate() {
        let is_selected = selected == Some(agent.provider.as_str());
        let is_hovered = s.orbit_hover == Some(index);
        let boost = if is_hovered || is_selected { 1.08 } else { 1.0 };
        let radius = (ORBIT_NODE_SIZE / 2.0) * boost;
        let (cx, cy) = (slot.cx, slot.cy);

        // 1. Breathing halo
        let (halo_alpha, halo_scale) =
            crate::pet_orbit::pulse_metrics(elapsed_ms, index, agent.phase, is_reduced);
        let [pr, pg, pb] = crate::pet_orbit::pulse_rgb(agent, dark);
        cr.save()?;
        cr.set_source_rgba(
            f64::from(pr) / 255.0,
            f64::from(pg) / 255.0,
            f64::from(pb) / 255.0,
            halo_alpha,
        );
        cr.set_line_width(2.4);
        cr.arc(
            cx,
            cy,
            radius * halo_scale + 1.5,
            0.0,
            std::f64::consts::TAU,
        );
        cr.stroke()?;
        cr.restore()?;

        // 2. Disc backdrop + border
        cr.save()?;
        cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        if dark {
            cr.set_source_rgba(0.137, 0.161, 0.212, 0.92);
        } else {
            cr.set_source_rgba(0.973, 0.980, 0.988, 0.94);
        }
        cr.fill_preserve()?;
        if is_selected {
            if dark {
                cr.set_source_rgba(0.639, 0.702, 1.0, 1.0);
            } else {
                cr.set_source_rgba(0.333, 0.400, 0.851, 1.0);
            }
            cr.set_line_width(1.6);
        } else {
            if dark {
                cr.set_source_rgba(0.529, 0.580, 0.678, 0.40);
            } else {
                cr.set_source_rgba(0.482, 0.529, 0.616, 0.34);
            }
            cr.set_line_width(0.9);
        }
        cr.stroke()?;
        cr.restore()?;

        // 3. Quota ring
        let is_overflow = agent.provider == ORBIT_OVERFLOW_PROVIDER;
        let ring_r = radius - 2.2;
        let ring_width = 2.4;
        if !is_overflow {
            let ring = agent
                .quota
                .as_ref()
                .map(|q| &q.ring)
                .unwrap_or(&Ring::Empty);
            cr.save()?;
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_width(ring_width);
            let set_track = |cr: &cairo::Context| {
                if dark {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.24);
                } else {
                    cr.set_source_rgba(0.145, 0.145, 0.157, 0.17);
                }
            };
            match ring {
                Ring::Duo { top, bottom } => {
                    let half = (ring_width / 2.0 + 0.8) / ring_r;
                    let span = std::f64::consts::PI - 2.0 * half;
                    set_track(cr);
                    cr.arc(
                        cx,
                        cy,
                        ring_r,
                        std::f64::consts::PI + half,
                        std::f64::consts::TAU - half,
                    );
                    cr.stroke()?;
                    cr.arc_negative(cx, cy, ring_r, std::f64::consts::PI - half, half);
                    cr.stroke()?;
                    let top_rem = top.remaining.clamp(0.0, 100.0);
                    if top_rem > 0.0 {
                        let [r, g, b] = crate::pet_orbit::quota_arc_rgb(
                            &agent.display_provider,
                            top.remaining,
                            dark,
                        );
                        cr.set_source_rgb(
                            f64::from(r) / 255.0,
                            f64::from(g) / 255.0,
                            f64::from(b) / 255.0,
                        );
                        cr.arc(
                            cx,
                            cy,
                            ring_r,
                            std::f64::consts::PI + half,
                            std::f64::consts::PI + half + (span * top_rem / 100.0).max(0.02),
                        );
                        cr.stroke()?;
                    }
                    let bot_rem = bottom.remaining.clamp(0.0, 100.0);
                    if bot_rem > 0.0 {
                        let [r, g, b] = crate::pet_orbit::quota_arc_rgb(
                            &agent.display_provider,
                            bottom.remaining,
                            dark,
                        );
                        cr.set_source_rgb(
                            f64::from(r) / 255.0,
                            f64::from(g) / 255.0,
                            f64::from(b) / 255.0,
                        );
                        cr.arc_negative(
                            cx,
                            cy,
                            ring_r,
                            std::f64::consts::PI - half,
                            std::f64::consts::PI - half - (span * bot_rem / 100.0).max(0.02),
                        );
                        cr.stroke()?;
                    }
                }
                Ring::Single { top } => {
                    set_track(cr);
                    cr.arc(cx, cy, ring_r, 0.0, std::f64::consts::TAU);
                    cr.stroke()?;
                    let rem = top.remaining.clamp(0.0, 100.0);
                    if rem > 0.0 {
                        let [r, g, b] = crate::pet_orbit::quota_arc_rgb(
                            &agent.display_provider,
                            top.remaining,
                            dark,
                        );
                        cr.set_source_rgb(
                            f64::from(r) / 255.0,
                            f64::from(g) / 255.0,
                            f64::from(b) / 255.0,
                        );
                        let start = -std::f64::consts::FRAC_PI_2;
                        cr.arc(
                            cx,
                            cy,
                            ring_r,
                            start,
                            start + (std::f64::consts::TAU * rem / 100.0).max(0.02),
                        );
                        cr.stroke()?;
                    }
                }
                Ring::Coin { low, .. } => {
                    let [r, g, b] = if *low {
                        if dark {
                            [0xff, 0x92, 0x9c]
                        } else {
                            [0xb8, 0x32, 0x45]
                        }
                    } else {
                        crate::pet_orbit::provider_rgb(&agent.display_provider, dark)
                    };
                    cr.set_line_width(1.8);
                    cr.set_source_rgba(
                        f64::from(r) / 255.0,
                        f64::from(g) / 255.0,
                        f64::from(b) / 255.0,
                        0.65,
                    );
                    cr.arc(cx, cy, ring_r, 0.0, std::f64::consts::TAU);
                    cr.stroke()?;
                }
                Ring::Empty => {
                    set_track(cr);
                    cr.set_line_width(1.6);
                    cr.set_dash(&[1.5, 3.0], 0.0);
                    cr.arc(cx, cy, ring_r, 0.0, std::f64::consts::TAU);
                    cr.stroke()?;
                }
            }
            cr.restore()?;
        }

        // 4. Center provider icon or +N overflow label
        if is_overflow {
            let text = format!("+{}", agent.overflow_providers.len());
            let layout = s.area.create_pango_layout(Some(&text));
            layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
                "HarmonyOS Sans SC Bold 8",
            )));
            let (tw, th) = layout.pixel_size();
            if dark {
                cr.set_source_rgb(0.93, 0.95, 0.99);
            } else {
                cr.set_source_rgb(0.14, 0.19, 0.29);
            }
            cr.move_to(cx - f64::from(tw) / 2.0, cy - f64::from(th) / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
        } else if let Some(icon) = crate::pet_icons::load(&agent.display_provider, dark)
            && let Ok(surface) = cairo::ImageSurface::create_for_data(
                icon.pixels,
                cairo::Format::ARgb32,
                icon.width as i32,
                icon.height as i32,
                (icon.width * 4) as i32,
            )
        {
            let icon_size = 16.0 * boost;
            cr.save()?;
            cr.translate(cx - icon_size / 2.0, cy - icon_size / 2.0);
            cr.scale(
                icon_size / f64::from(icon.width),
                icon_size / f64::from(icon.height),
            );
            cr.set_source_surface(&surface, 0.0, 0.0)?;
            cr.source().set_filter(cairo::Filter::Best);
            cr.paint()?;
            cr.restore()?;
        }

        // 5. Multi-session count badge
        if !is_overflow && agent.active_count > 1 {
            let count_text = if agent.active_count > 9 {
                "9+".to_owned()
            } else {
                agent.active_count.to_string()
            };
            let layout = s.area.create_pango_layout(Some(&count_text));
            layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
                "HarmonyOS Sans SC Bold 7",
            )));
            let (tw, th) = layout.pixel_size();
            let pw = f64::from(tw + 6).max(13.0);
            let ph = 13.0;
            let px = cx + radius - pw + 3.0;
            let py = cy + radius - ph + 3.0;
            rounded(cr, px, py, pw, ph, ph / 2.0);
            if agent.phase == aieyes_core::sessions::Phase::NeedsInput {
                cr.set_source_rgb(0.54, 0.34, 0.07);
            } else {
                cr.set_source_rgb(0.30, 0.35, 0.85);
            }
            cr.fill()?;
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(
                px + (pw - f64::from(tw)) / 2.0,
                py + (ph - f64::from(th)) / 2.0,
            );
            pangocairo::functions::show_layout(cr, &layout);
        }
    }
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
            mask: vec![xinput::XIEventMask::RAW_BUTTON_PRESS],
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
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
}

// Cache alpha regions per sprite/scale. Only frame selection and small label unions
// run during animation; transparent pixels never intercept clicks on the desktop.
fn update_shape(s: &mut Surface) {
    let Some(window) = s.window.window() else {
        return;
    };
    let scale = normalized_scale(s.info["petScale"].as_f64().unwrap_or(1.0));
    if s.shapes.len() != s.frames.len() || s.shape_scale != scale {
        s.shape_scale = scale;
        s.shape_key.clear();
        s.shapes = s
            .frames
            .iter()
            .map(|image| {
                let mut rectangles = Vec::new();
                let width = (132.0 * scale).round() as i32;
                let height = (143.0 * scale).round() as i32;
                let _ = image.with_data(|bytes| {
                    for y in 0..height {
                        let mut start = None;
                        for x in 0..=width {
                            let opaque = x < width && {
                                let sx = x * image.width() / width;
                                let sy = y * image.height() / height;
                                let offset = (sy * image.stride() + sx * 4) as usize;
                                bytes
                                    .get(
                                        offset + if cfg!(target_endian = "little") { 3 } else { 0 },
                                    )
                                    .is_some_and(|alpha| *alpha >= 20)
                            };
                            if opaque && start.is_none() {
                                start = Some(x)
                            }
                            if !opaque && let Some(left) = start.take() {
                                rectangles.push(cairo::RectangleInt::new(
                                    left + (6.0 * scale).round() as i32,
                                    y,
                                    x - left,
                                    1,
                                ));
                            }
                        }
                    }
                });
                cairo::Region::create_rectangles(&rectangles)
            })
            .collect();
    }
    let frame = frame(&s.state, s.start.elapsed().as_millis() as u64, reduced()) as usize;
    let composited =
        gtk::prelude::WidgetExt::screen(&s.window).is_some_and(|screen| screen.is_composited());
    let has_orbit = !s.orbit_agents.is_empty();
    let (pad_x, pad_top) = orbit_padding(has_orbit);
    let key = format!(
        "{frame}:{composited}:{}:{}:{}:{}",
        s.state.label,
        s.info["showCount"],
        s.info["activeCount"],
        s.orbit_agents.len()
    );
    if key == s.shape_key {
        return;
    }
    s.shape_key = key;
    let region = s
        .shapes
        .get(frame)
        .map(cairo::Region::copy)
        .unwrap_or_else(cairo::Region::create);
    if has_orbit {
        region.translate(
            (pad_x * scale).round() as i32,
            (pad_top * scale).round() as i32,
        );
    }
    let label = s
        .area
        .create_pango_layout(Some(&crate::localization::text(&s.state.label)));
    label.set_font_description(Some(&gtk::pango::FontDescription::from_string(
        "HarmonyOS Sans SC 9",
    )));
    let (w, h) = label.pixel_size();
    let w = f64::from(w + 24).min(140.0);
    let h = f64::from(h + 6);
    let _ = region.union_rectangle(&cairo::RectangleInt::new(
        ((pad_x + (144.0 - w) / 2.0) * scale) as i32,
        ((pad_top + aieyes_core::pets::height(&s.state) - h - 4.0) * scale) as i32,
        (w * scale).ceil() as i32,
        (h * scale).ceil() as i32,
    ));
    if s.info["showCount"] == true && s.info["activeCount"].as_u64().is_some_and(|n| n > 0) {
        let _ = region.union_rectangle(&cairo::RectangleInt::new(
            ((pad_x + 108.0) * scale) as i32,
            ((pad_top + 18.0) * scale) as i32,
            (36.0 * scale) as i32,
            (20.0 * scale) as i32,
        ));
    }
    for slot in orbit_slots(s.orbit_agents.len()) {
        let r = crate::pet_orbit::HIT_RADIUS;
        let _ = region.union_rectangle(&cairo::RectangleInt::new(
            ((slot.cx - r) * scale).floor() as i32,
            ((slot.cy - r) * scale).floor() as i32,
            (r * 2.0 * scale).ceil() as i32,
            (r * 2.0 * scale).ceil() as i32,
        ));
    }
    window.input_shape_combine_region(&region, 0, 0);
    window.shape_combine_region(if composited { None } else { Some(&region) }, 0, 0);
}
