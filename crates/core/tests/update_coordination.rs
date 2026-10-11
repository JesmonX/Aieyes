use aieyes_core::{operations, process};
use std::{
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

// This integration binary owns its registry; no global update state leaks into
// unrelated parallel core tests.
#[test]
fn update_cancels_queries_preserves_safe_boundaries_and_allows_login_completion() {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _scope = operations::enter("quotas.refresh").unwrap();
        let critical = operations::critical().unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        assert!(
            operations::check().is_ok(),
            "credential journal must finish"
        );
        drop(critical);
        assert!(operations::check().is_err());
        assert!(
            operations::critical().is_err(),
            "cancelled work cannot start another write"
        );
    });
    ready_rx.recv().unwrap();
    assert_eq!(
        operations::prepare()["protected"],
        serde_json::json!(["quotas.refresh"])
    );
    assert!(operations::enter("settings.save").is_err());
    assert!(operations::enter("sources.scan").is_err());
    for method in [
        "codexAuth.login.status",
        "codexAuth.login.cancel",
        "agyAuth.submitCode",
        "agyAuth.confirmWorkspace",
        "codexAuth.switch.commit",
    ] {
        let _completion = operations::enter(method).unwrap();
        assert!(operations::check().is_ok());
    }
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    {
        let _read = operations::enter("settings.get").unwrap();
        assert_eq!(operations::status()["protected"], serde_json::json!([]));
    }
    operations::resume();
    assert!(operations::enter("settings.save").is_ok());

    let (tx, rx) = mpsc::channel();
    let query = std::thread::spawn(move || {
        let _scope = operations::enter("hosts.sample").unwrap();
        tx.send(()).unwrap();
        let mut cmd = Command::new(if cfg!(windows) { "python" } else { "python3" });
        cmd.args(["-c", "import time; time.sleep(30)"]);
        process::run(cmd, vec![], Duration::from_secs(40)).unwrap_err()
    });
    rx.recv().unwrap();
    let start = Instant::now();
    operations::prepare();
    assert!(query.join().unwrap().to_string().contains("更新取消"));
    assert!(start.elapsed() < Duration::from_secs(5));
    operations::resume();
}
