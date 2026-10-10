//! Reuse the CLI's own credential store. Authentication sessions never store tokens or switch profiles.
use crate::{Engine, models::*, network, process, quota, ssh, store};
use anyhow::{Context, Result, ensure};
use portable_pty::{CommandBuilder, PtySize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Session {
    source: String,
    configuration: String,
    phase: String,
    message: String,
    url: Option<String>,
    observation: Value,
    settings: Option<Settings>,
    auth_revision: Value,
    account_status: Value,
    persisted: bool,
    workspace_confirmation_required: bool,
    workspace_confirmed: bool,
    output_epoch: u64,
    writer: Option<Box<dyn Write + Send>>,
    killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}
type Sessions = HashMap<String, Arc<Mutex<Session>>>;
static SESSIONS: OnceLock<Mutex<Sessions>> = OnceLock::new();
fn sessions() -> &'static Mutex<Sessions> {
    SESSIONS.get_or_init(Default::default)
}
fn active(phase: &str) -> bool {
    matches!(phase, "checking" | "starting" | "authorizing" | "verifying")
}
pub fn busy() -> bool {
    sessions()
        .lock()
        .unwrap()
        .values()
        .any(|s| active(&s.lock().unwrap().phase))
}
fn snapshot(id: &str, s: &Session) -> Value {
    json!({"id":id,"sourceId":s.source,"phase":s.phase,"message":s.message,"authUrl":s.url,
        "authenticated":s.phase=="authenticated","identityConfirmed":s.observation["identityConfirmed"].as_bool().unwrap_or(false),
        "identity":s.observation["identity"],"current":s.observation["current"],"credential":s.observation["credential"],
        "metadataError":s.observation["metadataError"],"accountStatus":s.account_status,
        "workspaceConfirmationRequired":s.workspace_confirmation_required})
}
fn authenticated(session: &Arc<Mutex<Session>>, observation: Value) {
    let message = if observation["current"] == false {
        "登录有效，但设备当前身份与此账户不一致"
    } else if observation["identityConfirmed"] == true {
        "登录有效，已确认账户身份"
    } else {
        "登录有效；账户身份暂无法确认"
    };
    let mut s = session.lock().unwrap();
    if s.phase == "cancelled" {
        return;
    }
    s.observation = observation;
    finish_locked(&mut s, "authenticated", message);
}
fn finish(session: &Arc<Mutex<Session>>, phase: &str, message: &str) {
    let mut s = session.lock().unwrap();
    finish_locked(&mut s, phase, message);
}
fn finish_locked(s: &mut Session, phase: &str, message: &str) {
    if s.phase == "cancelled" {
        return;
    }
    s.phase = phase.into();
    s.message = message.into();
    if !active(phase) {
        if let Some(mut killer) = s.killer.take() {
            let _ = killer.kill();
        }
        s.writer = None;
        s.url = None;
        s.workspace_confirmation_required = false;
    }
}

fn saved_snapshot(
    store: &store::Store,
    source: &Source,
    id: &str,
    session: &Arc<Mutex<Session>>,
) -> Result<Value> {
    let mut s = session.lock().unwrap();
    if s.phase == "authenticated" {
        ensure!(
            crate::accounts::status_revision(store, source)? == s.auth_revision,
            "登录状态已变化，请重新检查"
        );
        if !s.persisted {
            let committed_revision =
                json!({"session":id,"confirmed":crate::codex_auth::files::new_id()});
            let row = crate::accounts::record_agy_status(
                store,
                s.settings.as_ref().context("登录配置不可用")?,
                source,
                s.observation.clone(),
                &s.auth_revision,
                Some(&committed_revision),
            )?;
            if !row.is_null() {
                s.observation = row.clone();
                s.auth_revision = committed_revision;
            }
            s.account_status = row;
            s.persisted = true;
        }
    }
    Ok(snapshot(id, &s))
}

