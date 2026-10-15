//! Read-only subscription adapters. Dashboard endpoints are versioned observations,
//! not public API guarantees; see docs/agent-subscriptions.md.
use crate::{Engine, credentials, models::*, network};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubscriptionConfig {
    pub mode: String,
    pub credential_ref: String,
    pub workspace_id: String,
}

pub fn supported(provider: &str) -> bool {
    matches!(provider, "zcode" | "qoder" | "grok" | "cursor" | "opencode")
}

pub fn login_url(provider: &str, mode: &str) -> Result<&'static str> {
    match (provider, mode) {
        ("zcode", "global") => Ok("https://z.ai/manage-apikey/coding-plan/personal/my-plan"),
        ("zcode", "cn") => Ok("https://bigmodel.cn/coding-plan/personal/usage"),
        ("qoder", "global") => Ok("https://qoder.com/account/usage"),
        ("qoder", "cn-legacy") => Ok("https://qoder.com.cn/account/usage"),
        ("cursor", "session") => Ok("https://cursor.com/dashboard"),
        ("grok", "subscription") => Ok("https://grok.com"),
        ("grok", "api") => Ok("https://console.x.ai"),
        ("opencode", "go" | "console") => Ok("https://opencode.ai/console"),
        _ => bail!("不支持的登录类型"),
    }
}

pub(crate) fn open_login(params: &Value) -> Result<Value> {
    let url = login_url(
        params["provider"].as_str().unwrap_or(""),
        params["mode"].as_str().unwrap_or(""),
    )?;
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url);
    crate::process::run(cmd, vec![], std::time::Duration::from_secs(10))
        .context("无法打开官方登录页")?;
    Ok(json!({"opened":true}))
}

/// Explicit import copies one selected native credential; later client account
/// switches cannot silently replace the account being monitored.
pub fn local_secret(provider: &str, mode: &str, value: &Value) -> Result<String> {
    let token = match (provider, mode) {
        ("opencode", "go") => {
            ensure!(
                value["opencode-go"]["type"] == "api",
                "未找到 OpenCode Go API Key；请先在 OpenCode /connect 登录"
            );
            value["opencode-go"]["key"].as_str()
        }
        ("grok", "subscription") => {
            let entries = value.as_object().context("Grok 登录文件格式不支持")?;
            let oidc: Vec<_> = entries
                .iter()
                .filter(|(key, entry)| {
                    key.starts_with("https://auth.x.ai::")
                        && entry["key"].as_str().is_some_and(|s| !s.is_empty())
                })
                .map(|(_, v)| v)
                .collect();
            let entry = if oidc.is_empty() {
                &value["https://accounts.x.ai/sign-in"]
            } else {
                ensure!(
                    oidc.len() == 1,
                    "Grok 有多个登录，请手动填写要监测账户的令牌"
                );
                oidc[0]
            };
            if !entry["expires_at"].is_null() {
                ensure!(
                    stamp(&entry["expires_at"]).is_some_and(|t| t > now() + 60),
                    "Grok 登录已过期，请运行 grok login 后重新连接"
                );
            }
            entry["key"].as_str()
        }
        _ => bail!("此类型请手动填写凭据"),
    };
    token
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .context("未找到有效的本机登录")
}

fn import_local(provider: &str, mode: &str) -> Result<String> {
    let root = if provider == "grok" {
        std::env::var_os("GROK_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".grok"))
    } else if provider == "opencode" {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::store::home().join(".local/share"))
            .join("opencode")
    } else {
        bail!("此 Agent 不支持读取本机登录");
    };
    let mut bytes = Vec::new();
    std::fs::File::open(root.join("auth.json"))
        .context("找不到本机 auth.json，请先在 Agent 中登录")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("无法读取本机登录")?;
    ensure!(bytes.len() <= 1024 * 1024, "登录文件过大");
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("登录文件格式无效"))?;
    local_secret(provider, mode, &value)
}

