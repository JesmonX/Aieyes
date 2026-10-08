use crate::{
    models::*,
    network, process, ssh,
    store::{expand, home},
    usage,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub fn read(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    anyhow::ensure!(!source.account_id.is_empty(), "此数据源未关联账户");
    if !source.quota_command.trim().is_empty() {
        let mut cmd = if let Some(id) = &source.host_id {
            let host = settings
                .hosts
                .iter()
                .find(|h| &h.id == id)
                .context("主机不存在")?;
            ssh::command(&quota_host(host, source), &source.quota_command)?
        } else {
            let mut c = Command::new(if cfg!(windows) {
                "powershell"
            } else {
                ssh::DEFAULT_SHELL
            });
            if cfg!(windows) {
                c.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &source.quota_command,
                ]);
            } else {
                // Run local commands without login profiles, with a fallback PATH.
                c.args([
                    "-c",
                    &format!("{}{}", ssh::PATH_FALLBACK, source.quota_command),
                ]);
            }
            network::apply_env(&mut c, source.proxy.as_ref().unwrap_or(&settings.proxy));
            c
        };
        cmd.stderr(Stdio::null());
        let data = process::run(cmd, vec![], Duration::from_secs(40))?;
        let v: Value = serde_json::from_slice(
            data.split(|b| *b == b'\n')
                .rev()
                .find(|s| !s.is_empty())
                .context("查询没有返回数据")?,
        )?;
        return normalize(source, &v, "command");
    }
    match source.provider.as_str() {
        "agy" => normalize(source, &agy(source, settings)?, "live"),
        "deepseek" => deepseek(source, settings),
        "codex" => {
            let v = codex(source, settings)?;
            normalize(source, &v, "live")
        }
        "claude" if source.host_id.is_some() => {
            let host = settings
                .hosts
                .iter()
                .find(|h| Some(&h.id) == source.host_id.as_ref())
                .context("主机不存在")?;
            let v = ssh::python(
                &quota_host(host, source),
                include_str!("../../../scripts/remote_quota.py"),
                std::slice::from_ref(&source.path),
            )?;
            if let Some(error) = v["error"].as_str() {
                anyhow::bail!("{error}");
            }
            normalize(source, &v, "live")
        }
        "claude" => {
            let root = expand(&source.path);
            let credentials = root.join(".credentials.json");
            let data = if credentials.exists() {
                std::fs::read(credentials)?
            } else {
                #[cfg(target_os = "macos")]
                {
                    anyhow::ensure!(
                        root == home().join(".claude"),
                        "未找到此目录的 Claude Code 登录信息"
                    );
                    let mut cmd = Command::new("/usr/bin/security");
                    cmd.args([
                        "find-generic-password",
                        "-s",
                        "Claude Code-credentials",
                        "-w",
                    ]);
                    process::run(cmd, vec![], Duration::from_secs(5))
                        .context("未找到 Claude Code 登录信息")?
                }
                #[cfg(not(target_os = "macos"))]
                {
                    anyhow::bail!("未找到 Claude Code 登录信息");
                }
            };
            let credentials: Value =
                serde_json::from_slice(&data).context("读取 Claude Code 登录信息失败")?;
            let token = credentials["claudeAiOauth"]["accessToken"]
                .as_str()
                .context("未找到 Claude Code 登录信息")?;
            let response = network::client(source.proxy.as_ref().unwrap_or(&settings.proxy))?
                .get("https://api.anthropic.com/api/oauth/usage")
                .bearer_auth(token)
                .header("anthropic-beta", "oauth-2025-04-20")
                .send()
                .map_err(|_| anyhow::anyhow!("限额连接失败"))?;
            anyhow::ensure!(
                response.status().is_success(),
                "限额查询失败（HTTP {}）",
                response.status().as_u16()
            );
            normalize(source, &response.json::<Value>()?, "live")
        }
        _ => anyhow::bail!("此 Agent 暂未接入自动限额查询"),
    }
}

