//! Keep Qoder's token counters available for future requests.
//!
//! `QODER_EXPOSE_TOKEN_USAGE=true` must be present before `qodercli` starts,
//! including processes launched by the desktop environment. We update shell
//! startup files plus the macOS launchd / Linux systemd user environment.
use crate::models::{Host, Settings};
#[cfg(unix)]
use anyhow::Result;
#[cfg(unix)]
use std::process::Command;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) const ENV_NAME: &str = "QODER_EXPOSE_TOKEN_USAGE";
const ENV_VALUE: &str = "true";
const START: &str = "# >>> Aieyes Qoder token usage >>>";
const END: &str = "# <<< Aieyes Qoder token usage <<<";
const REMOTE: &str = include_str!("../../../scripts/qoder_env_remote.py");

pub(crate) fn sync(settings: &Settings) -> Vec<String> {
    let mut issues = Vec::new();
    let mut local = false;
    let mut hosts = Vec::<&Host>::new();
    for source in settings
        .sources
        .iter()
        .filter(|source| crate::qoder_family::is_qoder(&source.provider) && source.enabled)
    {
        match &source.host_id {
            None => local = true,
            Some(id) => {
                if let Some(host) = settings
                    .hosts
                    .iter()
                    .find(|host| &host.id == id && host.enabled)
                    && !hosts.iter().any(|seen| seen.id == host.id)
                {
                    hosts.push(host);
                }
            }
        }
    }
    if local {
        issues.extend(local_sync());
    }
    for host in hosts {
        match crate::ssh::python(host, REMOTE, &[]) {
            Ok(value) => {
                if let Some(rows) = value["issues"].as_array() {
                    issues.extend(
                        rows.iter()
                            .filter_map(|row| row.as_str())
                            .map(|row| format!("{}: {row}", host.name)),
                    );
                }
            }
            Err(error) => issues.push(format!("{}: 无法配置 Qoder 环境变量（{error}）", host.name)),
        }
    }
    issues
}

