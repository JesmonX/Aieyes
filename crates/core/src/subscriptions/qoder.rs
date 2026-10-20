//! Qoder's SDK control protocol, without an Agent prompt or copied login secret.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn read(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
) -> Result<QuotaSnapshot> {
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == source.provider && a.id == source.account_id);
    let usage = query(
        source,
        settings,
        Some(config),
        &crate::accounts::pre_command(settings, account, source),
    )?;
    parse(source, config, &usage)
}

/// Inspect the login at the source's actual location, independently of quota or
/// its association with an Aieyes account. Never read identity from old logs.
pub(crate) fn inspect(
    source: &Source,
    settings: &Settings,
    pre_command: &str,
) -> Result<AccountIdentity> {
    identity(&query(
        source,
        settings,
        source.subscription.as_ref(),
        pre_command,
    )?)
}

pub(crate) fn config_dir(source: &Source, config: Option<&SubscriptionConfig>) -> Result<String> {
    let path = if let Some(config) = config {
        ensure!(
            native_cli(&source.provider, config),
            "此来源不是 Qoder CLI 登录"
        );
        if config.native_config_dir.trim().is_empty() {
            crate::provider_paths::default_path("qoder", source.host_id.is_some())?
        } else {
            config.native_config_dir.clone()
        }
    } else {
        source.path.clone()
    };
    ensure!(
        !path.trim().is_empty() && !path.chars().any(char::is_control),
        "请选择已有的 Qoder 配置目录"
    );
    if source.host_id.is_some() {
        ensure!(
            path.starts_with('/') || path == "~" || path.starts_with("~/"),
            "远程 Qoder 配置目录应为绝对路径或 ~/ 下的目录"
        );
        Ok(path)
    } else {
        let root = crate::store::expand(&path);
        ensure!(root.is_absolute(), "Qoder 配置目录应为绝对路径");
        root.to_str()
            .map(str::to_owned)
            .context("Qoder 配置目录编码无效")
    }
}

const ARGS: &[&str] = &[
    "--print",
    "--output-format",
    "stream-json",
    "--input-format",
    "stream-json",
    "--no-session-persistence",
    "--setting-sources",
    "user",
    "--tools",
    "",
    "--strict-mcp-config",
    "--mcp-config",
    "{\"mcpServers\":{}}",
];

fn remote_script(root: &str) -> String {
    use crate::process::quote;
    // Expand only a leading ~/ on the remote machine; the rest remains literal.
    let root = if root == "~" {
        "\"$HOME\"".into()
    } else if let Some(relative) = root.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote(relative))
    } else {
        quote(root)
    };
    format!(
        "cd \"$HOME\"\nif ! command -v qodercli >/dev/null 2>&1; then\n{}fi\ntest -d {root}\nexec qodercli {} --config-dir {root}",
        crate::ssh::PATH_FALLBACK,
        ARGS.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    )
}

fn query(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
) -> Result<Value> {
    let root = config_dir(source, config)?;
    let command = if let Some(host_id) = &source.host_id {
        let mut host = settings
            .hosts
            .iter()
            .find(|h| &h.id == host_id)
            .context("Qoder 所在的 SSH 机器已移除")?
            .clone();
        host.pre_command = pre_command.into();
        crate::ssh::account_command(&host, &remote_script(&root))?
    } else {
        ensure!(
            std::path::Path::new(&root).is_dir(),
            "Qoder 配置目录不存在，请先运行 qodercli login 或选择已有配置目录"
        );
        let mut args = ARGS.to_vec();
        args.extend(["--config-dir", root.as_str()]);
        // The GUI's PATH may omit ~/.local/bin. Keep the existing shell fallback.
        let mut command = crate::process::cli_command_with_pre_command(
            "qodercli",
            &args,
            if pre_command.trim().is_empty() && cfg!(unix) {
                ":"
            } else {
                pre_command
            },
            "",
        );
        command.current_dir(crate::store::home());
        network::apply_env(
            &mut command,
            source.proxy.as_ref().unwrap_or(&settings.proxy),
        );
        command
    };
    let frames = [
        ("initialize", "aieyes-init"),
        ("get_usage_info", "aieyes-usage"),
    ]
    .into_iter()
    .map(|(kind, id)| {
        json!({"type":"control_request","request_id":id,"request":{"type":kind,"subtype":kind}})
            .to_string()
            + "\n"
    })
    .collect::<String>();
    let output = crate::process::run(
        command,
        frames.into_bytes(),
        std::time::Duration::from_secs(45),
    )
    .map_err(|_| {
        anyhow::anyhow!("Qoder CLI 账户查询失败；请检查所在机器的安装、目录、登录和网络")
    })?;
    response(&output)
}

