//! Windows switch targets are scoped to the current token; no taskkill tree or elevation.
use super::*;
use anyhow::Context;
use std::{path::PathBuf, ptr, time::SystemTime};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, HWND, LPARAM,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
    System::Threading::*,
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE},
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn open(pid: u32, access: u32) -> Result<Handle> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    ensure!(
        !h.is_null(),
        "无法读取或关闭进程 PID {pid}，请手动关闭后重新检查"
    );
    Ok(Handle(h))
}
fn token_user(process: HANDLE) -> Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
        "无法确认进程用户"
    );
    let token = Handle(token);
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    ensure!(size > 0, "无法确认进程用户");
    // Pointer-aligned storage for TOKEN_USER and its embedded SID.
    let mut bytes = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } != 0,
        "无法确认进程用户"
    );
    Ok(bytes)
}
fn same_user(h: HANDLE) -> Result<bool> {
    let ours = token_user(unsafe { GetCurrentProcess() })?;
    let theirs = token_user(h)?;
    Ok(unsafe {
        EqualSid(
            (*(ours.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            (*(theirs.as_ptr().cast::<TOKEN_USER>())).User.Sid,
        )
    } != 0)
}
fn image(h: HANDLE) -> Result<String> {
    let mut text = vec![0u16; 32768];
    let mut len = text.len() as u32;
    ensure!(
        unsafe { QueryFullProcessImageNameW(h, 0, text.as_mut_ptr(), &mut len) } != 0,
        "无法确认进程路径"
    );
    Ok(String::from_utf16(&text[..len as usize])?)
}
fn identity(h: HANDLE, pid: u32) -> Result<(String, String, SystemTime)> {
    ensure!(same_user(h)?, "此进程属于其他用户，请手动处理后重新检查");
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit = created;
    let mut kernel = created;
    let mut user = created;
    ensure!(
        unsafe { GetProcessTimes(h, &mut created, &mut exit, &mut kernel, &mut user) } != 0,
        "无法确认进程启动时间"
    );
    let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    let path = image(h)?;
    let fingerprint = crate::models::hash(&format!("{pid}:{ticks}:{}", normalize(&path)));
    let unix = ticks
        .checked_sub(116_444_736_000_000_000)
        .context("无效进程启动时间")?;
    Ok((
        fingerprint,
        path,
        SystemTime::UNIX_EPOCH + Duration::from_nanos(unix * 100),
    ))
}
fn normalize(path: &str) -> String {
    path.replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_lowercase()
}
fn service_image(command: &str) -> Option<&str> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return rest.split_once('"').map(|(path, _)| path);
    }
    command.to_lowercase().ends_with(".exe").then_some(command)
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Service {
    name: String,
    process_id: u32,
    path_name: String,
}
fn services() -> Result<Vec<Service>> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Service -Filter \"Name LIKE 'CodexSandboxService%'\" | Select-Object Name,ProcessId,PathName)"]);
    let bytes = crate::process::run(cmd, vec![], Duration::from_secs(10))?;
    serde_json::from_slice(&bytes).context("无法确认 Windows 沙箱服务，请重新检查")
}
fn sandbox_image(path: &str) -> bool {
    normalize(path).rsplit('\\').next() == Some("codex-windows-sandbox-service.exe")
}
fn sandbox_service(pid: u32, path: &str, services: &[Service]) -> bool {
    sandbox_image(path)
        && services.iter().any(|s| {
            s.process_id == pid
                && (s.name.eq_ignore_ascii_case("CodexSandboxService")
                    || s.name
                        .to_lowercase()
                        .strip_prefix("codexsandboxservice.")
                        .is_some_and(|suffix| !suffix.is_empty()))
                && service_image(&s.path_name)
                    .is_some_and(|image| normalize(image) == normalize(path))
        })
}
pub(super) fn observe() -> Result<Vec<Observation>> {
    let mut service_rows = None;
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything()
            .with_cmd(UpdateKind::Always)
            .with_environ(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    ensure!(
        system
            .process(sysinfo::Pid::from_u32(std::process::id()))
            .is_some(),
        "无法枚举 Windows 进程，账号未切换，请重新检查"
    );
    let mut rows = Vec::new();
    for (pid, process) in system.processes() {
        let pid = pid.as_u32();
        if pid == std::process::id() {
            continue;
        }
        let name = process.name().to_string_lossy().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name).to_owned();
        let candidate = name.starts_with("codex") || name.starts_with("chatgpt");
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION);
        let actual_path = handle.as_ref().ok().and_then(|h| image(h.0).ok());
        if let Some(path) = actual_path.as_deref().filter(|path| sandbox_image(path)) {
            // Most reviews have no sandbox service. Query registration only for
            // a matching image, retaining all identity checks before exclusion.
            if service_rows.is_none() {
                service_rows = Some(services()?);
            }
            if service_rows
                .as_ref()
                .is_some_and(|services| sandbox_service(pid, path, services))
            {
                continue;
            }
        }
        if handle
            .as_ref()
            .ok()
            .is_some_and(|h| same_user(h.0).ok() == Some(false))
        {
            continue;
        }
        let verified = handle.and_then(|h| identity(h.0, pid));
        let (fingerprint, executable, started, verified) = match verified {
            Ok((f, path, time)) => (f, path, Some(time), true),
            Err(_) => (
                crate::models::hash(&format!("{pid}:unverified:{}", process.start_time())),
                actual_path.unwrap_or_default(),
                None,
                false,
            ),
        };
        // Retain unreadable candidates/descendants as blockers, never as closable targets.
        let args: Vec<_> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let env: Vec<_> = process
            .environ()
            .iter()
            .flat_map(|s| {
                let mut bytes = s.to_string_lossy().as_bytes().to_vec();
                bytes.push(0);
                bytes
            })
            .collect();
        let name = if args.iter().any(|arg| arg.starts_with("--type="))
            && matches!(name.as_str(), "codex" | "chatgpt")
        {
            format!("{name}-app-helper")
        } else {
            name
        };
        let mut auth_usage =
            if !verified || (name == "codex" && (args.is_empty() || env.is_empty())) {
                Usage::UnknownProfile
            } else if name == "codex" {
                process_auth::inspect(&args, &env, started)
            } else {
                Usage::Account
            };
        if auth_usage == Usage::IndependentApi && process_auth::uses_default(&args) {
            auth_usage = Usage::DefaultApi;
        }
        let home = process_auth::environment(&env, "CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                process_auth::environment(&env, "USERPROFILE").map(|s| Path::new(s).join(".codex"))
            })
            .filter(|home| home.is_absolute())
            .and_then(|home| std::fs::canonicalize(home).ok());
        if verified || candidate || process.parent().is_some() {
            let fingerprint = format!("{fingerprint}:{}", auth_usage as u8);
            rows.push(Observation {
                pid,
                parent_pid: process.parent().map(|p| p.as_u32()).unwrap_or(0),
                fingerprint,
                name,
                executable,
                home,
                auth_usage,
            });
        }
    }
    Ok(rows)
}
pub(super) fn list(root: &Path) -> Result<Vec<Running>> {
    Ok(observe()?
        .iter()
        .filter(|p| p.name == "codex" || p.name.starts_with("codex-"))
        .filter(|p| {
            p.home.as_ref().is_none_or(|home| {
                normalize(&home.to_string_lossy()) == normalize(&root.to_string_lossy())
            })
        })
        .map(|p| Running {
            pid: p.pid,
            fingerprint: p.fingerprint.clone(),
            name: p.name.clone(),
            parent_pid: Some(p.parent_pid),
            can_close: false,
            blocking_reason: Some("请先关闭此目录的 Codex 进程后重试".into()),
        })
        .collect())
}
struct Windows {
    pid: u32,
    handles: Vec<HWND>,
}
unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> i32 {
    let windows = unsafe { &mut *(data as *mut Windows) };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == windows.pid {
        windows.handles.push(hwnd);
    }
    1
}
fn gui_image(path: &str) -> Option<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).ok()?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().ok()?);
    if offset > 1024 * 1024 {
        return None;
    }
    file.seek(SeekFrom::Start(offset as u64)).ok()?;
    let mut pe = [0u8; 94];
    file.read_exact(&mut pe).ok()?;
    if &pe[..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes(pe[92..94].try_into().ok()?) == 2)
}
pub(super) fn terminate(expected: &Running) -> Result<()> {
    let handle = match open(
        expected.pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
    ) {
        Ok(handle) => handle,
        Err(_) if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER => return Ok(()),
        Err(error) => return Err(error),
    };
    if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 {
        return Ok(());
    }
    let (fingerprint, path, _) = identity(handle.0, expected.pid)?;
    ensure!(
        Some(fingerprint.as_str()) == expected.fingerprint.split(':').next(),
        "进程已重新启动，账号未切换，请重新检查"
    );
    let mut windows = Windows {
        pid: expected.pid,
        handles: vec![],
    };
    ensure!(
        unsafe {
            EnumWindows(
                Some(collect_window),
                (&mut windows as *mut Windows) as LPARAM,
            )
        } != 0,
        "无法检查应用窗口，请手动关闭后重试"
    );
    let path = normalize(&path);
    let gui_root = matches!(expected.name.as_str(), "Codex" | "ChatGPT 应用");
    let gui = (gui_root && gui_image(&path).unwrap_or(true))
        || !windows.handles.is_empty()
        || path.ends_with("\\chatgpt.exe")
        || (path.contains("\\windowsapps\\")
            && !path.contains("\\resources\\")
            && path.ends_with("\\codex.exe"));
    if gui {
        ensure!(
            !windows.handles.is_empty(),
            "应用没有可关闭的窗口，请手动退出后重新检查"
        );
        for hwnd in windows.handles {
            if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 {
                return Ok(());
            }
            let mut owner = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut owner);
            }
            if owner == 0 {
                continue;
            } // The window was closed naturally.
            ensure!(owner == expected.pid, "应用窗口已变化，请重新检查");
            ensure!(
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) } != 0,
                "应用拒绝退出，请手动关闭后重新检查"
            );
        }
    } else {
        // Reopen with terminate rights and revalidate this exact process instance.
        let kill = match open(
            expected.pid,
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
        ) {
            Ok(handle) => handle,
            Err(_) if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 => return Ok(()),
            Err(error) => return Err(error),
        };
        ensure!(
            Some(identity(kill.0, expected.pid)?.0.as_str())
                == expected.fingerprint.split(':').next(),
            "进程已重新启动，请重新检查"
        );
        ensure!(
            unsafe { TerminateProcess(kill.0, 0) } != 0
                || unsafe { WaitForSingleObject(kill.0, 0) } == WAIT_OBJECT_0,
            "无法关闭进程，请手动关闭后重新检查"
        );
    }
    let result = unsafe { WaitForSingleObject(handle.0, 10_000) };
    ensure!(
        result != WAIT_TIMEOUT,
        "Codex / ChatGPT 未退出，账号未切换，请保存工作并手动退出后重新检查"
    );
    ensure!(
        result == WAIT_OBJECT_0,
        "无法确认进程已退出，账号未切换，请重新检查"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sandbox_requires_registered_pid_name_and_image() {
        let path = r"C:\Codex\codex-windows-sandbox-service.exe";
        let mut rows = vec![Service {
            name: "CodexSandboxService.package".into(),
            process_id: 42,
            path_name: format!("\"{path}\" --service"),
        }];
        assert!(sandbox_service(42, path, &rows));
        assert!(!sandbox_service(43, path, &rows));
        assert!(!sandbox_service(
            42,
            r"C:\Other\codex-windows-sandbox-service.exe",
            &rows
        ));
        rows[0].name = "CodexSandboxServiceFake".into();
        assert!(!sandbox_service(42, path, &rows));
        assert!(serde_json::from_slice::<Vec<Service>>(b"invalid").is_err());
    }
    #[test]
    fn helper_scope_requires_parent_or_confirmed_identity() {
        let make = |pid, parent_pid, name: &str, auth_usage| Observation {
            pid,
            parent_pid,
            name: name.into(),
            executable: format!(r"C:\fixture\{name}.exe"),
            home: None,
            fingerprint: format!("start-{pid}"),
            auth_usage,
        };
        let observations = vec![
            make(10, 0, "codex", Usage::IndependentApi),
            make(11, 10, "codex-app-helper", Usage::Account),
            make(20, 0, "codex", Usage::Account),
            make(21, 20, "codex-code-mode-host", Usage::Account),
            make(30, 0, "codex-code-mode-host", Usage::Account),
            make(40, 0, "codex", Usage::UnknownProfile),
        ];
        let (rows, preserved) = switch_review(&observations, &[]);
        assert_eq!(
            preserved.iter().map(|r| r.pid).collect::<Vec<_>>(),
            [10, 11]
        );
        assert!(rows.iter().find(|r| r.pid == 21).unwrap().can_close);
        assert!(!rows.iter().find(|r| r.pid == 30).unwrap().can_close);
        assert!(!rows.iter().find(|r| r.pid == 40).unwrap().can_close);
        let mut confirmed = rows.iter().find(|r| r.pid == 30).unwrap().clone();
        confirmed.can_close = true;
        assert!(
            switch_review(&observations, &[confirmed.clone()])
                .0
                .iter()
                .find(|r| r.pid == 30)
                .unwrap()
                .can_close
        );
        confirmed.fingerprint = "reused".into();
        assert!(
            !switch_review(&observations, &[confirmed])
                .0
                .iter()
                .find(|r| r.pid == 30)
                .unwrap()
                .can_close
        );
        assert!(open(0, PROCESS_QUERY_LIMITED_INFORMATION).is_err());
        assert_eq!(
            process_auth::environment(b"CodeX_Home=C:\\fixture\0", "CODEX_HOME"),
            Some(r"C:\fixture")
        );
    }
    struct Fixture {
        child: std::process::Child,
        _root: tempfile::TempDir,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn fixture(mode: &str, api: bool) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("codex.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        if api {
            std::fs::write(root.path().join("config.toml"), "model_provider='fixture'\n[model_providers.fixture]\nbase_url='https://fixture.invalid'\nexperimental_bearer_token='fixture-token'\n").unwrap();
        }
        let ready = root.path().join("ready");
        let child = Command::new(executable)
            .args([
                "--exact",
                "codex_auth::processes::windows::tests::disposable_fixture",
                "--ignored",
            ])
            .env("AIEYES_WINDOWS_FIXTURE", mode)
            .env("AIEYES_WINDOWS_READY", &ready)
            .env("CODEX_HOME", root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let fixture = Fixture { child, _root: root };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture did not start"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        fixture
    }
    #[test]
    #[ignore = "child process fixture, explicitly launched by the integration tests"]
    fn disposable_fixture() {
        use windows_sys::Win32::{
            Foundation::{LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::*,
        };
        unsafe extern "system" fn messages(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == WM_CLOSE {
                if std::env::var("AIEYES_WINDOWS_FIXTURE").as_deref() != Ok("refuse") {
                    std::process::exit(0);
                }
                return 0;
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        let mode = std::env::var("AIEYES_WINDOWS_FIXTURE").unwrap();
        if mode != "cli" {
            let class: Vec<u16> = "AieyesDisposableSwitchFixture\0".encode_utf16().collect();
            let instance = unsafe { GetModuleHandleW(ptr::null()) };
            let definition = WNDCLASSW {
                lpfnWndProc: Some(messages),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert!(unsafe { RegisterClassW(&definition) } != 0);
            assert!(
                !unsafe {
                    CreateWindowExW(
                        0,
                        class.as_ptr(),
                        class.as_ptr(),
                        WS_OVERLAPPEDWINDOW,
                        0,
                        0,
                        100,
                        100,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        instance,
                        ptr::null(),
                    )
                }
                .is_null()
            );
        }
        std::fs::write(std::env::var_os("AIEYES_WINDOWS_READY").unwrap(), b"ready").unwrap();
        if mode == "cli" {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        let mut message = unsafe { std::mem::zeroed() };
        while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    #[test]
    fn handles_reject_changed_identity_and_close_disposable_cli() {
        let mut fixture = fixture("cli", false);
        let (rows, _) = review(&[]).unwrap();
        let mut row = rows
            .into_iter()
            .find(|p| p.pid == fixture.child.id())
            .unwrap();
        assert!(row.can_close);
        let fingerprint = row.fingerprint.clone();
        row.fingerprint = "reused-pid".into();
        assert!(terminate(&row).is_err());
        assert!(fixture.child.try_wait().unwrap().is_none());
        row.fingerprint = fingerprint;
        terminate(&row).unwrap();
        assert!(fixture.child.wait().unwrap().code().is_some());
    }
    #[test]
    fn native_review_preserves_independent_api_and_gui_exits_normally() {
        let mut app = fixture("gui", false);
        let mut api = fixture("cli", true);
        let (rows, preserved) = review(&[]).unwrap();
        assert!(preserved.iter().any(|p| p.pid == api.child.id()));
        assert!(!rows.iter().any(|p| p.pid == api.child.id()));
        let row = rows.iter().find(|p| p.pid == app.child.id()).unwrap();
        assert!(row.can_close);
        terminate(row).unwrap();
        assert!(app.child.wait().unwrap().success());
        assert!(api.child.try_wait().unwrap().is_none());
    }
    #[test]
    fn refusing_gui_is_never_force_terminated() {
        let mut app = fixture("refuse", false);
        let row = review(&[])
            .unwrap()
            .0
            .into_iter()
            .find(|p| p.pid == app.child.id())
            .unwrap();
        assert!(terminate(&row).is_err());
        assert!(app.child.try_wait().unwrap().is_none());
    }
}
