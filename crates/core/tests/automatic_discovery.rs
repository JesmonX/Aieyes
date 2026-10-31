//! Exercise discovery through the same background status request used by the apps.
//! Only isolated credentials and a local SSH transport fixture are used.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aieyes-core"));
    command.args(["--data-dir", root.join("data").to_str().unwrap()]);
    for (name, suffix) in [
        ("HOME", ""),
        ("USERPROFILE", ""),
        ("CODEX_HOME", ".codex"),
        ("CURSOR_CONFIG_DIR", ".cursor"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("APPDATA", "AppData/Roaming"),
    ] {
        command.env(name, root.join("home").join(suffix));
    }
    command.env_remove("AIEYES_AUTO_SCAN");
    if root.join("bin").is_dir() {
        command.env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    command
}

fn rpc(root: &Path, method: &str, params: Value) -> Value {
    let output = command(root)
        .args(["--call", method, &params.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{method}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn setup(provider: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("home")).unwrap();
    let mut settings = rpc(root.path(), "settings.get", json!({}));
    settings["sources"] = json!([]);
    for agent in settings["agents"].as_array_mut().unwrap() {
        agent["enabled"] = json!(agent["provider"] == provider);
        agent["machineIds"] = json!([]);
    }
    rpc(root.path(), "settings.save", settings);
    root
}

fn jwt(claims: Value) -> String {
    format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

fn codex_auth() -> Value {
    json!({"tokens":{"account_id":"workspace","access_token":jwt(json!({
        "sub":"user","exp":4102444800_i64,
        "https://api.openai.com/auth":{"chatgpt_user_id":"user","chatgpt_account_id":"workspace"}
    })),"id_token":jwt(json!({"sub":"user","email":"discovery@example.test"}))}})
}

fn write(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

#[test]
fn server_startup_discovers_the_existing_native_login_without_an_rpc_scan() {
    let root = setup("codex");
    let root = root.path();
    let path = root.join("home/.codex/auth.json");
    write(&path, codex_auth());
    let original = std::fs::read(&path).unwrap();
    let mut child = command(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut found = false;
    while Instant::now() < deadline {
        let settings = rpc(root, "settings.get", json!({}));
        if settings["accounts"].as_array().unwrap().len() == 1 {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    child.wait().unwrap();
    assert!(
        found,
        "Startup did not discover the fixture login within five seconds"
    );
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn newly_created_glob_credentials_are_found_without_a_manual_scan() {
    let root = setup("codex");
    let root = root.path();
    rpc(root, "accounts.autoScan", json!({}));
    write(
        &root.join("home/.codex/.aieyes/accounts/new/auth.json"),
        codex_auth(),
    );
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    let settings = rpc(root, "settings.get", json!({}));
    assert_eq!(settings["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(settings["nativeConnections"][0]["status"], "observed");
}

#[test]
fn first_login_in_a_configured_nondefault_directory_is_discovered() {
    let root = setup("codex");
    let root = root.path();
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["sources"] = json!([{
        "id":"custom-home","provider":"codex","path":root.join("custom"),
        "name":"Custom directory","enabled":true,"accountId":""
    }]);
    rpc(root, "settings.save", settings);
    rpc(root, "accounts.autoScan", json!({}));
    write(&root.join("custom/auth.json"), codex_auth());
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    assert_eq!(
        rpc(root, "settings.get", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn enabling_an_agent_discovers_an_unchanged_existing_login() {
    let root = setup("codex");
    let root = root.path();
    let mut settings = rpc(root, "settings.get", json!({}));
    for agent in settings["agents"].as_array_mut().unwrap() {
        agent["enabled"] = json!(false);
    }
    rpc(root, "settings.save", settings);
    rpc(root, "accounts.autoScan", json!({}));
    write(&root.join("home/.codex/auth.json"), codex_auth());
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["agents"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|agent| agent["provider"] == "codex")
        .unwrap()["enabled"] = json!(true);
    rpc(root, "settings.save", settings);
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    assert_eq!(
        rpc(root, "settings.get", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn first_ide_login_is_detected_when_only_the_sqlite_wal_changes() {
    let root = setup("cursor");
    let root = root.path();
    let base = if cfg!(target_os = "macos") {
        "home/Library/Application Support"
    } else if cfg!(windows) {
        "home/AppData/Roaming"
    } else {
        "home/.config"
    };
    let path = root
        .join(base)
        .join("Cursor/User/globalStorage/state.vscdb");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT); PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    rpc(root, "accounts.autoScan", json!({}));
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    for (key, value) in [
        (
            "cursorAuth/accessToken",
            jwt(json!({"sub":"cursor-user","exp":4102444800_i64})),
        ),
        ("cursorAuth/cachedEmail", "cursor@example.test".to_owned()),
    ] {
        db.execute("INSERT INTO ItemTable VALUES(?1,?2)", [key, &value])
            .unwrap();
    }
    assert_eq!(
        before,
        std::fs::metadata(&path).unwrap().modified().unwrap()
    );
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    let settings = rpc(root, "settings.get", json!({}));
    assert_eq!(settings["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(settings["accounts"][0]["provider"], "cursor");
    assert_eq!(
        before,
        std::fs::metadata(&path).unwrap().modified().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn first_login_on_an_enabled_remote_host_is_discovered() {
    use std::os::unix::fs::PermissionsExt;
    let root = setup("codex");
    let root = root.path();
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\nfor last do :; done\nexec /bin/sh -c \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut settings = rpc(root, "settings.get", json!({}));
    settings["localDeviceEnabled"] = json!(false);
    settings["hosts"] = json!([{
        "id":"remote","name":"Fixture SSH","target":"fixture.invalid",
        "enabled":true,"shell":"/bin/sh"
    }]);
    rpc(root, "settings.save", settings);
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    assert!(
        rpc(root, "settings.get", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    write(&root.join("home/.codex/auth.json"), codex_auth());
    // Unchanged machines are checked at most once per five minutes. A login
    // appearing after the first empty scan is found on the next due check.
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    assert!(
        rpc(root, "settings.get", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let db = rusqlite::Connection::open(root.join("data/aieyes.sqlite")).unwrap();
    let poll: String = db
        .query_row(
            "SELECT value FROM kv WHERE key='native-remote-poll:remote'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut poll: Value = serde_json::from_str(&poll).unwrap();
    poll["checkedAt"] = json!(0);
    db.execute(
        "UPDATE kv SET value=?1 WHERE key='native-remote-poll:remote'",
        [poll.to_string()],
    )
    .unwrap();
    drop(db);
    rpc(
        root,
        "accounts.status.refreshBatch",
        json!({"dueOnly":true}),
    );
    let settings = rpc(root, "settings.get", json!({}));
    assert_eq!(settings["accounts"].as_array().unwrap().len(), 1);
    assert!(
        settings["nativeConnections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["machineId"] == "remote")
    );
}

#[test]
fn fixture_opt_out_also_disables_discovery_from_status_refresh() {
    let root = setup("codex");
    let root = root.path();
    write(&root.join("home/.codex/auth.json"), codex_auth());
    let output = command(root)
        .env("AIEYES_AUTO_SCAN", "off")
        .args([
            "--call",
            "accounts.status.refreshBatch",
            "{\"dueOnly\":true}",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        rpc(root, "settings.get", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
