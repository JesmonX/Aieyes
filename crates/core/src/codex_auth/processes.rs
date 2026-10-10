//! Credential operations use home ownership; switching closes this user's Codex/ChatGPT trees.
#[cfg(unix)]
use super::process_auth::{self, Usage};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub pid: u32,
    pub fingerprint: String,
    pub name: String,
    pub can_close: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
}

#[cfg(unix)]
struct Observation {
    pid: u32,
    parent_pid: u32,
    fingerprint: String,
    name: String,
    executable: String,
    home: Option<std::path::PathBuf>,
    auth_usage: Usage,
}

#[cfg(unix)]
fn environment_home(bytes: &[u8]) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let value = |key: &[u8]| {
        bytes
            .split(|b| *b == 0)
            .find_map(|part| part.strip_prefix(key))
            .filter(|value| !value.is_empty())
            .map(|value| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(value)))
    };
    let home = value(b"CODEX_HOME=").or_else(|| value(b"HOME=").map(|p| p.join(".codex")))?;
    // Relative homes require the launch working directory, which may since have changed.
    home.is_absolute()
        .then(|| std::fs::canonicalize(home).ok())
        .flatten()
}

#[cfg(all(test, unix))]
fn macos_environment(bytes: &[u8]) -> Option<&[u8]> {
    macos_launch(bytes).map(|(_, env)| env)
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn macos_launch(bytes: &[u8]) -> Option<(Vec<String>, &[u8])> {
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    if !(1..=65536).contains(&argc) {
        return None;
    }
    let mut cursor = 4 + bytes.get(4..)?.iter().position(|b| *b == 0)?;
    while *bytes.get(cursor)? == 0 {
        cursor += 1;
    }
    let mut args = Vec::new();
    for _ in 0..argc {
        let end = cursor + bytes.get(cursor..)?.iter().position(|b| *b == 0)?;
        args.push(
            std::str::from_utf8(bytes.get(cursor..end)?)
                .ok()?
                .to_owned(),
        );
        cursor = end + 1;
    }
    let env = bytes.get(cursor..)?;
    env.last().is_some_and(|b| *b == 0).then_some((args, env))
}

#[cfg(target_os = "macos")]
fn process_launch(pid: u32) -> Option<(Vec<String>, Vec<u8>)> {
    let mut limit: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&limit);
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut limit as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || !(1..=16 * 1024 * 1024).contains(&limit)
    {
        return None;
    }
    let mut bytes = vec![0u8; limit as usize];
    size = bytes.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    bytes.truncate(size);
    let (args, env) = macos_launch(&bytes)?;
    Some((args, env.to_vec()))
}

#[cfg(target_os = "linux")]
fn process_launch(pid: u32) -> Option<(Vec<String>, Vec<u8>)> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args = bytes
        .strip_suffix(&[0])?
        .split(|b| *b == 0)
        .map(|arg| std::str::from_utf8(arg).ok().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some((args, std::fs::read(format!("/proc/{pid}/environ")).ok()?))
}

#[cfg(any(target_os = "linux", test))]
fn start_from_ticks(
    stat: &str,
    ticks_per_second: u64,
    boot_elapsed: Duration,
    now: std::time::SystemTime,
) -> Option<std::time::SystemTime> {
    if ticks_per_second == 0 {
        return None;
    }
    // Field 22 follows 19 fields after comm, whose name can contain spaces and ')'.
    let ticks: u64 = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()?;
    let started =
        Duration::from_secs(ticks / ticks_per_second).checked_add(Duration::from_nanos(
            (ticks % ticks_per_second).checked_mul(1_000_000_000)? / ticks_per_second,
        ))?;
    now.checked_sub(boot_elapsed.checked_sub(started)?)
}

#[cfg(target_os = "linux")]
fn process_started(pid: u32) -> Option<std::time::SystemTime> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let mut boot: libc::timespec = unsafe { std::mem::zeroed() };
    if ticks <= 0 || unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut boot) } != 0 {
        return None;
    }
    // ps lstart loses fractional seconds; combining rounded boot/start times can
    // make a pre-launch profile appear newer than the process on Linux.
    start_from_ticks(
        &stat,
        ticks as u64,
        Duration::new(boot.tv_sec.try_into().ok()?, boot.tv_nsec.try_into().ok()?),
        std::time::SystemTime::now(),
    )
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn process_launch(_pid: u32) -> Option<(Vec<String>, Vec<u8>)> {
    None
}

