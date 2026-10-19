use crate::models::ProxyConfig;
#[cfg(target_os = "macos")]
mod system_proxy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{process::Command, time::Duration};

pub fn default_test_urls() -> Vec<String> {
    ["https://api.github.com/rate_limit", "https://openrouter.ai"]
        .map(str::to_owned)
        .to_vec()
}
pub fn validate_test_urls(urls: &[String]) -> Result<()> {
    anyhow::ensure!((1..=5).contains(&urls.len()), "请配置 1–5 个测试地址");
    for value in urls {
        let url = reqwest::Url::parse(value).context("测试地址格式不正确")?;
        anyhow::ensure!(
            ["http", "https"].contains(&url.scheme())
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "测试地址需要为 HTTP/HTTPS URL"
        );
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteLatency {
    pub url: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkTest {
    pub tested_at: i64,
    pub mode: String,
    pub average_ms: Option<u64>,
    pub status: String,
    pub sites: Vec<SiteLatency>,
}
pub fn test(proxy: &ProxyConfig, urls: &[String]) -> Result<NetworkTest> {
    validate_test_urls(urls)?;
    let client = client(proxy)?;
    let sites = std::thread::scope(|scope| {
        let jobs: Vec<_> = urls
            .iter()
            .map(|url| {
                let client = &client;
                scope.spawn(move || {
                    let started = std::time::Instant::now();
                    let result = client.get(url).timeout(Duration::from_secs(8)).send();
                    match result {
                        Ok(response) if response.status().is_success() => SiteLatency {
                            url: url.clone(),
                            latency_ms: Some(started.elapsed().as_millis() as u64),
                            error: None,
                        },
                        Ok(response) => SiteLatency {
                            url: url.clone(),
                            latency_ms: None,
                            error: Some(format!("HTTP {}", response.status().as_u16())),
                        },
                        Err(error) => SiteLatency {
                            url: url.clone(),
                            latency_ms: None,
                            error: Some(
                                if error.is_timeout() {
                                    "连接超时"
                                } else {
                                    "连接失败"
                                }
                                .into(),
                            ),
                        },
                    }
                })
            })
            .collect();
        jobs.into_iter()
            .map(|j| j.join().unwrap())
            .collect::<Vec<_>>()
    });
    let successful: Vec<_> = sites.iter().filter_map(|s| s.latency_ms).collect();
    let status = if successful.len() == sites.len() {
        "ok"
    } else if successful.is_empty() {
        "failed"
    } else {
        "unstable"
    };
    Ok(NetworkTest {
        tested_at: crate::models::now(),
        mode: proxy.mode.clone(),
        average_ms: (!successful.is_empty())
            .then(|| successful.iter().sum::<u64>() / successful.len() as u64),
        status: status.into(),
        sites,
    })
}

pub fn effective_proxy(proxy: &ProxyConfig) -> ProxyConfig {
    effective_proxy_with_refresh(proxy, false)
}

pub(crate) fn effective_proxy_with_refresh(proxy: &ProxyConfig, force: bool) -> ProxyConfig {
    #[cfg(target_os = "macos")]
    if proxy.mode == "system" {
        return system_proxy::get(force);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = force;
    proxy.clone()
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ClientPolicy {
    pub redirects: usize,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub https_only: bool,
    pub extra_proxy: Option<String>,
}
impl Default for ClientPolicy {
    fn default() -> Self {
        Self {
            redirects: 10,
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(25),
            https_only: false,
            extra_proxy: None,
        }
    }
}
#[derive(PartialEq, Eq)]
struct ClientKey {
    mode: String,
    url: String,
    environment: Vec<Option<std::ffi::OsString>>,
    policy: ClientPolicy,
}
const PROXY_ENVIRONMENT: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];
pub fn client(proxy: &ProxyConfig) -> Result<reqwest::blocking::Client> {
    client_with_policy(proxy, ClientPolicy::default())
}
pub(crate) fn client_with_policy(
    proxy: &ProxyConfig,
    policy: ClientPolicy,
) -> Result<reqwest::blocking::Client> {
    use std::{
        collections::VecDeque,
        sync::{Mutex, OnceLock},
    };
    static CLIENTS: OnceLock<Mutex<VecDeque<(ClientKey, reqwest::blocking::Client)>>> =
        OnceLock::new();
    let proxy = effective_proxy(proxy);
    let key = ClientKey {
        mode: proxy.mode.clone(),
        url: proxy.url.clone(),
        environment: if proxy.mode == "system" {
            PROXY_ENVIRONMENT.iter().map(std::env::var_os).collect()
        } else {
            Vec::new()
        },
        policy,
    };
    let cache = CLIENTS.get_or_init(Default::default);
    {
        let mut clients = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = clients.iter().position(|(known, _)| known == &key) {
            let entry = clients.remove(index).unwrap();
            let client = entry.1.clone();
            clients.push_back(entry);
            return Ok(client);
        }
    }
    // TLS initialization and the last Client drop can start/join runtime
    // threads. Neither should block unrelated requests on the cache mutex.
    let mut builder = effective_builder(&proxy)?
        .connect_timeout(key.policy.connect_timeout)
        .timeout(key.policy.timeout)
        .https_only(key.policy.https_only)
        .redirect(if key.policy.redirects == 0 {
            reqwest::redirect::Policy::none()
        } else {
            reqwest::redirect::Policy::limited(key.policy.redirects)
        });
    if let Some(extra) = &key.policy.extra_proxy {
        builder = builder.proxy(reqwest::Proxy::all(extra)?);
    }
    let client = builder.build()?;
    let mut clients = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(index) = clients.iter().position(|(known, _)| known == &key) {
        let entry = clients.remove(index).unwrap();
        let existing = entry.1.clone();
        clients.push_back(entry);
        drop(clients);
        return Ok(existing);
    }
    let evicted = if clients.len() >= 16 {
        clients.pop_front()
    } else {
        None
    };
    clients.push_back((key, client.clone()));
    drop(clients);
    drop(evicted);
    Ok(client)
}

pub(crate) fn client_builder(proxy: &ProxyConfig) -> Result<reqwest::blocking::ClientBuilder> {
    effective_builder(&effective_proxy(proxy))
}
fn effective_builder(proxy: &ProxyConfig) -> Result<reqwest::blocking::ClientBuilder> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent("Aieyes/0.1.0");
    match proxy.mode.as_str() {
        "direct" => builder = builder.no_proxy(),
        "custom" => {
            builder = builder
                .no_proxy()
                .proxy(reqwest::Proxy::all(&proxy.url).context("代理地址格式不正确")?)
        }
        _ => {}
    }
    Ok(builder)
}

pub fn apply_env(command: &mut Command, proxy: &ProxyConfig) {
    let proxy = effective_proxy(proxy);
    if proxy.mode == "direct" || proxy.mode == "custom" {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            command.env_remove(key);
        }
    }
    if proxy.mode == "custom" {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(key, &proxy.url);
        }
        command.env("NO_PROXY", "localhost,127.0.0.1,::1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_clients_reuse_connections_and_isolate_headers_and_redirect_policy() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            sync::{Arc, Mutex},
            time::Instant,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let output = records.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut workers = Vec::new();
            while Instant::now() < deadline && output.lock().unwrap().len() < 3 {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection = workers.len();
                        let output = output.clone();
                        workers.push(std::thread::spawn(move || {
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut reader = BufReader::new(stream);
                            loop {
                                let mut authorization = String::new();
                                let mut line = String::new();
                                if reader.read_line(&mut line).ok().is_none_or(|n| n == 0) {
                                    break;
                                }
                                loop {
                                    line.clear();
                                    if reader.read_line(&mut line).ok().is_none_or(|n| n == 0) {
                                        return;
                                    }
                                    if line == "\r\n" {
                                        break;
                                    }
                                    if line.to_ascii_lowercase().starts_with("authorization:") {
                                        authorization = line.trim().into();
                                    }
                                }
                                output.lock().unwrap().push((connection, authorization));
                                if reader
                                    .get_mut()
                                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("{error}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        let proxy = ProxyConfig {
            mode: "direct".into(),
            url: String::new(),
        };
        let first = client(&proxy).unwrap();
        let second = client(&proxy).unwrap();
        for (client, token) in [(first, "synthetic-a"), (second, "synthetic-b")] {
            assert_eq!(
                client
                    .get(&url)
                    .header("Authorization", token)
                    .send()
                    .unwrap()
                    .text()
                    .unwrap(),
                "ok"
            );
            // Let Hyper return the fully consumed response to its idle pool.
            std::thread::sleep(Duration::from_millis(20));
        }
        client_with_policy(
            &proxy,
            ClientPolicy {
                redirects: 0,
                ..Default::default()
            },
        )
        .unwrap()
        .get(&url)
        .send()
        .unwrap()
        .text()
        .unwrap();
        server.join().unwrap();
        let rows = records.lock().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, rows[1].0);
        assert_ne!(rows[1].0, rows[2].0);
        assert!(rows[0].1.ends_with("synthetic-a"));
        assert!(rows[1].1.ends_with("synthetic-b"));
        assert!(rows[2].1.is_empty());
    }
}
