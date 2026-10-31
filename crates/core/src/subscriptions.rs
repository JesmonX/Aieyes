//! Read-only subscription adapters. Dashboard endpoints are versioned observations,
//! not public API guarantees; see docs/agent-subscriptions.md.
use crate::{Engine, credentials, models::*, network};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;
mod buddy;
pub(crate) mod gemini;
mod kimi;
pub(crate) mod kiro;
mod prepaid;
pub(crate) mod qoder;
pub(crate) mod trae;
mod transport;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubscriptionConfig {
    pub mode: String,
    pub credential_ref: String,
    pub workspace_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub native_config_dir: String,
}

pub fn native_cli(provider: &str, config: &SubscriptionConfig) -> bool {
    provider == "qoder" && config.mode == "cli"
}

pub fn supported(provider: &str) -> bool {
    matches!(
        provider,
        "zcode"
            | "workbuddy"
            | "codebuddy"
            | "qoder"
            | "grok"
            | "cursor"
            | "opencode"
            | "kimi"
            | "deepseek"
            | "deepseek-harness"
    )
}

pub fn login_url(provider: &str, mode: &str) -> Result<&'static str> {
    match (provider, mode) {
        ("zcode", "global") => Ok("https://z.ai/manage-apikey/coding-plan/personal/my-plan"),
        ("zcode", "cn") => Ok("https://bigmodel.cn/coding-plan/personal/usage"),
        ("qoder", "global") => Ok("https://qoder.com/account/usage"),
        ("qoder", "cli") => Ok("https://docs.qoder.com/cli/using-cli"),
        ("qoder", "cn-legacy") => Ok("https://qoder.com.cn/account/usage"),
        ("kimi", "cn" | "global") => Ok("https://www.kimi.com/code/console"),
        ("deepseek" | "deepseek-harness", "api") => Ok("https://platform.deepseek.com/usage"),
        ("cursor", "api2") => Ok("https://cursor.com/dashboard"),
        ("grok", "subscription") => Ok("https://grok.com"),
        ("grok", "api") => Ok("https://console.x.ai"),
        ("opencode", "go" | "console") => Ok("https://opencode.ai/console"),
        // Official usage pages (`bootConfig.doc.accountUsageUrl`). Trae has nothing
        // to paste: its login is encrypted, so this page is the only fallback.
        ("trae", _) => Ok("https://www.trae.ai/account-setting#usage"),
        ("trae-cn", _) => Ok("https://www.trae.cn/account-setting#usage"),
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
    crate::process::open_external(cmd).context("无法打开官方登录页")?;
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
                grok_latest(&oidc).context("Grok 有多个登录，请手动填写要监测账户的令牌")?
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

/// Several `grok login` entries: the most recently used one that is still valid.
/// A manually entered token still overrides this choice.
pub(crate) fn grok_latest<'a>(entries: &[&'a Value]) -> Option<&'a Value> {
    // Grok 1.0.62 stores expiry, but no last_used auth field. Token issuance
    // and expiry preserve recency when explicit usage metadata is absent.
    let used = |e: &Value| {
        let explicit = [
            "last_used",
            "lastUsed",
            "last_used_at",
            "updated_at",
            "created_at",
        ]
        .iter()
        .find_map(|k| stamp(&e[*k]));
        use base64::Engine as _;
        let issued = e["key"]
            .as_str()
            .and_then(|s| s.split('.').nth(1))
            .and_then(|s| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(s.trim_end_matches('='))
                    .ok()
            })
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| stamp(&v["iat"]));
        (explicit, issued, stamp(&e["expires_at"]))
    };
    let valid = |e: &Value| {
        e["expires_at"].is_null() || stamp(&e["expires_at"]).is_some_and(|t| t > now() + 60)
    };
    let ranked: Vec<_> = entries.iter().copied().filter(|e| valid(e)).collect();
    if ranked.is_empty() {
        // All expired: the expiry error below names the newest one.
        return entries.iter().copied().max_by_key(|e| used(e));
    }
    // Deterministic ties do not turn an otherwise valid saved login into an
    // error. Explicitly supplied credentials still bypass this selection.
    ranked.into_iter().max_by_key(|e| used(e))
}

