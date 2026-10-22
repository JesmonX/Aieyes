//! Use the same embedded, unmodified fonts for WebKit and native GTK text.
use gtk::{glib::translate::ToGlibPtr, prelude::*};
use std::{ffi::CString, os::raw::c_void, os::unix::ffi::OsStrExt};
use tauri::{AppHandle, Manager};

const FONTS: [&str; 3] = [
    "HarmonyOS_Sans_SC_Regular.ttf",
    "HarmonyOS_Sans_SC_Medium.ttf",
    "HarmonyOS_Sans_SC_Bold.ttf",
];

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcInitLoadConfigAndFonts() -> *mut c_void;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const u8) -> i32;
    fn FcConfigDestroy(config: *mut c_void);
}

#[link(name = "pangoft2-1.0")]
unsafe extern "C" {
    fn pango_fc_font_map_set_config(
        font_map: *mut gtk::pango::ffi::PangoFontMap,
        config: *mut c_void,
    );
}

struct FontConfig(*mut c_void);
impl Drop for FontConfig {
    fn drop(&mut self) {
        // SAFETY: setup creates this owned configuration and rejects null.
        unsafe { FcConfigDestroy(self.0) };
    }
}

// Fontconfig opens files lazily, so retain them for the application's lifetime.
struct FontFiles {
    _directory: tempfile::TempDir,
}

pub fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::Builder::new().prefix("aieyes-fonts-").tempdir()?;
    for name in FONTS.into_iter().chain(["LICENSE.txt", "sources.json"]) {
        let asset = app
            .asset_resolver()
            .get(format!("fonts/{name}"))
            .ok_or_else(|| format!("Missing bundled font asset: {name}"))?;
        std::fs::write(directory.path().join(name), asset.bytes)?;
    }

    // A private configuration retains system fallbacks without installing fonts
    // or changing the user's Fontconfig settings. Do not touch the live map until
    // all three faces have been registered successfully.
    let pointer = unsafe { FcInitLoadConfigAndFonts() };
    if pointer.is_null() {
        return Err("Could not initialize the application font configuration".into());
    }
    let config = FontConfig(pointer);
    for name in FONTS {
        let path = CString::new(directory.path().join(name).as_os_str().as_bytes())?;
        // SAFETY: config is live and path is a NUL-terminated filename.
        if unsafe { FcConfigAppFontAddFile(config.0, path.as_ptr().cast()) } == 0 {
            return Err(format!("Could not register bundled font: {name}").into());
        }
    }
    let font_map = pangocairo::FontMap::default();
    // SAFETY: Linux's PangoCairo map uses Fontconfig. set_config keeps its own
    // reference and invalidates caches, including those created during GTK init.
    unsafe { pango_fc_font_map_set_config(font_map.to_glib_none().0, config.0) };
    app.manage(FontFiles {
        _directory: directory,
    });

    if let Some(settings) = gtk::Settings::default() {
        let existing = settings.gtk_font_name();
        let mut font =
            gtk::pango::FontDescription::from_string(existing.as_deref().unwrap_or("Sans 10"));
        font.set_family("HarmonyOS Sans SC");
        settings.set_gtk_font_name(Some(font.to_string().as_str()));
    }
    Ok(())
}
