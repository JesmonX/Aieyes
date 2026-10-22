use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn language() -> &'static str {
    if ENGLISH.load(Ordering::Relaxed) {
        "en"
    } else {
        "zh-CN"
    }
}
pub fn set_language(value: &str) -> bool {
    ENGLISH.swap(value == "en", Ordering::Relaxed) != (value == "en")
}
pub fn label(source: &'static str) -> &'static str {
    aieyes_core::localization::label(language(), source)
}
pub fn text(source: &str) -> String {
    aieyes_core::localization::translate(language(), source).into_owned()
}
