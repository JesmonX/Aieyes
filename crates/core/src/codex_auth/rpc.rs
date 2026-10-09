//! A bidirectional, bounded app-server connection. Auth data never enters diagnostics.
use crate::{
    models::{ProxyConfig, hash},
    network, process,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub struct Rpc {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Value>,
    next: u64,
    notifications: std::collections::VecDeque<Value>,
}
impl Rpc {
    pub fn start(
        binary: &str,
        root: &std::path::Path,
        proxy: &ProxyConfig,
        managed: bool,
    ) -> Result<Self> {
        let resolved = crate::quota::resolve_codex(binary);
        if !managed {
            let mut version = process::cli_command(&resolved);
            version.arg("--version");
            let value = process::run(version, vec![], Duration::from_secs(5))
                .map_err(|_| anyhow::anyhow!("接口不兼容：无法确认 Codex CLI 版本"))?;
            ensure!(
                external_version_supported(&String::from_utf8_lossy(&value)),
                "接口不兼容：内存凭据查询需要 Codex 0.99.0 或更新版本"
            );
        }
        let mut command = process::cli_command(&resolved);
        command
            .args([
                "-c",
                if managed {
                    "cli_auth_credentials_store=\"file\""
                } else {
                    "cli_auth_credentials_store=\"ephemeral\""
                },
                "app-server",
            ])
            .env("CODEX_HOME", root)
            .current_dir(root);
        for key in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "CODEX_ACCESS_TOKEN",
            "CHATGPT_ACCESS_TOKEN",
        ] {
            command.env_remove(key);
        }
        network::apply_env(&mut command, proxy);
        Self::command(command)
    }
    pub fn command(mut command: Command) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        process::prepare(&mut command);
        let mut child = command.spawn().context("无法启动 Codex，请检查 CLI 路径")?;
        let input = child.stdin.take().context("无法打开 Codex 输入")?;
        let stdout = child.stdout.take().context("无法打开 Codex 输出")?;
        let (tx, messages) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                // Bounded per-line parsing avoids retaining unlimited process output.
                let mut limited = (&mut reader).take(4 * 1024 * 1024 + 1);
                if limited
                    .read_until(b'\n', &mut line)
                    .ok()
                    .is_none_or(|n| n == 0 || n > 4 * 1024 * 1024)
                {
                    break;
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    break;
                }
            }
        });
        let mut rpc = Self {
            child,
            input,
            messages,
            next: 0,
            notifications: Default::default(),
        };
        rpc.call("initialize", json!({"clientInfo":{"name":"aieyes","title":"Aieyes","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), &mut |_| Ok(None))?;
        rpc.send(&json!({"method":"initialized"}))?;
        Ok(rpc)
    }
    pub fn send(&mut self, value: &Value) -> Result<()> {
        writeln!(self.input, "{value}").map_err(|_| anyhow::anyhow!("Codex 连接已断开"))?;
        self.input.flush().context("Codex 连接已断开")
    }
    pub fn receive(&mut self, timeout: Duration) -> Result<Value> {
        if let Some(value) = self.notifications.pop_front() {
            return Ok(value);
        }
        self.wire(timeout)
    }
    fn wire(&self, timeout: Duration) -> Result<Value> {
        self.messages.recv_timeout(timeout).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!("Codex 响应超时"),
            _ => anyhow::anyhow!("Codex 连接已断开"),
        })
    }
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        refresh: &mut dyn FnMut(&Value) -> Result<Option<Value>>,
    ) -> Result<Value> {
        self.next += 1;
        let request = self.next;
        self.send(&json!({"id":request,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let value = self.wire(deadline.saturating_duration_since(Instant::now()))?;
            if value.get("method").is_some() {
                if value.get("id").is_some() {
                    let result = if value["method"] == "account/chatgptAuthTokens/refresh" {
                        refresh(&value["params"])?
                    } else {
                        None
                    };
                    let response = match result {
                        Some(result) => json!({"id":value["id"],"result":result}),
                        None => {
                            json!({"id":value["id"],"error":{"code":-32000,"message":"Credential refresh deferred by host"}})
                        }
                    };
                    self.send(&response)?;
                } else if self.notifications.len() < 64 {
                    self.notifications.push_back(value);
                }
                continue;
            }
            if value["id"] != request {
                continue;
            }
            if value.get("error").is_some() {
                let code = value["error"]["code"].as_i64().unwrap_or(-32000);
                // Never reflect upstream error text: it may include token/URL arguments.
                let message = value["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase();
                let kind = if [
                    "401",
                    "unauthorized",
                    "refresh token",
                    "not authenticated",
                    "not logged in",
                ]
                .iter()
                .any(|v| message.contains(v))
                {
                    "认证已失效"
                } else if [-32601, -32602].contains(&code) || message.contains("experimental") {
                    "接口不兼容"
                } else {
                    "请求失败"
                };
                anyhow::bail!("Codex {method}：{kind}（{code}）");
            }
            return value.get("result").cloned().context("Codex 响应缺少结果");
        }
    }
    pub fn login_external(&mut self, auth: &Value) -> Result<Value> {
        self.call(
            "account/login/start",
            super::files::external(auth)?,
            &mut |_| Ok(None),
        )?;
        let account = self.call("account/read", json!({"refreshToken":false}), &mut |_| {
            Ok(None)
        })?;
        ensure!(
            account["account"]["type"] == "chatgpt",
            "查询进程未使用 ChatGPT 登录"
        );
        let ident = super::files::identity(auth)?;
        if let Some(workspace) = account["workspaceRouting"]["chatgptAccountId"].as_str() {
            ensure!(workspace == ident.workspace, "查询工作区与档案不一致");
        }
        if let Some(email) = account["account"]["email"].as_str() {
            ensure!(
                ident.email.is_empty() || email.eq_ignore_ascii_case(&ident.email),
                "查询用户与档案不一致"
            );
        }
        Ok(account)
    }
    pub fn quota(
        &mut self,
        auth: &Value,
        latest: &mut dyn FnMut() -> Result<Value>,
    ) -> Result<Value> {
        let ident = super::files::identity(auth)?;
        let old = hash(auth["tokens"]["access_token"].as_str().unwrap_or(""));
        self.call("account/rateLimits/read", Value::Null, &mut |p| {
            if let Some(previous) = p["previousAccountId"].as_str() { ensure!(previous == ident.workspace, "刷新请求工作区不一致"); }
            let next = latest()?;
            ensure!(super::files::identity(&next)?.key == ident.key, "账号已改变，已丢弃查询结果");
            if hash(next["tokens"]["access_token"].as_str().unwrap_or("")) == old { return Ok(None); }
            let e = super::files::external(&next)?;
            Ok(Some(json!({"accessToken":e["accessToken"],"chatgptAccountId":e["chatgptAccountId"],"chatgptPlanType":e["chatgptPlanType"]})))
        })
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        process::kill(&mut self.child);
    }
}

fn external_version_supported(version: &str) -> bool {
    version
        .split_whitespace()
        .find_map(|part| {
            let mut components = part.trim_start_matches('v').split('.');
            let major = components.next()?.parse::<u64>().ok()?;
            let minor = components.next()?.parse::<u64>().ok()?;
            Some(major > 0 || minor >= 99)
        })
        .unwrap_or(false)
}