// Only these origins can receive a credential. Redirects are disabled below.
pub fn endpoint(provider: &str, config: &SubscriptionConfig) -> Result<String> {
    let url = match (provider, config.mode.as_str()) {
        ("zcode", "global") => "https://api.z.ai/api/monitor/usage/quota/limit",
        ("zcode", "cn") => "https://open.bigmodel.cn/api/monitor/usage/quota/limit",
        ("qoder", "global") => "https://qoder.com/api/v2/me/usages/big_model_credits",
        ("qoder", "cn-legacy") => "https://qoder.com.cn/api/v2/me/usages/big_model_credits",
        ("cursor", "session") => "https://cursor.com/api/usage-summary",
        ("grok", "subscription") => "https://cli-chat-proxy.grok.com/v1/billing?format=credits",
        ("grok", "api") => {
            scope(&config.workspace_id)?;
            return Ok(format!(
                "https://management-api.x.ai/v1/billing/teams/{}/prepaid/balance",
                config.workspace_id
            ));
        }
        ("opencode", "go") => "https://opencode.ai/zen/go/v1/usage",
        ("opencode", "console") => {
            scope(&config.workspace_id)?;
            "https://opencode.ai/console/api/go/status"
        }
        _ => bail!("不支持的订阅类型，请重新选择；地区之间不会自动重试凭据"),
    };
    Ok(url.into())
}

fn scope(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
        "请输入有效的 Workspace / Team ID"
    );
    Ok(())
}

fn request(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
) -> Result<Value> {
    ensure!(
        !secret.is_empty() && secret.len() <= 32768 && !secret.contains(['\r', '\n', '\0']),
        "登录凭据格式无效"
    );
    let client = network::client_builder(source.proxy.as_ref().unwrap_or(&settings.proxy))?
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建订阅查询连接")?;
    let mut request = client.get(url).header("Accept", "application/json");
    if matches!(source.provider.as_str(), "qoder" | "cursor") || config.mode == "console" {
        let cookie = secret.strip_prefix("Cookie:").unwrap_or(secret).trim();
        ensure!(
            cookie.contains('='),
            "请输入完整 Cookie 请求头，不是密码或 API Key"
        );
        request = request.header("Cookie", cookie);
    } else {
        request = request.bearer_auth(secret);
    }
    if source.provider == "grok" && config.mode == "subscription" {
        ensure!(
            !secret.starts_with("xai-"),
            "Grok 订阅需要登录令牌；API 管理密钥请选择 API 余额"
        );
        request = request.header("x-xai-token-auth", "xai-grok-cli");
    }
    if source.provider == "qoder" {
        let origin = if config.mode == "global" {
            "https://qoder.com"
        } else {
            "https://qoder.com.cn"
        };
        request = request
            .header("Origin", origin)
            .header("Referer", format!("{origin}/account/usage"));
    }
    if config.mode == "console" {
        request = request.header("x-org-id", &config.workspace_id);
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("订阅连接失败，请检查网络或代理"))?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "订阅查询失败（HTTP {}）；401/403 请重新登录或检查权限",
        status.as_u16()
    );
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("订阅响应读取失败")?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "订阅响应过大");
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("订阅响应不是有效 JSON，请检查登录是否过期"))
}

pub fn read(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    ensure!(
        source.host_id.is_none(),
        "订阅凭据仅在本机查询；远程历史可单独配置"
    );
    let config = source
        .subscription
        .as_ref()
        .context("请在账户中连接订阅登录")?;
    let url = endpoint(&source.provider, config)?;
    let secret = credentials::read(&config.credential_ref)
        .context("无法读取订阅凭据，请解锁系统凭据库或重新连接")?;
    fetch(source, settings, config, &secret, &url)
}

fn fetch(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
) -> Result<QuotaSnapshot> {
    let body = request(source, settings, config, secret, url)?;
    let mut snapshot = parse(source, config, &body)?;
    if source.provider == "opencode" && config.mode == "console" {
        // A balance failure must not discard independently valid Go quota.
        let balance = request(
            source,
            settings,
            config,
            secret,
            "https://opencode.ai/console/api/billing/status",
        )
        .and_then(|v| zen_balance(&v));
        match balance {
            Ok(balance) => snapshot.balances.push(balance),
            Err(_) if !snapshot.windows.is_empty() => {
                snapshot.metadata_error = Some("Zen 余额暂不可用；Go 额度已更新".into())
            }
            Err(e) => return Err(e),
        }
    }
    Ok(snapshot)
}