#[cfg(unix)]
fn classify(root: &Path, observations: &[Observation], macos: bool) -> Vec<Running> {
    let mut result = Vec::new();
    for p in observations {
        if p.home.as_deref().is_some_and(|home| home != root) {
            continue;
        }
        let belongs = p.home.as_deref() == Some(root);
        let known_child = p.name == "codex-code-mode-host"
            && observations.iter().any(|parent| {
                parent.pid == p.parent_pid
                    && parent.name == "codex"
                    && parent.home.as_deref() == Some(root)
            });
        let reason = if !belongs {
            Some("无法读取此进程的 Codex 目录，请手动关闭后重新检查")
        } else if macos && p.name != "codex" && !known_child {
            Some("无法确认此辅助进程与 Codex 的父子关系，请手动关闭后重新检查")
        } else {
            None
        };
        result.push(Running {
            pid: p.pid,
            fingerprint: p.fingerprint.clone(),
            name: if p.name == "codex" {
                "Codex".into()
            } else {
                p.name.clone()
            },
            can_close: reason.is_none(),
            parent_pid: Some(p.parent_pid),
            blocking_reason: reason.map(str::to_owned),
        });
    }
    result.sort_by_key(|p| p.pid);
    result
}

#[cfg(unix)]
fn process_finished(pid: u32) -> bool {
    // Signal 0 checks existence without delivering a signal. Recheck zombies too:
    // their environment is already gone even if the earlier ps snapshot said running.
    let gone = || unsafe { libc::kill(pid as i32, 0) } != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    if gone() {
        return true;
    }
    let mut command = Command::new("ps");
    command.args(["-p", &pid.to_string(), "-o", "stat="]);
    crate::process::run(command, vec![], Duration::from_secs(3))
        .ok()
        .is_some_and(|bytes| String::from_utf8_lossy(&bytes).trim().starts_with('Z'))
        || gone()
}

#[cfg(unix)]
pub fn list(root: &Path) -> Result<Vec<Running>> {
    Ok(classify(root, &observe(true)?, cfg!(target_os = "macos")))
}

#[cfg(unix)]
fn observe(with_home: bool) -> Result<Vec<Observation>> {
    let mut cmd = Command::new("ps");
    cmd.env("LC_ALL", "C");
    cmd.args(["-axo", "pid=,ppid=,uid=,lstart=,stat=,comm="]);
    let output = crate::process::run(cmd, vec![], Duration::from_secs(10))?;
    let mut observations = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 10 || fields[8].starts_with('Z') {
            continue;
        }
        let (Ok(pid), Ok(parent_pid)) = (fields[0].parse::<u32>(), fields[1].parse::<u32>()) else {
            continue;
        };
        if pid == std::process::id()
            || fields[2].parse::<u32>().ok() != Some(unsafe { libc::geteuid() })
        {
            continue;
        }
        let executable = fields[9..].join(" ");
        // Linux comm truncates names to 15 bytes (including codex-code-mode-host).
        #[cfg(target_os = "linux")]
        let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|path| path.into_os_string().into_string().ok())
            .unwrap_or(executable);
        let name = Path::new(&executable)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_lowercase();
        if with_home && name != "codex" && !name.starts_with("codex-") {
            continue;
        }
        let mut home = None;
        let mut auth_usage = Usage::Account;
        if (with_home || name == "codex")
            && let Some((args, env)) = process_launch(pid)
        {
            if with_home {
                home = environment_home(&env);
            }
            if name == "codex" && !with_home {
                #[cfg(target_os = "linux")]
                let started = process_started(pid);
                #[cfg(not(target_os = "linux"))]
                let started = {
                    use chrono::TimeZone;
                    chrono::NaiveDateTime::parse_from_str(
                        &fields[3..8].join(" "),
                        "%a %b %e %H:%M:%S %Y",
                    )
                    .ok()
                    .and_then(|time| chrono::Local.from_local_datetime(&time).single())
                    .map(std::time::SystemTime::from)
                };
                auth_usage = process_auth::inspect(&args, &env, started);
            }
        }
        if with_home && home.is_none() && process_finished(pid) {
            continue;
        }
        // Exclude PPID: a confirmed helper can be reparented when its parent exits normally.
        let fingerprint = crate::models::hash(&format!(
            "{pid}:{}:{executable}:{home:?}:{}",
            fields[2..8].join(" "),
            auth_usage as u8
        ));
        observations.push(Observation {
            pid,
            parent_pid,
            fingerprint,
            name,
            executable,
            home,
            auth_usage,
        });
    }
    Ok(observations)
}

