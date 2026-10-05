use super::*;
use std::fs;
fn label(root: &Path) -> String {
    format!("app.aieyes.wakeup.{}", &hash(&root.to_string_lossy())[..20])
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn installed_runner(root: &Path) -> Result<PathBuf> {
    let v: Value = serde_json::from_slice(&fs::read(root.join("runtime.json"))?)?;
    Ok(PathBuf::from(
        v["runner"].as_str().context("执行器路径缺失")?,
    ))
}
fn run_system(program: &str, args: &[String]) -> Result<Vec<u8>> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    process::run(cmd, vec![], Duration::from_secs(20))
}
fn cron(root: &Path, exe: &Path, remove: bool) -> Result<()> {
    let tag = format!("# {}", label(root));
    let output = Command::new("crontab")
        .arg("-l")
        .output()
        .context("目标系统未安装 crontab")?;
    ensure!(
        output.status.success()
            || (output.status.code() == Some(1)
                && (String::from_utf8_lossy(&output.stderr).contains("no crontab")
                    || output.stderr.is_empty())),
        "无法读取已有 crontab"
    );
    let old = String::from_utf8(output.stdout)?;
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| !l.ends_with(&tag))
        .map(str::to_owned)
        .collect();
    if !remove {
        lines.push(format!(
            "* * * * * {} --wake-runner {} >/dev/null 2>&1 {}",
            process::quote(&exe.to_string_lossy()).replace('%', "\\%"),
            process::quote(&root.to_string_lossy()).replace('%', "\\%"),
            tag
        ));
    }
    let mut cmd = Command::new("crontab");
    cmd.arg("-");
    process::run(
        cmd,
        (lines.join("\n") + "\n").into_bytes(),
        Duration::from_secs(20),
    )?;
    Ok(())
}
fn register(root: &Path, exe: &Path, remove: bool) -> Result<()> {
    if cfg!(target_os = "macos") {
        let path = store::home()
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label(root)));
        #[cfg(unix)]
        let domain = format!("gui/{}", unsafe { libc::geteuid() });
        #[cfg(not(unix))]
        let domain = String::new();
        let service = format!("{domain}/{}", label(root));
        if remove {
            if run_system("launchctl", &["print".into(), service.clone()]).is_ok() {
                run_system("launchctl", &["bootout".into(), service])?;
            }
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let contents = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string><string>--wake-service</string><string>{}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ProcessType</key><string>Background</string></dict></plist>",
            xml(&label(root)),
            xml(&exe.to_string_lossy()),
            xml(&root.to_string_lossy())
        );
        let old = fs::read(&path).ok();
        if run_system("launchctl", &["print".into(), service.clone()]).is_ok() {
            run_system("launchctl", &["bootout".into(), service])?;
        }
        fs::write(&path, contents)?;
        if let Err(error) = run_system(
            "launchctl",
            &[
                "bootstrap".into(),
                domain.clone(),
                path.to_string_lossy().into(),
            ],
        ) {
            if let Some(old) = old {
                fs::write(&path, old)?;
                let _ = run_system(
                    "launchctl",
                    &["bootstrap".into(), domain, path.to_string_lossy().into()],
                );
            } else {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
    } else if cfg!(windows) {
        let name = label(root);
        if remove {
            // An already removed task is success, but failure to enumerate the
            // scheduler must remain an error so ownership is not forgotten.
            let tasks = run_system(
                "schtasks.exe",
                &["/Query".into(), "/FO".into(), "CSV".into(), "/NH".into()],
            )?;
            if !String::from_utf8_lossy(&tasks).contains(&name) {
                return Ok(());
            }
            run_system(
                "schtasks.exe",
                &["/Delete".into(), "/TN".into(), name, "/F".into()],
            )?;
            return Ok(());
        }
        let path = root.join("scheduler.xml");
        let args = format!("--wake-runner \"{}\"", root.to_string_lossy());
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><Triggers><TimeTrigger><Repetition><Interval>PT1M</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers><Principals><Principal id=\"Author\"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>false</StartWhenAvailable><ExecutionTimeLimit>PT3M</ExecutionTimeLimit><Enabled>true</Enabled><WakeToRun>false</WakeToRun></Settings><Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>{}</Arguments></Exec></Actions></Task>",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:00"),
            xml(&exe.to_string_lossy()),
            xml(&args)
        );
        fs::write(&path, xml)?;
        run_system(
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
        cron(root, exe, remove)?;
    }
    Ok(())
}
pub fn deploy(root: &Path, m: &Manifest) -> Result<Value> {
    runner::private_dir(root)?;
    runner::private_dir(&root.join("tasks"))?;
    let source = RUNNER_PATH
        .get()
        .cloned()
        .unwrap_or(std::env::current_exe()?);
    ensure!(source.is_file(), "独立执行器未打包；请重新构建安装包");
    let bytes = fs::read(&source)?;
    let fingerprint = hash(&format!("{}:{}", bytes.len(), hash_bytes(&bytes)));
    let exe = root.join(format!(
        "runner-{}{}",
        &fingerprint[..16],
        if cfg!(windows) { ".exe" } else { "" }
    ));
    if !exe.exists() {
        fs::copy(source, &exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o700))?;
        }
    }
    let path = root.join("tasks").join(format!("{}.json", m.id));
    let old = fs::read(&path).ok();
    runner::atomic_json(&path, m)?;
    if let Err(e) = register(root, &exe, false) {
        if let Some(old) = old {
            fs::write(path, old)?;
        } else {
            fs::remove_file(path)?;
        }
        return Err(e);
    }
    runner::atomic_json(&root.join("runtime.json"), &json!({"runner":exe}))?;
    Ok(json!({"root":root,"timezone":chrono::Local::now().format("%Z %:z").to_string()}))
}
fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
pub fn control(root: &Path, id: &str, action: &str) -> Result<Value> {
    let path = root.join("tasks").join(format!("{id}.json"));
    match action {
        "history" => Ok(json!(runner::history(root, id)?)),
        "status" => {
            let m: Option<Manifest> = fs::read(&path)
                .ok()
                .and_then(|v| serde_json::from_slice(&v).ok());
            let registered = if cfg!(target_os = "macos") {
                #[cfg(unix)]
                let domain = format!("gui/{}/{}", unsafe { libc::geteuid() }, label(root));
                #[cfg(not(unix))]
                let domain = String::new();
                run_system("launchctl", &["print".into(), domain]).is_ok()
            } else if cfg!(windows) {
                run_system(
                    "schtasks.exe",
                    &["/Query".into(), "/TN".into(), label(root)],
                )
                .is_ok()
            } else {
                run_system("crontab", &["-l".into()]).is_ok_and(|v| {
                    String::from_utf8_lossy(&v).contains(&format!("# {}", label(root)))
                })
            };
            let next = m
                .as_ref()
                .filter(|m| m.enabled && registered)
                .and_then(|m| next_run(&m.times));
            Ok(
                json!({"installed":m.is_some()&&registered,"enabled":m.as_ref().is_some_and(|m|m.enabled),"timezone":chrono::Local::now().format("%Z %:z").to_string(),"nextRunAt":next,"history":runner::history(root,id)?}),
            )
        }
        "run" => {
            ensure!(path.is_file(), "部署文件不存在");
            let mut cmd = Command::new(installed_runner(root)?);
            cmd.arg("--wake-runner")
                .arg(root)
                .arg(id)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            process::prepare(&mut cmd);
            let mut child = cmd.spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(json!({"started":true}))
        }
        "enable" | "disable" => {
            let mut m: Manifest = serde_json::from_slice(&fs::read(&path)?)?;
            m.enabled = action == "enable";
            runner::atomic_json(&path, &m)?;
            Ok(json!({"enabled":m.enabled}))
        }
        "remove" => {
            let others = fs::read_dir(root.join("tasks"))
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| e.path() != path && e.path().extension().is_some_and(|v| v == "json"));
            if !others {
                register(
                    root,
                    &installed_runner(root).unwrap_or_else(|_| root.join("runner")),
                    true,
                )?;
            }
            if path.exists() {
                fs::remove_file(path)?;
            }
            Ok(json!({"removed":true}))
        }
        _ => anyhow::bail!("未知部署操作"),
    }
}
fn next_run(times: &[String]) -> Option<i64> {
    use chrono::Timelike;
    let start = chrono::Local::now();
    (1..=2880)
        .map(|m| start + chrono::Duration::minutes(m))
        .find(|t| times.contains(&format!("{:02}:{:02}", t.hour(), t.minute())))
        .map(|t| t.timestamp() - i64::from(t.second()))
}
