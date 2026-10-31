//! Optional, explicitly requested Buddy login. Uses the official desktop's
//! `/v2/auth/state` -> browser -> `/v2/auth/token` -> `/v2/login/account` flow.
//! Never reads, refreshes, replaces or logs WorkBuddy/CodeBuddy's credentials.
use crate::{Engine, credentials, models::*, network, subscriptions::SubscriptionConfig};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub(crate) const NOTICE: &str = "可选登录，使用非公开接口；可随时退出登录。仅用于查看余额。";
const WAITING: &str = "请在浏览器中完成 Buddy 登录";
const FAILED: &str = "Buddy 登录失败，请重试或在官方客户端查看余额";
const EXPIRED: &str = "Buddy 登录已过期，请重新登录查看余额";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Login {
    pub token: String,
    pub region: String,
    pub uid: String,
    pub nickname: String,
    pub enterprise: String,
    pub domain: String,
    pub expires_at: i64,
}
impl Login {
    fn valid(&self) -> bool {
        [&self.token, &self.uid, &self.enterprise, &self.domain]
            .iter()
            .all(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
            && !self.token.is_empty()
            && !self.uid.is_empty()
            && matches!(self.region.as_str(), "cn" | "global")
    }
    pub(crate) fn identity(&self) -> String {
        format!(
            "tencent:{}",
            hash(&json!([self.region, self.uid, self.enterprise]).to_string())
        )
    }
}
#[derive(Default)]
struct Flow {
    id: String,
    provider: String,
    phase: String,
    url: Option<String>,
    login: Option<Login>,
    expires_at: i64,
}
type Shared = Arc<Mutex<Flow>>;
fn flow() -> &'static Mutex<Shared> {
    static FLOW: OnceLock<Mutex<Shared>> = OnceLock::new();
    FLOW.get_or_init(Default::default)
}
pub(crate) fn region_base(region: &str) -> Result<&'static str> {
    match region {
        "cn" => Ok("https://copilot.tencent.com"),
        "global" => Ok("https://www.codebuddy.ai"),
        _ => bail!("请选择 Buddy 登录地区"),
    }
}
pub(crate) fn client(proxy: &ProxyConfig) -> Result<reqwest::blocking::Client> {
    network::client_with_policy(
        proxy,
        network::ClientPolicy {
            redirects: 0,
            ..Default::default()
        },
    )
    .context(FAILED)
}
/// Do not propagate reqwest errors (their URLs may contain the login state).
pub(crate) fn response(request: reqwest::blocking::RequestBuilder) -> Result<Value> {
    let response = request
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|_| anyhow::anyhow!(FAILED))?;
    let status = response.status();
    ensure!(status.as_u16() != 401 && status.as_u16() != 403, EXPIRED);
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= 2 * 1024 * 1024),
        "Buddy 响应过大"
    );
    use std::io::Read;
    let mut raw = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| anyhow::anyhow!(FAILED))?;
    ensure!(raw.len() <= 2 * 1024 * 1024, "Buddy 响应过大");
    let v: Value =
        serde_json::from_slice(&raw).map_err(|_| anyhow::anyhow!("Buddy 响应格式不支持"))?;
    // The desktop receives pending responses as HTTP 400 + code 11217/12151.
    ensure!(
        status.is_success() || matches!(v["code"].as_i64(), Some(11217 | 12151)),
        FAILED
    );
    Ok(v)
}
pub(crate) fn data(v: &Value) -> Result<&Value> {
    ensure!(v["code"] == 0, "Buddy 请求被拒绝");
    v.get("data").context("Buddy 响应格式不支持")
}
pub(crate) fn authorized(
    request: reqwest::blocking::RequestBuilder,
    login: &Login,
) -> Result<reqwest::blocking::RequestBuilder> {
    ensure!(login.valid(), "Buddy 登录凭据格式无效");
    ensure!(login.expires_at > now(), EXPIRED);
    let mut request = request
        .bearer_auth(&login.token)
        .header("X-User-Id", &login.uid)
        .header("X-Product", "SaaS");
    if !login.domain.is_empty() {
        request = request.header("X-Domain", &login.domain);
    }
    if !login.enterprise.is_empty() {
        request = request
            .header("X-Enterprise-Id", &login.enterprise)
            .header("X-Tenant-Id", &login.enterprise);
    }
    Ok(request)
}
fn safe_login_url(value: &str, region: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && match region {
                "cn" => matches!(
                    u.host_str(),
                    Some(
                        "www.workbuddy.cn"
                            | "www.codebuddy.cn"
                            | "workbuddy.cn"
                            | "codebuddy.cn"
                            | "copilot.tencent.com"
                    )
                ),
                "global" => matches!(
                    u.host_str(),
                    Some("www.workbuddy.ai" | "www.codebuddy.ai" | "workbuddy.ai" | "codebuddy.ai")
                ),
                _ => false,
            }
    })
}
fn view(f: &Flow, connected: bool) -> Value {
    let message = match f.phase.as_str() {
        "waiting" => WAITING,
        "authorized" | "connected" => "Buddy 登录完成，余额与 CodeBuddy / WorkBuddy 共用",
        "failed" => FAILED,
        "expired" => EXPIRED,
        "cancelled" => "已取消",
        _ => NOTICE,
    };
    json!({"id":f.id,"phase":if f.phase.is_empty(){"idle"}else{&f.phase},"provider":f.provider,"message":message,"authUrl":f.url,"expiresAt":f.expires_at,"connected":connected,"notice":NOTICE})
}
fn poll(
    state: Shared,
    client: reqwest::blocking::Client,
    base: String,
    region: String,
    auth_state: String,
    interval: Duration,
) {
    let result = (|| -> Result<()> {
        let endpoint_domain = reqwest::Url::parse(&base)?
            .host_str()
            .context(FAILED)?
            .to_owned();
        let mut token: Option<Value> = None;
        loop {
            std::thread::sleep(interval);
            {
                let mut f = state.lock().unwrap();
                if f.phase != "waiting" {
                    return Ok(());
                }
                if now() >= f.expires_at {
                    f.phase = "expired".into();
                    f.url = None;
                    return Ok(());
                }
            }
            if token.is_none() {
                let v = response(
                    client
                        .get(format!("{base}/v2/auth/token"))
                        .query(&[("state", &auth_state)])
                        .header("X-Domain", &endpoint_domain)
                        .header("X-Product", "SaaS"),
                )?;
                if v["code"] == 11217 {
                    continue;
                }
                token = Some(data(&v)?.clone());
            }
            let token = token.as_ref().unwrap();
            let access = token["accessToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context(FAILED)?;
            let mut request = client
                .get(format!("{base}/v2/login/account"))
                .query(&[("state", &auth_state)])
                .bearer_auth(access)
                .header("X-Domain", &endpoint_domain)
                .header("X-Product", "SaaS");
            if let Some(domain) = token["domain"].as_str() {
                request = request.header("X-Domain", domain);
            }
            let v = response(request)?;
            if v["code"] == 12151 {
                continue;
            }
            let account = data(&v)?;
            let mut login = Login {
                token: access.into(),
                region: region.clone(),
                uid: account["uid"].as_str().context(FAILED)?.into(),
                nickname: account["nickname"]
                    .as_str()
                    .unwrap_or("Buddy")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(128)
                    .collect(),
                enterprise: account["enterpriseId"].as_str().unwrap_or_default().into(),
                domain: token["domain"].as_str().unwrap_or(&endpoint_domain).into(),
                // Official calculateExpiresAt uses expiresIn seconds. Unknown expiry is not infinite.
                expires_at: now()
                    + token["expiresIn"]
                        .as_i64()
                        .filter(|n| *n > 0)
                        .unwrap_or(3600)
                        .min(86400 * 90),
            };
            if login.nickname.trim().is_empty() {
                login.nickname = "Buddy".into();
            }
            ensure!(login.valid(), FAILED);
            let mut f = state.lock().unwrap();
            if f.phase == "waiting" {
                f.login = Some(login);
                f.phase = "authorized".into();
                f.url = None;
            }
            return Ok(());
        }
    })();
    if result.is_err() {
        let mut f = state.lock().unwrap();
        if f.phase == "waiting" {
            f.phase = "failed".into();
            f.url = None;
        }
    }
}
fn start(
    proxy: &ProxyConfig,
    provider: &str,
    region: &str,
    base: &str,
    interval: Duration,
) -> Result<Shared> {
    let client = client(proxy)?;
    let platform = if region == "global" {
        "workbuddy-ai"
    } else {
        "workbuddy"
    };
    let endpoint_domain = reqwest::Url::parse(base)?
        .host_str()
        .context(FAILED)?
        .to_owned();
    let v = response(
        client
            .post(format!("{base}/v2/auth/state"))
            .header("X-Domain", endpoint_domain)
            .query(&[("platform", platform)])
            .header("X-Product", "SaaS")
            .json(&json!({})),
    )?;
    let d = data(&v)?;
    let auth_state = d["state"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .context(FAILED)?
        .to_owned();
    let url = d["authUrl"].as_str().context(FAILED)?;
    ensure!(safe_login_url(url, region), "Buddy 授权网址无效");
    let state = Arc::new(Mutex::new(Flow {
        id: crate::codex_auth::files::new_id(),
        provider: provider.into(),
        phase: "waiting".into(),
        url: Some(url.into()),
        expires_at: now() + 300,
        ..Default::default()
    }));
    let worker = state.clone();
    let base = base.to_owned();
    let region = region.to_owned();
    std::thread::spawn(move || poll(worker, client, base, region, auth_state, interval));
    Ok(state)
}
fn is_buddy_source(s: &Source) -> bool {
    matches!(s.provider.as_str(), "workbuddy" | "codebuddy")
        && s.subscription
            .as_ref()
            .is_some_and(|c| matches!(c.mode.as_str(), "buddy-cn" | "buddy-global"))
}

/// Historical snapshots stay in storage when a shared login moves products.
/// Display its server balance once while preserving each account's local usage.
pub(crate) fn dedup_balances(settings: &Settings, quotas: &mut [QuotaSnapshot]) {
    let pool = |q: &QuotaSnapshot| {
        settings
            .accounts
            .iter()
            .find(|a| {
                matches!(a.provider.as_str(), "codebuddy" | "workbuddy")
                    && a.provider == q.provider
                    && a.id == q.account_id
            })
            .and_then(|a| a.upstream_identity.as_deref())
            .filter(|id| id.starts_with("tencent:"))
    };
    let mut owners = std::collections::BTreeMap::new();
    for (index, q) in quotas.iter().enumerate() {
        let Some(pool) = pool(q) else { continue };
        if !q.meters.iter().any(|m| m.id.starts_with("buddy-")) {
            continue;
        }
        let active = settings.sources.iter().any(|s| {
            is_buddy_source(s)
                && s.provider == q.provider
                && s.account_id == q.account_id
                && settings.source_enabled(s)
                && settings.quota_enabled(s)
        });
        let rank = (
            active,
            q.error.is_none(),
            q.snapshot_at.unwrap_or(q.updated_at),
        );
        let owner = owners.entry(pool).or_insert((index, rank, q.name.clone()));
        if rank > owner.1 {
            *owner = (index, rank, q.name.clone());
        }
    }
    for (index, q) in quotas.iter_mut().enumerate() {
        let Some((owner, _, name)) = pool(q).and_then(|p| owners.get(p)) else {
            continue;
        };
        if index == *owner {
            continue;
        }
        q.meters.retain(|m| !m.id.starts_with("buddy-"));
        q.credits = None;
        q.plan = None;
        q.notice = Some(format!("与 {name} 共享 Buddy 余额，余额只在该账户显示"));
    }
}

impl Engine {
    fn store_buddy_login(&mut self, provider: &str, login: &Login) -> Result<()> {
        let base = self.store.settings()?;
        let mut next = base.clone();
        let upstream = login.identity();
        // One server pool has one owner across both products. Local Credits stay
        // on each product's own account; never duplicate a shared balance card.
        let previous: Vec<_> = next
            .sources
            .iter()
            .filter(|s| {
                is_buddy_source(s)
                    && next.accounts.iter().any(|a| {
                        a.id == s.account_id
                            && a.provider == s.provider
                            && a.upstream_identity.as_deref() == Some(&upstream)
                    })
            })
            .cloned()
            .collect();
        let active = previous
            .iter()
            .find(|s| next.source_enabled(s) && next.quota_enabled(s));
        let owner = active.map_or(provider, |s| s.provider.as_str()).to_owned();
        let provider = owner.as_str();
        let native_identity = format!("{provider}:{}", hash(&login.uid));
        let account_id = active
            .map(|s| s.account_id.clone())
            .or_else(|| {
                next.accounts
                    .iter()
                    .filter(|a| {
                        a.provider == provider && a.upstream_identity.as_deref() == Some(&upstream)
                    })
                    .min_by_key(|a| a.archived)
                    .map(|a| a.id.clone())
            })
            .or_else(|| {
                next.accounts
                    .iter()
                    .find(|a| {
                        a.provider == provider
                            && a.identity_key.as_deref() == Some(&native_identity)
                            && a.upstream_identity.is_none()
                            && !a.archived
                    })
                    .map(|a| a.id.clone())
            })
            .unwrap_or_else(crate::codex_auth::files::new_id);
        // Native history may only name a uid. Reuse it for the first verified
        // pool, but never overwrite that pool with another region or enterprise.
        let identity_key =
            if next.accounts.iter().any(|a| {
                a.provider == provider && a.identity_key.as_deref() == Some(&native_identity)
            }) {
                format!("{provider}:buddy:{}", hash(&upstream))
            } else {
                native_identity
            };
        let reference = format!("buddy-{}", crate::codex_auth::files::new_id());
        credentials::save_agent(&reference, &serde_json::to_string(login)?)?;
        let mut source = active
            .or(previous.first())
            .cloned()
            .unwrap_or_else(|| Source {
                id: crate::codex_auth::files::new_id(),
                provider: provider.into(),
                account_id: account_id.clone(),
                ..Default::default()
            });
        source.provider = provider.into();
        source.account_id = account_id.clone();
        source.name = login.nickname.clone();
        source.enabled = true;
        source.subscription = Some(SubscriptionConfig {
            mode: format!("buddy-{}", login.region),
            credential_ref: reference.clone(),
            ..Default::default()
        });
        // Move the existing source when its product/account was paused. Keeping
        // a second source would query the same pool again when it is re-enabled.
        for account in &mut next.accounts {
            if account
                .quota_source_id
                .as_ref()
                .is_some_and(|id| previous.iter().any(|s| &s.id == id))
            {
                account.quota_source_id = None;
            }
        }
        if let Some(account) = next
            .accounts
            .iter_mut()
            .find(|a| a.provider == provider && a.id == account_id)
        {
            account.upstream_identity = Some(upstream);
            account.quota_source_id = Some(source.id.clone());
            account.archived = false;
            account.quota_enabled = true;
        } else {
            next.accounts.push(Account {
                provider: provider.into(),
                id: account_id,
                name: login.nickname.clone(),
                identity_key: Some(identity_key),
                upstream_identity: Some(upstream),
                quota_source_id: Some(source.id.clone()),
                ..Default::default()
            });
        }
        if let Some(agent) = next.agents.iter_mut().find(|a| a.provider == provider)
            && !agent.machine_ids.iter().any(|id| id == "local")
        {
            agent.machine_ids.push("local".into());
        }
        next.sources
            .retain(|s| !previous.iter().any(|p| s.id == p.id));
        next.sources.push(source);
        if let Err(error) = self.store.patch_settings(&json!(base), &json!(next), false) {
            let _ = credentials::delete_agent(&reference);
            return Err(error);
        }
        for old in previous.iter().filter_map(|s| s.subscription.as_ref()) {
            let _ = credentials::delete_agent(&old.credential_ref);
        }
        Ok(())
    }
    pub(crate) fn buddy_auth_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let connected = settings
            .sources
            .iter()
            .any(|s| is_buddy_source(s) && s.enabled);
        if method == "buddyAuth.start" {
            let provider = params["provider"].as_str().unwrap_or("");
            ensure!(
                matches!(provider, "codebuddy" | "workbuddy")
                    && settings
                        .agents
                        .iter()
                        .any(|a| a.provider == provider && a.enabled),
                "请先启用此 Agent"
            );
            let region = params["region"].as_str().unwrap_or("cn");
            let state = start(
                &settings.proxy,
                provider,
                region,
                region_base(region)?,
                Duration::from_secs(2),
            )?;
            let mut slot = flow().lock().unwrap();
            {
                let mut old = slot.lock().unwrap();
                old.phase = "cancelled".into();
                old.login = None;
                old.url = None;
            }
            *slot = state.clone();
            return Ok(view(&state.lock().unwrap(), connected));
        }
        let state = flow().lock().unwrap().clone();
        let mut f = state.lock().unwrap();
        if method != "buddyAuth.logout"
            && (method != "buddyAuth.status"
                || params["id"].as_str().is_some_and(|id| !id.is_empty()))
        {
            ensure!(
                params["id"].as_str() == Some(f.id.as_str()),
                "Buddy 登录请求已变化"
            );
        }
        match method {
            "buddyAuth.status" => {
                if f.phase == "authorized" {
                    let login = f.login.as_ref().context(FAILED)?;
                    self.store_buddy_login(&f.provider, login)?;
                    f.phase = "connected".into();
                    f.login = None;
                    return Ok(view(&f, true));
                }
            }
            "buddyAuth.cancel" => {
                f.phase = "cancelled".into();
                f.login = None;
                f.url = None;
            }
            "buddyAuth.logout" => {
                // Logout all saved Buddy balance logins, without touching native agents.
                let sources: Vec<_> = settings
                    .sources
                    .iter()
                    .filter(|s| is_buddy_source(s))
                    .collect();
                for s in &sources {
                    credentials::delete_agent(&s.subscription.as_ref().unwrap().credential_ref)?;
                }
                let mut next = settings.clone();
                next.sources.retain(|s| !is_buddy_source(s));
                for a in &mut next.accounts {
                    if a.quota_source_id
                        .as_ref()
                        .is_some_and(|id| sources.iter().any(|s| &s.id == id))
                    {
                        a.quota_source_id = None;
                    }
                }
                self.store
                    .patch_settings(&json!(settings), &json!(next), false)?;
                f.phase = "idle".into();
                f.login = None;
                f.url = None;
                return Ok(view(&f, false));
            }
            _ => bail!("未知 Buddy 授权操作"),
        }
        Ok(view(&f, connected))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    pub(crate) fn mock(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = vec![];
            for reply in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    request.push_str(&line);
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                request.push_str(&String::from_utf8(body).unwrap());
                requests.push(request);
                let body = reply.to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
            requests
        });
        (base, thread)
    }
    pub(crate) fn login() -> Login {
        Login {
            token: "BUDDY_FIXTURE_SECRET".into(),
            region: "cn".into(),
            uid: "fixture-user".into(),
            nickname: "Fixture Buddy".into(),
            enterprise: String::new(),
            domain: "copilot.tencent.com".into(),
            expires_at: now() + 3600,
        }
    }

    fn engine() -> (tempfile::TempDir, Engine) {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path()).unwrap();
        let mut settings = engine.store.settings().unwrap();
        for agent in &mut settings.agents {
            if matches!(agent.provider.as_str(), "workbuddy" | "codebuddy") {
                agent.enabled = true;
                agent.machine_ids = vec!["local".into()];
            }
        }
        settings.accounts.push(Account {
            provider: "workbuddy".into(),
            id: "native-buddy".into(),
            name: "Native Buddy".into(),
            identity_key: Some(format!("workbuddy:{}", hash("fixture-user"))),
            ..Default::default()
        });
        engine.store.save_settings(&settings).unwrap();
        (root, engine)
    }

    fn remember_balance(engine: &Engine, source: &Source, remaining: f64) {
        engine
            .store
            .quota(&QuotaSnapshot {
                source_id: source.id.clone(),
                account_id: source.account_id.clone(),
                provider: source.provider.clone(),
                updated_at: now() - 60,
                plan: Some("Buddy 免费套餐".into()),
                meters: vec![
                    QuotaMeter {
                        id: "buddy-summary-0".into(),
                        remaining: Some(remaining),
                        unit: "credits".into(),
                        ..Default::default()
                    },
                    QuotaMeter {
                        id: "month-credits".into(),
                        used: 12.,
                        unit: "credits".into(),
                        estimated: true,
                        ..Default::default()
                    },
                ],
                credits: Some(CreditsSnapshot {
                    has_credits: true,
                    balance: Some(remaining.to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn buddy_same_uid_in_different_regions_or_enterprises_keeps_history() {
        let (_root, mut engine) = engine();
        let mut first = login();
        engine.store_buddy_login("workbuddy", &first).unwrap();
        let saved = engine.store.settings().unwrap();
        let original = saved
            .sources
            .iter()
            .find(|s| is_buddy_source(s))
            .unwrap()
            .clone();
        assert_eq!(original.account_id, "native-buddy");
        remember_balance(&engine, &original, 75.);

        first.enterprise = "enterprise-two".into();
        engine.store_buddy_login("workbuddy", &first).unwrap();
        first.region = "global".into();
        first.domain = "www.codebuddy.ai".into();
        engine.store_buddy_login("workbuddy", &first).unwrap();
        let settings = engine.store.settings().unwrap();
        let accounts: Vec<_> = settings
            .accounts
            .iter()
            .filter(|a| {
                a.upstream_identity
                    .as_deref()
                    .is_some_and(|id| id.starts_with("tencent:"))
            })
            .collect();
        assert_eq!(
            accounts.len(),
            3,
            "a uid alone cannot identify a regional or enterprise pool"
        );
        assert_eq!(
            accounts
                .iter()
                .map(|a| &a.identity_key)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            accounts
                .iter()
                .find(|a| a.id == original.account_id)
                .unwrap()
                .upstream_identity
                .as_deref(),
            Some(login().identity().as_str())
        );
        assert!(accounts.iter().all(|a| {
            settings.sources.iter().any(|s| {
                s.account_id == a.id && a.quota_source_id.as_deref() == Some(s.id.as_str())
            })
        }));
        assert_eq!(
            crate::account_scan::last_snapshot(
                &engine.store,
                accounts
                    .iter()
                    .find(|a| a.id == original.account_id)
                    .unwrap()
            )
            .unwrap()
            .meters[0]
                .remaining,
            Some(75.)
        );
    }

    #[test]
    fn buddy_relogin_reassigns_an_inactive_owner_without_duplicate_balance_queries() {
        for inactive in ["agent", "archived", "quota"] {
            let (_root, mut engine) = engine();
            engine.store_buddy_login("workbuddy", &login()).unwrap();
            let mut settings = engine.store.settings().unwrap();
            let original = settings
                .sources
                .iter()
                .find(|s| is_buddy_source(s))
                .unwrap()
                .clone();
            let old_reference = original
                .subscription
                .as_ref()
                .unwrap()
                .credential_ref
                .clone();
            remember_balance(&engine, &original, 75.);
            if inactive == "agent" {
                settings
                    .agents
                    .iter_mut()
                    .find(|a| a.provider == "workbuddy")
                    .unwrap()
                    .enabled = false;
            } else {
                let old = settings
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == original.account_id)
                    .unwrap();
                if inactive == "archived" {
                    old.archived = true;
                } else {
                    old.quota_enabled = false;
                }
            }
            engine.store.save_settings(&settings).unwrap();
            engine.store_buddy_login("codebuddy", &login()).unwrap();
            let mut settings = engine.store.settings().unwrap();
            let sources: Vec<_> = settings
                .sources
                .iter()
                .filter(|s| is_buddy_source(s))
                .cloned()
                .collect();
            assert_eq!(sources.len(), 1);
            assert_eq!(
                sources[0].provider, "codebuddy",
                "a {inactive} owner cannot hide the new login"
            );
            assert!(settings.source_enabled(&sources[0]) && settings.quota_enabled(&sources[0]));
            let old = settings
                .accounts
                .iter_mut()
                .find(|a| a.id == original.account_id)
                .unwrap();
            assert!(old.quota_source_id.is_none());
            assert_eq!(old.archived, inactive == "archived");
            old.archived = false;
            old.quota_enabled = true;
            settings
                .agents
                .iter_mut()
                .find(|a| a.provider == "workbuddy")
                .unwrap()
                .enabled = true;
            engine.store.save_settings(&settings).unwrap();
            assert!(credentials::read_agent(&old_reference).is_err());
            remember_balance(&engine, &sources[0], 50.);
            let dashboard = engine.store.dashboard(&Filter::default()).unwrap();
            assert_eq!(
                dashboard.quotas.len(),
                2,
                "both historical product accounts remain visible"
            );
            let meters: Vec<_> = dashboard.quotas.iter().flat_map(|q| &q.meters).collect();
            assert_eq!(
                meters.iter().filter(|m| m.id.starts_with("buddy-")).count(),
                1
            );
            assert_eq!(meters.iter().filter(|m| m.id == "month-credits").count(), 2);
            let old = settings
                .accounts
                .iter()
                .find(|a| a.id == original.account_id)
                .unwrap();
            assert_eq!(
                crate::account_scan::last_snapshot(&engine.store, old)
                    .unwrap()
                    .meters[0]
                    .remaining,
                Some(75.),
                "display dedup must preserve the stored historical balance"
            );
        }
    }

    #[test]
    fn buddy_logout_keeps_the_last_balance_visible_without_a_native_connection() {
        let (_root, mut engine) = engine();
        engine.store_buddy_login("workbuddy", &login()).unwrap();
        let settings = engine.store.settings().unwrap();
        let source = settings
            .sources
            .iter()
            .find(|s| is_buddy_source(s))
            .unwrap();
        remember_balance(&engine, source, 75.);
        engine
            .buddy_auth_call("buddyAuth.logout", json!({}))
            .unwrap();
        let dashboard = engine.store.dashboard(&Filter::default()).unwrap();
        let quota = dashboard
            .quotas
            .iter()
            .find(|q| q.account_id == source.account_id)
            .expect("logout preserves the last known balance");
        assert_eq!(quota.meters[0].remaining, Some(75.));
        assert!(quota.stale_since.is_some());
    }
    #[test]
    fn buddy_polls_pending_token_and_account_without_exposing_credentials() {
        let (base, server) = mock(vec![
            json!({"code":0,"data":{"state":"fixture-state","authUrl":"https://www.workbuddy.cn/login?state=fixture-state"}}),
            json!({"code":11217}),
            json!({"code":0,"data":{"accessToken":"BUDDY_FIXTURE_SECRET","expiresIn":3600}}),
            json!({"code":12151}),
            json!({"code":0,"data":{"uid":"fixture-user","nickname":"Fixture Buddy"}}),
        ]);
        let state = start(
            &ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            "workbuddy",
            "cn",
            &base,
            Duration::from_millis(2),
        )
        .unwrap();
        let requests = server.join().unwrap();
        for _ in 0..100 {
            if state.lock().unwrap().phase != "waiting" {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let f = state.lock().unwrap();
        assert_eq!(f.phase, "authorized");
        assert_eq!(f.login.as_ref().unwrap().uid, "fixture-user");
        assert!(!view(&f, false).to_string().contains("BUDDY_FIXTURE_SECRET"));
        assert!(view(&f, false)["authUrl"].is_null());
        assert!(requests[0].starts_with("POST /v2/auth/state?platform=workbuddy"));
        assert!(requests[1].starts_with("GET /v2/auth/token?state=fixture-state"));
        assert!(
            requests[3]
                .to_lowercase()
                .contains("authorization: bearer buddy_fixture_secret")
        );
    }
    #[test]
    fn buddy_cancelled_flow_never_pollutes_a_later_login() {
        let (base, server) = mock(vec![
            json!({"code":0,"data":{"state":"fixture","authUrl":"https://www.workbuddy.cn/login"}}),
        ]);
        let state = start(
            &ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            "workbuddy",
            "cn",
            &base,
            Duration::from_millis(50),
        )
        .unwrap();
        state.lock().unwrap().phase = "cancelled".into();
        assert_eq!(server.join().unwrap().len(), 1);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(state.lock().unwrap().phase, "cancelled");
        assert!(state.lock().unwrap().login.is_none());
    }
    #[test]
    fn buddy_save_reuses_native_identity_and_deduplicates_the_two_products() {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(root.path()).unwrap();
        let mut settings = engine.store.settings().unwrap();
        for agent in &mut settings.agents {
            if matches!(agent.provider.as_str(), "workbuddy" | "codebuddy") {
                agent.enabled = true;
                agent.machine_ids = vec!["local".into()];
            }
        }
        settings.accounts.push(Account {
            provider: "workbuddy".into(),
            id: "local-buddy".into(),
            name: "Existing Buddy".into(),
            identity_key: Some(format!("workbuddy:{}", hash("fixture-user"))),
            ..Default::default()
        });
        engine.store.save_settings(&settings).unwrap();
        let login = login();
        engine.store_buddy_login("workbuddy", &login).unwrap();
        engine.store_buddy_login("codebuddy", &login).unwrap();
        let saved = engine.store.settings().unwrap();
        let sources: Vec<_> = saved
            .sources
            .iter()
            .filter(|s| is_buddy_source(s))
            .collect();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].account_id, "local-buddy");
        let reference = &sources[0].subscription.as_ref().unwrap().credential_ref;
        assert!(
            credentials::read_agent(reference)
                .unwrap()
                .contains("BUDDY_FIXTURE_SECRET")
        );
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("BUDDY_FIXTURE_SECRET")
        );
        assert!(credentials::read(reference).is_err());
        // An expired/missing optional credential preserves the last successful
        // balance instead of replacing it with zero or just local usage.
        engine
            .store
            .quota(&QuotaSnapshot {
                source_id: sources[0].id.clone(),
                account_id: "local-buddy".into(),
                provider: "workbuddy".into(),
                name: "Fixture Buddy".into(),
                updated_at: now() - 60,
                meters: vec![QuotaMeter {
                    id: "buddy-plan".into(),
                    used: 25.,
                    total: Some(100.),
                    remaining: Some(75.),
                    unit: "credits".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        credentials::delete_agent(reference).unwrap();
        let stale = engine
            .read_quotas_unlocked(&json!({"accountId":"local-buddy"}))
            .unwrap();
        assert_eq!(stale[0].meters[0].remaining, Some(75.));
        assert!(stale[0].stale_since.is_some());
        let result = engine
            .buddy_auth_call("buddyAuth.logout", json!({}))
            .unwrap();
        assert_eq!(result["connected"], false);
        assert!(credentials::read_agent(reference).is_err());
        assert!(
            engine
                .store
                .settings()
                .unwrap()
                .accounts
                .iter()
                .any(|a| a.id == "local-buddy")
        );
        engine
            .buddy_auth_call("buddyAuth.logout", json!({}))
            .unwrap();
    }
    #[test]
    fn buddy_login_urls_and_headers_do_not_cross_regions_or_accept_injection() {
        assert!(safe_login_url(
            "https://www.workbuddy.cn/login?state=abc",
            "cn"
        ));
        for url in [
            "http://www.workbuddy.cn/login",
            "https://www.workbuddy.cn.evil.test/login",
            "https://www.workbuddy.ai/login",
            "https://secret@www.workbuddy.cn/login",
        ] {
            assert!(!safe_login_url(url, "cn"));
        }
        let mut token = login();
        token.uid = "user\r\nX-Evil: true".into();
        assert!(!token.valid());
        assert!(region_base("unknown").is_err());
    }
}