#[cfg(unix)]
fn independent_processes(observations: &[Observation]) -> std::collections::HashSet<u32> {
    let mut protected: std::collections::HashSet<_> = observations
        .iter()
        .filter(|p| p.auth_usage == Usage::IndependentApi)
        .map(|p| p.pid)
        .collect();
    loop {
        let before = protected.len();
        for p in observations {
            // A nested Codex invocation has its own configuration, not necessarily
            // its parent's profile. Helpers inherit the owning instance's scope.
            if p.name != "codex" && protected.contains(&p.parent_pid) {
                protected.insert(p.pid);
            }
        }
        if before == protected.len() {
            return protected;
        }
    }
}

#[cfg(unix)]
fn switch_targets(observations: &[Observation], confirmed: &[Running]) -> Vec<Running> {
    switch_review(observations, confirmed).0
}

#[cfg(unix)]
fn switch_review(
    observations: &[Observation],
    confirmed: &[Running],
) -> (Vec<Running>, Vec<Running>) {
    use std::collections::HashSet;
    // Exact bundle components also find orphaned Electron/XPC/CUA helpers. Do not
    // match arbitrary paths merely containing "codex" or "chatgpt".
    let bundled = |p: &Observation| {
        let path = p.executable.to_lowercase();
        path.contains("/codex.app/contents/") || path.contains("/chatgpt.app/contents/")
    };
    let known = |p: &Observation| {
        matches!(
            p.name.as_str(),
            "codex" | "chatgpt" | "codex-code-mode-host"
        ) || bundled(p)
    };
    let independent = independent_processes(observations);
    let mut protected_parents = HashSet::new();
    for p in observations.iter().filter(|p| independent.contains(&p.pid)) {
        let mut parent = p.parent_pid;
        while let Some(p) = observations.iter().find(|p| p.pid == parent) {
            if !protected_parents.insert(p.pid) {
                break;
            }
            parent = p.parent_pid;
        }
    }
    let mut selected: HashSet<u32> = observations
        .iter()
        .filter(|p| {
            !independent.contains(&p.pid)
                && (known(p)
                    || confirmed
                        .iter()
                        .any(|old| old.pid == p.pid && old.fingerprint == p.fingerprint))
        })
        .map(|p| p.pid)
        .collect();
    loop {
        let before = selected.len();
        for p in observations {
            if !independent.contains(&p.pid) && selected.contains(&p.parent_pid) {
                selected.insert(p.pid);
            }
        }
        if selected.len() == before {
            break;
        }
    }
    let mut preserved = Vec::new();
    let mut rows: Vec<_> = observations
        .iter()
        .filter(|p| {
            independent.contains(&p.pid)
                || selected.contains(&p.pid)
                || p.name.starts_with("codex-")
                || p.name.starts_with("chatgpt-")
        })
        .filter_map(|p| {
            let reason = if independent.contains(&p.pid) {
                Some("已确认使用独立 API 凭据，将保留")
            } else if protected_parents.contains(&p.pid) {
                Some("此进程仍承载独立 API 实例，请先分离或手动关闭后重新检查")
            } else if p.auth_usage == Usage::UnknownProfile {
                Some("无法确认此进程的 profile 认证方式，请手动处理后重新检查")
            } else if !selected.contains(&p.pid) {
                Some("无法确认辅助进程归属，请手动关闭后重新检查")
            } else {
                None
            };
            let row = Running {
                pid: p.pid,
                parent_pid: Some(p.parent_pid),
                fingerprint: p.fingerprint.clone(),
                name: match p.name.as_str() {
                    "chatgpt" => "ChatGPT 应用".into(),
                    "codex" if bundled(p) => "Codex（应用进程）".into(),
                    "codex" => "Codex".into(),
                    _ => p.name.clone(),
                },
                can_close: reason.is_none(),
                blocking_reason: reason.map(str::to_owned),
            };
            if independent.contains(&p.pid) {
                preserved.push(row);
                None
            } else {
                Some(row)
            }
        })
        .collect();
    rows.sort_by_key(|p| p.pid);
    preserved.sort_by_key(|p| p.pid);
    (rows, preserved)
}

