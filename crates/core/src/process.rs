use anyhow::{Context, Result};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn run(mut cmd: Command, input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    prepare(&mut cmd);
    let mut child = cmd.spawn().context("启动命令失败")?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (error_tx, error_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut captured = Vec::new();
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let keep = n.min(65536usize.saturating_sub(captured.len()));
            captured.extend_from_slice(&buffer[..keep]);
        }
        let _ = error_tx.send(captured);
    });
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
        drop(stdin);
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map(|_| data);
        let _ = tx.send(result);
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            kill(&mut child);
            anyhow::bail!("查询超时");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let output = match rx.recv_timeout(timeout.saturating_sub(start.elapsed())) {
        Ok(result) => result?,
        Err(_) => {
            kill(&mut child);
            anyhow::bail!("查询超时");
        }
    };
    if !status.success() {
        let error = error_rx
            .recv_timeout(Duration::from_millis(100))
            .unwrap_or_default();
        anyhow::bail!(classify_error(
            &String::from_utf8_lossy(&error),
            status.code()
        ));
    }
    anyhow::ensure!(output.len() <= 64 * 1024 * 1024, "查询结果超过 64 MB");
    Ok(output)
}
/// Every background child must avoid opening a console window on Windows.
pub fn prepare(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
}

/// Resolve native and npm CLI launchers without composing a shell command.
pub fn cli_command(program: &str) -> Command {
    let path = crate::store::expand(program);
    #[cfg(windows)]
    let path = resolve_windows_cli(&path);
    Command::new(path)
}

#[cfg(windows)]
fn resolve_windows_cli(program: &std::path::Path) -> std::path::PathBuf {
    if program.is_absolute() || program.components().count() > 1 {
        return windows_executable(program).unwrap_or_else(|| program.to_owned());
    }
    let mut directories: Vec<_> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        directories.push(std::path::PathBuf::from(appdata).join("npm"));
    }
    directories
        .into_iter()
        .find_map(|dir| windows_executable(&dir.join(program)))
        .unwrap_or_else(|| program.to_owned())
}

#[cfg(any(windows, test))]
fn windows_executable(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.extension().is_some() {
        return path.is_file().then(|| path.to_owned());
    }
    // npm also writes an extensionless POSIX shell script; never prefer it to the .cmd launcher.
    ["exe", "com", "cmd", "bat"]
        .iter()
        .map(|ext| path.with_extension(ext))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod executable_tests {
    use super::*;
    #[test]
    fn windows_prefers_native_or_cmd_over_npm_posix_script() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("codex");
        std::fs::write(&binary, "#!/bin/sh\n").unwrap();
        assert!(windows_executable(&binary).is_none());
        std::fs::write(binary.with_extension("cmd"), "@echo off\r\n").unwrap();
        assert_eq!(
            windows_executable(&binary),
            Some(binary.with_extension("cmd"))
        );
        std::fs::write(binary.with_extension("exe"), "fixture").unwrap();
        assert_eq!(
            windows_executable(&binary),
            Some(binary.with_extension("exe"))
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn npm_launcher_in_a_path_with_spaces_and_unicode() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("测试 tools");
        std::fs::create_dir(&dir).unwrap();
        let script = dir.join("codex.cmd");
        std::fs::write(dir.join("codex"), "#!/bin/sh\n").unwrap();
        std::fs::write(&script, "@echo off\r\necho %~1\r\n").unwrap();
        let mut cmd = cli_command(&dir.join("codex").to_string_lossy());
        assert_eq!(cmd.get_program(), script.as_os_str());
        cmd.arg("hello world");
        assert_eq!(
            String::from_utf8(run(cmd, vec![], Duration::from_secs(5)).unwrap())
                .unwrap()
                .trim(),
            "hello world"
        );
    }
}
fn classify_error(stderr: &str, code: Option<i32>) -> String {
    for (pattern, message) in [
        ("Operation not permitted", "网络连接被系统拒绝"),
        ("Could not resolve hostname", "无法解析 SSH 主机"),
        ("Host key verification failed", "SSH 主机指纹尚未确认"),
        ("Permission denied (", "SSH 认证失败"),
        ("Connection refused", "服务器拒绝连接"),
        ("No route to host", "无法到达服务器"),
        ("Connection timed out", "服务器连接超时"),
        ("python3: command not found", "远程主机未找到 Python 3"),
    ] {
        if stderr.contains(pattern) {
            return message.into();
        }
    }
    format!(
        "命令执行失败（{}）",
        code.map(|n| n.to_string())
            .unwrap_or_else(|| "已停止".into())
    )
}
pub fn kill(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    if child.try_wait().ok().flatten().is_none() {
        // npm launchers are cmd.exe parents; terminating only that parent leaves Node running.
        let mut command = Command::new("taskkill");
        command.args(["/F", "/T", "/PID", &child.id().to_string()]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        prepare(&mut command);
        let _ = command.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn quotes_keep_shell_code_literal() {
        let s = "a' b\n$(printf injected)`echo no`";
        let mut c = Command::new("/bin/sh");
        c.args(["-c", &format!("printf %s {}", quote(s))]);
        assert_eq!(
            run(c, vec![], Duration::from_secs(2)).unwrap(),
            s.as_bytes()
        );
    }
    #[test]
    fn timeout_stops_descendants() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 3 & wait"]);
        let start = Instant::now();
        assert!(run(command, vec![], Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn diagnostics_do_not_echo_credentials() {
        let message = classify_error(
            "private-token=example-secret\nPermission denied (publickey)",
            Some(255),
        );
        assert_eq!(message, "SSH 认证失败");
        assert!(!message.contains("example-secret"));
    }
}
