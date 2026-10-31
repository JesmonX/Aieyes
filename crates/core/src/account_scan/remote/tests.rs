use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

const TOKEN: &str = "native-quota-test-access-do-not-export";

fn jwt(value: Value) -> String {
    format!(
        "fixture.{}.signature",
        URL_SAFE_NO_PAD.encode(value.to_string())
    )
}

fn inspect(row: &NativeConnection) -> Projection {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/remote_native_quota.py");
    let mut process = Command::new("python3")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"reference":{"provider":row.provider,"root":row.root,"configPath":row.config_path,
        "selector":row.selector,"kind":row.kind}});
    process
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    serde_json::from_value(value["result"].clone())
        .unwrap_or_else(|_| panic!("projection error: {}", value["errorCode"]))
}

fn write_candidate(
    root: &Path,
    provider: &str,
    filename: &str,
    value: Value,
    selector: &str,
) -> Candidate {
    let path = root.join(filename);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let document = Document {
        root: root.to_string_lossy().into(),
        path: path.to_string_lossy().into(),
        bytes,
    };
    extract(provider, "quota-fixture", &document)
        .unwrap()
        .into_iter()
        .find(|c| c.connection.selector == selector)
        .unwrap_or_else(|| panic!("missing {provider} {selector}"))
}

#[test]
fn public_remote_projection_matches_local_native_identity_and_fingerprint() {
    let temp = tempfile::tempdir().unwrap();
    let access = jwt(
        json!({"sub":"user-1","iss":"https://auth.fixture.invalid","email":"fixture@example.invalid","exp":4102444800_i64}),
    );
    let openai = jwt(
        json!({"https://api.openai.com/auth":{"chatgpt_user_id":"user-1","chatgpt_account_id":"default-team","chatgpt_plan_type":"plus"},
        "https://api.openai.com/profile":{"email":"fixture@example.invalid"},"exp":4102444800_i64}),
    );
    let cases = [
        (
            "claude",
            ".credentials.json",
            json!({"claudeAiOauth":{"accessToken":access}}),
            "/claudeAiOauth",
        ),
        (
            "cursor",
            "cli-config.json",
            json!({"accessToken":access,"email":"fixture@example.invalid"}),
            "",
        ),
        (
            "grok",
            "auth.json",
            json!({"https://auth.x.ai::subject":{"key":access,"user_id":"grok-user","expires_at":4102444800_i64,"last_used":9}}),
            "/https:~1~1auth.x.ai::subject",
        ),
        (
            "zcode",
            "v2/credentials.json",
            json!({"zcodeJwtToken":access,"provider":"zai","zaiUserInfo":{"userId":42}}),
            "/zcodeJwtToken",
        ),
        (
            "zcode",
            "v2/credentials.json",
            json!({"zcodejwttoken":access,"oauth:active_provider":"bigmodel",
            "oauth:bigmodel:user_info":json!({"id":"organization-1","user_id":"different-user"}).to_string()}),
            "/zcodejwttoken",
        ),
        (
            "deepseek-harness",
            ".credentials.yaml",
            json!({"version":1,"records":{"deepseek-account-platform/default":{
            "kind":"grant","payload":{"version":1,"issuer":"https://platform.deepseek.com","token":access,"accountId":"dsh-user"}}}}),
            "/records/deepseek-account-platform~1default",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"cline":{"settings":{"auth":{"accessToken":access,"accountId":"cline-user"}}}}}),
            "/providers/cline/settings/auth",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"cline":{"settings":{"apiKey":TOKEN}}}}),
            "/providers/cline/settings",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"cline":{"settings":{"auth":{"apiKey":TOKEN,"accountId":"cline-user"}}}}}),
            "/providers/cline/settings",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"named-login":{"settings":{"provider":"cline","apiKey":TOKEN,"auth":{"accountId":"cline-user"}}}}}),
            "/providers/named-login/settings",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"named-login":{"settings":{"provider":"cline","auth":{"accessToken":access,"accountId":"cline-user"}}}}}),
            "/providers/named-login/settings/auth",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"openrouter":{"settings":{"auth":{"apiKey":TOKEN}}}}}),
            "/providers/openrouter/settings",
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"named-router":{"settings":{"provider":"openrouter","auth":{"apiKey":TOKEN}}}}}),
            "/providers/named-router/settings",
        ),
        (
            "kilo",
            "auth.json",
            json!({"kilo":{"type":"oauth","access":TOKEN,"refresh":"unused-refresh-fixture","accountId":"org-not-user"}}),
            "/kilo",
        ),
        (
            "kilo",
            "config.json",
            json!({"providers":{"openrouter":{"apiKey":TOKEN}}}),
            "/providers/openrouter",
        ),
        (
            "kilo",
            "config.json",
            json!({"provider":{"openrouter":{"options":{"apiKey":TOKEN}}}}),
            "/provider/openrouter",
        ),
        (
            "kiro",
            "kiro-auth-token.json",
            json!({"accessToken":TOKEN,"profileArn":"arn:aws:codewhisperer:us-east-1:1:profile/a","provider":"Google","expiresAt":"2100-01-01T00:00:00Z"}),
            "/kiro-auth-token",
        ),
        (
            "trae",
            "storage.json",
            json!({"iCubeAuthInfo://icube.cloudide":{"token":TOKEN,"userId":"trae-user","host":"grow-normal.trae.ai","expiredAt":4102444800_i64}}),
            "/iCubeAuthInfo:~1~1icube.cloudide",
        ),
        (
            "copilot",
            "config.json",
            json!({"copilot_tokens":{"https://github.com:fixture":"gho_fixture-native-token"}}),
            "/copilot_tokens/https:~1~1github.com:fixture",
        ),
        (
            "opencode",
            "auth.json",
            json!({"opencode-go":{"type":"api","key":TOKEN}}),
            "/opencode-go",
        ),
        (
            "pi",
            "auth.json",
            json!({"anthropic":{"type":"oauth","access":TOKEN,"refresh":"unused-refresh-fixture","expires":4102444800000_i64}}),
            "/anthropic",
        ),
        (
            "omp",
            "auth.json",
            json!({"github-copilot":{"type":"oauth","access":"derived-token","refresh":"gho_fixture-native-token","expires":1}}),
            "/github-copilot",
        ),
        (
            "opencode",
            "auth.json",
            json!({"openai":{"type":"oauth","access":openai,"refresh":"unused-refresh-fixture","accountId":"selected-team"}}),
            "/openai",
        ),
        (
            "codex",
            "auth.json",
            json!({"tokens":{"access_token":openai,"account_id":"selected-team","refresh_token":"unused-refresh-fixture"}}),
            "",
        ),
    ];
    for (index, (provider, filename, value, selector)) in cases.into_iter().enumerate() {
        let root = temp.path().join(index.to_string());
        let expected = write_candidate(&root, provider, filename, value, selector);
        let projection = inspect(&expected.connection);
        let actual = candidate_from_projection(&expected.connection, &projection).unwrap();
        assert_eq!(
            actual.connection.identity_key, expected.connection.identity_key,
            "{provider}"
        );
        assert_eq!(
            actual.connection.upstream_identity, expected.connection.upstream_identity,
            "{provider}"
        );
        assert_eq!(
            actual.connection.token_expired_at, expected.connection.token_expired_at,
            "{provider}"
        );
        assert_eq!(
            actual.connection.base_url, expected.connection.base_url,
            "{provider}"
        );
        assert_eq!(actual.fingerprint, expected.fingerprint, "{provider}");
        assert_eq!(actual.plan, expected.plan, "{provider}");
        assert_eq!(
            native_subscription(&actual.connection),
            native_subscription(&expected.connection),
            "{provider}"
        );
        assert_ne!(actual.secret, expected.secret, "{provider}");
    }
}

