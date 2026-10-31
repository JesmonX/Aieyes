//! Provider HTTP rules shared by local requests and the SSH native transport.
use super::*;
use crate::native_quota_transport::{Request, Response};

pub(super) fn request(
    source: &Source,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Request> {
    ensure!(
        !secret.is_empty() && secret.len() <= 32768 && !secret.contains(['\r', '\n', '\0']),
        "登录凭据格式无效"
    );
    let mut request = if source.provider == "cursor" {
        Request::new("POST", url)
            .header("Content-Type", "application/json")
            .header("Connect-Protocol-Version", "1")
            .body("{}")
    } else {
        Request::new("GET", url)
    }
    .header("Accept", "application/json");
    if source.provider == "qoder" || config.mode == "console" {
        let cookie = secret.strip_prefix("Cookie:").unwrap_or(secret).trim();
        ensure!(
            cookie.contains('='),
            "请输入完整 Cookie 请求头，不是密码或 API Key"
        );
        request = request.header("Cookie", cookie);
    } else if source.provider == "copilot" {
        ensure!(
            !secret.starts_with("ghp_"),
            "经典 ghp_ 令牌不支持 Copilot，请使用 copilot login 或细粒度令牌"
        );
        request = request
            .header("Authorization", format!("token {secret}"))
            .header("User-Agent", COPILOT_AGENT)
            .header("Editor-Version", COPILOT_EDITOR)
            .header("Editor-Plugin-Version", COPILOT_EDITOR)
            .header("X-GitHub-Api-Version", "2025-04-01");
    } else if source.provider == "deepseek-harness" && config.mode == "account" {
        request = request.header("x-dsh-auth-token", secret);
    } else if source.provider == "zcode" {
        // Coding Plan's monitor endpoint accepts the raw login JWT or API key.
        request = request.header("Authorization", secret);
    } else {
        request = request.header("Authorization", format!("Bearer {secret}"));
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
    for (key, value) in headers {
        request = request.header(key, *value);
    }
    Ok(request)
}

pub(super) fn response(
    source: &Source,
    config: &SubscriptionConfig,
    response: Response,
) -> Result<Value> {
    let status = response.status;
    let expired = matches!(status, 401 | 403);
    if source.provider == "deepseek-harness" && config.mode == "account" {
        ensure!(!expired, DSH_RELOGIN);
    }
    if source.provider == "kiro" {
        ensure!(!expired, kiro::RELOGIN);
    }
    if trae::family(&source.provider) {
        ensure!(!expired, trae::RELOGIN);
    }
    ensure!(
        (200..300).contains(&status),
        "订阅查询失败（HTTP {status}）；401/403 请重新登录或检查权限"
    );
    Ok(response.data)
}

pub(super) fn trae_request(source: &Source, secret: &str, url: &str) -> Result<Request> {
    let login: Value =
        serde_json::from_str(secret).map_err(|_| anyhow::anyhow!("登录凭据格式无效"))?;
    let token = text(&login["token"]);
    ensure!(
        !token.is_empty() && token.len() <= 32768 && !token.contains(['\r', '\n', '\0', ' ']),
        "登录凭据格式无效"
    );
    let url = match (
        trae::api_origin(&source.provider, text(&login["host"])),
        url.split_once("/trae/"),
    ) {
        (Some(origin), Some((_, path))) => format!("{origin}/trae/{path}"),
        _ => url.to_owned(),
    };
    let body = if source.provider == "trae-cn" {
        json!({"require_usage":true,"req_source":1})
    } else {
        json!({"require_usage":true})
    };
    Ok(Request::new("POST", &url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Authorization", format!("Cloud-IDE-JWT {token}"))
        .body(body.to_string()))
}

pub(super) fn kiro_request(secret: &str) -> Result<Request> {
    let credential: Value =
        serde_json::from_str(secret).map_err(|_| anyhow::anyhow!("登录凭据格式无效"))?;
    let token = text(&credential["accessToken"]);
    let arn = text(&credential["profileArn"]);
    ensure!(
        !token.is_empty() && token.len() <= 32768 && !token.contains(['\r', '\n', '\0']),
        "登录凭据格式无效"
    );
    let url = kiro::region_url(arn)?;
    let body = if arn.is_empty() {
        json!({})
    } else {
        json!({"profileArn":arn})
    };
    Ok(Request::new("POST", url)
        .header("Content-Type", "application/x-amz-json-1.0")
        .header("X-Amz-Target", kiro::TARGET)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(body.to_string()))
}

pub(super) fn fetch_using(
    source: &Source,
    config: &SubscriptionConfig,
    secret: &str,
    url: &str,
    mut send: impl FnMut(Request) -> Result<Response>,
) -> Result<QuotaSnapshot> {
    // The private quota response commonly contains only a mutable login. Read
    // the public numeric subject with the same token/SSH pin before attributing
    // its quota. A missing `/user` id is an error, never a guessed account.
    let github = if source.provider == "copilot" && config.mode == "github" {
        // Apply the same token restrictions before either request (notably ghp_).
        request(source, config, secret, url, &[])?;
        Some(crate::upstream::probe_using(
            "copilot", secret, "", &mut send,
        )?)
    } else {
        None
    };
    let mut read = |request| response(source, config, send(request)?);
    let body = match (source.provider.as_str(), config.mode.as_str()) {
        ("trae" | "trae-cn", "usage") => read(trae_request(source, secret, url)?)?,
        ("kiro", "usage") => read(kiro_request(secret)?)?,
        ("cline", "account") => prepaid::cline_using(secret, url, |token, url| {
            read(request(source, config, token, url, &[])?)
        })?,
        ("kilo", "profile") => prepaid::kilo_using(url, |url, headers| {
            read(request(source, config, secret, url, headers)?)
        })?,
        _ => read(request(source, config, secret, url, &[])?)?,
    };
    let mut snapshot = parse(source, config, &body)?;
    if let Some(github) = github {
        let login = snapshot
            .identity
            .as_ref()
            .context("Copilot 额度响应未确认账户身份")?;
        ensure!(
            login.email.eq_ignore_ascii_case(&github.email),
            "查询期间原生账户身份已变化"
        );
        snapshot.identity = Some(AccountIdentity {
            key: format!("copilot:{}", github.subject),
            email: github.email,
            checked_at: now(),
            ..Default::default()
        });
    }
    if source.provider == "opencode" && config.mode == "console" {
        let balance = read(request(
            source,
            config,
            secret,
            "https://opencode.ai/console/api/billing/status",
            &[],
        )?)
        .and_then(|v| zen_balance(&v));
        match balance {
            Ok(balance) => snapshot.balances.push(balance),
            Err(_) if !snapshot.windows.is_empty() => {
                snapshot.metadata_error = Some("Zen 余额暂不可用；Go 额度已更新".into());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod copilot_tests {
    use super::*;

    fn copilot_read(
        token: &str,
        replies: Vec<(u16, Value)>,
    ) -> (Result<QuotaSnapshot>, Vec<Request>) {
        let source = Source {
            provider: "copilot".into(),
            ..Default::default()
        };
        let config = SubscriptionConfig {
            mode: "github".into(),
            ..Default::default()
        };
        let mut replies = replies.into_iter();
        let mut requests = vec![];
        let result = fetch_using(
            &source,
            &config,
            token,
            "https://api.github.com/copilot_internal/user",
            |request| {
                requests.push(request);
                let (status, data) = replies.next().expect("unexpected request");
                Ok(Response { status, data })
            },
        );
        (result, requests)
    }

    fn quota(login: &str) -> Value {
        json!({"login":login,"copilot_plan":"individual_pro","quota_snapshots":{"premium_interactions":{"entitlement":300,"remaining":250}}})
    }

    #[test]
    fn copilot_quota_binds_numeric_identity_and_compares_private_response_before_returning() {
        let (result, requests) = copilot_read(
            "gho_PRIVATE",
            vec![
                (200, json!({"id":7,"login":"New-Name"})),
                (200, quota("new-name")),
            ],
        );
        let snapshot = result.unwrap();
        assert_eq!(snapshot.identity.as_ref().unwrap().key, "copilot:7");
        assert_eq!(snapshot.identity.as_ref().unwrap().email, "New-Name");
        assert_eq!(snapshot.meters[0].remaining, Some(250.));
        assert_eq!(
            requests.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
            [
                "https://api.github.com/user",
                "https://api.github.com/copilot_internal/user"
            ]
        );
        assert!(
            requests
                .iter()
                .all(|r| r.headers["Authorization"] == "token gho_PRIVATE")
        );
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("PRIVATE")
        );
        let (result, _) = copilot_read(
            "gho_PRIVATE",
            vec![
                (200, json!({"id":7,"login":"one"})),
                (200, quota("different")),
            ],
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "查询期间原生账户身份已变化"
        );
    }

    #[test]
    fn copilot_missing_server_id_or_unsupported_token_cannot_reach_the_quota_endpoint() {
        for value in [
            json!({"login":"old-name"}),
            json!({"id":"7","login":"old-name"}),
            json!({"id":0,"login":"old-name"}),
        ] {
            let (result, requests) = copilot_read("gho_PRIVATE", vec![(200, value)]);
            assert!(result.is_err());
            assert_eq!(requests.len(), 1);
        }
        for status in [401, 403, 429] {
            let (result, requests) =
                copilot_read("gho_PRIVATE", vec![(status, json!({"error":"PRIVATE"}))]);
            assert!(result.is_err());
            assert!(!result.unwrap_err().to_string().contains("PRIVATE"));
            assert_eq!(requests.len(), 1);
        }
        let (result, requests) = copilot_read("ghp_CLASSIC", vec![]);
        assert!(result.is_err());
        assert!(requests.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(provider: &str) -> Source {
        Source {
            provider: provider.into(),
            ..Default::default()
        }
    }
    fn config(mode: &str) -> SubscriptionConfig {
        SubscriptionConfig {
            mode: mode.into(),
            ..Default::default()
        }
    }
    #[test]
    fn provider_headers_are_shared_by_local_and_remote_requests() {
        for (provider, mode, header, value) in [
            ("zcode", "cn", "Authorization", "fixture"),
            ("deepseek-harness", "account", "x-dsh-auth-token", "fixture"),
            ("deepseek-harness", "api", "Authorization", "Bearer fixture"),
            ("copilot", "github", "Authorization", "token fixture"),
            ("kimi", "global", "Authorization", "Bearer fixture"),
        ] {
            let c = config(mode);
            let r = request(
                &source(provider),
                &c,
                "fixture",
                &endpoint(provider, &c).unwrap(),
                &[],
            )
            .unwrap();
            assert_eq!(r.headers[header], value);
            if provider == "deepseek-harness" && mode == "account" {
                assert!(!r.headers.contains_key("Authorization"));
            }
            if provider == "copilot" {
                for key in [
                    "User-Agent",
                    "Editor-Version",
                    "Editor-Plugin-Version",
                    "X-GitHub-Api-Version",
                ] {
                    assert!(r.headers.contains_key(key));
                }
            }
        }
        let r = request(
            &source("cursor"),
            &config("api2"),
            "fixture",
            "https://api2.cursor.sh/x",
            &[],
        )
        .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.body.as_deref(), Some("{}"));
        assert_eq!(r.headers["Connect-Protocol-Version"], "1");
    }
    #[test]
    fn rejected_cline_token_never_becomes_a_dsh_login_error() {
        let result = Response {
            status: 401,
            data: Value::Null,
        };
        let error = response(&source("cline"), &config("account"), result).unwrap_err();
        assert!(error.to_string().contains("HTTP 401"));
        assert!(!error.to_string().contains("DeepSeek"));
    }
}
