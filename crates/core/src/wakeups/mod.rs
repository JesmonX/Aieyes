//! Opt-in deployments. Saved drafts and installed configurations are deliberately separate.
pub mod runner;
mod scheduler;
use crate::{
    Engine,
    models::*,
    network, pricing, process, quota, ssh,
    store::{self, Store},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

static RUNNER_PATH: OnceLock<PathBuf> = OnceLock::new();
pub fn set_runner_path(path: PathBuf) {
    let _ = RUNNER_PATH.set(path);
}
pub(crate) fn runtime_binary() -> Result<PathBuf> {
    Ok(RUNNER_PATH
        .get()
        .cloned()
        .unwrap_or(std::env::current_exe()?))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub codex_profile_id: Option<String>,
    pub id: String,
    pub name: String,
    pub source_id: String,
    pub times: Vec<String>,
    pub model: String,
    pub effort: String,
    pub prompt: String,
    pub binary: String,
}
impl Default for Task {
    fn default() -> Self {
        Self {
            codex_profile_id: None,
            id: String::new(),
            name: String::new(),
            source_id: String::new(),
            times: vec!["08:00".into()],
            model: String::new(),
            effort: "low".into(),
            prompt: "Hi. Reply only OK. Do not use any tools.".into(),
            binary: String::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub codex_profile_id: Option<String>,
    pub version: u32,
    pub id: String,
    pub account_key: String,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub binary: String,
    pub args: Vec<String>,
    pub config_path: String,
    pub pre_command: String,
    pub shell: String,
    pub proxy: ProxyConfig,
    pub times: Vec<String>,
    pub enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub task: Task,
    pub source: Source,
    pub host: Option<Host>,
    pub root: String,
    pub deployed_at: i64,
    pub enabled: bool,
    pub state: String,
    pub timezone: String,
}
impl Deployment {
    pub(crate) fn account_environment_changed(&self, settings: &Settings) -> bool {
        let Some(source) = settings
            .sources
            .iter()
            .find(|source| source.id == self.source.id)
        else {
            return true;
        };
        let account = settings.accounts.iter().find(|account| {
            account.provider == self.source.provider && account.id == self.source.account_id
        });
        let previous_command = crate::accounts::pre_command(
            &Settings {
                hosts: self.host.clone().into_iter().collect(),
                ..Default::default()
            },
            None,
            &self.source,
        );
        crate::accounts::pre_command(settings, account, source) != previous_command
            || (self.host.is_some()
                && self
                    .source
                    .proxy
                    .as_ref()
                    .is_some_and(|proxy| proxy.mode != "system"))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub task: Task,
    pub deployment: Option<Deployment>,
}
impl Store {
    pub fn wakeups(&self) -> Result<Vec<Record>> {
        let mut stmt = self
            .db
            .prepare("SELECT value FROM kv WHERE key LIKE 'wakeup:%' ORDER BY key")?;
        let raws = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raws.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub(crate) fn wakeup(&self, id: &str) -> Result<Record> {
        self.wakeups()?
            .into_iter()
            .find(|r| r.task.id == id)
            .context("任务不存在")
    }
    pub(crate) fn save_wakeup(&self, record: &Record) -> Result<()> {
        self.db.execute(
            "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![
                format!("wakeup:{}", record.task.id),
                serde_json::to_string(record)?
            ],
        )?;
        Ok(())
    }
    pub fn check_wakeup_settings(&self, s: &Settings) -> Result<()> {
        for r in self.wakeups()? {
            if let Some(d) = r.deployment {
                ensure!(
                    s.sources.iter().any(|v| v.id == d.source.id
                        && (v.account_id == d.source.account_id
                            || (v.codex_home_id.is_some()
                                && v.codex_home_id == d.source.codex_home_id
                                && d.task.codex_profile_id.is_some()))
                        && crate::providers::canonical(&v.provider)
                            == crate::providers::canonical(&d.source.provider)
                        && v.host_id == d.source.host_id),
                    "请先移除关联的定时唤醒部署，再删除来源或更换账户/主机"
                );
                ensure!(
                    s.accounts.iter().any(|a| a.id == d.source.account_id
                        && crate::providers::canonical(&a.provider)
                            == crate::providers::canonical(&d.source.provider)
                        && (d.task.codex_profile_id.is_none()
                            || d.task
                                .codex_profile_id
                                .as_deref()
                                .is_some_and(|r| a.has_profile(r)))),
                    "请先移除账户的定时唤醒部署，再归档、删除或重新绑定账户"
                );
                if let Some(h) = d.host {
                    ensure!(
                        s.hosts
                            .iter()
                            .any(|v| v.id == h.id && v.target == h.target && v.port == h.port),
                        "请先移除服务器上的定时任务，再删除或更换服务器"
                    );
                }
            }
        }
        Ok(())
    }
}
fn validate(task: &mut Task) -> Result<()> {
    if task.id.is_empty() {
        task.id = format!(
            "wake-{}",
            &hash(&format!(
                "{:?}:{}",
                std::time::SystemTime::now(),
                std::process::id()
            ))[..20]
        );
    }
    ensure!(
        task.id.starts_with("wake-")
            && task.id.len() <= 64
            && task
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "任务 ID 无效"
    );
    task.name = task.name.trim().into();
    task.model = task.model.trim().into();
    ensure!(
        !task.name.is_empty() && task.name.len() <= 200,
        "请输入任务名称（最多 200 字节）"
    );
    ensure!(
        !task.prompt.trim().is_empty() && task.prompt.len() <= 4000 && !task.prompt.contains('\0'),
        "请输入简短提示词（最多 4000 字节）"
    );
    ensure!(
        !task.model.is_empty()
            && task.model.len() <= 200
            && !task.model.starts_with('-')
            && !task.model.chars().any(char::is_whitespace),
        "请选择或填写模型 ID"
    );
    ensure!(
        [
            "", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"
        ]
        .contains(&task.effort.as_str()),
        "effort 无效"
    );
    ensure!(
        !task.times.is_empty() && task.times.len() <= 24,
        "每天需要 1–24 个固定时刻"
    );
    for t in &task.times {
        ensure!(
            t.len() == 5
                && t.is_ascii()
                && t.as_bytes()[2] == b':'
                && t.bytes()
                    .enumerate()
                    .all(|(i, b)| i == 2 || b.is_ascii_digit())
                && t[..2].parse::<u8>().is_ok_and(|n| n < 24)
                && t[3..].parse::<u8>().is_ok_and(|n| n < 60),
            "时间格式应为 HH:mm"
        );
    }
    task.times.sort();
    task.times.dedup();
    Ok(())
}
/// A draft needs a valid association, but never contacts or requires an enabled machine.
pub(crate) fn source_for_draft(s: &Settings, id: &str, profile: Option<&str>) -> Result<Source> {
    let source = s
        .sources
        .iter()
        .find(|v| v.id == id)
        .context("目标数据源已删除，请重新选择")?
        .clone();
    let mut source = source;
    ensure!(
        source.machine_binding == MachineBinding::Bound,
        "目录尚未绑定机器，请先重新关联"
    );
    if let Some(home) = &source.codex_home_id {
        ensure!(
            source.provider == "codex"
                && s.sources
                    .iter()
                    .filter(|s| s.codex_home_id.as_ref() == Some(home))
                    .count()
                    == 1,
            "Codex home 关联不唯一，请先修复目录关联"
        );
        let reference = profile.context("请选择此共享来源的唤醒账户")?;
        let (bound_home, profile_id) = reference.split_once(':').context("账户档案引用无效")?;
        crate::codex_auth::files::id(profile_id)?;
        ensure!(bound_home == home, "唤醒档案与来源不匹配");
        let mut accounts = s
            .accounts
            .iter()
            .filter(|a| a.provider == source.provider && a.has_profile(reference));
        let account = accounts.next().context("请先将账户档案关联到额度账户")?;
        ensure!(
            accounts.next().is_none(),
            "账户档案被多个账户引用，请重新关联"
        );
        ensure!(!account.archived, "账户已归档，请先恢复账户");
        ensure!(
            account
                .connections
                .iter()
                .any(|c| c.source_id == source.id && c.profile_id.as_deref() == Some(reference)),
            "唤醒档案与来源关联不匹配"
        );
        source.account_id = account.id.clone();
    } else {
        ensure!(profile.is_none(), "此来源不使用账户档案，请重新选择目标");
    }
    ensure!(
        crate::providers::supports_wakeup(&source.provider),
        "此 CLI 暂不支持订阅唤醒"
    );
    ensure!(
        s.accounts
            .iter()
            .any(|a| a.id == source.account_id && a.provider == source.provider && !a.archived),
        "请选择关联订阅账户的来源"
    );
    if let Some(id) = &source.host_id {
        ensure!(
            s.hosts.iter().any(|h| &h.id == id),
            "目标服务器已删除，请先重新关联"
        );
    }
    Ok(source)
}
fn source_for(s: &Settings, id: &str, profile: Option<&str>) -> Result<Source> {
    let source = source_for_draft(s, id, profile)?;
    ensure!(
        s.source_enabled(&source),
        "目标机器或 Agent 已暂停或未选择，请在设置中恢复后重试"
    );
    Ok(source)
}
fn config_root(source: &Source) -> String {
    let root = source.path.trim_end_matches('/');
    let root = if source.provider == "codex" {
        root.strip_suffix("/sessions")
            .or_else(|| root.strip_suffix("/archived_sessions"))
            .unwrap_or(root)
    } else if source.provider == "claude" {
        root.strip_suffix("/projects").unwrap_or(root)
    } else {
        root
    };
    if source.host_id.is_none() {
        store::expand(root).to_string_lossy().into()
    } else {
        root.into()
    }
}
fn manifest(task: &Task, source: &Source, settings: &Settings) -> Result<Manifest> {
    let configured = if !task.binary.is_empty() {
        task.binary.as_str()
    } else {
        match crate::providers::canonical(&source.provider) {
            "codex" => source.codex_binary.as_str(),
            "antigravity" => source.agy_binary.as_str(),
            provider => crate::contracts::provider(provider).map_or("", |p| p.binary),
        }
    };
    let binary: String = if configured.is_empty() {
        crate::contracts::provider(crate::providers::canonical(&source.provider))
            .map_or(source.provider.as_str(), |p| p.binary)
            .into()
    } else {
        configured.into()
    };
    let binary = if source.host_id.is_none() {
        resolve_binary(&binary)?
    } else {
        binary
    };
    let account = settings
        .accounts
        .iter()
        .find(|a| a.provider == source.provider && a.id == source.account_id);
    let pre = source
        .host_id
        .as_ref()
        .and_then(|id| settings.hosts.iter().find(|h| &h.id == id));
    let pre_command = crate::accounts::pre_command(settings, account, source);
    let args = match source.provider.as_str() {
        "codex" => vec![
            "exec",
            "--ignore-user-config",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--json",
            "--color",
            "never",
            "-c",
            "forced_login_method=\"chatgpt\"",
            "-c",
            "approval_policy=\"never\"",
            "-m",
            &task.model,
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(if task.effort.is_empty() {
            vec![]
        } else {
            vec![
                "-c".into(),
                format!("model_reasoning_effort=\"{}\"", task.effort),
            ]
        })
        .chain(["--".into(), task.prompt.clone()])
        .collect(),
        "claude" => vec![
            "--print",
            "--safe-mode",
            "--tools",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--output-format",
            "json",
            "--model",
            &task.model,
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(if task.effort.is_empty() {
            vec![]
        } else {
            vec!["--effort".into(), task.effort.clone()]
        })
        .chain(["--".into(), task.prompt.clone()])
        .collect(),
        "agy" | "antigravity" => vec![
            "--print",
            &task.prompt,
            "--mode",
            "plan",
            "--sandbox",
            "--disable-slash-commands",
            "--output-format",
            "json",
            "--print-timeout",
            "120s",
            "--model",
            &task.model,
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(if task.effort.is_empty() {
            vec![]
        } else {
            vec!["--effort".into(), task.effort.clone()]
        })
        .collect(),
        _ => anyhow::bail!("不支持的 CLI"),
    };
    Ok(Manifest {
        codex_profile_id: task.codex_profile_id.clone(),
        version: if task.codex_profile_id.is_some() {
            2
        } else {
            1
        },
        id: task.id.clone(),
        account_key: format!("{}:{}", source.provider, source.account_id),
        provider: source.provider.clone(),
        model: task.model.clone(),
        effort: task.effort.clone(),
        binary,
        args,
        config_path: config_root(source),
        pre_command,
        shell: pre
            .map(|h| h.shell.clone())
            .filter(|shell| !shell.trim().is_empty())
            .unwrap_or_else(|| ssh::DEFAULT_SHELL.into()),
        proxy: if source.host_id.is_some() {
            // Only the account × machine command configures the remote environment.
            ProxyConfig {
                mode: "system".into(),
                url: String::new(),
            }
        } else {
            network::effective_proxy(source.proxy.as_ref().unwrap_or(&settings.proxy))
        },
        times: task.times.clone(),
        enabled: true,
    })
}
fn resolve_binary(binary: &str) -> Result<String> {
    let path = store::expand(binary);
    let mut candidates = vec![
        path.clone(),
        store::home().join(".local/bin").join(binary),
        store::home()
            .join(".codex/packages/standalone/current/bin")
            .join(binary),
    ];
    candidates.extend(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(binary)),
    );
    if cfg!(windows)
        && let Some(appdata) = std::env::var_os("APPDATA")
    {
        candidates.push(PathBuf::from(appdata).join("npm").join(binary));
    }
    for p in candidates {
        let command = process::cli_command(&p.to_string_lossy());
        let resolved = PathBuf::from(command.get_program());
        if resolved.is_file() {
            return Ok(if resolved.is_absolute() {
                resolved
            } else {
                std::env::current_dir()?.join(resolved)
            }
            .to_string_lossy()
            .into());
        }
    }
    anyhow::bail!("目标机器找不到 CLI；请填写可执行文件的完整路径")
}
fn agy_models(output: &str) -> Vec<Value> {
    output.lines().filter_map(|line| {
        let (id, _) = line.split_once('\t')?;
        if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c)) { return None; }
        let effort = id.rsplit_once('-').map(|(_,e)| e).filter(|e| ["low","medium","high","xhigh","max"].contains(e));
        Some(json!({"id":id,"efforts":effort.into_iter().collect::<Vec<_>>(),"defaultEffort":effort}))
    }).collect()
}

fn remote(host: &Host, mut request: Value) -> Result<Value> {
    let script = format!(
        "_aieyes_client_version = {:?}\n{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../../../../scripts/remote_wakeup.py")
    );
    request["authHelper"] = json!(String::from_utf8(crate::codex_auth::remote::script())?);
    // Deployment/control does not need the provider's proxy pre-command.
    if request["action"] == "deploy" {
        request["script"] = json!(script);
    }
    let mut host = host.clone();
    host.pre_command.clear();
    // Bundled helpers exceed Linux's per-argument limit after shell quoting.
    // Keep the entire request off argv, including any initialization environment.
    let loader = "import json,sys; p=json.load(sys.stdin); sys.argv=['aieyes-wakeup',json.dumps(p['request'])]; exec(compile(p['script'],'<aieyes-wakeup>','exec'),{'__name__':'__main__'})";
    let command = ssh::command(
        &host,
        &format!("exec python3 -c {}", process::quote(loader)),
    )?;
    let output = process::run(
        command,
        serde_json::to_vec(&json!({"script":script,"request":request}))?,
        Duration::from_secs(90),
    )?;
    let v: Value = serde_json::from_slice(
        output
            .split(|b| *b == b'\n')
            .rev()
            .find(|v| !v.is_empty())
            .context("远程唤醒操作没有返回数据")?,
    )?;
    if let Some(error) = v["error"].as_str() {
        anyhow::bail!("{error}");
    }
    Ok(v)
}
fn root(engine: &Engine) -> Result<PathBuf> {
    Ok(Path::new(engine.store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")?
        .join("wakeups"))
}
fn public_record(r: &Record) -> Value {
    // Never expose copied Host/source configuration in a deployment status response.
    json!({"task":r.task,"deployment":r.deployment.as_ref().map(|d|json!({"task":d.task,"deployedAt":d.deployed_at,"enabled":d.enabled,"state":d.state,"timezone":d.timezone,"target":d.host.as_ref().map(|h|h.name.as_str()).unwrap_or("本机"),"changed":d.state == "settings-pending" || serde_json::to_value(&r.task).ok()!=serde_json::to_value(&d.task).ok()}))})
}
impl Engine {
    pub fn wakeup_call(&mut self, method: &str, mut p: Value) -> Result<Value> {
        let action = method.split_once('.').map(|(_, a)| a).unwrap_or("");
        if action == "save" {
            // Allocate the draft ID before selecting the operation lock.
            let mut task: Task =
                serde_json::from_value(p.get("task").cloned().unwrap_or(p.clone()))?;
            validate(&mut task)?;
            p = json!({"task":task});
        }
        let _task_lock = if [
            "save", "deploy", "remove", "delete", "enable", "disable", "run",
        ]
        .contains(&action)
        {
            let id = if action == "save" {
                p["task"]["id"].as_str()
            } else {
                p["id"].as_str()
            }
            .context("缺少任务 ID")?;
            Some(lock_task(&self.store, id)?)
        } else {
            None
        };
        self.wakeup_call_unlocked(method, p)
    }
    pub(crate) fn wakeup_call_unlocked(&mut self, method: &str, p: Value) -> Result<Value> {
        let action = method.split_once('.').map(|(_, a)| a).unwrap_or("");
        if action == "list" {
            return Ok(json!(
                self.store
                    .wakeups()?
                    .iter()
                    .map(public_record)
                    .collect::<Vec<_>>()
            ));
        }
        let settings = self.store.settings()?;
        if action == "save" {
            let mut task: Task =
                serde_json::from_value(p.get("task").cloned().unwrap_or(p.clone()))?;
            validate(&mut task)?;
            let source =
                source_for_draft(&settings, &task.source_id, task.codex_profile_id.as_deref())?;
            if let Some(account) = settings
                .accounts
                .iter()
                .find(|a| a.provider == source.provider && a.id == source.account_id)
            {
                crate::accounts::ensure_not_deleting(&self.store, account)?;
            }
            let old = self
                .store
                .wakeups()?
                .into_iter()
                .find(|r| r.task.id == task.id);
            if let Some(d) = old.as_ref().and_then(|r| r.deployment.as_ref()) {
                ensure!(
                    d.task.source_id == task.source_id
                        && d.task.codex_profile_id == task.codex_profile_id,
                    "更换部署位置前请先移除旧部署"
                );
            }
            let r = Record {
                task,
                deployment: old.and_then(|r| r.deployment),
            };
            self.store.save_wakeup(&r)?;
            return Ok(public_record(&r));
        }
        if action == "probe" {
            let mut task: Task =
                serde_json::from_value(p.get("task").cloned().unwrap_or(p.clone()))?;
            if task.id.is_empty() {
                task.id = "wake-probe".into();
            }
            let source = source_for(&settings, &task.source_id, task.codex_profile_id.as_deref())?;
            let m = manifest(&task, &source, &settings)?;
            let mut info = if let Some(id) = &source.host_id {
                remote(
                    settings.hosts.iter().find(|h| &h.id == id).unwrap(),
                    json!({"action":"probe","manifest":m}),
                )?
            } else {
                self.local_probe(&m, &source, &settings)?
            };
            let prices = self.store.prices()?;
            let mut rows: Vec<Value> = info["models"].as_array().cloned().unwrap_or_default();
            for row in &mut rows {
                let price = pricing::find_price(
                    row["id"].as_str().unwrap_or(""),
                    &prices,
                    &settings.model_mappings,
                );
                row["referenceCost"] = price
                    .and_then(|p| Some(p.input? * 1000.0 + p.output? * 100.0))
                    .map_or(Value::Null, |v| json!(v));
            }
            rows.sort_by(|a, b| {
                a["referenceCost"]
                    .as_f64()
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&b["referenceCost"].as_f64().unwrap_or(f64::INFINITY))
            });
            info["recommendedModel"] = rows
                .first()
                .filter(|r| r["referenceCost"].as_f64().is_some())
                .map(|r| r["id"].clone())
                .unwrap_or(Value::Null);
            info["models"] = json!(rows);
            return Ok(info);
        }
        let id = p["id"].as_str().context("缺少任务 ID")?;
        let mut r = self.store.wakeup(id)?;
        if action == "delete" {
            ensure!(r.deployment.is_none(), "请先移除已部署的自动任务");
            self.store
                .db
                .execute("DELETE FROM kv WHERE key=?1", [format!("wakeup:{id}")])?;
            return Ok(json!({"deleted":true}));
        }
        if action == "deploy" {
            let deploying_task = if p["useDeployedTask"] == true {
                r.deployment
                    .as_ref()
                    .map(|d| d.task.clone())
                    .unwrap_or_else(|| r.task.clone())
            } else {
                r.task.clone()
            };
            let source = source_for(
                &settings,
                &deploying_task.source_id,
                deploying_task.codex_profile_id.as_deref(),
            )?;
            if let Some(account) = settings
                .accounts
                .iter()
                .find(|a| a.provider == source.provider && a.id == source.account_id)
            {
                crate::accounts::ensure_not_deleting(&self.store, account)?;
            }
            let mut m = manifest(&deploying_task, &source, &settings)?;
            m.enabled = p["preserveEnabled"].as_bool().unwrap_or(true);
            let host = source
                .host_id
                .as_ref()
                .and_then(|id| settings.hosts.iter().find(|h| &h.id == id))
                .cloned();
            let local_root = root(self)?;
            let namespace = &hash(&local_root.to_string_lossy())[..20];
            if host.is_none() {
                self.local_probe(&m, &source, &settings)?;
            }
            let intended_root = host
                .as_ref()
                .map(|_| format!("~/.local/share/aieyes-wakeups/{namespace}"))
                .unwrap_or_else(|| local_root.to_string_lossy().into());
            let was_settings_pending = r.deployment.as_ref().is_some_and(|deployed| {
                deployed.state == "settings-pending"
                    || deployed.account_environment_changed(&settings)
            });
            let mut pending = r.deployment.clone().unwrap_or(Deployment {
                task: deploying_task.clone(),
                source: source.clone(),
                host: host.clone(),
                root: intended_root,
                deployed_at: 0,
                enabled: false,
                state: String::new(),
                timezone: "目标机器时区".into(),
            });
            pending.state = if was_settings_pending {
                // Keep this marker until the new environment is confirmed,
                // including failures after the target responded but before the
                // local commit. A later start must still retry synchronization.
                "settings-pending"
            } else {
                "deployment-unconfirmed"
            }
            .into();
            r.deployment = Some(pending);
            // Record ownership before touching the target, so a lost SSH response cannot orphan a job.
            self.store.save_wakeup(&r)?;
            let result = if let Some(h) = &host {
                remote(
                    h,
                    json!({"action":"deploy","namespace":namespace,"manifest":m}),
                )?
            } else {
                scheduler::deploy(&local_root, &m)?
            };
            let tx = self.store.write_transaction()?;
            let latest = self.store.settings()?;
            let command_changed = latest
                .sources
                .iter()
                .find(|s| s.id == source.id)
                .is_none_or(|s| {
                    let account = latest
                        .accounts
                        .iter()
                        .find(|a| a.id == source.account_id && a.provider == source.provider);
                    crate::accounts::pre_command(&latest, account, s) != m.pre_command
                });
            let mut deployed_source = source;
            deployed_source.quota_pre_command = m.pre_command.clone();
            if deployed_source.host_id.is_some() {
                deployed_source.proxy = None;
            }
            r.deployment = Some(Deployment {
                task: deploying_task.clone(),
                source: deployed_source,
                host: host.map(|mut h| {
                    h.pre_command = m.pre_command.clone();
                    h
                }),
                root: result["root"].as_str().context("部署目录缺失")?.into(),
                deployed_at: now(),
                enabled: m.enabled,
                state: if command_changed {
                    "settings-pending"
                } else {
                    "deployed"
                }
                .into(),
                timezone: result["timezone"].as_str().unwrap_or("目标机器时区").into(),
            });
            self.store.save_wakeup(&r)?;
            tx.commit()?;
            return Ok(public_record(&r));
        }
        if action == "run" || action == "enable" {
            let deployed = r.deployment.as_ref().context("任务尚未部署")?;
            if action == "run" {
                source_for(
                    &settings,
                    &deployed.task.source_id,
                    deployed.task.codex_profile_id.as_deref(),
                )?;
            }
            // Removal or status recovery may have changed the display state.
            // Always compare the actual deployed environment before starting.
            if self.sync_wakeup_settings_unlocked(id)?.is_some() {
                r = self.store.wakeup(id)?;
            }
        }
        let d = r.deployment.as_ref().context("任务尚未部署")?;
        let result = if let Some(host) = &d.host {
            remote(host, json!({"action":action,"root":d.root,"id":id}))
        } else {
            scheduler::control(Path::new(&d.root), id, action)
        };
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                if action == "remove" {
                    let tx = self.store.write_transaction()?;
                    let mut latest = self.store.wakeup(id)?;
                    if let Some(deployment) = &mut latest.deployment {
                        deployment.state = "pending-removal".into();
                    }
                    self.store.save_wakeup(&latest)?;
                    tx.commit()?;
                }
                return Err(e);
            }
        };
        if matches!(action, "status" | "history" | "run") {
            return Ok(value);
        }
        // A concurrent settings save may have marked this deployment pending
        // while the target was responding. Merge only the operation's result.
        let tx = self.store.write_transaction()?;
        let mut latest = self.store.wakeup(id)?;
        match action {
            "remove" => latest.deployment = None,
            "enable" | "disable" => {
                let d = latest.deployment.as_mut().context("任务尚未部署")?;
                d.enabled = action == "enable";
                if d.state != "settings-pending" {
                    d.state = "deployed".into();
                }
            }
            _ => anyhow::bail!("未知任务操作"),
        }
        self.store.save_wakeup(&latest)?;
        tx.commit()?;
        Ok(public_record(&latest))
    }
    /// Caller holds this task's lock. Keep a failed update pending so the next
    /// start, enable or retry cannot use the previously deployed environment.
    pub(crate) fn sync_wakeup_settings_unlocked(&mut self, id: &str) -> Result<Option<Value>> {
        let current = self.store.wakeup(id)?;
        let deployment = current.deployment.as_ref().context("任务尚未部署")?;
        if deployment.state != "settings-pending"
            && !deployment.account_environment_changed(&self.store.settings()?)
        {
            return Ok(None);
        }
        let result = self
            .wakeup_call_unlocked(
                "wakeups.deploy",
                json!({"id":id,"preserveEnabled":deployment.enabled,"useDeployedTask":true}),
            )
            .and_then(|value| {
                ensure!(
                    value["deployment"]["state"] != "settings-pending",
                    "同步期间配置再次变化，请重试同步"
                );
                Ok(Some(value))
            });
        if result.is_err() {
            let mut current = self.store.wakeup(id)?;
            if let Some(deployment) = &mut current.deployment {
                deployment.state = "settings-pending".into();
                self.store.save_wakeup(&current)?;
            }
        }
        result
    }
    fn local_probe(&self, m: &Manifest, source: &Source, settings: &Settings) -> Result<Value> {
        let help = runner::invoke(m, &["--help".into()], &std::env::temp_dir(), 15)?;
        let help = String::from_utf8_lossy(&help);
        ensure!(help.contains("--model"), "CLI 版本不支持指定模型，请升级");
        runner::auth_check(m, &std::env::temp_dir())?;
        let mut models = vec![];
        if source.provider == "codex" {
            let mut source = source.clone();
            source.codex_binary = m.binary.clone();
            let account = if let Some(reference) = &m.codex_profile_id {
                crate::codex_auth::backend(
                    &source,
                    settings,
                    "model.list",
                    &json!({"profileId":reference.split_once(':').context("档案引用无效")?.1}),
                )?
            } else {
                quota::codex_rpc(
                    &source,
                    settings,
                    "account/read",
                    json!({"refreshToken":false}),
                )?
            };
            ensure!(
                matches!(
                    account["account"]["type"].as_str(),
                    Some("chatgpt" | "chatgptAuthTokens")
                ),
                "Codex 唤醒需要 ChatGPT 订阅登录"
            );
            let v = if m.codex_profile_id.is_some() {
                account["models"].clone()
            } else {
                quota::codex_rpc(
                    &source,
                    settings,
                    "model/list",
                    json!({"includeHidden":false}),
                )?
            };
            for row in v["data"].as_array().into_iter().flatten() {
                models.push(json!({"id":row["model"].as_str().or(row["id"].as_str()).unwrap_or(""),"efforts":row["supportedReasoningEfforts"].as_array().map(|a|a.iter().filter_map(|v|v["reasoningEffort"].as_str()).collect::<Vec<_>>()).unwrap_or_default(),"defaultEffort":row["defaultReasoningEffort"]}));
            }
            let exec = runner::invoke(
                m,
                &["exec".into(), "--help".into()],
                &std::env::temp_dir(),
                15,
            )?;
            ensure!(
                String::from_utf8_lossy(&exec).contains("--ignore-user-config"),
                "请升级 Codex CLI 以支持隔离唤醒配置"
            );
        } else if source.provider == "claude" {
            ensure!(
                help.contains("--safe-mode") && help.contains("--tools"),
                "请升级 Claude Code CLI"
            );
        }
        if crate::providers::antigravity(&source.provider) {
            let output = runner::invoke(m, &["models".into()], &std::env::temp_dir(), 20)?;
            models = agy_models(&String::from_utf8_lossy(&output));
        }
        if !m.model.is_empty() && !models.is_empty() {
            let model = models
                .iter()
                .find(|v| v["id"] == m.model)
                .context("所选模型不在当前可用列表中")?;
            ensure!(
                m.effort.is_empty()
                    || model["efforts"]
                        .as_array()
                        .is_some_and(|e| e.contains(&json!(m.effort))),
                "所选模型不支持该 effort"
            );
        }
        ensure!(
            m.effort.is_empty() || source.provider == "codex" || help.contains("--effort"),
            "当前 CLI 不支持推理强度参数，请选择模型默认值"
        );
        Ok(
            json!({"binary":m.binary,"models":models,"efforts":if source.provider=="codex"||help.contains("--effort"){vec!["low","medium","high","xhigh","max"]}else{vec![]},"timezone":chrono::Local::now().format("%Z %:z").to_string(),"message":if models.is_empty(){"CLI 未提供模型列表，请填写模型 ID；运行时检查可用性"}else{"参考 1000 输入 + 100 输出 Token 的 API 等价成本推荐模型；部署后使用所选模型"}}),
        )
    }
}

pub(crate) struct TaskLock {
    _ownership: crate::file_lock::FileLock,
    _task: crate::file_lock::FileLock,
}
pub(crate) fn lock_task(store: &Store, id: &str) -> Result<TaskLock> {
    let ownership = crate::accounts::lock_task_ownership(store, true)?;
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root.join(format!("wakeup-{}.lock", hash(id))))?;
    let task = crate::file_lock::FileLock::try_exclusive(file)
        .map_err(|_| anyhow::anyhow!("此唤醒任务正在操作，请稍后重试"))?;
    Ok(TaskLock {
        _ownership: ownership,
        _task: task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paused_targets_allow_valid_drafts_but_reject_deploy_probe_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "source".into(),
            provider: "claude".into(),
            account_id: "account".into(),
            path: "~/.claude".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        let host = Host {
            id: "remote".into(),
            enabled: false,
            target: "fixture.invalid".into(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            hosts: vec![host.clone()],
            accounts: vec![Account {
                id: "account".into(),
                name: "Fixture account".into(),
                provider: "claude".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        engine.store.save_settings(&settings).unwrap();
        let task = Task {
            id: "wake-paused".into(),
            name: "Paused draft".into(),
            source_id: "source".into(),
            model: "fixture-model".into(),
            ..Default::default()
        };
        let saved = engine
            .wakeup_call("wakeups.save", json!({"task":task}))
            .unwrap();
        assert!(saved["deployment"].is_null());
        assert!(!dir.path().join("wakeups").exists());
        for (method, params) in [
            ("wakeups.deploy", json!({"id":task.id})),
            ("wakeups.probe", json!({"task":task})),
        ] {
            assert!(
                engine
                    .wakeup_call(method, params)
                    .unwrap_err()
                    .to_string()
                    .contains("暂停")
            );
        }
        let record = Record {
            task: task.clone(),
            deployment: Some(Deployment {
                task: task.clone(),
                source,
                host: Some(host),
                root: "~/fixture".into(),
                deployed_at: 1,
                enabled: false,
                state: "deployed".into(),
                timezone: "UTC".into(),
            }),
        };
        engine.store.save_wakeup(&record).unwrap();
        assert!(
            engine
                .wakeup_call("wakeups.run", json!({"id":task.id}))
                .unwrap_err()
                .to_string()
                .contains("暂停")
        );
        let mut edited = task.clone();
        edited.name = "Edited offline".into();
        engine
            .wakeup_call("wakeups.save", json!({"task":edited}))
            .unwrap();
        assert_eq!(
            engine.store.wakeup(&task.id).unwrap().task.name,
            "Edited offline"
        );
    }
    #[test]
    fn offline_shared_drafts_still_require_the_correct_unique_profile_association() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "shared".into(),
            path: "/fixture/codex".into(),
            codex_home_id: Some("home".into()),
            enabled: false,
            ..Default::default()
        };
        let mut settings = Settings {
            sources: vec![source],
            local_device_enabled: false,
            accounts: vec![Account {
                id: "a".into(),
                name: "Fixture account".into(),
                connections: vec![AccountConnection {
                    source_id: "shared".into(),
                    profile_id: Some("home:profile".into()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        engine.store.save_settings(&settings).unwrap();
        let mut task = Task {
            name: "Offline shared".into(),
            source_id: "shared".into(),
            codex_profile_id: Some("home:profile".into()),
            model: "fixture-model".into(),
            ..Default::default()
        };
        engine
            .wakeup_call("wakeups.save", json!({"task":task}))
            .unwrap();
        for profile in [
            None,
            Some("other:profile"),
            Some("home:unbound"),
            Some("home:../../invalid"),
        ] {
            task.codex_profile_id = profile.map(str::to_owned);
            assert!(
                engine
                    .wakeup_call("wakeups.save", json!({"task":task}))
                    .is_err(),
                "{profile:?}"
            );
        }
        settings.accounts[0].archived = true;
        engine.store.save_settings(&settings).unwrap();
        task.codex_profile_id = Some("home:profile".into());
        assert!(
            engine
                .wakeup_call("wakeups.save", json!({"task":task}))
                .unwrap_err()
                .to_string()
                .contains("归档")
        );
    }
    #[test]
    fn a_busy_task_does_not_block_other_drafts_but_keeps_account_cleanup_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        engine
            .store
            .save_settings(&Settings {
                sources: vec![Source {
                    id: "s".into(),
                    path: "/fixture/codex".into(),
                    account_id: "a".into(),
                    ..Default::default()
                }],
                accounts: vec![Account {
                    id: "a".into(),
                    name: "Fixture account".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        let held = lock_task(&engine.store, "wake-first").unwrap();
        let mut task = Task {
            id: "wake-first".into(),
            name: "Draft".into(),
            source_id: "s".into(),
            model: "fixture".into(),
            ..Default::default()
        };
        assert!(
            engine
                .wakeup_call("wakeups.save", json!({"task":task}))
                .unwrap_err()
                .to_string()
                .contains("此唤醒任务")
        );
        task.id = "wake-second".into();
        engine
            .wakeup_call("wakeups.save", json!({"task":task}))
            .unwrap();
        assert!(crate::accounts::lock_tasks(&engine.store).is_err());
        drop(held);
        let _cleanup = crate::accounts::lock_tasks(&engine.store).unwrap();
        assert!(
            engine
                .wakeup_call("wakeups.save", json!({"task":task}))
                .is_err()
        );
    }
    #[cfg(unix)]
    fn check_local_model_probe_inherits_account_command(managed: bool) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let binary = root.join("fake-codex");
        std::fs::write(&binary, r#"#!/usr/bin/env python3
import json, os, sys
if '--version' in sys.argv:
    print('codex-cli 0.161.0'); sys.exit(0)
if '--help' in sys.argv:
    print('--model --ignore-user-config'); sys.exit(0)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'account/read': result = {'account': {'type': 'chatgpt'}}
    elif method == 'model/list':
        result = {'data': [{'model': os.environ.get('AIEYES_WAKE_FIXTURE', 'plain'), 'supportedReasoningEfforts': [{'reasoningEffort': 'low'}]}]}
    else: result = {}
    print(json.dumps({'id': request['id'], 'result': result}), flush=True)
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut engine = Engine::open(&root.join("db")).unwrap();
        let source = Source {
            id: "source".into(),
            account_id: if managed {
                String::new()
            } else {
                "first".into()
            },
            path: root.to_string_lossy().into(),
            codex_binary: binary.to_string_lossy().into(),
            codex_home_id: managed.then(|| "home".into()),
            quota_pre_command: "export AIEYES_WAKE_FIXTURE=legacy".into(),
            ..Default::default()
        };
        let mut settings = Settings {
            sources: vec![source],
            accounts: vec![Account {
                id: "first".into(),
                name: "First".into(),
                device_settings: vec![AccountDeviceSettings {
                    machine_id: "local".into(),
                    pre_command: "export AIEYES_WAKE_FIXTURE=first".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        if managed {
            use base64::Engine as _;
            let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                json!({"https://api.openai.com/auth":{"chatgpt_user_id":"second","chatgpt_account_id":"team"}}).to_string(),
            );
            crate::codex_auth::files::save_profile(&root, "second", "Second", &json!({
                "auth_mode":"chatgpt", "tokens":{"access_token":format!("h.{claims}.s"), "refresh_token":"fixture", "account_id":"team"}
            })).unwrap();
            settings.accounts.push(Account {
                id: "second".into(),
                name: "Second".into(),
                connections: vec![AccountConnection {
                    source_id: "source".into(),
                    profile_id: Some("home:second".into()),
                }],
                ..Default::default()
            });
        }
        let task = Task {
            source_id: "source".into(),
            codex_profile_id: managed.then(|| "home:second".into()),
            effort: String::new(),
            ..Default::default()
        };
        let selected = if managed { 1 } else { 0 };
        for (command, expected) in [
            (
                Some(
                    "export AIEYES_WAKE_FIXTURE=selected\nprintf 'private banner\\n'\nread ignored || true",
                ),
                "selected",
            ),
            (Some(""), "plain"),
            (None, "legacy"),
        ] {
            settings.accounts[selected].device_settings = command
                .map(|pre_command| AccountDeviceSettings {
                    machine_id: "local".into(),
                    pre_command: pre_command.into(),
                    ..Default::default()
                })
                .into_iter()
                .chain([AccountDeviceSettings {
                    machine_id: "other-machine".into(),
                    pre_command: "export AIEYES_WAKE_FIXTURE=wrong".into(),
                    ..Default::default()
                }])
                .collect();
            engine.store.save_settings(&settings).unwrap();
            let info = engine
                .wakeup_call("wakeups.probe", json!({"task":task}))
                .unwrap();
            assert_eq!(info["models"][0]["id"], expected);
        }
        assert!(!root.join(".aieyes/codex/second/query/auth.json").exists());
    }
    #[cfg(unix)]
    #[test]
    fn local_model_probe_inherits_account_command() {
        check_local_model_probe_inherits_account_command(false);
    }
    #[cfg(unix)]
    #[test]
    fn local_profile_model_probe_inherits_selected_account_command() {
        check_local_model_probe_inherits_account_command(true);
    }
    #[test]
    fn times_are_validated_without_unicode_panics_and_duplicates_removed() {
        let mut t = Task {
            name: "test".into(),
            model: "gpt-6-luna".into(),
            times: vec!["23:59".into(), "08:00".into(), "08:00".into()],
            ..Default::default()
        };
        validate(&mut t).unwrap();
        assert_eq!(t.times, vec!["08:00", "23:59"]);
        for invalid in ["24:00", "08:60", "8:00", "你ab", "00;00", "+8:00", "08:+1"] {
            t.times = vec![invalid.into()];
            assert!(validate(&mut t).is_err());
        }
    }
    #[test]
    fn drafts_do_not_deploy_and_installed_source_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            account_id: "a".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        let mut s = Settings {
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: "a".into(),
                name: "Test".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        e.store.save_settings(&s).unwrap();
        let v=e.wakeup_call("wakeups.save",json!({"task":{"name":"daily","sourceId":"s","model":"gpt-6-luna","times":["08:00"]}})).unwrap();
        let id = v["task"]["id"].as_str().unwrap();
        assert!(v["deployment"].is_null());
        assert!(!dir.path().join("wakeups").exists());
        let mut r = e.store.wakeup(id).unwrap();
        r.deployment = Some(Deployment {
            task: r.task.clone(),
            source,
            host: None,
            root: "/fixture".into(),
            deployed_at: now(),
            enabled: true,
            state: "deployed".into(),
            timezone: "UTC".into(),
        });
        e.store.save_wakeup(&r).unwrap();
        s.sources.clear();
        assert!(e.store.save_settings(&s).is_err());
        assert!(e.wakeup_call("wakeups.delete", json!({"id":id})).is_err());
    }
    #[test]
    fn account_command_changes_mark_deployments_pending_without_touching_remote_or_enabling_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            provider: "claude".into(),
            account_id: "a".into(),
            path: "~/.claude".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        let host = Host {
            id: "remote".into(),
            target: "fixture.invalid".into(),
            ..Default::default()
        };
        let mut settings = Settings {
            sources: vec![source.clone()],
            hosts: vec![host.clone()],
            accounts: vec![Account {
                id: "a".into(),
                name: "A".into(),
                provider: "claude".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        e.store.save_settings(&settings).unwrap();
        let task = Task {
            id: "job".into(),
            source_id: "s".into(),
            model: "fixture-model".into(),
            ..Default::default()
        };
        let record = Record {
            task: task.clone(),
            deployment: Some(Deployment {
                task: task.clone(),
                source: source.clone(),
                host: Some(host),
                root: "~/fixture".into(),
                deployed_at: 1,
                enabled: false,
                state: "deployed".into(),
                timezone: "UTC".into(),
            }),
        };
        e.store.save_wakeup(&record).unwrap();
        settings.accounts[0].device_settings = vec![AccountDeviceSettings {
            machine_id: "remote".into(),
            pre_command: "export ACCOUNT_PROXY=fixture".into(),
            ..Default::default()
        }];
        e.store.save_settings(&settings).unwrap();
        let record = e.store.wakeup("job").unwrap();
        assert_eq!(
            record.deployment.as_ref().unwrap().state,
            "settings-pending"
        );
        assert!(!record.deployment.as_ref().unwrap().enabled);
        assert_eq!(
            manifest(&task, &source, &settings).unwrap().pre_command,
            "export ACCOUNT_PROXY=fixture"
        );
        assert_eq!(public_record(&record)["deployment"]["changed"], true);
    }
    #[test]
    fn failed_command_sync_stays_pending_before_run_or_enable_and_keeps_the_draft() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            provider: "claude".into(),
            account_id: "a".into(),
            path: "~/.claude".into(),
            host_id: Some("remote".into()),
            ..Default::default()
        };
        // An invalid fixture target fails before any SSH process is started,
        // after the attempted deployment has recorded ownership locally.
        let host = Host {
            id: "remote".into(),
            target: "-invalid-fixture".into(),
            ..Default::default()
        };
        let settings = Settings {
            version: 10,
            sources: vec![source.clone()],
            hosts: vec![host.clone()],
            accounts: vec![Account {
                id: "a".into(),
                name: "Fixture".into(),
                provider: "claude".into(),
                device_settings: vec![AccountDeviceSettings {
                    machine_id: "remote".into(),
                    pre_command: "export ACCOUNT_ENV=new".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        engine
            .store
            .db
            .execute(
                "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
                [json!(settings).to_string()],
            )
            .unwrap();
        let task = Task {
            id: "wake-pending".into(),
            name: "Installed task".into(),
            source_id: source.id.clone(),
            model: "fixture".into(),
            ..Default::default()
        };
        let mut draft = task.clone();
        draft.name = "Unsynced draft".into();
        draft.prompt = "Keep the draft prompt".into();
        let record = Record {
            task: draft.clone(),
            deployment: Some(Deployment {
                task: task.clone(),
                source,
                host: Some(host),
                root: "~/fixture".into(),
                deployed_at: 1,
                enabled: false,
                state: "settings-pending".into(),
                timezone: "UTC".into(),
            }),
        };
        engine.store.save_wakeup(&record).unwrap();
        for method in [
            "wakeups.deploy",
            "wakeups.run",
            "wakeups.run",
            "wakeups.enable",
            "wakeups.enable",
        ] {
            let error = engine
                .wakeup_call(method, json!({"id":task.id}))
                .unwrap_err();
            assert!(
                error.to_string().contains("SSH 主机格式不正确"),
                "{method}: {error}"
            );
            let current = engine.store.wakeup(&task.id).unwrap();
            let deployed = current.deployment.unwrap();
            assert_eq!(deployed.state, "settings-pending", "{method}");
            assert!(!deployed.enabled, "{method}");
            assert_eq!(json!(current.task), json!(draft));
            assert_eq!(json!(deployed.task), json!(task));
        }
        let result = engine
            .account_call("accounts.deployments.sync", json!({"taskId":task.id}))
            .unwrap();
        assert_eq!(result["failedTasks"].as_array().unwrap().len(), 1);
        assert!(result["syncedTasks"].as_array().unwrap().is_empty());
        assert_eq!(
            engine
                .store
                .wakeup(&task.id)
                .unwrap()
                .deployment
                .unwrap()
                .state,
            "settings-pending"
        );
    }
    #[test]
    fn upgrade_persists_conflict_reset_and_remote_proxy_sync_without_enabling_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        let host = Host {
            id: "remote".into(),
            target: "fixture.invalid".into(),
            enabled: false,
            pre_command: "legacy host".into(),
            ..Default::default()
        };
        let sources = ["one", "two"]
            .into_iter()
            .map(|id| Source {
                id: id.into(),
                provider: "claude".into(),
                account_id: "a".into(),
                host_id: Some("remote".into()),
                path: format!("~/.claude/{id}"),
                quota_pre_command: format!("legacy {id}"),
                proxy: Some(ProxyConfig {
                    mode: "custom".into(),
                    url: "http://legacy.invalid:8080".into(),
                }),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let old = Settings {
            version: 9,
            sources: sources.clone(),
            hosts: vec![host.clone()],
            accounts: vec![Account {
                id: "a".into(),
                name: "Fixture account".into(),
                provider: "claude".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        engine
            .store
            .db
            .execute(
                "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
                [json!(old).to_string()],
            )
            .unwrap();
        engine
            .store
            .db
            .execute(
                "DELETE FROM kv WHERE key='account-commands.migrated.v10'",
                [],
            )
            .unwrap();
        for (index, source) in sources.into_iter().enumerate() {
            let task = Task {
                id: format!("wake-{}", source.id),
                name: source.id.clone(),
                source_id: source.id.clone(),
                model: "fixture".into(),
                ..Default::default()
            };
            engine
                .store
                .save_wakeup(&Record {
                    task: task.clone(),
                    deployment: Some(Deployment {
                        task,
                        source,
                        host: Some(host.clone()),
                        root: "~/fixture".into(),
                        deployed_at: 1,
                        enabled: index == 0,
                        state: "deployed".into(),
                        timezone: "UTC".into(),
                    }),
                })
                .unwrap();
        }
        drop(engine);
        engine = Engine::open(dir.path()).unwrap();
        let mut settings = engine.store.settings().unwrap();
        assert_eq!(settings.version, 10);
        assert!(settings.accounts[0].device_settings[0].pre_command_reset_required);
        for source in &settings.sources {
            assert!(source.quota_pre_command.is_empty());
            assert_eq!(
                crate::accounts::pre_command(&settings, Some(&settings.accounts[0]), source),
                ""
            );
            let task = Task {
                source_id: source.id.clone(),
                ..Default::default()
            };
            assert_eq!(
                manifest(&task, source, &settings).unwrap().proxy.mode,
                "system"
            );
        }
        let records = engine.store.wakeups().unwrap();
        assert!(
            records
                .iter()
                .all(|r| r.deployment.as_ref().unwrap().state == "settings-pending")
        );
        let enabled = records
            .iter()
            .map(|r| r.deployment.as_ref().unwrap().enabled)
            .collect::<Vec<_>>();
        let result = engine
            .account_call("accounts.deployments.sync", json!({}))
            .unwrap();
        assert_eq!(result["failedTasks"].as_array().unwrap().len(), 2);
        let result = engine
            .account_call("accounts.deployments.sync", json!({"taskId":"wake-one"}))
            .unwrap();
        assert_eq!(result["failedTasks"].as_array().unwrap().len(), 1);
        assert_eq!(result["failedTasks"][0]["id"], "wake-one");
        assert!(result["syncedTasks"].as_array().unwrap().is_empty());
        assert_eq!(
            engine
                .store
                .wakeups()
                .unwrap()
                .iter()
                .map(|r| r.deployment.as_ref().unwrap().enabled)
                .collect::<Vec<_>>(),
            enabled
        );
        settings.accounts[0].device_settings[0] = AccountDeviceSettings {
            machine_id: "remote".into(),
            pre_command: "export SHARED=ready".into(),
            ..Default::default()
        };
        engine.store.save_settings(&settings).unwrap();
        drop(engine);
        let reopened = Engine::open(dir.path()).unwrap().store.settings().unwrap();
        assert!(!reopened.accounts[0].device_settings[0].pre_command_reset_required);
        assert_eq!(
            reopened.accounts[0].device_settings[0].pre_command,
            "export SHARED=ready"
        );
    }
    #[test]
    fn remote_legacy_proxy_values_do_not_change_manifests_or_codex_locations() {
        let task = Task {
            model: "fixture".into(),
            ..Default::default()
        };
        let mut source = Source {
            host_id: Some("remote".into()),
            ..Default::default()
        };
        let settings = Settings {
            proxy: ProxyConfig {
                mode: "custom".into(),
                url: "http://local.invalid:8080".into(),
            },
            ..Default::default()
        };
        for mode in ["direct", "custom", "invalid-legacy"] {
            source.proxy = Some(ProxyConfig {
                mode: mode.into(),
                url: "http://legacy.invalid:8080".into(),
            });
            let manifest = manifest(&task, &source, &settings).unwrap();
            assert_eq!(manifest.proxy.mode, "system");
            assert!(manifest.proxy.url.is_empty());
            let location = crate::codex_auth::location(&source, &settings);
            assert_eq!(location.proxy.mode, "system");
            assert!(location.proxy.url.is_empty());
        }
        source.host_id = None;
        source.proxy = Some(ProxyConfig {
            mode: "direct".into(),
            url: String::new(),
        });
        assert_eq!(
            crate::codex_auth::location(&source, &settings).proxy.mode,
            "direct"
        );
    }
    #[test]
    fn response_requires_terminal_success_not_just_exit_zero() {
        assert!(!runner::response_ok("codex", br#"{"type":"turn.failed"}"#));
        assert!(runner::response_ok(
            "codex",
            br#"{"type":"turn.completed"}"#
        ));
        assert!(!runner::response_ok(
            "claude",
            br#"{"type":"result","subtype":"success","is_error":true}"#
        ));
        assert!(!runner::response_ok("agy", br#"{"status":"ERROR"}"#));
    }
    #[cfg(unix)]
    #[test]
    fn independent_accounts_run_concurrently() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake-cli");
        std::fs::write(
            &binary,
            r#"#!/bin/sh
touch ready
for n in $(seq 1 200); do
  if [ -f ../wake-a/ready ] && [ -f ../wake-b/ready ]; then
    echo '{"type":"turn.completed"}'
    exit 0
  fi
  sleep 0.01
done
exit 1
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = dir.path().join("runner");
        for id in ["wake-a", "wake-b"] {
            let t = Task {
                id: id.into(),
                binary: binary.to_string_lossy().into(),
                model: "fixture".into(),
                times: vec![chrono::Local::now().format("%H:%M").to_string()],
                ..Default::default()
            };
            let source = Source {
                account_id: id.into(),
                path: dir.path().to_string_lossy().into(),
                ..Default::default()
            };
            let m = manifest(&t, &source, &Settings::default()).unwrap();
            runner::atomic_json(&root.join("tasks").join(format!("{id}.json")), &m).unwrap();
        }
        runner::run(&root, None).unwrap();
        for id in ["wake-a", "wake-b"] {
            assert_eq!(runner::history(&root, id).unwrap()[0]["status"], "success");
        }
    }
    #[cfg(unix)]
    #[test]
    fn headless_runner_skips_missed_slots_deduplicates_and_keeps_prompt_literal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake cli");
        std::fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"turn.completed\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let settings = Settings::default();
        let source = Source {
            path: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        let mut task = Task {
            id: "wake-test".into(),
            name: "Test".into(),
            binary: binary.to_string_lossy().into(),
            model: "gpt-6-luna".into(),
            prompt: "$(touch should-not-exist) ' \" ;".into(),
            times: vec![chrono::Local::now().format("%H:%M").to_string()],
            ..Default::default()
        };
        let m = manifest(&task, &source, &settings).unwrap();
        let root = dir.path().join("runner");
        runner::atomic_json(&root.join("tasks/wake-test.json"), &m).unwrap();
        runner::run(&root, None).unwrap();
        runner::run(&root, None).unwrap();
        let h = runner::history(&root, &task.id).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0]["status"], "success");
        assert!(!root.join("work/wake-test/should-not-exist").exists());
        task.id = "wake-missed".into();
        task.times = vec![
            (chrono::Local::now() - chrono::Duration::minutes(1))
                .format("%H:%M")
                .to_string(),
        ];
        let m = manifest(&task, &source, &settings).unwrap();
        runner::atomic_json(&root.join("tasks/wake-missed.json"), &m).unwrap();
        runner::run(&root, None).unwrap();
        assert!(runner::history(&root, &task.id).unwrap().is_empty());
    }
}