/// A source override only applies to provider requests; history and metrics keep the host setup.
pub fn quota_host(host: &Host, source: &Source) -> Host {
    let mut host = host.clone();
    if !source.quota_pre_command.trim().is_empty() {
        host.pre_command = source.quota_pre_command.clone();
    }
    host
}

pub fn normalize(source: &Source, v: &Value, origin: &str) -> Result<QuotaSnapshot> {
    if source.provider == "codex"
        && !v.get("windows").is_some_and(Value::is_array)
        && (v.get("rateLimits").is_some()
            || v.get("primary").is_some()
            || v.get("rateLimitsByLimitId").is_some()
            || v.get("credits").is_some())
    {
        return Ok(usage::codex_quota(source, v, now(), origin));
    }
    let mut q = QuotaSnapshot {
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: source.provider.clone(),
        name: source.name.clone(),
        updated_at: now(),
        origin: origin.into(),
        ..Default::default()
    };
    if let Some(windows) = v["windows"].as_array() {
        q.windows = windows
            .iter()
            .map(|w| serde_json::from_value(w.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        q.credits = v
            .get("credits")
            .filter(|v| v.is_object())
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        q.credits_updated_at = q.credits.as_ref().map(|_| now());
        q.credits_origin = q.credits.as_ref().map(|_| origin.into());
        q.bank_reset = v.get("bankReset").filter(|v| v.is_object()).cloned();
    } else if source.provider == "agy" {
        parse_agy(&mut q, v)?;
    } else if source.provider == "deepseek" {
        q.is_available = Some(
            v["is_available"]
                .as_bool()
                .context("DeepSeek 余额状态格式不正确")?,
        );
        for b in v["balance_infos"]
            .as_array()
            .context("DeepSeek 未返回余额")?
        {
            let amount = |key: &str| -> Result<String> {
                let value = b[key].as_str().context("DeepSeek 余额格式不正确")?;
                anyhow::ensure!(
                    value.parse::<f64>().is_ok_and(|n| n.is_finite()),
                    "DeepSeek 余额格式不正确"
                );
                Ok(value.into())
            };
            q.balances.push(Balance {
                currency: b["currency"].as_str().context("余额币种缺失")?.into(),
                total: amount("total_balance")?,
                granted: amount("granted_balance")?,
                topped_up: amount("topped_up_balance")?,
            });
        }
    } else if source.provider == "claude" {
        for (key, label, minutes) in [
            ("five_hour", "5h", 300),
            ("seven_day", "7d", 10080),
            ("seven_day_sonnet", "Sonnet · 7d", 10080),
            ("seven_day_opus", "Opus · 7d", 10080),
            ("seven_day_oauth_apps", "OAuth · 7d", 10080),
        ] {
            if let Some(used) = v[key]["utilization"].as_f64() {
                q.windows.push(QuotaWindow {
                    id: key.into(),
                    group_id: key.into(),
                    group_name: label.into(),
                    name: label.into(),
                    used_percent: used,
                    window_minutes: Some(minutes),
                    resets_at: timestamp(&v[key]["resets_at"]),
                });
            }
        }
    }
    q.bank_updated_at = q.bank_reset.as_ref().map(|_| now());
    anyhow::ensure!(
        !q.windows.is_empty()
            || !q.balances.is_empty()
            || q.bank_reset.is_some()
            || q.credits.is_some(),
        "查询结果中没有限额数据"
    );
    anyhow::ensure!(
        q.windows
            .iter()
            .all(|w| w.used_percent.is_finite() && w.used_percent >= 0.0),
        "限额数值格式不正确"
    );
    Ok(q)
}

fn resolve_codex(configured: &str) -> String {
    if configured != "codex" && !configured.is_empty() {
        return expand(configured).to_string_lossy().into();
    }
    #[cfg(unix)]
    for p in [
        home().join(".local/bin/codex"),
        home().join(".codex/packages/standalone/current/bin/codex"),
        "/opt/homebrew/bin/codex".into(),
        "/usr/local/bin/codex".into(),
    ] {
        if p.is_file() {
            return p.to_string_lossy().into();
        }
    }
    "codex".into()
}

fn codex(source: &Source, settings: &Settings) -> Result<Value> {
    codex_rpc(
        source,
        settings,
        "account/rateLimits/read",
        json!({"excludeResetCreditDetails":false,"supportsLunaReserve":false}),
    )
}
pub fn codex_rpc(
    source: &Source,
    settings: &Settings,
    method: &str,
    params: Value,
) -> Result<Value> {
    let cmd = if let Some(id) = &source.host_id {
        let host = settings
            .hosts
            .iter()
            .find(|h| &h.id == id)
            .context("主机不存在")?;
        let path = source.path.trim_end_matches('/');
        let path = path
            .strip_suffix("/sessions")
            .or_else(|| path.strip_suffix("/archived_sessions"))
            .unwrap_or(path);
        let home_expr = if let Some(tail) = path.strip_prefix("~/") {
            format!("\"$HOME\"/{}", process::quote(tail))
        } else {
            if path == "~" {
                "\"$HOME\"".into()
            } else {
                process::quote(path)
            }
        };
        ssh::command(
            &quota_host(host, source),
            &format!(
                "export CODEX_HOME={}\nexec {} app-server",
                home_expr,
                process::quote(if source.codex_binary.is_empty() {
                    "codex"
                } else {
                    &source.codex_binary
                })
            ),
        )?
    } else {
        let mut c = process::cli_command(&resolve_codex(&source.codex_binary));
        // Stdio is the default. Older CLIs reject the newer --stdio alias.
        c.arg("app-server");
        let root = expand(&source.path);
        let root = if root
            .file_name()
            .is_some_and(|s| s == "sessions" || s == "archived_sessions")
        {
            root.parent().unwrap_or(&root).to_path_buf()
        } else {
            root
        };
        c.env("CODEX_HOME", root);
        network::apply_env(&mut c, source.proxy.as_ref().unwrap_or(&settings.proxy));
        c
    };
    run_codex_rpc(
        cmd,
        source.host_id.is_some(),
        method,
        params,
        Duration::from_secs(30),
    )
}

fn run_codex_rpc(
    mut cmd: Command,
    remote: bool,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value> {
    let location = if remote { "SSH" } else { "本地" };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::prepare(&mut cmd);
    let mut child = cmd.spawn().map_err(|error| {
        let reason = match error.kind() {
            std::io::ErrorKind::NotFound => "未找到可执行文件，请检查 CLI 路径和 PATH",
            std::io::ErrorKind::PermissionDenied => "没有执行权限",
            _ => "请检查 CLI 路径和运行环境",
        };
        anyhow::anyhow!(
            "启动 Codex 查询失败（{location}，系统错误 {}）：{reason}",
            error
                .raw_os_error()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "未知".into())
        )
    })?;
    let mut stderr = child.stderr.take().context("打开 Codex 错误输出失败")?;
    let (error_tx, error_rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Drain even after the capture limit, so verbose CLIs cannot deadlock.
        let mut captured = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let keep = n.min(65536usize.saturating_sub(captured.len()));
            captured.extend_from_slice(&buffer[..keep]);
        }
        let _ = error_tx.send(captured);
    });
    let mut stage = "初始化";
    // Keep stdin alive until after recording the natural exit status. Closing it
    // here would make a live server exit cleanly and disguise our own cleanup.
    let mut stdin = child.stdin.take().context("打开 Codex 输入失败")?;
    let result = (|| -> Result<Value> {
        let stdout = child.stdout.take().context("打开 Codex 输出失败")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        writeln!(
            stdin,
            "{}",
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"aieyes","title":"Aieyes","version":"0.1.0"},"capabilities":{"experimentalApi":true,"explicitGatewayOauth":true}}})
        )?;
        stdin.flush()?;
        let wait = |id: i64| -> Result<Value> {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let v = rx.recv_timeout(remaining).map_err(|e| match e {
                    mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!("Codex 查询超时"),
                    mpsc::RecvTimeoutError::Disconnected => anyhow::anyhow!("Codex 查询连接已断开"),
                })?;
                if v["id"] == id {
                    return Ok(v);
                }
            }
        };
        codex_rpc_result(wait(1)?)?;
        stage = match method {
            "account/rateLimits/read" => "读取限额",
            "account/read" => "读取账户",
            "model/list" => "读取模型",
            _ => "读取数据",
        };
        writeln!(stdin, "{}", json!({"method":"initialized"}))?;
        writeln!(stdin, "{}", json!({"id":2,"method":method,"params":params}))?;
        stdin.flush()?;
        let reply = wait(2)?;
        // Older versions model this read-only request as unit/null, while newer
        // ones accept options. Retry only this exact parameter-shape rejection.
        if method == "account/rateLimits/read"
            && params.is_object()
            && matches!(reply["error"]["code"].as_i64(), Some(-32600 | -32602))
            && reply["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("expected unit"))
        {
            writeln!(stdin, "{}", json!({"id":3,"method":method,"params":null}))?;
            stdin.flush()?;
            return codex_rpc_result(wait(3)?);
        }
        codex_rpc_result(reply)
    })();
    // EOF may arrive just before the OS publishes the exit status. Record the
    // natural status before cleanup, never the SIGKILL/taskkill we initiate.
    let mut status = child.try_wait().ok().flatten();
    if result.is_err() && status.is_none() {
        let deadline = Instant::now() + Duration::from_millis(100);
        while status.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            status = child.try_wait().ok().flatten();
        }
    }
    process::kill(&mut child);
    result.map_err(|error| {
        let stderr = error_rx
            .recv_timeout(Duration::from_millis(100))
            .unwrap_or_default();
        let reason = if let Some(reason) = codex_error_reason(&String::from_utf8_lossy(&stderr)) {
            reason.to_owned()
        } else if let Some(status) = status.filter(|s| !s.success()) {
            process::classify_error(&String::from_utf8_lossy(&stderr), status.code())
        } else {
            String::new()
        };
        let exit = status
            .map(|s| {
                s.code()
                    .map(|n| format!("，退出码 {n}"))
                    .unwrap_or_else(|| "，被信号终止".into())
            })
            .unwrap_or_default();
        let detail = if reason.is_empty() {
            String::new()
        } else {
            format!("；{reason}")
        };
        anyhow::anyhow!("Codex {stage}失败（{location}{exit}）：{error}{detail}")
    })
}

