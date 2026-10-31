use crate::contracts::{FIVE_HOUR_MINUTES, WEEK_MINUTES};
use crate::{
    models::*,
    network, process, ssh,
    store::{expand, home},
    usage,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub fn read(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    let mut configured = source.clone();
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == source.provider && a.id == source.account_id);
    configured.quota_pre_command = crate::accounts::pre_command(settings, account, source);
    let source = &configured;
    anyhow::ensure!(!source.account_id.is_empty(), "此数据源未关联账户");
    if !source.quota_command.trim().is_empty() {
        let mut cmd = if let Some(id) = &source.host_id {
            let host = settings
                .hosts
                .iter()
                .find(|h| &h.id == id)
                .context("主机不存在")?;
            ssh::account_command(&quota_host(host, source), &source.quota_command)?
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
        "agy" | "antigravity" => {
            let before = crate::agy_identity::read_key(source, settings);
            let value = agy(source, settings)?;
            let mut q = normalize(source, &value, "live")?;
            let observation = crate::agy_identity::read(source, settings);
            anyhow::ensure!(
                observation.identity.is_some()
                    || (before.is_none() && account.is_none_or(|a| a.identity_key.is_none())),
                "无法确认此设备的 Antigravity 账户身份；未更新额度"
            );
            if let Some(identity) = &observation.identity {
                anyhow::ensure!(
                    before.as_ref().is_none_or(|key| key == &identity.key),
                    "查询期间登录账户已变化，请重新查询额度"
                );
                anyhow::ensure!(
                    crate::agy_identity::matches(account, identity),
                    "此设备当前登录 {}，与此账户身份不一致；未更新额度",
                    identity.email
                );
                q.plan = identity.subscription.clone().or(q.plan.take());
            }
            q.identity = observation.identity;
            q.metadata_error = observation
                .metadata_error
                .or_else(|| value["metadataError"].as_str().map(str::to_owned));
            Ok(q)
        }
        "deepseek" if source.subscription.is_none() => deepseek(source, settings),
        p if crate::subscriptions::supported(p) => crate::subscriptions::read(source, settings),
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
            let v = ssh::account_python(
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
            let keychain = !credentials.exists();
            let token = if !keychain {
                let credentials: Value = serde_json::from_slice(&std::fs::read(&credentials)?)
                    .context("读取 Claude Code 登录信息失败")?;
                credentials["claudeAiOauth"]["accessToken"]
                    .as_str()
                    .context("未找到 Claude Code 登录信息")?
                    .to_string()
            } else {
                // macOS keeps the login in the Keychain, read only after the one-time
                // consent (claude_keychain); the token stays in this process's memory.
                anyhow::ensure!(
                    cfg!(target_os = "macos") && crate::claude_keychain::default_dir(&root),
                    "未找到 Claude Code 登录信息"
                );
                crate::claude_keychain::query_token(settings.claude_keychain.as_ref())?
            };
            let client = network::client(source.proxy.as_ref().unwrap_or(&settings.proxy))?;
            let send = |token: &str| {
                client
                    .get("https://api.anthropic.com/api/oauth/usage")
                    .bearer_auth(token)
                    .header("anthropic-beta", "oauth-2025-04-20")
                    .send()
                    .map_err(|_| anyhow::anyhow!("额度查询连接失败"))
            };
            let mut response = send(&token)?;
            if response.status().as_u16() == 401 && keychain {
                // Wait for Claude to refresh its own login; never refresh it here.
                if let Some(refreshed) = crate::claude_keychain::unauthorized(&token) {
                    response = send(&refreshed)?;
                }
            }
            anyhow::ensure!(
                response.status().as_u16() != 401,
                "Claude Code 登录已过期，打开 Claude Code 一次即可刷新"
            );
            anyhow::ensure!(
                response.status().is_success(),
                "额度查询失败（HTTP {}）",
                response.status().as_u16()
            );
            normalize(source, &response.json::<Value>()?, "live")
        }
        _ => anyhow::bail!("此 Agent 暂未接入自动额度查询"),
    }
}

/// A source override only applies to provider requests; history and metrics keep the host setup.
pub fn quota_host(host: &Host, source: &Source) -> Host {
    let mut host = host.clone();
    host.pre_command = source.quota_pre_command.clone();
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
        provider: crate::providers::canonical(&source.provider).into(),
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
    } else if crate::providers::antigravity(&source.provider) {
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
            ("five_hour", "5h", FIVE_HOUR_MINUTES),
            ("seven_day", "7d", WEEK_MINUTES),
            ("seven_day_sonnet", "Sonnet · 7d", WEEK_MINUTES),
            ("seven_day_opus", "Opus · 7d", WEEK_MINUTES),
            ("seven_day_oauth_apps", "OAuth · 7d", WEEK_MINUTES),
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
        parse_claude_limits(&mut q, v);
        q.extra_usage = claude_extra_usage(&v["extra_usage"]);
    }
    if let Some(meters) = v.get("meters").filter(|v| v.is_array()) {
        q.meters = serde_json::from_value(meters.clone())?;
        anyhow::ensure!(q.meters.iter().all(|m| m.valid()), "计量项格式不正确");
    }
    q.snapshot_at = timestamp(&v["snapshotAt"]);
    q.stale_since = timestamp(&v["staleSince"]);
    q.notice = v["notice"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Into::into);
    q.bank_updated_at = q.bank_reset.as_ref().map(|_| now());
    anyhow::ensure!(
        !q.windows.is_empty()
            || !q.meters.is_empty()
            || !q.balances.is_empty()
            || q.bank_reset.is_some()
            || q.extra_usage.is_some()
            || q.credits.is_some(),
        "查询结果中没有额度数据"
    );
    anyhow::ensure!(
        q.windows
            .iter()
            .all(|w| w.used_percent.is_finite() && w.used_percent >= 0.0),
        "限额数值格式不正确"
    );
    Ok(q)
}

