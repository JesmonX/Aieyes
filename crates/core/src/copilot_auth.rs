//! "GitHub 授权" for Copilot when no token is found in a file or the environment.
//!
//! The device flow runs only with Aieyes' own GitHub OAuth App, whose client id is
//! supplied at build time (`AIEYES_GITHUB_CLIENT_ID`). No other application's
//! client id is ever used. Without it, the button guides the user to sign in with
//! `copilot login` or `gh auth login` and re-detects; nothing is stored then.
//! A granted token lives only in the `app.aieyes.agents` credential store entry.
use crate::{Engine, credentials, models::*, network};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// The single Aieyes-held Copilot login; a new authorization replaces it.
pub(crate) const REFERENCE: &str = "copilot-github";
const CLIENT_ID: Option<&str> = option_env!("AIEYES_GITHUB_CLIENT_ID");

#[derive(Clone)]
struct Endpoints {
    device: String,
    token: String,
    user: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            device: "https://github.com/login/device/code".into(),
            token: "https://github.com/login/oauth/access_token".into(),
            user: "https://api.github.com/user".into(),
        }
    }
}

#[derive(Default)]
struct Flow {
    /// `external`, `waiting`, `authorized`, `detected`, `failed`, `expired` or `cancelled`.
    phase: String,
    message: String,
    user_code: String,
    verification_uri: String,
    expires_at: i64,
    login: String,
    cancelled: bool,
    scanned: bool,
}
fn flow() -> &'static Mutex<Arc<Mutex<Flow>>> {
    static FLOW: OnceLock<Mutex<Arc<Mutex<Flow>>>> = OnceLock::new();
    FLOW.get_or_init(Default::default)
}
fn current() -> Arc<Mutex<Flow>> {
    flow().lock().unwrap().clone()
}

fn client_id() -> Option<&'static str> {
    CLIENT_ID.filter(|id| {
        !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
    })
}
/// True only when this build carries Aieyes' own OAuth App client id.
pub(crate) fn enabled() -> bool {
    client_id().is_some()
}

const EXTERNAL: &str = "在终端运行 copilot login 或 gh auth login 完成登录；如果 CLI 询问，请允许以明文保存，Aieyes 会自动识别";

fn snapshot(f: &Flow) -> Value {
    json!({
        "phase": f.phase,
        "message": f.message,
        "deviceFlow": enabled(),
        "userCode": (!f.user_code.is_empty()).then_some(&f.user_code),
        "verificationUri": (!f.verification_uri.is_empty()).then_some(&f.verification_uri),
        "expiresAt": (f.expires_at > 0).then_some(f.expires_at),
        "login": (!f.login.is_empty()).then_some(&f.login),
        "commands": if f.phase == "external" { json!(["copilot login", "gh auth login"]) } else { json!([]) },
    })
}

fn post(client: &reqwest::blocking::Client, url: &str, body: String) -> Result<Value> {
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", concat!("Aieyes/", env!("CARGO_PKG_VERSION")))
        .body(body)
        .send()
        .map_err(|_| anyhow::anyhow!("GitHub 连接失败，请检查网络或代理"))?;
    read_json(response)
}
fn read_json(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let mut bytes = vec![];
    response
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("GitHub 响应读取失败")?;
    ensure!(bytes.len() <= 256 * 1024, "GitHub 响应过大");
    ensure!(
        status.is_success(),
        "GitHub 授权失败（HTTP {}）",
        status.as_u16()
    );
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("GitHub 响应格式无效"))
}
fn safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The stable GitHub account a token belongs to (`GET /user`). A login cached
/// on disk is not a numeric identity. The token is sent to api.github.com only.
pub(crate) fn github_identity(
    proxy: &ProxyConfig,
    token: &str,
) -> Result<crate::upstream::Identity> {
    let client = network::client_with_policy(
        proxy,
        network::ClientPolicy {
            redirects: 0,
            timeout: Duration::from_secs(15),
            ..Default::default()
        },
    )
    .context("无法创建 GitHub 连接")?;
    user_identity(&client, &Endpoints::default().user, token)
}
fn user_identity(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
) -> Result<crate::upstream::Identity> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", concat!("Aieyes/", env!("CARGO_PKG_VERSION")))
        .send()
        .map_err(|_| anyhow::anyhow!("GitHub 连接失败，请检查网络或代理"))?;
    ensure!(
        !matches!(response.status().as_u16(), 401 | 403),
        "GitHub 令牌无效或已过期"
    );
    let v = read_json(response)?;
    crate::upstream::github_identity(&v).context("GitHub 未返回有效的账户 ID")
}

