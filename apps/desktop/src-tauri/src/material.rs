//! One desktop backdrop per HWND. Report success only after the native call succeeds.
use std::{ffi::c_void, mem::size_of, sync::OnceLock};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Dwm::*,
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Registry::*,
        },
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, s, w},
};

#[repr(C)]
struct Version {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

pub(crate) fn build_number() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| unsafe {
        let Ok(module) = GetModuleHandleW(w!("ntdll.dll")) else {
            return 0;
        };
        let Some(address) = GetProcAddress(module, s!("RtlGetVersion")) else {
            return 0;
        };
        let version_fn: unsafe extern "system" fn(*mut Version) -> i32 =
            std::mem::transmute(address);
        let mut version = Version {
            size: size_of::<Version>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            service_pack: [0; 128],
        };
        if version_fn(&mut version) >= 0 && version.major >= 10 {
            version.build
        } else {
            0
        }
    })
}

pub fn effects_enabled() -> bool {
    unsafe {
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
            || contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
        {
            return false;
        }
        let mut enabled = 1u32;
        let mut size = size_of::<u32>() as u32;
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut enabled as *mut u32).cast()),
            Some(&mut size),
        );
        enabled != 0
    }
}

/// The taskbar has its own light/dark setting; a missing value means Windows 10's default dark taskbar.
pub fn taskbar_dark() -> bool {
    let (mut light, mut size) = (0u32, size_of::<u32>() as u32);
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut light as *mut u32).cast()),
            Some(&mut size),
        )
    } == windows::Win32::Foundation::ERROR_SUCCESS;
    !found || light == 0
}
/// Long-lived main windows use Mica (cheaper, Windows 11 22H2+); the transient panel keeps Acrylic.
fn preferred(build: u32, effects: bool, main: bool, requested: &str) -> &'static str {
    if !effects || requested == "opaque" {
        "opaque"
    } else if build >= 22621 {
        match requested {
            "mica" => "mica",
            "acrylic" => "acrylic",
            _ if main => "mica",
            _ => "acrylic",
        }
    } else if build >= 17763 {
        "blur"
    } else {
        "opaque"
    }
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    color: u32,
    animation: u32,
}
#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

// The compatibility path is intentionally restricted to 1809..Windows 11 21H2.
// Legacy Acrylic stalls interactive resizing there; modern systems use DWM instead.
unsafe fn legacy_blur(hwnd: HWND, enabled: bool, dark: bool) -> bool {
    unsafe {
        let Ok(module) = GetModuleHandleW(w!("user32.dll")) else {
            return false;
        };
        let Some(address) = GetProcAddress(module, s!("SetWindowCompositionAttribute")) else {
            return false;
        };
        let set: unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL =
            std::mem::transmute(address);
        // ABGR tint supplies a readable base on old DWM, which has no luminosity layer.
        let mut policy = AccentPolicy {
            state: if enabled { 3 } else { 0 },
            flags: 2,
            color: if dark { 0xb3242020 } else { 0xb3fafafa },
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: 19,
            data: (&mut policy as *mut AccentPolicy).cast(),
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data).as_bool()
    }
}

pub fn apply(hwnd: HWND, dark: bool, effects: bool, main: bool, requested: &str) -> &'static str {
    let build = build_number();
    let preferred = preferred(build, effects, main, requested);
    unsafe {
        let dark = i32::from(dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let result = if build >= 22621 {
            let backdrop = match preferred {
                "mica" => DWMSBT_MAINWINDOW,
                "acrylic" => DWMSBT_TRANSIENTWINDOW,
                _ => DWMSBT_NONE,
            };
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&backdrop as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
                size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            )
            .is_ok()
        } else if build >= 17763 {
            legacy_blur(hwnd, preferred == "blur", dark != 0)
        } else {
            false
        };
        reshape(hwnd);
        if result { preferred } else { "opaque" }
    }
}

/// Windows 11 rounds through DWM: 8 logical pixels, matching the web pane, square when maximized.
pub fn rounded_corners() -> bool {
    build_number() >= 22000
}

/// Windows 10 windows stay square, as native ones are there: a window region would also drop the DWM
/// shadow, and GetWindowRect includes the invisible resize border, which hid the rounding anyway.
pub fn reshape(hwnd: HWND) {
    if !rounded_corners() {
        return;
    }
    unsafe {
        let corner = if IsZoomed(hwnd).as_bool() {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::preferred;
    #[test]
    fn material_tracks_os_support_and_accessibility() {
        assert_eq!(preferred(22621, true, true, "default"), "mica");
        assert_eq!(preferred(22621, true, false, "default"), "acrylic");
        assert_eq!(preferred(22000, true, true, "default"), "blur");
        assert_eq!(preferred(19045, true, false, "default"), "blur");
        assert_eq!(preferred(17763, true, true, "default"), "blur");
        assert_eq!(preferred(17134, true, true, "default"), "opaque");
        assert_eq!(preferred(26100, false, true, "default"), "opaque");
        assert_eq!(preferred(26100, true, true, "acrylic"), "acrylic");
        assert_eq!(preferred(26100, true, false, "mica"), "mica");
        assert_eq!(preferred(26100, true, true, "opaque"), "opaque");
        assert_eq!(preferred(19045, true, true, "mica"), "blur");
    }
}
