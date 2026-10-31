//! Subscription logins another client keeps for a different service: OpenCode
//! (`opencode.db` credential rows or the legacy `auth.json`), Pi and Oh My Pi
//! (`auth.json`, or OMP's `agent.db`). Their quota is read with that service's own adapter and only
//! read: the owning client keeps refreshing its tokens, nothing is written back,
//! and a server account that is also signed in natively (Claude Code, Codex,
//! Copilot) is queried there instead (see `account_scan::quota_sources`).
use crate::{
    models::*,
    native_quota_transport::{Request, Response},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

const CLAUDE_USAGE: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_PROFILE: &str = "https://api.anthropic.com/api/oauth/profile";
const GITHUB_USER: &str = "https://api.github.com/user";
const AGENT: &str = concat!("Aieyes/", env!("CARGO_PKG_VERSION"));
/// A rejected token: the owning client refreshes it, it is never refreshed here.
const REJECTED: &str = "上游登录已失效，等待原客户端刷新";

/// The service whose adapter reads a stored login, by the client's entry name:
/// OMP database references retain a positive row id; it is not an account id.
fn entry_name<'a>(provider: &str, selector: &'a str) -> Option<&'a str> {
    let name = selector.strip_prefix('/')?;
    if provider == "omp"
        && let Some((name, id)) = name.split_once('#')
    {
        let row = id.parse::<i64>().ok()?;
        return (row > 0 && row.to_string() == id).then_some(name);
    }
    Some(name)
}

/// OpenCode integrations and Pi providers share these entry names. Gemini
/// CLI OAuth is supported only in Pi/OMP, never generic Google API keys.
pub(crate) fn kind(provider: &str, selector: &str) -> Option<&'static str> {
    if !matches!(provider, "opencode" | "pi" | "omp") {
        return None;
    }
    match entry_name(provider, selector)? {
        "anthropic" => Some("claude"),
        // OpenCode `openai` (ChatGPT login); Pi `openai-codex` and `openai`.
        "openai" | "openai-codex" => Some("codex"),
        "github-copilot" => Some("copilot"),
        "google-gemini-cli" if matches!(provider, "pi" | "omp") => Some("gemini"),
        _ => None,
    }
}

/// Shown on the reference instead of the generic native-login note.
pub(crate) fn note(kind: &str) -> &'static str {
    match kind {
        "claude" => "已识别 Claude 订阅登录",
        "codex" => "已识别 ChatGPT 订阅登录",
        "gemini" => "已识别 Gemini CLI 订阅登录",
        _ => "已识别 GitHub Copilot 登录",
    }
}

fn github_token(token: &str) -> bool {
    ["gho_", "ghu_", "ghp_", "github_pat_"]
        .iter()
        .any(|p| token.starts_with(p))
}

/// The token the service accepts, or `None` when the entry cannot be read here.
/// Pi and older OpenCode keep the GitHub token as `refresh` and a short-lived
/// Copilot token as `access`; OpenCode 2 stores the GitHub token in both.
pub(crate) fn token(kind: &str, entry: &Value) -> Option<String> {
    let text = |key: &str| {
        entry[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.contains(char::is_whitespace))
    };
    if kind != "copilot" {
        return text("access").map(str::to_owned);
    }
    // github.com only: the quota endpoint is api.github.com.
    let enterprise = entry["enterpriseUrl"]
        .as_str()
        .or(entry["metadata"]["enterpriseUrl"].as_str())
        .unwrap_or("")
        .trim()
        .trim_start_matches("https://")
        .trim_end_matches('/');
    if !enterprise.is_empty() && enterprise != "github.com" {
        return None;
    }
    ["refresh", "access"]
        .into_iter()
        .filter_map(text)
        .find(|t| github_token(t))
        .map(str::to_owned)
}

/// When a stored access token expired (seconds). GitHub OAuth tokens do not
/// expire; Pi's `expires` there belongs to the Copilot token it derives.
pub(crate) fn expired_at(kind: &str, entry: &Value) -> Option<i64> {
    if kind == "copilot" {
        return None;
    }
    entry["expires"]
        .as_i64()
        .filter(|t| *t > 0)
        .map(|t| if t > 10_000_000_000 { t / 1000 } else { t })
        .filter(|t| *t <= now())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// Shared with the native login of the same server account.
    pub upstream: String,
    /// Server-side subject (never a token or an email address).
    pub subject: String,
    pub email: String,
}

