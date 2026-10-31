//! Kiro session state, checked against kiro-cli 2.36.0 and Kiro IDE 1.4.62.
//! kiro-cli's default (V2) engine writes `sessions/cli/<id>.json`, plus
//! `<id>.lock` naming the live process; kiro-cli V3 and the IDE write
//! `sessions/<workspace>/sess_<id>/session.json` beside `messages.jsonl`.
//! Only ids, status, parent links and the working directory are read;
//! prompts and replies never leave memory.
use super::Activity;
use crate::session_events::{HEADLESS, INTERACTIVE, INTERNAL};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Directory entries visited per listing; Kiro never prunes old sessions.
const MAX_ENTRIES: usize = 4096;

/// Kiro's home: `$KIRO_HOME` when set, else `~/.kiro`.
pub(crate) fn home() -> PathBuf {
    std::env::var_os("KIRO_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::store::home().join(".kiro"))
}

/// Working directory of Aieyes' own `kiro-cli /usage` check. Sessions started
/// there are Aieyes' and never listed.
pub(crate) fn probe_dir() -> PathBuf {
    crate::store::home().join(".aieyes/kiro-probe")
}

/// The session root of a source path naming the Kiro home, its `sessions`
/// directory or `sessions/cli`.
pub(crate) fn directory(path: &str) -> Option<PathBuf> {
    let root = crate::store::expand(path);
    let root = if root.ends_with("sessions/cli") {
        root.parent()?.to_owned()
    } else if root.file_name().is_some_and(|n| n == "sessions") {
        root
    } else {
        root.join("sessions")
    };
    root.is_dir().then_some(root)
}

fn modified_ms(path: &Path) -> Option<u64> {
    let at = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(at.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

fn entries(dir: &Path) -> impl Iterator<Item = std::fs::DirEntry> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .take(MAX_ENTRIES)
}

/// Every session metadata file with its last activity in ms: V2
/// `cli/<id>.json`, and V3 / IDE `session.json`, whose messages are appended
/// to the `messages.jsonl` beside it.
pub(crate) fn files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = vec![];
    for entry in entries(&dir.join("cli")) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Some(at) = modified_ms(&path)
        {
            out.push((path, at));
        }
    }
    let mut visited = 0;
    for workspace in entries(dir) {
        if workspace.file_name() == "cli" || !workspace.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for session in entries(&workspace.path()) {
            visited += 1;
            if visited > MAX_ENTRIES {
                return out;
            }
            if !session.file_name().to_string_lossy().starts_with("sess_") {
                continue;
            }
            let path = session.path().join("session.json");
            let Some(at) = modified_ms(&path) else {
                continue;
            };
            let at = modified_ms(&session.path().join("messages.jsonl")).map_or(at, |m| m.max(at));
            out.push((path, at));
        }
    }
    out
}

fn recent(dir: &Path, now: u64) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<_> = files(dir)
        .into_iter()
        .filter(|(_, at)| now.saturating_sub(at / 1000) < 86400)
        .collect();
    files.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    files.truncate(256);
    files
}

/// Changes when a recent session is written, and when a V2 session gains or
/// loses its lock (its process starts or exits).
pub(crate) fn fingerprint(dir: &Path) -> String {
    let files = recent(dir, crate::models::now() as u64);
    let mut locks: Vec<_> = entries(&dir.join("cli"))
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().ends_with(".lock"))
        .collect();
    locks.sort();
    format!(
        "kiro:{}:{}",
        files.len(),
        crate::models::hash(&format!("{files:?}{locks:?}"))
    )
}

fn v3(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "session.json")
}

fn id(v: &Value, path: &Path) -> Option<String> {
    ["session_id", "id"]
        .iter()
        .find_map(|k| v[*k].as_str().filter(|s| !s.is_empty() && s.len() <= 256))
        .map(str::to_owned)
        .or_else(|| {
            let name = if v3(path) {
                path.parent()?.file_name()?
            } else {
                path.file_stem()?
            };
            Some(name.to_string_lossy().into_owned())
        })
}