#[cfg(test)]
thread_local! {
    static ISOLATE_SWITCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn isolated_switch_test() -> impl Drop {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            ISOLATE_SWITCH.set(self.0);
        }
    }
    Reset(ISOLATE_SWITCH.replace(true))
}

pub fn list_for_switch() -> Result<Vec<Running>> {
    read_for_switch(&[])
}

pub fn review_for_switch() -> Result<(Vec<Running>, Vec<Running>)> {
    #[cfg(test)]
    if ISOLATE_SWITCH.get() {
        return Ok((vec![], vec![]));
    }
    #[cfg(unix)]
    {
        Ok(switch_review(&observe(false)?, &[]))
    }
    #[cfg(windows)]
    {
        Ok((windows_list(true)?, vec![]))
    }
}

fn read_for_switch(confirmed: &[Running]) -> Result<Vec<Running>> {
    // Compile-time-only isolation: coordinator tests must never signal host apps.
    #[cfg(test)]
    if ISOLATE_SWITCH.get() {
        return Ok(vec![]);
    }
    #[cfg(unix)]
    {
        Ok(switch_targets(&observe(false)?, confirmed))
    }
    #[cfg(windows)]
    {
        let _ = confirmed;
        windows_list(true)
    }
}

#[cfg(windows)]
pub fn list(_root: &Path) -> Result<Vec<Running>> {
    windows_list(false)
}

#[cfg(windows)]
fn windows_list(include_apps: bool) -> Result<Vec<Running>> {
    let mut cmd = Command::new("powershell.exe");
    let filter = if include_apps {
        "Name = 'codex.exe' OR Name = 'ChatGPT.exe' OR Name LIKE 'codex-%' OR Name LIKE 'ChatGPT %' OR Name LIKE 'Codex %'"
    } else {
        "Name = 'codex.exe'"
    };
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &format!("@(Get-CimInstance Win32_Process -Filter \"{filter}\" | Select-Object ProcessId,CreationDate,Name) | ConvertTo-Json -Compress")]);
    let bytes = crate::process::run(cmd, vec![], Duration::from_secs(10))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!([]));
    let rows = value.as_array().cloned().unwrap_or_else(|| {
        if value.is_object() {
            vec![value]
        } else {
            vec![]
        }
    });
    Ok(rows
        .iter()
        .filter_map(|v| {
            v["ProcessId"].as_u64().map(|pid| Running {
                pid: pid as u32,
                fingerprint: crate::models::hash(&v.to_string()),
                name: v["Name"].as_str().unwrap_or("Codex / ChatGPT").into(),
                can_close: false,
                parent_pid: None,
                blocking_reason: Some("请关闭所有 Codex 和 ChatGPT 进程及应用后重新检查".into()),
            })
        })
        .collect())
}

fn unchanged(actual: &[Running], expected: &[Running]) -> Result<()> {
    ensure!(
        actual.iter().all(|p| p.can_close),
        "进程认证方式或依赖已变化，账号未切换，请重新检查"
    );
    ensure!(
        actual.iter().all(|p| expected
            .iter()
            .any(|old| p.pid == old.pid && p.fingerprint == old.fingerprint)),
        "Codex / ChatGPT 已重新启动或运行进程已变化，账号未切换，请重新检查"
    );
    Ok(())
}