#[test]
fn dotenv_comment_and_grok_expiry_grace_match_on_both_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".env");
    let raw = format!(
        "export DEEPSEEK_API_KEY=\"{TOKEN}\" # native comment\nDEEPSEEK_BASE_URL=https://api.deepseek.com # route\n"
    );
    std::fs::write(&path, &raw).unwrap();
    let doc = Document {
        root: temp.path().to_string_lossy().into(),
        path: path.to_string_lossy().into(),
        bytes: raw.into_bytes(),
    };
    let expected = extract("deepseek-harness", "quota-fixture", &doc)
        .unwrap()
        .pop()
        .unwrap();
    let actual =
        candidate_from_projection(&expected.connection, &inspect(&expected.connection)).unwrap();
    assert_eq!(actual.fingerprint, expected.fingerprint);
    assert_eq!(actual.connection.base_url, expected.connection.base_url);
    for (index, offset) in [30, -300, -STALE_AFTER - 1].into_iter().enumerate() {
        let expiry = now() + offset;
        let stamp = chrono::DateTime::from_timestamp(expiry, 0)
            .unwrap()
            .to_rfc3339();
        let expected = write_candidate(
            &temp.path().join(index.to_string()),
            "grok",
            "auth.json",
            json!({"https://auth.x.ai::user":{"key":TOKEN,"user_id":"fixture","expires_at":stamp}}),
            "/https:~1~1auth.x.ai::user",
        );
        assert_eq!(
            expected.connection.token_expired_at,
            (offset <= 0).then_some(expiry)
        );
        assert_eq!(
            expected.connection.status,
            if offset <= -STALE_AFTER {
                "stale"
            } else {
                "observed"
            }
        );
        if offset <= -STALE_AFTER {
            assert_eq!(expected.connection.note, stale_note("grok"));
        }
        let actual =
            candidate_from_projection(&expected.connection, &inspect(&expected.connection))
                .unwrap();
        assert_eq!(
            actual.connection.token_expired_at,
            expected.connection.token_expired_at
        );
        assert_eq!(actual.connection.status, expected.connection.status);
    }
}