/// Polls until GitHub grants or refuses, then stores the token. Runs on its own thread.
fn poll(
    state: Arc<Mutex<Flow>>,
    client: reqwest::blocking::Client,
    endpoints: Endpoints,
    client_id: String,
    device_code: String,
    mut interval: u64,
    store: impl Fn(&str) -> Result<()>,
) {
    let fail = |phase: &str, message: &str| {
        let mut f = state.lock().unwrap();
        if !f.cancelled {
            f.phase = phase.into();
            f.message = message.into();
        }
    };
    loop {
        std::thread::sleep(Duration::from_secs(interval));
        let (cancelled, expires_at) = {
            let f = state.lock().unwrap();
            (f.cancelled, f.expires_at)
        };
        if cancelled {
            return;
        }
        if now() >= expires_at {
            return fail("expired", "授权码已过期，请重新授权");
        }
        let body = format!(
            "client_id={client_id}&device_code={device_code}&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
        );
        let v = match post(&client, &endpoints.token, body) {
            Ok(v) => v,
            // A transient network error: keep polling until the code expires.
            Err(_) => continue,
        };
        match v["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = v["interval"].as_u64().unwrap_or(interval + 5).clamp(1, 60);
                continue;
            }
            Some("expired_token") => return fail("expired", "授权码已过期，请重新授权"),
            Some("access_denied") => return fail("failed", "已在 GitHub 拒绝授权"),
            Some(_) => return fail("failed", "GitHub 授权失败，请重新授权"),
            None => {}
        }
        let Some(token) = v["access_token"]
            .as_str()
            .filter(|t| safe(t))
            .map(str::to_owned)
        else {
            return fail("failed", "GitHub 未返回有效令牌");
        };
        let login = user_identity(&client, &endpoints.user, &token)
            .map(|identity| identity.email)
            .unwrap_or_default();
        if state.lock().unwrap().cancelled {
            return;
        }
        if store(&token).is_err() {
            return fail("failed", "无法保存登录凭据，请检查系统凭据库是否已解锁");
        }
        let mut f = state.lock().unwrap();
        f.phase = "authorized".into();
        f.message = "GitHub 授权完成，正在读取 Copilot 额度".into();
        f.login = login.chars().take(64).collect();
        return;
    }
}