fn response(output: &[u8]) -> Result<Value> {
    ensure!(output.len() <= 2 * 1024 * 1024, "Qoder 额度响应过大");
    let mut usage = None;
    for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(frame) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if frame["type"] != "control_response" || frame["response"]["request_id"] != "aieyes-usage"
        {
            continue;
        }
        ensure!(
            usage.is_none() && frame["response"]["subtype"] == "success",
            "Qoder CLI 不支持额度查询或当前登录不可用"
        );
        usage = Some(frame["response"]["response"]["usage"].clone());
    }
    usage.context("Qoder CLI 未返回账户额度；请升级客户端并检查登录")
}

pub(super) fn check_identity(
    snapshot: &QuotaSnapshot,
    account: Option<&Account>,
    require_bound: bool,
) -> Result<()> {
    let key = &snapshot
        .identity
        .as_ref()
        .context("Qoder 未返回账户身份")?
        .key;
    let expected = account.and_then(|a| a.identity_key.as_ref());
    ensure!(
        !require_bound || expected.is_some(),
        "Qoder 账户身份尚未绑定，请重新连接 CLI 登录"
    );
    ensure!(
        expected.is_none_or(|expected| expected == key),
        "Qoder CLI 当前登录与此账户不一致；请在 CLI 切换回原账户或另建账户"
    );
    Ok(())
}