#[test]
fn grok_selection_uses_the_same_issuance_metadata_on_both_hosts() {
    let temp = tempfile::tempdir().unwrap();
    for other_issued in [100, 400] {
        let value = json!({
            "https://auth.x.ai::a":{"key":jwt(json!({"sub":"user-a","iat":300,"exp":4102444800_i64})),"expires_at":4102444800_i64},
            "https://auth.x.ai::b":{"key":jwt(json!({"sub":"user-b","iat":other_issued,"exp":4102444800_i64})),"expires_at":4102444800_i64}
        });
        for name in ["a", "b"] {
            let expected = write_candidate(
                temp.path(),
                "grok",
                "auth.json",
                value.clone(),
                &format!("/https:~1~1auth.x.ai::{name}"),
            );
            let actual =
                candidate_from_projection(&expected.connection, &inspect(&expected.connection))
                    .unwrap();
            let active = if other_issued > 300 { "b" } else { "a" };
            assert_eq!(
                expected.connection.status,
                if name == active {
                    "observed"
                } else {
                    "remembered"
                }
            );
            assert_eq!(actual.connection.status, expected.connection.status);
            assert_eq!(
                actual.connection.identity_key,
                expected.connection.identity_key
            );
        }
    }
}

#[test]
fn removed_or_invalid_native_routes_never_reuse_the_cached_origin() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let raw = format!(
        "[providers.moonshot]\napi_key=\"{TOKEN}\"\nbase_url=\"https://api.moonshot.ai/v1\"\n"
    );
    std::fs::write(&path, &raw).unwrap();
    let doc = Document {
        root: temp.path().to_string_lossy().into(),
        path: path.to_string_lossy().into(),
        bytes: raw.into_bytes(),
    };
    let expected = extract("kimi", "quota-fixture", &doc)
        .unwrap()
        .pop()
        .unwrap();
    for base in ["", "base_url=\"https://user:private@example.invalid/v1\"\n"] {
        std::fs::write(
            &path,
            format!("[providers.moonshot]\napi_key=\"{TOKEN}\"\n{base}"),
        )
        .unwrap();
        let current =
            candidate_from_projection(&expected.connection, &inspect(&expected.connection))
                .unwrap();
        assert!(current.connection.base_url.is_empty());
        assert!(verify(&expected.connection, &current, &expected.fingerprint).is_err());
    }
    let value = json!({"version":1,"records":{"deepseek-account-platform/default":{"kind":"grant","payload":{"version":1,"issuer":"https://platform.deepseek.com","token":TOKEN}}}});
    let expected = write_candidate(
        temp.path(),
        "deepseek-harness",
        ".credentials.yaml",
        value.clone(),
        "/records/deepseek-account-platform~1default",
    );
    let mut changed = value;
    changed["records"]["deepseek-account-platform/default"]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("issuer");
    std::fs::write(temp.path().join(".credentials.yaml"), changed.to_string()).unwrap();
    let current =
        candidate_from_projection(&expected.connection, &inspect(&expected.connection)).unwrap();
    assert!(current.connection.base_url.is_empty());
    assert!(native_subscription(&current.connection).is_none());
}

#[cfg(unix)]
#[test]
fn native_ssh_quota_fixture_has_local_parity() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(temporary.path()).unwrap();
    let root = canonical.as_path();
    let ssh = root.join("ssh");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/native-quota-ssh.py"),
        &ssh,
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "account_scan::remote::tests::native_ssh_quota_fixture_worker",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("AIEYES_NATIVE_QUOTA_FIXTURE", root)
        .env("AIEYES_AUTO_SCAN", "off")
        .env("HOME", root)
        .env("COPILOT_HOME", root.join(".copilot"))
        .env("XDG_CONFIG_HOME", root.join(".config"))
        .env("GH_CONFIG_DIR", root.join(".config/gh"))
        .env_remove("COPILOT_GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("AIEYES_NATIVE_QUOTA_REMOTE_ENV")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
}

#[test]
fn native_ssh_quota_fixture_worker() {
    let Some(directory) = std::env::var_os("AIEYES_NATIVE_QUOTA_FIXTURE") else {
        return;
    };
    worker(Path::new(&directory));
}

