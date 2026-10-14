//! Isolated transport fixtures: no SSH, credentials, CLI inference or scheduler.
#![cfg(unix)]
use aieyes_core::{
    Engine,
    models::{Account, AccountDeviceSettings, Host, Settings, Source},
    wakeups::{Deployment, Record, Task},
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

const FIXTURE_ENV: &str = "AIEYES_WAKEUP_SYNC_FIXTURE";
const FAKE_SSH: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(os.environ['AIEYES_WAKEUP_SYNC_FIXTURE'])
request = json.load(sys.stdin)['request']
with (root / 'requests.jsonl').open('a') as stream:
    stream.write(json.dumps(request) + '\n')
action = request['action']
if action == 'disable':
    (root / 'disable.started').write_text('ready')
    deadline = time.monotonic() + 10
    while not (root / 'disable.release').exists():
        if time.monotonic() >= deadline:
            raise RuntimeError('fixture release timed out')
        time.sleep(0.01)
    response = {'enabled': False}
elif action == 'deploy':
    response = {'error': 'fixture deploy failure'} if (root / 'deploy.fail').exists() else {'root': '~/.fixture-aieyes-wakeups', 'timezone': 'UTC'}
elif action == 'enable':
    response = {'enabled': True}
elif action == 'run':
    response = {'started': True}
elif action == 'remove':
    response = {'error': 'fixture removal failure'}
else:
    raise RuntimeError('unexpected fixture action')
print(json.dumps(response))
"#;

fn isolated(name: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(FIXTURE_ENV) {
        return Some(path.into());
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("ssh"), FAKE_SSH).unwrap();
    fs::set_permissions(bin.join("ssh"), fs::Permissions::from_mode(0o700)).unwrap();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin).chain(std::env::split_paths(&original_path));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads=1"])
        .env(FIXTURE_ENV, &root)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("TMPDIR", &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    None
}

fn fixture(root: &Path) -> Engine {
    let engine = Engine::open(&root.join("db")).unwrap();
    let host = Host {
        id: "remote".into(),
        name: "Fixture server".into(),
        target: "fixture.invalid".into(),
        pre_command: "old command".into(),
        ..Default::default()
    };
    let source = Source {
        id: "source".into(),
        provider: "claude".into(),
        account_id: "account".into(),
        host_id: Some("remote".into()),
        path: "~/.claude-fixture".into(),
        quota_pre_command: "old command".into(),
        ..Default::default()
    };
    engine
        .store
        .save_settings(&Settings {
            hosts: vec![host.clone()],
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: "account".into(),
                provider: "claude".into(),
                name: "Fixture account".into(),
                device_settings: vec![AccountDeviceSettings {
                    machine_id: "remote".into(),
                    pre_command: "old command".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let task = Task {
        id: "wake-review".into(),
        name: "Fixture task".into(),
        source_id: source.id.clone(),
        model: "fixture-model".into(),
        prompt: "Deployed prompt".into(),
        ..Default::default()
    };
    let mut draft = task.clone();
    draft.prompt = "Keep the un-deployed draft prompt".into();
    let record = Record {
        task: draft,
        deployment: Some(Deployment {
            task,
            source,
            host: Some(host),
            root: "~/.fixture-aieyes-wakeups".into(),
            deployed_at: 1,
            enabled: true,
            state: "deployed".into(),
            timezone: "UTC".into(),
        }),
    };
    engine
        .store
        .db
        .execute(
            "INSERT INTO kv(key,value) VALUES('wakeup:wake-review',?1)",
            [serde_json::to_string(&record).unwrap()],
        )
        .unwrap();
    engine
}

fn record(engine: &Engine) -> Value {
    let raw: String = engine
        .store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='wakeup:wake-review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn change_command(engine: &Engine) {
    let mut settings = engine.store.settings().unwrap();
    settings.accounts[0].device_settings[0].pre_command = "new command".into();
    engine.store.save_settings(&settings).unwrap();
    assert_eq!(record(engine)["deployment"]["state"], "settings-pending");
}

fn requests(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("requests.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect()
}

#[test]
fn disabling_during_a_command_edit_preserves_pending_sync_and_the_saved_draft() {
    let Some(root) =
        isolated("disabling_during_a_command_edit_preserves_pending_sync_and_the_saved_draft")
    else {
        return;
    };
    let mut engine = fixture(&root);
    let before = record(&engine);
    let database = root.join("db");
    let worker = std::thread::spawn(move || {
        Engine::open(&database)
            .unwrap()
            .wakeup_call("wakeups.disable", json!({"id":"wake-review"}))
    });
    let started = Instant::now();
    while !root.join("disable.started").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "fake SSH did not receive disable"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    change_command(&engine);
    fs::write(root.join("disable.release"), "continue").unwrap();
    let disabled = worker.join().unwrap().unwrap();
    assert_eq!(disabled["deployment"]["state"], "settings-pending");
    assert_eq!(disabled["deployment"]["enabled"], false);
    assert_eq!(record(&engine)["task"], before["task"]);
    assert_eq!(
        record(&engine)["deployment"]["task"],
        before["deployment"]["task"]
    );
    engine
        .wakeup_call("wakeups.enable", json!({"id":"wake-review"}))
        .unwrap();
    let requests = requests(&root);
    assert_eq!(
        requests
            .iter()
            .map(|r| r["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["disable", "deploy", "enable"]
    );
    assert_eq!(requests[1]["manifest"]["preCommand"], "new command");
    assert_eq!(
        requests[1]["manifest"]["enabled"], false,
        "synchronization must preserve the disabled state"
    );
    assert_eq!(
        requests[1]["manifest"]["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "Deployed prompt"
    );
    let after = record(&engine);
    assert_eq!(after["deployment"]["state"], "deployed");
    assert_eq!(after["deployment"]["enabled"], true);
    assert_eq!(after["task"], before["task"]);
    assert_eq!(after["deployment"]["task"], before["deployment"]["task"]);
}

#[test]
fn failed_removal_command_changes_are_synchronized_before_run_or_enable() {
    let Some(root) =
        isolated("failed_removal_command_changes_are_synchronized_before_run_or_enable")
    else {
        return;
    };
    for action in ["run", "enable"] {
        let mut engine = fixture(&root.join(action));
        let mut before = record(&engine);
        if action == "enable" {
            before["deployment"]["enabled"] = json!(false);
            engine
                .store
                .db
                .execute(
                    "UPDATE kv SET value=?1 WHERE key='wakeup:wake-review'",
                    [before.to_string()],
                )
                .unwrap();
        }
        fs::write(root.join("requests.jsonl"), "").unwrap();
        change_command(&engine);
        let error = engine
            .wakeup_call("wakeups.remove", json!({"id":"wake-review"}))
            .unwrap_err();
        assert!(error.to_string().contains("fixture removal failure"));
        assert_eq!(record(&engine)["deployment"]["state"], "pending-removal");

        fs::write(root.join("deploy.fail"), "fail the sync").unwrap();
        let error = engine
            .wakeup_call(&format!("wakeups.{action}"), json!({"id":"wake-review"}))
            .unwrap_err();
        assert!(
            error.to_string().contains("fixture deploy failure"),
            "{error}"
        );
        let failed = record(&engine);
        assert_eq!(failed["deployment"]["state"], "settings-pending");
        assert_eq!(
            failed["deployment"]["enabled"],
            before["deployment"]["enabled"]
        );
        assert_eq!(failed["task"], before["task"]);
        assert_eq!(failed["deployment"]["task"], before["deployment"]["task"]);
        let attempted = requests(&root);
        assert_eq!(
            attempted
                .iter()
                .map(|r| r["action"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["remove", "deploy"],
            "must not {action} before synchronization succeeds"
        );
        assert_eq!(attempted[1]["manifest"]["preCommand"], "new command");
        assert_eq!(
            attempted[1]["manifest"]["enabled"],
            before["deployment"]["enabled"]
        );

        fs::remove_file(root.join("deploy.fail")).unwrap();
        engine
            .wakeup_call(&format!("wakeups.{action}"), json!({"id":"wake-review"}))
            .unwrap();
        let completed = requests(&root);
        assert_eq!(
            completed
                .iter()
                .map(|r| r["action"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["remove", "deploy", "deploy", action]
        );
        assert_eq!(record(&engine)["deployment"]["state"], "deployed");
        assert_eq!(record(&engine)["deployment"]["enabled"], true);
        assert_eq!(record(&engine)["task"], before["task"]);
        assert_eq!(
            record(&engine)["deployment"]["task"],
            before["deployment"]["task"]
        );
    }
}
