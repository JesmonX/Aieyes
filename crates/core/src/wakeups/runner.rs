//! Headless local runner. Does not open the GUI or the main application database.
use super::*;
use chrono::{Local, Timelike};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    time::Duration,
};

pub(crate) const CLEAR_ENV: &[&str] = &[
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

pub fn private_dir(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "任务目录不能是符号链接"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let root = path.parent().context("任务路径无效")?;
    private_dir(root)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    let result = (|| -> Result<()> {
        f.write_all(&serde_json::to_vec(value)?)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
fn command(m: &Manifest, args: &[String], cwd: &Path) -> Command {
    let mut cmd = if m.pre_command.trim().is_empty() {
        let mut cmd = process::cli_command(&m.binary);
        cmd.args(args);
        cmd
    } else if cfg!(windows) {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let script = format!(
            "$ErrorActionPreference='Stop'\n{}\n& {} {}\nexit $LASTEXITCODE",
            m.pre_command,
            quote(&m.binary),
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        );
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        cmd
    } else {
        let script = format!(
            "set -e\n{}{}\nexec {} {}",
            ssh::PATH_FALLBACK,
            m.pre_command,
            process::quote(&m.binary),
            args.iter()
                .map(|a| process::quote(a))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut cmd = Command::new(&m.shell);
        cmd.args(["-c", &script]);
        cmd
    };
    cmd.current_dir(cwd);
    for key in CLEAR_ENV {
        cmd.env_remove(key);
    }
    if m.provider == "codex" {
        cmd.env("CODEX_HOME", &m.config_path);
    }
    if m.provider == "claude" {
        cmd.env("CLAUDE_CONFIG_DIR", &m.config_path);
    }
    network::apply_env(&mut cmd, &m.proxy);
    cmd
}
pub(crate) fn invoke(m: &Manifest, args: &[String], cwd: &Path, seconds: u64) -> Result<Vec<u8>> {
    process::run(command(m, args, cwd), vec![], Duration::from_secs(seconds))
}
pub(crate) fn response_ok(provider: &str, output: &[u8]) -> bool {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .any(|v| match provider {
            "codex" => v["type"] == "turn.completed",
            "claude" => {
                v["type"] == "result" && v["is_error"] == false && v["subtype"] == "success"
            }
            "agy" => {
                v["status"] == "SUCCESS"
                    || (v["event"] == "result" && v["result"]["status"] == "SUCCESS")
            }
            _ => false,
        })
}
pub(crate) fn auth_check(m: &Manifest, cwd: &Path) -> Result<()> {
    if m.provider == "claude" {
        let bytes = invoke(m, &["auth".into(), "status".into()], cwd, 20)?;
        let v: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            v["loggedIn"] == true
                && matches!(v["authMethod"].as_str(), Some("claude.ai" | "oauth_token")),
            "目标 Claude CLI 未使用订阅登录"
        );
    }
    if m.provider == "agy" {
        // agy has no documented custom authentication-root switch.
        ensure!(
            Path::new(&m.config_path) == store::home().join(".gemini/antigravity-cli"),
            "agy 唤醒当前仅支持目标用户的默认登录目录"
        );
        let bytes = invoke(
            m,
            &[
                "--print".into(),
                "/usage".into(),
                "--output-format".into(),
                "json".into(),
                "--print-timeout".into(),
                "20s".into(),
            ],
            cwd,
            25,
        )?;
        ensure!(
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .any(|v| v["status"] == "SUCCESS" && v["command"]["name"] == "usage"),
            "目标 agy CLI 未提供订阅额度"
        );
    }
    Ok(())
}
/// Launchd does not overlap invocations of the same job. Keep a lightweight
/// dispatcher alive so a long response does not suppress the next minute.
pub fn service(root: &Path) -> Result<()> {
    let mut minute = None;
    let mut children: Vec<std::process::Child> = Vec::new();
    loop {
        children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        let current = now() / 60;
        if minute != Some(current) {
            minute = Some(current);
            let mut cmd = Command::new(std::env::current_exe()?);
            cmd.arg("--wake-runner")
                .arg(root)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            process::prepare(&mut cmd);
            children.push(cmd.spawn()?);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
pub fn run(root: &Path, manual: Option<&str>) -> Result<()> {
    private_dir(root)?;
    let mut lock = rusqlite::Connection::open(root.join("runner.sqlite"))?;
    lock.busy_timeout(Duration::from_secs(3))?;
    lock.execute_batch("CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,task TEXT,occurrence TEXT UNIQUE,started INTEGER,ended INTEGER,status TEXT,model TEXT,effort TEXT); CREATE TABLE IF NOT EXISTS leases(account TEXT PRIMARY KEY,expires INTEGER);")?;
    let local = Local::now();
    let slot = format!("{:02}:{:02}", local.hour(), local.minute());
    let mut jobs = Vec::new();
    for entry in fs::read_dir(root.join("tasks"))?.flatten() {
        if entry.path().extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let m: Manifest = serde_json::from_slice(&fs::read(entry.path())?)?;
        if manual.is_some_and(|id| id != m.id)
            || (manual.is_none() && (!m.enabled || !m.times.contains(&slot)))
        {
            continue;
        }
        if manual.is_none() && Local::now().format("%H:%M").to_string() != slot {
            continue;
        }
        let occurrence = if manual.is_some() {
            format!("{}:manual:{}:{}", m.id, now(), std::process::id())
        } else {
            format!("{}:{}:{slot}", m.id, local.format("%Y-%m-%d"))
        };
        let tx = lock.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let stamp = now();
        tx.execute("UPDATE runs SET status='interrupted',ended=?1 WHERE status='running' AND started<?1-150", [stamp])?;
        tx.execute("DELETE FROM leases WHERE expires<?1", [stamp])?;
        let already: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE occurrence=?1)",
            [&occurrence],
            |r| r.get(0),
        )?;
        if already {
            continue;
        }
        let acquired = tx.execute(
            "INSERT OR IGNORE INTO leases VALUES(?1,?2)",
            rusqlite::params![m.account_key, stamp + 150],
        )? > 0;
        tx.execute("INSERT INTO runs(task,occurrence,started,status,model,effort) VALUES(?1,?2,?3,?4,?5,?6)", rusqlite::params![m.id, occurrence, stamp, if acquired { "running" } else { "skipped-overlap" }, m.model, m.effort])?;
        let row = tx.last_insert_rowid();
        tx.commit()?;
        if !acquired {
            continue;
        }
        jobs.push((m, row));
    }
    // Reserve all due jobs before starting inference, so one slow account cannot
    // cause another account to miss this minute or bypass overlap detection.
    std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = jobs
            .into_iter()
            .map(|(m, row)| {
                scope.spawn(move || -> Result<()> {
                    let cwd = root.join("work").join(&m.id);
                    let result = private_dir(&cwd)
                        .and_then(|_| auth_check(&m, &cwd))
                        .and_then(|_| invoke(&m, &m.args, &cwd, 120))
                        .and_then(|bytes| {
                            ensure!(response_ok(&m.provider, &bytes), "模型没有返回成功完成事件");
                            Ok(())
                        });
                    let status = match result {
                        Ok(()) => "success",
                        Err(ref e) if e.to_string().contains("超时") => "timeout",
                        Err(_) => "failed",
                    };
                    let db = rusqlite::Connection::open(root.join("runner.sqlite"))?;
                    db.busy_timeout(Duration::from_secs(3))?;
                    db.execute(
                        "UPDATE runs SET status=?1,ended=?2 WHERE id=?3",
                        rusqlite::params![status, now(), row],
                    )?;
                    db.execute("DELETE FROM leases WHERE account=?1", [m.account_key])?;
                    Ok(())
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("唤醒执行线程中断"))??;
        }
        Ok(())
    })?;
    Ok(())
}
pub fn history(root: &Path, id: &str) -> Result<Vec<Value>> {
    if !root.join("runner.sqlite").exists() {
        return Ok(vec![]);
    }
    let db = rusqlite::Connection::open_with_flags(
        root.join("runner.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt = db.prepare("SELECT started,ended,status,model,effort FROM runs WHERE task=?1 ORDER BY id DESC LIMIT 30")?;
    Ok(stmt.query_map([id], |r| Ok(json!({"startedAt":r.get::<_,i64>(0)?,"endedAt":r.get::<_,Option<i64>>(1)?,"status":r.get::<_,String>(2)?,"model":r.get::<_,String>(3)?,"effort":r.get::<_,String>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)
}
