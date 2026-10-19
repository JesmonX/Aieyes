use aieyes_core::{
    session_events::{self, Activity},
    sessions::Phase,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn rpc(root: &Path, method: &str, params: Value) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_aieyes-core"))
        .args([
            "--data-dir",
            root.join("data").to_str().unwrap(),
            "--call",
            method,
            &params.to_string(),
        ])
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn observed(helper: &Path, home: &Path, kind: &str, input: Value) {
    let mut child = Command::new(helper)
        .args([
            "--aieyes-session-hook",
            "cursor",
            kind,
            home.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
}

#[test]
fn cursor_native_observer_is_silent_preserves_config_and_projects_lifecycle_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home/.cursor");
    fs::create_dir_all(&home).unwrap();
    let original = json!({"version":1,"otherSetting":"keep","hooks":{"preToolUse":[{"command":"other-guard","matcher":"Shell"}]}});
    fs::write(home.join("hooks.json"), original.to_string()).unwrap();
    let settings = json!({"version":6,"sources":[{"id":"cursor","provider":"cursor","name":"Cursor","path":home,"enabled":true}],"accounts":[],"hosts":[],"agents":[{"provider":"cursor","enabled":true,"machineIds":["local"]}]});
    rpc(root, "settings.save", settings);
    let first = rpc(root, "sessions.hooks.repair", json!({"sourceId":"cursor"}));
    assert_eq!(first[0]["state"], "installed");
    let installed = fs::read(home.join("hooks.json")).unwrap();
    rpc(root, "sessions.list", json!({}));
    assert_eq!(fs::read(home.join("hooks.json")).unwrap(), installed);
    let config: Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(
        config["hooks"]["preToolUse"][0],
        original["hooks"]["preToolUse"][0]
    );
    assert!(config["hooks"]["preToolUse"][1]["command"].is_string());
    assert!(config["hooks"]["preToolUse"][1].get("hooks").is_none());
    let helper: PathBuf = fs::read_dir(home.join(".aieyes"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("aieyes-session-hook-")
        })
        .unwrap();
    let payload = |turn: &str| json!({"conversation_id":"session","generation_id":turn,"tool_call_id":"call","tool_name":"Shell","prompt":"PRIVATE_PROMPT","tool_input":{"command":"PRIVATE_COMMAND"},"tool_output":"PRIVATE_OUTPUT","user_email":"PRIVATE_EMAIL","transcript_path":"PRIVATE_PATH"});
    observed(&helper, &home, "sessionStart", payload(""));
    for (kind, phase) in [
        ("beforeSubmitPrompt", "working"),
        ("afterAgentThought", "thinking"),
        ("preToolUse", "tool"),
        ("postToolUseFailure", "working"),
    ] {
        observed(&helper, &home, kind, payload("one"));
        assert_eq!(
            rpc(root, "sessions.list", json!({}))["sessions"][0]["phase"],
            phase
        );
    }
    let mut end = payload("one");
    end["status"] = json!("completed");
    observed(&helper, &home, "stop", end);
    assert_eq!(
        rpc(root, "sessions.list", json!({}))["sessions"][0]["phase"],
        "complete"
    );
    for (turn, status, phase) in [
        ("two", "error", "failed"),
        ("three", "aborted", "interrupted"),
        ("four", "future-status", "unknown"),
    ] {
        observed(&helper, &home, "beforeSubmitPrompt", payload(turn));
        let mut end = payload(turn);
        end["status"] = json!(status);
        observed(&helper, &home, "stop", end);
        assert_eq!(
            rpc(root, "sessions.list", json!({}))["sessions"][0]["phase"],
            phase
        );
    }
    let log = fs::read_dir(home.join(".aieyes/session-events"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .unwrap();
    assert!(!fs::read_to_string(&log).unwrap().contains("PRIVATE"));
    assert_eq!(
        rpc(root, "sessions.hooks.remove", json!({"sourceId":"cursor"}))[0]["state"],
        "disabled"
    );
    let removed: Value =
        serde_json::from_slice(&fs::read(home.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(removed["otherSetting"], "keep");
    assert_eq!(
        removed["hooks"]["preToolUse"],
        original["hooks"]["preToolUse"]
    );
    let before = fs::read(&log).unwrap();
    observed(&helper, &home, "beforeSubmitPrompt", payload("disabled"));
    assert_eq!(fs::read(&log).unwrap(), before);
    assert_eq!(
        rpc(root, "sessions.hooks.status", json!({"sourceId":"cursor"}))[0]["state"],
        "disabled"
    );
    assert_eq!(
        rpc(root, "sessions.list", json!({}))["sources"][0]["state"],
        "degraded"
    );
    let future = json!({"version":2,"hooks":{"preToolUse":[]}}).to_string();
    fs::write(home.join("hooks.json"), &future).unwrap();
    assert_eq!(
        rpc(root, "sessions.hooks.repair", json!({"sourceId":"cursor"}))[0]["state"],
        "error"
    );
    assert_eq!(fs::read_to_string(home.join("hooks.json")).unwrap(), future);
}

#[test]
fn cursor_waiting_and_stale_stops_preserve_the_current_generation() {
    let mut activity = Activity::default();
    let row = |turn: &str| json!({"conversation_id":"c","generation_id":turn,"tool_name":"AskQuestion","tool_call_id":"ask"});
    activity.apply(session_events::hook(
        "cursor",
        "beforeSubmitPrompt",
        &row("one"),
        100,
    ));
    activity.apply(session_events::hook(
        "cursor",
        "preToolUse",
        &row("one"),
        101,
    ));
    assert_eq!(activity.phase, Some(Phase::NeedsInput));
    activity.apply(session_events::hook(
        "cursor",
        "postToolUse",
        &row("one"),
        102,
    ));
    assert_eq!(activity.phase, Some(Phase::Working));
    activity.apply(session_events::hook(
        "cursor",
        "beforeSubmitPrompt",
        &row("two"),
        103,
    ));
    let mut old = row("one");
    old["status"] = json!("completed");
    activity.apply(session_events::hook("cursor", "stop", &old, 104));
    assert_eq!(activity.phase, Some(Phase::Working));
}

#[test]
fn quota_only_cursor_source_never_installs_observers_or_assumes_local_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home/.cursor");
    fs::create_dir_all(&home).unwrap();
    rpc(
        root,
        "settings.save",
        json!({"version":6,"sources":[{"id":"quota","provider":"cursor","path":"","enabled":true,"subscription":{"mode":"session","credentialRef":"fixture-credential"}}],"accounts":[],"hosts":[],"agents":[{"provider":"cursor","enabled":true,"machineIds":["local"]}]}),
    );
    let snapshot = rpc(root, "sessions.list", json!({}));
    assert_eq!(snapshot["sources"], json!([]));
    assert_eq!(snapshot["sessions"], json!([]));
    assert_eq!(
        rpc(root, "sessions.hooks.repair", json!({"sourceId":"quota"})),
        json!([])
    );
    assert!(!home.join("hooks.json").exists());
    assert!(!home.join(".aieyes").exists());
}

#[test]
fn cursor_private_history_is_explicitly_unavailable_and_standard_exports_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("home/.cursor");
    let private = home.join("chats/workspace/session/store.db");
    fs::create_dir_all(private.parent().unwrap()).unwrap();
    fs::write(&private, b"PRIVATE_DATABASE_BODY").unwrap();
    let transcript = home.join("projects/project/agent-transcripts/transcript.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(transcript,json!({"id":"private","sessionId":"p","model":"private","timestamp":aieyes_core::models::now(),"tokens":{"input":999},"content":"PRIVATE_PROMPT"}).to_string()+"\n").unwrap();
    let export = home.join("imports/usage.jsonl");
    fs::create_dir_all(export.parent().unwrap()).unwrap();
    fs::write(&export,json!({"id":"request","sessionId":"s","model":"fixture","timestamp":aieyes_core::models::now(),"tokens":{"input":20}}).to_string()+"\n").unwrap();
    rpc(
        root,
        "settings.save",
        json!({"version":6,"sources":[{"id":"cursor","provider":"cursor","path":home,"enabled":true}],"accounts":[],"hosts":[],"agents":[{"provider":"cursor","enabled":true,"machineIds":["local"]}]}),
    );
    let first = rpc(root, "sources.scan", json!({}));
    assert_eq!(first[0]["result"]["newEvents"], 1);
    assert_eq!(first[0]["result"]["partial"], true);
    assert_eq!(
        first[0]["result"]["issues"][0]["code"],
        "unsupportedNativeHistory"
    );
    assert_eq!(rpc(root, "dashboard", json!({}))["summary"]["total"], 20);
    assert_eq!(fs::read(&private).unwrap(), b"PRIVATE_DATABASE_BODY");
    assert_eq!(
        rpc(root, "sources.scan", json!({}))[0]["result"]["newEvents"],
        0
    );
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["sources"][0]["path"] = json!(export);
    rpc(root, "settings.save", settings);
    let file = rpc(root, "sources.scan", json!({}));
    assert_eq!(file[0]["result"]["newEvents"], 0);
    assert_ne!(file[0]["result"]["partial"], true);
    let legacy = root.join("standard/projects/usage.jsonl");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy,json!({"id":"legacy","sessionId":"s","model":"fixture","timestamp":aieyes_core::models::now(),"tokens":{"input":30}}).to_string()+"\n").unwrap();
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["sources"][0]["path"] = json!(root.join("standard"));
    rpc(root, "settings.save", settings);
    let folder = rpc(root, "sources.scan", json!({}));
    assert_eq!(folder[0]["result"]["newEvents"], 1);
    assert_ne!(folder[0]["result"]["partial"], true);
}
