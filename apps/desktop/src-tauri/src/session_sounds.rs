//! Native playback continues while WebViews are hidden, without browser autoplay policy.
use aieyes_core::session_sounds::{Preferences, Tracker};
use aieyes_core::sessions::Snapshot;
use std::sync::Mutex;
use tauri::Manager;

#[derive(Default)]
pub struct Sounds(Mutex<Tracker>);
fn bytes(kind: &str) -> Result<&'static [u8], String> {
    match kind {
        "needsInput" => Ok(include_bytes!("../../web/sounds/needs-input.wav")),
        "complete" => Ok(include_bytes!("../../web/sounds/complete.wav")),
        _ => Err("无效提示音".into()),
    }
}
pub fn update(app: &tauri::AppHandle, snapshot: &Snapshot, preferences: &Preferences) {
    let service = app.state::<Sounds>();
    let alerts = service
        .0
        .lock()
        .map(|mut tracker| {
            tracker.update(
                snapshot,
                aieyes_core::models::now().max(0) as u64,
                preferences,
            )
        })
        .unwrap_or_default();
    if !alerts.is_empty() {
        std::thread::spawn(move || {
            for kind in alerts {
                let _ = play(bytes(kind).expect("known sound"));
            }
        });
    }
}
#[tauri::command]
pub async fn session_sounds_preview(kind: String) -> Result<(), String> {
    let bytes = bytes(&kind)?;
    tauri::async_runtime::spawn_blocking(move || play(bytes))
        .await
        .map_err(|e| e.to_string())?
}
#[cfg(windows)]
fn play(bytes: &'static [u8]) -> Result<(), String> {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    // SND_MEMORY | SND_NODEFAULT, synchronous on a worker so bytes outlive playback.
    if unsafe { PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), 0x0004 | 0x0002) } == 0 {
        Err("无法播放提示音，请检查系统音频输出".into())
    } else {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn play(bytes: &'static [u8]) -> Result<(), String> {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::io::Write;
    type Ptr = *mut c_void;
    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(name: *const c_char, flags: c_int) -> Ptr;
        fn dlsym(handle: Ptr, symbol: *const c_char) -> Ptr;
    }
    // WebKitGTK uses GStreamer. Load the installed runtime rather than an external command.
    static LIBRARY: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let library = *LIBRARY
        .get_or_init(|| unsafe { dlopen(c"libgstreamer-1.0.so.0".as_ptr(), 2) as usize })
        as Ptr;
    if library.is_null() {
        return Err("无法读取 GStreamer 音频运行库".into());
    }
    macro_rules! symbol {
        ($name:literal, $t:ty) => {{
            let address = unsafe { dlsym(library, concat!($name, "\0").as_ptr().cast()) };
            if address.is_null() {
                return Err("音频运行库缺少必要接口".into());
            }
            unsafe { std::mem::transmute::<Ptr, $t>(address) }
        }};
    }
    let init = symbol!(
        "gst_init_check",
        unsafe extern "C" fn(Ptr, Ptr, Ptr) -> c_int
    );
    let parse = symbol!(
        "gst_parse_launch",
        unsafe extern "C" fn(*const c_char, Ptr) -> Ptr
    );
    let state = symbol!(
        "gst_element_set_state",
        unsafe extern "C" fn(Ptr, c_int) -> c_int
    );
    let bus = symbol!("gst_element_get_bus", unsafe extern "C" fn(Ptr) -> Ptr);
    let wait = symbol!(
        "gst_bus_timed_pop_filtered",
        unsafe extern "C" fn(Ptr, u64, c_int) -> Ptr
    );
    let unref = symbol!("gst_object_unref", unsafe extern "C" fn(Ptr));
    let message_unref = symbol!("gst_mini_object_unref", unsafe extern "C" fn(Ptr));
    if unsafe {
        init(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err("无法初始化音频输出".into());
    }
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    let uri: String = file
        .path()
        .to_string_lossy()
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let pipeline = CString::new(format!("playbin uri=file://{uri}")).map_err(|e| e.to_string())?;
    let element = unsafe { parse(pipeline.as_ptr(), std::ptr::null_mut()) };
    if element.is_null() {
        return Err("无法建立音频播放管线".into());
    }
    let bus = unsafe { bus(element) };
    let started = unsafe { state(element, 4) } != 0; // GST_STATE_PLAYING
    let message = if started && !bus.is_null() {
        unsafe { wait(bus, 2_000_000_000, 1) }
    } else {
        std::ptr::null_mut()
    }; // EOS
    unsafe {
        state(element, 1); // GST_STATE_NULL
        if !message.is_null() {
            message_unref(message);
        }
        if !bus.is_null() {
            unref(bus);
        }
        unref(element);
    }
    if message.is_null() {
        Err("无法播放提示音，请检查系统音频输出及 GStreamer 音频插件".into())
    } else {
        Ok(())
    }
}
#[cfg(not(any(windows, target_os = "linux")))]
fn play(_bytes: &'static [u8]) -> Result<(), String> {
    Err("此宿主使用原生 macOS 应用播放提示音".into())
}
