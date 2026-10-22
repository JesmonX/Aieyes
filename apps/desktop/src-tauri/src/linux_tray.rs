//! StatusNotifierItem tray for Linux.
//!
//! Tauri's AppIndicator backend is tied to the GTK/GDK connection and its
//! item disappears when the app runs through XWayland. The native pet and
//! panel deliberately use X11 for absolute placement, so the tray is served
//! directly over D-Bus instead. This also keeps the tray independent from the
//! display backend on Wayland sessions.

use aieyes_core::sessions::Phase;
use ksni::{
    Icon, ToolTip, Tray,
    blocking::{Handle, TrayMethods},
    menu::{MenuItem, StandardItem},
};
use tauri::{AppHandle, Emitter};

/// A watcher without a registered host cannot display the tray. Query it in
/// process: AppImage's bundled GLib can break a newer host's gdbus executable
/// through LD_LIBRARY_PATH, falsely hiding an otherwise working tray.
pub(crate) fn host_available() -> bool {
    use gtk::{gio, glib::variant::ToVariant};
    let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    connection
        .call_sync(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(
                &(
                    "org.kde.StatusNotifierWatcher",
                    "IsStatusNotifierHostRegistered",
                )
                    .to_variant(),
            ),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2_000,
            gio::Cancellable::NONE,
        )
        .is_ok_and(|reply| host_registered(&reply))
}

fn host_registered(reply: &gtk::glib::Variant) -> bool {
    reply
        .get::<(gtk::glib::Variant,)>()
        .is_some_and(|(value,)| value.get::<bool>() == Some(true))
}

pub(crate) struct LinuxTray {
    app: AppHandle,
    pet_visible: bool,
    can_hide: bool,
    sampling: bool,
    summary: String,
    phase: Option<Phase>,
    dark: bool,
}

impl LinuxTray {
    fn dispatch(&self, action: &'static str) {
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            if let Err(error) = crate::desktop::action(&app, action) {
                #[cfg(debug_assertions)]
                eprintln!("Linux tray action {action} failed: {error}");
                let _ = app.emit("desktop:error", error);
            }
        });
    }

    fn item(&self, label: &str, action: &'static str, enabled: bool) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            enabled,
            activate: Box::new(move |tray: &mut LinuxTray| tray.dispatch(action)),
            ..Default::default()
        }
        .into()
    }
}

impl Tray for LinuxTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "aieyes-status".into()
    }

    fn title(&self) -> String {
        "Aieyes".into()
    }

    fn status(&self) -> ksni::Status {
        // The tray is the fallback entry point and must stay visible even when
        // the pet is temporarily hidden.
        ksni::Status::Active
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let image = crate::desktop::icon(self.phase, self.dark);
        let mut data = Vec::with_capacity(image.rgba().len());
        for pixel in image.rgba().as_chunks::<4>().0 {
            data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
        }
        vec![Icon {
            width: image.width() as i32,
            height: image.height() as i32,
            data,
        }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Aieyes".into(),
            description: self.summary.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = vec![self.item(crate::localization::label("打开面板"), "open-panel", true)];
        for row in
            crate::context_menu::entries(false, self.pet_visible, self.can_hide, self.sampling)
        {
            if row.action == "separator" {
                items.push(MenuItem::Separator);
            } else {
                items.push(self.item(row.label, row.action, row.enabled));
            }
        }
        items
    }
}

pub(crate) struct LinuxTrayHandle {
    handle: Handle<LinuxTray>,
}

impl LinuxTrayHandle {
    pub(crate) fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    fn update(&self, f: impl FnOnce(&mut LinuxTray)) {
        let _ = self.handle.update(f);
    }

    pub(crate) fn set_menu(&self, visible: bool, can_hide: bool, sampling: bool) {
        self.update(|tray| {
            tray.pet_visible = visible;
            tray.can_hide = can_hide;
            tray.sampling = sampling;
        });
    }

    pub(crate) fn set_icon(&self, phase: Option<Phase>, dark: bool) {
        self.update(|tray| {
            tray.phase = phase;
            tray.dark = dark;
        });
    }

    pub(crate) fn set_summary(&self, summary: String) {
        self.update(|tray| tray.summary = summary);
    }
}

pub(crate) fn start(app: &AppHandle) -> Result<LinuxTrayHandle, String> {
    let tray = LinuxTray {
        app: app.clone(),
        pet_visible: true,
        can_hide: false,
        sampling: false,
        summary: "Aieyes".into(),
        phase: None,
        dark: false,
    };
    let handle = tray
        .spawn()
        .map_err(|error| format!("无法创建 Linux 状态栏图标：{error}"))?;
    Ok(LinuxTrayHandle { handle })
}

#[cfg(test)]
mod tests {
    use super::host_registered;
    use gtk::glib::variant::ToVariant;

    #[test]
    fn only_a_registered_status_notifier_host_can_replace_the_pet() {
        assert!(host_registered(&(true.to_variant(),).to_variant()));
        assert!(!host_registered(&(false.to_variant(),).to_variant()));
        assert!(!host_registered(&("true".to_variant(),).to_variant()));
        assert!(!host_registered(&true.to_variant()));
    }
}
