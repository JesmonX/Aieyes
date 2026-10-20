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

pub(crate) struct LinuxTray {
    app: AppHandle,
    visible: bool,
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
        let mut items = vec![self.item("打开面板", "open-panel", true)];
        for row in crate::context_menu::entries(false, self.visible, self.can_hide, self.sampling) {
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
    fn update(&self, f: impl FnOnce(&mut LinuxTray)) {
        let _ = self.handle.update(f);
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.update(|tray| tray.visible = visible);
    }

    pub(crate) fn set_menu(&self, visible: bool, can_hide: bool, sampling: bool) {
        self.update(|tray| {
            tray.visible = visible;
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
        visible: true,
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
