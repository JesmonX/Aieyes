use super::*;
use std::process::{Command, Stdio};
fn label(root: &Path) -> String {
    format!("app.aieyes.api.{}", &hash(&root.to_string_lossy())[..20])
}
fn xml(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn run(program: &str, args: &[String]) -> Result<Vec<u8>> {
    let mut command = Command::new(program);
    command.args(args);
    process::run(command, vec![], Duration::from_secs(20))
}
fn mac_path(root: &Path) -> PathBuf {
    crate::store::home()
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(root)))
}
fn mac_domain() -> String {
    #[cfg(unix)]
    {
        format!("gui/{}", unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
}
fn cron_lines() -> Result<Vec<String>> {
    let output = Command::new("crontab")
        .arg("-l")
        .env("LC_ALL", "C")
        .output()
        .context("目标系统未安装 crontab")?;
    ensure!(
        output.status.success()
            || output.status.code() == Some(1)
                && String::from_utf8_lossy(&output.stderr).contains("no crontab"),
        "无法读取 crontab"
    );
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_owned)
        .collect())
}
fn set_cron(root: &Path, owned: &[String]) -> Result<()> {
    let tag = format!("# {}", label(root));
    let old = cron_lines()?;
    let mut lines: Vec<_> = old.iter().filter(|v| !v.ends_with(&tag)).cloned().collect();
    lines.extend_from_slice(owned);
    if lines == old {
        return Ok(());
    }
    let mut command = Command::new("crontab");
    command.arg("-");
    process::run(
        command,
        (lines.join("\n") + "\n").into_bytes(),
        Duration::from_secs(20),
    )?;
    Ok(())
}
pub(super) enum Registration {
    Mac {
        bytes: Option<Vec<u8>>,
        loaded: bool,
    },
    Windows {
        bytes: Option<Vec<u8>>,
    },
    Cron(Vec<String>),
}
pub(super) fn snapshot(root: &Path) -> Result<Registration> {
    if cfg!(target_os = "macos") {
        let path = mac_path(root);
        crate::codex_auth::files::safe(&path)?;
        let bytes = match std::fs::read(path) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Registration::Mac {
            bytes,
            loaded: run(
                "launchctl",
                &["print".into(), format!("{}/{}", mac_domain(), label(root))],
            )
            .is_ok(),
        })
    } else if cfg!(windows) {
        Ok(Registration::Windows {
            bytes: run(
                "schtasks.exe",
                &["/Query".into(), "/TN".into(), label(root), "/XML".into()],
            )
            .ok(),
        })
    } else {
        let tag = format!("# {}", label(root));
        Ok(Registration::Cron(
            cron_lines()?
                .into_iter()
                .filter(|s| s.ends_with(&tag))
                .collect(),
        ))
    }
}
pub(super) fn unregister(root: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        let service = format!("{}/{}", mac_domain(), label(root));
        if run("launchctl", &["print".into(), service.clone()]).is_ok() {
            run("launchctl", &["bootout".into(), service])?;
        }
        let path = mac_path(root);
        crate::codex_auth::files::safe(&path)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    } else if cfg!(windows) {
        if run(
            "schtasks.exe",
            &["/Query".into(), "/TN".into(), label(root)],
        )
        .is_ok()
        {
            let _ = run("schtasks.exe", &["/End".into(), "/TN".into(), label(root)]);
            run(
                "schtasks.exe",
                &["/Delete".into(), "/TN".into(), label(root), "/F".into()],
            )?;
        }
        let path = root.join("scheduler.xml");
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    } else {
        set_cron(root, &[])?;
    }
    Ok(())
}
pub(super) fn restore(root: &Path, previous: &Registration) -> Result<()> {
    match previous {
        Registration::Mac { bytes, loaded } => {
            let path = mac_path(root);
            let current = std::fs::read(&path).ok();
            let running = run(
                "launchctl",
                &["print".into(), format!("{}/{}", mac_domain(), label(root))],
            )
            .is_ok();
            if &current == bytes && running == *loaded {
                return Ok(());
            }
            unregister(root)?;
            if let Some(bytes) = bytes {
                write_bytes(&path, bytes)?;
                if *loaded {
                    run(
                        "launchctl",
                        &[
                            "bootstrap".into(),
                            mac_domain(),
                            path.to_string_lossy().into(),
                        ],
                    )?;
                }
            }
        }
        Registration::Windows { bytes } => {
            if let Some(bytes) = bytes {
                let path = root.join("scheduler.xml");
                write_bytes(&path, bytes)?;
                run(
                    "schtasks.exe",
                    &[
                        "/Create".into(),
                        "/TN".into(),
                        label(root),
                        "/XML".into(),
                        path.to_string_lossy().into(),
                        "/F".into(),
                    ],
                )?;
            } else {
                unregister(root)?;
            }
        }
        Registration::Cron(lines) => set_cron(root, lines)?,
    }
    Ok(())
}
pub(super) fn install_runtime(root: &Path) -> Result<Runtime> {
    let source = crate::wakeups::runtime_binary()?;
    let bytes = std::fs::read(&source)?;
    use sha2::{Digest, Sha256};
    let fingerprint = format!("{:x}", Sha256::digest(&bytes));
    let runner = root.join(format!(
        "runner-{}{}",
        &fingerprint[..16],
        if cfg!(windows) { ".exe" } else { "" }
    ));
    if !runner.exists() {
        std::fs::copy(&source, &runner)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let previous: Option<Runtime> = read_json(&root.join("runtime.json")).ok();
    let port = match previous {
        Some(v) => v.port,
        None => std::net::TcpListener::bind(("127.0.0.1", 0))?
            .local_addr()?
            .port(),
    };
    let runtime = Runtime { port, runner };
    atomic(&root.join("runtime.json"), &runtime)?;
    Ok(runtime)
}
pub(super) fn register(root: &Path, r: &Runtime) -> Result<()> {
    let name = label(root);
    if cfg!(target_os = "macos") {
        let path = crate::store::home()
            .join("Library/LaunchAgents")
            .join(format!("{name}.plist"));
        std::fs::create_dir_all(path.parent().unwrap())?;
        #[cfg(unix)]
        let domain = format!("gui/{}", unsafe { libc::geteuid() });
        #[cfg(not(unix))]
        let domain = String::new();
        let service = format!("{domain}/{name}");
        let old = std::fs::read(&path).ok();
        let desired = format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>{name}</string><key>ProgramArguments</key><array><string>{}</string><string>--codex-api-service</string><string>{}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ProcessType</key><string>Background</string></dict></plist>",
            xml(&r.runner.to_string_lossy()),
            xml(&root.to_string_lossy())
        );
        if old.as_deref() == Some(desired.as_bytes())
            && run("launchctl", &["print".into(), service.clone()]).is_ok()
        {
            return Ok(());
        }
        if run("launchctl", &["print".into(), service.clone()]).is_ok() {
            run("launchctl", &["bootout".into(), service])?;
        }
        write_bytes(&path, desired.as_bytes())?;
        if let Err(error) = run(
            "launchctl",
            &[
                "bootstrap".into(),
                domain.clone(),
                path.to_string_lossy().into(),
            ],
        ) {
            if let Some(old) = old {
                std::fs::write(&path, old)?;
                let _ = run(
                    "launchctl",
                    &["bootstrap".into(), domain, path.to_string_lossy().into()],
                );
            } else {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
    } else if cfg!(windows) {
        let path = root.join("scheduler.xml");
        let desired = format!(
            "<?xml version=\"1.0\"?><Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers><Principals><Principal id=\"Author\"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure></Settings><Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>--codex-api-service &quot;{}&quot;</Arguments></Exec></Actions></Task>",
            xml(&r.runner.to_string_lossy()),
            xml(&root.to_string_lossy())
        );
        if std::fs::read(&path).ok().as_deref() == Some(desired.as_bytes())
            && run(
                "schtasks.exe",
                &["/Query".into(), "/TN".into(), name.clone()],
            )
            .is_ok()
        {
            return Ok(());
        }
        write_bytes(&path, desired.as_bytes())?;
        run(
            "schtasks.exe",
            &[
                "/Create".into(),
                "/TN".into(),
                name,
                "/XML".into(),
                path.to_string_lossy().into(),
                "/F".into(),
            ],
        )?;
    } else {
        let tag = format!("# {name}");
        let line = format!(
            "* * * * * {} --codex-api-service {} >/dev/null 2>&1 {tag}",
            process::quote(&r.runner.to_string_lossy()).replace('%', "\\%"),
            process::quote(&root.to_string_lossy()).replace('%', "\\%")
        );
        set_cron(root, &[line])?;
    }
    Ok(())
}
pub(super) fn healthy(root: &Path, r: &Runtime) -> bool {
    let entries = match std::fs::read_dir(root.join("connections")) {
        Ok(v) => v,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if let Ok(c) = read_json::<PrivateConnection>(&entry.path()) {
            let response = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(400))
                .build()
                .ok()
                .and_then(|client| {
                    client
                        .get(format!(
                            "http://127.0.0.1:{}/providers/{}/v1/health",
                            r.port, c.connection.id
                        ))
                        .bearer_auth(c.local_token)
                        .send()
                        .ok()
                })
                .and_then(|r| r.json::<Value>().ok());
            return response.is_some_and(|v| v["ready"] == true);
        }
    }
    false
}
pub(super) fn ensure_started(root: &Path) -> Result<()> {
    let runtime: Runtime = read_json(&root.join("runtime.json"))?;
    if healthy(root, &runtime) {
        return Ok(());
    }
    if cfg!(windows) {
        run("schtasks.exe", &["/Run".into(), "/TN".into(), label(root)])?;
    }
    let mut command = Command::new(&runtime.runner);
    command
        .args(["--codex-api-service", &root.to_string_lossy()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process::prepare(&mut command);
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if healthy(root, &runtime) {
            return Ok(());
        }
    }
    anyhow::bail!("API 计量服务未就绪；请检查服务与监听端口")
}
