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
    let _registration = track_child(&mut child)?;
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

/// NDJSON transport with bounded buffering and cleanup on parser/consumer failure.
pub fn run_lines(
    mut cmd: Command,
    input: Vec<u8>,
    timeout: Duration,
    mut receive: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    use std::io::BufRead;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    prepare(&mut cmd);
    let mut child = cmd.spawn().context("启动命令失败")?;
    let _registration = track_child(&mut child)?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(4);
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let errors = std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let keep = n.min(65536usize.saturating_sub(captured.len()));
            captured.extend_from_slice(&buf[..keep]);
        }
        captured
    });
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let result = reader
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut line);
            match result {
                Ok(0) => break,
                Ok(_) if line.len() <= 1024 * 1024 && line.last() == Some(&b'\n') => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Ok(_) => {
                    let _ = tx.send(Err(std::io::Error::other("远程统计批次过大或不完整")));
                    break;
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    let started = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            anyhow::ensure!(!remaining.is_zero(), "查询超时");
            match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(line) => receive(&line?)?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "命令执行失败（{}）", status);
                break;
            }
            anyhow::ensure!(started.elapsed() < timeout, "查询超时");
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    })();
    drop(rx);
    // A successful launcher can still leave descendants holding a pipe open.
    kill(&mut child);
    let _ = writer.join();
    let _ = reader.join();
    let stderr = errors.join().unwrap_or_default();
    result.map_err(|error| {
        if error.to_string().starts_with("命令执行失败") {
            anyhow::anyhow!(classify_error(&String::from_utf8_lossy(&stderr), None))
        } else {
            error
        }
    })
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
pub(crate) fn classify_error(stderr: &str, code: Option<i32>) -> String {
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

#[cfg(all(test, unix))]
mod line_transport_tests {
    use super::*;
    #[test]
    fn lines_are_streamed_and_input_is_not_a_command_argument() {
        let mut command = Command::new("python3");
        command.args(["-c","import sys,json; data=json.load(sys.stdin); [print(json.dumps({'n':n,'text':data['text']}),flush=True) for n in range(20)]"]);
        let input = serde_json::to_vec(&serde_json::json!({"text":"literal '$()` text"})).unwrap();
        let mut values = vec![];
        run_lines(command, input, Duration::from_secs(5), |line| {
            values.push(serde_json::from_slice::<serde_json::Value>(line)?);
            Ok(())
        })
        .unwrap();
        assert_eq!(values.len(), 20);
        assert_eq!(values[19]["n"], 19);
        assert_eq!(values[19]["text"], "literal '$()` text");
    }
    #[test]
    fn errors_and_timeouts_kill_descendants_without_hanging_on_pipes() {
        for (program, limit, consumer_error) in [
            (
                "import sys,time; print('{}',flush=True); time.sleep(30)",
                Duration::from_secs(3),
                true,
            ),
            (
                "import time; time.sleep(30)",
                Duration::from_millis(100),
                false,
            ),
            (
                "print('x'*1048577,flush=True)",
                Duration::from_secs(3),
                false,
            ),
        ] {
            let mut command = Command::new("python3");
            command.args(["-c", program]);
            let started = Instant::now();
            assert!(
                run_lines(command, vec![], limit, |_| {
                    if consumer_error {
                        anyhow::bail!("fixture consumer")
                    };
                    Ok(())
                })
                .is_err()
            );
            assert!(started.elapsed() < Duration::from_secs(3));
        }
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 >&2 & printf '{}\\n'"]);
        let started = Instant::now();
        run_lines(command, vec![], Duration::from_millis(150), |_| Ok(())).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

// Only short-lived queries are registered. Installed wake services and OS URL
// launchers intentionally outlive their caller and are never added here.
use std::sync::atomic::{AtomicU32, Ordering};
static QUERY_CHILDREN: [AtomicU32; 128] = [const { AtomicU32::new(0) }; 128];
pub(crate) struct ChildRegistration(usize);
pub(crate) fn track_child(child: &mut std::process::Child) -> Result<ChildRegistration> {
    for (index, slot) in QUERY_CHILDREN.iter().enumerate() {
        if slot
            .compare_exchange(0, child.id(), Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            return Ok(ChildRegistration(index));
        }
    }
    kill(child);
    anyhow::bail!("后台查询进程数超过上限")
}
impl Drop for ChildRegistration {
    fn drop(&mut self) {
        QUERY_CHILDREN[self.0].store(0, Ordering::SeqCst);
    }
}
/// Called on desktop exit; the Unix variant also uses only signal-safe operations.
pub fn terminate_query_children() {
    for slot in &QUERY_CHILDREN {
        let pid = slot.load(Ordering::SeqCst);
        if pid == 0 {
            continue;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("taskkill");
            command
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            prepare(&mut command);
            let _ = command.status();
        }
    }
}
/// Standalone IPC cores must clean up SSH/CLI children even when the native app
/// terminates them during a blocked call. The embedded Tauri engine uses its exit hook.
pub fn install_query_shutdown_handler() {
    #[cfg(unix)]
    {
        extern "C" fn terminate(signal: libc::c_int) {
            terminate_query_children();
            unsafe {
                libc::_exit(128 + signal);
            }
        }
        unsafe {
            libc::signal(libc::SIGTERM, terminate as *const () as libc::sighandler_t);
            libc::signal(libc::SIGINT, terminate as *const () as libc::sighandler_t);
        }
    }
}