/// GitHub usernames can change or be reused. Only `/user`'s positive numeric
/// id is a stable subject; missing ids never fall back to a username hash.
pub(crate) fn github_identity(value: &Value) -> Option<Identity> {
    let id = value["id"].as_u64().filter(|id| *id > 0)?;
    let login = value["login"].as_str().filter(|login| {
        !login.is_empty()
            && login.len() <= 64
            && login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    })?;
    Some(Identity {
        upstream: format!("github:{id}"),
        subject: id.to_string(),
        email: login.into(),
    })
}

/// Distinguishes a verified numeric identity from legacy login-hash records.
pub(crate) fn github_id(identity: &str) -> Option<u64> {
    let subject = identity.strip_prefix("github:")?;
    subject
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == subject)
}

pub(crate) fn copilot_matches(identity: &AccountIdentity, upstream: &str) -> bool {
    github_id(upstream).is_some_and(|id| identity.key == format!("copilot:{id}"))
}

/// ChatGPT tokens carry the user and workspace as JWT claims, like Codex logins.
pub(crate) fn local_identity(kind: &str, token: &str, entry: &Value) -> Option<Identity> {
    if kind != "codex" {
        return None;
    }
    let id = crate::codex_auth::files::identity(&auth(kind, token, entry)).ok()?;
    Some(Identity {
        upstream: format!("openai:{}", id.key),
        subject: id.key,
        email: id.email,
    })
}

/// Preserve the workspace the client selected, which can differ from the JWT's
/// default workspace. No refresh token is ever handed to the query process.
pub(crate) fn auth(kind: &str, token: &str, entry: &Value) -> Value {
    if kind == "gemini" {
        let mut auth = json!({"access": token});
        for field in ["expires", "projectId"] {
            if !entry[field].is_null() {
                auth[field] = entry[field].clone();
            }
        }
        return auth;
    }
    if kind != "codex" {
        return json!({"access": token});
    }
    let mut tokens = json!({"access_token": token});
    if let Some(account) = entry["accountId"]
        .as_str()
        .or(entry["metadata"]["accountID"].as_str())
        .or(entry["metadata"]["accountId"].as_str())
        .filter(|s| !s.is_empty())
    {
        tokens["account_id"] = json!(account);
    }
    json!({"tokens": tokens})
}

/// Keep legacy client identities stable while merging OMP's enabled pool rows
/// and auth.json references only after the service verifies the same subject.
pub(crate) fn client_identity(provider: &str, selector: &str, kind: &str, subject: &str) -> String {
    let normalized = if provider == "omp" {
        entry_name(provider, selector).map(|name| format!("/{name}"))
    } else {
        None
    };
    format!(
        "{provider}:{}",
        hash(&json!([normalized.as_deref().unwrap_or(selector), kind, subject]).to_string())
    )
}

pub(crate) fn gemini_identity_using(
    auth: &Value,
    send: impl FnMut(Request) -> Result<Response>,
) -> Result<Identity> {
    let profile = crate::subscriptions::gemini::probe_using(auth, send)?;
    Ok(Identity {
        upstream: profile.upstream_identity(),
        subject: profile.subject_key(),
        email: profile.email,
    })
}

