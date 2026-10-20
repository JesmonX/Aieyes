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
    _registration: process::ChildRegistration,
    input: ChildStdin,
    messages: Receiver<Value>,
    next: u64,
    notifications: std::collections::VecDeque<Value>,
}

fn resolve_binary(binary: &str, pre_command: &str) -> String {
    if (binary.is_empty() || binary == "codex") && !pre_command.trim().is_empty() {
        "codex".into()
    } else {
        crate::quota::resolve_codex(binary)
    }
}

pub(super) fn inspect_cli(
    binary: &str,
    root: &std::path::Path,
    proxy: &ProxyConfig,
    pre_command: &str,
    shell: &str,
) -> Value {
    let resolved = resolve_binary(binary, pre_command);
    let mut command =
        process::cli_command_with_pre_command(&resolved, &["--version"], pre_command, shell);
    command.env("CODEX_HOME", root).current_dir(root);
    network::apply_env(&mut command, proxy);
    let mut result = json!({"resolvedBinary":resolved,"version":null,"cliAvailable":false});
    match process::run(command, vec![], Duration::from_secs(5)) {
        Ok(bytes) => {
            let output = String::from_utf8_lossy(&bytes);
            match external_version_supported(&output) {
                Some(supported) => {
                    let version = output
                        .split_whitespace()
                        .find(|p| {
                            p.trim_start_matches('v')
                                .split('.')
                                .take(3)
                                .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
                                && p.contains('.')
                        })
                        .unwrap_or("unknown");
                    result["version"] = json!(format!("codex-cli {version}"));
                    result["cliAvailable"] = json!(supported);
                    if !supported {
                        result["cliErrorCode"] = json!("incompatible");
                        result["cliError"] =
                            json!("接口不兼容：内存凭据查询需要 Codex 0.99.0 或更新版本");
                    }
                }
                None => {
                    result["cliErrorCode"] = json!("invalidVersion");
                    result["cliError"] =
                        json!("无法识别 Codex CLI 版本输出，请检查“Codex 程序”是否指向正确的 CLI");
                }
            }
        }
        Err(error) => {
            result["cliError"] = json!(version_check_error(&error));
            result["cliErrorCode"] = json!(if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                "notFound"
            } else if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
            {
                "permission"
            } else if error.to_string() == "查询超时" {
                "timeout"
            } else {
                "failed"
            });
        }
    }
    result
}
impl Rpc {
    pub(super) fn start_with_pre_command(
        binary: &str,
        root: &std::path::Path,
        proxy: &ProxyConfig,
        managed: bool,
        pre_command: &str,
        shell: &str,
    ) -> Result<Self> {
        let resolved = resolve_binary(binary, pre_command);
        if !managed {
            let probe = inspect_cli(binary, root, proxy, pre_command, shell);
            ensure!(
                probe["cliAvailable"] == true,
                "{}",
                probe["cliError"].as_str().unwrap_or("Codex CLI 暂不可用")
            );
        }
        let mut command = process::cli_command_with_pre_command(
            &resolved,
            &[
                "-c",
                if managed {
                    "cli_auth_credentials_store=\"file\""
                } else {
                    "cli_auth_credentials_store=\"ephemeral\""
                },
                "app-server",
            ],
            pre_command,
            shell,
        );
        command.env("CODEX_HOME", root).current_dir(root);
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
        let registration = process::track_child(&mut child)?;
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
            _registration: registration,
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
        let start = Instant::now();
        loop {
            crate::operations::check()?;
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                anyhow::bail!("Codex 响应超时");
            }
            match self
                .messages
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(value) => return Ok(value),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => anyhow::bail!("Codex 连接已断开"),
            }
        }
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
            ensure!(super::files::identity(&next)?.key == ident.key, "账户已改变，已丢弃查询结果");
            if hash(next["tokens"]["access_token"].as_str().unwrap_or("")) == old { return Ok(None); }
            let e = super::files::external(&next)?;
            Ok(Some(json!({"accessToken":e["accessToken"],"chatgptAccountId":e["chatgptAccountId"],"chatgptPlanType":e["chatgptPlanType"]})))
        })
    }

    /// An isolated turn using access tokens only. The daily CLI remains the
    /// authority for refresh-token rotation while it is running.
    pub fn wake_turn(
        &mut self,
        auth: &Value,
        manifest: &crate::wakeups::Manifest,
        cwd: &std::path::Path,
        latest: &mut dyn FnMut() -> Result<Value>,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let identity = super::files::identity(auth)?;
        let mut revision = hash(auth["tokens"]["access_token"].as_str().unwrap_or(""));
        let mut refresh = |params: &Value| -> Result<Option<Value>> {
            ensure!(
                params["previousAccountId"]
                    .as_str()
                    .is_none_or(|id| id == identity.workspace),
                "刷新请求工作区不一致"
            );
            let next = latest()?;
            ensure!(
                super::files::identity(&next)?.key == identity.key,
                "账户已改变，已停止唤醒"
            );
            let next_revision = hash(next["tokens"]["access_token"].as_str().unwrap_or(""));
            if next_revision == revision {
                return Ok(None);
            }
            revision = next_revision;
            let e = super::files::external(&next)?;
            Ok(Some(
                json!({"accessToken":e["accessToken"],"chatgptAccountId":e["chatgptAccountId"],"chatgptPlanType":e["chatgptPlanType"]}),
            ))
        };
        let thread = self.call(
            "thread/start",
            json!({
                "model":manifest.model,"modelProvider":"openai","cwd":cwd,
                "approvalPolicy":"never","sandbox":"read-only","ephemeral":true,
                "baseInstructions":"Reply briefly. Do not use tools.",
                "config":{"features.shell_tool":false}
            }),
            &mut refresh,
        )?;
        let thread_id = thread["thread"]["id"]
            .as_str()
            .context("唤醒会话响应无效")?;
        let mut params = json!({"threadId":thread_id,"input":[{"type":"text","text":manifest.args.last().context("唤醒提示词缺失")?}]});
        if !manifest.effort.is_empty() {
            params["effort"] = json!(manifest.effort);
        }
        let turn = self.call("turn/start", params, &mut refresh)?;
        let turn_id = turn["turn"]["id"].as_str().context("唤醒轮次响应无效")?;
        loop {
            ensure!(Instant::now() < deadline, "Codex 唤醒超时");
            let event = self.receive(deadline.saturating_duration_since(Instant::now()))?;
            if let Some(id) = event.get("id") {
                let result = if event["method"] == "account/chatgptAuthTokens/refresh" {
                    refresh(&event["params"])?
                } else {
                    None
                };
                self.send(&match result {
                    Some(value) => json!({"id":id,"result":value}),
                    None => json!({"id":id,"error":{"code":-32000,"message":"Wakeup does not authorize tools or credential rotation"}}),
                })?;
            }
            if event["method"] == "turn/completed"
                && event["params"]["threadId"] == thread_id
                && event["params"]["turn"]["id"] == turn_id
            {
                let status = &event["params"]["turn"]["status"];
                ensure!(
                    *status == "completed",
                    "Codex 模型调用未完成；请检查认证、模型及网络"
                );
                return Ok(());
            }
        }
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        process::kill(&mut self.child);
    }
}

