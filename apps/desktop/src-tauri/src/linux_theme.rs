//! Resolve system appearance without resetting GTK's startup preference.
use gtk::{gio, glib, prelude::*};
use std::{
    cell::RefCell,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
use tauri::{AppHandle, Emitter};
static PORTAL: AtomicU8 = AtomicU8::new(0);
static STARTUP_DARK: AtomicBool = AtomicBool::new(false);
static PREFERENCE: Mutex<String> = Mutex::new(String::new());
thread_local! { static BUS: RefCell<Option<gio::DBusConnection>> = const { RefCell::new(None) }; }
fn scheme(value: &glib::Variant) -> Option<u32> {
    value
        .get::<u32>()
        .or_else(|| value.as_variant().and_then(|v| scheme(&v)))
}
pub fn apply(app: &AppHandle, preference: &str) {
    *PREFERENCE.lock().unwrap_or_else(|e| e.into_inner()) = preference.into();
    let dark = match preference {
        "dark" => true,
        "light" => false,
        _ => match PORTAL.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => STARTUP_DARK.load(Ordering::Relaxed),
        },
    };
    let theme = if dark {
        tauri::Theme::Dark
    } else {
        tauri::Theme::Light
    };
    app.set_theme(Some(theme));
    let _ = app.emit("desktop:system-theme", serde_json::json!({"dark":dark}));
}
pub fn setup(app: &AppHandle) {
    STARTUP_DARK.store(
        gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme()),
        Ordering::Relaxed,
    );
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    use glib::variant::ToVariant;
    if let Ok(reply) = bus.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "Read",
        Some(&("org.freedesktop.appearance", "color-scheme").to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1500,
        gio::Cancellable::NONE,
    ) {
        if let Some(value) = reply.child_value(0).as_variant().and_then(|v| scheme(&v)) {
            PORTAL.store(value.min(2) as u8, Ordering::Relaxed);
        }
    }
    let handle = app.clone();
    bus.signal_subscribe(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.Settings"),
        Some("SettingChanged"),
        Some("/org/freedesktop/portal/desktop"),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, value| {
            if value.child_value(0).str() != Some("org.freedesktop.appearance")
                || value.child_value(1).str() != Some("color-scheme")
            {
                return;
            }
            if let Some(value) = scheme(&value.child_value(2)) {
                PORTAL.store(value.min(2) as u8, Ordering::Relaxed);
                let preference = PREFERENCE.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if preference == "system" || preference.is_empty() {
                    apply(&handle, "system");
                }
            }
        },
    );
    BUS.with(|slot| *slot.borrow_mut() = Some(bus));
}