pub(crate) fn grok_expiry(entry: &Value) -> Option<i64> {
    stamp(&entry["expires_at"])
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
        // The IDE / CLI login token (state.vscdb or cli-config.json), read at query time.
        ("cursor", "api2") => {
            "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage"
        }
        ("kimi", "moonshot-cn") => "https://api.moonshot.cn/v1/users/me/balance",
        ("kimi", "moonshot-global") => "https://api.moonshot.ai/v1/users/me/balance",
        ("kimi", "cn") => "https://api.kimi.com/coding/v1/usages",
        ("kimi", "global") => "https://api.kimi.ai/coding/v1/usages",
        ("deepseek" | "deepseek-harness", "api") => "https://api.deepseek.com/user/balance",
        // Account login read from dsh's .credentials.yaml (native connections only).
        ("deepseek-harness", "account") => {
            "https://platform.deepseek.com/api/v0/users/get_user_summary"
        }
        ("grok", "subscription") => "https://cli-chat-proxy.grok.com/v1/billing?format=credits",
        ("grok", "api") => {
            scope(&config.workspace_id)?;
            return Ok(format!(
                "https://management-api.x.ai/v1/billing/teams/{}/prepaid/balance",
                config.workspace_id
            ));
        }
        ("opencode", "go") => "https://opencode.ai/zen/go/v1/usage",
        // A plain-text Trae IDE login (native connections only); the login's own
        // API host may replace the origin.
        ("trae" | "trae-cn", "usage") => trae::default_url(provider),
        // GitHub token found in env / apps.json or granted to Aieyes (native connections only).
        ("copilot", "github") => COPILOT_USER,
        // Prepaid wallets read with the client's own login (native connections only).
        ("cline", "account") => return Ok(format!("{}/users/me", prepaid::CLINE_API)),
        ("kilo", "profile") => return Ok(format!("{}/profile", prepaid::KILO_API)),
        ("cline" | "kilo", "openrouter") => prepaid::OPENROUTER_KEY,
        // Kiro login token; the region comes from the profile ARN at query time.
        ("kiro", "usage") => "https://codewhisperer.us-east-1.amazonaws.com/",
        ("workbuddy" | "codebuddy", "buddy-cn") => {
            "https://copilot.tencent.com/billing/meter/get-user-resource-summary"
        }
        ("workbuddy" | "codebuddy", "buddy-global") => {
            "https://www.codebuddy.ai/billing/meter/get-user-resource-summary"
        }
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
    request_with(source, settings, config, secret, url, &[])
}

fn request_with(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Value> {
    let request = transport::request(source, config, secret, url, headers)?;
    transport::response(
        source,
        config,
        request.send(source.proxy.as_ref().unwrap_or(&settings.proxy))?,
    )
}

pub fn read(source: &Source, settings: &Settings) -> Result<QuotaSnapshot> {
    let config = source
        .subscription
        .as_ref()
        .context("请在账户中连接订阅登录")?;
    if native_cli(&source.provider, config) {
        let snapshot = fetch(source, settings, config, "", "")?;
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        qoder::check_identity(&snapshot, account, true)?;
        return Ok(snapshot);
    }
    ensure!(
        source.host_id.is_none(),
        "此订阅凭据仅在本机查询；原生 CLI 连接可在远端查询"
    );
    let url = endpoint(&source.provider, config)?;
    let secret = if matches!(source.provider.as_str(), "workbuddy" | "codebuddy") {
        credentials::read_agent(&config.credential_ref)
    } else {
        credentials::read(&config.credential_ref)
    }
    .context("无法读取订阅凭据，请解锁系统凭据库或重新连接")?;
    fetch(source, settings, config, &secret, &url)
}

const COPILOT_USER: &str = "https://api.github.com/copilot_internal/user";
const COPILOT_AGENT: &str = concat!("Aieyes/", env!("CARGO_PKG_VERSION"));
const COPILOT_EDITOR: &str = concat!("aieyes/", env!("CARGO_PKG_VERSION"));
const DSH_RELOGIN: &str = "DeepSeek 登录已失效，在 dsh 中重新登录";

pub(crate) fn prepaid_identity(
    provider: &str,
    settings: &Settings,
    secret: &str,
) -> Result<AccountIdentity> {
    prepaid::inspect_identity(provider, settings, secret)
}

/// A native connection's credential read at query time from the client's file.
pub(crate) fn read_native(
    source: &Source,
    settings: &Settings,
    secret: &str,
) -> Result<QuotaSnapshot> {
    let config = source
        .subscription
        .as_ref()
        .context("请在账户中连接订阅登录")?;
    ensure!(source.host_id.is_none(), "此订阅凭据仅在本机查询");
    let url = endpoint(&source.provider, config)?;
    fetch(source, settings, config, secret, &url)
}

fn fetch(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
) -> Result<QuotaSnapshot> {
    if native_cli(&source.provider, config) {
        return qoder::read(source, settings, config);
    }
    if matches!(source.provider.as_str(), "workbuddy" | "codebuddy")
        && matches!(config.mode.as_str(), "buddy-cn" | "buddy-global")
    {
        return parse(source, config, &buddy::fetch(source, settings, secret)?);
    }
    transport::fetch_using(source, config, secret, url, |request| {
        request.send(source.proxy.as_ref().unwrap_or(&settings.proxy))
    })
}

/// Shared native request/response adapters; SSH passes an identity-only token
/// stub and resolves the real credential on the source host.
pub(crate) fn read_native_using(
    source: &Source,
    secret: &str,
    send: impl FnMut(
        crate::native_quota_transport::Request,
    ) -> Result<crate::native_quota_transport::Response>,
) -> Result<QuotaSnapshot> {
    let config = source
        .subscription
        .as_ref()
        .context("此 Agent 暂未接入自动额度查询")?;
    let url = endpoint(&source.provider, config)?;
    transport::fetch_using(source, config, secret, &url, send)
}

pub(crate) fn prepaid_identity_using(
    provider: &str,
    secret: &str,
    mut send: impl FnMut(
        crate::native_quota_transport::Request,
    ) -> Result<crate::native_quota_transport::Response>,
) -> Result<AccountIdentity> {
    let source = Source {
        provider: provider.into(),
        ..Default::default()
    };
    let config = SubscriptionConfig::default();
    prepaid::inspect_identity_using(provider, secret, |token, url| {
        let request = transport::request(&source, &config, token, url, &[])?;
        transport::response(&source, &config, send(request)?)
    })
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
    // Connect/proto JSON writes int64 milliseconds as digit strings.
    v.as_i64()
        .or_else(|| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse().ok())
        })
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

