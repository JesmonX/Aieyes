use crate::models::ProxyConfig;
#[cfg(target_os = "macos")]
use crate::process;
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
    if proxy.mode != "system" {
        return proxy.clone();
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("/usr/sbin/scutil");
        cmd.arg("--proxy");
        if let Ok(raw) = process::run(cmd, vec![], Duration::from_secs(2)) {
            let text = String::from_utf8_lossy(&raw);
            let mut fields = std::collections::HashMap::new();
            for line in text.lines() {
                if let Some((key, value)) = line.trim().split_once(" : ") {
                    fields.insert(key, value);
                }
            }
            for (name, scheme) in [("HTTPS", "http"), ("HTTP", "http"), ("SOCKS", "socks5h")] {
                if fields.get(format!("{name}Enable").as_str()) == Some(&"1")
                    && let (Some(host), Some(port)) = (
                        fields.get(format!("{name}Proxy").as_str()),
                        fields.get(format!("{name}Port").as_str()),
                    )
                {
                    return ProxyConfig {
                        mode: "custom".into(),
                        url: format!("{scheme}://{host}:{port}"),
                    };
                }
            }
        }
    }
    proxy.clone()
}
pub fn client(proxy: &ProxyConfig) -> Result<reqwest::blocking::Client> {
    Ok(client_builder(proxy)?.build()?)
}

pub(crate) fn client_builder(proxy: &ProxyConfig) -> Result<reqwest::blocking::ClientBuilder> {
    let proxy = effective_proxy(proxy);
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
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

pub fn open_release_page() -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = Command::new("rundll32.exe");
    #[cfg(target_os = "windows")]
    command.arg("url.dll,FileProtocolHandler");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg("https://github.com/JesmonX/Aieyes/releases/latest");
    crate::process::prepare(&mut command);
    // Only a fixed product URL is passed to the OS; no shell or user arguments.
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("无法打开发布页")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