// Only fixed labels and numeric codes leave the query process. Provider errors
// and shell output can contain credentials, proxy URLs, or private config text.
fn codex_error_reason(message: &str) -> Option<&'static str> {
    let message = message.to_ascii_lowercase();
    let categories: &[(&[&str], &str)] = &[
        (
            &[
                "unexpected argument",
                "unrecognized option",
                "unknown option",
                "unrecognized subcommand",
            ],
            "Codex CLI 启动参数不兼容，请检查版本",
        ),
        (
            &[
                "command not found",
                "not recognized as an internal",
                "not recognized as the name",
            ],
            "未找到 Codex 或其运行依赖，请检查 CLI 路径和 PATH",
        ),
        (
            &[
                "error loading config",
                "failed to load config",
                "error parsing",
                "toml parse error",
            ],
            "Codex 配置解析失败，请检查所选配置目录",
        ),
        (
            &[
                "not logged in",
                "not authenticated",
                "authentication required",
                "requires chatgpt",
                "requires a chatgpt",
                "only available for chatgpt",
            ],
            "需要 ChatGPT 订阅登录，请检查所选配置目录的登录方式",
        ),
        (
            &[
                "401 unauthorized",
                "status code 401",
                "http 401",
                "token expired",
                "token has expired",
                "refresh token",
            ],
            "Codex 登录已失效，请在查询所在机器重新登录",
        ),
        (
            &["403 forbidden", "status code 403", "http 403"],
            "Codex 服务拒绝访问，请检查账户权限和查询所在机器的网络",
        ),
        (
            &["407 proxy", "proxy authentication"],
            "代理认证失败，请检查查询所在机器的代理",
        ),
        (
            &[
                "error sending request",
                "connection refused",
                "dns error",
                "failed to lookup address",
                "certificate verify",
                "invalid peer certificate",
                "connection reset",
            ],
            "Codex 服务连接失败，请检查查询所在机器的网络和代理",
        ),
        (
            &["timed out", "timeout"],
            "Codex 服务连接超时，请检查查询所在机器的网络和代理",
        ),
    ];
    categories.iter().find_map(|(patterns, label)| {
        patterns
            .iter()
            .any(|p| message.contains(p))
            .then_some(*label)
    })
}

