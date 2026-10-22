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

fn command(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
) -> Result<std::process::Command> {
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
    Ok(command)
}

#[derive(Debug)]
struct QueryFailure {
    message: &'static str,
    retry: bool,
}

fn control_frame(kind: &str, id: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"type":"control_request","request_id":id,"request":{"type":kind,"subtype":kind}}),
    )
    .unwrap()
}

fn await_response(
    process: &crate::process::LineProcess,
    id: &str,
    deadline: std::time::Instant,
    size: &mut usize,
) -> std::result::Result<Value, QueryFailure> {
    let initializing = id == "aieyes-init";
    loop {
        let line = process
            .read(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| QueryFailure {
                message: match (initializing, std::time::Instant::now() >= deadline) {
                    (true, true) => "Qoder CLI 初始化超时",
                    (false, true) => "Qoder CLI 额度查询超时",
                    (true, false) => "Qoder CLI 连接中断，未收到初始化响应",
                    (false, false) => "Qoder CLI 连接中断，未收到额度响应",
                },
                retry: true,
            })?;
        *size += line.len();
        if *size > 2 * 1024 * 1024 {
            return Err(QueryFailure {
                message: "Qoder 额度响应过大",
                retry: false,
            });
        }
        let Ok(frame) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if frame["type"] != "control_response" || frame["response"]["request_id"] != id {
            continue;
        }
        let response = &frame["response"];
        if response["subtype"] != "success" {
            let code = response["error"]["code"].as_str().unwrap_or("");
            return Err(QueryFailure {
                message: if initializing {
                    "Qoder CLI 初始化失败，请检查登录"
                } else {
                    "Qoder CLI 未返回额度，请检查登录或客户端版本"
                },
                retry: matches!(
                    code,
                    "temporarily_unavailable" | "quota_unavailable" | "network_error" | "timeout"
                ),
            });
        }
        return Ok(response["response"].clone());
    }
}

fn exchange(
    command: std::process::Command,
    deadline: std::time::Instant,
) -> std::result::Result<Value, QueryFailure> {
    let process = crate::process::LineProcess::spawn(command).map_err(|_| QueryFailure {
        message: "Qoder CLI 启动失败，请检查安装和配置目录",
        retry: false,
    })?;
    let mut size = 0;
    process
        .send(control_frame("initialize", "aieyes-init"))
        .map_err(|_| QueryFailure {
            message: "Qoder CLI 初始化连接中断",
            retry: true,
        })?;
    await_response(&process, "aieyes-init", deadline, &mut size)?;
    process
        .send(control_frame("get_usage_info", "aieyes-usage"))
        .map_err(|_| QueryFailure {
            message: "Qoder CLI 额度查询连接中断",
            retry: true,
        })?;
    let result = await_response(&process, "aieyes-usage", deadline, &mut size)?;
    let usage = &result["usage"];
    if usage.is_null() {
        return Err(QueryFailure {
            message: "Qoder 账户额度暂不可用",
            retry: true,
        });
    }
    Ok(usage.clone())
}

fn query(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
) -> Result<Value> {
    query_with_budget(
        source,
        settings,
        config,
        pre_command,
        std::time::Duration::from_secs(45),
    )
}

fn query_with_budget(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
    budget: std::time::Duration,
) -> Result<Value> {
    query_attempts(|| command(source, settings, config, pre_command), budget)
}

fn query_attempts(
    mut command: impl FnMut() -> Result<std::process::Command>,
    budget: std::time::Duration,
) -> Result<Value> {
    let started = std::time::Instant::now();
    let deadline = started + budget;
    for attempt in 0..2 {
        crate::operations::check()?;
        let command = command()?;
        let until = if attempt == 0 {
            started + budget / 2
        } else {
            deadline
        };
        match exchange(command, until) {
            Ok(value) => return Ok(value),
            Err(error) if !error.retry || attempt == 1 || std::time::Instant::now() >= deadline => {
                anyhow::bail!(error.message)
            }
            Err(_) => {}
        }
    }
    unreachable!()
}