fn number(value: &Value) -> Result<f64> {
    let n = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .context("额度数值缺失或格式不支持")?;
    ensure!(n.is_finite(), "额度数值无效");
    Ok(n)
}
fn percent(value: &Value) -> Result<f64> {
    let n = number(value)?;
    ensure!(n >= 0., "额度百分比无效");
    Ok(n.min(100.))
}
fn ratio(used: &Value, limit: &Value) -> Result<f64> {
    let used = number(used)?;
    let limit = number(limit)?;
    ensure!(used >= 0. && limit > 0., "额度上限不可用，不能推算剩余比例");
    Ok((100. * used / limit).clamp(0., 100.))
}
fn stamp(v: &Value) -> Option<i64> {
    v.as_i64()
        .filter(|n| *n > 0)
        .map(|n| if n > 100_000_000_000 { n / 1000 } else { n })
        .or_else(|| {
            v.as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp())
        })
}
fn duration(start: &Value, end: &Value) -> Option<i64> {
    let (start, end) = (stamp(start)?, stamp(end)?);
    (end > start && start <= now())
        .then_some((end - start) / 60)
        .filter(|m| *m > 0)
}
fn window(
    q: &mut QuotaSnapshot,
    id: &str,
    name: &str,
    used: f64,
    minutes: Option<i64>,
    reset: Option<i64>,
) {
    q.windows.push(QuotaWindow {
        id: id.into(),
        name: name.into(),
        used_percent: used,
        window_minutes: minutes,
        resets_at: reset,
        ..Default::default()
    });
}
fn label(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned)
}
fn zen_balance(v: &Value) -> Result<Balance> {
    ensure!(v["mode"] == "pay-as-you-go", "Zen 余额计费类型不支持");
    let amount = number(&v["balanceMicroCents"])? / 100_000_000.;
    Ok(Balance {
        currency: "USD".into(),
        total: format!("{amount:.8}"),
        ..Default::default()
    })
}