pub(crate) fn session_id_in_directory(path: &Path, directory: &Path) -> Option<String> {
    let v = crate::task_history::read(path).ok()?;
    if canonical(Path::new(cwd(&v)?)) != canonical(directory) {
        return None;
    }
    id(&v, path)
}

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// `<id>.lock` beside a V2 session while its process runs: `{pid, started_at}`.
fn lock(path: &Path) -> Option<(u32, Option<i64>)> {
    let path = path.with_extension("lock");
    if std::fs::metadata(&path).ok()?.len() > 4096 {
        return None;
    }
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let pid = u32::try_from(v["pid"].as_u64()?).ok().filter(|p| *p > 0)?;
    let started = v["started_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp());
    Some((pid, started))
}

/// One process from a session's lock holder upward. Only whether
/// `AIEYES_INTERNAL` / `KIRO_API_KEY` are set is kept, never their values.
pub(crate) struct Link {
    pub(crate) argv: Vec<String>,
    pub(crate) internal: bool,
    pub(crate) api_key: bool,
}

/// Aieyes' own run first, then the first kiro-cli argv rule (`chat
/// --no-interactive`), then the TUI front end (interactive), then
/// `KIRO_API_KEY`, which kiro-cli documents for headless and CI use.
pub(crate) fn chain_verdict(chain: &[Link]) -> Option<&'static str> {
    if chain.iter().any(|l| l.internal) {
        return Some(INTERNAL);
    }
    if let Some(verdict) = chain
        .iter()
        .find_map(|l| crate::session_events::headless_argv("kiro", &l.argv))
    {
        return Some(verdict);
    }
    let tui = |argv: &[String]| {
        argv.iter().take(4).any(|a| {
            let a = a.replace('\\', "/");
            a.ends_with("/tui.js") && a.contains("kiro")
        })
    };
    if chain.iter().any(|l| tui(&l.argv)) {
        return Some(INTERACTIVE);
    }
    chain.iter().any(|l| l.api_key).then_some(HEADLESS)
}

/// The class of the live process holding a V2 session lock (kiro-cli's `acp`
/// worker, whose parent is `kiro-cli chat` or the TUI).
fn live_verdict(pid: u32, started: Option<i64>) -> Option<&'static str> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    let mut pid = Pid::from_u32(pid);
    let mut chain = vec![];
    for depth in 0..6 {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .with_environ(UpdateKind::Always),
        );
        let Some(process) = system.process(pid) else {
            break;
        };
        // A stale lock whose pid now belongs to another process says nothing.
        if depth == 0 && started.is_some_and(|s| (process.start_time() as i64 - s).abs() > 120) {
            return None;
        }
        let set = |key: &str| {
            process.environ().iter().any(|e| {
                e.to_str()
                    .and_then(|e| e.strip_prefix(key)?.strip_prefix('='))
                    .is_some_and(|v| !v.is_empty())
            })
        };
        chain.push(Link {
            argv: process
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            internal: set("AIEYES_INTERNAL"),
            api_key: set("KIRO_API_KEY"),
        });
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    chain_verdict(&chain)
}

fn cwd(v: &Value) -> Option<&str> {
    v["cwd"]
        .as_str()
        .or_else(|| v["workspacePaths"][0].as_str())
        .or_else(|| v["rootPaths"][0].as_str())
}

/// V2: the last turn of `conversation_metadata.user_turn_metadatas`.
fn v2_kind(v: &Value) -> &'static str {
    let metadata = &v["session_state"]["conversation_metadata"];
    if !metadata["user_turn_start_request"].is_null() {
        return "start";
    }
    let Some(last) = metadata["user_turn_metadatas"]
        .as_array()
        .and_then(|t| t.last())
    else {
        return "unknown";
    };
    if last["result"].get("Err").is_some() {
        return "failed";
    }
    match last["end_reason"].as_str() {
        Some("UserTurnEnd") => "complete",
        Some("ToolUseRejected" | "DidNotRun") => "interrupted",
        _ => "unknown",
    }
}

/// V3 / IDE `status`.
fn v3_kind(v: &Value) -> &'static str {
    match v["status"].as_str() {
        Some("in_progress" | "provisioning") => "start",
        Some("waiting_on_user") => "PermissionRequest",
        Some("completed" | "idle") => "complete",
        Some("failed") => "failed",
        _ => "unknown",
    }
}