fn codex_rpc_error(error: &Value) -> String {
    let code = error["code"].as_i64();
    let reason = match code {
        Some(-32601) => "此 Codex CLI 不支持该查询接口，请升级 CLI",
        Some(-32602) => "此 Codex CLI 不接受查询参数，请检查版本兼容性",
        _ => codex_error_reason(error["message"].as_str().unwrap_or(""))
            .unwrap_or("Codex 拒绝查询，请检查登录方式、配置目录和网络"),
    };
    format!(
        "RPC {}：{reason}",
        code.map(|c| c.to_string())
            .unwrap_or_else(|| "未知错误".into())
    )
}

fn codex_rpc_result(reply: Value) -> Result<Value> {
    if let Some(error) = reply.get("error").filter(|e| !e.is_null()) {
        anyhow::bail!(codex_rpc_error(error));
    }
    reply.get("result").cloned().context("Codex 返回数据为空")
}

#[cfg(test)]
mod codex_rpc_tests {
    use super::*;

    // Spawn this test executable as the fake CLI on every OS, including Windows.
    // No Python, account credentials, SSH connection, or real CLI is required.
    fn fixture_command(case: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--ignored",
            "--exact",
            "quota::codex_rpc_tests::fixture",
            "--nocapture",
        ]);
        command.env("AIEYES_CODEX_RPC_FIXTURE", case);
        command
    }

    #[test]
    #[ignore = "subprocess fixture"]
    fn fixture() {
        let case = std::env::var("AIEYES_CODEX_RPC_FIXTURE").unwrap();
        match case.as_str() {
            "exit" => {
                eprintln!("unexpected argument '--stdio' found; secret-token");
                std::process::exit(2);
            }
            "ssh" => {
                eprintln!("private-host: Permission denied (publickey). secret-token");
                std::process::exit(255);
            }
            "clean-exit" => std::process::exit(0),
            _ => {}
        }
        for line in std::io::stdin().lock().lines() {
            let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let Some(id) = value["id"].as_i64() else {
                continue;
            };
            let reply = match (case.as_str(), id) {
                ("init-error", 1) => {
                    json!({"id":id,"error":{"code":-32602,"message":"secret-token"}})
                }
                (_, 1) => json!({"id":1,"result":{}}),
                ("legacy", 2) => {
                    assert!(value["params"].is_object());
                    json!({"id":id,"error":{"code":-32600,"message":"Invalid request: invalid type: map, expected unit"}})
                }
                ("legacy", 3) => {
                    assert_eq!(value["method"], "account/rateLimits/read");
                    assert!(value["params"].is_null());
                    json!({"id":id,"result":{"legacy":true}})
                }
                ("auth", 2) => {
                    json!({"id":id,"error":{"code":-32600,"message":"codex account authentication required to read rate limits secret-token"}})
                }
                ("timeout", 2) => {
                    std::thread::sleep(Duration::from_secs(10));
                    continue;
                }
                ("noisy", 2) => {
                    eprint!("{}", "secret-token".repeat(12000));
                    json!({"id":id,"result":{"ok":true}})
                }
                (_, 2) => json!({"id":id,"result":{"ok":true}}),
                _ => panic!("unexpected retry"),
            };
            println!("ignored banner");
            println!("{}", json!({"method":"notification"}));
            println!("{reply}");
            std::io::stdout().flush().unwrap();
        }
    }

    fn query(case: &str, remote: bool) -> Result<Value> {
        run_codex_rpc(
            fixture_command(case),
            remote,
            "account/rateLimits/read",
            json!({"excludeResetCreditDetails":false}),
            Duration::from_secs(2),
        )
    }

    #[test]
    fn startup_exit_and_ssh_auth_errors_keep_codes_without_private_output() {
        for (case, remote, expected) in [
            ("exit", false, "启动参数不兼容"),
            ("ssh", true, "SSH 认证失败"),
        ] {
            let error = query(case, remote).unwrap_err().to_string();
            assert!(error.contains("初始化失败"), "{error}");
            assert!(
                error.contains(if remote {
                    "SSH，退出码 255"
                } else {
                    "本地，退出码 2"
                }),
                "{error}"
            );
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("secret-token") && !error.contains("private-host"));
        }
        let error = query("clean-exit", true).unwrap_err().to_string();
        assert!(error.contains("退出码 0"), "{error}");
        assert!(!error.contains("命令执行失败（0）"), "{error}");
    }

    #[test]
    fn rpc_errors_keep_stage_and_code_and_never_retry_authentication() {
        for (case, expected) in [("init-error", "初始化失败"), ("auth", "读取限额失败")]
        {
            let error = query(case, false).unwrap_err().to_string();
            assert!(
                error.contains(expected) && error.contains("RPC -3260"),
                "{error}"
            );
            assert!(
                !error.contains("secret-token") && !error.contains("退出码"),
                "{error}"
            );
            if case == "auth" {
                assert!(error.contains("ChatGPT 订阅登录"), "{error}");
            }
        }
    }

    #[test]
    fn old_cli_unit_params_fallback_and_verbose_success_are_supported() {
        assert_eq!(query("legacy", true).unwrap(), json!({"legacy":true}));
        assert_eq!(query("noisy", false).unwrap(), json!({"ok":true}));
    }

    #[test]
    fn timeout_does_not_report_our_cleanup_as_a_natural_exit() {
        let error = run_codex_rpc(
            fixture_command("timeout"),
            false,
            "account/rateLimits/read",
            json!({}),
            Duration::from_secs(2),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("读取限额失败") && error.contains("查询超时"),
            "{error}"
        );
        assert!(
            !error.contains("退出码") && !error.contains("被信号终止"),
            "{error}"
        );
    }

    #[test]
    fn errors_use_safe_categories_instead_of_raw_server_messages() {
        for (message, expected) in [
            ("HTTP 401 secret-token", "登录已失效"),
            (
                "error sending request for https://private:secret-token@host",
                "服务连接失败",
            ),
            ("TOML parse error: secret-token", "配置解析失败"),
            ("unknown secret-token", "Codex 拒绝查询"),
        ] {
            let error = codex_rpc_error(&json!({"code":-32603,"message":message}));
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("secret-token"));
        }
    }
}