fn request(url: &str, kind: &str, token: &str) -> Result<Request> {
    ensure!(
        !token.is_empty() && token.len() <= 32768 && !token.contains(['\r', '\n', '\0']),
        "登录凭据格式无效"
    );
    let request = Request::new("GET", url)
        .timeouts(5, 15)
        .header("Accept", "application/json")
        .header("User-Agent", AGENT);
    Ok(match kind {
        "copilot" => request
            .header("Authorization", format!("token {token}"))
            .header("X-GitHub-Api-Version", "2022-11-28"),
        "claude" => request
            .header("Authorization", format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20"),
        _ => request.header("Authorization", format!("Bearer {token}")),
    })
}
fn response(response: Response) -> Result<Value> {
    let status = response.status;
    ensure!(!matches!(status, 401 | 403), REJECTED);
    ensure!(
        (200..300).contains(&status),
        "额度查询失败（HTTP {status}）"
    );
    Ok(response.data)
}

/// The provider request and bounded JSON response rules are identical for
/// local HTTP and a credential kept on its SSH host.
fn get_using(
    url: &str,
    kind: &str,
    token: &str,
    mut send: impl FnMut(Request) -> Result<Response>,
) -> Result<Value> {
    let received = send(request(url, kind, token)?).map_err(|error| {
        let message = match error.to_string().as_str() {
            "订阅连接失败，请检查网络或代理" => "额度查询连接失败",
            "订阅响应读取失败" => "额度响应读取失败",
            "订阅响应过大" => "额度响应过大",
            "订阅响应格式无效，请检查登录状态" => "额度响应格式无效",
            _ => return error,
        };
        anyhow::anyhow!("{message}")
    })?;
    response(received)
}
fn get(proxy: &ProxyConfig, url: &str, kind: &str, token: &str) -> Result<Value> {
    get_using(url, kind, token, |request| request.send(proxy))
}

pub(crate) fn usage_using(
    token: &str,
    send: impl FnMut(Request) -> Result<Response>,
) -> Result<Value> {
    get_using(CLAUDE_USAGE, "claude", token, send)
}

pub(crate) fn probe_using(
    kind: &str,
    token: &str,
    base: &str,
    send: impl FnMut(Request) -> Result<Response>,
) -> Result<Identity> {
    let url = match kind {
        "claude" => CLAUDE_PROFILE.to_owned(),
        "copilot" => GITHUB_USER.to_owned(),
        "kimi" => format!("{}/me", base.trim_end_matches('/')),
        _ => bail!("此 Agent 暂未接入自动额度查询"),
    };
    let value = get_using(&url, kind, token, send)?;
    identity(kind, &value).context("上游账户身份格式不支持")
}

/// Server identity of an opaque token (Anthropic OAuth, GitHub, Kimi), asked
/// from the service's read-only profile endpoint without any model prompt.
/// `base` is Kimi Code's API base for its region.
pub(crate) fn probe(kind: &str, token: &str, proxy: &ProxyConfig, base: &str) -> Result<Identity> {
    match kind {
        "claude" => probe_at(kind, token, proxy, CLAUDE_PROFILE),
        "copilot" => probe_at(kind, token, proxy, GITHUB_USER),
        "kimi" => probe_at(
            kind,
            token,
            proxy,
            &format!("{}/me", base.trim_end_matches('/')),
        ),
        _ => bail!("此 Agent 暂未接入自动额度查询"),
    }
}

fn probe_at(kind: &str, token: &str, proxy: &ProxyConfig, url: &str) -> Result<Identity> {
    let v = get(proxy, url, kind, token)?;
    identity(kind, &v).context("上游账户身份格式不支持")
}

fn identity(kind: &str, v: &Value) -> Option<Identity> {
    let text = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .or_else(|| v.as_i64().map(|n| n.to_string()))
            .filter(|s| !s.is_empty() && s.len() <= 256)
    };
    match kind {
        // Claude Code's own profile request: `account.uuid` is its accountUuid.
        "claude" => {
            let uuid = text(&v["account"]["uuid"])?;
            Some(Identity {
                upstream: format!("anthropic:{}", hash(&uuid)),
                subject: json!([uuid, v["organization"]["uuid"]]).to_string(),
                email: text(&v["account"]["email"])
                    .or_else(|| text(&v["account"]["email_address"]))
                    .unwrap_or_default(),
            })
        }
        "copilot" => github_identity(v),
        "kimi" => {
            let data = if v["user_id"].is_null() {
                &v["data"]
            } else {
                v
            };
            let user = text(&data["user_id"])?;
            Some(Identity {
                upstream: format!("kimi:{}", hash(&user)),
                subject: user,
                email: text(&data["email"])
                    .or_else(|| text(&data["nickname"]))
                    .unwrap_or_default(),
            })
        }
        _ => None,
    }
}