fn close_with(
    expected: &[Running],
    mut read: impl FnMut() -> Result<Vec<Running>>,
    mut signal: impl FnMut(u32) -> Result<()>,
    timeout: Duration,
) -> Result<()> {
    ensure!(
        expected.iter().all(|p| p.can_close),
        "无法确认部分进程归属，请手动关闭 Codex 和 ChatGPT 后重试"
    );
    unchanged(&read()?, expected)?;
    let mut ordered = expected.to_vec();
    ordered.sort_by_key(|p| (process_depth(p, expected), p.pid));
    for p in ordered {
        let current = read()?;
        unchanged(&current, expected)?;
        if current.iter().any(|actual| actual.pid == p.pid) {
            signal(p.pid)?;
        }
    }
    let until = std::time::Instant::now() + timeout;
    loop {
        let current = read()?;
        unchanged(&current, expected)?;
        if current.is_empty() {
            return Ok(());
        }
        ensure!(
            std::time::Instant::now() < until,
            "Codex / ChatGPT 未退出，账号未切换，请手动关闭后重新检查"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn process_depth(p: &Running, rows: &[Running]) -> usize {
    let mut parent = p.parent_pid;
    let mut depth = 0;
    while let Some(row) = rows.iter().find(|r| Some(r.pid) == parent) {
        depth += 1;
        if depth >= rows.len() {
            break;
        }
        parent = row.parent_pid;
    }
    depth
}

pub fn close_for_switch(expected: &[Running]) -> Result<()> {
    close_with(
        expected,
        || read_for_switch(expected),
        terminate,
        Duration::from_secs(10),
    )
}

fn terminate(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
            let error = std::io::Error::last_os_error();
            ensure!(
                error.raw_os_error() == Some(libc::ESRCH),
                "无法请求 Codex / ChatGPT 退出，请手动关闭"
            );
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let _ = pid;
        anyhow::bail!("请手动关闭 Codex 和 ChatGPT 后重试");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[test]
    fn linux_start_ticks_preserve_fractional_seconds_and_reject_invalid_data() {
        let stat = format!("123 (fixture ) with spaces) S {}1250", "0 ".repeat(18));
        let now = std::time::SystemTime::UNIX_EPOCH + Duration::from_millis(1_000_100);
        assert_eq!(
            start_from_ticks(&stat, 100, Duration::from_millis(13_250), now),
            Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_millis(999_350))
        );
        assert!(start_from_ticks(&stat, 0, Duration::from_secs(20), now).is_none());
        assert!(start_from_ticks(&stat, 100, Duration::from_secs(12), now).is_none());
        assert!(start_from_ticks("invalid", 100, Duration::from_secs(20), now).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn independent_api_tree_is_preserved_and_shared_parent_is_blocked() {
        let make = |pid, parent_pid, name: &str, auth_usage| Observation {
            pid,
            parent_pid,
            name: name.into(),
            executable: name.into(),
            home: None,
            fingerprint: format!("start-{pid}"),
            auth_usage,
        };
        let mut rows = vec![
            make(10, 1, "codex", Usage::IndependentApi),
            make(11, 10, "codex-code-mode-host", Usage::Account),
            make(12, 11, "node", Usage::Account),
            make(13, 12, "codex", Usage::Account),
            make(20, 1, "codex", Usage::Account),
            make(30, 1, "chatgpt", Usage::Account),
        ];
        let (close, preserved) = switch_review(&rows, &[]);
        assert_eq!(
            close.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [13, 20, 30]
        );
        assert!(close.iter().all(|p| p.can_close));
        assert_eq!(
            preserved.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [10, 11, 12]
        );
        rows[0].parent_pid = 30;
        let (close, _) = switch_review(&rows, &close);
        assert!(!close.iter().find(|p| p.pid == 30).unwrap().can_close);
        assert!(
            close_with(
                &close,
                || panic!("must not read"),
                |_| panic!("must not signal"),
                Duration::ZERO
            )
            .is_err()
        );
        // An API child appearing after confirmation must also prevent its parent
        // from being signalled, even if the parent's PID/fingerprint is unchanged.
        let expected = [running(30, Some(1))];
        let mut current = expected[0].clone();
        current.can_close = false;
        assert!(
            close_with(
                &expected,
                || Ok(vec![current.clone()]),
                |_| panic!("must not signal"),
                Duration::ZERO
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_profile_switch_preserves_api_fixture_and_closes_login_fixture() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().canonicalize().unwrap();
        std::fs::write(
            home.join("fixture.c"),
            "#include <stdio.h>\n#include <unistd.h>\nint main(void) { puts(\"ready\"); fflush(stdout); sleep(30); return 0; }\n",
        )
        .unwrap();
        let executable = home.join("codex");
        assert!(
            Command::new("cc")
                .arg(home.join("fixture.c"))
                .arg("-o")
                .arg(&executable)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(home.join("api.config.toml"), "model_provider='gateway'\n[model_providers.gateway]\nbase_url='https://fixture.invalid/v1'\nexperimental_bearer_token='FIXTURE_SECRET_NEVER_EXPOSE'\nrequires_openai_auth=false\n").unwrap();
        std::fs::write(home.join("login.config.toml"), "model='fixture-model'\n").unwrap();
        let mut api = Command::new(&executable)
            .args(["--profile", "api"])
            .env("CODEX_HOME", &home)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut login = Command::new(&executable)
            .args(["--profile=login"])
            .env("CODEX_HOME", &home)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let api_id = api.id();
        let login_id = login.id();
        let result = (|| -> Result<()> {
            use std::io::BufRead;
            for child in [&mut api, &mut login] {
                let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
                let mut ready = String::new();
                output.read_line(&mut ready)?;
                ensure!(ready == "ready\n", "fixture did not finish starting");
            }
            let (close, preserved) = review_for_switch()?;
            ensure!(preserved.iter().any(|p| p.pid == api_id));
            ensure!(!close.iter().any(|p| p.pid == api_id));
            let close: Vec<_> = close.into_iter().filter(|p| p.pid == login_id).collect();
            ensure!(close.len() == 1 && close[0].can_close);
            let public = serde_json::to_string(&preserved)?;
            ensure!(!public.contains("FIXTURE_SECRET") && !public.contains("fixture.invalid"));
            close_with(
                &close,
                || {
                    Ok(list_for_switch()?
                        .into_iter()
                        .filter(|p| [api_id, login_id].contains(&p.pid))
                        .collect())
                },
                |pid| {
                    ensure!(
                        pid == login_id,
                        "only the disposable login fixture may be signalled"
                    );
                    terminate(pid)
                },
                Duration::from_secs(10),
            )?;
            ensure!(
                api.try_wait()?.is_none(),
                "API instance must remain running"
            );
            Ok(())
        })();
        for child in [&mut api, &mut login] {
            let _ = child.kill();
            let _ = child.wait();
        }
        result.unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn switch_finds_apps_other_homes_helpers_and_retains_orphans() {
        let make = |pid, parent_pid, executable: &str, home: &str| Observation {
            pid,
            parent_pid,
            executable: executable.into(),
            name: Path::new(executable)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_lowercase(),
            fingerprint: format!("start-{pid}"),
            home: Some(home.into()),
            auth_usage: Usage::Account,
        };
        let mut observations = vec![
            make(
                90,
                1,
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
                "/other",
            ),
            make(
                50,
                90,
                "/Applications/ChatGPT.app/Contents/Resources/bin/codex",
                "/other",
            ),
            make(20, 50, "/bin/node", "/other"),
            make(10, 20, "/bin/helper", "/other"),
            make(100, 1, "/bin/codex", "/another-home"),
            make(101, 1, "/bin/codex-code-mode-host", "/unknown-home"),
            make(
                110,
                1,
                "/Applications/Codex.app/Contents/Frameworks/Helper",
                "/none",
            ),
            make(
                120,
                1,
                "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node",
                "/none",
            ),
            make(130, 1, "/Users/me/chatgpt-project/bin/node", "/none"),
            make(
                140,
                1,
                "/Applications/MyCodex.app/Contents/MacOS/unrelated",
                "/none",
            ),
            make(150, 1, "/bin/codex-unknown", "/none"),
        ];
        let rows = switch_targets(&observations, &[]);
        assert_eq!(
            rows.iter()
                .map(|p| (p.pid, p.can_close))
                .collect::<Vec<_>>(),
            [
                (10, true),
                (20, true),
                (50, true),
                (90, true),
                (100, true),
                (101, true),
                (110, true),
                (120, true),
                (150, false)
            ]
        );
        assert_eq!(
            rows.iter().find(|p| p.pid == 90).unwrap().name,
            "ChatGPT 应用"
        );
        observations.retain(|p| [10, 20].contains(&p.pid));
        observations.iter_mut().for_each(|p| p.parent_pid = 1);
        assert!(switch_targets(&observations, &[]).is_empty());
        assert_eq!(switch_targets(&observations, &rows).len(), 2);
        observations[0].fingerprint = "reused-pid".into();
        assert_eq!(switch_targets(&observations, &rows).len(), 1);
    }

    #[test]
    fn application_tree_closes_parent_before_every_descendant() {
        let rows = vec![
            running(10, Some(20)),
            running(20, Some(50)),
            running(50, Some(90)),
            running(90, Some(1)),
        ];
        let state = RefCell::new(rows.clone());
        let mut order = vec![];
        close_with(
            &rows,
            || Ok(state.borrow().clone()),
            |pid| {
                order.push(pid);
                state.borrow_mut().retain(|p| p.pid != pid);
                Ok(())
            },
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(order, [90, 50, 20, 10]);
    }

    #[cfg(unix)]
    #[test]
    fn native_switch_scope_finds_chatgpt_and_codex_without_readable_home() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("ChatGPT.app/Contents/MacOS/ChatGPT");
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::copy("/bin/sleep", &app).unwrap();
        let cli = dir.path().join("codex");
        std::fs::copy("/bin/sleep", &cli).unwrap();
        let mut children = [app, cli].map(|exe| {
            Command::new(exe)
                .arg("30")
                .env("CODEX_HOME", "/nonexistent-fixture-home")
                .spawn()
                .unwrap()
        });
        let ids = children.each_ref().map(|child| child.id());
        // Restrict reads AND signals to our own disposable children, even though
        // enumeration uses the production all-app scope on this development host.
        let read = || -> Result<Vec<Running>> {
            Ok(list_for_switch()?
                .into_iter()
                .filter(|p| ids.contains(&p.pid))
                .collect())
        };
        let result = (|| -> Result<()> {
            let rows = read()?;
            ensure!(
                rows.len() == 2 && rows.iter().all(|p| p.can_close),
                "app/CLI fixtures missing"
            );
            ensure!(
                rows.iter().any(|p| p.name == "ChatGPT 应用"),
                "app not identified"
            );
            close_with(
                &rows,
                read,
                |pid| {
                    ensure!(ids.contains(&pid));
                    terminate(pid)
                },
                Duration::from_secs(10),
            )
        })();
        for child in &mut children {
            let _ = child.kill();
            let _ = child.wait();
        }
        result.unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn native_enumeration_closes_only_the_disposable_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let executable = root.join("codex");
        std::fs::copy("/bin/sleep", &executable).unwrap();
        let mut child = Command::new(executable)
            .arg("30")
            .env("CODEX_HOME", &root)
            .spawn()
            .unwrap();
        let read = || -> Result<Vec<Running>> {
            Ok(list(&root)?
                .into_iter()
                .filter(|p| p.pid == child.id())
                .collect())
        };
        let result = (|| -> Result<()> {
            let rows = read()?;
            ensure!(
                rows.len() == 1 && rows[0].pid == child.id() && rows[0].can_close,
                "fixture ownership was not recognized"
            );
            close_with(&rows, read, terminate, Duration::from_secs(10))
        })();
        let _ = child.kill();
        let _ = child.wait();
        result.unwrap();
    }
    fn running(pid: u32, parent: Option<u32>) -> Running {
        Running {
            pid,
            parent_pid: parent,
            fingerprint: format!("start-{pid}"),
            name: "Codex".into(),
            can_close: true,
            blocking_reason: None,
        }
    }
    #[test]
    fn natural_exit_and_reparenting_do_not_fail_close() {
        let parent = running(10, None);
        let child = running(11, Some(10));
        let state = RefCell::new(vec![parent.clone(), child.clone()]);
        let mut signalled = vec![];
        close_with(
            &[parent, child],
            || Ok(state.borrow().clone()),
            |pid| {
                signalled.push(pid);
                let mut rows = state.borrow_mut();
                rows.retain(|p| p.pid != pid);
                for p in rows.iter_mut() {
                    p.parent_pid = Some(1);
                }
                Ok(())
            },
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(signalled, [10, 11]);
        close_with(
            &[running(10, None)],
            || Ok(vec![]),
            |_| panic!("already exited"),
            Duration::ZERO,
        )
        .unwrap();
    }
    #[test]
    fn unknown_reused_and_restarted_processes_are_never_signalled() {
        let original = running(10, None);
        let mut blocked = original.clone();
        blocked.can_close = false;
        assert!(
            close_with(
                &[blocked],
                || Ok(vec![]),
                |_| panic!("unknown"),
                Duration::ZERO
            )
            .is_err()
        );
        let mut reused = original.clone();
        reused.fingerprint = "new-start".into();
        assert!(
            close_with(
                std::slice::from_ref(&original),
                || Ok(vec![reused.clone()]),
                |_| panic!("reused PID"),
                Duration::ZERO
            )
            .is_err()
        );
        let state = RefCell::new(vec![original.clone()]);
        let error = close_with(
            &[original],
            || Ok(state.borrow().clone()),
            |_| {
                *state.borrow_mut() = vec![running(20, None)];
                Ok(())
            },
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("重新启动"));
    }
    #[test]
    fn refusal_to_exit_times_out_and_old_records_decode() {
        let p = running(10, None);
        assert!(
            close_with(
                std::slice::from_ref(&p),
                || Ok(vec![p.clone()]),
                |_| Ok(()),
                Duration::ZERO
            )
            .unwrap_err()
            .to_string()
            .contains("未退出")
        );
        let old: Running = serde_json::from_value(
            serde_json::json!({"pid":1,"fingerprint":"old","name":"Codex","canClose":false}),
        )
        .unwrap();
        assert!(old.parent_pid.is_none() && old.blocking_reason.is_none());
    }
    #[cfg(unix)]
    #[test]
    fn macos_helpers_require_a_verified_parent_and_home() {
        let root = Path::new("/fixture/home");
        let make = |pid: u32, parent_pid, name: &str, home: Option<&str>| Observation {
            pid,
            parent_pid,
            name: name.into(),
            executable: name.into(),
            home: home.map(Into::into),
            auth_usage: Usage::Account,
            fingerprint: pid.to_string(),
        };
        let rows = classify(
            root,
            &[
                make(1, 0, "codex", Some("/fixture/home")),
                make(2, 1, "codex-code-mode-host", Some("/fixture/home")),
                make(3, 99, "codex-code-mode-host", Some("/fixture/home")),
                make(4, 1, "codex-unknown", Some("/fixture/home")),
                make(5, 0, "codex", Some("/other/home")),
                make(6, 0, "codex", Some("/fixture/home/.aieyes/query")),
                make(7, 0, "codex", None),
            ],
            true,
        );
        assert_eq!(
            rows.iter()
                .map(|p| (p.pid, p.can_close))
                .collect::<Vec<_>>(),
            [(1, true), (2, true), (3, false), (4, false), (7, false)]
        );
        assert!(rows[1].parent_pid == Some(1) && rows[2].blocking_reason.is_some());
    }
    #[cfg(unix)]
    #[test]
    fn macos_arguments_are_not_mistaken_for_environment() {
        let mut bytes = 2i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(
            b"/bin/codex\0\0codex\0CODEX_HOME=/argument\0HOME=/actual\0SECRET=never-returned\0",
        );
        let env = macos_environment(&bytes).unwrap();
        assert!(env.starts_with(b"HOME=/actual\0"));
        assert!(macos_environment(&bytes[..bytes.len() - 1]).is_none());
        assert!(macos_environment(&[0; 4]).is_none());
        let d = tempfile::tempdir().unwrap();
        let text = format!("CODEX_HOME={}\0HOME=/wrong\0", d.path().display());
        assert_eq!(
            environment_home(text.as_bytes()),
            Some(d.path().canonicalize().unwrap())
        );
        assert!(environment_home(b"CODEX_HOME=relative\0HOME=/wrong\0").is_none());
    }
}
