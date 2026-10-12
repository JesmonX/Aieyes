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
        if let Err(error) = crate::operations::check() {
            kill(&mut child);
            return Err(error);
        }
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
/// The timeout limits waiting for the next line or process exit, not the total
/// transfer or time spent committing a batch in the consumer.
pub fn run_lines(
    mut cmd: Command,
    input: Vec<u8>,
    idle_timeout: Duration,
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
    let mut waiting_since = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            crate::operations::check()?;
            let remaining = idle_timeout.saturating_sub(waiting_since.elapsed());
            anyhow::ensure!(!remaining.is_zero(), "查询超时");
            match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(line) => {
                    receive(&line?)?;
                    waiting_since = Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "命令执行失败（{}）", status);
                break;
            }
            anyhow::ensure!(waiting_since.elapsed() < idle_timeout, "查询超时");
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

/// Run initialization in the CLI's shell, keeping its output out of the CLI protocol.
pub(crate) fn cli_command_with_pre_command(
    program: &str,
    args: &[&str],
    pre_command: &str,
    shell: &str,
) -> Command {
    if pre_command.trim().is_empty() {
        let mut command = cli_command(program);
        command.args(args);
        return command;
    }
    let path = crate::store::expand(program);
    let program = path.to_string_lossy();
    if cfg!(windows) {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
        // Resolve bare names after initialization so a changed PATH takes effect.
        // Application excludes npm's PowerShell shim and selects its native/cmd launcher.
        let invocation = if path.is_absolute() || path.components().count() > 1 {
            #[cfg(windows)]
            let path = resolve_windows_cli(&path);
            quote(&path.to_string_lossy())
        } else {
            format!(
                "(Get-Command -Name {} -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source",
                quote(&program)
            )
        };
        let script = format!(
            "$ErrorActionPreference='Stop'\n$LASTEXITCODE=0\n. {{\n{pre_command}\n}} 3>&1 4>&1 5>&1 6>&1 | ForEach-Object {{ [Console]::Error.WriteLine([string]$_) }}\nif ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}\n& {invocation} {}\nexit $LASTEXITCODE",
            args.iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        command
    } else {
        let script = format!(
            "set -e\n{}{{\n{pre_command}\n}} </dev/null >&2\nexec {} {}",
            crate::ssh::PATH_FALLBACK,
            quote(&program),
            args.iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut command = Command::new(if shell.trim().is_empty() {
            crate::ssh::DEFAULT_SHELL
        } else {
            shell
        });
        command.args(["-c", &script]);
        command
    }
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
    fn initialization_preserves_home_paths_environment_arguments_and_stdin() {
        if std::env::var_os("AIEYES_CLI_COMMAND_FIXTURE").is_none() {
            let root = tempfile::tempdir().unwrap();
            for name in ["home", "profile"] {
                std::fs::create_dir(root.path().join(name)).unwrap();
            }
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process::executable_tests::initialization_preserves_home_paths_environment_arguments_and_stdin",
                    "--nocapture",
                ])
                .env("AIEYES_CLI_COMMAND_FIXTURE", "1")
                .env("HOME", root.path().join("home"))
                .env("USERPROFILE", root.path().join("profile"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let directory = crate::store::home().join("测试 tools");
        std::fs::create_dir(&directory).unwrap();
        let name = if cfg!(windows) {
            "fixture-cli.cmd"
        } else {
            "fixture-cli"
        };
        let script = directory.join(name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$AIEYES_CLI_INIT\" \"$1\"\nIFS= read -r value\nprintf '%s\\n' \"$value\"\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        #[cfg(windows)]
        {
            // Match npm launchers: the batch file forwards stdin to a native
            // CLI process. `set /p` is a console-oriented reader and can hang
            // on the redirected, hidden-console pipe in Windows CI.
            std::fs::write(
                directory.join("fixture-cli.py"),
                "import os, sys\nprint(os.environ['AIEYES_CLI_INIT'])\nprint(sys.argv[1])\nprint(sys.stdin.buffer.readline().decode('utf-8').rstrip('\\r\\n'))\n",
            )
            .unwrap();
            std::fs::write(
                &script,
                "@echo off\r\npython \"%~dp0fixture-cli.py\" %*\r\nexit /b %errorlevel%\r\n",
            )
            .unwrap();
        }

        let initialization = if cfg!(windows) {
            "Write-Output 'setup banner'; Write-Host 'host banner'; $env:AIEYES_CLI_INIT='ready'"
        } else {
            "printf 'setup banner\\n'; read ignored || true; export AIEYES_CLI_INIT=ready"
        };
        let path = format!("~/测试 tools/{name}");
        for pre_command in ["", initialization] {
            let mut command = cli_command_with_pre_command(
                &path,
                &["argument with spaces"],
                pre_command,
                "/bin/sh",
            );
            command.env("AIEYES_CLI_INIT", "direct");
            let output = run(
                command,
                b"protocol-input\n".to_vec(),
                Duration::from_secs(20),
            )
            .unwrap_or_else(|error| {
                panic!("CLI fixture initialization {pre_command:?}: {error:#}")
            });
            assert_eq!(
                String::from_utf8(output)
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                [
                    if pre_command.is_empty() {
                        "direct"
                    } else {
                        "ready"
                    },
                    "argument with spaces",
                    "protocol-input"
                ]
            );
        }
        let path_setup = if cfg!(windows) {
            format!(
                "$env:PATH='{};'+$env:PATH; {initialization}",
                directory.to_string_lossy().replace('\'', "''")
            )
        } else {
            format!(
                "export PATH={}:\"$PATH\"; {initialization}",
                quote(&directory.to_string_lossy())
            )
        };
        let output = run(
            cli_command_with_pre_command(
                "fixture-cli",
                &["argument with spaces"],
                &path_setup,
                "/bin/sh",
            ),
            b"protocol-input\n".to_vec(),
            Duration::from_secs(20),
        )
        .unwrap_or_else(|error| panic!("CLI fixture with initialized PATH: {error:#}"));
        assert_eq!(
            String::from_utf8(output)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["ready", "argument with spaces", "protocol-input"]
        );

        for fail in if cfg!(windows) {
            ["throw 'fixture setup failed'", "cmd.exe /c exit 7"]
        } else {
            ["exit 7", "false"]
        } {
            let error = run(
                cli_command_with_pre_command(&path, &[], fail, "/bin/sh"),
                vec![],
                Duration::from_secs(20),
            )
            .unwrap_err();
            assert!(error.to_string().contains("命令执行失败"), "{error:#}");
        }
    }

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

#[cfg(test)]
mod line_transport_tests {
    use super::*;

    const IDLE_TIMEOUT: Duration = Duration::from_secs(1);
    const FIXTURE_WAIT: Duration = Duration::from_secs(30);
    // Catch waiting for the child's natural exit, while allowing platform
    // process termination and a loaded CI host to finish cleanup.
    const CLEANUP_LIMIT: Duration = Duration::from_secs(15);

    fn run_fixture(
        mode: &str,
        input: Vec<u8>,
        timeout: Duration,
        mut receive: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "process::line_transport_tests::stream_fixture",
                "--ignored",
                "--nocapture",
                "--quiet",
                "--color=never",
            ])
            .env("AIEYES_STREAM_FIXTURE", mode);
        run_lines(command, input, timeout, |line| {
            // libtest prints this preamble before calling the child fixture.
            if line == b"\n" || line == b"running 1 test\n" {
                return Ok(());
            }
            receive(line)
        })
    }

    #[test]
    #[ignore = "subprocess fixture, exercised by the line transport tests"]
    fn stream_fixture() {
        let emit = |value: &[u8]| {
            let mut output = std::io::stdout().lock();
            output.write_all(value).unwrap();
            output.write_all(b"\n").unwrap();
            output.flush().unwrap();
        };
        match std::env::var("AIEYES_STREAM_FIXTURE").unwrap().as_str() {
            "active" => {
                for _ in 0..16 {
                    emit(b"{}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            "batches" => {
                emit(b"{}");
                emit(b"{}");
            }
            "stalled" => {
                emit(b"{}");
                std::thread::sleep(FIXTURE_WAIT);
            }
            "silent" => std::thread::sleep(FIXTURE_WAIT),
            "oversized" => emit(&vec![b'x'; 1024 * 1024 + 1]),
            "incomplete" => {
                let mut output = std::io::stdout().lock();
                output.write_all(b"{}").unwrap();
                output.flush().unwrap();
            }
            "echo" => {
                let input: serde_json::Value = serde_json::from_reader(std::io::stdin()).unwrap();
                let arguments: Vec<_> = std::env::args().collect();
                for n in 0..20 {
                    emit(
                        &serde_json::to_vec(
                            &serde_json::json!({"n":n,"text":input["text"],"args":arguments}),
                        )
                        .unwrap(),
                    );
                }
            }
            mode => panic!("unknown stream fixture: {mode}"),
        }
        // Do not append libtest's human-readable result to the NDJSON stream.
        std::process::exit(0);
    }

    #[test]
    fn active_streams_can_outlast_the_receive_timeout() {
        let mut count = 0;
        let started = Instant::now();
        run_fixture("active", vec![], IDLE_TIMEOUT, |line| {
            assert_eq!(line, b"{}\n");
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 16);
        assert!(started.elapsed() > IDLE_TIMEOUT);
    }
    #[test]
    fn local_batch_processing_does_not_consume_the_receive_timeout() {
        let mut count = 0;
        run_fixture("batches", vec![], IDLE_TIMEOUT, |line| {
            assert_eq!(line, b"{}\n");
            std::thread::sleep(IDLE_TIMEOUT + Duration::from_millis(100));
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 2);
    }
    #[test]
    fn stalled_streams_still_timeout_after_a_successful_batch() {
        let mut count = 0;
        let started = Instant::now();
        let error = run_fixture("stalled", vec![], IDLE_TIMEOUT, |line| {
            assert_eq!(line, b"{}\n");
            count += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(count, 1);
        assert!(error.to_string().contains("查询超时"));
        assert!(started.elapsed() < CLEANUP_LIMIT);
    }
    #[test]
    fn lines_are_streamed_and_input_is_not_a_command_argument() {
        let input = serde_json::to_vec(&serde_json::json!({"text":"literal '$()` text"})).unwrap();
        let mut values = vec![];
        run_fixture("echo", input, Duration::from_secs(5), |line| {
            values.push(serde_json::from_slice::<serde_json::Value>(line)?);
            Ok(())
        })
        .unwrap();
        assert_eq!(values.len(), 20);
        assert_eq!(values[19]["n"], 19);
        assert_eq!(values[19]["text"], "literal '$()` text");
        assert!(
            values[19]["args"]
                .as_array()
                .unwrap()
                .iter()
                .all(|arg| !arg.as_str().unwrap().contains("literal '$()` text"))
        );
    }
    #[test]
    fn errors_and_timeouts_close_the_stream_without_hanging_on_pipes() {
        for (mode, limit, expected) in [
            ("stalled", Duration::from_secs(3), "fixture consumer"),
            ("silent", IDLE_TIMEOUT, "查询超时"),
            (
                "oversized",
                Duration::from_secs(3),
                "远程统计批次过大或不完整",
            ),
            (
                "incomplete",
                Duration::from_secs(3),
                "远程统计批次过大或不完整",
            ),
        ] {
            let started = Instant::now();
            let error = run_fixture(mode, vec![], limit, |_| anyhow::bail!("fixture consumer"))
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert!(
                started.elapsed() < CLEANUP_LIMIT,
                "{mode}: cleanup took {:?}",
                started.elapsed()
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn successful_launcher_cleans_descendants_holding_pipes() {
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