/// `limits[]` entries with `kind=="weekly_scoped"` carry per-model weekly limits.
/// Models already reported by `seven_day_opus`/`seven_day_sonnet` are not repeated.
fn parse_claude_limits(q: &mut QuotaSnapshot, v: &Value) {
    for limit in v["limits"].as_array().into_iter().flatten() {
        if limit["kind"] != "weekly_scoped" {
            continue;
        }
        let Some(model) = limit["scope"]["model"]["display_name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(used) = limit["percent"]
            .as_f64()
            .or_else(|| limit["utilization"].as_f64())
        else {
            continue;
        };
        let name = format!("{model} · 7d");
        if q.windows.iter().any(|w| w.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        q.windows.push(QuotaWindow {
            id: format!("limits:{}", model.to_ascii_lowercase()),
            group_id: format!("limits:{}", model.to_ascii_lowercase()),
            group_name: name.clone(),
            name,
            used_percent: used,
            window_minutes: Some(WEEK_MINUTES),
            resets_at: timestamp(&limit["resets_at"]),
        });
    }
}

/// `extra_usage` amounts are in US cents. A disabled wallet with nothing spent is omitted.
fn claude_extra_usage(v: &Value) -> Option<crate::models::ExtraUsageWallet> {
    if !v.is_object() {
        return None;
    }
    let cents = |key: &str| {
        v[key]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n / 100.0)
    };
    let enabled = v["is_enabled"].as_bool();
    let used = cents("used_credits");
    let limit = cents("monthly_limit");
    if enabled != Some(true) && used.is_none_or(|n| n == 0.0) {
        return None;
    }
    let money = |n: f64| format!("{n:.2}");
    Some(crate::models::ExtraUsageWallet {
        currency: "USD".into(),
        total: money(limit.or(used).unwrap_or(0.0)),
        balance: limit.zip(used).map(|(l, u)| money((l - u).max(0.0))),
        monthly_limit_enabled: enabled.map(|e| e && limit.is_some()),
        monthly_limit: limit.map(money),
        monthly_used: used.map(money),
    })
}

pub(crate) fn resolve_codex(configured: &str) -> String {
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
        ssh::account_command(
            &quota_host(host, source),
            &format!(
                "export AIEYES_INTERNAL=1\nexport CODEX_HOME={}\nexec {} app-server",
                home_expr,
                process::quote(if source.codex_binary.is_empty() {
                    "codex"
                } else {
                    &source.codex_binary
                })
            ),
        )?
    } else {
        // Stdio is the default. Older CLIs reject the newer --stdio alias.
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        let mut c = process::cli_command_with_pre_command(
            &resolve_codex(&source.codex_binary),
            &["app-server"],
            &crate::accounts::pre_command(settings, account, source),
            ssh::DEFAULT_SHELL,
        );
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
        c.env("AIEYES_INTERNAL", "1");
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
    let _registration = process::track_child(&mut child)?;
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
                crate::operations::check()?;
                let remaining = deadline.saturating_duration_since(Instant::now());
                ensure!(!remaining.is_zero(), "Codex 查询超时");
                let v = match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
                    Ok(value) => value,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        anyhow::bail!("Codex 查询连接已断开")
                    }
                };
                if v["id"] == id {
                    return Ok(v);
                }
            }
        };
        codex_rpc_result(wait(1)?)?;
        stage = match method {
            "account/rateLimits/read" => "读取额度",
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

pub(crate) fn agy(source: &Source, settings: &Settings) -> Result<Value> {
    crate::agy_identity::quota(source, settings)
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
                        "weekly" => Some(WEEK_MINUTES),
                        "5h" => Some(FIVE_HOUR_MINUTES),
                        _ => None,
                    },
                    resets_at: timestamp(&bucket["reset_time"]),
                });
            }
        }
    }
    agy_credits(q, &v["command"]["data"]);
    Ok(())
}