fn agy(source: &Source, settings: &Settings) -> Result<Value> {
    let binary = if source.agy_binary.is_empty() {
        "agy"
    } else {
        &source.agy_binary
    };
    let mut cmd = if let Some(id) = &source.host_id {
        let host = settings
            .hosts
            .iter()
            .find(|h| &h.id == id)
            .context("主机不存在")?;
        ssh::command(
            &quota_host(host, source),
            &format!(
                "exec {} --print /usage --output-format json --print-timeout 30s",
                process::quote(binary)
            ),
        )?
    } else {
        let local = home().join(".local/bin/agy");
        let resolved = if binary == "agy" && local.is_file() {
            local
        } else {
            expand(binary)
        };
        let mut c = process::cli_command(&resolved.to_string_lossy());
        c.args([
            "--print",
            "/usage",
            "--output-format",
            "json",
            "--print-timeout",
            "30s",
        ]);
        c.current_dir(std::env::temp_dir());
        network::apply_env(&mut c, source.proxy.as_ref().unwrap_or(&settings.proxy));
        c
    };
    cmd.stderr(Stdio::null());
    let bytes = process::run(cmd, vec![], Duration::from_secs(40))
        .context("agy 查询失败，请确认已安装并登录 agy")?;
    let line = bytes
        .split(|b| *b == b'\n')
        .rev()
        .find(|l| !l.is_empty())
        .context("agy 没有返回数据")?;
    serde_json::from_slice(line).context("agy 返回格式不正确，请更新 agy")
}