/// `get_user_summary`: wallets per currency (normal = topped up, bonus = granted).
fn deepseek_summary(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let data = &v["data"];
    ensure!(
        !(v["code"] == 40003 || data["biz_code"] == 40003),
        DSH_RELOGIN
    );
    ensure!(
        v["code"] == 0 && data["biz_code"] == 0,
        "DeepSeek 账户查询被拒绝"
    );
    let biz = &data["biz_data"];
    let mut wallets: std::collections::BTreeMap<String, (f64, f64)> = Default::default();
    for (key, bonus) in [("normal_wallets", false), ("bonus_wallets", true)] {
        for w in biz[key].as_array().context("DeepSeek 账户余额字段缺失")? {
            let currency = text(&w["currency"]).to_ascii_uppercase();
            ensure!(matches!(currency.as_str(), "CNY" | "USD"), "余额币种不支持");
            let amount = number(&w["balance"])?;
            let entry = wallets.entry(currency).or_default();
            if bonus {
                entry.1 += amount;
            } else {
                entry.0 += amount;
            }
        }
    }
    ensure!(!wallets.is_empty(), "DeepSeek 账户余额字段缺失");
    let decimal = |n: f64| {
        let s = format!("{n:.8}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    for (currency, (topped_up, granted)) in wallets {
        q.balances.push(Balance {
            currency,
            total: decimal(topped_up + granted),
            granted: decimal(granted),
            topped_up: decimal(topped_up),
        });
    }
    Ok(())
}
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

pub fn parse(source: &Source, config: &SubscriptionConfig, v: &Value) -> Result<QuotaSnapshot> {
    if !native_cli(&source.provider, config) {
        endpoint(&source.provider, config)?;
    }
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
        "deepseek-harness" if config.mode == "account" => {
            deepseek_summary(&mut q, v)?;
        }
        "deepseek" | "deepseek-harness" => {
            // This is the selected DeepSeek backend's API wallet, not a
            // Harness subscription or a quota inferred from local Tokens.
            let mut backend = source.clone();
            backend.provider = "deepseek".into();
            let mut snapshot = crate::quota::normalize(&backend, v, "live")?;
            ensure!(
                snapshot
                    .balances
                    .iter()
                    .all(|b| matches!(b.currency.as_str(), "CNY" | "USD")),
                "余额币种不支持"
            );
            snapshot.provider = source.provider.clone();
            return Ok(snapshot);
        }
        "kimi" if config.mode.starts_with("moonshot-") => {
            ensure!(
                v["status"] != false && (v["code"].is_null() || v["code"] == 0),
                "Moonshot 余额查询被拒绝"
            );
            let data = &v["data"];
            let currency = if config.mode == "moonshot-cn" {
                "CNY"
            } else {
                "USD"
            };
            q.balances.push(Balance {
                currency: currency.into(),
                total: format!("{:.2}", number(&data["available_balance"])?),
                ..Default::default()
            });
            q.plan = Some("Moonshot 开放平台余额（独立于 Kimi Code 订阅）".into());
        }
        "kimi" => {
            kimi::apply(&mut q, v)?;
        }
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
            if native_cli(&source.provider, config) {
                qoder::apply(&mut q, v)?;
                return Ok(q);
            }
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
        "trae" | "trae-cn" => trae::parse(&mut q, v)?,
        "cursor" => cursor_usage(&mut q, v)?,
        "copilot" => copilot_usage(&mut q, v)?,
        "cline" | "kilo" if config.mode == "openrouter" => prepaid::openrouter(&mut q, v)?,
        "cline" | "kilo" => prepaid::wallets(&mut q, &source.provider, v)?,
        "kiro" => kiro::parse(&mut q, v)?,
        "workbuddy" | "codebuddy" => buddy::parse(&mut q, v)?,
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
            || !q.meters.is_empty()
            || !q.balances.is_empty()
            || q.extra_usage.is_some()
            || q.notice.is_some()
            || source.provider == "opencode"
                && config.mode == "console"
                && (v.is_null() || v["access"].is_null()),
        "未返回可识别的额度，请在官方用量页查看"
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
            !matches!(provider, "workbuddy" | "codebuddy"),
            "请使用可选的 Buddy 登录查看余额"
        );
        ensure!(
            supported(provider)
                && base.agents.iter().any(|a| {
                    a.enabled
                        && crate::contracts::provider(&a.provider)
                            .zip(crate::contracts::provider(provider))
                            .is_some_and(|(a, b)| a.agent_id == b.agent_id)
                }),
            "请先启用此 Agent"
        );
        let mut config: SubscriptionConfig =
            serde_json::from_value(params["subscription"].clone())?;
        config.credential_ref.clear(); // Never accept another account's secret reference over RPC.
        let native = native_cli(provider, &config);
        let url = if native {
            String::new()
        } else {
            endpoint(provider, &config)?
        };
        let imported = if !native && params["useLocalLogin"] == true {
            Some(import_local(provider, &config.mode)?)
        } else {
            None
        };
        let secret = if native {
            ""
        } else {
            imported
                .as_deref()
                .or_else(|| {
                    params["secret"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .context("请输入登录令牌、API Key 或 Cookie")?
        };
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
        if native {
            qoder::check_identity(
                &snapshot,
                base.accounts
                    .iter()
                    .find(|a| a.provider == provider && a.id == id),
                false,
            )?;
        } else {
            config.native_config_dir.clear();
            config.credential_ref = credentials::save(secret).context("无法保存订阅凭据")?;
        }
        let new_ref = config.credential_ref.clone();
        source.subscription = Some(config);
        let mut next = base.clone();
        if requested_id.is_empty() {
            next.accounts.push(Account {
                id: id.clone(),
                provider: provider.into(),
                name,
                identity_key: if native {
                    snapshot.identity.as_ref().map(|i| i.key.clone())
                } else {
                    None
                },
                ..Default::default()
            });
        } else if let Some(account) = next
            .accounts
            .iter_mut()
            .find(|a| a.provider == provider && a.id == id)
        {
            account.name = name;
            if native && account.identity_key.is_none() {
                account.identity_key = snapshot.identity.as_ref().map(|i| i.key.clone());
            }
        }
        if let Some(agent) = next.agents.iter_mut().find(|a| a.provider == provider) {
            agent.enabled = true;
            if !agent.machine_ids.iter().any(|id| id == "local") {
                agent.machine_ids.push("local".into());
            }
        }
        next.sources.retain(|s| s.id != source.id);
        next.sources.push(source);
        let saved = self.store.patch_settings(&json!(base), &json!(next), false);
        if saved.is_err() && !new_ref.is_empty() {
            let _ = credentials::delete(&new_ref);
        }
        let saved = saved?;
        // Old references are retained for concurrent readers; no credential is copied to SSH.
        Ok(json!({"settings":saved,"accountId":id,"quota":snapshot}))
    }
}

/// `GetCurrentPeriodUsage`: plan pools as percent windows, on-demand spend
/// (cents) as meters. A team pool is keyed by its team so members share one meter.
fn cursor_usage(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let plan = &v["planUsage"];
    ensure!(
        plan.is_object() || v["spendLimitUsage"].is_object(),
        "Cursor 用量响应格式不支持"
    );
    let minutes = duration(&v["billingCycleStart"], &v["billingCycleEnd"]);
    let reset = stamp(&v["billingCycleEnd"]);
    for (key, id, name) in [
        ("autoPercentUsed", "cursor-models", "Cursor 模型"),
        ("apiPercentUsed", "other-models", "其他模型"),
    ] {
        if !plan[key].is_null() {
            window(q, id, name, percent(&plan[key])?, minutes, reset);
        }
    }
    if q.windows.is_empty() {
        if !plan["totalPercentUsed"].is_null() {
            let used = percent(&plan["totalPercentUsed"])?;
            window(q, "plan", "套餐额度", used, minutes, reset);
        } else if number(&plan["limit"]).is_ok_and(|n| n > 0.) {
            let used = ratio(&plan["totalSpend"], &plan["limit"])?;
            window(q, "plan", "套餐额度", used, minutes, reset);
        }
    }
    let spend = &v["spendLimitUsage"];
    let cents = |key: &str| number(&spend[key]).ok().map(|n| n / 100.);
    let team = text(&spend["limitType"]) == "team";
    let (used, limit, id, label) = if team {
        let team_id = spend["teamId"]
            .as_i64()
            .map(|n| n.to_string())
            .or_else(|| label(&spend["teamId"]));
        (
            cents("pooledUsed"),
            cents("pooledLimit"),
            team_id.map_or("team-pool".into(), |t| format!("team-pool:{t}")),
            "团队共享额度",
        )
    } else {
        (
            cents("individualUsed"),
            cents("individualLimit"),
            "on-demand".into(),
            "按需用量",
        )
    };
    if let Some(used) = used.filter(|u| *u >= 0.) {
        let total = limit.filter(|l| *l > 0.);
        q.meters.push(QuotaMeter {
            id,
            label: label.into(),
            used,
            total,
            remaining: total.map(|t| (t - used).max(0.)),
            unit: "usd".into(),
            resets_at: reset,
            ..Default::default()
        });
    }
    ensure!(
        !q.windows.is_empty() || !q.meters.is_empty(),
        "Cursor 用量响应格式不支持"
    );
    Ok(())
}

/// `copilot_internal/user` (unpublished; see docs/agent-subscriptions.md). Limited
/// pools become meters, unlimited ones are omitted unless usage-based billing
/// reports `credits_used`. `remaining` may be negative once paid overage is used.
/// With `token_based_billing` the premium pool is GitHub AI Credits, not requests.
fn copilot_usage(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    ensure!(v.is_object(), "Copilot 用量响应格式不支持");
    let reset = stamp(&v["quota_reset_date_utc"])
        .or_else(|| stamp(&v["quota_reset_date"]))
        .or_else(|| stamp(&v["limited_user_reset_date"]))
        .or_else(|| {
            [&v["quota_reset_date"], &v["limited_user_reset_date"]]
                .into_iter()
                .find_map(|d| chrono::NaiveDate::parse_from_str(d.as_str()?, "%Y-%m-%d").ok())
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|t| t.and_utc().timestamp())
        });
    let names = [
        ("premium_interactions", "Premium 请求"),
        ("chat", "Chat 消息"),
        ("completions", "代码补全"),
    ];
    let credits = v["token_based_billing"] == true;
    let snapshots = &v["quota_snapshots"];
    for (key, label) in names {
        let s = &snapshots[key];
        if !s.is_object() {
            continue;
        }
        let premium = key == "premium_interactions";
        let (label, unit) = if premium && credits {
            ("AI Credits", "credits")
        } else {
            (label, "requests")
        };
        if s["unlimited"] == true {
            // No allowance to compare against; only what usage-based billing spent.
            if premium && let Ok(used) = number(&s["credits_used"]) {
                ensure!(used >= 0., "Copilot 额度格式无效");
                q.meters.push(QuotaMeter {
                    id: key.replace('_', "-"),
                    label: "AI Credits".into(),
                    used,
                    unit: "credits".into(),
                    resets_at: reset,
                    ..Default::default()
                });
            }
            continue;
        }
        let total = number(&s["entitlement"])?;
        // `quota_remaining` keeps the fraction left by request multipliers; either
        // field going negative is paid overage.
        let precise = number(&s["quota_remaining"]).ok();
        let signed = number(&s["remaining"]).ok();
        let remaining = precise.or(signed).context("额度数值缺失或格式不支持")?;
        ensure!(total >= 0., "Copilot 额度格式无效");
        if total == 0. && remaining == 0. && s["overage_count"].is_null() {
            continue;
        }
        let overage = [precise, signed]
            .into_iter()
            .flatten()
            .fold(number(&s["overage_count"]).unwrap_or(0.), |o, r| o.max(-r))
            .max(0.);
        q.meters.push(QuotaMeter {
            id: key.replace('_', "-"),
            label: label.into(),
            used: (total - remaining.max(0.)).max(0.),
            total: Some(total),
            remaining: Some(remaining.max(0.)),
            overage: (overage > 0.).then_some(overage),
            unit: unit.into(),
            resets_at: reset,
            ..Default::default()
        });
    }
    // Copilot Free (older responses): monthly allowance and what is left of it.
    if q.meters.is_empty() {
        for (key, label) in names {
            let (Ok(total), Ok(left)) = (
                number(&v["monthly_quotas"][key]),
                number(&v["limited_user_quotas"][key]),
            ) else {
                continue;
            };
            ensure!(total >= 0. && left >= 0., "Copilot 额度格式无效");
            q.meters.push(QuotaMeter {
                id: key.replace('_', "-"),
                label: label.into(),
                used: (total - left).max(0.),
                total: Some(total),
                remaining: Some(left.min(total)),
                unit: "requests".into(),
                resets_at: reset,
                ..Default::default()
            });
        }
    }
    q.plan = match text(&v["copilot_plan"]) {
        "free" => Some("Copilot Free".into()),
        "individual" | "individual_pro" => Some("Copilot Pro".into()),
        "individual_pro_plus" | "pro_plus" => Some("Copilot Pro+".into()),
        "business" => Some("Copilot Business".into()),
        "enterprise" => Some("Copilot Enterprise".into()),
        other => label(&json!(other)).or_else(|| label(&v["access_type_sku"])),
    };
    if let Some(login) = label(&v["login"]) {
        q.identity = Some(AccountIdentity {
            key: copilot_identity(&login),
            email: login,
            checked_at: now(),
            ..Default::default()
        });
    }
    if q.meters.is_empty() && v["quota_snapshots"].is_object() {
        // Every pool unlimited (e.g. Business without premium metering).
        q.notice = Some("当前套餐无计量额度".into());
    }
    ensure!(
        !q.meters.is_empty() || q.notice.is_some(),
        "Copilot 用量响应格式不支持"
    );
    Ok(())
}
/// Legacy/offline quota projection only. Live quota reads bind `/user`'s numeric
/// subject in `transport`; usernames are not stable across renames or reuse.
pub(crate) fn copilot_identity(login: &str) -> String {
    format!("copilot:{}", hash(&login.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qoder_native_connection_binds_identity_without_a_secret_and_rejects_account_switch() {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        engine
            .call(
                "agents.set",
                json!({"provider":"qoder","enabled":true,"machineIds":[]}),
            )
            .unwrap();
        let params = json!({"provider":"qoder","secret":"MUST_NOT_BE_SAVED","subscription":{"mode":"cli","credentialRef":"MUST_NOT_BE_REUSED","nativeConfigDir":"/native/config"}});
        fn quota(
            source: &Source,
            _: &Settings,
            config: &SubscriptionConfig,
            secret: &str,
            url: &str,
        ) -> Result<QuotaSnapshot> {
            assert!(secret.is_empty() && url.is_empty() && config.credential_ref.is_empty());
            parse(
                source,
                config,
                &json!({"userId":"fixture-user","userQuota":{"total":0,"used":0,"remaining":0,"unit":"credits"}}),
            )
        }
        let result = engine
            .connect_subscription_using(params.clone(), quota)
            .unwrap();
        let before = engine.store.settings().unwrap();
        assert!(!json!(before).to_string().contains("MUST_NOT"));
        let source = before
            .sources
            .iter()
            .find(|s| s.subscription.is_some())
            .unwrap();
        let account = before
            .accounts
            .iter()
            .find(|a| a.id == result["accountId"])
            .unwrap();
        let config = source.subscription.as_ref().unwrap();
        assert_eq!(config.native_config_dir, "/native/config");
        assert!(config.credential_ref.is_empty() && source.path.is_empty());
        let snapshot = quota(source, &before, config, "", "").unwrap();
        qoder::check_identity(&snapshot, Some(account), true).unwrap();
        assert!(qoder::check_identity(&snapshot, None, true).is_err());
        let mut changed = snapshot.clone();
        changed.identity.as_mut().unwrap().key = "qoder:other".into();
        assert!(qoder::check_identity(&changed, Some(account), true).is_err());
        let mut update = params;
        update["accountId"] = result["accountId"].clone();
        assert!(engine.connect_subscription_using(update, |source,_,config,_,_| parse(source,config,&json!({"userId":"other-user","userQuota":{"total":10,"used":0,"remaining":10,"unit":"credits"}}))).is_err());
        assert_eq!(json!(engine.store.settings().unwrap()), json!(before));
    }

    #[test]
    fn new_deepseek_wallet_uses_canonical_provider_with_legacy_agent_enabled() {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        let mut settings = engine.store.settings().unwrap();
        settings
            .agents
            .iter_mut()
            .find(|a| a.provider == "deepseek-harness")
            .unwrap()
            .enabled = true;
        engine.store.save_settings(&settings).unwrap();
        let result = engine.connect_subscription_using(
            json!({"provider":"deepseek","secret":"wallet-test-secret","subscription":{"mode":"api"}}),
            |source, _, config, _, url| {
                assert_eq!(url, "https://api.deepseek.com/user/balance");
                parse(source, config, &json!({"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"1.00","granted_balance":"0.00","topped_up_balance":"1.00"}]}))
            },
        ).unwrap();
        assert!(!result.to_string().contains("wallet-test-secret"));
        let saved = engine.store.settings().unwrap();
        let source = saved
            .sources
            .iter()
            .find(|s| s.provider == "deepseek")
            .unwrap();
        assert!(source.path.is_empty());
        assert!(source.subscription.is_some());
        assert!(saved.source_enabled(source));
        assert_eq!(
            saved
                .accounts
                .iter()
                .filter(|a| a.provider == "deepseek")
                .count(),
            1
        );
    }

    #[test]
    fn harness_balance_account_has_no_history_path_and_native_sources_still_require_one() {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        // This scenario starts with no native history source, regardless of the host's installed clients.
        engine.store.save_settings(&Settings::default()).unwrap();
        let mut settings = engine.store.settings().unwrap();
        let agent = settings
            .agents
            .iter_mut()
            .find(|a| a.provider == "deepseek-harness")
            .unwrap();
        agent.enabled = true;
        agent.machine_ids = vec!["local".into()];
        engine.store.save_settings(&settings).unwrap();
        let result=engine.connect_subscription_using(json!({"provider":"deepseek-harness","secret":"private-balance-fixture","subscription":{"mode":"api"}}),|source,_,config,_,url|{
            assert_eq!(url,"https://api.deepseek.com/user/balance");parse(source,config,&json!({"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"10.00","granted_balance":"0.00","topped_up_balance":"10.00"}]}))
        }).unwrap();
        assert!(!result.to_string().contains("private-balance-fixture"));
        let saved = engine.store.settings().unwrap();
        let source = saved
            .sources
            .iter()
            .find(|s| s.provider == "deepseek-harness")
            .unwrap();
        assert!(source.path.is_empty());
        assert!(source.subscription.is_some());
        let mut invalid = saved;
        invalid
            .sources
            .iter_mut()
            .find(|s| s.provider == "deepseek-harness")
            .unwrap()
            .subscription = None;
        assert!(engine.store.save_settings(&invalid).is_err());
    }
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn cline_account_uses_bearer_auth_and_dsh_keeps_its_native_header() {
        for provider in ["cline", "deepseek-harness"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/account", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.is_empty() || line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line.to_lowercase());
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .unwrap();
                headers
            });
            let source = Source {
                provider: provider.into(),
                ..Default::default()
            };
            let settings = Settings {
                proxy: ProxyConfig {
                    mode: "direct".into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let config = SubscriptionConfig {
                mode: "account".into(),
                ..Default::default()
            };
            request(&source, &settings, &config, "workos:fixture", &url).unwrap();
            let headers = server.join().unwrap();
            if provider == "cline" {
                assert!(headers.contains("authorization: bearer workos:fixture"));
                assert!(!headers.contains("x-dsh-auth-token"));
            } else {
                assert!(headers.contains("x-dsh-auth-token: workos:fixture"));
                assert!(!headers.contains("authorization:"));
            }
        }
    }

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
        let mut rotate = params.clone();
        rotate["accountId"] = result["accountId"].clone();
        rotate["secret"] = json!("new-secret");
        let before = json!(settings);
        assert!(
            engine
                .connect_subscription_using(rotate, |_, _, _, _, _| bail!("HTTP 401"))
                .is_err()
        );
        assert_eq!(before, json!(engine.store.settings().unwrap()));
        engine
            .connect_subscription_using(params, |source, _, config, _, _| {
                parse(source, config, &json!({"usage":{"rolling":{"percent":24}}}))
            })
            .unwrap();
        let mut edited = source.clone();
        edited.name = "Updated subscription".into();
        engine
            .call(
                "sources.configure",
                json!({"source":edited,"baseSource":source}),
            )
            .unwrap();
        let saved = engine.store.settings().unwrap();
        assert_eq!(
            saved
                .sources
                .iter()
                .filter(|s| s.subscription.is_some())
                .count(),
            2
        );
        assert_eq!(
            saved
                .sources
                .iter()
                .find(|s| s.id == source.id)
                .unwrap()
                .name,
            "Updated subscription"
        );
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
    fn copilot_pro_free_and_overage_become_request_meters() {
        let source = Source {
            provider: "copilot".into(),
            ..Default::default()
        };
        let config = SubscriptionConfig {
            mode: "github".into(),
            ..Default::default()
        };
        assert_eq!(
            endpoint("copilot", &config).unwrap(),
            "https://api.github.com/copilot_internal/user"
        );
        // Pro: chat/completions unlimited, premium metered.
        let pro = parse(&source, &config, &json!({
            "login":"Octo-Fixture","copilot_plan":"individual_pro","access_type_sku":"plus_monthly_subscriber_quota",
            "quota_reset_date_utc":"2026-11-01T00:00:00.000Z",
            "quota_snapshots":{
                "chat":{"entitlement":0,"remaining":0,"unlimited":true},
                "completions":{"entitlement":0,"remaining":0,"unlimited":true},
                "premium_interactions":{"entitlement":300,"remaining":250,"percent_remaining":83.3,"unlimited":false,"overage_count":0,"overage_permitted":true}}
        })).unwrap();
        assert_eq!(pro.plan.as_deref(), Some("Copilot Pro"));
        assert_eq!(pro.meters.len(), 1);
        let m = &pro.meters[0];
        assert_eq!(
            (m.id.as_str(), m.unit.as_str()),
            ("premium-interactions", "requests")
        );
        assert_eq!(
            (m.used, m.total, m.remaining, m.overage),
            (50., Some(300.), Some(250.), None)
        );
        assert_eq!(m.resets_at, Some(1793491200));
        assert!(m.valid());
        let identity = pro.identity.unwrap();
        assert_eq!(identity.key, copilot_identity("octo-fixture"));
        // Overage: remaining goes negative once paid requests are used.
        let over = parse(&source, &config, &json!({
            "copilot_plan":"individual_pro","quota_reset_date":"2026-11-01",
            "quota_snapshots":{"premium_interactions":{"entitlement":300,"remaining":-12,"unlimited":false,"overage_count":12,"overage_permitted":true}}
        })).unwrap();
        let m = &over.meters[0];
        assert_eq!(
            (m.used, m.remaining, m.overage),
            (300., Some(0.), Some(12.))
        );
        assert_eq!(m.resets_at, Some(1793491200));
        // Free, older response shape: monthly allowance and what is left.
        let free = parse(
            &source,
            &config,
            &json!({
                "copilot_plan":"free","limited_user_reset_date":"2026-11-01",
                "monthly_quotas":{"chat":50,"completions":2000},
                "limited_user_quotas":{"chat":45,"completions":1990}
            }),
        )
        .unwrap();
        assert_eq!(free.plan.as_deref(), Some("Copilot Free"));
        let ids: Vec<_> = free
            .meters
            .iter()
            .map(|m| (m.id.as_str(), m.used))
            .collect();
        assert_eq!(ids, [("chat", 5.), ("completions", 10.)]);
        // Everything unlimited is a notice, not an error.
        let business = parse(&source, &config, &json!({
            "copilot_plan":"business",
            "quota_snapshots":{"chat":{"unlimited":true},"premium_interactions":{"unlimited":true}}
        })).unwrap();
        assert!(business.meters.is_empty() && business.notice.is_some());
        assert!(parse(&source, &config, &json!({"message":"Not Found"})).is_err());
    }

    #[test]
    fn copilot_usage_based_billing_reports_ai_credits() {
        // Field shapes from community reports of `copilot_internal/user`; not yet seen live.
        let source = Source {
            provider: "copilot".into(),
            ..Default::default()
        };
        let config = SubscriptionConfig {
            mode: "github".into(),
            ..Default::default()
        };
        // Token-based billing: the premium pool is AI Credits; the fractional
        // `quota_remaining` is preferred over the rounded `remaining`.
        let pro = parse(&source, &config, &json!({
            "copilot_plan":"individual_pro","token_based_billing":true,"quota_reset_date_utc":"2026-11-01T00:00:00.000Z",
            "quota_snapshots":{"chat":{"unlimited":true},"completions":{"unlimited":true},
                "premium_interactions":{"entitlement":1000,"remaining":746,"quota_remaining":746.5,"unlimited":false,"overage_permitted":false}}
        })).unwrap();
        let m = &pro.meters[0];
        assert_eq!(
            (m.label.as_str(), m.unit.as_str()),
            ("AI Credits", "credits")
        );
        assert_eq!(
            (m.used, m.total, m.remaining),
            (253.5, Some(1000.), Some(746.5))
        );
        assert!(m.valid());
        // A clamped `quota_remaining` does not hide overage that `remaining` reports.
        let over = parse(&source, &config, &json!({
            "token_based_billing":true,
            "quota_snapshots":{"premium_interactions":{"entitlement":1000,"remaining":-40,"quota_remaining":0,"unlimited":false}}
        })).unwrap();
        let m = &over.meters[0];
        assert_eq!(
            (m.used, m.remaining, m.overage),
            (1000., Some(0.), Some(40.))
        );
        // An unlimited pool has no allowance; usage-based billing still reports spend.
        let business = parse(
            &source,
            &config,
            &json!({
                "copilot_plan":"business","token_based_billing":true,
                "quota_snapshots":{"premium_interactions":{"unlimited":true,"credits_used":42.25}}
            }),
        )
        .unwrap();
        let m = &business.meters[0];
        assert_eq!(
            (m.used, m.total, m.remaining, m.unit.as_str()),
            (42.25, None, None, "credits")
        );
        assert!(business.notice.is_none());
        assert!(
            parse(
                &source,
                &config,
                &json!({
                    "quota_snapshots":{"premium_interactions":{"unlimited":true,"credits_used":-1}}
                })
            )
            .is_err()
        );
    }

    #[test]
    fn copilot_request_uses_token_header_and_refuses_classic_tokens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/copilot_internal/user",
            listener.local_addr().unwrap()
        );
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
                headers.push_str(&line.to_lowercase());
            }
            assert!(headers.contains("authorization: token gho_fixture"));
            assert!(headers.contains("editor-version: aieyes/"));
            let body = "{}";
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let settings = Settings {
            proxy: ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let source = Source {
            provider: "copilot".into(),
            ..Default::default()
        };
        let config = SubscriptionConfig {
            mode: "github".into(),
            ..Default::default()
        };
        assert!(request(&source, &settings, &config, "gho_fixture", &url).is_ok());
        server.join().unwrap();
        let error = request(&source, &settings, &config, "ghp_classic", &url)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ghp_") && !error.contains("ghp_classic"));
    }

    #[test]
    fn zcode_quota_header_matches_desktop_native_authorization() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/api/monitor/usage/quota/limit",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut authorization = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if line.to_lowercase().starts_with("authorization:") {
                    authorization.push(line.trim().to_owned());
                }
            }
            assert_eq!(authorization.len(), 1);
            assert!(authorization[0].ends_with(": fixture-coding-plan-key"));
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .unwrap();
        });
        let settings = Settings {
            proxy: ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let source = Source {
            provider: "zcode".into(),
            ..Default::default()
        };
        let config = SubscriptionConfig {
            mode: "global".into(),
            ..Default::default()
        };
        assert!(request(&source, &settings, &config, "fixture-coding-plan-key", &url).is_ok());
        server.join().unwrap();
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