fn start_device_flow(
    proxy: &ProxyConfig,
    endpoints: Endpoints,
    client_id: &str,
    store: impl Fn(&str) -> Result<()> + Send + 'static,
) -> Result<Value> {
    let client = network::client_with_policy(
        proxy,
        network::ClientPolicy {
            redirects: 0,
            ..Default::default()
        },
    )
    .context("无法创建 GitHub 连接")?;
    // No scope: Copilot's user endpoint only needs the user's identity.
    let v = post(&client, &endpoints.device, format!("client_id={client_id}"))?;
    let device_code = v["device_code"]
        .as_str()
        .filter(|c| safe(c))
        .context("GitHub 未返回授权码")?
        .to_owned();
    let user_code = v["user_code"]
        .as_str()
        .filter(|c| c.len() <= 32)
        .context("GitHub 未返回授权码")?
        .to_owned();
    let verification_uri = v["verification_uri"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/") || cfg!(test))
        .context("GitHub 授权地址无效")?
        .to_owned();
    let interval = v["interval"].as_u64().unwrap_or(5).clamp(1, 60);
    let expires = v["expires_in"].as_i64().unwrap_or(900).clamp(60, 1800);
    let state = Arc::new(Mutex::new(Flow {
        phase: "waiting".into(),
        message: "在 GitHub 页面输入授权码".into(),
        user_code,
        verification_uri,
        expires_at: now() + expires,
        ..Default::default()
    }));
    {
        let mut slot = flow().lock().unwrap();
        slot.lock().unwrap().cancelled = true;
        *slot = state.clone();
    }
    let result = snapshot(&state.lock().unwrap());
    let id = client_id.to_owned();
    std::thread::spawn(move || poll(state, client, endpoints, id, device_code, interval, store));
    Ok(result)
}

impl Engine {
    pub(crate) fn copilot_auth_call(&mut self, method: &str, _params: Value) -> Result<Value> {
        match method {
            "copilotAuth.start" => {
                let settings = self.store.settings()?;
                let Some(id) = client_id() else {
                    let state = Arc::new(Mutex::new(Flow {
                        phase: "external".into(),
                        message: EXTERNAL.into(),
                        ..Default::default()
                    }));
                    {
                        let mut slot = flow().lock().unwrap();
                        slot.lock().unwrap().cancelled = true;
                        *slot = state.clone();
                    }
                    return self.copilot_detect(state);
                };
                start_device_flow(&settings.proxy, Endpoints::default(), id, |token| {
                    credentials::save_agent(REFERENCE, token)
                })
            }
            "copilotAuth.status" => {
                let state = current();
                let phase = state.lock().unwrap().phase.clone();
                match phase.as_str() {
                    // Re-detect after the user ran `copilot login` / `gh auth login`.
                    "external" => self.copilot_detect(state),
                    "authorized" if !state.lock().unwrap().scanned => {
                        state.lock().unwrap().scanned = true;
                        self.scan_accounts(json!({"targets":[["copilot","local"]]}))?;
                        let mut f = state.lock().unwrap();
                        f.phase = "detected".into();
                        f.message = "GitHub 授权完成".into();
                        Ok(snapshot(&f))
                    }
                    "" => Ok(json!({"phase":"idle","deviceFlow":enabled(),"commands":[]})),
                    _ => Ok(snapshot(&state.lock().unwrap())),
                }
            }
            "copilotAuth.cancel" => {
                let state = current();
                let mut f = state.lock().unwrap();
                if matches!(f.phase.as_str(), "waiting" | "external") {
                    f.cancelled = true;
                    f.phase = "cancelled".into();
                    f.message = "已取消".into();
                }
                Ok(snapshot(&f))
            }
            _ => bail!("未知 Copilot 授权操作"),
        }
    }
    fn copilot_detect(&mut self, state: Arc<Mutex<Flow>>) -> Result<Value> {
        self.scan_accounts(json!({"targets":[["copilot","local"]]}))?;
        let settings = self.store.settings()?;
        // Detected means attributed to a GitHub account: a classic `ghp_` token or
        // one GitHub could not name keeps the guidance open.
        let found = settings.native_connections.iter().find(|r| {
            r.provider == "copilot"
                && r.machine_id == "local"
                && r.has_credential
                && r.status == "observed"
                && r.account_key.is_some()
        });
        let mut f = state.lock().unwrap();
        if let Some(row) = found
            && !f.cancelled
        {
            f.phase = "detected".into();
            f.message = "已识别 GitHub 登录".into();
            f.login = row.email.clone();
        }
        Ok(snapshot(&f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serves canned JSON by path; the token endpoint answers pending once.
    fn server(count: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = vec![];
            let mut polls = 0;
            for _ in 0..count {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                let mut body = vec![0; length];
                std::io::Read::read_exact(&mut reader, &mut body).unwrap();
                request.push_str(&String::from_utf8(body).unwrap());
                let reply = if request.starts_with("POST /device") {
                    json!({"device_code":"dev-code","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","interval":1,"expires_in":120})
                } else if request.starts_with("POST /token") {
                    polls += 1;
                    if polls == 1 {
                        json!({"error":"authorization_pending"})
                    } else {
                        json!({"access_token":"gho_fixture","token_type":"bearer"})
                    }
                } else {
                    json!({"login":"octo-fixture","id":1})
                };
                let reply = reply.to_string();
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .unwrap();
                seen.push(request);
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn device_flow_polls_until_granted_and_stores_only_through_the_callback() {
        let (base, handle) = server(4);
        let endpoints = Endpoints {
            device: format!("{base}/device"),
            token: format!("{base}/token"),
            user: format!("{base}/user"),
        };
        let stored = Arc::new(Mutex::new(String::new()));
        let sink = stored.clone();
        let proxy = ProxyConfig {
            mode: "direct".into(),
            ..Default::default()
        };
        let started = start_device_flow(&proxy, endpoints, "Iv1.fixture", move |t| {
            *sink.lock().unwrap() = t.into();
            Ok(())
        })
        .unwrap();
        assert_eq!(started["phase"], "waiting");
        assert_eq!(started["userCode"], "ABCD-1234");
        assert!(!started.to_string().contains("dev-code"));
        let requests = handle.join().unwrap();
        for _ in 0..50 {
            if current().lock().unwrap().phase != "waiting" {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let state = current();
        let f = state.lock().unwrap();
        assert_eq!(f.phase, "authorized");
        assert_eq!(f.login, "octo-fixture");
        assert_eq!(*stored.lock().unwrap(), "gho_fixture");
        let view = snapshot(&f).to_string();
        assert!(!view.contains("gho_fixture"), "token never reaches the UI");
        assert!(requests[0].contains("client_id=Iv1.fixture"));
        assert!(requests[1].contains("grant-type%3Adevice_code"));
        assert!(
            requests[3]
                .to_lowercase()
                .contains("authorization: token gho_fixture")
        );
    }

    #[test]
    fn github_identity_keeps_numeric_id_and_rejects_missing_id_or_revoked_tokens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/user", listener.local_addr().unwrap());
        let replies = [
            (200, json!({"login":"Octo-Fixture","id":1}).to_string()),
            (401, json!({"message":"Bad credentials"}).to_string()),
            (200, json!({"login":"not a login"}).to_string()),
            (200, json!({"login":"Octo-Fixture"}).to_string()),
        ];
        let server = std::thread::spawn(move || {
            let mut headers = vec![];
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line.to_lowercase());
                }
                write!(
                    socket,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                headers.push(request);
            }
            headers
        });
        let client = network::client_with_policy(
            &ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            network::ClientPolicy {
                redirects: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let identity = user_identity(&client, &url, "gho_fixture").unwrap();
        assert_eq!(identity.upstream, "github:1");
        assert_eq!(identity.subject, "1");
        assert_eq!(identity.email, "Octo-Fixture");
        let revoked = user_identity(&client, &url, "gho_fixture")
            .unwrap_err()
            .to_string();
        assert_eq!(revoked, "GitHub 令牌无效或已过期");
        assert!(user_identity(&client, &url, "gho_fixture").is_err());
        assert!(user_identity(&client, &url, "gho_fixture").is_err());
        let headers = server.join().unwrap();
        assert!(headers[0].starts_with("get /user"));
        assert!(headers[0].contains("authorization: token gho_fixture"));
    }

    #[test]
    fn without_an_own_oauth_app_no_client_id_is_borrowed() {
        // Release builds without AIEYES_GITHUB_CLIENT_ID fall back to `copilot login`.
        if CLIENT_ID.is_none() {
            assert!(!enabled());
        }
        let f = Flow {
            phase: "external".into(),
            message: EXTERNAL.into(),
            ..Default::default()
        };
        let v = snapshot(&f);
        assert_eq!(v["commands"], json!(["copilot login", "gh auth login"]));
        assert!(v["userCode"].is_null());
    }
}