fn parse_agy(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    anyhow::ensure!(
        v["status"] == "SUCCESS" && v["command"]["name"] == "usage",
        "agy 未返回额度，请在终端登录并运行 /usage"
    );
    let groups = v["command"]["data"]["groups"]
        .as_array()
        .context("agy 未返回模型组额度")?;
    for group in groups {
        for bucket in group["buckets"]
            .as_array()
            .context("agy 模型组格式不正确")?
        {
            if bucket["disabled"] == true {
                continue;
            }
            if let Some(remaining) = bucket["remaining_fraction"].as_f64() {
                anyhow::ensure!((0.0..=1.0).contains(&remaining), "agy 剩余额度格式不正确");
                let window = bucket["window"].as_str().unwrap_or("");
                let label = match window {
                    "weekly" => "7d",
                    "5h" => "5h",
                    _ => bucket["name"].as_str().unwrap_or(window),
                };
                q.windows.push(QuotaWindow {
                    id: format!(
                        "{}:{window}",
                        group["id"]
                            .as_str()
                            .or(group["name"].as_str())
                            .unwrap_or("agy")
                    ),
                    group_id: group["id"]
                        .as_str()
                        .or(group["name"].as_str())
                        .unwrap_or("agy")
                        .into(),
                    group_name: group["name"].as_str().unwrap_or("agy").into(),
                    name: format!("{} · {}", group["name"].as_str().unwrap_or("agy"), label),
                    used_percent: (1.0 - remaining) * 100.0,
                    window_minutes: match window {
                        "weekly" => Some(10080),
                        "5h" => Some(300),
                        _ => None,
                    },
                    resets_at: timestamp(&bucket["reset_time"]),
                });
            }
        }
    }
    Ok(())
}

fn deepseek(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    anyhow::ensure!(
        source.host_id.is_none(),
        "DeepSeek 余额请使用本机数据源和 API Key"
    );
    let key = if source.path.trim().is_empty() {
        std::env::var("DEEPSEEK_API_KEY").context("请在数据源中填写 DeepSeek API Key")?
    } else {
        std::fs::read_to_string(expand(&source.path)).context("无法读取 DeepSeek API Key 文件")?
    };
    let key = key.trim();
    anyhow::ensure!(
        !key.is_empty() && !key.contains(['\n', '\r']),
        "DeepSeek API Key 格式不正确"
    );
    let response = network::client(source.proxy.as_ref().unwrap_or(&settings.proxy))?
        .get("https://api.deepseek.com/user/balance")
        .bearer_auth(key)
        .send()
        .map_err(|_| anyhow::anyhow!("DeepSeek 余额连接失败"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "DeepSeek 余额查询失败（HTTP {}）",
        response.status().as_u16()
    );
    normalize(
        source,
        &response
            .json::<Value>()
            .context("DeepSeek 余额响应格式不正确")?,
        "live",
    )
}
