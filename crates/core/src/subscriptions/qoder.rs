//! Qoder's SDK control protocol, without an Agent prompt or copied login secret.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

pub(super) fn read(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
) -> Result<QuotaSnapshot> {
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == source.provider && a.id == source.account_id);
    let usage = query(
        source,
        settings,
        Some(config),
        &crate::accounts::pre_command(settings, account, source),
    )?;
    parse(source, config, &usage)
}

/// Inspect the login at the source's actual location, independently of quota or
/// its association with an Aieyes account. Never read identity from old logs.
pub(crate) fn inspect(
    source: &Source,
    settings: &Settings,
    pre_command: &str,
) -> Result<AccountIdentity> {
    let value = exchange_for(
        &source.provider,
        true,
        command(source, settings, source.subscription.as_ref(), pre_command)?,
        std::time::Instant::now() + std::time::Duration::from_secs(45),
    )
    .map_err(|e| anyhow::anyhow!(product_message(&source.provider, e.message)))?;
    identity_for(&source.provider, &value)
}

pub(crate) fn config_dir(source: &Source, config: Option<&SubscriptionConfig>) -> Result<String> {
    let path = if let Some(config) = config {
        ensure!(
            native_cli(&source.provider, config),
            "此来源不是 Qoder CLI 登录"
        );
        if config.native_config_dir.trim().is_empty() {
            crate::provider_paths::default_path(&source.provider, source.host_id.is_some())?
        } else {
            config.native_config_dir.clone()
        }
    } else {
        source.path.clone()
    };
    ensure!(
        !path.trim().is_empty() && !path.chars().any(char::is_control),
        "请选择已有的 Qoder 配置目录"
    );
    if source.host_id.is_some() {
        ensure!(
            path.starts_with('/') || path == "~" || path.starts_with("~/"),
            "远程 Qoder 配置目录应为绝对路径或 ~/ 下的目录"
        );
        Ok(path)
    } else {
        let root = crate::store::expand(&path);
        ensure!(root.is_absolute(), "Qoder 配置目录应为绝对路径");
        root.to_str()
            .map(str::to_owned)
            .context("Qoder 配置目录编码无效")
    }
}

#[cfg(test)]
fn remote_script(root: &str) -> String {
    remote_script_for("qoder", root)
}
fn protocol_args(_provider: &str) -> Vec<&'static str> {
    // Both products expose SDK v2. Print-only flags are rejected in SDK mode;
    // these control requests never create a model session.
    vec!["--sdk"]
}
fn frame_for(_provider: &str, kind: &str, id: &str) -> Vec<u8> {
    let kind = format!("daemon.{kind}");
    let mut request = json!({"type":kind,"subtype":kind});
    if kind == "daemon.initialize" {
        request["daemon_protocol_version"] = json!(2);
        request["sdk_version"] = json!("aieyes");
        request["capabilities"] = json!(["multi_session_v2"]);
        request["auth"] = json!({"type":"qodercli"});
    }
    serde_json::to_vec(&json!({"type":"control_request","request_id":id,"request":request}))
        .unwrap()
}
fn remote_script_for(provider: &str, root: &str) -> String {
    let binary = crate::qoder_family::spec(provider).binary;
    use crate::process::quote;
    // Expand only a leading ~/ on the remote machine; the rest remains literal.
    let root = if root == "~" {
        "\"$HOME\"".into()
    } else if let Some(relative) = root.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote(relative))
    } else {
        quote(root)
    };
    format!(
        "export AIEYES_INTERNAL=1\ncd \"$HOME\"\nif ! command -v {binary} >/dev/null 2>&1; then\n{}fi\ntest -d {root}\nexec {binary} {} --config-dir {root}",
        crate::ssh::PATH_FALLBACK,
        protocol_args(provider)
            .iter()
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn command(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
) -> Result<std::process::Command> {
    let root = config_dir(source, config)?;
    let command = if let Some(host_id) = &source.host_id {
        let mut host = settings
            .hosts
            .iter()
            .find(|h| &h.id == host_id)
            .context("Qoder 所在的 SSH 机器已移除")?
            .clone();
        host.pre_command = pre_command.into();
        crate::ssh::account_command(&host, &remote_script_for(&source.provider, &root))?
    } else {
        ensure!(
            std::path::Path::new(&root).is_dir(),
            "{}",
            product_message(
                &source.provider,
                "Qoder 配置目录不存在，请先运行 qodercli login 或选择已有配置目录"
            )
        );
        let mut args = protocol_args(&source.provider);
        args.extend(["--config-dir", root.as_str()]);
        // The GUI's PATH may omit ~/.local/bin. Keep the existing shell fallback.
        let mut command = crate::process::cli_command_with_pre_command(
            crate::qoder_family::spec(&source.provider).binary,
            &args,
            if pre_command.trim().is_empty() && cfg!(unix) {
                ":"
            } else {
                pre_command
            },
            "",
        );
        // An Aieyes-owned cwd keeps qodercli's per-directory logs of these runs out
        // of the user's projects; `record_internal` also records it as internal.
        let probe = probe_dir_for(&source.provider);
        command.current_dir(if std::fs::create_dir_all(&probe).is_ok() {
            probe
        } else {
            crate::store::home()
        });
        network::apply_env(
            &mut command,
            source.proxy.as_ref().unwrap_or(&settings.proxy),
        );
        command
    };
    let mut command = command;
    // The session monitor never counts runs Aieyes spawned itself.
    command.env("AIEYES_INTERNAL", "1");
    Ok(command)
}

/// Working directory of local quota checks, recorded as an internal session cwd.
#[cfg(test)]
fn probe_dir() -> std::path::PathBuf {
    probe_dir_for("qoder")
}
fn probe_dir_for(provider: &str) -> std::path::PathBuf {
    crate::store::home()
        .join(".aieyes")
        .join(format!("{provider}-probe"))
}

/// Control-session ids seen by this process, drained by `record_internal`.
/// Quota checks run in worker threads of several lane processes, so the engine
/// drains the list after every call instead of waiting for a session read.
static INTERNAL: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Marks outlive the one-day window in which transcripts count as live.
const INTERNAL_RETENTION: i64 = 2 * 86400;

/// `system/init`, `assistant` and `result` frames carry the run's session id.
fn session_id(frame: &Value) -> Option<&str> {
    frame["session_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
}

fn remember(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|known| known == id) {
        if ids.len() >= 64 {
            ids.remove(0);
        }
        ids.push(id.to_owned());
    }
}

/// Marks the control sessions seen since the last call, and the probe cwd, as
/// Aieyes' own so the session monitor never counts them. A failed write keeps
/// the ids for the next call.
pub(crate) fn record_internal(store: &crate::store::Store) -> Result<()> {
    let ids = INTERNAL
        .lock()
        .map(|mut ids| std::mem::take(&mut *ids))
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(());
    }
    let saved = persist_internal(store, &ids);
    if saved.is_err()
        && let Ok(mut pending) = INTERNAL.lock()
    {
        for id in &ids {
            remember(&mut pending, id);
        }
    }
    saved
}