/// Strip terminal escapes across read boundaries; keep only a bounded recognition window.
#[derive(Default)]
struct TerminalOutput {
    text: String,
    escape: u8,
}
impl TerminalOutput {
    fn push(&mut self, bytes: &[u8]) {
        let mut clean = Vec::new();
        for &b in bytes {
            match self.escape {
                1 => {
                    self.escape = match b {
                        b'[' => 2,
                        b']' => 3,
                        _ => 0,
                    }
                }
                2 => {
                    if (0x40..=0x7e).contains(&b) {
                        self.escape = 0;
                    }
                }
                3 => {
                    if b == 7 {
                        self.escape = 0;
                    } else if b == 27 {
                        self.escape = 4;
                    }
                }
                4 => self.escape = if b == b'\\' { 0 } else { 3 },
                _ if b == 27 => self.escape = 1,
                _ if b >= 32 || b == b'\n' || b == b'\r' || b == b'\t' => clean.push(b),
                _ => {}
            }
        }
        self.text.push_str(&String::from_utf8_lossy(&clean));
        if self.text.len() > 16384 {
            let mut start = self.text.len() - 8192;
            while !self.text.is_char_boundary(start) {
                start += 1;
            }
            self.text.drain(..start);
        }
    }
    fn observe(&self, s: &mut Session) {
        if !s.workspace_confirmed {
            let text = self.text.to_ascii_lowercase();
            if [
                "trust this folder",
                "trust this directory",
                "trust this workspace",
            ]
            .iter()
            .any(|p| text.contains(p))
            {
                s.workspace_confirmation_required = true;
                s.message = "CLI 正在询问是否信任临时空目录；点击“确认临时目录”继续授权".into();
            }
        }
        if !s.workspace_confirmation_required
            && let Some(url) = authorization_url(&self.text)
        {
            s.url = Some(url);
            s.message = "打开授权网址，完成浏览器登录；若返回授权码，请粘贴下方".into();
        }
    }
}