/// Recent Kiro sessions. Aieyes' own `/usage` check and V2 sessions whose live
/// process is headless or Aieyes' are skipped; `verdicts` keeps that decision
/// after the process exits. A session never seen running carries no signal
/// and counts. V3 subagents fold into their parent session.
pub(crate) fn read(dir: &Path, now: u64, verdicts: &mut HashMap<String, bool>) -> Vec<Activity> {
    let probe = canonical(&probe_dir());
    let mut seen = HashMap::new();
    let mut out = vec![];
    for (path, at) in recent(dir, now) {
        let Ok(v) = crate::task_history::read(&path) else {
            continue;
        };
        let Some(id) = id(&v, &path) else {
            continue;
        };
        let v3 = v3(&path);
        let live = (!v3)
            .then(|| lock(&path))
            .flatten()
            .and_then(|(pid, started)| live_verdict(pid, started));
        let skip = verdicts.get(&id).copied().unwrap_or(false)
            || cwd(&v).is_some_and(|c| canonical(Path::new(c)) == probe)
            || match live {
                Some(verdict) => verdict != INTERACTIVE,
                None => verdicts.get(&id).copied().unwrap_or(false),
            };
        seen.insert(id.clone(), skip);
        if skip {
            continue;
        }
        let (kind, parent) = if v3 {
            let parent = if v["createdReason"] == "subagent" {
                v["parentSessionId"].as_str().unwrap_or("")
            } else {
                ""
            };
            (v3_kind(&v), parent.to_owned())
        } else {
            (v2_kind(&v), String::new())
        };
        out.push((id.clone(), id, kind.into(), at, parent));
    }
    *verdicts = seen;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(path: &Path, v: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, v.to_string()).unwrap();
    }
    fn link(argv: &[&str], internal: bool, api_key: bool) -> Link {
        Link {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            internal,
            api_key,
        }
    }

    #[test]
    fn cli_and_ide_sessions_are_read_and_the_usage_probe_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let turn = |end: &str| json!({"end_reason":end,"metering_usage":[{"value":0.4,"unit":"credit"}],"result":{"Ok":{"content":[{"kind":"text","data":"PRIVATE"}]}}});
        // Shapes written by kiro-cli 2.36.0 (V2) and Kiro IDE 1.4.62 / kiro-cli --v3.
        write(
            &sessions.join("cli/a.json"),
            json!({"session_id":"s1","cwd":"/work","session_created_reason":"subagent",
                "session_state":{"version":"v1","conversation_metadata":{"user_turn_metadatas":[turn("UserTurnEnd")]}}}),
        );
        write(
            &sessions.join("cli/b.json"),
            json!({"session_id":"s2","cwd":"/work","session_state":{"conversation_metadata":{"user_turn_metadatas":[]}}}),
        );
        write(
            &sessions.join("cli/c.json"),
            json!({"session_id":"probe","cwd":probe_dir().to_string_lossy()}),
        );
        write(
            &sessions.join("cli/d.json"),
            json!({"session_id":"s4","cwd":"/work","session_state":{"conversation_metadata":{"user_turn_metadatas":[turn("ToolUseRejected")],"user_turn_start_request":{"id":"r"}}}}),
        );
        write(
            &sessions.join("5d0275c3da257b44/sess_x/session.json"),
            json!({"schemaVersion":"1.0.0","id":"sess_x","title":"PRIVATE","workspacePaths":["/work"],"status":"waiting_on_user"}),
        );
        std::fs::write(
            sessions.join("5d0275c3da257b44/sess_x/messages.jsonl"),
            "{}\n",
        )
        .unwrap();
        write(
            &sessions.join("5d0275c3da257b44/sess_y/session.json"),
            json!({"id":"sess_y","workspacePaths":["/work"],"createdReason":"subagent","parentSessionId":"sess_x","status":"completed"}),
        );
        write(
            &sessions.join("5d0275c3da257b44/sess_z/session.json"),
            json!({"id":"sess_z","workspacePaths":[probe_dir().to_string_lossy()],"title":"/usage"}),
        );
        write(
            &sessions.join("5d0275c3da257b44/other/session.json"),
            json!({"id":"ignored"}),
        );
        let old = sessions.join("cli/old.json");
        write(&old, json!({"session_id":"old"}));
        std::fs::File::options()
            .append(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(90_000))
            .unwrap();

        for path in [temp.path(), &sessions, &sessions.join("cli")] {
            assert_eq!(directory(path.to_str().unwrap()).unwrap(), sessions);
        }
        assert!(directory(temp.path().join("missing").to_str().unwrap()).is_none());
        assert_eq!(
            session_id_in_directory(&sessions.join("cli/a.json"), Path::new("/work")).as_deref(),
            Some("s1")
        );
        assert_eq!(
            session_id_in_directory(&sessions.join("cli/a.json"), Path::new("/other")),
            None
        );
        assert_eq!(
            session_id_in_directory(
                &sessions.join("5d0275c3da257b44/sess_x/session.json"),
                Path::new("/work")
            )
            .as_deref(),
            Some("sess_x")
        );
        assert_eq!(files(&sessions).len(), 8);

        let mut verdicts = HashMap::new();
        let rows = read(&sessions, crate::models::now() as u64, &mut verdicts);
        let mut rows: Vec<_> = rows
            .iter()
            .map(|r| (r.0.as_str(), r.2.as_str(), r.4.as_str()))
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            [
                ("s1", "complete", ""),
                ("s2", "unknown", ""),
                ("s4", "start", ""),
                ("sess_x", "PermissionRequest", ""),
                ("sess_y", "complete", "sess_x"),
            ]
        );
        assert_eq!(verdicts.get("probe"), Some(&true));
        assert_eq!(verdicts.get("sess_z"), Some(&true));
        assert_eq!(verdicts.get("s1"), Some(&false));

        let before = fingerprint(&sessions);
        assert!(before.starts_with("kiro:7:"));
        std::fs::write(sessions.join("cli/a.lock"), "{}").unwrap();
        assert_ne!(
            fingerprint(&sessions),
            before,
            "a lock change re-reads the sessions"
        );
    }

    #[test]
    fn process_chain_marks_headless_internal_and_tui_sessions() {
        let headless = [
            link(
                &["/Users/u/.local/bin/kiro-cli", "acp", "--agent-engine=v2"],
                false,
                false,
            ),
            link(
                &["kiro-cli", "chat", "--no-interactive", "fix it"],
                false,
                false,
            ),
        ];
        assert_eq!(chain_verdict(&headless), Some(HEADLESS));
        let stream = [link(
            &["kiro-cli", "chat", "--output-format", "stream-json", "x"],
            false,
            false,
        )];
        assert_eq!(chain_verdict(&stream), Some(HEADLESS));
        let tui = [
            link(&["kiro-cli", "acp", "--agent-engine=v2"], false, true),
            link(
                &[
                    "/u/Library/Application Support/kiro-cli/bun",
                    "--no-env-file",
                    "--config=/u/bunfig.toml",
                    "/u/Library/Application Support/kiro-cli/tui.js",
                ],
                false,
                true,
            ),
            link(&["kiro-cli", "chat", "--v2"], false, true),
        ];
        assert_eq!(
            chain_verdict(&tui),
            Some(INTERACTIVE),
            "an exported key does not hide the TUI"
        );
        let keyed = [
            link(&["kiro-cli", "acp"], false, false),
            link(&["kiro-cli", "chat", "x"], false, true),
        ];
        assert_eq!(chain_verdict(&keyed), Some(HEADLESS));
        let ours = [
            link(&["kiro-cli", "acp"], true, false),
            link(&["kiro-cli", "chat", "--v2"], true, false),
        ];
        assert_eq!(chain_verdict(&ours), Some(INTERNAL));
        // An editor driving `kiro-cli acp` directly, or a plain `kiro-cli`, has no signal.
        assert_eq!(
            chain_verdict(&[
                link(&["kiro-cli", "acp"], false, false),
                link(&["zed"], false, false)
            ]),
            None
        );
        assert_eq!(chain_verdict(&[link(&["kiro-cli"], false, false)]), None);
        assert_eq!(chain_verdict(&[]), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_live_headless_run_stays_hidden_after_its_process_exits() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        for id in ["run", "plain"] {
            write(
                &sessions.join(format!("cli/{id}.json")),
                json!({"session_id":id,"cwd":"/work","session_state":{"conversation_metadata":{"user_turn_metadatas":[]}}}),
            );
        }
        // A script named kiro-cli: the shell keeps `sh <dir>/kiro-cli chat --no-interactive …`.
        let program = temp.path().join("kiro-cli");
        std::fs::write(&program, "#!/bin/sh\nwhile :; do sleep 1; done\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .arg(&program)
            .args(["chat", "--no-interactive", "summarize"])
            .spawn()
            .unwrap();
        let started = chrono::Utc::now().to_rfc3339();
        write(
            &sessions.join("cli/run.lock"),
            json!({"pid":child.id(),"started_at":started}),
        );
        // The lock of a process that is not kiro-cli carries no signal.
        write(
            &sessions.join("cli/plain.lock"),
            json!({"pid":std::process::id()}),
        );
        let mut verdicts = HashMap::new();
        let now = crate::models::now() as u64;
        let ids = |verdicts: &mut HashMap<String, bool>| {
            let mut ids: Vec<_> = read(&sessions, now, verdicts)
                .into_iter()
                .map(|r| r.0)
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(&mut verdicts), ["plain"]);
        child.kill().unwrap();
        child.wait().unwrap();
        std::fs::remove_file(sessions.join("cli/run.lock")).unwrap();
        assert_eq!(
            ids(&mut verdicts),
            ["plain"],
            "the verdict outlives the process"
        );
        verdicts.clear();
        assert_eq!(
            ids(&mut verdicts),
            ["plain", "run"],
            "never seen running: counted"
        );
    }
}
