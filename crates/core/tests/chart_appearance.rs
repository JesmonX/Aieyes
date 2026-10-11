use aieyes_core::{models::Settings, store::Store};
use serde_json::json;

#[test]
fn chart_preferences_default_roundtrip_and_reject_invalid_values() {
    let mut settings: Settings = serde_json::from_value(json!({
        "appearance": {"theme": "dark", "accent": "teal"}
    }))
    .unwrap();
    assert_eq!(settings.appearance.chart_palette, "classic");
    assert_eq!(settings.appearance.chart_contrast, "high");
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    for palette in ["classic", "tableau", "tol", "set2", "nord"] {
        for contrast in ["high", "low"] {
            settings.appearance.chart_palette = palette.into();
            settings.appearance.chart_contrast = contrast.into();
            store.save_settings(&settings).unwrap();
            let saved = store.settings().unwrap();
            assert_eq!(saved.appearance.chart_palette, palette);
            assert_eq!(saved.appearance.chart_contrast, contrast);
            assert_eq!(saved.appearance.theme, "dark");
            assert_eq!(saved.appearance.accent, "teal");
        }
    }
    settings.appearance.chart_palette = "invalid".into();
    assert!(store.save_settings(&settings).is_err());
    assert_eq!(store.settings().unwrap().appearance.chart_palette, "nord");
    settings.appearance.chart_palette = "nord".into();
    settings.appearance.chart_contrast = "invalid".into();
    assert!(store.save_settings(&settings).is_err());
    assert_eq!(store.settings().unwrap().appearance.chart_contrast, "low");
}
