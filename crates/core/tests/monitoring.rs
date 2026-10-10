use aieyes_core::{Engine, metrics, models::*, monitoring};
use serde_json::json;

#[test]
fn legacy_intervals_and_local_monitor_are_independent() {
    let mut settings: Settings =
        serde_json::from_value(json!({"version":4,"refreshSeconds":71,"serverRefreshSeconds":19}))
            .unwrap();
    settings.migrate();
    assert_eq!(settings.history_refresh_seconds, 71);
    assert_eq!(settings.server_foreground_refresh_seconds, 2);
    assert_eq!(settings.server_refresh_seconds, 19);
    settings.history_refresh_seconds = 91;
    settings.migrate();
    assert_eq!(settings.history_refresh_seconds, 91);
    assert_eq!(
        settings
            .monitored_hosts()
            .iter()
            .filter(|h| h.id == "local")
            .count(),
        1
    );
    assert!(settings.hosts.is_empty());
}

#[test]
fn filesystem_defaults_are_recommended_but_explicit_selection_wins() {
    let sample = json!({"cpu":[{"id":"cpu"},{"id":"cpu0"}],"filesystems":[{"id":"/","type":"ext4"},{"id":"/run/user/1000","type":"tmpfs"},{"id":"/boot/efi","type":"vfat"},{"id":"/snap/pkg/12","type":"squashfs"},{"id":"/tmp","type":"tmpfs"},{"id":"/data","type":"xfs"}]});
    let mut host = Host::default();
    let mut filtered = sample.clone();
    monitoring::filter_devices(&mut filtered, &host);
    assert_eq!(filtered["filesystems"].as_array().unwrap().len(), 2);
    host.devices = vec!["filesystems:__all__".into()];
    filtered = sample.clone();
    monitoring::filter_devices(&mut filtered, &host);
    assert_eq!(filtered["filesystems"], sample["filesystems"]);
    host.devices = vec!["filesystems:/boot/efi".into()];
    filtered = sample.clone();
    monitoring::filter_devices(&mut filtered, &host);
    assert_eq!(filtered["filesystems"].as_array().unwrap().len(), 1);
    host.devices = vec!["filesystems:__none__".into(), "cpu:__none__".into()];
    filtered = sample.clone();
    monitoring::filter_devices(&mut filtered, &host);
    assert!(filtered["filesystems"].as_array().unwrap().is_empty());
    assert_eq!(filtered["cpu"], json!([{"id":"cpu"}]));
}

#[test]
fn rates_preserve_available_counters_without_inventing_missing_readings() {
    let old = json!({"bootId":"a","monotonic":10,"disk":[{"id":"disk0","readBytes":100,"writeBytes":50}],"cpu":[{"id":"cpu","total":100}]});
    let next = json!({"bootId":"a","monotonic":12,"disk":[{"id":"disk0","readBytes":300,"writeBytes":5}],"cpu":[{"id":"cpu","total":200}]});
    let result = metrics::rates(&next, Some(&old));
    assert_eq!(result["disk"][0]["readBytesPerSecond"], 100.0);
    assert!(result["disk"][0]["writeBytesPerSecond"].is_null());
    assert!(result["disk"][0]["readIops"].is_null());
    assert!(result["cpu"][0]["utilization"].is_null());
}

#[test]
fn local_sampling_uses_native_metrics_and_can_be_disabled() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let mut settings = engine.store.settings().unwrap();
    settings.local_monitor.metrics = vec![
        "cpu".into(),
        "memory".into(),
        "filesystems".into(),
        "network".into(),
        "disk".into(),
    ];
    engine.store.save_settings(&settings).unwrap();
    let result = engine
        .call("hosts.sample", json!({"hostId":"local"}))
        .unwrap();
    assert_eq!(result[0]["id"], "local");
    assert!(result[0]["sample"]["memory"]["total"].as_u64().unwrap() > 0);
    assert!(result[0]["sample"]["cpu"][0]["utilization"].is_number());
    assert!(result[0]["sample"]["uptime"].is_number());
    settings.local_monitor.enabled = false;
    engine.store.save_settings(&settings).unwrap();
    assert!(
        engine
            .call("hosts.sample", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