/// Quota of an upstream login through the service's adapter, shown on the
/// client's own account. `dir` is a private Aieyes directory for the Codex
/// app-server; `expected` guards Copilot attribution with GitHub's numeric id.
pub(crate) fn read(
    dir: &std::path::Path,
    settings: &Settings,
    source: &Source,
    kind: &str,
    auth: &Value,
    expected: Option<&str>,
) -> Result<QuotaSnapshot> {
    ensure!(source.host_id.is_none(), "此订阅凭据仅在本机查询");
    let token = auth["access"].as_str().unwrap_or_default();
    let proxy = source.proxy.as_ref().unwrap_or(&settings.proxy);
    let mut upstream = source.clone();
    upstream.provider = kind.into();
    upstream.quota_command.clear();
    let mut q = match kind {
        "gemini" => {
            crate::subscriptions::gemini::read_using(&upstream, auth, expected, |request| {
                request.send(proxy)
            })
            .map_err(|error| expired(error, &source.provider))?
        }
        "claude" => {
            let v =
                get(proxy, CLAUDE_USAGE, kind, token).map_err(|e| expired(e, &source.provider))?;
            crate::quota::normalize(&upstream, &v, "live")?
        }
        "codex" => {
            std::fs::create_dir_all(dir).context("无法准备额度查询目录")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
            let value = crate::codex_auth::external_quota(dir, proxy, auth)?;
            crate::quota::normalize(&upstream, &value, "live")?
        }
        "copilot" => {
            upstream.subscription = Some(crate::subscriptions::SubscriptionConfig {
                mode: "github".into(),
                ..Default::default()
            });
            let q = crate::subscriptions::read_native(&upstream, settings, token)
                .map_err(|e| expired(e, &source.provider))?;
            if let Some(expected) = expected {
                ensure!(
                    q.identity
                        .as_ref()
                        .is_some_and(|identity| copilot_matches(identity, expected)),
                    "查询期间原生账户身份已变化"
                );
            }
            q
        }
        _ => bail!("此 Agent 暂未接入自动额度查询"),
    };
    q.provider = source.provider.clone();
    q.source_id = source.id.clone();
    q.account_id = source.account_id.clone();
    q.name = source.name.clone();
    Ok(q)
}