fn local_sync() -> Vec<String> {
    let mut issues = Vec::new();
    let home = crate::store::home();
    let mut changed = Vec::new();
    for path in shell_files(&home) {
        match merge_file(&path) {
            Ok(true) => changed.push(display_path(&path)),
            Ok(false) => {}
            Err(error) => issues.push(format!("{}: {error}", display_path(&path))),
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = macos_launch_agent(&home, ENV_NAME, "qoder")
            .and_then(|_| macos_launch_agent(&home, "QODERCN_EXPOSE_TOKEN_USAGE", "qoder-cn"))
        {
            issues.push(format!("launchd: {error}"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = linux_user_environment(&home) {
            issues.push(format!("systemd 用户环境: {error}"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "linux")))]
    {
        issues.push(
            "当前系统未自动配置图形界面环境；请手动设置 QODER_EXPOSE_TOKEN_USAGE=true。".into(),
        );
    }
    #[cfg(windows)]
    {
        issues.push("Windows 图形界面环境未自动配置；请在系统环境变量中设置 QODER_EXPOSE_TOKEN_USAGE=true。".into());
    }
    if !changed.is_empty() {
        eprintln!(
            "Qoder Token 计数环境已更新：{}；已启动的 qodercli 需要重启后生效。",
            changed.join("、")
        );
    }
    issues
}

fn shell_files(home: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for name in [
        ".profile",
        ".bashrc",
        ".zshrc",
        ".bash_profile",
        ".bash_login",
        ".zprofile",
        ".zshenv",
    ] {
        let path = home.join(name);
        if [".profile", ".bashrc", ".zshrc"].contains(&name) || path.is_file() {
            files.push(path);
        }
    }
    files
}

fn merge_file(path: &Path) -> std::io::Result<bool> {
    // Keep dotfile symlinks intact; write the real file instead of replacing
    // the link with a regular file.
    let path = if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let old = match fs::read_to_string(&path) {
        Ok(old) => old,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let block = format!(
        "{START}\nexport {ENV_NAME}={ENV_VALUE}\nexport QODERCN_EXPOSE_TOKEN_USAGE={ENV_VALUE}\n{END}\n"
    );
    let next = if let Some(start) = old.find(START) {
        if let Some(relative) = old[start + START.len()..].find(END) {
            let end = start + START.len() + relative + END.len();
            let rest = &old[end..];
            let rest = rest
                .strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest);
            let mut next = String::with_capacity(old.len());
            next.push_str(&old[..start]);
            next.push_str(&block);
            next.push_str(rest);
            next
        } else {
            append_block(&old, &block)
        }
    } else {
        append_block(&old, &block)
    };
    if next == old {
        return Ok(false);
    }
    atomic_write(&path, next.as_bytes())?;
    Ok(true)
}

fn append_block(old: &str, block: &str) -> String {
    let mut next = String::with_capacity(old.len() + block.len() + 2);
    next.push_str(old);
    if !old.is_empty() && !old.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(block);
    next
}

fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    file.write_all(content)?;
    file.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o644);
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_launch_agent(home: &Path, env_name: &str, provider: &str) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let dir = home.join("Library/LaunchAgents");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("com.aieyes.{provider}-token-usage.plist"));
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.aieyes.{provider}-token-usage</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/launchctl</string>
    <string>setenv</string>
    <string>{env_name}</string>
    <string>{ENV_VALUE}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/dev/null</string>
  <key>StandardErrorPath</key>
  <string>/dev/null</string>
</dict>
</plist>
"#
    );
    if fs::read_to_string(&path).ok().as_deref() != Some(&plist) {
        atomic_write(&path, plist.as_bytes())?;
    }
    let _ = runtime_command(Command::new("/bin/launchctl").args(["setenv", env_name, ENV_VALUE]));
    let domain = format!("gui/{uid}");
    let _ = runtime_command(Command::new("/bin/launchctl").args([
        "bootout",
        &domain,
        &format!("com.aieyes.{provider}-token-usage"),
    ]));
    let status = runtime_command(Command::new("/bin/launchctl").args([
        "bootstrap",
        &domain,
        &path.to_string_lossy(),
    ]));
    if !status {
        let status = runtime_command(Command::new("/bin/launchctl").args([
            "load",
            "-w",
            &path.to_string_lossy(),
        ]));
        anyhow::ensure!(status, "launchctl 无法加载 Qoder 环境项");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_user_environment(home: &Path) -> Result<()> {
    let dir = home.join(".config/environment.d");
    fs::create_dir_all(&dir)?;
    let path = dir.join("50-aieyes-qoder.conf");
    let content = format!("{ENV_NAME}={ENV_VALUE}\nQODERCN_EXPOSE_TOKEN_USAGE={ENV_VALUE}\n");
    if fs::read_to_string(&path).ok().as_deref() != Some(&content) {
        atomic_write(&path, content.as_bytes())?;
    }
    let _ = runtime_command(Command::new("systemctl").args([
        "--user",
        "set-environment",
        &format!("{ENV_NAME}={ENV_VALUE}"),
        &format!("QODERCN_EXPOSE_TOKEN_USAGE={ENV_VALUE}"),
    ]));
    Ok(())
}

#[cfg(unix)]
fn runtime_command(command: &mut Command) -> bool {
    command
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_shell_content_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".profile");
        let bytes = b"export KEEP=\xff\n";
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            merge_file(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(merge_file(root.path()).is_err());
    }

    #[test]
    fn managed_block_is_idempotent_and_replaces_old_values() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".profile");
        fs::write(&path, "export KEEP=1\n").unwrap();
        assert!(merge_file(&path).unwrap());
        assert!(!merge_file(&path).unwrap());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("export KEEP=1"));
        assert_eq!(content.matches(START).count(), 1);
        fs::write(
            &path,
            format!("export KEEP=1\n{START}\nexport {ENV_NAME}=false\n{END}\n"),
        )
        .unwrap();
        assert!(merge_file(&path).unwrap());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(&format!("export {ENV_NAME}=true")));
        assert!(!content.contains("=false"));
        assert_eq!(content.matches(START).count(), 1);
    }
}
