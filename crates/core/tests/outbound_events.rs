//! Launch state, server history and outbound notification settings through the engine API.
use aieyes_core::Engine;
use serde_json::json;

#[test]
fn launch_reports_a_version_change_once() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let first = engine.call("app.launch", json!({})).unwrap();
    assert_eq!(first["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(first["updated"], false);
    assert!(first["previousVersion"].is_null());
    assert_eq!(
        engine.call("app.launch", json!({})).unwrap()["updated"],
        false
    );
    engine
        .store
        .db
        .execute(
            "UPDATE kv SET value='0.0.1' WHERE key='last-run-version'",
            [],
        )
        .unwrap();
    let updated = engine.call("app.launch", json!({})).unwrap();
    assert_eq!(updated["updated"], true);
    assert_eq!(updated["previousVersion"], "0.0.1");
    assert_eq!(
        engine.call("app.launch", json!({})).unwrap()["updated"],
        false
    );
}

#[test]
fn local_samples_feed_the_minute_history_and_leave_with_the_host() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    assert_eq!(
        engine.call("hosts.history", json!({})).unwrap(),
        json!({"hosts":[]})
    );
    let mut settings = engine.store.settings().unwrap();
    settings.local_monitor.metrics = vec!["cpu".into(), "memory".into()];
    engine.store.save_settings(&settings).unwrap();
    engine
        .call("hosts.sample", json!({"hostId":"local"}))
        .unwrap();
    let history = engine
        .call("hosts.history", json!({"hostId":"local"}))
        .unwrap();
    assert_eq!(history["hosts"][0]["id"], "local");
    let minute = &history["hosts"][0]["minutes"][0];
    assert!(minute["t"].as_i64().unwrap() % 60 == 0);
    let memory = minute["memory"].as_f64().unwrap();
    assert!((0.0..=100.0).contains(&memory));
    assert!(
        engine
            .call("hosts.history", json!({"hostId":"other"}))
            .unwrap()["hosts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // A host that leaves monitoring takes its history with it.
    settings.local_monitor.enabled = false;
    engine.store.save_settings(&settings).unwrap();
    assert_eq!(
        engine.call("hosts.history", json!({})).unwrap(),
        json!({"hosts":[]})
    );
}

#[test]
fn alert_queue_requires_the_notification_session_start() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    assert!(engine.call("hostAlerts.take", json!({})).is_err());
    assert_eq!(
        engine.call("hostAlerts.take", json!({"since":0})).unwrap(),
        json!([])
    );
}

#[test]
fn settings_keep_webhook_and_alert_rules_valid() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let base = engine.call("settings.get", json!({})).unwrap();
    assert_eq!(base["webhooks"]["enabled"], false);
    assert_eq!(base["serverAlerts"]["enabled"], false);

    let mut next = base.clone();
    next["webhooks"] = json!({
        "enabled": true,
        "targets": [{"id":"a","name":"Ops","url":"https://example.com/hook","format":"slack","enabled":true}],
        "sessionComplete": false
    });
    next["serverAlerts"] =
        json!({"enabled":true,"cpuPercent":85,"memoryPercent":92,"durationSeconds":600});
    let saved = engine
        .call("settings.patch", json!({"base":base,"settings":next}))
        .unwrap();
    assert_eq!(saved["webhooks"]["targets"][0]["format"], "slack");
    assert_eq!(saved["webhooks"]["sessionComplete"], false);
    assert_eq!(saved["webhooks"]["sessionNeedsInput"], true);
    assert_eq!(saved["serverAlerts"]["cpuPercent"], 85.0);
    assert_eq!(saved["serverAlerts"]["durationSeconds"], 600);

    for (field, value) in [
        (
            "webhooks",
            json!({"enabled":true,"targets":[{"id":"a","url":"ftp://example.com","format":"json"}]}),
        ),
        (
            "webhooks",
            json!({"enabled":true,"targets":[{"id":"a","url":"https://example.com","format":"carrier-pigeon"}]}),
        ),
        ("serverAlerts", json!({"enabled":true,"cpuPercent":0})),
        ("serverAlerts", json!({"enabled":true,"durationSeconds":5})),
    ] {
        let mut invalid = saved.clone();
        invalid[field] = value.clone();
        assert!(
            engine
                .call("settings.patch", json!({"base":saved,"settings":invalid}))
                .is_err(),
            "{field}: {value}"
        );
    }
    let current = engine.call("settings.get", json!({})).unwrap();
    assert_eq!(current["webhooks"], saved["webhooks"]);
    assert_eq!(current["serverAlerts"], saved["serverAlerts"]);
}

#[test]
fn application_rejection_is_not_reported_as_delivered() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0; 8192];
        let _ = s.read(&mut buf);
        let body = r#"{"errcode":93000,"errmsg":"invalid webhook url"}"#;
        write!(s,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let mut e = Engine::open(root.path()).unwrap();
    let mut settings = e.store.settings().unwrap();
    settings.proxy.mode = "direct".into();
    e.store.save_settings(&settings).unwrap();
    let result = e
        .call(
            "webhooks.test",
            json!({"target":{"id":"test","url":url,"format":"wechat","enabled":true}}),
        )
        .unwrap_err();
    assert!(result.to_string().contains("93000"));
    thread.join().unwrap();
}