/// A rejected token is refreshed by its own client the next time it runs.
pub(crate) fn expired(error: anyhow::Error, provider: &str) -> anyhow::Error {
    let text = error.to_string();
    if text == REJECTED || text.contains("HTTP 401") || text.contains("HTTP 403") {
        anyhow::anyhow!("{}", crate::account_scan::stale_note(provider))
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Answers each request with the canned JSON for its path and records the request head.
    fn server(
        replies: Vec<(&'static str, u16, Value)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = vec![];
            for _ in 0..replies.len() {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (_, status, body) = replies
                    .iter()
                    .find(|(p, ..)| path.starts_with(p))
                    .cloned()
                    .unwrap_or(("", 404, json!({})));
                let body = body.to_string();
                write!(
                    socket,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                seen.push(head);
            }
            seen
        });
        (base, handle)
    }
    fn direct() -> ProxyConfig {
        ProxyConfig {
            mode: "direct".into(),
            ..Default::default()
        }
    }

    #[test]
    fn only_client_entries_for_the_three_services_reuse_their_adapters() {
        assert_eq!(kind("opencode", "/anthropic"), Some("claude"));
        assert_eq!(kind("pi", "/openai-codex"), Some("codex"));
        assert_eq!(kind("omp", "/github-copilot"), Some("copilot"));
        assert_eq!(kind("opencode", "/opencode-go"), None);
        assert_eq!(kind("claude", "/anthropic"), None);
        assert_eq!(kind("kimi", "/github-copilot"), None);
    }

    #[test]
    fn gemini_cli_and_omp_row_selectors_stay_product_scoped() {
        assert_eq!(kind("pi", "/google-gemini-cli"), Some("gemini"));
        assert_eq!(kind("omp", "/google-gemini-cli#7"), Some("gemini"));
        assert_eq!(kind("omp", "/anthropic#8"), Some("claude"));
        assert_eq!(kind("omp", "/openai-codex#9"), Some("codex"));
        assert_eq!(kind("omp", "/github-copilot#3"), Some("copilot"));
        for selector in [
            "/google",
            "/google-antigravity",
            "/google-gemini-cli#0",
            "/google-gemini-cli#01",
            "/google-gemini-cli#-1",
            "/google-gemini-cli#9223372036854775808",
            "/google-gemini-cli#1#2",
        ] {
            assert_eq!(kind("omp", selector), None, "{selector}");
        }
        assert_eq!(kind("pi", "/google-gemini-cli#7"), None);
        assert_eq!(kind("opencode", "/google-gemini-cli"), None);
    }

    #[test]
    fn omp_pool_rows_merge_only_by_verified_subject_and_gemini_auth_keeps_project() {
        for (name, service) in [
            ("anthropic", "claude"),
            ("github-copilot", "copilot"),
            ("google-gemini-cli", "gemini"),
        ] {
            let legacy = client_identity("omp", &format!("/{name}"), service, "subject");
            assert_eq!(
                legacy,
                client_identity("omp", &format!("/{name}#7"), service, "subject")
            );
            assert_ne!(
                legacy,
                client_identity("omp", &format!("/{name}#7"), service, "different-subject")
            );
        }
        let before = format!(
            "pi:{}",
            hash(&json!(["/anthropic", "claude", "subject"]).to_string())
        );
        assert_eq!(
            client_identity("pi", "/anthropic", "claude", "subject"),
            before
        );
        let entry = json!({"type":"oauth","access":"PRIVATE","refresh":"DO-NOT-USE","expires":4102444800000_i64,"projectId":"project-one","email":"label@example.test"});
        let value = auth("gemini", "PRIVATE", &entry);
        assert_eq!(
            value,
            json!({"access":"PRIVATE","expires":4102444800000_i64,"projectId":"project-one"})
        );
        let mut other = entry;
        other["projectId"] = json!("project-two");
        assert_ne!(
            hash(&value.to_string()),
            hash(&auth("gemini", "PRIVATE", &other).to_string())
        );
    }

    #[test]
    fn copilot_uses_the_github_token_never_expires_and_skips_enterprise() {
        // Pi: `refresh` is the GitHub token, `access` the derived Copilot token.
        let pi = json!({"type":"oauth","refresh":"gho_PRIVATE","access":"tid=1;exp=2;sku=x","expires":1});
        assert_eq!(token("copilot", &pi).as_deref(), Some("gho_PRIVATE"));
        assert_eq!(expired_at("copilot", &pi), None);
        // OpenCode 2 keeps the GitHub token in both fields and `expires: 0`.
        let opencode = json!({"type":"oauth","methodID":"device","refresh":"gho_X","access":"gho_X","expires":0});
        assert_eq!(token("copilot", &opencode).as_deref(), Some("gho_X"));
        let enterprise =
            json!({"type":"oauth","access":"gho_X","metadata":{"enterpriseUrl":"company.ghe.com"}});
        assert_eq!(token("copilot", &enterprise), None);
        assert_eq!(token("copilot", &json!({"access":"tid=1;exp=2"})), None);
        // Other services: the access token, with millisecond expiry and 0 meaning none.
        let claude = json!({"type":"oauth","access":"sk-ant-oat01-PRIVATE","refresh":"R","expires":1_000_000_000_000i64});
        assert_eq!(
            token("claude", &claude).as_deref(),
            Some("sk-ant-oat01-PRIVATE")
        );
        assert_eq!(expired_at("claude", &claude), Some(1_000_000_000));
        assert_eq!(expired_at("claude", &json!({"expires":0})), None);
        assert_eq!(
            expired_at("claude", &json!({"expires":4102444800000i64})),
            None
        );
    }

    #[test]
    fn chatgpt_identity_comes_from_claims_like_codex() {
        let claims = json!({"https://api.openai.com/auth":{"chatgpt_account_id":"ws","chatgpt_user_id":"u"}});
        let access = format!(
            "e30.{}.sig",
            base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                claims.to_string()
            )
        );
        let id = local_identity("codex", &access, &json!({"metadata":{"accountID":"ws"}})).unwrap();
        let codex =
            crate::codex_auth::files::identity(&json!({"tokens":{"access_token":access}})).unwrap();
        assert_eq!(id.upstream, format!("openai:{}", codex.key));
        assert!(local_identity("claude", &access, &json!({})).is_none());
    }

    #[test]
    fn profiles_name_the_server_account_and_rejections_wait_for_the_client() {
        let (base, handle) = server(vec![
            (
                "/claude",
                200,
                json!({"account":{"uuid":"acct-1","email":"dev@example.test"},"organization":{"uuid":"org-1"}}),
            ),
            ("/github", 200, json!({"login":"Octo-Fixture","id":7})),
            (
                "/kimi/me",
                200,
                json!({"user_id":"k-1","nickname":"kimi-fixture"}),
            ),
            ("/denied", 401, json!({"error":"PRIVATE-SERVER-TEXT"})),
        ]);
        let claude = probe_at(
            "claude",
            "sk-ant-oat01-PRIVATE",
            &direct(),
            &format!("{base}/claude"),
        )
        .unwrap();
        assert_eq!(claude.upstream, format!("anthropic:{}", hash("acct-1")));
        assert_eq!(claude.email, "dev@example.test");
        let github = probe_at(
            "copilot",
            "gho_PRIVATE",
            &direct(),
            &format!("{base}/github"),
        )
        .unwrap();
        // The same form as Copilot's own references, so native wins the dedupe.
        assert_eq!(github.upstream, "github:7");
        assert_eq!(github.subject, "7");
        let kimi = probe("kimi", "PRIVATE", &direct(), &format!("{base}/kimi/")).unwrap();
        assert_eq!(kimi.subject, "k-1");
        let error =
            probe_at("claude", "PRIVATE", &direct(), &format!("{base}/denied")).unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        assert!(error.to_string().contains("等待原客户端刷新"));
        let heads = handle.join().unwrap();
        assert!(
            heads[0].contains("authorization: Bearer sk-ant-oat01-PRIVATE")
                || heads[0].contains("Authorization: Bearer sk-ant-oat01-PRIVATE")
        );
        assert!(
            heads[1]
                .to_ascii_lowercase()
                .contains("authorization: token gho_private")
        );
        assert!(probe("codex", "x", &direct(), "").is_err());
    }

    #[test]
    fn copilot_numeric_identity_survives_rename_and_rejects_missing_or_reused_ids() {
        let old = github_identity(&json!({"id":7,"login":"before-rename"})).unwrap();
        let renamed = github_identity(&json!({"id":7,"login":"After-Rename"})).unwrap();
        assert_eq!(old.upstream, renamed.upstream);
        assert_eq!(old.subject, renamed.subject);
        assert_ne!(old.email, renamed.email);
        let reused = github_identity(&json!({"id":8,"login":"before-rename"})).unwrap();
        assert_ne!(
            old.upstream, reused.upstream,
            "a reused username is a different account"
        );
        for id in [
            Value::Null,
            json!(0),
            json!(-1),
            json!(7.5),
            json!("7"),
            json!(true),
        ] {
            assert!(github_identity(&json!({"id":id,"login":"before-rename"})).is_none());
        }
        for invalid in ["github:0", "github:07", "github:-7", "github:", "other:7"] {
            assert_eq!(github_id(invalid), None);
        }
        assert_eq!(
            github_id(&format!("github:{}", hash("before-rename"))),
            None
        );
    }

    #[test]
    fn claude_usage_from_another_client_is_the_clients_card() {
        let raw: Value = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/claude-oauth-usage.json"
        ))
        .unwrap();
        let source = Source {
            id: "native-oc".into(),
            account_id: "oc".into(),
            provider: "claude".into(),
            ..Default::default()
        };
        let q = crate::quota::normalize(&source, &raw, "live").unwrap();
        assert!(!q.windows.is_empty());
        assert_eq!(
            expired(anyhow::anyhow!("额度查询失败（HTTP 401）"), "opencode").to_string(),
            crate::account_scan::stale_note("opencode")
        );
        assert_eq!(
            expired(anyhow::anyhow!("额度查询连接失败"), "opencode").to_string(),
            "额度查询连接失败"
        );
    }
}
