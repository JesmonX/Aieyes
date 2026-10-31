use super::*;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
};

fn fixture(count: usize, shared_home: bool) -> (tempfile::TempDir, Engine) {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::open(root.path()).unwrap();
    std::fs::write(root.path().join("quota.py"), r#"import json, os, sys, time
root, account, delay = sys.argv[1:]
def record(kind):
    lock=os.open(os.path.join(root,'queries.lock'),os.O_CREAT|os.O_RDWR,0o600)
    if os.name == 'nt':
        import msvcrt
        msvcrt.locking(lock,msvcrt.LK_LOCK,1)
    else:
        import fcntl
        fcntl.flock(lock,fcntl.LOCK_EX)
    line = json.dumps({'kind':kind,'account':account,'time':time.monotonic()})+'\n'
    fd=os.open(os.path.join(root,'queries.jsonl'),os.O_CREAT|os.O_APPEND|os.O_WRONLY,0o600)
    os.write(fd,line.encode());os.close(fd)
    if os.name == 'nt':
        msvcrt.locking(lock,msvcrt.LK_UNLCK,1)
    os.close(lock)
record('start')
deadline = time.monotonic() + 15
while os.path.exists(os.path.join(root,'hold-queries')):
    if time.monotonic() > deadline:
        raise RuntimeError('fixture release timed out')
    time.sleep(0.01)
while os.path.exists(os.path.join(root,'wait-for-two')) and not os.path.exists(os.path.join(root,'two.ready')):
    time.sleep(0.01)
time.sleep(float(delay))
record('end')
print(json.dumps({'windows':[{'name':'5h','usedPercent':20,'windowMinutes':300}]}))
"#).unwrap();
    let quote = |s: &str| {
        if cfg!(windows) {
            format!("'{}'", s.replace('\'', "''"))
        } else {
            format!("'{}'", s.replace('\'', "'\"'\"'"))
        }
    };
    let mut settings = Settings {
        refresh_seconds: 5,
        ..Default::default()
    };
    for index in 0..count {
        let id = format!("a{index}");
        let home = root.path().join(if shared_home {
            "home".into()
        } else {
            id.clone()
        });
        std::fs::create_dir_all(&home).unwrap();
        settings.accounts.push(Account {
            id: id.clone(),
            name: id.clone(),
            provider: "custom".into(),
            ..Default::default()
        });
        settings.sources.push(Source {
            id: format!("s{index}"),
            name: format!("Source {index}"),
            account_id: id.clone(),
            provider: "custom".into(),
            path: home.display().to_string(),
            quota_command: format!(
                "{} {} {} {} {}",
                if cfg!(windows) { "python" } else { "python3" },
                quote(&root.path().join("quota.py").display().to_string()),
                quote(&root.path().display().to_string()),
                id,
                if index == 0 { "0.7" } else { "0.15" }
            ),
            ..Default::default()
        });
    }
    engine.store.save_settings(&settings).unwrap();
    (root, engine)
}
fn worker(root: &Path, name: &str, params: Value, fresh: bool) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "quota_queries::tests::process_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("AIEYES_QUOTA_TEST_ROOT", root)
        .env("AIEYES_QUOTA_TEST_NAME", name)
        .env("AIEYES_QUOTA_TEST_PARAMS", params.to_string())
        .env("AIEYES_QUOTA_TEST_FRESH", if fresh { "1" } else { "0" })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
fn finish(child: Child) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn records(root: &Path, file: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join(file))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn wait_started(root: &Path, count: usize) {
    let start = Instant::now();
    while records(root, "queries.jsonl")
        .iter()
        .filter(|r| r["kind"] == "start")
        .count()
        < count
    {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "fixture failed to start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn scheduled_queries_do_not_decode_completed_sampling_history() {
    let (_root, mut engine) = fixture(1, false);
    // Valid JSON with an invalid typed history field detects accidental full
    // history reads without making this a timing-sensitive performance test.
    engine.store.db.execute(
        "INSERT INTO estimates VALUES('quota','historical','custom:a0','completed',?1)",
        [json!({"id":"historical","accountKey":"custom:a0","status":"completed","prices":"history-only"}).to_string()],
    ).unwrap();
    assert!(engine.store.estimates().is_err());
    let rows = engine
        .call("quotas.refresh", json!({"accountKey":"custom:a0"}))
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert!(rows[0]["error"].is_null(), "{rows}");
    assert_eq!(rows[0]["windows"][0]["usedPercent"], 20.0);
}

#[test]
#[ignore = "subprocess fixture, invoked by quota coordination tests"]
fn process_worker() {
    let root = PathBuf::from(std::env::var_os("AIEYES_QUOTA_TEST_ROOT").unwrap());
    let name = std::env::var("AIEYES_QUOTA_TEST_NAME").unwrap();
    let params = serde_json::from_str(&std::env::var("AIEYES_QUOTA_TEST_PARAMS").unwrap()).unwrap();
    let mut engine = Engine::open(&root).unwrap();
    std::fs::write(root.join(format!("{name}.ready")), "").unwrap();
    let results =
        if std::env::var("AIEYES_QUOTA_TEST_FRESH").unwrap() == "1" {
            serde_json::to_value(engine.read_quotas(&params).unwrap()).unwrap()
        } else {
            let progress_root = root.clone();
            engine
                .call_with_progress(
                    "quotas.refresh",
                    params,
                    Box::new(move |row| {
                        if row["kind"] == "quotas.refresh" {
                            if row["row"]["error"].is_null() {
                                let persisted = Store::open(&progress_root)
                                    .unwrap()
                                    .dashboard(&Filter::default())
                                    .unwrap()
                                    .quotas;
                                assert!(
                                    persisted.iter().any(|q| q.account_id
                                        == row["row"]["accountId"].as_str().unwrap())
                                );
                            }
                            let mut file = OpenOptions::new()
                                .create(true)
                                .read(true)
                                .append(true)
                                .open(progress_root.join("progress.jsonl"))
                                .unwrap();
                            // Formatting JSON may write several chunks; serialize fixture
                            // appends so concurrent processes cannot corrupt a record.
                            file.lock().unwrap();
                            writeln!(file, "{}", row["row"]).unwrap();
                            file.unlock().unwrap();
                        }
                    }),
                )
                .unwrap()
        };
    std::fs::write(root.join(format!("{name}.json")), results.to_string()).unwrap();
}
#[test]
fn processes_share_four_slots_publish_fast_results_and_cool_down_after_completion() {
    let (root, mut engine) = fixture(7, false);
    let mut settings = engine.store.settings().unwrap();
    settings.refresh_seconds = 60;
    engine.store.save_settings(&settings).unwrap();
    // Process startup can exceed the fast query's 150ms on Windows CI. Hold the
    // first wave until all four slots are observable, then measure completion.
    let gate = root.path().join("hold-queries");
    std::fs::write(&gate, "").unwrap();
    let first = worker(root.path(), "one", json!({"dueOnly":true}), false);
    let second = worker(root.path(), "two", json!({"dueOnly":true}), false);
    wait_started(root.path(), 4);
    std::fs::remove_file(gate).unwrap();
    finish(first);
    finish(second);
    let mut active = 0;
    let mut max_active = 0;
    let mut starts = std::collections::BTreeMap::new();
    for row in records(root.path(), "queries.jsonl") {
        if row["kind"] == "start" {
            active += 1;
            *starts
                .entry(row["account"].as_str().unwrap().to_owned())
                .or_insert(0) += 1;
        } else {
            active -= 1;
        }
        max_active = max_active.max(active);
        assert!((0..=4).contains(&active), "global slot limit: {row}");
    }
    assert_eq!(active, 0);
    assert_eq!(max_active, 4);
    assert_eq!(starts.len(), 7);
    assert!(starts.values().all(|n| *n == 1));
    let published = records(root.path(), "progress.jsonl");
    assert_eq!(published.len(), 7);
    assert_ne!(
        published[0]["accountId"], "a0",
        "slow account must not hold faster results"
    );
    let schedule = engine.call("quotas.schedule", json!({})).unwrap();
    for account in schedule["accounts"].as_array().unwrap() {
        let key = account["accountKey"].as_str().unwrap();
        assert_eq!(
            account["nextDueAt"],
            finished(&engine.store, key).unwrap().unwrap() + 60
        );
        assert_eq!(account["inFlight"], false);
    }
    assert_eq!(
        engine
            .call("quotas.refresh", json!({"dueOnly":true}))
            .unwrap(),
        json!([])
    );
}
#[test]
fn shared_directory_serializes_distinct_accounts() {
    let (root, _) = fixture(2, true);
    let first = worker(root.path(), "one", json!({"accountKey":"custom:a0"}), false);
    wait_started(root.path(), 1);
    let second = worker(root.path(), "two", json!({"accountKey":"custom:a1"}), false);
    finish(first);
    finish(second);
    let kinds: Vec<_> = records(root.path(), "queries.jsonl")
        .iter()
        .map(|v| v["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["start", "end", "start", "end"]);
}
#[test]
fn overlapping_manual_queries_coalesce_but_sampling_requires_a_new_read() {
    for fresh in [false, true] {
        let (root, _) = fixture(1, false);
        std::fs::write(root.path().join("wait-for-two"), "").unwrap();
        let first = worker(root.path(), "one", json!({"accountKey":"custom:a0"}), false);
        wait_started(root.path(), 1);
        let second = worker(root.path(), "two", json!({"accountKey":"custom:a0"}), fresh);
        finish(first);
        finish(second);
        assert_eq!(
            records(root.path(), "queries.jsonl")
                .iter()
                .filter(|v| v["kind"] == "start")
                .count(),
            if fresh { 2 } else { 1 }
        );
    }
}
#[test]
fn pausing_a_machine_discards_its_in_flight_result() {
    let (root, engine) = fixture(1, false);
    let child = worker(
        root.path(),
        "changed",
        json!({"accountKey":"custom:a0"}),
        false,
    );
    wait_started(root.path(), 1);
    let mut settings = engine.store.settings().unwrap();
    settings.local_device_enabled = false;
    engine.store.save_settings(&settings).unwrap();
    // The public batch response reports the discarded query without publishing usage.
    finish(child);
    let result: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("changed.json")).unwrap()).unwrap();
    assert!(result[0]["error"].as_str().unwrap().contains("配置已改变"));
    let stored: i64 = engine
        .store
        .db
        .query_row("SELECT COUNT(*) FROM quotas", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        stored, 0,
        "discarded results must not replace the retained quota snapshot"
    );
    assert!(finished(&engine.store, "custom:a0").unwrap().is_some());
}

#[test]
fn future_error_cache_is_replaced_once_by_a_successful_observation() {
    for origin in ["live", "command"] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut old = QuotaSnapshot {
            provider: "codex".into(),
            account_id: "clock".into(),
            origin: origin.into(),
            updated_at: now() + 18000,
            bank_reset: Some(json!({"availableCount":0})),
            error: Some("busy".into()),
            ..Default::default()
        };
        store
            .db
            .execute(
                "INSERT INTO quotas VALUES('codex:clock',?1)",
                [serde_json::to_string(&old).unwrap()],
            )
            .unwrap();
        old.updated_at = now();
        old.error = None;
        store.quota(&old).unwrap();
        let read = || -> QuotaSnapshot {
            serde_json::from_str(
                &store
                    .db
                    .query_row::<String, _, _>(
                        "SELECT payload FROM quotas WHERE account_key='codex:clock'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap(),
            )
            .unwrap()
        };
        let accepted = read();
        assert!(accepted.error.is_none());
        assert_eq!(accepted.observation_revision, 1);
        store.quota(&old).unwrap();
        assert_eq!(read().observation_revision, 1);
        old.updated_at -= 60;
        store.quota(&old).unwrap();
        assert_eq!(read().updated_at, accepted.updated_at);
        old.updated_at = now();
        old.error = Some("temporary".into());
        store.quota(&old).unwrap();
        assert_eq!(read().observation_revision, 2);
        store.quota(&old).unwrap();
        assert_eq!(read().observation_revision, 2);
        old.error = None;
        old.updated_at += 1;
        store.quota(&old).unwrap();
        assert!(read().error.is_none());
        assert_eq!(read().observation_revision, 3);
    }
}

#[test]
fn due_only_queries_keep_the_qoder_floor() {
    // Each Qoder check starts qodercli: a 30 s account interval still waits 10 minutes.
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    engine
        .store
        .save_settings(&Settings {
            refresh_seconds: 60,
            accounts: vec![Account {
                id: "q".into(),
                name: "Qoder".into(),
                provider: "qoder".into(),
                quota_enabled: true,
                quota_refresh_seconds: Some(30),
                ..Default::default()
            }],
            sources: vec![Source {
                id: "s".into(),
                name: "Qoder".into(),
                provider: "qoder".into(),
                account_id: "q".into(),
                path: root.path().display().to_string(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let last = now() - 300;
    engine
        .store
        .db
        .execute(
            "INSERT INTO kv VALUES('quota-attempt:qoder:q',?1)",
            [last.to_string()],
        )
        .unwrap();
    let rows = engine
        .call("quotas.refresh", json!({"dueOnly":true}))
        .unwrap();
    assert_eq!(rows, json!([]));
    assert_eq!(finished(&engine.store, "qoder:q").unwrap(), Some(last));
    let schedule = engine.call("quotas.schedule", json!({})).unwrap();
    assert_eq!(schedule["accounts"][0]["intervalSeconds"], 600);
    assert_eq!(schedule["nextDueAt"], last + 600);
}