fn worker(root: &Path) {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let remote_environment = root.join("remote-environment.json");
    std::fs::write(&remote_environment, "{}").unwrap();
    // This worker is a dedicated one-test process; the parent test's environment
    // and concurrent tests are not changed.
    unsafe {
        std::env::set_var("AIEYES_NATIVE_QUOTA_HTTP", &base);
        std::env::set_var("AIEYES_NATIVE_QUOTA_REMOTE_ENV", &remote_environment);
        std::env::set_var(
            "COPILOT_GITHUB_TOKEN",
            "gho_DESKTOP_PRIVATE_NEVER_USE_REMOTELY",
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(Mutex::new(Vec::<(
        String,
        std::collections::BTreeMap<String, String>,
    )>::new()));
    let server_stop = stop.clone();
    let server_captured = captured.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let path = first
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut headers = std::collections::BTreeMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                let (key, value) = line.split_once(':').unwrap();
                headers.insert(key.to_ascii_lowercase(), value.trim().to_owned());
            }
            let length = headers
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let (status, response) = fixture_response(&path, &headers);
            let body = response.to_string();
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            server_captured.lock().unwrap().push((path, headers));
        }
    });
    let access = jwt(json!({"sub":"user-1","exp":4102444800_i64}));
    let cases = [
        (
            "claude",
            ".credentials.json",
            json!({"claudeAiOauth":{"accessToken":access}}),
            "/claudeAiOauth",
            None,
        ),
        (
            "cursor",
            "cli-config.json",
            json!({"accessToken":access}),
            "",
            None,
        ),
        (
            "zcode",
            "v2/credentials.json",
            json!({"zcodejwttoken":access,"oauth:active_provider":"zai", "oauth:zai:user_info":"{\"user_id\":\"user-1\"}"}),
            "/zcodejwttoken",
            None,
        ),
        (
            "deepseek-harness",
            ".credentials.yaml",
            json!({"version":1,"records":{"deepseek-account-platform/default":{"kind":"grant","payload":{"version":1,"issuer":"https://platform.deepseek.com","token":access}}}}),
            "/records/deepseek-account-platform~1default",
            None,
        ),
        (
            "grok",
            "auth.json",
            json!({"https://auth.x.ai::user":{"key":access,"user_id":"grok-user"}}),
            "/https:~1~1auth.x.ai::user",
            None,
        ),
        (
            "cline",
            "settings/providers.json",
            json!({"providers":{"cline":{"settings":{"auth":{"accessToken":TOKEN}}}}}),
            "/providers/cline/settings/auth",
            Some(format!("cline:{}", hash("user-1"))),
        ),
        (
            "kilo",
            "auth.json",
            json!({"kilo":{"type":"api","key":TOKEN}}),
            "/kilo",
            Some(format!("kilo:{}", hash("user-1"))),
        ),
        (
            "kimi",
            "credentials.json",
            json!({"access_token":TOKEN}),
            "",
            Some(format!("kimi:{}", hash("user-1"))),
        ),
        (
            "copilot",
            "config.json",
            json!({"copilot_tokens":{"https://github.com:fixture":"gho_fixture-native-token"}}),
            "/copilot_tokens/https:~1~1github.com:fixture",
            Some("copilot:7".into()),
        ),
        (
            "opencode",
            "auth.json",
            json!({"opencode-go":{"type":"api","key":TOKEN}}),
            "/opencode-go",
            None,
        ),
        (
            "kiro",
            "kiro-auth-token.json",
            json!({"accessToken":TOKEN,"profileArn":"arn:aws:codewhisperer:us-east-1:1:profile/a","provider":"Google","expiresAt":4102444800_i64}),
            "/kiro-auth-token",
            None,
        ),
        (
            "trae",
            "storage.json",
            json!({"iCubeAuthInfo://icube.cloudide":{"token":TOKEN,"userId":"user-1","host":"grow-normal.trae.ai","expiredAt":4102444800_i64}}),
            "/iCubeAuthInfo:~1~1icube.cloudide",
            None,
        ),
        (
            "pi",
            "auth.json",
            json!({"anthropic":{"type":"oauth","access":TOKEN}}),
            "/anthropic",
            Some(format!(
                "pi:{}",
                hash(
                    &json!([
                        "/anthropic",
                        "claude",
                        json!(["user-1", "org-1"]).to_string()
                    ])
                    .to_string()
                )
            )),
        ),
        (
            "omp",
            "auth.json",
            json!({"github-copilot":{"type":"oauth","access":"derived-token","refresh":"gho_fixture-native-token"}}),
            "/github-copilot",
            Some(format!(
                "omp:{}",
                hash(&json!(["/github-copilot", "copilot", "7"]).to_string())
            )),
        ),
    ];
    for (index, (provider, filename, value, selector, expected_identity)) in
        cases.into_iter().enumerate()
    {
        let home = root.join(format!("case-{index}"));
        let candidate = write_candidate(&home, provider, filename, value, selector);
        let mut row = candidate.connection.clone();
        if let Some(identity) = expected_identity {
            row.identity_key = Some(identity);
        }
        if provider == "kimi" {
            row.base_url = KIMI_CN.into();
            row.upstream_identity = row.identity_key.clone();
        }
        if provider == "omp" {
            row.upstream_identity = Some("github:7".into());
        }
        let account = Account {
            id: format!("account-{index}"),
            provider: provider.into(),
            name: provider.into(),
            ..Default::default()
        };
        let source = Source {
            id: format!("native-{}", row.id),
            provider: provider.into(),
            name: provider.into(),
            account_id: account.id.clone(),
            path: row.root.clone(),
            host_id: Some("quota-fixture".into()),
            subscription: native_subscription(&row),
            proxy: Some(ProxyConfig {
                mode: "custom".into(),
                url: "http://127.0.0.1:9".into(),
            }),
            ..Default::default()
        };
        let settings = Settings {
            proxy: ProxyConfig {
                mode: if index % 2 == 0 { "direct" } else { "custom" }.into(),
                url: "http://127.0.0.1:9".into(),
            },
            hosts: vec![Host {
                id: "quota-fixture".into(),
                target: "quota-fixture.invalid".into(),
                ..Default::default()
            }],
            accounts: vec![account.clone()],
            ..Default::default()
        };
        let store = crate::store::Store::open(&home.join("store")).unwrap();
        if matches!(
            provider,
            "cline" | "kilo" | "kimi" | "pi" | "omp" | "copilot"
        ) {
            let mut discovered = vec![candidate.clone()];
            probe_scan_identities(&store, &settings, &mut discovered);
            assert_eq!(
                discovered[0].connection.identity_key, row.identity_key,
                "remote first discovery: {provider}"
            );
        }
        let mut visible = settings.clone();
        let mut registered = row.clone();
        registered.account_key = Some(accounts::key(&account));
        registered.machine_fingerprint =
            hash(&host_signature(&visible, &row.machine_id).to_string());
        visible.native_connections.push(registered);
        visible.agents = vec![AgentConfiguration {
            provider: provider.into(),
            enabled: true,
            machine_ids: vec![row.machine_id.clone()],
        }];
        assert!(
            quota_sources(&visible, &account)
                .iter()
                .any(|s| s.id == source.id),
            "remote quota target: {provider}"
        );
        store
            .db
            .execute(
                "INSERT INTO kv VALUES(?1,?2)",
                rusqlite::params![
                    format!("native-fingerprint:{}", row.id),
                    candidate.fingerprint
                ],
            )
            .unwrap();
        let remote_snapshot = read(&store, &settings, &account, &source, &row)
            .unwrap_or_else(|error| panic!("{provider}: {error}"));
        let send = |mut request: crate::native_quota_transport::Request| {
            let url = reqwest::Url::parse(&request.url).unwrap();
            request.url = format!(
                "{base}{}{}",
                url.path(),
                url.query().map(|q| format!("?{q}")).unwrap_or_default()
            );
            request.send(&ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            })
        };
        let mut local_snapshot = if matches!(provider, "pi" | "claude") {
            let value = crate::upstream::usage_using(&candidate.secret, send).unwrap();
            let mut upstream = source.clone();
            upstream.provider = "claude".into();
            crate::quota::normalize(&upstream, &value, "live").unwrap()
        } else if provider == "omp" {
            let mut upstream = source.clone();
            upstream.provider = "copilot".into();
            upstream.subscription = Some(crate::subscriptions::SubscriptionConfig {
                mode: "github".into(),
                ..Default::default()
            });
            crate::subscriptions::read_native_using(&upstream, &candidate.secret, send).unwrap()
        } else {
            crate::subscriptions::read_native_using(&source, &candidate.secret, send).unwrap()
        };
        local_snapshot.provider = provider.into();
        if local_snapshot.plan.is_none() && !candidate.plan.is_empty() {
            local_snapshot.plan = Some(candidate.plan);
        }
        let canonical = |snapshot: QuotaSnapshot| {
            let mut value = serde_json::to_value(snapshot).unwrap();
            value.as_object_mut().unwrap().remove("updatedAt");
            if let Some(identity) = value["identity"].as_object_mut() {
                identity.remove("checkedAt");
            }
            value
        };
        assert_eq!(
            canonical(remote_snapshot),
            canonical(local_snapshot),
            "{provider}"
        );
    }
    copilot_environment_routes(root, &remote_environment);
    copilot_scan_identity_lifecycle(root);
    native_codex_routes(root);
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    let captured = captured.lock().unwrap();
    let requests = |prefix: &'static str| {
        captured
            .iter()
            .filter(move |(path, _)| path.starts_with(prefix))
    };
    for (_, headers) in requests("/api/v0/users/get_user_summary") {
        assert!(headers.contains_key("x-dsh-auth-token"));
        assert!(!headers.contains_key("authorization"));
    }
    for (_, headers) in requests("/api/monitor/usage/quota/limit") {
        assert!(!headers["authorization"].starts_with("Bearer "));
    }
    for (_, headers) in requests("/api/v1/users/") {
        assert!(headers["authorization"].starts_with("Bearer workos:"));
    }
    assert!(requests("/api/profile/balance").any(|(_, headers)| {
        headers.get("x-kilocode-organizationid").map(String::as_str) == Some("org-1")
    }));
    assert!(
        requests("/copilot_internal/user")
            .all(|(_, headers)| headers["x-github-api-version"] == "2025-04-01")
    );
    assert!(captured.iter().all(|(_, headers)| {
        !headers
            .values()
            .any(|value| value.contains("DESKTOP_PRIVATE"))
    }));
    for token in [
        "gho_REMOTE_ENV_HIGH",
        "gho_REMOTE_ENV_MIDDLE",
        "gho_REMOTE_ENV_LOW",
    ] {
        assert!(
            requests("/copilot_internal/user")
                .any(|(_, headers)| headers["authorization"] == format!("token {token}"))
        );
    }
}

