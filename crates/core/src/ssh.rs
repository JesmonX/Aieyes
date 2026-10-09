use crate::{
    models::*,
    process::{self, quote},
    store::expand,
};
use anyhow::Result;
use std::{process::Command, time::Duration};

pub fn command(host: &Host, remote: &str) -> Result<Command> {
    command_inner(host, remote, false)
}
pub fn account_command(host: &Host, remote: &str) -> Result<Command> {
    command_inner(host, remote, true)
}
fn command_inner(host: &Host, remote: &str, include_account_command: bool) -> Result<Command> {
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
        if host.auth_mode == "password" {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        },
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
    ]);
    if !host.username.is_empty() {
        cmd.arg("-l").arg(&host.username);
    }
    if host.auth_mode == "password" {
        // Validate accessibility before spawning so a missing/locked secret is a
        // useful error instead of an opaque SSH authentication failure.
        crate::credentials::read(&host.password_ref)?;
        cmd.args([
            "-o",
            "PreferredAuthentications=password",
            "-o",
            "PubkeyAuthentication=no",
            "-o",
            "NumberOfPasswordPrompts=1",
            "-o",
            "StrictHostKeyChecking=yes",
        ]);
        cmd.env("SSH_ASKPASS", crate::credentials::helper()?)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("AIEYES_SSH_ASKPASS_REF", &host.password_ref);
    }
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
        let identity = format!(
            "{}:{:?}:{}:{}:{}:{}",
            host.target,
            host.port,
            host.identity_file,
            host.auth_mode,
            host.username,
            host.password_ref
        );
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
    let shell = if host.shell.is_empty() {
        DEFAULT_SHELL
    } else {
        host.shell.as_str()
    };
    // A login POSIX shell re-reads /etc/profile and profile.d scripts before running
    // the command. On Debian-style hosts `/bin/sh` is dash and those files are often
    // bash-only, so the shell exits with status 2 before the script starts. The
    // fallback PATH keeps user-installed CLIs reachable without the profiles.
    let login = login_shell(shell);
    let mut script = String::from("set -e\n");
    if !login {
        script.push_str(PATH_FALLBACK);
    }
    if include_account_command && !host.pre_command.trim().is_empty() {
        script.push_str("{\n");
        script.push_str(&host.pre_command);
        script.push_str("\n} </dev/null >&2\n");
    }
    script.push_str(remote);
    let flag = if login { "-lc" } else { "-c" };
    cmd.arg("--")
        .arg(&host.target)
        .arg(format!("{} {} {}", quote(shell), flag, quote(&script)));
    Ok(cmd)
}

/// Shell used when a host does not configure one.
pub const DEFAULT_SHELL: &str = "/bin/bash";

/// Keeps user-installed CLIs reachable without a login shell; mirrors the local
/// CLI resolver so remote agents installed in the home directory keep working.
pub const PATH_FALLBACK: &str = "PATH=\"$HOME/.local/bin:$HOME/bin:$HOME/.codex/packages/standalone/current/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin${PATH:+:$PATH}\"\nexport PATH\n";

/// Only explicitly configured non-POSIX shells are invoked as login shells: a
/// login `/bin/sh` or `/bin/dash` re-reads profiles that may not be POSIX.
fn login_shell(shell: &str) -> bool {
    !matches!(shell.rsplit('/').next().unwrap_or(shell), "sh" | "dash")
}
pub fn python(host: &Host, script: &str, args: &[String]) -> Result<serde_json::Value> {
    python_inner(host, script, args, false)
}
pub fn account_python(host: &Host, script: &str, args: &[String]) -> Result<serde_json::Value> {
    python_inner(host, script, args, true)
}
fn python_inner(
    host: &Host,
    script: &str,
    args: &[String],
    account: bool,
) -> Result<serde_json::Value> {
    let remote = format!(
        "exec python3 - {}",
        args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    );
    let output = process::run(
        command_inner(host, &remote, account)?,
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