#[cfg(test)]
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

    #[cfg(unix)]
    fn scripted_cli(tail: &str) -> std::process::Command {
        let mut command = std::process::Command::new("python3");
        command.args(["-u", "-c", &format!(r#"
import json,sys,time,select
init=json.loads(sys.stdin.readline())
assert init['request']['type']=='initialize'
# A request before the initialization response is a protocol violation.
assert not select.select([sys.stdin],[],[],.04)[0]
print(json.dumps(dict(type='control_response',response=dict(request_id='aieyes-init',subtype='success',response={{}}))))
request=json.loads(sys.stdin.readline())
assert request['request']['type']=='get_usage_info'
{tail}
"#)]);
        command
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_initialization_and_keeps_input_open_until_matched_usage() {
        let usage = json!({"userId":"fixture","userQuota":{"unit":"credits","total":0,"used":0,"remaining":0}});
        let tail = format!(
            "print('{}')\nprint('{}')\nassert not select.select([sys.stdin],[],[],.03)[0]\ntime.sleep(30)",
            json!({"type":"control_response","response":{"request_id":"other","subtype":"success","response":{"usage":{"wrong":true}}}}),
            json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":usage}}})
        );
        let started = std::time::Instant::now();
        let result = exchange(
            scripted_cli(&tail),
            started + std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(result, usage);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let mut snapshot = QuotaSnapshot::default();
        apply(&mut snapshot, &result).unwrap();
        assert_eq!(snapshot.is_available, Some(false));
        assert_eq!(snapshot.windows.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn retries_only_once_for_eof_or_missing_usage_and_never_for_login_errors() {
        for (tail, expected) in [
            ("sys.exit(0)", 2),
            (
                "print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"session\":{\"credits\":999}}}}')",
                2,
            ),
            (
                "print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":{\"code\":\"unauthorized\",\"message\":\"PRIVATE\"}}}')",
                1,
            ),
        ] {
            let mut attempts = 0;
            let error = query_attempts(
                || {
                    attempts += 1;
                    Ok(scripted_cli(tail))
                },
                std::time::Duration::from_secs(2),
            )
            .unwrap_err();
            assert_eq!(attempts, expected);
            assert!(!error.to_string().contains("PRIVATE"));
        }
        let mut attempts = 0;
        let result=query_attempts(|| {
            attempts+=1;
            Ok(scripted_cli(if attempts==1 {"sys.exit(0)"} else {"print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"usage\":{\"userId\":\"fixture\"}}}}')"}))
        },std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(result["userId"], "fixture");
    }

    #[cfg(unix)]
    #[test]
    fn total_deadline_includes_both_attempts_and_hung_initialization() {
        let mut attempts = 0;
        let started = std::time::Instant::now();
        let error = query_attempts(
            || {
                attempts += 1;
                let mut command = std::process::Command::new("python3");
                command.args(["-c", "import time;time.sleep(30)"]);
                Ok(command)
            },
            std::time::Duration::from_millis(400),
        )
        .unwrap_err();
        assert_eq!(attempts, 2);
        assert!(error.to_string().contains("初始化超时"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

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
IFS= read -r init
printf '%s\n' "$init" > "$config/requests.jsonl"
printf '%s\n' '{"type":"control_response","response":{"request_id":"aieyes-init","subtype":"success","response":{}}}'
IFS= read -r usage
printf '%s\n' "$usage" >> "$config/requests.jsonl"
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
    fn failed_refresh_retains_last_successful_quota_and_timestamp() {
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let profile = root.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let usage = json!({"userId":"fixture","userType":"personal_standard",
            "userQuota":{"unit":"credits","total":100,"used":25,"remaining":75}});
        let success = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":usage}}});
        std::fs::write(profile.join("response.jsonl"), format!("{success}\n")).unwrap();
        let mut engine = Engine::open(&root.path().join("data")).unwrap();
        engine
            .store
            .save_auth_settings(&Settings {
                accounts: vec![Account {
                    id: "a".into(),
                    provider: "qoder".into(),
                    name: "Quota fixture".into(),
                    quota_enabled: true,
                    identity_key: Some(identity(&usage).unwrap().key),
                    ..Default::default()
                }],
                sources: vec![Source {
                    id: "quota".into(),
                    provider: "qoder".into(),
                    account_id: "a".into(),
                    subscription: Some(SubscriptionConfig {
                        mode: "cli".into(),
                        native_config_dir: profile.to_string_lossy().into(),
                        ..Default::default()
                    }),
                    quota_pre_command: pre_command,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        let params = json!({"accountKey":"qoder:a","sourceId":"quota"});
        engine.call("quotas.refresh", params.clone()).unwrap();
        let before = engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .quotas
            .remove(0);
        assert!(before.error.is_none());
        assert!(before.updated_at > 0);
        assert_eq!(before.windows.len(), 1);
        let failure = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"error","error":{"code":"unauthorized","message":"PRIVATE_CREDENTIAL"}}});
        std::fs::write(profile.join("response.jsonl"), format!("{failure}\n")).unwrap();
        engine.call("quotas.refresh", params).unwrap();
        let after = engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .quotas
            .remove(0);
        assert!(after.error.is_some());
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(
            serde_json::to_value(&after.windows).unwrap(),
            serde_json::to_value(&before.windows).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&after.balances).unwrap(),
            serde_json::to_value(&before.balances).unwrap()
        );
        assert!(
            !serde_json::to_string(&after)
                .unwrap()
                .contains("PRIVATE_CREDENTIAL")
        );
        assert_eq!(
            std::fs::read_to_string(profile.join("queries.txt"))
                .unwrap()
                .lines()
                .count(),
            2,
            "login failures must not retry"
        );
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