pub fn parse(source: &Source, config: &SubscriptionConfig, v: &Value) -> Result<QuotaSnapshot> {
    endpoint(&source.provider, config)?;
    let mut q = QuotaSnapshot {
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: source.provider.clone(),
        name: source.name.clone(),
        updated_at: now(),
        origin: "live".into(),
        ..Default::default()
    };
    match source.provider.as_str() {
        "zcode" => {
            ensure!(
                v["success"] != false
                    && (v["code"].is_null() || v["code"] == 200 || v["code"] == 0),
                "ZCode 套餐查询被拒绝"
            );
            let data = &v["data"];
            q.plan = label(&data["planName"]).or_else(|| label(&data["plan"]));
            for (index, limit) in data["limits"]
                .as_array()
                .context("ZCode 额度字段缺失")?
                .iter()
                .enumerate()
            {
                let kind = limit["type"].as_str().unwrap_or("");
                if !matches!(kind, "TOKENS_LIMIT" | "CREDIT_LIMIT" | "TIME_LIMIT") {
                    continue;
                }
                let used = if number(&limit["usage"]).is_ok_and(|n| n > 0.)
                    && (!limit["currentValue"].is_null() || !limit["remaining"].is_null())
                {
                    let total = number(&limit["usage"])?;
                    let current = if limit["currentValue"].is_null() {
                        0.
                    } else {
                        number(&limit["currentValue"])?
                    };
                    let remaining_used = if limit["remaining"].is_null() {
                        0.
                    } else {
                        total - number(&limit["remaining"])?
                    };
                    ratio(&json!(current.max(remaining_used)), &limit["usage"])?
                } else {
                    percent(&limit["percentage"])?
                };
                let multiplier: Option<i64> = match limit["unit"].as_i64() {
                    Some(1) => Some(1440),
                    Some(3) => Some(60),
                    Some(5) => Some(1),
                    Some(6) => Some(10080),
                    _ => None,
                };
                let minutes = multiplier
                    .and_then(|m| limit["number"].as_i64()?.checked_mul(m))
                    .filter(|n| *n > 0);
                let reset = stamp(&limit["nextResetTime"])
                    .filter(|t| minutes != Some(300) || *t <= now() + 18060);
                window(
                    &mut q,
                    &format!("glm-{index}"),
                    if kind == "TIME_LIMIT" {
                        "MCP 调用"
                    } else {
                        "GLM Coding Plan"
                    },
                    used,
                    minutes,
                    reset,
                );
            }
        }
        "qoder" => {
            let mut used = 0.;
            let mut total = 0.;
            for key in ["totalQuota", "sharedQuota"] {
                if key == "sharedQuota" && v[key].is_null() {
                    continue;
                }
                let summary = &v[key]["quotaSummary"];
                let (u, t) = (
                    number(&summary["usedValue"])?,
                    number(&summary["limitValue"])?,
                );
                ensure!(u >= 0. && t >= 0., "Qoder Credits 格式无效");
                used += u;
                total += t;
            }
            let usage = if total == 0. && used == 0. {
                100.
            } else {
                ratio(&json!(used), &json!(total))?
            };
            window(
                &mut q,
                "credits",
                &format!("Credits · {used} / {total}"),
                usage,
                None,
                stamp(&v["nextResetAt"]),
            );
            q.plan = label(&v["planName"]);
        }
        "cursor" => {
            q.plan = label(&v["membershipType"]);
            let plan = &v["individualUsage"]["plan"];
            let minutes = duration(&v["billingCycleStart"], &v["billingCycleEnd"]);
            let reset = stamp(&v["billingCycleEnd"]);
            for (key, id, name) in [
                ("autoPercentUsed", "cursor-models", "Cursor 模型"),
                ("apiPercentUsed", "other-models", "其他模型"),
            ] {
                if !plan[key].is_null() {
                    window(&mut q, id, name, percent(&plan[key])?, minutes, reset);
                }
            }
            if q.windows.is_empty() {
                if !plan["totalPercentUsed"].is_null() {
                    window(
                        &mut q,
                        "plan",
                        "套餐额度",
                        percent(&plan["totalPercentUsed"])?,
                        minutes,
                        reset,
                    );
                } else {
                    for (item, id, name) in [
                        (plan, "plan", "套餐额度"),
                        (&v["individualUsage"]["overall"], "personal-cap", "个人上限"),
                        (&v["teamUsage"]["pooled"], "team-pool", "团队共享额度"),
                    ] {
                        if item.is_object() && number(&item["limit"]).is_ok_and(|n| n > 0.) {
                            window(
                                &mut q,
                                id,
                                name,
                                ratio(&item["used"], &item["limit"])?,
                                minutes,
                                reset,
                            );
                            break;
                        }
                    }
                }
            }
        }
        "grok" if config.mode == "api" => {
            let amount = number(&v["total"]["val"])? / 100.;
            q.balances.push(Balance {
                currency: "USD".into(),
                total: format!("{amount:.2}"),
                ..Default::default()
            });
            q.plan = Some("xAI API 预付余额（独立于 Grok 订阅）".into());
        }
        "grok" => {
            let c = &v["config"];
            let used = percent(&c["creditUsagePercent"])?;
            let (start, end) = if stamp(&c["currentPeriod"]["end"]).is_some() {
                (&c["currentPeriod"]["start"], &c["currentPeriod"]["end"])
            } else {
                (&c["billingPeriodStart"], &c["billingPeriodEnd"])
            };
            window(
                &mut q,
                "subscription",
                "Grok 订阅",
                used,
                duration(start, end),
                stamp(end),
            );
            q.plan = label(&c["subscriptionTier"]).or_else(|| label(&v["subscriptionTier"]));
        }
        "opencode" if config.mode == "go" => {
            for (key, name, minutes) in [
                ("rolling", "Go · 5 小时", Some(300)),
                ("weekly", "Go · 每周", Some(10080)),
                ("monthly", "Go · 本期", None),
            ] {
                let meter = &v["usage"][key];
                if meter.is_null() {
                    continue;
                }
                let reset = stamp(&meter["resetAt"]).or_else(|| {
                    meter["resetInSec"]
                        .as_i64()
                        .filter(|n| *n >= 0)
                        .and_then(|n| now().checked_add(n))
                });
                window(
                    &mut q,
                    key,
                    name,
                    percent(&meter["percent"])?,
                    minutes,
                    reset,
                );
            }
            q.plan = label(&v["plan"]);
        }
        "opencode" => {
            let access = &v["access"];
            ensure!(
                v.is_null() || v.is_object() && v.get("access").is_some(),
                "OpenCode Console 额度字段缺失"
            );
            for (key, name, minutes) in [
                ("fiveHour", "Go · 5 小时", Some(300)),
                ("week", "Go · 每周", Some(10080)),
                ("month", "Go · 本期", None),
            ] {
                let meter = &access["meters"][key];
                if meter.is_null() {
                    continue;
                }
                let reset = stamp(&meter["resetsAt"])
                    .or_else(|| (key == "month").then(|| stamp(&access["endsAt"])).flatten());
                window(
                    &mut q,
                    key,
                    name,
                    ratio(&meter["usedMicroCents"], &meter["limitMicroCents"])?,
                    minutes,
                    reset,
                );
            }
            q.plan = label(&access["plan"]);
        }
        _ => bail!("不支持的 Agent"),
    }
    ensure!(
        !q.windows.is_empty()
            || !q.balances.is_empty()
            || source.provider == "opencode"
                && config.mode == "console"
                && (v.is_null() || v["access"].is_null()),
        "未返回可识别的额度，请查看官方用量页；未知用量不代表零消耗"
    );
    Ok(q)
}

