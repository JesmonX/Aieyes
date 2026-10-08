//! Private loopback transport for Sparkle. Sparkle still parses the original appcast,
//! verifies the original archive signature, and owns installation. No files are cached.
use crate::{network, store::Store};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Configuration {
    feed: String,
    token: String,
}

fn https_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "更新地址必须为 HTTPS"
    );
    Ok(url)
}

pub fn serve(root: &Path) -> Result<()> {
    let mut input = BufReader::new(io::stdin());
    let mut line = String::new();
    input.by_ref().take(8192).read_line(&mut line)?;
    let config: Configuration = serde_json::from_str(&line)?;
    ensure!(
        config.token.len() >= 32 && config.token.bytes().all(|c| c.is_ascii_hexdigit()),
        "无效的传输凭证"
    );
    let feed = https_url(&config.feed)?;
    // Read saved settings in this independent process, including at app startup.
    let proxy = Store::open(root)?.settings()?.proxy;
    let client = network::client_builder(&proxy)?.https_only(true).build()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    emit(
        json!({"endpoint":format!("http://127.0.0.1:{}/{}", listener.local_addr()?.port(), config.token), "mode":proxy.mode}),
    );
    // Parent closes stdin on cancellation/exit. Termination also interrupts blocked reads.
    std::thread::spawn(move || {
        let _ = io::copy(&mut input, &mut io::sink());
        std::process::exit(0);
    });
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = stream?;
        if active.fetch_add(1, Ordering::SeqCst) >= 4 {
            active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (client, token, feed, mode, active) = (
            client.clone(),
            config.token.clone(),
            feed.clone(),
            proxy.mode.clone(),
            active.clone(),
        );
        std::thread::spawn(move || {
            let _ = handle(stream, &client, &token, &feed, &mode);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}
fn emit(value: serde_json::Value) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}

fn route(target: &str, token: &str, feed: &Url) -> Result<(Url, bool)> {
    let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() == format!("/{token}/feed.xml") {
        return Ok((feed.clone(), true));
    }
    ensure!(
        url.path().starts_with(&format!("/{token}/archive/")),
        "无效的传输路径"
    );
    let value = url
        .query_pairs()
        .find(|(key, _)| key == "url")
        .context("缺少下载地址")?
        .1;
    Ok((https_url(&value)?, false))
}

fn handle(
    mut socket: TcpStream,
    client: &Client,
    token: &str,
    feed: &Url,
    mode: &str,
) -> Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut raw = Vec::new();
    // Bound headers and never accept a request body, redirects, or proxy CONNECT.
    while !raw.ends_with(b"\r\n\r\n") && raw.len() < 8192 {
        let mut byte = [0];
        socket.read_exact(&mut byte)?;
        raw.push(byte[0]);
    }
    ensure!(raw.ends_with(b"\r\n\r\n"), "请求头过长");
    let request = String::from_utf8(raw)?;
    let mut words = request.lines().next().unwrap_or("").split_whitespace();
    let method = words.next().unwrap_or("");
    let target = words.next().unwrap_or("");
    if method != "GET" || !target.starts_with('/') {
        write!(
            socket,
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    }
    let Ok((url, is_feed)) = route(target, token, feed) else {
        write!(
            socket,
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    };
    let started = Instant::now();
    let stage = if is_feed { "feed" } else { "archive" };
    let result = transfer(client, &url, is_feed, &mut socket, |attempt| {
        emit(
            json!({"stage":stage, "mode":mode, "attempt":attempt, "elapsedMs":started.elapsed().as_millis()}),
        );
    });
    match result {
        Ok(()) => emit(
            json!({"stage":stage,"mode":mode,"result":"ok","elapsedMs":started.elapsed().as_millis()}),
        ),
        Err(error) => {
            // Never emit URLs, tokens, or proxy credentials in diagnostics.
            let reason = if error.chain().any(|e| {
                e.downcast_ref::<reqwest::Error>()
                    .is_some_and(|e| e.is_timeout())
            }) {
                "更新连接超时，请检查应用代理后重试"
            } else {
                "更新传输失败，请检查应用代理或稍后重试"
            };
            emit(
                json!({"stage":stage,"mode":mode,"result":"error","message":reason,"elapsedMs":started.elapsed().as_millis()}),
            );
            // If streaming has started, close without a second HTTP response. The
            // original Content-Length or chunk terminator lets Sparkle detect truncation.
        }
    }
    Ok(())
}

fn transfer(
    client: &Client,
    url: &Url,
    is_feed: bool,
    output: &mut impl Write,
    mut attempt_event: impl FnMut(u8),
) -> Result<()> {
    for attempt in 1..=2 {
        attempt_event(attempt);
        let request = client
            .get(url.clone())
            .timeout(Duration::from_secs(if is_feed { 9 } else { 600 }));
        let result = request
            .send()
            .and_then(|response| response.error_for_status());
        let mut response = match result {
            Ok(response) => response,
            Err(error) => {
                if attempt == 1
                    && (error.is_timeout()
                        || error.is_connect()
                        || error.status().is_some_and(|s| s.is_server_error()))
                {
                    continue;
                }
                write!(
                    output,
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )?;
                return Err(error.into());
            }
        };
        if is_feed {
            let mut data = Vec::new();
            if let Err(error) = response
                .by_ref()
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut data)
            {
                if attempt == 1 {
                    continue;
                }
                write!(
                    output,
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )?;
                return Err(error.into());
            }
            ensure!(data.len() <= 4 * 1024 * 1024, "更新清单过大");
            write!(
                output,
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                data.len()
            )?;
            output.write_all(&data)?;
        } else {
            let length = response.content_length();
            write!(
                output,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nCache-Control: no-store\r\nConnection: close\r\n"
            )?;
            if let Some(length) = length {
                write!(output, "Content-Length: {length}\r\n\r\n")?;
            } else {
                write!(output, "Transfer-Encoding: chunked\r\n\r\n")?;
            }
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = response.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if length.is_none() {
                    write!(output, "{count:x}\r\n")?;
                }
                output.write_all(&buffer[..count])?;
                if length.is_none() {
                    write!(output, "\r\n")?;
                }
            }
            if length.is_none() {
                write!(output, "0\r\n\r\n")?;
            }
        }
        return Ok(());
    }
    bail!("更新传输失败")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_require_private_token_and_https() {
        let feed = Url::parse("https://example.org/feed.xml").unwrap();
        assert!(route("/secret/feed.xml", "secret", &feed).unwrap().1);
        assert!(route("/wrong/feed.xml", "secret", &feed).is_err());
        assert!(
            route(
                "/secret/archive/update.zip?url=file:///tmp/update.zip",
                "secret",
                &feed
            )
            .is_err()
        );
        assert!(
            route(
                "/secret/archive/update.zip?url=http://example.org/update.zip",
                "secret",
                &feed
            )
            .is_err()
        );
        assert!(
            route(
                "/secret/archive/update.zip?url=https://user:pass@example.org/update.zip",
                "secret",
                &feed
            )
            .is_err()
        );
        let (url, feed) = route(
            "/secret/archive/update.zip?url=https%3A%2F%2Fexample.org%2Fupdate.zip%3Fsignature%3Da%252Bb",
            "secret",
            &feed,
        )
        .unwrap();
        assert!(!feed);
        assert_eq!(
            url.as_str(),
            "https://example.org/update.zip?signature=a%2Bb"
        );
    }
    fn fixture(replies: Vec<&'static [u8]>) -> (Url, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = Url::parse(&format!("http://{}/update", listener.local_addr().unwrap())).unwrap();
        let thread = std::thread::spawn(move || {
            for reply in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                socket.write_all(reply).unwrap();
            }
        });
        (url, thread)
    }
    #[test]
    fn transient_feed_failure_retries_once_and_preserves_signed_bytes() {
        let (url, server) = fixture(vec![
            b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 19\r\nConnection: close\r\n\r\n<rss/>\n<!--sig-->\r\n",
        ]);
        let client = Client::builder().no_proxy().build().unwrap();
        let mut output = Vec::new();
        let mut attempts = Vec::new();
        transfer(&client, &url, true, &mut output, |n| attempts.push(n)).unwrap();
        server.join().unwrap();
        assert_eq!(attempts, [1, 2]);
        assert!(output.ends_with(b"<rss/>\n<!--sig-->\r\n"));
    }
    #[test]
    fn permanent_failure_does_not_retry() {
        let (url, server) = fixture(vec![
            b"HTTP/1.1 404 Missing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        let mut output = Vec::new();
        let mut attempts = Vec::new();
        assert!(
            transfer(
                &Client::builder().no_proxy().build().unwrap(),
                &url,
                true,
                &mut output,
                |n| attempts.push(n)
            )
            .is_err()
        );
        server.join().unwrap();
        assert_eq!(attempts, [1]);
        assert!(output.starts_with(b"HTTP/1.1 502"));
    }
    #[test]
    fn unknown_length_archives_are_chunked_without_modifying_the_payload() {
        let (url, server) = fixture(vec![
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\narchive-bytes",
        ]);
        let mut output = Vec::new();
        transfer(
            &Client::builder().no_proxy().build().unwrap(),
            &url,
            false,
            &mut output,
            |_| {},
        )
        .unwrap();
        server.join().unwrap();
        let body = output.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(&output[body..], b"d\r\narchive-bytes\r\n0\r\n\r\n");
    }
    #[test]
    fn truncated_archives_fail_without_retrying_or_appending_a_second_response() {
        let (url, server) = fixture(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\nConnection: close\r\n\r\npartial",
        ]);
        let mut output = Vec::new();
        let mut attempts = Vec::new();
        assert!(
            transfer(
                &Client::builder().no_proxy().build().unwrap(),
                &url,
                false,
                &mut output,
                |n| attempts.push(n)
            )
            .is_err()
        );
        server.join().unwrap();
        assert_eq!(attempts, [1]);
        assert_eq!(output.windows(8).filter(|w| *w == b"HTTP/1.1").count(), 1);
    }
}
