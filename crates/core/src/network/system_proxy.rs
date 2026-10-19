//! Observe SystemConfiguration directly; no `scutil` process in request paths.
use crate::models::ProxyConfig;
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
    sync::{
        Mutex, Once, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFStringCreateWithCString(allocator: Ref, value: *const c_char, encoding: u32) -> Ref;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    static kCFTypeArrayCallBacks: [usize; 5];
    static kCFRunLoopDefaultMode: Ref;
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopAddSource(run_loop: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRun();
}
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyProxies(store: Ref) -> Ref;
    fn SCDynamicStoreCreate(
        allocator: Ref,
        name: Ref,
        callback: Option<unsafe extern "C" fn(Ref, Ref, *mut c_void)>,
        context: Ref,
    ) -> Ref;
    fn SCDynamicStoreSetNotificationKeys(store: Ref, keys: Ref, patterns: Ref) -> u8;
    fn SCDynamicStoreCreateRunLoopSource(allocator: Ref, store: Ref, order: isize) -> Ref;
}
struct Owned(Ref);
impl Owned {
    fn new(value: Ref) -> Option<Self> {
        (!value.is_null()).then(|| Self(value))
    }
    fn string(value: &str) -> Option<Self> {
        let value = CString::new(value).ok()?;
        Self::new(unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), 0x0800_0100) })
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
fn field(dictionary: Ref, key: &str) -> Option<Ref> {
    let key = Owned::string(key)?;
    let value = unsafe { CFDictionaryGetValue(dictionary, key.0) };
    (!value.is_null()).then_some(value)
}
fn number(value: Ref) -> Option<i64> {
    let mut result = 0i64;
    unsafe {
        (CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(value, 4, (&mut result as *mut i64).cast()) != 0)
            .then_some(result)
    }
}
fn string(value: Ref) -> Option<String> {
    let mut buffer = [0u8; 4096];
    unsafe {
        if CFGetTypeID(value) != CFStringGetTypeID()
            || CFStringGetCString(
                value,
                buffer.as_mut_ptr().cast(),
                buffer.len() as isize,
                0x0800_0100,
            ) == 0
        {
            return None;
        }
    }
    Some(
        std::ffi::CStr::from_bytes_until_nul(&buffer)
            .ok()?
            .to_string_lossy()
            .into_owned(),
    )
}
fn read() -> Option<ProxyConfig> {
    let dictionary = Owned::new(unsafe { SCDynamicStoreCopyProxies(ptr::null()) })?;
    for (name, scheme) in [("HTTPS", "http"), ("HTTP", "http"), ("SOCKS", "socks5h")] {
        if field(dictionary.0, &format!("{name}Enable")).and_then(number) == Some(1)
            && let (Some(host), Some(port)) = (
                field(dictionary.0, &format!("{name}Proxy")).and_then(string),
                field(dictionary.0, &format!("{name}Port")).and_then(number),
            )
        {
            let host = if host.contains(':') && !host.starts_with('[') {
                format!("[{host}]")
            } else {
                host
            };
            return Some(ProxyConfig {
                mode: "custom".into(),
                url: format!("{scheme}://{host}:{port}"),
            });
        }
    }
    Some(ProxyConfig {
        mode: "system".into(),
        url: String::new(),
    })
}
static REVISION: AtomicU64 = AtomicU64::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();
unsafe extern "C" fn changed(_: Ref, _: Ref, _: *mut c_void) {
    REVISION.fetch_add(1, Ordering::AcqRel);
}
fn observe() -> Option<()> {
    let name = Owned::string("Aieyes network")?;
    let store = Owned::new(unsafe {
        SCDynamicStoreCreate(ptr::null(), name.0, Some(changed), ptr::null())
    })?;
    let patterns = [
        Owned::string("State:/Network/Global/Proxies")?,
        Owned::string("Setup:/Network/.*")?,
    ];
    let values: Vec<_> = patterns.iter().map(|value| value.0).collect();
    let array = Owned::new(unsafe {
        CFArrayCreate(
            ptr::null(),
            values.as_ptr(),
            values.len() as isize,
            (&raw const kCFTypeArrayCallBacks).cast(),
        )
    })?;
    if unsafe { SCDynamicStoreSetNotificationKeys(store.0, ptr::null(), array.0) } == 0 {
        return None;
    }
    let source = Owned::new(unsafe { SCDynamicStoreCreateRunLoopSource(ptr::null(), store.0, 0) })?;
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source.0, kCFRunLoopDefaultMode);
    }
    ACTIVE.store(true, Ordering::Release);
    REVISION.fetch_add(1, Ordering::AcqRel);
    unsafe {
        CFRunLoopRun();
    }
    ACTIVE.store(false, Ordering::Release);
    None
}
struct Cached {
    revision: u64,
    checked: Instant,
    value: ProxyConfig,
    valid: bool,
}
pub(super) fn get(force: bool) -> ProxyConfig {
    START.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("aieyes-proxy-changes".into())
            .spawn(|| {
                observe();
            });
    });
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let revision = REVISION.load(Ordering::Acquire);
    if let Some(cached) = cache.as_ref().filter(|c| {
        !force
            && c.revision == revision
            && (c.valid && ACTIVE.load(Ordering::Acquire)
                || c.checked.elapsed() < Duration::from_secs(5))
    }) {
        return cached.value.clone();
    }
    let value = read();
    let valid = value.is_some();
    let value = value.unwrap_or_else(|| ProxyConfig {
        mode: "system".into(),
        url: String::new(),
    });
    *cache = Some(Cached {
        revision,
        checked: Instant::now(),
        value: value.clone(),
        valid,
    });
    value
}