fn identity(v: &Value) -> Result<AccountIdentity> {
    let user = v["userId"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .context("Qoder 未返回账户身份；会话 Credits 不能代替账户额度")?;
    Ok(AccountIdentity {
        key: format!("qoder:{:x}", Sha256::digest(user.as_bytes())),
        subscription: label(&v["userType"]),
        checked_at: now(),
        subscription_checked_at: Some(now()),
        ..Default::default()
    })
}

pub(super) fn apply(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let identity = identity(v)?;
    q.plan = identity.subscription.clone();
    let mut any_remaining = false;
    for (field, total_field, name) in [
        ("userQuota", "total", "套餐"),
        ("addOnQuota", "total", "加购"),
        ("orgResourcePackage", "cap", "组织共享"),
    ] {
        let bucket = &v[field];
        if bucket.is_null() {
            continue;
        }
        ensure!(
            bucket["unit"]
                .as_str()
                .is_some_and(|u| u.eq_ignore_ascii_case("credits")),
            "Qoder 额度单位不支持"
        );
        let (total, used, remaining) = (
            number(&bucket[total_field])?,
            number(&bucket["used"])?,
            number(&bucket["remaining"])?,
        );
        ensure!(
            total >= 0. && used >= 0. && remaining >= 0.,
            "Qoder Credits 数值无效"
        );
        let usage = if total == 0. {
            ensure!(used == 0. && remaining == 0., "Qoder 零上限额度不一致");
            100.
        } else {
            ratio(&bucket["used"], &bucket[total_field])?
        };
        any_remaining |= remaining > 0. && bucket["available"] != false;
        window(
            q,
            field,
            &format!("{name} Credits · {used} / {total} · 剩余 {remaining}"),
            usage,
            None,
            None,
        );
    }
    ensure!(
        !q.windows.is_empty(),
        "Qoder 未返回账户额度；会话 Credits 不能代替账户额度"
    );
    q.is_available = Some(v["isQuotaExceeded"] != true && any_remaining);
    q.identity = Some(identity);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_identity_is_independent_of_quota_and_never_uses_tokens_or_logs() {
        let usage =
            json!({"userId":"account-a","userType":"personal_standard", "token":"PRIVATE_TOKEN"});
        let login = identity(&usage).unwrap();
        assert_eq!(
            login.key,
            format!("qoder:{:x}", Sha256::digest(b"account-a"))
        );
        assert_eq!(identity(&usage).unwrap().key, login.key);
        assert_ne!(
            identity(&json!({"userId":"account-b"})).unwrap().key,
            login.key
        );
        assert!(!json!(login).to_string().contains("PRIVATE_TOKEN"));
        assert!(apply(&mut QuotaSnapshot::default(), &usage).is_err());
        for value in [
            json!({"token":"PRIVATE_TOKEN"}),
            json!({"userId":" "}),
            json!({"userId":"bad\nidentity"}),
        ] {
            assert!(identity(&value).is_err());
        }
    }

    #[test]
    fn config_roots_distinguish_remote_home_and_never_default_empty_history_paths() {
        let mut source = Source {
            provider: "qoder".into(),
            path: "~/.qoder profile".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        assert_eq!(config_dir(&source, None).unwrap(), "~/.qoder profile");
        source.host_id = None;
        assert_eq!(
            config_dir(&source, None).unwrap(),
            crate::store::home()
                .join(".qoder profile")
                .to_str()
                .unwrap()
        );
        source.path.clear();
        assert!(config_dir(&source, None).is_err());
        let config = SubscriptionConfig {
            mode: "cli".into(),
            native_config_dir: crate::store::home()
                .join(".qoder custom")
                .to_string_lossy()
                .into(),
            ..Default::default()
        };
        assert_eq!(
            config_dir(&source, Some(&config)).unwrap(),
            config.native_config_dir
        );
        source.host_id = Some("remote".into());
        source.path = "relative/directory".into();
        assert!(config_dir(&source, None).is_err());
    }

    #[cfg(unix)]
    fn fixture_cli(root: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("qodercli");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
set -eu
for arg do
    if [ "${next:-}" = config ]; then config=$arg; next=; fi
    if [ "$arg" = --config-dir ]; then next=config; fi
done
cat > "$config/requests.jsonl"
printf '%s\n' "$@" > "$config/args.txt"
printf 'query\n' >> "$config/queries.txt"
cat "$config/response.jsonl"
"#,
        )
        .unwrap();
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        format!(
            "export PATH={}:\"$PATH\"",
            crate::process::quote(bin.to_str().unwrap())
        )
    }

    #[cfg(unix)]
    fn fixture_response(root: &std::path::Path, user: &str) {
        std::fs::create_dir_all(root).unwrap();
        let frame = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":{"userId":user,"userType":"personal_standard"}}}});
        std::fs::write(root.join("response.jsonl"), frame.to_string() + "\n").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_and_remote_shell_queries_preserve_literal_paths_and_control_only_frames() {
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let config_root = root.path().join("qoder ' $dollar `literal` $(literal)");
        fixture_response(&config_root, "account-a");
        let source = Source {
            provider: "qoder".into(),
            path: config_root.to_string_lossy().into(),
            ..Default::default()
        };
        let local = inspect(&source, &Settings::default(), &pre_command).unwrap();
        let frames = std::fs::read(config_root.join("requests.jsonl")).unwrap();
        let requests: Vec<Value> = String::from_utf8(frames.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["request"]["type"], "initialize");
        assert_eq!(requests[1]["request"]["type"], "get_usage_info");
        let args = std::fs::read_to_string(config_root.join("args.txt")).unwrap();
        assert!(args.contains("--no-session-persistence\n"));
        assert!(args.contains("--tools\n\n--strict-mcp-config\n"));
        assert!(args.ends_with(&format!("--config-dir\n{}\n", config_root.display())));
        for shell in ["/bin/sh", "/bin/bash"] {
            let mut command = std::process::Command::new(shell);
            command.args([
                "-c",
                &format!("set -e\n{pre_command}\n{}", remote_script(&source.path)),
            ]);
            let output =
                crate::process::run(command, frames.clone(), std::time::Duration::from_secs(5))
                    .unwrap();
            assert_eq!(
                identity(&response(&output).unwrap()).unwrap().key,
                local.key
            );
            assert_eq!(
                std::fs::read_to_string(config_root.join("args.txt")).unwrap(),
                args
            );
        }
        let script = remote_script("~/qoder ' $(literal)");
        assert!(script.contains("\"$HOME\"/'qoder '"));
        assert!(!script.contains(crate::store::home().to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn device_batch_shares_queries_compares_all_accounts_and_retains_failed_observations() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let profile_a = root.path().join("profile-a");
        let profile_b = root.path().join("profile-b");
        fixture_response(&profile_a, "account-a");
        fixture_response(&profile_b, "account-b");
        let key_a = identity(&json!({"userId":"account-a"})).unwrap().key;
        let key_b = identity(&json!({"userId":"account-b"})).unwrap().key;
        let mut engine = Engine::open(&root.path().join("data")).unwrap();
        let settings = Settings {
            accounts: vec![
                Account {
                    id: "a".into(),
                    name: "Account A".into(),
                    provider: "qoder".into(),
                    identity_key: Some(key_a.clone()),
                    ..Default::default()
                },
                Account {
                    id: "b".into(),
                    name: "Account B".into(),
                    provider: "qoder".into(),
                    identity_key: Some(key_b.clone()),
                    ..Default::default()
                },
                Account {
                    id: "unbound".into(),
                    name: "Unbound account".into(),
                    provider: "qoder".into(),
                    ..Default::default()
                },
            ],
            sources: vec![
                Source {
                    id: "local".into(),
                    name: "Local".into(),
                    provider: "qoder".into(),
                    path: profile_a.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "same-profile".into(),
                    name: "Same profile".into(),
                    provider: "qoder".into(),
                    path: profile_a.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "other-profile".into(),
                    name: "Other profile".into(),
                    provider: "qoder".into(),
                    path: profile_b.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "quota-profile".into(),
                    name: "Native quota connection".into(),
                    provider: "qoder".into(),
                    account_id: "a".into(),
                    subscription: Some(SubscriptionConfig {
                        mode: "cli".into(),
                        native_config_dir: profile_a.to_string_lossy().into(),
                        ..Default::default()
                    }),
                    quota_pre_command: pre_command,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        engine.store.save_auth_settings(&settings).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        engine.status_observation_reads = Some(reads.clone());
        let rows = engine.refresh_account_status_batch(&json!({})).unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert_eq!(rows.as_array().unwrap().len(), 12);
        for row in rows.as_array().unwrap() {
            let expected = if row["sourceId"] == "other-profile" {
                &key_b
            } else {
                &key_a
            };
            assert_eq!(row["identity"]["key"], *expected);
            assert_eq!(row["authenticated"], true);
            assert_eq!(row["identityConfirmed"], true);
            let current = if row["accountKey"] == "qoder:unbound" {
                Value::Null
            } else {
                json!(
                    row["accountKey"]
                        == if expected == &key_a {
                            "qoder:a"
                        } else {
                            "qoder:b"
                        }
                )
            };
            assert_eq!(row["current"], current);
            assert_eq!(row["credential"], current);
        }
        assert_eq!(
            engine
                .refresh_account_status_batch(&json!({"dueOnly":true}))
                .unwrap(),
            json!([])
        );
        // Switching a native login updates observations, never the bound account.
        fixture_response(&profile_a, "account-b");
        let switched = engine
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"qoder:a","sourceId":"local"}),
            )
            .unwrap();
        assert_eq!(switched["current"], false);
        assert_eq!(switched["identity"]["key"], key_b);
        std::fs::write(profile_a.join("response.jsonl"), "{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":\"PRIVATE_TOKEN\"}}\n").unwrap();
        let failed = engine
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"qoder:a","sourceId":"local"}),
            )
            .unwrap();
        assert_eq!(failed["identity"]["key"], key_b);
        assert_eq!(failed["current"], false);
        assert!(failed["error"].is_string());
        assert!(!failed.to_string().contains("PRIVATE_TOKEN"));
        assert_eq!(
            engine.store.settings().unwrap().accounts[0]
                .identity_key
                .as_ref(),
            Some(&key_a)
        );
    }

    #[test]
    fn control_response_cannot_substitute_session_or_other_request_for_account_quota() {
        let data = b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-init\",\"subtype\":\"success\",\"response\":{\"usage\":{\"wrong\":true}}}}\n{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"session\":{\"total_credits\":0}}}}\n";
        let usage = response(data).unwrap();
        assert!(apply(&mut QuotaSnapshot::default(), &usage).is_err());
        assert!(response(b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":\"PRIVATE_TOKEN\"}}\n").unwrap_err().to_string().find("PRIVATE_TOKEN").is_none());
    }
}
