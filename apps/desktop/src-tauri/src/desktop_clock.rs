//! Native user hour-cycle overrides, independent of the interface language.
pub fn hour_cycle() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    {
        use windows::{
            Win32::Globalization::{GetLocaleInfoEx, LOCALE_ITIME},
            core::PCWSTR,
        };
        let mut value = [0u16; 16];
        let count = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_ITIME, Some(&mut value)) };
        if count > 1 {
            Some(if value[0] == b'0' as u16 {
                "h12"
            } else {
                "h23"
            })
        } else {
            None
        }
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::gio::{self, prelude::*};
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();
        if desktop
            .split(':')
            .any(|name| matches!(name, "gnome" | "unity" | "cinnamon" | "budgie"))
            && let Some(schema) = gio::SettingsSchemaSource::default()
                .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
            && schema.has_key("clock-format")
        {
            let settings = gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None);
            return match settings.string("clock-format").as_str() {
                "12h" => Some("h12"),
                "24h" => Some("h23"),
                _ => None,
            };
        }
        // Plasma's regional time format is LC_TIME. A GNOME schema can be installed
        // on KDE/Xfce too; its default must not override that desktop's locale.
        locale_hour_cycle()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    None
}

#[cfg(target_os = "linux")]
fn locale_hour_cycle() -> Option<&'static str> {
    use std::ffi::{CStr, CString};
    let locale = CString::new("").ok()?;
    unsafe {
        // A per-call locale sees environment overrides without changing process-wide
        // GTK/Rust locale state or borrowing nl_langinfo's shared mutable buffer.
        let locale = libc::newlocale(libc::LC_TIME_MASK, locale.as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return None;
        }
        let pattern = CStr::from_ptr(libc::nl_langinfo_l(libc::T_FMT, locale))
            .to_string_lossy()
            .into_owned();
        libc::freelocale(locale);
        pattern_hour_cycle(&pattern)
    }
}

#[cfg(any(target_os = "linux", test))]
fn pattern_hour_cycle(pattern: &str) -> Option<&'static str> {
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            continue;
        }
        let mut code = chars.next()?;
        while matches!(code, 'E' | 'O' | '-' | '_' | '0' | '^' | '#') {
            code = chars.next()?;
        }
        match code {
            'I' | 'l' | 'r' => return Some("h12"),
            'H' | 'k' | 'R' | 'T' => return Some("h23"),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::pattern_hour_cycle;
    #[test]
    fn native_time_patterns_keep_the_hour_cycle() {
        for pattern in ["%I:%M:%S %p", "%r", "%OI时%M分", "%-l:%M %p"] {
            assert_eq!(pattern_hour_cycle(pattern), Some("h12"));
        }
        for pattern in ["%H:%M:%S", "%T", "%R", "%k.%M"] {
            assert_eq!(pattern_hour_cycle(pattern), Some("h23"));
        }
        assert_eq!(pattern_hour_cycle("%%I"), None);
        assert_eq!(pattern_hour_cycle("%X"), None);
    }
}
