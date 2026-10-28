use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn observer(provider: &str, event: &str, input: &Value) -> (String, String) {
    observer_with_entrypoint(provider, event, input, None)
}
fn observer_with_entrypoint(
    provider: &str,
    event: &str,
    input: &Value,
    entrypoint: Option<&str>,
) -> (String, String) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".aieyes/session-events")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_aieyes-core"));
    command.env_remove("CLAUDE_CODE_ENTRYPOINT");
    if let Some(entrypoint) = entrypoint {
        command.env("CLAUDE_CODE_ENTRYPOINT", entrypoint);
    }
    let mut child = command
        .args([
            "--aieyes-session-hook",
            provider,
            event,
            root.path().to_str().unwrap(),
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
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let rows = fs::read_dir(root.path().join(".aieyes/session-events"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect();
    (String::from_utf8(result.stdout).unwrap(), rows)
}
#[test]
fn claude_observer_retains_noninteractive_source_without_prompt_content() {
    let (_, rows) = observer_with_entrypoint(
        "claude",
        "UserPromptSubmit",
        &json!({"session_id":"headless","entrypoint":"cli","prompt":"SECRET"}),
        Some("sdk-cli"),
    );
    let event: Value = serde_json::from_str(rows.trim()).unwrap();
    assert_eq!(event["sessionSource"], "sdk-cli");
    assert!(!rows.contains("SECRET"));
}
#[test]
fn observer_protocol_is_noop_and_persists_only_lifecycle_metadata() {
    let payload = json!({"session_id":"s","conversationId":"a","tool_name":"Bash","tool_input":{"command":"SECRET"},"prompt":"SECRET","response":"SECRET","fullyIdle":false});
    let (stdout, rows) = observer("claude", "PermissionRequest", &payload);
    assert!(stdout.is_empty());
    assert!(!rows.contains("SECRET"));
    assert!(rows.contains("PermissionRequest"));
    let (stdout, rows) = observer("antigravity", "Stop", &payload);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).unwrap(),
        json!({"decision":"allow"})
    );
    assert!(rows.contains("fullyIdle"));
    assert!(!rows.contains("SECRET"));
    let (stdout, _) = observer("antigravity", "PostInvocation", &json!({}));
    assert_eq!(stdout.trim(), "{}");
}
#[test]
fn a_provider_that_never_closes_stdin_cannot_hang_the_observer() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aieyes-core"))
        .args([
            "--aieyes-session-hook",
            "antigravity",
            "Stop",
            root.path().to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(start.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
