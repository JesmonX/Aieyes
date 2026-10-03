use crate::{
    models::*,
    process::{self, quote},
    store::expand,
};
use anyhow::Result;
use std::{process::Command, time::Duration};

pub fn command(host: &Host, remote: &str) -> Result<Command> {
    anyhow::ensure!(
        !host.target.is_empty()
            && !host.target.starts_with('-')
            && !host.target.chars().any(char::is_whitespace),
        "SSH 主机格式不正确"
    );
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
    ]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let uid = unsafe { libc::geteuid() };
        let root = std::env::temp_dir().join(format!("aieyes-ssh-{uid}"));
        std::fs::create_dir_all(&root)?;
        let metadata = std::fs::symlink_metadata(&root)?;
        anyhow::ensure!(
            metadata.is_dir() && metadata.uid() == uid,
            "SSH 连接缓存目录不可用"
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let identity = format!("{}:{:?}:{}", host.target, host.port, host.identity_file);
        let path = root.join(&hash(&identity)[..16]);
        cmd.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=60", "-o"]);
        cmd.arg(format!("ControlPath={}", path.display()));
    }
    if let Some(port) = host.port {
        cmd.args(["-p", &port.to_string()]);
    }
    if !host.identity_file.is_empty() {
        cmd.arg("-i").arg(expand(&host.identity_file));
    }
    let mut script = String::from("set -e\n");
    if !host.pre_command.trim().is_empty() {
        script.push_str("{\n");
        script.push_str(&host.pre_command);
        script.push_str("\n} </dev/null >&2\n");
    }
    script.push_str(remote);
    let shell = if host.shell.is_empty() {
        "/bin/sh"
    } else {
        &host.shell
    };
    cmd.arg("--")
        .arg(&host.target)
        .arg(format!("{} -lc {}", quote(shell), quote(&script)));
    Ok(cmd)
}
pub fn python(host: &Host, script: &str, args: &[String]) -> Result<serde_json::Value> {
    let remote = format!(
        "exec python3 - {}",
        args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    );
    let output = process::run(
        command(host, &remote)?,
        script.as_bytes().to_vec(),
        Duration::from_secs(45),
    )?;
    // A pre-command may print a banner. Only the final JSON line is part of the protocol.
    let line = output
        .split(|b| *b == b'\n')
        .rev()
        .find(|line| !line.is_empty())
        .ok_or_else(|| anyhow::anyhow!("远程查询没有返回数据"))?;
    Ok(serde_json::from_slice(line)?)
}

pub const HISTORY_SCRIPT: &str = include_str!("../../../scripts/remote_history.py");
pub const METRICS_SCRIPT: &str = include_str!("../../../scripts/linux_metrics.py");