/// `loadCodeAssist` tier and Google One AI credits, when agy includes them.
/// Only the saved access token is used; Aieyes never refreshes it.
fn agy_credits(q: &mut QuotaSnapshot, data: &Value) {
    let assist = ["loadCodeAssist", "load_code_assist"]
        .iter()
        .map(|k| &data[*k])
        .find(|v| v.is_object())
        .unwrap_or(data);
    let tiers = ["paidTier", "paid_tier", "currentTier", "current_tier"].map(|k| &assist[k]);
    let tier = tiers.iter().find(|t| t.is_object());
    let Some(tier) = tier else {
        return;
    };
    if q.plan.is_none() {
        q.plan = tier["name"]
            .as_str()
            .or(tier["id"].as_str())
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .map(str::to_owned);
    }
    for credit in tiers
        .iter()
        .find_map(|t| {
            t["availableCredits"]
                .as_array()
                .or(t["available_credits"].as_array())
        })
        .into_iter()
        .flatten()
    {
        let amount = credit["creditAmount"]
            .as_f64()
            .or(credit["creditAmount"].as_str().and_then(|s| s.parse().ok()))
            .or(credit["credit_amount"].as_f64());
        let kind = credit["creditType"]
            .as_str()
            .or(credit["credit_type"].as_str())
            .unwrap_or("credits");
        let Some(amount) = amount.filter(|n| n.is_finite() && *n >= 0.0) else {
            continue;
        };
        let meter = QuotaMeter {
            id: format!("credits:{}", kind.to_ascii_lowercase()),
            label: "AI Credits".into(),
            used_known: Some(false),
            remaining: Some(amount),
            unit: "credits".into(),
            ..Default::default()
        };
        if meter.valid() {
            q.meters.push(meter);
        }
    }
}

