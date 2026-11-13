//! Bounded WebSockets over the selected HTTP/SOCKS proxy, or an existing local Unix socket.
use crate::models::ProxyConfig;
use anyhow::{Context, Result, ensure};
use base64::Engine;
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tungstenite::{WebSocket, protocol::WebSocketConfig};

pub(super) trait Wire: Read + Write + Send {}
impl<T: Read + Write + Send> Wire for T {}
type Stream = Box<dyn Wire>;
pub(super) type Socket = WebSocket<Stream>;

fn tls(stream: Stream, domain: &str) -> Result<Stream> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let config = CONFIG
        .get_or_init(|| {
            let roots =
                rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .expect("supported TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
            )
        })
        .clone();
    let name =
        rustls::pki_types::ServerName::try_from(domain.to_owned()).context("长连接主机名无效")?;
    Ok(Box::new(rustls::StreamOwned::new(
        rustls::ClientConnection::new(config, name)?,
        stream,
    )))
}

fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(2 * 1024 * 1024))
        .max_frame_size(Some(2 * 1024 * 1024))
}

fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    let mut last = None;
    for address in (host, port)
        .to_socket_addrs()
        .context("无法解析长连接主机")?
        .take(8)
    {
        match TcpStream::connect_timeout(&address, Duration::from_secs(8)) {
            Ok(socket) => {
                socket.set_read_timeout(Some(Duration::from_secs(8)))?;
                socket.set_write_timeout(Some(Duration::from_secs(8)))?;
                return Ok(socket);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("无法解析长连接主机")))
}

fn proxy_for(proxy: &ProxyConfig, host: &str) -> Option<String> {
    let p = crate::network::effective_proxy(proxy);
    match p.mode.as_str() {
        "direct" => None,
        "custom" => Some(p.url),
        _ => {
            let excluded = std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default();
            if excluded.split(',').map(str::trim).any(|value| {
                value == "*"
                    || host == value
                    || !value.is_empty()
                        && host.ends_with(&format!(".{}", value.trim_start_matches('.')))
            }) {
                return None;
            }
            ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
                .iter()
                .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        }
    }
}

pub(super) fn connect(url: &str, proxy: &ProxyConfig) -> Result<Socket> {
    let url = reqwest::Url::parse(url).context("长连接地址无效")?;
    ensure!(
        url.scheme() == "wss" && url.username().is_empty() && url.password().is_none(),
        "飞书长连接必须使用加密连接"
    );
    let host = url.host_str().context("长连接地址缺少主机")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let proxy = proxy_for(proxy, host)
        .map(|v| reqwest::Url::parse(&v))
        .transpose()
        .context("长连接代理地址无效")?;
    let socket = if let Some(p) = &proxy {
        tcp(
            p.host_str().context("代理地址缺少主机")?,
            p.port_or_known_default().unwrap_or(1080),
        )?
    } else {
        tcp(host, port)?
    };
    let timer = socket.try_clone()?;
    let mut stream: Stream = Box::new(socket);
    if let Some(p) = proxy {
        if p.scheme() == "https" {
            stream = tls(stream, p.host_str().unwrap())?;
        }
        match p.scheme() {
            "http" | "https" => tunnel_http(&mut stream, &p, host, port)?,
            "socks5" | "socks5h" => tunnel_socks(&mut stream, &p, host, port)?,
            _ => anyhow::bail!("长连接代理需要 HTTP、HTTPS 或 SOCKS5"),
        }
    }
    stream = tls(stream, host)?;
    let (ws, _) = tungstenite::client::client_with_config(url.as_str(), stream, Some(config()))
        .map_err(|_| anyhow::anyhow!("飞书长连接握手失败"))?;
    timer.set_read_timeout(Some(Duration::from_millis(250)))?;
    timer.set_write_timeout(Some(Duration::from_secs(3)))?;
    Ok(ws)
}

fn tunnel_http(stream: &mut Stream, proxy: &reqwest::Url, host: &str, port: u16) -> Result<()> {
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let auth = if proxy.username().is_empty() {
        String::new()
    } else {
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(format!(
                "{}:{}",
                decode(proxy.username())?,
                decode(proxy.password().unwrap_or(""))?
            ))
        )
    };
    write!(
        stream,
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}\r\n"
    )?;
    stream.flush()?;
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        ensure!(response.len() < 16384, "代理握手响应过大");
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        response.push(byte[0]);
    }
    let first = std::str::from_utf8(&response)?.lines().next().unwrap_or("");
    ensure!(
        first.split_whitespace().nth(1) == Some("200"),
        "代理拒绝长连接隧道"
    );
    Ok(())
}

fn decode(value: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let mut source = value.as_bytes().iter().copied();
    while let Some(b) = source.next() {
        if b == b'%' {
            let a = source.next().and_then(|v| (v as char).to_digit(16));
            let b = source.next().and_then(|v| (v as char).to_digit(16));
            bytes
                .push((a.context("代理凭据编码无效")? * 16 + b.context("代理凭据编码无效")?) as u8);
        } else {
            bytes.push(b);
        }
    }
    Ok(String::from_utf8(bytes)?)
}

fn tunnel_socks(stream: &mut Stream, proxy: &reqwest::Url, host: &str, port: u16) -> Result<()> {
    let auth = !proxy.username().is_empty();
    stream.write_all(if auth { &[5, 2, 0, 2] } else { &[5, 1, 0] })?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply)?;
    ensure!(reply[0] == 5, "SOCKS5 代理握手无效");
    if reply[1] == 2 && auth {
        let user = decode(proxy.username())?;
        let pass = decode(proxy.password().unwrap_or(""))?;
        ensure!(user.len() <= 255 && pass.len() <= 255, "SOCKS5 凭据过长");
        let mut request = vec![1, user.len() as u8];
        request.extend_from_slice(user.as_bytes());
        request.push(pass.len() as u8);
        request.extend_from_slice(pass.as_bytes());
        stream.write_all(&request)?;
        stream.read_exact(&mut reply)?;
        ensure!(reply == [1, 0], "SOCKS5 代理认证失败");
    } else {
        ensure!(reply[1] == 0, "SOCKS5 代理认证失败");
    }
    ensure!(host.len() <= 255, "长连接主机名过长");
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;
    let mut response = [0u8; 4];
    stream.read_exact(&mut response)?;
    ensure!(
        response[0] == 5 && response[1] == 0 && response[2] == 0,
        "SOCKS5 代理连接失败"
    );
    let size = match response[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut b = [0];
            stream.read_exact(&mut b)?;
            b[0] as usize
        }
        _ => anyhow::bail!("SOCKS5 代理响应无效"),
    };
    stream.read_exact(&mut vec![0; size + 2])?;
    Ok(())
}

#[cfg(unix)]
pub(super) fn unix(path: &std::path::Path) -> Result<Socket> {
    use std::os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    };
    let meta = std::fs::symlink_metadata(path).context("原会话控制接口不可用")?;
    ensure!(
        meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() },
        "会话控制接口身份不符"
    );
    let socket = UnixStream::connect(path).context("无法连接原会话控制接口")?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(3)))?;
    let timer = socket.try_clone()?;
    let (ws, _) = tungstenite::client::client_with_config(
        "ws://localhost/",
        Box::new(socket) as Stream,
        Some(config()),
    )
    .map_err(|_| anyhow::anyhow!("会话控制接口握手失败"))?;
    timer.set_read_timeout(Some(Duration::from_millis(150)))?;
    Ok(ws)
}

pub(super) fn timed_out(error: &tungstenite::Error) -> bool {
    matches!(error,tungstenite::Error::Io(e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut))
}