fn persist_internal(store: &crate::store::Store, ids: &[String]) -> Result<()> {
    for marked in ids {
        let (provider, id) = marked.split_once('/').unwrap_or(("qoder", marked));
        if !crate::qoder_family::is_qoder(provider) {
            continue;
        }
        store.mark_internal_session(provider, &probe_dir_for(provider).to_string_lossy())?;
        store.mark_internal_session(provider, id)?;
    }
    store.db.execute("DELETE FROM kv WHERE (key LIKE 'internal-session:qoder:%' OR key LIKE 'internal-session:qoder-cn:%') AND CAST(value AS INTEGER)<?1", [now() - INTERNAL_RETENTION])?;
    Ok(())
}

#[cfg(test)]
fn cli_in(dirs: &[std::path::PathBuf]) -> bool {
    let names: &[&str] = if cfg!(windows) {
        &["qodercli.exe", "qodercli.cmd"]
    } else {
        &["qodercli"]
    };
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .any(|path| {
            std::fs::metadata(path).is_ok_and(|meta| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    meta.is_file() && meta.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    meta.is_file()
                }
            })
        })
}

impl crate::Engine {
    /// `qoderCli.status`: users with only the desktop, IDE or QoderWork app get
    /// one prompt to install the CLI that quota checks borrow. The UI re-detects
    /// the login with `accounts.autoScan` after the user ran the command.
    pub(crate) fn qoder_cli_status(&self, provider: &str) -> Result<Value> {
        ensure!(crate::qoder_family::is_qoder(provider), "不支持的 Agent");
        let spec = crate::qoder_family::spec(provider);
        let cli = crate::qoder_family::cli_path(provider).is_some();
        let settings = self.store.settings()?;
        let account = settings
            .accounts
            .iter()
            .any(|a| a.provider == provider && !a.archived)
            || settings.native_connections.iter().any(|r| {
                r.provider == provider && r.identity_key.is_some() && r.status != "remembered"
            });
        let apps = crate::qoder_family::installed_apps(provider);
        let command = if cli {
            format!("{} login", spec.binary)
        } else {
            format!(
                "curl -fsSL {}/install | bash && ~/.local/bin/{} login",
                spec.site, spec.binary
            )
        };
        let message = if account {
            format!("已识别 {} 登录", spec.name)
        } else {
            format!("请在终端完成 {} login 后重新检测", spec.binary)
        };
        Ok(
            json!({"provider":provider,"apps":apps,"cli":cli,"account":account,"offerInstall":!apps.is_empty()&&!account,"command":command,"message":message,"loginUrl":format!("{}/account/integrations",spec.site)}),
        )
    }
}

#[derive(Debug)]
struct QueryFailure {
    message: &'static str,
    retry: bool,
}

fn product_message(provider: &str, message: &str) -> String {
    if provider == "qoder-cn" {
        message
            .replace("qodercli", "qoderclicn")
            .replace("Qoder", "Qoder CN")
    } else {
        message.into()
    }
}

const SIGNED_OUT: &str = "Qoder CLI 未登录，请在终端运行 qodercli login";

