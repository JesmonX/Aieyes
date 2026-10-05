use crate::{
    models::*,
    network, process, ssh,
    store::{expand, home},
    usage,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
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
                "/bin/sh"
            });
            if cfg!(windows) {
                c.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &source.quota_command,
                ]);
            } else {
                // dash exits with status 2 on bash-only profile scripts, so the local
                // shell also runs without login profiles, with a fallback PATH.
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
    let mut cmd = if let Some(id) = &source.host_id {
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
                "export CODEX_HOME={}\nexec {} app-server --stdio",
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
        c.args(["app-server", "--stdio"]);
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
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    process::prepare(&mut cmd);
    let mut child = cmd.spawn().context("启动 Codex 查询失败")?;
    let result = (|| -> Result<Value> {
        let mut stdin = child.stdin.take().context("打开 Codex 输入失败")?;
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
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let v = rx.recv_timeout(remaining).map_err(|e| match e {
                    mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!("Codex 查询超时"),
                    mpsc::RecvTimeoutError::Disconnected => anyhow::anyhow!("Codex 查询进程已退出"),
                })?;
                if v["id"] == id {
                    if v.get("error").is_some() {
                        anyhow::bail!("Codex 限额读取失败");
                    }
                    return v.get("result").cloned().context("Codex 返回数据为空");
                }
            }
        };
        wait(1)?;
        writeln!(stdin, "{}", json!({"method":"initialized"}))?;
        writeln!(stdin, "{}", json!({"id":2,"method":method,"params":params}))?;
        stdin.flush()?;
        wait(2)
    })();
    process::kill(&mut child);
    result
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