impl Engine {
    pub(crate) fn connect_subscription(&mut self, params: Value) -> Result<Value> {
        self.connect_subscription_using(params, fetch)
    }

    fn connect_subscription_using(
        &mut self,
        params: Value,
        fetcher: fn(&Source, &Settings, &SubscriptionConfig, &str, &str) -> Result<QuotaSnapshot>,
    ) -> Result<Value> {
        let base = self.store.settings()?;
        let provider = params["provider"].as_str().context("请选择 Agent")?;
        ensure!(
            supported(provider)
                && base
                    .agents
                    .iter()
                    .any(|a| a.provider == provider && a.enabled),
            "请先启用此 Agent"
        );
        let mut config: SubscriptionConfig =
            serde_json::from_value(params["subscription"].clone())?;
        config.credential_ref.clear(); // Never accept another account's secret reference over RPC.
        let url = endpoint(provider, &config)?;
        let imported = if params["useLocalLogin"] == true {
            Some(import_local(provider, &config.mode)?)
        } else {
            None
        };
        let secret = imported
            .as_deref()
            .or_else(|| {
                params["secret"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .context("请输入登录令牌、API Key 或 Cookie")?;
        let requested_id = params["accountId"].as_str().unwrap_or("");
        ensure!(
            requested_id.is_empty()
                || base
                    .accounts
                    .iter()
                    .any(|a| a.provider == provider && a.id == requested_id && !a.archived),
            "账户不存在或已归档"
        );
        let id = if requested_id.is_empty() {
            crate::codex_auth::files::new_id()
        } else {
            requested_id.into()
        };
        let name = params["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| crate::accounts::default_name(provider));
        let previous = base
            .sources
            .iter()
            .find(|s| s.provider == provider && s.account_id == id && s.subscription.is_some());
        let mut source = previous.cloned().unwrap_or_else(|| Source {
            id: crate::codex_auth::files::new_id(),
            provider: provider.into(),
            account_id: id.clone(),
            name: name.clone(),
            ..Default::default()
        });
        source.host_id = None;
        source.path.clear(); // Quota credentials are never scanned as conversation history.
        source.quota_command.clear();
        source.enabled = true;
        source.name = name.clone();
        let snapshot = fetcher(&source, &base, &config, secret, &url)?; // Fail before persisting any account.
        config.credential_ref = credentials::save(secret).context("无法保存订阅凭据")?;
        let new_ref = config.credential_ref.clone();
        source.subscription = Some(config);
        let mut next = base.clone();
        if requested_id.is_empty() {
            next.accounts.push(Account {
                id: id.clone(),
                provider: provider.into(),
                name,
                ..Default::default()
            });
        } else if let Some(account) = next
            .accounts
            .iter_mut()
            .find(|a| a.provider == provider && a.id == id)
        {
            account.name = name;
        }
        if let Some(agent) = next.agents.iter_mut().find(|a| a.provider == provider)
            && !agent.machine_ids.iter().any(|id| id == "local")
        {
            agent.machine_ids.push("local".into());
        }
        next.sources.retain(|s| s.id != source.id);
        next.sources.push(source);
        let saved = self.store.patch_settings(&json!(base), &json!(next), false);
        if saved.is_err() {
            let _ = credentials::delete(&new_ref);
        }
        let saved = saved?;
        // Old references are retained for concurrent readers; no credential is copied to SSH.
        Ok(json!({"settings":saved,"accountId":id,"quota":snapshot}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn validated_connection_persists_only_secret_reference_and_rotation_is_atomic() {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        engine
            .call(
                "agents.set",
                json!({"provider":"opencode","enabled":true,"machineIds":[]}),
            )
            .unwrap();
        let params = json!({"provider":"opencode","name":"Go account","secret":"private-fixture","subscription":{"mode":"go"}});
        let result = engine
            .connect_subscription_using(params.clone(), |source, _, config, secret, _| {
                assert_eq!(secret, "private-fixture");
                parse(source, config, &json!({"usage":{"rolling":{"percent":12}}}))
            })
            .unwrap();
        assert!(!result.to_string().contains("private-fixture"));
        let settings = engine.store.settings().unwrap();
        let source = settings
            .sources
            .iter()
            .find(|s| s.subscription.is_some())
            .unwrap();
        assert!(source.path.is_empty());
        assert!(settings.source_enabled(source));
        assert_eq!(
            settings
                .accounts
                .iter()
                .filter(|a| a.provider == "opencode")
                .count(),
            1
        );
        assert_eq!(
            credentials::read(&source.subscription.as_ref().unwrap().credential_ref).unwrap(),
            "private-fixture"
        );
        let mut rotate = params;
        rotate["accountId"] = result["accountId"].clone();
        rotate["secret"] = json!("new-secret");
        let before = json!(settings);
        assert!(
            engine
                .connect_subscription_using(rotate, |_, _, _, _, _| bail!("HTTP 401"))
                .is_err()
        );
        assert_eq!(before, json!(engine.store.settings().unwrap()));
    }

    #[test]
    fn http_does_not_follow_redirects_or_echo_secrets_and_preserves_header_auth() {
        for (status, body) in [
            ("302 Found", "fixture-secret"),
            ("401 Unauthorized", "fixture-secret"),
            ("200 OK", "{\"usage\":{}} "),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/usage", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                assert!(
                    headers
                        .to_lowercase()
                        .contains("authorization: bearer fixture-secret")
                );
                write!(socket, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/must-not-follow\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let settings = Settings {
                proxy: ProxyConfig {
                    mode: "direct".into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let result = request(
                &Source {
                    provider: "opencode".into(),
                    ..Default::default()
                },
                &settings,
                &SubscriptionConfig {
                    mode: "go".into(),
                    ..Default::default()
                },
                "fixture-secret",
                &url,
            );
            if status == "200 OK" {
                assert!(result.is_ok());
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains(&status[..3]));
                assert!(!error.contains("fixture-secret"));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn zen_balance_uses_signed_microcents_not_available_credit_or_monthly_allowance() {
        assert_eq!(zen_balance(&json!({"mode":"pay-as-you-go","balanceMicroCents":"-125000000","availableMicroCents":"900000000"})).unwrap().total,"-1.25000000");
        assert_eq!(
            zen_balance(&json!({"mode":"pay-as-you-go","balanceMicroCents":"0"}))
                .unwrap()
                .total,
            "0.00000000"
        );
        assert!(
            zen_balance(&json!({"mode":"pay-as-you-go","availableMicroCents":"900000000"}))
                .is_err()
        );
        assert!(zen_balance(&json!({"mode":"invoice","balanceMicroCents":"0"})).is_err());
    }
}
