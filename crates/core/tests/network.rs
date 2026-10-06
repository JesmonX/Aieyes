use aieyes_core::{models::ProxyConfig, network};
use std::{
    io::{Read, Write},
    net::TcpListener,
};

#[test]
fn parallel_probe_reports_http_failures_separately_from_successful_latency() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for stream in listener.incoming().take(3) {
            let mut stream = stream.unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let n = stream.read(&mut request).unwrap();
            let ok = String::from_utf8_lossy(&request[..n]).starts_with("GET /ok ");
            write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                if ok { "200 OK" } else { "503 Unavailable" }
            )
            .unwrap();
        }
    });
    let proxy = ProxyConfig {
        mode: "direct".into(),
        url: String::new(),
    };
    let result = network::test(
        &proxy,
        &[
            format!("http://{address}/ok"),
            format!("http://{address}/bad"),
        ],
    )
    .unwrap();
    assert_eq!(result.status, "unstable");
    assert_eq!(result.average_ms, result.sites[0].latency_ms);
    assert_eq!(result.sites[1].error.as_deref(), Some("HTTP 503"));
    let failure = network::test(&proxy, &[format!("http://{address}/bad")]).unwrap();
    assert_eq!(failure.status, "failed");
    assert!(failure.average_ms.is_none());
    server.join().unwrap();
    assert!(network::validate_test_urls(&[]).is_err());
    assert!(network::validate_test_urls(&["file:///private".into()]).is_err());
}

#[test]
fn app_probe_discards_the_result_if_settings_change_during_the_request() {
    use aieyes_core::{Engine, models::Settings, store::Store};
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let read = stream.read(&mut request).unwrap();
        assert!(
            read > 0,
            "the probe must send a request before settings change"
        );
        ready_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let mut engine = Engine::open(directory.path()).unwrap();
    let mut settings = Settings {
        proxy: ProxyConfig {
            mode: "direct".into(),
            url: String::new(),
        },
        proxy_test_urls: vec![format!("http://{address}/old")],
        ..Default::default()
    };
    engine.store.save_settings(&settings).unwrap();
    let probe = std::thread::spawn(move || engine.call("network.test", json!({"force":true})));
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    settings.proxy_test_urls = vec![format!("http://{address}/new")];
    Store::open(directory.path())
        .unwrap()
        .save_settings(&settings)
        .unwrap();
    release_tx.send(()).unwrap();
    assert!(
        probe
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("连接设置已变化")
    );
    server.join().unwrap();
}