fn submit_input(s: &mut Session, code: &str, confirm_workspace: bool) -> Result<()> {
    ensure!(s.phase == "authorizing", "当前不在等待授权输入");
    if confirm_workspace {
        ensure!(s.workspace_confirmation_required, "CLI 未请求目录确认");
    } else {
        ensure!(!s.workspace_confirmation_required, "请先确认临时目录");
    }
    ensure!(
        code.len() <= 4096
            && !code.chars().any(char::is_control)
            && (confirm_workspace || !code.is_empty()),
        "授权码格式无效"
    );
    let writer = s.writer.as_mut().context("登录进程已结束，请重试")?;
    writer.write_all(code.as_bytes())?;
    writer.write_all(b"\r")?;
    writer.flush()?;
    if confirm_workspace {
        s.workspace_confirmation_required = false;
        s.workspace_confirmed = true;
    }
    s.output_epoch += 1;
    s.message = "已提交，请完成浏览器授权后验证登录".into();
    Ok(())
}
pub fn inspect(source: &Source, settings: &Settings) -> Result<Value> {
    let mut source = source.clone();
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == source.provider && a.id == source.account_id);
    source.quota_pre_command = crate::accounts::pre_command(settings, account, &source);
    let before = crate::agy_identity::read_key(&source, settings);
    let value = quota::agy(&source, settings)?;
    ensure!(
        value["command"]["name"] == "usage" && value["command"]["data"]["groups"].is_array(),
        "未取得有效的 Antigravity 登录状态"
    );
    let observation = crate::agy_identity::read(&source, settings);
    if let Some(identity) = &observation.identity {
        ensure!(
            before.as_ref().is_none_or(|key| key == &identity.key),
            "检查期间登录账户已变化，请重新检查"
        );
    }
    Ok(crate::accounts::agy_status(
        account,
        observation.identity.as_ref(),
        observation.metadata_error.as_deref(),
    ))
}
fn command(source: &Source, settings: &Settings) -> Result<CommandBuilder> {
    let binary = if source.agy_binary.is_empty() {
        "agy"
    } else {
        &source.agy_binary
    };
    let mut cmd = if let Some(id) = &source.host_id {
        let host = settings
            .hosts
            .iter()
            .find(|h| &h.id == id && h.enabled)
            .context("服务器已暂停")?;
        let mut host = host.clone();
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        host.pre_command = crate::accounts::pre_command(settings, account, source);
        let environment = quota::agy_proxy(source);
        ssh::account_command(
            &host,
            &format!(
                "{environment}export AGY_CLI_DISABLE_AUTO_UPDATE=true\naieyes_auth_dir=$(mktemp -d) || exit 1\ntrap 'rmdir \"$aieyes_auth_dir\" 2>/dev/null' EXIT\ncd \"$aieyes_auth_dir\" || exit 1\n{} --log-file /dev/null --prompt-interactive /usage",
                process::quote(binary)
            ),
        )?
    } else {
        let installed = store::home().join(".local/bin/agy");
        let path = if binary == "agy" && installed.is_file() {
            installed
        } else {
            store::expand(binary)
        };
        let pre = crate::accounts::pre_command(
            settings,
            settings
                .accounts
                .iter()
                .find(|a| a.provider == source.provider && a.id == source.account_id),
            source,
        );
        let mut cmd = if !pre.trim().is_empty() && !cfg!(windows) {
            let mut cmd = std::process::Command::new(ssh::DEFAULT_SHELL);
            cmd.args(["-c", &format!("set -e\n{}{{\n{}\n}} </dev/null >&2\nexec {} --log-file /dev/null --prompt-interactive /usage", ssh::PATH_FALLBACK, pre, process::quote(&path.to_string_lossy()))]);
            cmd
        } else {
            let mut cmd = process::cli_command(&path.to_string_lossy());
            cmd.args([
                "--log-file",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
                "--prompt-interactive",
                "/usage",
            ]);
            cmd
        };
        network::apply_env(&mut cmd, source.proxy.as_ref().unwrap_or(&settings.proxy));
        cmd
    };
    cmd.env("AGY_CLI_DISABLE_AUTO_UPDATE", "true");
    let mut builder = CommandBuilder::new(cmd.get_program());
    for arg in cmd.get_args() {
        builder.arg(if arg == "-T" {
            std::ffi::OsStr::new("-tt")
        } else {
            arg
        });
    }
    for (key, value) in cmd.get_envs() {
        if let Some(value) = value {
            builder.env(key, value)
        } else {
            builder.env_remove(key)
        }
    }
    builder.env("TERM", "xterm-256color");
    Ok(builder)
}
// Only Google's authorization URLs are exposed; terminal output and pasted codes are never returned.
fn authorization_url(text: &str) -> Option<String> {
    text.split("https://")
        .skip(1)
        .filter_map(|part| {
            let tail: String = part
                .chars()
                .take_while(|c| {
                    !c.is_whitespace() && !c.is_control() && !['<', '>', '"', '\''].contains(c)
                })
                .collect();
            let candidate = format!("https://{tail}");
            let url = reqwest::Url::parse(&candidate).ok()?;
            (matches!(
                url.host_str(),
                Some("accounts.google.com" | "antigravity.google" | "www.antigravity.google")
            ) && url.username().is_empty())
            .then_some(candidate)
        })
        .next()
}
fn launch(session: Arc<Mutex<Session>>, source: Source, settings: Settings) {
    if let Ok(observation) = inspect(&source, &settings) {
        authenticated(&session, observation);
        return;
    }
    if session.lock().unwrap().phase == "cancelled" {
        return;
    }
    let result = (|| -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "aieyes-agy-auth-{}",
            crate::codex_auth::files::new_id()
        ));
        std::fs::create_dir(&dir)?;
        let outcome = (|| -> Result<()> {
            let pair = portable_pty::native_pty_system().openpty(PtySize {
                rows: 40,
                cols: 200,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            let mut builder = command(&source, &settings)?;
            builder.cwd(&dir);
            let mut child = pair
                .slave
                .spawn_command(builder)
                .context("无法启动 agy，请检查二进制路径、SSH 与前置命令")?;
            drop(pair.slave);
            {
                let mut s = session.lock().unwrap();
                if s.phase == "cancelled" {
                    let _ = child.kill();
                    return Ok(());
                }
                s.writer = Some(pair.master.take_writer()?);
                s.killer = Some(child.clone_killer());
                s.phase = "authorizing".into();
                s.message =
                    "正在等待 CLI 的授权网址；若浏览器已打开，请完成授权后点击验证登录".into();
            }
            let mut reader = pair.master.try_clone_reader()?;
            let observed = session.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 4096];
                let mut pending = TerminalOutput::default();
                let mut epoch = 0;
                while let Ok(n) = reader.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    let mut s = observed.lock().unwrap();
                    if !active(&s.phase) {
                        break;
                    }
                    if epoch != s.output_epoch {
                        pending = TerminalOutput::default();
                        epoch = s.output_epoch;
                    }
                    pending.push(&buffer[..n]);
                    pending.observe(&mut s);
                }
            });
            let started = Instant::now();
            loop {
                if !active(&session.lock().unwrap().phase) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                if child.try_wait()?.is_some() {
                    finish(
                        &session,
                        "error",
                        "登录进程已退出；请检查 agy 安装、网络或前置命令，然后重试或验证现有登录",
                    );
                    break;
                }
                if started.elapsed() > Duration::from_secs(300) {
                    finish(&session, "error", "授权等待超过 5 分钟，请重新登录");
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(dir);
        outcome
    })();
    if let Err(error) = result {
        finish(&session, "error", &format!("{error:#}"));
    }
}
impl Engine {
    pub(crate) fn agy_auth_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let source = settings
            .sources
            .iter()
            .find(|s| params["sourceId"] == s.id && s.provider == "antigravity" && s.enabled)
            .context("请选择已启用的 Antigravity 目录")?;
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        let configuration = json!([
            source,
            source
                .host_id
                .as_ref()
                .and_then(|id| settings.hosts.iter().find(|h| &h.id == id)),
            settings.proxy,
            crate::accounts::pre_command(&settings, account, source)
        ])
        .to_string();
        if method == "agyAuth.inspect" {
            return inspect(source, &settings);
        }
        if method == "agyAuth.start" {
            let mut sessions = sessions().lock().unwrap();
            if let Some((id, s)) = sessions.iter().find(|(_, s)| {
                let s = s.lock().unwrap();
                s.source == source.id && s.configuration == configuration && active(&s.phase)
            }) {
                return saved_snapshot(&self.store, source, id, s);
            }
            sessions.retain(|_, s| {
                let s = s.lock().unwrap();
                s.source != source.id || s.configuration != configuration || active(&s.phase)
            });
            let id = crate::codex_auth::files::new_id();
            let revision = json!({"session":id});
            crate::accounts::codex_switched(&self.store, source, &revision)?;
            let session = Arc::new(Mutex::new(Session {
                source: source.id.clone(),
                configuration,
                phase: "checking".into(),
                message: "正在检查现有登录…".into(),
                settings: Some(settings.clone()),
                auth_revision: revision,
                ..Default::default()
            }));
            sessions.insert(id.clone(), session.clone());
            let result = snapshot(&id, &session.lock().unwrap());
            let source = source.clone();
            std::thread::spawn(move || launch(session, source, settings));
            return Ok(result);
        }
        let id = params["id"].as_str().context("缺少登录会话")?;
        let session = sessions()
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("登录会话已结束，请重试")?;
        {
            let s = session.lock().unwrap();
            ensure!(s.source == source.id, "登录会话不属于此目录");
            if s.configuration != configuration {
                drop(s);
                finish(&session, "error", "设备配置已变化，请重新登录");
                return Ok(snapshot(id, &session.lock().unwrap()));
            }
        }
        match method {
            "agyAuth.openUrl" => {
                let url = session
                    .lock()
                    .unwrap()
                    .url
                    .clone()
                    .context("尚未收到授权网址")?;
                ensure!(
                    authorization_url(&url).as_deref() == Some(url.as_str()),
                    "授权网址无效"
                );
                #[cfg(target_os = "macos")]
                let mut cmd = std::process::Command::new("open");
                #[cfg(target_os = "windows")]
                let mut cmd = {
                    let mut c = std::process::Command::new("rundll32.exe");
                    c.arg("url.dll,FileProtocolHandler");
                    c
                };
                #[cfg(all(unix, not(target_os = "macos")))]
                let mut cmd = std::process::Command::new("xdg-open");
                cmd.arg(&url);
                process::run(cmd, vec![], Duration::from_secs(10))
                    .context("无法打开授权网页，请复制网址")?;
            }
            "agyAuth.cancel" => finish(&session, "cancelled", "已取消此次授权，现有登录保留"),
            "agyAuth.verify" => {
                let mut state = session.lock().unwrap();
                ensure!(
                    !matches!(
                        state.phase.as_str(),
                        "checking" | "starting" | "verifying" | "cancelled"
                    ),
                    "正在检查登录或会话已取消"
                );
                let revision = json!({"session":id,"verifiedAt":now(),"attempt":crate::codex_auth::files::new_id()});
                crate::accounts::codex_switched(&self.store, source, &revision)?;
                state.auth_revision = revision;
                state.settings = Some(settings.clone());
                state.persisted = false;
                state.account_status = Value::Null;
                state.observation = Value::Null;
                finish_locked(&mut state, "verifying", "正在验证登录…");
                drop(state);
                let s = session.clone();
                let source = source.clone();
                let verification_settings = settings.clone();
                std::thread::spawn(move || match inspect(&source, &verification_settings) {
                    Ok(observation) => authenticated(&s, observation),
                    Err(_) => finish(
                        &s,
                        "authorizing",
                        "尚未验证到登录，请完成浏览器授权或检查网络后重试",
                    ),
                });
            }
            "agyAuth.submitCode" | "agyAuth.confirmWorkspace" => {
                let mut s = session.lock().unwrap();
                let code = if method == "agyAuth.confirmWorkspace" {
                    ""
                } else {
                    params["code"].as_str().context("请输入授权码")?.trim()
                };
                submit_input(&mut s, code, method == "agyAuth.confirmWorkspace")?;
            }
            "agyAuth.status" => {}
            _ => anyhow::bail!("未知 Antigravity 登录操作"),
        }
        saved_snapshot(&self.store, source, id, &session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_confirmation_write_keeps_the_prompt_and_cancel_clears_it() {
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut s = Session {
            phase: "authorizing".into(),
            workspace_confirmation_required: true,
            writer: Some(Box::new(FailedWriter)),
            ..Default::default()
        };
        assert!(submit_input(&mut s, "", true).is_err());
        assert!(s.workspace_confirmation_required);
        assert!(!s.workspace_confirmed);
        assert_eq!(s.output_epoch, 0);
        finish_locked(&mut s, "cancelled", "Cancelled");
        assert!(!s.workspace_confirmation_required);
        assert!(s.writer.is_none());
    }
    #[test]
    fn terminal_parser_ignores_non_workspace_trust_and_split_control_sequences() {
        let mut s = Session::default();
        let mut output = TerminalOutput::default();
        output.push(b"Do you trust this command?\x1b]");
        output.push(b"0;Yes, I trust this folder\x1b");
        output.push(b"\\\n");
        output.observe(&mut s);
        assert!(!s.workspace_confirmation_required);
        output.push(b"\x1b[3");
        output.push(b"2mYes, I trust this fol");
        output.push(b"der\x1b[0m");
        output.observe(&mut s);
        assert!(s.workspace_confirmation_required);
    }
    #[test]
    fn only_authorization_hosts_are_exposed() {
        assert!(authorization_url("https://evil.example/auth").is_none());
        assert!(authorization_url("https://accounts.google.com.evil.example/auth").is_none());
        assert_eq!(
            authorization_url("Open https://accounts.google.com/o/oauth2/auth?state=xyz\r\n"),
            Some("https://accounts.google.com/o/oauth2/auth?state=xyz".into())
        );
    }
}