fn deepseek(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    anyhow::ensure!(
        source.host_id.is_none(),
        "DeepSeek 余额请使用本机数据源和 API Key"
    );
    let key = if source.path.trim().is_empty() {
        std::env::var("DEEPSEEK_API_KEY").context("请在账户中添加 DeepSeek API Key")?
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
        for (case, expected) in [("init-error", "初始化失败"), ("auth", "读取额度失败")]
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
            error.contains("读取额度失败") && error.contains("查询超时"),
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

/// Providers whose card shows this month's locally counted credits as a meter.
pub(crate) fn local_credits(provider: &str) -> bool {
    matches!(provider, "codebuddy" | "workbuddy")
}

/// This month's credits counted from local session usage (`nativeCredits`).
/// An estimate: the server balance is not read; mirrored rows are counted once.
pub(crate) fn local_credits_meter(
    store: &crate::store::Store,
    settings: &Settings,
    account: &Account,
) -> Result<Option<QuotaMeter>> {
    use chrono::{Datelike, Local, TimeZone};
    let today = Local::now();
    let start = Local
        .with_ymd_and_hms(today.year(), today.month(), 1, 0, 0, 0)
        .earliest()
        .map_or(0, |t| t.timestamp());
    let (next_year, next_month) = if today.month() == 12 {
        (today.year() + 1, 1)
    } else {
        (today.year(), today.month() + 1)
    };
    let reset = Local
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .earliest()
        .map(|t| t.timestamp());
    let (used, requests): (Option<f64>, i64) = if account.provider == "workbuddy" {
        workbuddy_month_credits(settings, account, start)?
    } else {
        store.db.query_row(
        "SELECT SUM(json_extract(payload,'$.nativeCredits.credits')),COUNT(json_extract(payload,'$.nativeCredits.credits')) FROM events WHERE provider=?1 AND account_id=?2 AND stamp>=?3 AND json_extract(payload,'$.nativeCredits.mirrorOf') IS NULL",
        rusqlite::params![account.provider, account.id, start],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?
    };
    if requests == 0 {
        return Ok(None);
    }
    let meter = QuotaMeter {
        id: "month-credits".into(),
        label: "本月已用".into(),
        used: used.unwrap_or(0.).max(0.),
        unit: "credits".into(),
        resets_at: reset,
        estimated: true,
        ..Default::default()
    };
    Ok(meter.valid().then_some(meter))
}

/// This month's WorkBuddy Credits of the account, read from each local
/// `workbuddy.db` it is known in: the native login of the account (zero
/// configuration, also for a remembered earlier login) and directories linked
/// to it. With a native identity only that user's sessions count.
fn workbuddy_month_credits(
    settings: &Settings,
    account: &Account,
    start: i64,
) -> Result<(Option<f64>, i64)> {
    let key = crate::accounts::key(account);
    let roots = settings
        .native_connections
        .iter()
        .filter(|r| {
            r.provider == "workbuddy"
                && r.machine_id == "local"
                && r.account_key.as_deref() == Some(key.as_str())
        })
        .map(|r| r.root.clone())
        .chain(
            settings
                .sources
                .iter()
                .filter(|s| {
                    s.provider == "workbuddy"
                        && s.account_id == account.id
                        && s.subscription.is_none()
                        && s.host_id.as_deref().is_none_or(str::is_empty)
                })
                .map(|s| s.path.clone()),
        );
    let identity = account
        .identity_key
        .as_deref()
        .filter(|k| k.starts_with("workbuddy:"));
    let mut seen = std::collections::HashSet::new();
    let (mut used, mut requests) = (None::<f64>, 0);
    for root in roots {
        let Some(path) = crate::sessions::workbuddy_database(&root) else {
            continue;
        };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if !seen.insert(path.clone()) {
            continue;
        }
        // An unreadable or half-written database only hides its own share.
        let Ok((credits, count)) = crate::workbuddy::month_credits(&path, start * 1000, identity)
        else {
            continue;
        };
        if count > 0 {
            *used.get_or_insert(0.) += credits;
            requests += count;
        }
    }
    Ok((used, requests))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Source {
        Source {
            id: "claude-source".into(),
            account_id: "claude-account".into(),
            provider: "claude".into(),
            name: "Claude".into(),
            ..Default::default()
        }
    }

    #[test]
    fn claude_normalize_reads_windows_limits_and_extra_usage() {
        let raw: Value = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/claude-oauth-usage.json"
        ))
        .unwrap();
        let q = normalize(&claude(), &raw, "live").unwrap();
        let names: Vec<_> = q.windows.iter().map(|w| w.name.as_str()).collect();
        // Opus from limits[] repeats seven_day_opus; Haiku has no percent; session is not weekly_scoped.
        assert_eq!(names, ["5h", "7d", "Opus · 7d", "Sonnet · 7d"]);
        let sonnet = &q.windows[3];
        assert_eq!(sonnet.used_percent, 30.0);
        assert_eq!(sonnet.window_minutes, Some(WEEK_MINUTES));
        assert!(sonnet.resets_at.is_some());
        // Every window keeps its own group, so the card does not switch to the grouped layout.
        let groups: std::collections::BTreeSet<_> =
            q.windows.iter().map(|w| w.group_name.as_str()).collect();
        assert_eq!(groups.len(), q.windows.len());
        let wallet = q.extra_usage.unwrap();
        assert_eq!(wallet.currency, "USD");
        assert_eq!(wallet.monthly_used.as_deref(), Some("12.34"));
        assert_eq!(wallet.monthly_limit.as_deref(), Some("50.00"));
        assert_eq!(wallet.balance.as_deref(), Some("37.66"));
        assert_eq!(wallet.total, "50.00");
        assert_eq!(wallet.monthly_limit_enabled, Some(true));
    }

    #[test]
    fn claude_extra_usage_is_optional() {
        let q = normalize(
            &claude(),
            &json!({"five_hour":{"utilization":1},"extra_usage":{"is_enabled":false,"used_credits":0}}),
            "live",
        )
        .unwrap();
        assert!(q.extra_usage.is_none());
        let q = normalize(
            &claude(),
            &json!({"extra_usage":{"is_enabled":true,"used_credits":250,"monthly_limit":null}}),
            "live",
        )
        .unwrap();
        let wallet = q.extra_usage.unwrap();
        assert_eq!(wallet.monthly_used.as_deref(), Some("2.50"));
        assert_eq!(wallet.balance, None);
        assert_eq!(wallet.monthly_limit_enabled, Some(false));
        assert!(normalize(&claude(), &json!({"limits":"bad"}), "live").is_err());
    }
}

#[cfg(test)]
mod local_credit_tests {
    use super::*;

    #[test]
    fn workbuddy_month_credits_sum_local_session_usage() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(root.path()).unwrap();
        let home = root.path().join("wb");
        std::fs::create_dir(&home).unwrap();
        rusqlite::Connection::open(home.join("workbuddy.db"))
            .unwrap()
            .execute_batch(include_str!("../tests/fixtures/workbuddy/workbuddy.sql"))
            .unwrap();
        let mut account = Account {
            provider: "workbuddy".into(),
            id: "w".into(),
            ..Default::default()
        };
        let mut settings = Settings::default();
        assert!(
            local_credits_meter(&store, &settings, &account)
                .unwrap()
                .is_none()
        );
        // A directory linked by hand: every session of this month counts.
        settings.sources.push(Source {
            id: "s".into(),
            provider: "workbuddy".into(),
            account_id: "w".into(),
            path: home.to_string_lossy().into(),
            ..Default::default()
        });
        let meter = local_credits_meter(&store, &settings, &account)
            .unwrap()
            .unwrap();
        assert!((meter.used - 4.375).abs() < 1e-9, "{}", meter.used);
        assert!(meter.estimated);
        assert!(local_credits("workbuddy"));
        // Zero configuration: the detected login finds its database by itself, and
        // only that user's sessions count (an earlier login keeps its own total).
        settings.sources.clear();
        account.identity_key = Some(format!("workbuddy:{}", hash("fixture-user")));
        settings
            .native_connections
            .push(crate::account_scan::NativeConnection {
                id: "n".into(),
                provider: "workbuddy".into(),
                machine_id: "local".into(),
                root: home.to_string_lossy().into(),
                account_key: Some(crate::accounts::key(&account)),
                status: "remembered".into(),
                ..Default::default()
            });
        let meter = local_credits_meter(&store, &settings, &account)
            .unwrap()
            .unwrap();
        assert!((meter.used - 4.25).abs() < 1e-9, "{}", meter.used);
        account.identity_key = Some(format!("workbuddy:{}", hash("someone-else")));
        settings.native_connections[0].account_key = Some(crate::accounts::key(&account));
        assert!(
            local_credits_meter(&store, &settings, &account)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn codebuddy_month_credits_become_an_estimated_meter() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(root.path()).unwrap();
        let account = Account {
            provider: "codebuddy".into(),
            id: "a".into(),
            ..Default::default()
        };
        assert!(
            local_credits_meter(&store, &Settings::default(), &account)
                .unwrap()
                .is_none()
        );
        let put = |id: &str, stamp: i64, credits: Value| {
            store.db.execute(
                "INSERT INTO events(id,provider,account_id,model,stamp,payload,cost,priced_tokens) VALUES(?1,'codebuddy','a','m',?2,?3,0,0)",
                rusqlite::params![id, stamp, json!({"nativeCredits":credits}).to_string()],
            ).unwrap();
        };
        put("1", now(), json!({"credits":1.5}));
        put("2", now(), json!({"credits":2.0}));
        put("3", now(), json!({"credits":2.0,"mirrorOf":"2"}));
        put("4", 1, json!({"credits":99.0}));
        let meter = local_credits_meter(&store, &Settings::default(), &account)
            .unwrap()
            .unwrap();
        assert_eq!(meter.used, 3.5);
        assert!(meter.estimated);
        assert_eq!(meter.label, "本月已用");
        assert!(meter.resets_at.unwrap() > now());
        assert!(!local_credits("qoder"));
    }
}