fn copilot_fixture_settings(root: &Path) -> Settings {
    Settings {
        local_device_enabled: false,
        proxy: ProxyConfig {
            mode: "custom".into(),
            url: "http://127.0.0.1:9".into(),
        },
        hosts: vec![Host {
            id: "quota-fixture".into(),
            target: "quota-fixture.invalid".into(),
            ..Default::default()
        }],
        agents: vec![AgentConfiguration {
            provider: "copilot".into(),
            enabled: true,
            machine_ids: vec!["quota-fixture".into()],
        }],
        sources: vec![Source {
            id: "copilot-history".into(),
            provider: "copilot".into(),
            host_id: Some("quota-fixture".into()),
            path: root.join(".copilot").to_string_lossy().into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn scan_copilot(engine: &mut Engine) -> Value {
    engine
        .call(
            "accounts.autoScan",
            json!({"targets":[["copilot","quota-fixture"]]}),
        )
        .unwrap()
}

fn copilot_environment_routes(root: &Path, environment_file: &Path) {
    std::fs::create_dir_all(root.join(".copilot")).unwrap();
    for (index, (name, environment)) in [
        ("COPILOT_GITHUB_TOKEN", json!({"COPILOT_GITHUB_TOKEN":"gho_REMOTE_ENV_HIGH","GH_TOKEN":"gho_UNUSED_MIDDLE","GITHUB_TOKEN":"gho_UNUSED_LOW"})),
        ("GH_TOKEN", json!({"GH_TOKEN":"gho_REMOTE_ENV_MIDDLE","GITHUB_TOKEN":"gho_UNUSED_LOW"})),
        ("GITHUB_TOKEN", json!({"GITHUB_TOKEN":"gho_REMOTE_ENV_LOW"})),
    ].into_iter().enumerate() {
        std::fs::write(environment_file, environment.to_string()).unwrap();
        let mut engine = Engine::open(&root.join(format!("copilot-env-store-{index}"))).unwrap();
        engine.store.save_auth_settings(&copilot_fixture_settings(root)).unwrap();
        let result = scan_copilot(&mut engine);
        assert!(!result.to_string().contains("gho_REMOTE"));
        assert!(!result.to_string().contains("DESKTOP_PRIVATE"));
        let settings = engine.store.settings().unwrap();
        assert_eq!(settings.accounts.len(), 1, "remote env discovery {name}: {result}");
        assert_eq!(settings.accounts[0].upstream_identity.as_deref(), Some("github:7"));
        assert_eq!(settings.native_connections[0].config_path, format!("env:{name}"));
        let sources = quota_sources(&settings, &settings.accounts[0]);
        assert_eq!(sources.len(), 1);
        let snapshot = read_native_quota(&engine.store, &settings, &settings.accounts[0], &sources[0]).unwrap();
        assert_eq!(snapshot.identity.unwrap().key, "copilot:7");
        assert_eq!(snapshot.meters[0].remaining, Some(250.));
        // Removing the remote override must not borrow the desktop's token.
        std::fs::write(environment_file, "{}").unwrap();
        assert!(read_native_quota(&engine.store, &settings, &settings.accounts[0], &sources[0]).is_err());
        let removed = scan_copilot(&mut engine);
        assert_eq!(removed["settings"]["nativeConnections"][0]["status"], "remembered");
    }
}

fn copilot_scan_identity_lifecycle(root: &Path) {
    let native = root.join(".copilot");
    let value = |token: &str| json!({"copilot_tokens":{"https://github.com:former-name":token}});
    let mut candidate = write_candidate(
        &native,
        "copilot",
        "config.json",
        value("gho_fixture-native-token"),
        "/copilot_tokens/https:~1~1github.com:former-name",
    );
    let legacy_identity = crate::subscriptions::copilot_identity("former-name");
    candidate.connection.identity_key = Some(legacy_identity.clone());
    candidate.connection.upstream_identity = Some(format!("github:{}", hash("former-name")));
    let mut settings = copilot_fixture_settings(root);
    let account = Account {
        id: "legacy-account-preserved".into(),
        name: "User chosen account label".into(),
        provider: "copilot".into(),
        identity_key: Some(legacy_identity),
        upstream_identity: candidate.connection.upstream_identity.clone(),
        quota_enabled: true,
        ..Default::default()
    };
    candidate.connection.account_key = Some(accounts::key(&account));
    candidate.connection.machine_fingerprint =
        hash(&host_signature(&settings, "quota-fixture").to_string());
    settings.sources[0].account_id = account.id.clone();
    settings.accounts.push(account.clone());
    settings
        .native_connections
        .push(candidate.connection.clone());
    let mut engine = Engine::open(&root.join("copilot-identity-lifecycle")).unwrap();
    engine.store.save_auth_settings(&settings).unwrap();
    engine
        .store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2)",
            rusqlite::params![
                format!("native-fingerprint:{}", candidate.connection.id),
                candidate.fingerprint
            ],
        )
        .unwrap();
    let old_snapshot = QuotaSnapshot {
        provider: "copilot".into(),
        account_id: account.id.clone(),
        updated_at: 123,
        snapshot_at: Some(123),
        meters: vec![QuotaMeter {
            id: "premium".into(),
            used: 50.,
            total: Some(300.),
            remaining: Some(250.),
            unit: "requests".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    engine
        .store
        .db
        .execute(
            "INSERT INTO quotas VALUES(?1,?2)",
            rusqlite::params![
                accounts::key(&account),
                serde_json::to_string(&old_snapshot).unwrap()
            ],
        )
        .unwrap();
    let renamed = scan_copilot(&mut engine);
    assert_eq!(renamed["addedAccounts"], 0);
    assert_eq!(renamed["settings"]["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(renamed["settings"]["accounts"][0]["id"], account.id);
    assert_eq!(renamed["settings"]["accounts"][0]["name"], account.name);
    assert_eq!(
        renamed["settings"]["accounts"][0]["identityKey"],
        "copilot:7"
    );
    assert_eq!(
        renamed["settings"]["nativeConnections"][0]["email"],
        "fixture"
    );
    assert_eq!(renamed["settings"]["sources"][0]["accountId"], account.id);
    assert_eq!(
        last_snapshot(&engine.store, &account).unwrap().snapshot_at,
        Some(123)
    );
    // The new token reports the SAME login but a DIFFERENT server ID. Keep
    // the old account and its usage; never migrate by the reused username.
    std::fs::write(
        native.join("config.json"),
        value("gho_fixture-reused-login").to_string(),
    )
    .unwrap();
    let replaced = scan_copilot(&mut engine);
    assert_eq!(replaced["addedAccounts"], 1);
    let settings = engine.store.settings().unwrap();
    assert_eq!(settings.accounts.len(), 2);
    let old = settings
        .accounts
        .iter()
        .find(|a| a.identity_key.as_deref() == Some("copilot:7"))
        .unwrap();
    let new = settings
        .accounts
        .iter()
        .find(|a| a.identity_key.as_deref() == Some("copilot:8"))
        .unwrap();
    assert_eq!(old.id, account.id);
    assert_ne!(old.id, new.id);
    assert!(
        settings
            .native_connections
            .iter()
            .any(|r| r.identity_key.as_deref() == Some("copilot:7") && r.status == "remembered")
    );
    assert_eq!(settings.sources[0].account_id, old.id);
    assert_eq!(
        last_snapshot(&engine.store, old).unwrap().meters[0].remaining,
        Some(250.)
    );
    let sources = quota_sources(&settings, new);
    let snapshot = read_native_quota(&engine.store, &settings, new, &sources[0]).unwrap();
    assert_eq!(snapshot.identity.unwrap().key, "copilot:8");
    assert_eq!(scan_copilot(&mut engine)["addedAccounts"], 0);
    std::fs::write(
        native.join("config.json"),
        value("gho_fixture-missing-id").to_string(),
    )
    .unwrap();
    let missing = scan_copilot(&mut engine);
    assert_eq!(missing["addedAccounts"], 0);
    let settings = engine.store.settings().unwrap();
    assert_eq!(settings.accounts.len(), 2);
    assert!(read_native_quota(&engine.store, &settings, new, &sources[0]).is_err());
}

fn native_codex_routes(root: &Path) {
    let home = root.join("codex-native");
    let auth = |user: &str, expires: i64| {
        json!({"tokens":{"access_token":jwt(json!({
        "https://api.openai.com/auth":{"chatgpt_user_id":user,"chatgpt_account_id":"workspace","chatgpt_plan_type":"plus"},"exp":expires})),
        "account_id":"workspace","refresh_token":"private-refresh-fixture"}})
    };
    let candidate = write_candidate(&home, "codex", "auth.json", auth("user-1", 4102444800), "");
    let row = &candidate.connection;
    let account = Account {
        id: "native-codex".into(),
        provider: "codex".into(),
        ..Default::default()
    };
    let source = Source {
        id: format!("native-{}", row.id),
        provider: "codex".into(),
        account_id: account.id.clone(),
        path: row.root.clone(),
        host_id: Some(row.machine_id.clone()),
        ..Default::default()
    };
    let settings = Settings {
        hosts: vec![Host {
            id: row.machine_id.clone(),
            target: "quota-fixture.invalid".into(),
            ..Default::default()
        }],
        accounts: vec![account.clone()],
        ..Default::default()
    };
    let store = crate::store::Store::open(&home.join("store")).unwrap();
    store
        .db
        .execute(
            "INSERT INTO kv VALUES(?1,?2)",
            rusqlite::params![
                format!("native-fingerprint:{}", row.id),
                candidate.fingerprint
            ],
        )
        .unwrap();
    let before = inspect(row).pin;
    let snapshot = read_inner_using(&store, &settings, &account, &source, row, |query| {
        assert_eq!(query.path, row.root);
        assert_eq!(query.host_id.as_deref(), Some("quota-fixture"));
        assert_eq!(query.provider, "codex");
        std::fs::write(
            home.join("auth.json"),
            auth("user-1", 4102444860).to_string(),
        )
        .unwrap();
        Ok(QuotaSnapshot {
            origin: "native-home-fixture".into(),
            ..Default::default()
        })
    })
    .unwrap();
    assert_eq!(snapshot.origin, "native-home-fixture");
    assert_ne!(
        inspect(row).pin,
        before,
        "same-user refresh must be allowed despite a changed token pin"
    );
    let error = read_inner_using(&store, &settings, &account, &source, row, |_| {
        std::fs::write(
            home.join("auth.json"),
            auth("another-user", 4102444860).to_string(),
        )
        .unwrap();
        Ok(QuotaSnapshot::default())
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "查询期间原生账户身份已变化");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = root.join("codex-upstream-fixture");
        std::fs::write(&binary, r#"#!/usr/bin/env python3
import json,os,sys,pathlib
if '--version' in sys.argv:
 print('codex-cli 0.125.0');raise SystemExit()
assert 'cli_auth_credentials_store="ephemeral"' in sys.argv
assert os.environ.get('AIEYES_INTERNAL')=='1'
assert not (pathlib.Path(os.environ['CODEX_HOME'])/'auth.json').exists()
for line in sys.stdin:
 v=json.loads(line);method=v.get('method')
 if method=='initialized':continue
 if method=='account/login/start':
  assert v['params']['type']=='chatgptAuthTokens' and 'refresh' not in json.dumps(v['params']).lower();result={}
 elif method=='account/read':result={'account':{'type':'chatgpt'},'workspaceRouting':{'chatgptAccountId':'workspace'}}
 elif method=='account/rateLimits/read':result={'rateLimits':{'primary':{'usedPercent':25,'windowDurationMins':300}}}
 else:
  assert method=='initialize';result={}
 print(json.dumps({'id':v['id'],'result':result}),flush=True)
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let home = root.join("codex-upstream");
        let candidate = write_candidate(
            &home,
            "pi",
            "auth.json",
            json!({"openai":{"type":"oauth","access":auth("user-1", 4102444800)["tokens"]["access_token"],
            "accountId":"workspace","refresh":"private-refresh-fixture"}}),
            "/openai",
        );
        let row = &candidate.connection;
        let account = Account {
            id: "upstream-codex".into(),
            provider: "pi".into(),
            ..Default::default()
        };
        let source = Source {
            id: format!("native-{}", row.id),
            provider: "pi".into(),
            account_id: account.id.clone(),
            path: row.root.clone(),
            host_id: Some(row.machine_id.clone()),
            codex_binary: binary.to_string_lossy().into(),
            ..Default::default()
        };
        store
            .db
            .execute(
                "INSERT INTO kv VALUES(?1,?2)",
                rusqlite::params![
                    format!("native-fingerprint:{}", row.id),
                    candidate.fingerprint
                ],
            )
            .unwrap();
        let result = read_inner_using(&store, &settings, &account, &source, row, |_| {
            panic!("upstream login must not use the native HOME reader")
        })
        .unwrap();
        assert_eq!(result.provider, "pi");
        assert!(!result.windows.is_empty());
    }
}

fn fixture_response(
    path: &str,
    headers: &std::collections::BTreeMap<String, String>,
) -> (u16, Value) {
    (
        200,
        match path {
            "/user"
                if headers
                    .get("authorization")
                    .is_some_and(|v| v.ends_with("gho_fixture-reused-login")) =>
            {
                json!({"id":8,"login":"fixture"})
            }
            "/user"
                if headers
                    .get("authorization")
                    .is_some_and(|v| v.ends_with("gho_fixture-missing-id")) =>
            {
                json!({"login":"fixture"})
            }
            "/user" => json!({"id":7,"login":"fixture"}),
            "/copilot_internal/user" => {
                json!({"login":"fixture","copilot_plan":"pro","quota_snapshots":{"premium_interactions":{"entitlement":300,"remaining":250}}})
            }
            "/api/oauth/profile" => {
                json!({"account":{"uuid":"user-1"},"organization":{"uuid":"org-1"}})
            }
            "/api/oauth/usage" => {
                json!({"five_hour":{"utilization":25},"seven_day":{"utilization":40}})
            }
            "/coding/v1/me" => json!({"user_id":"user-1","email":"fixture@example.invalid"}),
            "/coding/v1/usages" => {
                json!({"usages":{"limit_5h":{"used_ratio":0.25,"reset_time":4102444800_i64}}})
            }
            "/api/v1/users/me" => {
                json!({"success":true,"data":{"id":"user-1","email":"fixture@example.invalid","organizations":[{"organizationId":"org-1","name":"Team"},{"organizationId":"org-failed","name":"Unavailable"}]}})
            }
            "/api/v1/users/user-1/balance" => json!({"success":true,"data":{"balance":12}}),
            "/api/v1/organizations/org-1/balance" => json!({"success":true,"data":{"balance":40}}),
            "/api/v1/organizations/org-failed/balance" => return (503, json!({"error":TOKEN})),
            "/api/profile" => {
                json!({"user":{"id":"user-1","email":"fixture@example.invalid"},"hasPersonalAccount":true,"organizations":[{"id":"org-1","name":"Team"}]})
            }
            "/api/profile/balance" => {
                json!({"balance":if headers.contains_key("x-kilocode-organizationid") { 40 } else { 12 }})
            }
            "/api/v0/users/get_user_summary" => {
                json!({"code":0,"data":{"biz_code":0,"biz_data":{"normal_wallets":[{"currency":"CNY","balance":"12.5"}],"bonus_wallets":[]}}})
            }
            "/api/monitor/usage/quota/limit" => {
                json!({"data":{"planName":"Fixture","limits":[{"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":25,"nextResetTime":4102444800000_i64}]}})
            }
            "/aiserver.v1.DashboardService/GetCurrentPeriodUsage" => {
                json!({"planUsage":{"totalPercentUsed":25}})
            }
            "/v1/billing?format=credits" => {
                json!({"config":{"creditUsagePercent":25,"subscriptionTier":"Fixture"}})
            }
            "/zen/go/v1/usage" => {
                json!({"plan":"Go","usage":{"rolling":{"percent":25,"resetAt":4102444800_i64}}})
            }
            "/" => {
                json!({"usageBreakdownList":[{"resourceType":"CREDIT","currentUsage":25,"usageLimit":100}],"subscriptionInfo":{"subscriptionTitle":"KIRO Fixture"}})
            }
            "/trae/api/v1/pay/ide_user_ent_usage" => {
                json!({"data":{"is_dollar_usage_billing":true,"user_entitlement_pack_list":[
            {"status":1,"entitlement_base_info":{"product_type":1,"quota":{"basic_usage_limit":100}},"usage":{"basic_usage_amount":25}}]}})
            }
            _ => return (404, json!({})),
        },
    )
}