fn await_response(
    provider: &str,
    process: &crate::process::LineProcess,
    id: &str,
    deadline: std::time::Instant,
    size: &mut usize,
) -> std::result::Result<Value, QueryFailure> {
    let initializing = id == "aieyes-init";
    loop {
        let line = process
            .read(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| QueryFailure {
                message: match (initializing, std::time::Instant::now() >= deadline) {
                    (true, true) => "Qoder CLI 初始化超时",
                    (false, true) => "Qoder CLI 额度查询超时",
                    (true, false) => "Qoder CLI 连接中断，未收到初始化响应",
                    (false, false) => "Qoder CLI 连接中断，未收到额度响应",
                },
                retry: true,
            })?;
        *size += line.len();
        if *size > 2 * 1024 * 1024 {
            return Err(QueryFailure {
                message: "Qoder 额度响应过大",
                retry: false,
            });
        }
        let Ok(frame) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if let Some(id) = session_id(&frame)
            && let Ok(mut ids) = INTERNAL.lock()
        {
            remember(&mut ids, &format!("{provider}/{id}"));
        }
        // Signed out, qodercli answers with an error turn and exits; retrying is futile.
        if frame["type"] == "assistant" && frame["error"] == "authentication_failed" {
            return Err(QueryFailure {
                message: SIGNED_OUT,
                retry: false,
            });
        }
        if frame["type"] != "control_response" || frame["response"]["request_id"] != id {
            continue;
        }
        let response = &frame["response"];
        if response["subtype"] != "success" {
            let code = response["error"]["code"].as_str().unwrap_or("");
            return Err(QueryFailure {
                message: if initializing {
                    "Qoder CLI 初始化失败，请检查登录"
                } else {
                    "Qoder CLI 未返回额度，请检查登录或客户端版本"
                },
                retry: matches!(
                    code,
                    "temporarily_unavailable" | "quota_unavailable" | "network_error" | "timeout"
                ),
            });
        }
        return Ok(response["response"].clone());
    }
}

fn exchange_for(
    provider: &str,
    identity_only: bool,
    command: std::process::Command,
    deadline: std::time::Instant,
) -> std::result::Result<Value, QueryFailure> {
    let process = crate::process::LineProcess::spawn(command).map_err(|_| QueryFailure {
        message: "Qoder CLI 启动失败，请检查安装和配置目录",
        retry: false,
    })?;
    let mut size = 0;
    process
        .send(frame_for(provider, "initialize", "aieyes-init"))
        .map_err(|_| QueryFailure {
            message: "Qoder CLI 初始化连接中断",
            retry: true,
        })?;
    let initialized = await_response(provider, &process, "aieyes-init", deadline, &mut size)?;
    if identity_only {
        if initialized["account"]["userId"].is_string() {
            return Ok(initialized["account"].clone());
        }
        process
            .send(frame_for(provider, "account_info", "aieyes-account"))
            .map_err(|_| QueryFailure {
                message: "账户查询连接中断",
                retry: true,
            })?;
        return Ok(
            await_response(provider, &process, "aieyes-account", deadline, &mut size)?["account"]
                .clone(),
        );
    }
    process
        .send(frame_for(provider, "get_usage_info", "aieyes-usage"))
        .map_err(|_| QueryFailure {
            message: "Qoder CLI 额度查询连接中断",
            retry: true,
        })?;
    let result = await_response(provider, &process, "aieyes-usage", deadline, &mut size)?;
    let usage = &result["usage"];
    if usage.is_null() {
        return Err(QueryFailure {
            message: "Qoder 账户额度暂不可用",
            retry: true,
        });
    }
    Ok(usage.clone())
}

fn query(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
) -> Result<Value> {
    query_with_budget(
        source,
        settings,
        config,
        pre_command,
        std::time::Duration::from_secs(45),
    )
}

fn query_with_budget(
    source: &Source,
    settings: &Settings,
    config: Option<&SubscriptionConfig>,
    pre_command: &str,
    budget: std::time::Duration,
) -> Result<Value> {
    query_attempts_for(
        &source.provider,
        || command(source, settings, config, pre_command),
        budget,
    )
}

fn query_attempts_for(
    provider: &str,
    mut command: impl FnMut() -> Result<std::process::Command>,
    budget: std::time::Duration,
) -> Result<Value> {
    let started = std::time::Instant::now();
    let deadline = started + budget;
    for attempt in 0..2 {
        crate::operations::check()?;
        let command = command()?;
        let until = if attempt == 0 {
            started + budget / 2
        } else {
            deadline
        };
        match exchange_for(provider, false, command, until) {
            Ok(value) => return Ok(value),
            Err(error) if !error.retry || attempt == 1 || std::time::Instant::now() >= deadline => {
                anyhow::bail!(product_message(provider, error.message))
            }
            Err(_) => {}
        }
    }
    unreachable!()
}

#[cfg(test)]
fn response(output: &[u8]) -> Result<Value> {
    ensure!(output.len() <= 2 * 1024 * 1024, "Qoder 额度响应过大");
    let mut usage = None;
    for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(frame) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if frame["type"] != "control_response" || frame["response"]["request_id"] != "aieyes-usage"
        {
            continue;
        }
        ensure!(
            usage.is_none() && frame["response"]["subtype"] == "success",
            "Qoder CLI 不支持额度查询或当前登录不可用"
        );
        usage = Some(frame["response"]["response"]["usage"].clone());
    }
    usage.context("Qoder CLI 未返回账户额度；请升级客户端并检查登录")
}

pub(super) fn check_identity(
    snapshot: &QuotaSnapshot,
    account: Option<&Account>,
    require_bound: bool,
) -> Result<()> {
    let key = &snapshot
        .identity
        .as_ref()
        .context("Qoder 未返回账户身份")?
        .key;
    let expected = account.and_then(|a| a.identity_key.as_ref());
    ensure!(
        !require_bound || expected.is_some(),
        "Qoder 账户身份尚未绑定，请重新连接 CLI 登录"
    );
    ensure!(
        expected.is_none_or(|expected| expected == key),
        "Qoder CLI 当前登录与此账户不一致；请在 CLI 切换回原账户或另建账户"
    );
    Ok(())
}