fn version_check_error(error: &anyhow::Error) -> &'static str {
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        match io.kind() {
            std::io::ErrorKind::NotFound => {
                return "未找到 Codex CLI，请检查“Codex 程序”路径、PATH 和前置命令";
            }
            std::io::ErrorKind::PermissionDenied => return "无法执行 Codex CLI，请检查程序执行权限",
            _ => {}
        }
    }
    if error.to_string() == "查询超时" {
        return "Codex CLI 版本检查超时（5 秒），请检查程序启动速度和前置命令";
    }
    // Do not include arbitrary launcher/pre-command stderr, which may contain secrets.
    "Codex CLI 版本检查失败，请检查程序路径、运行环境和前置命令"
}

fn external_version_supported(version: &str) -> Option<bool> {
    version.split_whitespace().find_map(|part| {
        let mut components = part.trim_start_matches('v').split('.');
        let major = components.next()?.parse::<u64>().ok()?;
        let minor = components.next()?.parse::<u64>().ok()?;
        Some(major > 0 || minor >= 99)
    })
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn version_failures_are_distinct_and_do_not_expose_launcher_output() {
        for (kind, expected) in [
            (std::io::ErrorKind::NotFound, "未找到"),
            (std::io::ErrorKind::PermissionDenied, "执行权限"),
        ] {
            let error =
                anyhow::Error::new(std::io::Error::new(kind, "PRIVATE")).context("启动命令失败");
            assert!(version_check_error(&error).contains(expected));
            assert!(!version_check_error(&error).contains("PRIVATE"));
        }
        assert!(version_check_error(&anyhow::anyhow!("查询超时")).contains("超时"));
        assert_eq!(
            version_check_error(&anyhow::anyhow!("PRIVATE")),
            "Codex CLI 版本检查失败，请检查程序路径、运行环境和前置命令"
        );
        assert_eq!(external_version_supported("codex-cli 0.98.0"), Some(false));
        assert_eq!(external_version_supported("codex-cli 0.99.0"), Some(true));
        assert_eq!(external_version_supported("codex-cli 1.0.0"), Some(true));
        assert_eq!(external_version_supported("wrong launcher"), None);
    }
}
