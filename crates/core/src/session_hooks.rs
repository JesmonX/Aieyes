//! Idempotent, owned observer hooks. They never return permission decisions to Claude.
use crate::{
    file_lock::FileLock,
    models::{Settings, Source},
    session_events,
    store::expand,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MARKER: &str = "--aieyes-session-hook";
pub const REMOTE: &str = include_str!("../../../scripts/session_remote.py");
const CLAUDE: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "StopFailure",
    "SessionEnd",
    "Elicitation",
    "ElicitationResult",
];
// AGY owns all permission UI. No PreToolUse observer is installed.
const AGY: &[&str] = &[
    "SessionStart",
    "PreInvocation",
    "PostToolUse",
    "PostInvocation",
    "Stop",
];

#[derive(Clone)]
pub struct Target {
    pub home: PathBuf,
    pub config: PathBuf,
    pub nested: bool,
}
pub fn target(source: &Source) -> Target {
    let raw = if source.path.trim().is_empty() {
        crate::contracts::provider(&source.provider)
            .map(|p| p.default_path)
            .unwrap_or("")
    } else {
        &source.path
    };
    let path = if source.host_id.is_some() {
        PathBuf::from(raw)
    } else {
        expand(raw)
    };
    if source.provider == "antigravity"
        && path
            .file_name()
            .is_some_and(|n| n == "antigravity-cli" || n == "antigravity")
    {
        let home = path.parent().unwrap_or(&path).join("config");
        Target {
            config: home.join("hooks.json"),
            home,
            nested: false,
        }
    } else {
        Target {
            config: path.join("settings.json"),
            home: path,
            nested: true,
        }
    }
}
pub fn event_root(source: &Source) -> PathBuf {
    target(source).home.join(".aieyes/session-events")
}
pub fn key(source: &Source) -> String {
    let home = if source.host_id.is_some() {
        target(source)
            .home
            .to_string_lossy()
            .trim_end_matches('/')
            .to_string()
    } else {
        fs::canonicalize(target(source).home)
            .unwrap_or_else(|_| target(source).home)
            .to_string_lossy()
            .into_owned()
    };
    crate::models::hash(&format!(
        "{}:{}:{home}",
        source.host_id.as_deref().unwrap_or("local"),
        source.provider
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub source_id: String,
    pub provider: String,
    pub machine_id: String,
    pub config_path: String,
    pub state: String,
    pub error: Option<String>,
}
fn owned(value: &Value) -> bool {
    value["command"]
        .as_str()
        .is_some_and(|c| c.contains(MARKER))
}
fn configured_state(config: &Value, provider: &str, nested: bool) -> &'static str {
    if config["disableAllHooks"] == true {
        return "disabled";
    }
    let hooks = if nested { &config["hooks"] } else { config };
    let events = if provider == "antigravity" {
        &hooks["aieyes"]
    } else {
        hooks
    };
    if events["enabled"] == false {
        return "disabled";
    }
    let names = if provider == "claude" { CLAUDE } else { AGY };
    for event in names {
        let Some(rows) = events[*event].as_array() else {
            return "missing";
        };
        let Some(row) = rows
            .iter()
            .find(|r| owned(r) || r["hooks"].as_array().is_some_and(|h| h.iter().any(owned)))
        else {
            return "missing";
        };
        if row["enabled"] == false
            || row["hooks"]
                .as_array()
                .is_some_and(|h| h.iter().any(|v| owned(v) && v["enabled"] == false))
        {
            return "disabled";
        }
    }
    "installed"
}
fn disabled(value: &Value) -> bool {
    if value["enabled"] == false || value["disableAllHooks"] == true {
        return true;
    }
    match value {
        Value::Array(a) => a.iter().any(disabled),
        Value::Object(o) => o.values().any(|v| match v {
            Value::Object(_) | Value::Array(_) => disabled(v),
            _ => false,
        }),
        _ => false,
    }
}
/// Edit only entries carrying our marker, preserving unrelated matchers, handlers and fields.
pub fn merge(
    config: &mut Value,
    provider: &str,
    nested: bool,
    commands: &[(String, String)],
    remove: bool,
    force: bool,
) -> Result<bool> {
    ensure!(config.is_object(), "Hook 配置必须是 JSON 对象");
    if !force && !remove && config["disableAllHooks"] == true {
        return Ok(false);
    }
    let hooks = if nested {
        if config.get("hooks").is_none() {
            config["hooks"] = json!({});
        }
        config.get_mut("hooks").unwrap()
    } else {
        config
    };
    ensure!(hooks.is_object(), "hooks 配置格式无效，原配置已保留");
    let events = if provider == "antigravity" {
        if !force && !remove && hooks.get("aieyes").is_some_and(disabled) {
            return Ok(false);
        }
        if hooks.get("aieyes").is_none() {
            if remove {
                return Ok(true);
            }
            hooks["aieyes"] = json!({});
        }
        hooks.get_mut("aieyes").unwrap()
    } else {
        hooks
    };
    ensure!(events.is_object(), "Aieyes hooks 配置格式无效");
    if provider == "antigravity" && force {
        events.as_object_mut().unwrap().remove("enabled");
    }
    for (event, command) in commands {
        if events.get(event).is_none() {
            if remove {
                continue;
            }
            events[event] = json!([]);
        }
        let rows = events[event]
            .as_array_mut()
            .context("Hook 事件配置必须是数组")?;
        let own_disabled = rows.iter().any(|v| {
            (owned(v) || v["hooks"].as_array().is_some_and(|h| h.iter().any(owned))) && disabled(v)
        });
        if own_disabled && !force && !remove {
            continue;
        }
        rows.retain_mut(|row| {
            if owned(row) {
                return false;
            }
            if let Some(children) = row["hooks"].as_array_mut() {
                let had = children.iter().any(owned);
                children.retain(|h| !owned(h));
                return !had || !children.is_empty();
            }
            true
        });
        if !remove {
            let handler = json!({"type":"command","command":command,"timeout":2});
            if provider == "antigravity" && event != "PostToolUse" {
                rows.push(handler);
            } else {
                rows.push(json!({"matcher":"*","hooks":[handler]}));
            }
        }
    }
    Ok(true)
}
fn read_config(path: &Path) -> Result<Value> {
    match fs::read(path) {
        Ok(bytes) => {
            ensure!(bytes.len() < 4 * 1024 * 1024, "Hook 配置过大");
            serde_json::from_slice(&bytes).context("Hook 配置 JSON 无效，原文件已保留")
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("aieyes-{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&temp)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&temp, meta.permissions())?;
        }
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn command(helper: &Path, provider: &str, event: &str, home: &Path) -> Result<String> {
    let values = [
        helper.to_string_lossy().to_string(),
        MARKER.into(),
        provider.into(),
        event.into(),
        home.to_string_lossy().to_string(),
    ];
    if cfg!(windows) {
        ensure!(
            values.iter().all(|s| !s.contains(['"', '%', '\n', '\r'])),
            "Hook 路径包含不支持的 shell 字符"
        );
        Ok(values
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(" "))
    } else {
        Ok(values
            .iter()
            .map(|s| crate::process::quote(s))
            .collect::<Vec<_>>()
            .join(" "))
    }
}
pub fn local(source: &Source, action: &str) -> HookStatus {
    let t = target(source);
    let mut status = HookStatus {
        source_id: source.id.clone(),
        provider: source.provider.clone(),
        machine_id: "local".into(),
        config_path: t.config.to_string_lossy().into_owned(),
        state: "missing".into(),
        error: None,
    };
    let result = (|| -> Result<String> {
        ensure!(
            matches!(source.provider.as_str(), "claude" | "antigravity"),
            "此 Agent 不需要 hooks"
        );
        let state_dir = t.home.join(".aieyes");
        let disabled_path = state_dir.join("session-hooks-disabled");
        if disabled_path.exists() && action != "repair" && action != "remove" {
            return Ok("disabled".into());
        }
        let original = read_config(&t.config)?;
        if action == "status" {
            return Ok(configured_state(&original, &source.provider, t.nested).into());
        }
        // Never create a phantom provider home just because a data source is configured.
        if !t.home.exists() {
            let source_home = expand(&source.path);
            ensure!(source_home.is_dir(), "Agent 数据目录尚不存在");
        }
        fs::create_dir_all(&state_dir)?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(state_dir.join("session-hooks.lock"))?;
        let _lock = FileLock::try_exclusive(lock).context("另一项 Hook 配置操作正在进行")?;
        ensure!(
            !fs::symlink_metadata(&t.config).is_ok_and(|m| m.file_type().is_symlink()),
            "Hook 配置是符号链接，自动写入已跳过"
        );
        let original = read_config(&t.config)?;
        let mut config = original.clone();
        let exe = crate::credentials::helper()?;
        let metadata = fs::metadata(&exe)?;
        let build = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
        let helper = state_dir.join(format!(
            "aieyes-session-hook-{}-{build}-{}{}",
            env!("CARGO_PKG_VERSION"),
            metadata.len(),
            std::env::consts::EXE_SUFFIX
        ));
        let names = if source.provider == "claude" {
            CLAUDE
        } else {
            AGY
        };
        let commands = names
            .iter()
            .map(|event| {
                Ok((
                    event.to_string(),
                    command(&helper, &source.provider, event, &t.home)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if !merge(
            &mut config,
            &source.provider,
            t.nested,
            &commands,
            action == "remove",
            action == "repair",
        )? {
            return Ok("disabled".into());
        }
        if action != "remove" {
            if !helper.exists() {
                let pending = helper.with_extension(format!("{}.tmp", std::process::id()));
                fs::copy(exe, &pending)?;
                fs::rename(pending, &helper)?;
            }
            fs::create_dir_all(state_dir.join("session-events"))?;
        }
        if config != original {
            atomic(&t.config, &serde_json::to_vec_pretty(&config)?)?;
        }
        if action == "remove" {
            fs::write(disabled_path, b"disabled\n")?;
            return Ok("disabled".into());
        }
        if action == "repair" {
            let _ = fs::remove_file(disabled_path);
        }
        Ok(configured_state(&config, &source.provider, t.nested).into())
    })();
    match result {
        Ok(state) => status.state = state,
        Err(error) => {
            status.state = "error".into();
            status.error = Some(format!("{error:#}"));
        }
    }
    status
}
pub fn call(settings: &Settings, method: &str, params: &Value) -> Result<Value> {
    let action = method.rsplit('.').next().unwrap_or("status");
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for source in settings.sources.iter().filter(|s| {
        matches!(s.provider.as_str(), "claude" | "antigravity")
            && params["sourceId"].as_str().is_none_or(|id| s.id == id)
    }) {
        if !seen.insert(key(source)) {
            continue;
        }
        if source.machine_binding == crate::models::MachineBinding::Unbound
            || source
                .host_id
                .as_ref()
                .is_some_and(|id| !settings.hosts.iter().any(|h| &h.id == id))
        {
            rows.push(json!({"sourceId":source.id,"provider":source.provider,"machineId":source.host_id,"configPath":source.path,"state":"unbound","error":"机器未绑定，请先重新绑定目录"}));
            continue;
        }
        if let Some(id) = &source.host_id {
            let host = settings
                .hosts
                .iter()
                .find(|h| &h.id == id)
                .context("服务器已移除")?;
            let response = crate::ssh::session_request(
                host,
                json!({"action":action,"sources":[{"id":source.id,"provider":source.provider,"path":source.path,"hostId":source.host_id}]}),
            );
            match response {
                Ok(response) => rows.extend(response["hooks"].as_array().cloned().unwrap_or_default()),
                Err(error) => rows.push(json!({"sourceId":source.id,"provider":source.provider,"machineId":id,"configPath":source.path,"state":"error","error":format!("{error:#}")})),
            }
        } else {
            rows.push(serde_json::to_value(local(source, action))?);
        }
    }
    Ok(Value::Array(rows))
}
fn ancestry(provider: &str) -> (u32, u64) {
    let mut system = sysinfo::System::new();
    let mut pid = sysinfo::Pid::from_u32(std::process::id());
    for _ in 0..8 {
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        let Some(process) = system.process(pid) else {
            break;
        };
        let name = process.name().to_string_lossy().to_lowercase();
        if (provider == "claude" && name.starts_with("claude"))
            || (provider == "antigravity" && (name == "agy" || name == "agy.exe"))
        {
            return (pid.as_u32(), process.start_time());
        }
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    (0, 0)
}
/// Called before any GUI, SQLite, keychain or updater initialization, including from a copied helper.
pub fn run_if_requested() -> bool {
    let args: Vec<_> = std::env::args().collect();
    let Some(index) = args.iter().position(|s| s == MARKER) else {
        return false;
    };
    let provider = args.get(index + 1).cloned().unwrap_or_default();
    let kind = args.get(index + 2).cloned().unwrap_or_default();
    let home = args.get(index + 3).map(PathBuf::from);
    let output = if provider == "antigravity" {
        if kind == "Stop" {
            "{\"decision\":\"allow\"}"
        } else {
            "{}"
        }
    } else {
        ""
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, "hook input too large");
            let input: Value = serde_json::from_slice(&bytes)?;
            let mut event = session_events::hook(&provider, &kind, &input, at);
            ensure!(!event.session.is_empty(), "missing session");
            (event.pid, event.process_start) = ancestry(&provider);
            let root = home.context("missing home")?.join(".aieyes");
            if root.join("session-hooks-disabled").exists() {
                return Ok(());
            }
            let path = root
                .join("session-events")
                .join(format!("{}.jsonl", crate::models::hash(&event.session)));
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path.with_extension("lock"))?;
            let _lock = FileLock::try_exclusive(lock)?;
            if fs::metadata(&path).is_ok_and(|m| m.len() > 4 * 1024 * 1024) {
                let _ = fs::remove_file(&path);
            }
            let mut options = fs::OpenOptions::new();
            options.append(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            let mut row = serde_json::to_vec(&event)?;
            row.push(b'\n');
            file.write_all(&row)?;
            Ok(())
        })();
        let _ = tx.send(result.is_ok());
    });
    let _ = rx.recv_timeout(Duration::from_millis(700));
    if !output.is_empty() {
        println!("{output}");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_hooks_are_idempotent_preserve_config_and_remove_cleanly() {
        let mut cfg = json!({"env":{"KEEP":"yes"},"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other"}]}]}});
        let commands = vec![("PreToolUse".into(), format!("helper {MARKER}"))];
        merge(&mut cfg, "claude", true, &commands, false, false).unwrap();
        let installed = cfg.clone();
        merge(&mut cfg, "claude", true, &commands, false, false).unwrap();
        assert_eq!(cfg, installed);
        merge(&mut cfg, "claude", true, &commands, true, false).unwrap();
        assert_eq!(cfg["env"]["KEEP"], "yes");
        assert_eq!(
            cfg["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "other"
        );
        assert!(!cfg.to_string().contains(MARKER));
    }
    #[test]
    fn global_disable_and_agy_disabled_group_are_honored() {
        let mut cfg = json!({"disableAllHooks":true});
        let original = cfg.clone();
        assert!(!merge(&mut cfg, "claude", true, &[], false, false).unwrap());
        assert_eq!(cfg, original);
        let mut cfg = json!({"aieyes":{"enabled":false},"other":{"enabled":false}});
        let original = cfg.clone();
        assert!(!merge(&mut cfg, "antigravity", false, &[], false, false).unwrap());
        assert_eq!(cfg, original);
    }
}