#[cfg(test)]
fn identity(v: &Value) -> Result<AccountIdentity> {
    identity_for("qoder", v)
}
fn identity_for(provider: &str, v: &Value) -> Result<AccountIdentity> {
    let user = v["userId"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .context("Qoder 未返回账户身份；会话 Credits 不能代替账户额度")?;
    Ok(AccountIdentity {
        key: format!("{provider}:{:x}", Sha256::digest(user.as_bytes())),
        email: v["email"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(256)
            .collect(),
        subscription: label(if v["userType"].is_null() {
            &v["subscriptionType"]
        } else {
            &v["userType"]
        }),
        checked_at: now(),
        subscription_checked_at: Some(now()),
        ..Default::default()
    })
}

#[cfg(test)]
fn apply(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    apply_for("qoder", q, v)
}
pub(super) fn apply_for(provider: &str, q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let identity = identity_for(provider, v)?;
    q.plan = identity.subscription.clone();
    let mut any_remaining = false;
    for (field, total_field, name) in [
        ("userQuota", "total", "套餐"),
        ("addOnQuota", "total", "加购"),
        ("orgResourcePackage", "cap", "组织共享"),
    ] {
        let bucket = &v[field];
        if bucket.is_null() {
            continue;
        }
        ensure!(
            bucket["unit"]
                .as_str()
                .is_some_and(|u| u.eq_ignore_ascii_case("credits")),
            "Qoder 额度单位不支持"
        );
        let (total, used, remaining) = (
            number(&bucket[total_field])?,
            number(&bucket["used"])?,
            number(&bucket["remaining"])?,
        );
        ensure!(
            total >= 0. && used >= 0. && remaining >= 0.,
            "Qoder Credits 数值无效"
        );
        let usage = if total == 0. {
            ensure!(used == 0. && remaining == 0., "Qoder 零上限额度不一致");
            100.
        } else {
            ratio(&bucket["used"], &bucket[total_field])?
        };
        any_remaining |= remaining > 0. && bucket["available"] != false;
        window(
            q,
            field,
            &format!("{name} Credits · {used} / {total} · 剩余 {remaining}"),
            usage,
            None,
            None,
        );
    }
    ensure!(
        !q.windows.is_empty(),
        "Qoder 未返回账户额度；会话 Credits 不能代替账户额度"
    );
    q.is_available = Some(v["isQuotaExceeded"] != true && any_remaining);
    q.identity = Some(identity);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_and_local_runs_are_marked_internal() {
        assert!(remote_script("~/.qoder").starts_with("export AIEYES_INTERNAL=1\n"));
        let root = tempfile::tempdir().unwrap();
        let source = Source {
            provider: "qoder".into(),
            path: root.path().to_string_lossy().into(),
            ..Default::default()
        };
        let local = command(&source, &Settings::default(), None, "").unwrap();
        assert!(
            local
                .get_envs()
                .any(|(k, v)| k == "AIEYES_INTERNAL" && v == Some(std::ffi::OsStr::new("1")))
        );
        assert_eq!(local.get_current_dir(), Some(probe_dir().as_path()));
    }

    #[test]
    fn control_session_ids_are_bounded_and_marked_with_the_probe_cwd() {
        let init = json!({"type":"system","subtype":"init","session_id":"qoder-ctl-1"});
        assert_eq!(session_id(&init), Some("qoder-ctl-1"));
        for frame in [
            json!({"type":"system","session_id":"x".repeat(200)}),
            json!({"type":"result","session_id":"bad\nid"}),
            json!({"type":"control_response","response":{"request_id":"aieyes-init"}}),
        ] {
            assert_eq!(session_id(&frame), None);
        }
        let mut ids = vec![];
        for index in 0..70 {
            remember(&mut ids, &format!("run-{index}"));
        }
        remember(&mut ids, "run-69");
        assert_eq!(ids.len(), 64);
        assert_eq!(ids[0], "run-6");
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(root.path()).unwrap();
        store.mark_internal_session("qoder", "expired").unwrap();
        store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key='internal-session:qoder:expired'",
                [(now() - INTERNAL_RETENTION - 1).to_string()],
            )
            .unwrap();
        store.mark_internal_session("kiro", "unrelated").unwrap();
        persist_internal(&store, &ids[..2]).unwrap();
        let keys = store.internal_sessions().unwrap();
        for key in ["run-6", "run-7", &probe_dir().to_string_lossy()] {
            assert!(
                keys.contains(&format!("internal-session:qoder:{key}")),
                "{key}"
            );
        }
        assert!(!keys.contains("internal-session:qoder:expired"));
        assert!(keys.contains("internal-session:kiro:unrelated"));
    }

    #[cfg(unix)]
    #[test]
    fn signed_out_cli_fails_once_with_a_login_hint() {
        // Recorded from qodercli 1.1.79 with an empty config directory.
        let mut attempts = 0;
        let error = query_attempts(
            || {
                attempts += 1;
                let mut command = std::process::Command::new("python3");
                command.args(["-u", "-c", r#"
import json,sys
print(json.dumps(dict(type='system',subtype='init',session_id='signed-out',apiKeySource='none')))
sys.stdin.readline()
print(json.dumps(dict(type='assistant',session_id='signed-out',error='authentication_failed',message=dict(content=[dict(type='text',text='PRIVATE')]))))
print(json.dumps(dict(type='result',subtype='success',is_error=True,result='Not logged in',session_id='signed-out')))
sys.exit(1)
"#]);
                Ok(command)
            },
            std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(error.to_string(), SIGNED_OUT);
    }

    #[test]
    fn install_prompt_needs_a_desktop_app_without_a_known_login() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let name = if cfg!(windows) {
            "qodercli.cmd"
        } else {
            "qodercli"
        };
        std::fs::write(bin.join(name), "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(!cli_in(std::slice::from_ref(&bin)), "not executable");
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert!(cli_in(std::slice::from_ref(&bin)));
        assert!(!cli_in(&[root.path().to_owned()]));
        let engine = Engine::open(&root.path().join("data")).unwrap();
        let before = engine.qoder_cli_status("qoder").unwrap();
        assert_eq!(before["account"], false);
        assert_eq!(
            before["offerInstall"],
            !before["apps"].as_array().unwrap().is_empty()
        );
        assert!(
            before["command"]
                .as_str()
                .unwrap()
                .ends_with("qodercli login")
        );
        assert!(before["message"].as_str().unwrap().contains("qodercli"));
        let mut settings = engine.store.settings().unwrap();
        settings.accounts.push(Account {
            id: "a".into(),
            provider: "qoder".into(),
            name: "Qoder".into(),
            ..Default::default()
        });
        engine.store.save_settings(&settings).unwrap();
        let after = engine.qoder_cli_status("qoder").unwrap();
        assert_eq!(after["account"], true);
        assert_eq!(after["offerInstall"], false);
        assert_eq!(after["message"], "已识别 Qoder 登录");
        settings.accounts[0].archived = true;
        engine.store.save_settings(&settings).unwrap();
        assert_eq!(engine.qoder_cli_status("qoder").unwrap()["account"], false);
    }

    #[cfg(unix)]
    fn scripted_cli(tail: &str) -> std::process::Command {
        let mut command = std::process::Command::new("python3");
        command.args(["-u", "-c", &format!(r#"
import json,sys,time,select
init=json.loads(sys.stdin.readline())
assert init['request']['type']=='daemon.initialize'
# A request before the initialization response is a protocol violation.
assert not select.select([sys.stdin],[],[],.04)[0]
print(json.dumps(dict(type='control_response',response=dict(request_id='aieyes-init',subtype='success',response={{}}))))
request=json.loads(sys.stdin.readline())
assert request['request']['type']=='daemon.get_usage_info'
{tail}
"#)]);
        command
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_initialization_and_keeps_input_open_until_matched_usage() {
        let usage = json!({"userId":"fixture","userQuota":{"unit":"credits","total":0,"used":0,"remaining":0}});
        let tail = format!(
            "print('{}')\nprint('{}')\nassert not select.select([sys.stdin],[],[],.03)[0]\ntime.sleep(30)",
            json!({"type":"control_response","response":{"request_id":"other","subtype":"success","response":{"usage":{"wrong":true}}}}),
            json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":usage}}})
        );
        let started = std::time::Instant::now();
        let result = exchange(
            scripted_cli(&tail),
            started + std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(result, usage);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let mut snapshot = QuotaSnapshot::default();
        apply(&mut snapshot, &result).unwrap();
        assert_eq!(snapshot.is_available, Some(false));
        assert_eq!(snapshot.windows.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn retries_only_once_for_eof_or_missing_usage_and_never_for_login_errors() {
        for (tail, expected) in [
            ("sys.exit(0)", 2),
            (
                "print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"session\":{\"credits\":999}}}}')",
                2,
            ),
            (
                "print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":{\"code\":\"unauthorized\",\"message\":\"PRIVATE\"}}}')",
                1,
            ),
        ] {
            let mut attempts = 0;
            let error = query_attempts(
                || {
                    attempts += 1;
                    Ok(scripted_cli(tail))
                },
                std::time::Duration::from_secs(2),
            )
            .unwrap_err();
            assert_eq!(attempts, expected);
            assert!(!error.to_string().contains("PRIVATE"));
        }
        let mut attempts = 0;
        let result=query_attempts(|| {
            attempts+=1;
            Ok(scripted_cli(if attempts==1 {"sys.exit(0)"} else {"print('{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"usage\":{\"userId\":\"fixture\"}}}}')"}))
        },std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(result["userId"], "fixture");
    }

    #[cfg(unix)]
    #[test]
    fn total_deadline_includes_both_attempts_and_hung_initialization() {
        let mut attempts = 0;
        let started = std::time::Instant::now();
        let error = query_attempts(
            || {
                attempts += 1;
                let mut command = std::process::Command::new("python3");
                command.args(["-c", "import time;time.sleep(30)"]);
                Ok(command)
            },
            std::time::Duration::from_millis(400),
        )
        .unwrap_err();
        assert_eq!(attempts, 2);
        assert!(error.to_string().contains("初始化超时"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn login_identity_is_independent_of_quota_and_never_uses_tokens_or_logs() {
        let usage =
            json!({"userId":"account-a","userType":"personal_standard", "token":"PRIVATE_TOKEN"});
        let login = identity(&usage).unwrap();
        assert_eq!(
            login.key,
            format!("qoder:{:x}", Sha256::digest(b"account-a"))
        );
        assert_eq!(identity(&usage).unwrap().key, login.key);
        assert_ne!(
            identity(&json!({"userId":"account-b"})).unwrap().key,
            login.key
        );
        assert!(!json!(login).to_string().contains("PRIVATE_TOKEN"));
        assert!(apply(&mut QuotaSnapshot::default(), &usage).is_err());
        for value in [
            json!({"token":"PRIVATE_TOKEN"}),
            json!({"userId":" "}),
            json!({"userId":"bad\nidentity"}),
        ] {
            assert!(identity(&value).is_err());
        }
    }

    #[test]
    fn config_roots_distinguish_remote_home_and_never_default_empty_history_paths() {
        let mut source = Source {
            provider: "qoder".into(),
            path: "~/.qoder profile".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        assert_eq!(config_dir(&source, None).unwrap(), "~/.qoder profile");
        source.host_id = None;
        assert_eq!(
            config_dir(&source, None).unwrap(),
            crate::store::home()
                .join(".qoder profile")
                .to_str()
                .unwrap()
        );
        source.path.clear();
        assert!(config_dir(&source, None).is_err());
        let config = SubscriptionConfig {
            mode: "cli".into(),
            native_config_dir: crate::store::home()
                .join(".qoder custom")
                .to_string_lossy()
                .into(),
            ..Default::default()
        };
        assert_eq!(
            config_dir(&source, Some(&config)).unwrap(),
            config.native_config_dir
        );
        source.host_id = Some("remote".into());
        source.path = "relative/directory".into();
        assert!(config_dir(&source, None).is_err());
    }

    #[cfg(unix)]
    fn fixture_cli(root: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("qodercli");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
set -eu
for arg do
    if [ "${next:-}" = config ]; then config=$arg; next=; fi
    if [ "$arg" = --config-dir ]; then next=config; fi
done
IFS= read -r init
printf '%s\n' "$init" > "$config/requests.jsonl"
printf '%s\n' '{"type":"control_response","response":{"request_id":"aieyes-init","subtype":"success","response":{}}}'
IFS= read -r usage
printf '%s\n' "$usage" >> "$config/requests.jsonl"
printf '%s\n' "$@" > "$config/args.txt"
printf 'query\n' >> "$config/queries.txt"
case "$usage" in
    *aieyes-account*) sed 's/aieyes-usage/aieyes-account/g;s/"usage":/"account":/g' "$config/response.jsonl" ;;
    *) cat "$config/response.jsonl" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        format!(
            "export PATH={}:\"$PATH\"",
            crate::process::quote(bin.to_str().unwrap())
        )
    }

    #[cfg(unix)]
    fn fixture_response(root: &std::path::Path, user: &str) {
        std::fs::create_dir_all(root).unwrap();
        let frame = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":{"userId":user,"userType":"personal_standard"}}}});
        std::fs::write(root.join("response.jsonl"), frame.to_string() + "\n").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_refresh_retains_last_successful_quota_and_timestamp() {
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let profile = root.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let usage = json!({"userId":"fixture","userType":"personal_standard",
            "userQuota":{"unit":"credits","total":100,"used":25,"remaining":75}});
        let success = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"success","response":{"usage":usage}}});
        std::fs::write(profile.join("response.jsonl"), format!("{success}\n")).unwrap();
        let mut engine = Engine::open(&root.path().join("data")).unwrap();
        engine
            .store
            .save_auth_settings(&Settings {
                accounts: vec![Account {
                    id: "a".into(),
                    provider: "qoder".into(),
                    name: "Quota fixture".into(),
                    quota_enabled: true,
                    identity_key: Some(identity(&usage).unwrap().key),
                    ..Default::default()
                }],
                sources: vec![Source {
                    id: "quota".into(),
                    provider: "qoder".into(),
                    account_id: "a".into(),
                    subscription: Some(SubscriptionConfig {
                        mode: "cli".into(),
                        native_config_dir: profile.to_string_lossy().into(),
                        ..Default::default()
                    }),
                    quota_pre_command: pre_command,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        let params = json!({"accountKey":"qoder:a","sourceId":"quota"});
        engine.call("quotas.refresh", params.clone()).unwrap();
        let before = engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .quotas
            .remove(0);
        assert!(before.error.is_none());
        assert!(before.updated_at > 0);
        assert_eq!(before.windows.len(), 1);
        let failure = json!({"type":"control_response","response":{"request_id":"aieyes-usage","subtype":"error","error":{"code":"unauthorized","message":"PRIVATE_CREDENTIAL"}}});
        std::fs::write(profile.join("response.jsonl"), format!("{failure}\n")).unwrap();
        engine.call("quotas.refresh", params).unwrap();
        let after = engine
            .store
            .dashboard(&Filter::default())
            .unwrap()
            .quotas
            .remove(0);
        assert!(after.error.is_some());
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(
            serde_json::to_value(&after.windows).unwrap(),
            serde_json::to_value(&before.windows).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&after.balances).unwrap(),
            serde_json::to_value(&before.balances).unwrap()
        );
        assert!(
            !serde_json::to_string(&after)
                .unwrap()
                .contains("PRIVATE_CREDENTIAL")
        );
        assert_eq!(
            std::fs::read_to_string(profile.join("queries.txt"))
                .unwrap()
                .lines()
                .count(),
            2,
            "login failures must not retry"
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_and_remote_shell_queries_preserve_literal_paths_and_control_only_frames() {
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let config_root = root.path().join("qoder ' $dollar `literal` $(literal)");
        fixture_response(&config_root, "account-a");
        let source = Source {
            provider: "qoder".into(),
            path: config_root.to_string_lossy().into(),
            ..Default::default()
        };
        let local = inspect(&source, &Settings::default(), &pre_command).unwrap();
        let frames = std::fs::read(config_root.join("requests.jsonl")).unwrap();
        let requests: Vec<Value> = String::from_utf8(frames.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["request"]["type"], "daemon.initialize");
        assert_eq!(requests[1]["request"]["type"], "daemon.account_info");
        let args = std::fs::read_to_string(config_root.join("args.txt")).unwrap();
        assert!(args.starts_with("--sdk\n"));
        assert!(!args.contains("--print"));
        assert_eq!(requests[0]["request"]["auth"]["type"], "qodercli");
        assert!(args.ends_with(&format!("--config-dir\n{}\n", config_root.display())));
        for shell in ["/bin/sh", "/bin/bash"] {
            let mut command = std::process::Command::new(shell);
            command.args([
                "-c",
                &format!("set -e\n{pre_command}\n{}", remote_script(&source.path)),
            ]);
            let output = crate::process::run(
                command,
                String::from_utf8(frames.clone())
                    .unwrap()
                    .replace("aieyes-account", "aieyes-usage")
                    .replace("account_info", "get_usage_info")
                    .into_bytes(),
                std::time::Duration::from_secs(5),
            )
            .unwrap();
            assert_eq!(
                identity(&response(&output).unwrap()).unwrap().key,
                local.key
            );
            assert_eq!(
                std::fs::read_to_string(config_root.join("args.txt")).unwrap(),
                args
            );
        }
        let script = remote_script("~/qoder ' $(literal)");
        assert!(script.contains("\"$HOME\"/'qoder '"));
        assert!(!script.contains(crate::store::home().to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn device_batch_shares_queries_compares_all_accounts_and_retains_failed_observations() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root = tempfile::tempdir().unwrap();
        let pre_command = fixture_cli(root.path());
        let profile_a = root.path().join("profile-a");
        let profile_b = root.path().join("profile-b");
        fixture_response(&profile_a, "account-a");
        fixture_response(&profile_b, "account-b");
        let key_a = identity(&json!({"userId":"account-a"})).unwrap().key;
        let key_b = identity(&json!({"userId":"account-b"})).unwrap().key;
        let mut engine = Engine::open(&root.path().join("data")).unwrap();
        let settings = Settings {
            accounts: vec![
                Account {
                    id: "a".into(),
                    name: "Account A".into(),
                    provider: "qoder".into(),
                    identity_key: Some(key_a.clone()),
                    ..Default::default()
                },
                Account {
                    id: "b".into(),
                    name: "Account B".into(),
                    provider: "qoder".into(),
                    identity_key: Some(key_b.clone()),
                    ..Default::default()
                },
                Account {
                    id: "unbound".into(),
                    name: "Unbound account".into(),
                    provider: "qoder".into(),
                    ..Default::default()
                },
            ],
            sources: vec![
                Source {
                    id: "local".into(),
                    name: "Local".into(),
                    provider: "qoder".into(),
                    path: profile_a.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "same-profile".into(),
                    name: "Same profile".into(),
                    provider: "qoder".into(),
                    path: profile_a.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "other-profile".into(),
                    name: "Other profile".into(),
                    provider: "qoder".into(),
                    path: profile_b.to_string_lossy().into(),
                    quota_pre_command: pre_command.clone(),
                    ..Default::default()
                },
                Source {
                    id: "quota-profile".into(),
                    name: "Native quota connection".into(),
                    provider: "qoder".into(),
                    account_id: "a".into(),
                    subscription: Some(SubscriptionConfig {
                        mode: "cli".into(),
                        native_config_dir: profile_a.to_string_lossy().into(),
                        ..Default::default()
                    }),
                    quota_pre_command: pre_command,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        engine.store.save_auth_settings(&settings).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        engine.status_observation_reads = Some(reads.clone());
        let rows = engine
            .refresh_account_status_batch(
                &json!({"sourceIds":["local","same-profile","other-profile","quota-profile"]}),
            )
            .unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert_eq!(rows.as_array().unwrap().len(), 12);
        for row in rows.as_array().unwrap() {
            let expected = if row["sourceId"] == "other-profile" {
                &key_b
            } else {
                &key_a
            };
            assert_eq!(row["identity"]["key"], *expected);
            assert_eq!(row["authenticated"], true);
            assert_eq!(row["identityConfirmed"], true);
            let current = if row["accountKey"] == "qoder:unbound" {
                Value::Null
            } else {
                json!(
                    row["accountKey"]
                        == if expected == &key_a {
                            "qoder:a"
                        } else {
                            "qoder:b"
                        }
                )
            };
            assert_eq!(row["current"], current);
            assert_eq!(row["credential"], current);
        }
        assert_eq!(
            engine
                .refresh_account_status_batch(&json!({"dueOnly":true}))
                .unwrap(),
            json!([])
        );
        // Background status checks also start qodercli: 5 minutes is not yet due.
        let age = |engine: &Engine, seconds: i64| {
            engine.store.db.execute(
                "UPDATE kv SET value=json_set(value,'$.attemptedAt',?1) WHERE key LIKE 'account-status:qoder:%'",
                [now() - seconds],
            )
            .unwrap()
        };
        age(&engine, 400);
        assert_eq!(
            engine
                .refresh_account_status_batch(&json!({"dueOnly":true}))
                .unwrap(),
            json!([])
        );
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        age(&engine, 700);
        let due = engine
            .refresh_account_status_batch(&json!({"dueOnly":true}))
            .unwrap();
        assert_eq!(due.as_array().unwrap().len(), 12);
        assert_eq!(reads.load(Ordering::Relaxed), 4);
        // Switching a native login updates observations, never the bound account.
        fixture_response(&profile_a, "account-b");
        let switched = engine
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"qoder:a","sourceId":"local"}),
            )
            .unwrap();
        assert_eq!(switched["current"], false);
        assert_eq!(switched["identity"]["key"], key_b);
        std::fs::write(profile_a.join("response.jsonl"), "{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":\"PRIVATE_TOKEN\"}}\n").unwrap();
        let failed = engine
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"qoder:a","sourceId":"local"}),
            )
            .unwrap();
        assert_eq!(failed["identity"]["key"], key_b);
        assert_eq!(failed["current"], false);
        assert!(failed["error"].is_string());
        assert!(!failed.to_string().contains("PRIVATE_TOKEN"));
        assert_eq!(
            engine.store.settings().unwrap().accounts[0]
                .identity_key
                .as_ref(),
            Some(&key_a)
        );
    }

    #[test]
    fn control_response_cannot_substitute_session_or_other_request_for_account_quota() {
        let data = b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-init\",\"subtype\":\"success\",\"response\":{\"usage\":{\"wrong\":true}}}}\n{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"session\":{\"total_credits\":0}}}}\n";
        let usage = response(data).unwrap();
        assert!(apply(&mut QuotaSnapshot::default(), &usage).is_err());
        assert!(response(b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":\"PRIVATE_TOKEN\"}}\n").unwrap_err().to_string().find("PRIVATE_TOKEN").is_none());
    }
}

#[cfg(test)]
fn query_attempts(
    command: impl FnMut() -> Result<std::process::Command>,
    budget: std::time::Duration,
) -> Result<Value> {
    query_attempts_for("qoder", command, budget)
}
#[cfg(test)]
fn exchange(
    command: std::process::Command,
    deadline: std::time::Instant,
) -> std::result::Result<Value, QueryFailure> {
    exchange_for("qoder", false, command, deadline)
}
#[cfg(test)]
mod region_tests {
    use super::*;
    #[test]
    fn same_server_id_never_merges_regions_and_cn_uses_its_cli() {
        let value = json!({"userId":"same-id","userType":"personal_standard","userQuota":{"unit":"credits","total":100,"used":20,"remaining":80}});
        let mut global = QuotaSnapshot::default();
        let mut cn = QuotaSnapshot::default();
        apply_for("qoder", &mut global, &value).unwrap();
        apply_for("qoder-cn", &mut cn, &value).unwrap();
        assert_ne!(global.identity.unwrap().key, cn.identity.unwrap().key);
        let cmd = remote_script_for("qoder-cn", "~/profile with spaces");
        assert!(cmd.contains("exec qoderclicn "));
        assert!(!cmd.contains("exec qodercli "));
        let engine = Engine::open(tempfile::tempdir().unwrap().path()).unwrap();
        assert!(engine.qoder_cli_status("unknown").is_err());
        assert!(
            engine.qoder_cli_status("qoder-cn").unwrap()["command"]
                .as_str()
                .unwrap()
                .contains("qoderclicn")
        );
    }
}
/// SDK v2 uses the client's credential strategy; no native login secret crosses this API.
pub(crate) fn native_byok(
    provider: &str,
    root: &std::path::Path,
    request: Value,
    proxy: &ProxyConfig,
) -> Result<Value> {
    let source = Source {
        provider: provider.into(),
        path: root.to_string_lossy().into_owned(),
        proxy: Some(proxy.clone()),
        ..Default::default()
    };
    let process =
        crate::process::LineProcess::spawn(command(&source, &Settings::default(), None, "")?)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let mut size = 0;
    process.send(frame_for(provider, "initialize", "aieyes-init"))?;
    let initialized = await_response(provider, &process, "aieyes-init", deadline, &mut size)
        .map_err(|e| anyhow::anyhow!(product_message(provider, e.message)))?;
    ensure!(
        initialized["daemon_protocol_version"] == 2,
        "当前 CLI 未确认 SDK 管理能力"
    );
    let mut request = request;
    let kind = format!(
        "daemon.{}",
        request["type"].as_str().context("API 操作类型缺失")?
    );
    request["type"] = json!(kind);
    request["subtype"] = json!(kind);
    process.send(serde_json::to_vec(
        &json!({"type":"control_request","request_id":"aieyes-byok","request":request}),
    )?)?;
    await_response(provider, &process, "aieyes-byok", deadline, &mut size)
        .map_err(|e| anyhow::anyhow!(product_message(provider, e.message)))
}
