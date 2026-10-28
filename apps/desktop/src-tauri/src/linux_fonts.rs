//! Use the same embedded, unmodified fonts for WebKit and native GTK text.
use gtk::glib::translate::ToGlibPtr;
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

// Cached files stay available for Fontconfig's lazy readers across restarts.
pub fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use std::hash::{Hash, Hasher};
    let assets: Vec<_> = FONTS
        .into_iter()
        .chain(["LICENSE.txt", "sources.json"])
        .map(|name| {
            app.asset_resolver()
                .get(format!("fonts/{name}"))
                .map(|asset| (name, asset.bytes))
                .ok_or_else(|| format!("Missing bundled font asset: {name}"))
        })
        .collect::<Result<_, _>>()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for (name, bytes) in &assets {
        name.hash(&mut hash);
        bytes.hash(&mut hash);
    }
    let directory = app
        .path()
        .app_cache_dir()?
        .join("fonts")
        .join(format!("{:016x}", hash.finish()));
    std::fs::create_dir_all(&directory)?;
    for (name, bytes) in assets {
        let path = directory.join(name);
        if !std::fs::read(&path).is_ok_and(|value| value == bytes) {
            let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
            std::io::Write::write_all(&mut temporary, &bytes)?;
            temporary.persist(path)?;
        }
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
        let path = CString::new(directory.join(name).as_os_str().as_bytes())?;
        // SAFETY: config is live and path is a NUL-terminated filename.
        if unsafe { FcConfigAppFontAddFile(config.0, path.as_ptr().cast()) } == 0 {
            return Err(format!("Could not register bundled font: {name}").into());
        }
    }
    let font_map = pangocairo::FontMap::default();
    // SAFETY: Linux's PangoCairo map uses Fontconfig. set_config keeps its own
    // reference and invalidates caches, including those created during GTK init.
    unsafe { pango_fc_font_map_set_config(font_map.to_glib_none().0, config.0) };
    Ok(())
}
