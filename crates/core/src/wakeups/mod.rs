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
fn source_for(s: &Settings, id: &str, profile: Option<&str>) -> Result<Source> {
    let source = s
        .sources
        .iter()
        .find(|v| v.id == id && v.enabled)
        .context("请选择已启用的目标数据源")?
        .clone();
    let mut source = source;
    if source.codex_home_id.is_some() {
        let reference = profile.context("请选择此共享来源的唤醒账号")?;
        let (bound, _) = crate::codex_auth::source_for_profile(s, reference)?;
        ensure!(bound.id == source.id, "唤醒档案与来源不匹配");
        source.account_id = s
            .accounts
            .iter()
            .find(|a| a.has_profile(reference) && !a.archived)
            .context("请先将账号档案关联到额度账户")?
            .id
            .clone();
    }
    ensure!(
        ["codex", "claude", "antigravity", "agy"].contains(&source.provider.as_str()),
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
            s.hosts.iter().any(|h| &h.id == id && h.enabled),
            "目标服务器未启用"
        );
    }
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
        match source.provider.as_str() {
            "codex" => source.codex_binary.as_str(),
            "agy" | "antigravity" => source.agy_binary.as_str(),
            _ => "claude",
        }
    };
    let binary = if configured.is_empty() {
        source.provider.clone()
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
            source.proxy.clone().unwrap_or(ProxyConfig {
                mode: "system".into(),
                url: String::new(),
            })
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
    request["authHelper"] = json!(include_str!("../../../../scripts/remote_codex_auth.py"));
    // Deployment/control does not need the provider's proxy pre-command.
    if request["action"] == "deploy" {
        request["script"] = json!(include_str!("../../../../scripts/remote_wakeup.py"));
    }
    let mut host = host.clone();
    host.pre_command.clear();
    let v = ssh::python(
        &host,
        include_str!("../../../../scripts/remote_wakeup.py"),
        &[serde_json::to_string(&request)?],
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
    pub fn wakeup_call(&mut self, method: &str, p: Value) -> Result<Value> {
        let action = method.split_once('.').map(|(_, a)| a).unwrap_or("");
        let _task_lock = if [
            "save", "deploy", "remove", "delete", "enable", "disable", "run",
        ]
        .contains(&action)
        {
            Some(crate::accounts::lock_tasks(&self.store)?)
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
            let source = source_for(&settings, &task.source_id, task.codex_profile_id.as_deref())?;
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
            pending.state = "deployment-unconfirmed".into();
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
            let tx = rusqlite::Transaction::new_unchecked(
                &self.store.db,
                rusqlite::TransactionBehavior::Immediate,
            )?;
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
        let d = r.deployment.as_mut().context("任务尚未部署")?;
        let result = if let Some(host) = &d.host {
            remote(host, json!({"action":action,"root":d.root,"id":id}))
        } else {
            scheduler::control(Path::new(&d.root), id, action)
        };
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                if action == "remove" {
                    d.state = "pending-removal".into();
                    self.store.save_wakeup(&r)?;
                }
                return Err(e);
            }
        };
        match action {
            "remove" => r.deployment = None,
            "enable" | "disable" => {
                d.enabled = action == "enable";
                d.state = "deployed".into();
            }
            "status" | "history" | "run" => return Ok(value),
            _ => anyhow::bail!("未知任务操作"),
        }
        self.store.save_wakeup(&r)?;
        Ok(public_record(&r))
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
            "当前 CLI 不支持 effort，请选择模型默认值"
        );
        Ok(
            json!({"binary":m.binary,"models":models,"efforts":if source.provider=="codex"||help.contains("--effort"){vec!["low","medium","high","xhigh","max"]}else{vec![]},"timezone":chrono::Local::now().format("%Z %:z").to_string(),"message":if models.is_empty(){"CLI 未提供可核验的模型列表，请填写模型 ID；可用性将在实际运行时验证"}else{"按 1000 输入 + 100 输出 Token 的 API 等价价格推荐；部署后固定模型"}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
