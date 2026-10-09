//! Conservative process detection. Unknown ownership blocks mutation, not guessed termination.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, time::Duration};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub pid: u32,
    pub fingerprint: String,
    pub name: String,
    pub can_close: bool,
}
#[cfg(unix)]
pub fn list(root: &Path) -> Result<Vec<Running>> {
    let mut cmd = Command::new("ps");
    cmd.args(["-axo", "pid=,uid=,lstart=,comm="]);
    let output = crate::process::run(cmd, vec![], Duration::from_secs(10))?;
    let mut result = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 8 {
            continue;
        }
        let Ok(pid) = fields[0].parse::<u32>() else {
            continue;
        };
        if pid == std::process::id()
            || fields[1].parse::<u32>().ok() != Some(unsafe { libc::geteuid() })
        {
            continue;
        }
        let executable = fields[7..].join(" ");
        let name = Path::new(&executable)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_lowercase();
        if name != "codex" && !name.starts_with("codex-") {
            continue;
        }
        // Linux provides the exact home without dumping the full environment.
        #[cfg(target_os = "linux")]
        {
            let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
            let env = std::fs::read(proc.join("environ")).unwrap_or_default();
            let home = env
                .split(|b| *b == 0)
                .find_map(|v| v.strip_prefix(b"CODEX_HOME="))
                .map(|v| String::from_utf8_lossy(v).into_owned());
            let actual = home
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| crate::store::home().join(".codex"));
            if actual.starts_with(root.join(".aieyes")) {
                continue;
            }
            if !env.is_empty() && actual != root {
                continue;
            }
            result.push(Running {
                pid,
                fingerprint: crate::models::hash(line),
                name: "Codex".into(),
                can_close: !env.is_empty() && actual == root,
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            let mut inspect = Command::new("/usr/sbin/lsof");
            inspect.args(["-a", "-p", &pid.to_string(), "-Fn"]);
            let paths =
                crate::process::run(inspect, vec![], Duration::from_secs(5)).unwrap_or_default();
            let paths = String::from_utf8_lossy(&paths);
            let root_text = format!("n{}/", root.display());
            let managed = format!("n{}/.aieyes/", root.display());
            if paths.lines().any(|s| s.starts_with(&managed)) {
                continue;
            }
            let matches = paths.lines().any(|s| s.starts_with(&root_text));
            result.push(Running {
                pid,
                fingerprint: crate::models::hash(line),
                name: "Codex".into(),
                can_close: matches && name == "codex",
            });
        }
    }
    result.sort_by_key(|p| p.pid);
    Ok(result)
}
#[cfg(windows)]
pub fn list(_root: &Path) -> Result<Vec<Running>> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", "@(Get-CimInstance Win32_Process -Filter \"Name = 'codex.exe'\" | Select-Object ProcessId,CreationDate) | ConvertTo-Json -Compress"]);
    let bytes = crate::process::run(cmd, vec![], Duration::from_secs(10))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!([]));
    let rows = value.as_array().cloned().unwrap_or_else(|| {
        if value.is_object() {
            vec![value]
        } else {
            vec![]
        }
    });
    Ok(rows
        .iter()
        .filter_map(|v| {
            v["ProcessId"].as_u64().map(|pid| Running {
                pid: pid as u32,
                fingerprint: crate::models::hash(&v.to_string()),
                name: "Codex（请在原窗口关闭）".into(),
                can_close: false,
            })
        })
        .collect())
}
pub fn close(root: &Path, expected: &[Running]) -> Result<()> {
    let actual = list(root)?;
    ensure!(actual == expected, "运行进程已变化，请重新检查后切换");
    ensure!(
        actual.iter().all(|p| p.can_close),
        "无法确认部分进程所属目录，请手动关闭 Codex 后重试"
    );
    for p in actual {
        #[cfg(unix)]
        {
            ensure!(
                unsafe { libc::kill(p.pid as i32, libc::SIGTERM) } == 0,
                "无法请求 Codex 退出，请手动关闭"
            );
        }
        #[cfg(windows)]
        {
            let _ = p;
            anyhow::bail!("请手动关闭 Codex 后重试");
        }
    }
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if list(root)?.is_empty() {
            return Ok(());
        }
        ensure!(
            std::time::Instant::now() < until,
            "Codex 未退出或已自动重启，账号未切换"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}
