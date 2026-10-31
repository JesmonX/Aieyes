//! Bounded native SQLite state. Each coexisting table has an independent cursor;
//! only visible rows and newly appended rows are read after discovery.
use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::{collections::HashMap, path::Path};
pub(crate) mod kiro;
pub(crate) mod tasks;

#[derive(Clone, Default)]
pub(crate) struct State {
    pub(crate) error: Option<String>,
    identity: String,
    tables: HashMap<String, TableState>,
    /// Observed writers by trajectory/task: `true` for headless or internal runs.
    trae: HashMap<std::path::PathBuf, (String, bool)>,
    /// Kiro session id → excluded, decided while its process was alive.
    kiro: HashMap<String, bool>,
    /// Last complete metadata while extension JSON files are rewritten.
    tasks: HashMap<std::path::PathBuf, tasks::Row>,
    #[cfg(test)]
    scanned: usize,
}
#[derive(Clone, Default)]
struct TableState {
    offset: i64,
    anchor: String,
    rows: HashMap<String, Row>,
}
#[derive(Clone)]
struct Row {
    rowid: i64,
    session: String,
    turn: String,
    created: u64,
    at: u64,
    data: Value,
}
impl Row {
    fn anchor(&self) -> String {
        format!("{}:{}", self.session, self.turn)
    }
}
pub(crate) type Activity = (String, String, String, u64, String);

pub(crate) fn refresh(
    provider: &str,
    path: &Path,
    mut state: State,
    now: u64,
) -> Result<(State, Vec<Activity>)> {
    state.error = None;
    if crate::qoder_family::is_qoder(provider) {
        return Ok((state, crate::qoder_desktop::activity(path, now)?));
    }
    #[cfg(test)]
    {
        state.scanned = 0;
    }
    if crate::subscriptions::trae::family(provider) {
        let rows = trae(path, &mut state, now, trae_processes);
        return Ok((state, rows));
    }
    if matches!(provider, "cline" | "roo" | "kilo")
        && (path.is_dir() || path.extension().is_some_and(|e| e == "json"))
    {
        let rows = tasks::refresh(provider, path, &mut state.tasks, now);
        return Ok((state, rows));
    }
    if provider == "kiro" {
        let rows = kiro::read(path, now, &mut state.kiro);
        return Ok((state, rows));
    }
    if provider == "codebuddy" {
        let (rows, error) = crate::codebuddy_ide::activity(path, now)?;
        state.error = error;
        return Ok((state, rows));
    }
    let db = crate::sqlite_ro::open_ro(path)?;
    db.execute_batch("BEGIN")?;
    if provider == "cline" {
        return Ok((state, tasks::cline(&db, now)?));
    }
    if provider == "workbuddy" {
        // One row per session, updated in place: read the recent window directly.
        return Ok((state, workbuddy(&db, now)?));
    }
    let tables = if crate::opencode::family(provider) {
        crate::opencode::schemas(&db)?
    } else {
        vec![("turn_usage", false)]
    };
    let meta = std::fs::metadata(path)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created());
    if state.identity != identity {
        state = State {
            identity,
            ..Default::default()
        };
    }
    state
        .tables
        .retain(|table, _| tables.iter().any(|(name, _)| name == table));
    let mut latest: HashMap<String, (Row, bool)> = HashMap::new();
    for (table, v2) in tables {
        let columns = if crate::opencode::family(provider) {
            crate::opencode::state_columns(v2)
        } else {
            "rowid,session_id,turn_id,started_at,COALESCE(NULLIF(completed_at,0),started_at),json_object('status',status)".into()
        };
        let query = format!("SELECT {columns} FROM {table}");
        let decode = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
            Ok(Row {
                rowid: r.get(0)?,
                session: r.get(1)?,
                turn: r.get(2)?,
                created: r.get::<_, i64>(3)?.max(0) as u64,
                at: r.get::<_, i64>(4)?.max(0) as u64,
                data: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
            })
        };
        let saved = state.tables.entry(table.into()).or_default();
        let anchor = db
            .query_row(&format!("{query} WHERE rowid=?1"), [saved.offset], decode)
            .optional()?;
        if saved.offset > 0 && anchor.as_ref().map(Row::anchor).as_deref() != Some(&saved.anchor) {
            *saved = TableState::default();
        }
        if !saved.rows.is_empty() {
            let ids = saved
                .rows
                .values()
                .map(|r| r.rowid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let rows = db
                .prepare(&format!("{query} WHERE rowid IN ({ids})"))?
                .query_map([], decode)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            #[cfg(test)]
            {
                state.scanned += rows.len();
            }
            saved.rows.clear();
            for row in rows {
                saved.rows.insert(row.session.clone(), row);
            }
        }
        loop {
            crate::operations::check()?;
            let rows = db
                .prepare(&format!("{query} WHERE rowid>?1 ORDER BY rowid LIMIT 256"))?
                .query_map([saved.offset], decode)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            #[cfg(test)]
            {
                state.scanned += rows.len();
            }
            for row in rows {
                saved.offset = row.rowid;
                saved.anchor = row.anchor();
                if now.saturating_sub(row.at / 1000) >= 86400 {
                    continue;
                }
                if saved
                    .rows
                    .get(&row.session)
                    .is_none_or(|old| (row.created, &row.turn) >= (old.created, &old.turn))
                {
                    saved.rows.insert(row.session.clone(), row);
                }
            }
            trim(&mut saved.rows, now);
        }
        trim(&mut saved.rows, now);
        for row in saved.rows.values() {
            if latest.get(&row.session).is_none_or(|(old, old_v2)| {
                (row.created, &row.turn, v2) >= (old.created, &old.turn, *old_v2)
            }) {
                latest.insert(row.session.clone(), (row.clone(), v2));
            }
        }
    }
    let parts = crate::opencode::family(provider) && crate::opencode::table_exists(&db, "part")?;
    let parents =
        if crate::opencode::family(provider) && crate::opencode::table_exists(&db, "session")? {
            db.prepare("PRAGMA table_info(session)")?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .any(|c| c == "parent_id")
        } else {
            false
        };
    let mut result = Vec::new();
    for (row, v2) in latest.into_values() {
        if provider == "kilo" && kilo_headless(&db, &row.session) {
            continue;
        }
        let mut kind = if provider == "zcode" {
            match row.data["status"].as_str() {
                Some("completed") => "complete",
                Some("error") => "failed",
                Some("cancelled") => "interrupted",
                Some("running") => "start",
                _ => "unknown",
            }
        } else {
            crate::opencode::phase(&row.data)
        };
        if parts && !v2 && !["complete", "failed", "interrupted"].contains(&kind) {
            let tools = db.prepare("SELECT json_extract(data,'$.tool') FROM part WHERE message_id=?1 AND json_valid(data) AND json_extract(data,'$.type')='tool' AND json_extract(data,'$.state.status') IN ('pending','running') LIMIT 64")?.query_map([&row.turn],|r|r.get::<_,Option<String>>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if !tools.is_empty() {
                kind = if tools.iter().any(|t| t.as_deref() == Some("question")) {
                    "PermissionRequest"
                } else {
                    "tool"
                };
            }
        }
        let parent = if parents {
            db.query_row(
                "SELECT parent_id FROM session WHERE id=?1",
                [&row.session],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or_default()
        } else {
            String::new()
        };
        result.push((row.session, row.turn, kind.into(), row.at, parent));
    }
    result.sort_by_key(|r| std::cmp::Reverse(r.3));
    result.truncate(256);
    Ok((state, result))
}
/// trae-cli (`trae-agent`) trajectories: `trajectories/*.json` in a directory
/// added for Trae, as trae-cli writes them where it runs. Trae IDE chats are kept
/// in an encrypted database and are not read.
pub(crate) fn trae_directory(path: &str) -> Option<std::path::PathBuf> {
    let root = crate::store::expand(path);
    if root.file_name().is_some_and(|n| n == "trajectories") && root.is_dir() {
        return Some(root);
    }
    Some(root.join("trajectories")).filter(|p| p.is_dir())
}
fn trae_recent(dir: &Path, now: u64) -> Vec<(std::path::PathBuf, u64)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e == "json")
                && p.file_name()
                    .is_some_and(|n| !n.to_string_lossy().starts_with('.'))
        })
        .filter_map(|p| {
            let at = std::fs::metadata(&p)
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            (now.saturating_sub(at / 1000) < 86400).then_some((p, at))
        })
        .collect();
    files.sort_by(|(a, at), (b, bt)| bt.cmp(at).then_with(|| a.cmp(b)));
    files.truncate(256);
    files
}
/// trae-cli rewrites a trajectory in place after every step.
pub(crate) fn trae_fingerprint(dir: &Path) -> String {
    let files = trae_recent(dir, crate::models::now() as u64);
    let signatures: Vec<_> = files
        .iter()
        .map(|(path, _)| {
            let meta = std::fs::metadata(path).ok();
            (
                path,
                meta.as_ref().map(std::fs::Metadata::len),
                meta.and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|t| t.as_nanos()),
            )
        })
        .collect();
    format!(
        "trae:{}:{}",
        files.len(),
        crate::models::hash(&format!("{signatures:?}"))
    )
}
/// `(start_time, ended, success)` of a valid trajectory's current task.
/// Deserialize metadata while discarding prompts, replies and tool payloads.
/// `interactive` reuses one file and leaves the previous task's `end_time` in
/// place, so a task is running while its end is earlier than its start.
fn trae_task(path: &Path) -> Option<(String, bool, bool)> {
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Metadata {
        start_time: String,
        #[serde(default)]
        end_time: String,
        #[serde(default)]
        success: bool,
        #[serde(rename = "llm_interactions")]
        _interactions: Vec<serde::de::IgnoredAny>,
    }
    let file = std::fs::File::open(path).ok()?;
    const LIMIT: u64 = 64 * 1024 * 1024;
    if file.metadata().ok()?.len() > LIMIT {
        return None;
    }
    let value: Metadata =
        serde_json::from_reader(std::io::BufReader::new(file.take(LIMIT + 1))).ok()?;
    let at = |text: &str| {
        chrono::DateTime::parse_from_rfc3339(text)
            .map(|t| t.naive_utc())
            .ok()
            .or_else(|| {
                chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                    .ok()?
                    .and_local_timezone(chrono::Local)
                    .earliest()
                    .map(|t| t.naive_utc())
            })
    };
    let start = at(&value.start_time)?;
    let ended = !value.end_time.is_empty() && at(&value.end_time)? >= start;
    Some((value.start_time, ended, value.success))
}
/// Live trae-cli process: start time, argv, initial working directory, internal flag.
type TraeProcess = (u64, Vec<String>, std::path::PathBuf, bool);
fn trae_processes() -> Vec<TraeProcess> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cmd(sysinfo::UpdateKind::Always)
            .with_cwd(sysinfo::UpdateKind::Always)
            .with_environ(sysinfo::UpdateKind::Always),
    );
    system
        .processes()
        .values()
        .filter_map(|p| {
            let argv: Vec<String> = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            if !crate::session_events::is_program("trae", &argv) {
                return None;
            }
            let internal = p
                .environ()
                .iter()
                .any(|e| e.to_string_lossy().starts_with("AIEYES_INTERNAL="));
            // The trajectory is resolved before Trae applies --working-dir.
            let initial = p.environ().iter().find_map(|entry| {
                let value = entry.to_string_lossy();
                let path = Path::new(value.strip_prefix("PWD=")?);
                path.is_absolute().then(|| path.to_owned())
            });
            Some((
                p.start_time(),
                argv,
                initial
                    .or_else(|| p.cwd().map(Path::to_owned))
                    .unwrap_or_default(),
                internal,
            ))
        })
        .collect()
}
/// The argv verdict of the one trae-cli process writing `path`: the process
/// given that file with `-t/--trajectory-file`, else the one started just before
/// the local time trae-cli names its default file after
/// (`trajectory_%Y%m%d_%H%M%S.json`). Several candidates give no verdict.
fn trae_writer(path: &Path, processes: &[TraeProcess]) -> Option<&'static str> {
    let name = path.file_name()?;
    let stamp = name
        .to_str()
        .and_then(|n| n.strip_prefix("trajectory_")?.strip_suffix(".json"))
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d_%H%M%S").ok())
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map(|t| t.timestamp());
    let writers: Vec<_> = processes
        .iter()
        .filter(|(start, argv, cwd, _)| {
            let file = argv.iter().enumerate().find_map(|(i, a)| match a.as_str() {
                "-t" | "--trajectory-file" => argv.get(i + 1).map(String::as_str),
                a => a.strip_prefix("--trajectory-file="),
            });
            match file {
                Some(file) => {
                    let file = Path::new(file);
                    if !file.is_absolute() && !cwd.is_absolute() {
                        return false;
                    }
                    let target = if file.is_absolute() {
                        file.to_owned()
                    } else {
                        cwd.join(file)
                    };
                    canonical(&target) == canonical(path)
                }
                None => {
                    cwd.is_absolute()
                        && canonical(&cwd.join("trajectories"))
                            == canonical(path.parent().unwrap_or(Path::new("")))
                        && stamp
                            .is_some_and(|t| (*start as i64 - 2..=*start as i64 + 120).contains(&t))
                }
            }
        })
        .collect();
    let [(_, argv, _, internal)] = writers.as_slice() else {
        return None;
    };
    if *internal {
        Some(crate::session_events::INTERNAL)
    } else {
        crate::session_events::headless_argv("trae", argv)
    }
}
fn canonical(path: &Path) -> std::path::PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

fn trae_ide_process(provider: &str, executable: &Path, argv: &[String]) -> bool {
    let executable = executable.to_string_lossy().replace('\\', "/");
    let name = if provider == "trae-cn" {
        "Trae CN"
    } else {
        "Trae"
    };
    let base = executable
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let main = executable.ends_with(&format!("/{name}.app/Contents/MacOS/Electron"))
        || base == format!("{}.exe", name.to_ascii_lowercase())
        || base
            == if provider == "trae-cn" {
                "trae-cn"
            } else {
                "trae"
            };
    main && !argv
        .iter()
        .any(|a| a == "--type" || a.starts_with("--type="))
}

/// Both editions share a short process snapshot. An IDE process never creates
/// a synthetic conversation or changes session counts.
pub(crate) fn trae_ide_running(provider: &str) -> Option<bool> {
    use std::{
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    type Snapshot = Option<(Instant, [bool; 2])>;
    static CACHE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if cache
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() >= Duration::from_secs(5))
    {
        let mut system = sysinfo::System::new();
        let refreshed = system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_exe(sysinfo::UpdateKind::Always),
        );
        if refreshed == 0 {
            return None;
        }
        let mut running = [false; 2];
        for process in system.processes().values() {
            let Some(executable) = process.exe() else {
                continue;
            };
            let argv: Vec<_> = process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            for (index, edition) in ["trae", "trae-cn"].iter().enumerate() {
                running[index] |= trae_ide_process(edition, executable, &argv);
            }
        }
        *cache = Some((Instant::now(), running));
    }
    cache
        .as_ref()
        .map(|(_, running)| running[usize::from(provider == "trae-cn")])
}
/// trae-cli keeps no headless marker on disk: `run` (and `--docker-*` /
/// `--must-patch` batches) are headless, `interactive` counts. The verdict of the
/// live writer is kept after it exits; a trajectory nobody was seen writing has
/// no signal and counts.
fn trae(
    dir: &Path,
    state: &mut State,
    now: u64,
    processes: impl FnOnce() -> Vec<TraeProcess>,
) -> Vec<Activity> {
    let recent = trae_recent(dir, now);
    let files: Vec<_> = recent
        .iter()
        .filter_map(|(path, at)| {
            let (task, ended, success) = trae_task(path)?;
            Some((path.clone(), *at, task, ended, success))
        })
        .collect();
    // An in-place rewrite can temporarily have no readable metadata. Keep the
    // writer verdict until the file is gone or valid metadata proves a new task.
    state.trae.retain(|path, (task, _)| {
        recent.iter().any(|(p, _)| p == path)
            && files
                .iter()
                .find(|(p, _, _, _, _)| p == path)
                .is_none_or(|(_, _, t, _, _)| t == task)
    });
    let live: Vec<_> = files
        .iter()
        .filter(|(p, at, _, _, _)| {
            !state.trae.contains_key(p) && now.saturating_sub(at / 1000) < 180
        })
        .map(|(p, _, task, _, _)| (p.clone(), task.clone()))
        .collect();
    if !live.is_empty() {
        let processes = processes();
        for (path, task) in live {
            if let Some(verdict) = trae_writer(&path, &processes) {
                state
                    .trae
                    .insert(path, (task, verdict != crate::session_events::INTERACTIVE));
            }
        }
    }
    files
        .into_iter()
        .filter(|(path, _, _, _, _)| !state.trae.get(path).is_some_and(|(_, excluded)| *excluded))
        .filter_map(|(path, at, task, ended, success)| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let kind = match (ended, success) {
                (false, _) => "start",
                (true, true) => "complete",
                (true, false) => "failed",
            };
            Some((id, task, kind.into(), at, String::new()))
        })
        .collect()
}
/// Roo Code task directory (`globalStorage/rooveterinaryinc.roo-cline/tasks`).
pub(crate) fn roo_directory(path: &str) -> Option<std::path::PathBuf> {
    tasks::directory("roo", path)
}
/// Every recent file contributes size and nanosecond mtime to the signature.
pub(crate) fn roo_fingerprint(dir: &Path) -> String {
    tasks::fingerprint("roo", dir)
}
/// `kilo run` stores its session with `question` denied (no one can answer);
/// TUI, IDE and `kilo serve` sessions keep it, so they are counted.
pub(crate) fn kilo_headless(db: &rusqlite::Connection, session: &str) -> bool {
    let mut current = session.to_owned();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..32 {
        if !seen.insert(current.clone()) {
            return false;
        }
        let Ok(Some((parent, permission))) = db
            .query_row(
                "SELECT parent_id,permission FROM session WHERE id=?1",
                [&current],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
        else {
            return false;
        };
        if let Some(parent) = parent.filter(|p| !p.is_empty()) {
            current = parent;
            continue;
        }
        // Subagents can deny questions even under an interactive lead. The
        // persisted root session decides the complete conversation's mode.
        return permission
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .and_then(|p| p.as_array().cloned())
            .is_some_and(|rules| {
                rules.iter().any(|r| {
                    r["permission"] == "question" && r["pattern"] == "*" && r["action"] == "deny"
                })
            });
    }
    false
}
/// WorkBuddy `sessions`: background automations (flagged, `source_mode`
/// `background`, or an `automation_runs` thread), playground runs and deleted
/// rows are not user sessions and are never listed. A turn is the last activity.
fn workbuddy(db: &rusqlite::Connection, now: u64) -> Result<Vec<Activity>> {
    let automation_runs = crate::opencode::table_exists(db, "automation_runs")?;
    let query = format!(
        "SELECT id,CAST(COALESCE(last_activity_at,updated_at) AS TEXT),status,COALESCE(last_activity_at,updated_at) AS at FROM sessions WHERE deleted_at IS NULL AND COALESCE(is_background_automation,0)=0 AND COALESCE(is_playground,0)=0 AND COALESCE(source_mode,'')<>'background'{} AND COALESCE(last_activity_at,updated_at)>?1 ORDER BY at DESC LIMIT 256",
        if automation_runs {
            " AND id NOT IN (SELECT thread_id FROM automation_runs)"
        } else {
            ""
        }
    );
    let since = now.saturating_sub(86400) as i64 * 1000;
    let rows = db
        .prepare(&query)?
        .query_map([since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                workbuddy_phase(&r.get::<_, String>(2)?).to_string(),
                r.get::<_, i64>(3)?.max(0) as u64,
                String::new(),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
/// `sessions.status`, by WorkBuddy's `normalizeTaskStatus`: `pending` waits for
/// the user (a finished turn or a new session), `cancelled` ends the turn.
pub(crate) fn workbuddy_phase(status: &str) -> &'static str {
    match status
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "completed" | "complete" | "done" | "pending" | "idle" | "await_input" | "awaitinput"
        | "waiting_input" | "waiting_user_input" | "connected" | "archived" => "complete",
        "waiting_permission" | "waiting_question" => "PermissionRequest",
        "failed" | "error" => "failed",
        "cancelled" | "canceled" | "aborted" | "interrupted" | "terminated" => "interrupted",
        "tool_start" | "tool_end" | "tool_executing" => "tool",
        "planning"
        | "preparing"
        | "connecting"
        | "working"
        | "running"
        | "in_progress"
        | "streaming"
        | "processing"
        | "handoff"
        | "summarizing"
        | "waiting_team_members"
        | "model_requesting"
        | "model_streaming"
        | "model_done" => "start",
        _ => "unknown",
    }
}
fn trim(rows: &mut HashMap<String, Row>, now: u64) {
    rows.retain(|_, r| now.saturating_sub(r.at / 1000) < 86400);
    if rows.len() > 256 {
        let mut sorted = rows.drain().collect::<Vec<_>>();
        sorted.sort_by_key(|(_, r)| std::cmp::Reverse(r.at));
        rows.extend(sorted.into_iter().take(256));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_refresh_reads_only_visible_and_new_rows_at_both_history_sizes() {
        for count in [10, 3000] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("opencode.db");
            let mut db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)").unwrap();
            let now = crate::models::now() as u64;
            let data = json!({"role":"assistant","finish":"stop","time":{"completed":now*1000},"content":"PRIVATE"}).to_string();
            let tx = db.transaction().unwrap();
            for index in 0..count {
                tx.execute(
                    "INSERT INTO message VALUES(?1,'s',?2,?2,?3)",
                    rusqlite::params![index.to_string(), now * 1000 + index, &data],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            let (state, rows) = refresh("opencode", &path, State::default(), now).unwrap();
            assert_eq!(state.scanned, count as usize);
            assert_eq!(rows.len(), 1);
            db.execute(
                "INSERT INTO message VALUES('new','s',?1,?1,?2)",
                rusqlite::params![
                    now * 1000 + count + 1,
                    json!({"role":"assistant"}).to_string()
                ],
            )
            .unwrap();
            let (state, rows) = refresh("opencode", &path, state, now).unwrap();
            assert_eq!(state.scanned, 2);
            assert_eq!((&*rows[0].1, &*rows[0].2), ("new", "thinking"));
            db.execute("UPDATE message SET data=?1 WHERE id='new'", [&data])
                .unwrap();
            let (state, rows) = refresh("opencode", &path, state, now).unwrap();
            assert_eq!(state.scanned, 1);
            assert_eq!(rows[0].2, "complete");
        }
    }

    #[test]
    fn trae_cli_trajectories_count_unless_their_writer_runs_headless() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        std::fs::create_dir_all(&dir).unwrap();
        // Top-level fields written by trae-agent 0.1.0 (`TrajectoryRecorder`, indent=2).
        for (name, start, end, success) in [
            (
                "trajectory_20261031_012437.json",
                "2026-10-31T01:24:38.728865",
                "",
                false,
            ),
            (
                "trajectory_20261031_020000.json",
                "2026-10-31T02:00:01.000001",
                "",
                false,
            ),
            // `interactive` reuses its file: the previous task's end stays behind.
            (
                "trajectory_20261031_030000.json",
                "2026-10-31T03:10:00.000000",
                "2026-10-31T03:05:00.000000",
                true,
            ),
            (
                "custom.json",
                "2026-10-31T04:00:00.000000",
                "2026-10-31T04:01:00.000000",
                true,
            ),
            (
                "failed.json",
                "2026-10-31T05:00:00.000000",
                "2026-10-31T05:01:00.000000",
                false,
            ),
        ] {
            let v = json!({"task":"PRIVATE","start_time":start,"end_time":end,"provider":"openai","model":"Qwen3.8-27B",
                "llm_interactions":[{"input_messages":[{"role":"user","content":"PRIVATE"}]}],"agent_steps":[],"success":success});
            std::fs::write(dir.join(name), serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
        let processes = || -> Vec<TraeProcess> {
            let at = |s: &str| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d_%H%M%S")
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .earliest()
                    .unwrap()
                    .timestamp() as u64
            };
            let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
            vec![
                // Process argv observed for `trae-cli run` installed with `uv tool install`.
                (
                    at("20261031_012435"),
                    argv(&[
                        "/u/.local/share/uv/tools/trae-agent/bin/python",
                        "/u/.local/bin/trae-cli",
                        "run",
                        "task",
                        "--working-dir",
                        "/w",
                    ]),
                    temp.path().to_owned(),
                    false,
                ),
                (
                    at("20261031_015958"),
                    argv(&["python3", "-m", "trae_agent.cli", "interactive"]),
                    temp.path().to_owned(),
                    false,
                ),
                (
                    1,
                    argv(&[
                        "trae-cli",
                        "run",
                        "--trajectory-file",
                        "/elsewhere/failed.json",
                    ]),
                    temp.path().to_owned(),
                    false,
                ),
            ]
        };
        let now = crate::models::now() as u64;
        let mut state = State::default();
        let rows = trae(&dir, &mut state, now, processes);
        let mut seen: Vec<_> = rows.iter().map(|r| (r.0.as_str(), r.2.as_str())).collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("custom", "complete"),
                ("failed", "failed"),
                ("trajectory_20261031_020000", "start"),
                ("trajectory_20261031_030000", "start"),
            ]
        );
        assert_eq!(
            rows.iter().find(|r| r.0 == "custom").unwrap().1,
            "2026-10-31T04:00:00.000000"
        );
        // The run has exited: its verdict stays; nothing else is re-sampled.
        let rows = trae(&dir, &mut state, now, Vec::new);
        assert!(!rows.iter().any(|r| r.0 == "trajectory_20261031_012437"));
        assert!(rows.iter().any(|r| r.0 == "failed"));
        // Without a seen writer, a trajectory has no signal and counts.
        let rows = trae(&dir, &mut State::default(), now, Vec::new);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.iter().find(|r| r.0 == "failed").unwrap().2, "failed");
        // Two possible writers: no verdict.
        let path = dir.join("trajectory_20261031_012437.json");
        let both = [
            processes()[0].clone(),
            (
                processes()[0].0 + 1,
                processes()[1].1.clone(),
                temp.path().to_owned(),
                false,
            ),
        ];
        assert_eq!(trae_writer(&path, &both), None);
        assert_eq!(
            trae_directory(temp.path().to_str().unwrap()),
            Some(dir.clone())
        );
        assert_eq!(trae_directory(dir.to_str().unwrap()), Some(dir.clone()));
        assert!(trae_fingerprint(&dir).starts_with("trae:5:"));
        assert!(refresh("trae", &dir, State::default(), now).is_ok());
        assert!(trae_directory(temp.path().join("missing").to_str().unwrap()).is_none());
    }

    #[test]
    fn trae_headless_verdict_survives_in_place_rewrites_but_not_new_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("tool.json");
        let task =
            |start| json!({"start_time":start,"end_time":"","llm_interactions":[]}).to_string();
        let first = task("2026-10-31T16:30:09.094402");
        let now = crate::models::now() as u64;
        for partial in [
            "",
            "{\"start_time\":",
            "{\"start_time\":\"2026-10-31T16:30:09.094402\"}",
        ] {
            std::fs::write(&path, &first).unwrap();
            let process = (
                now,
                vec![
                    "trae-cli".into(),
                    "run".into(),
                    "--trajectory-file".into(),
                    path.to_string_lossy().into(),
                ],
                temp.path().to_owned(),
                false,
            );
            let mut state = State::default();
            assert!(trae(&dir, &mut state, now, || vec![process]).is_empty());
            std::fs::write(&path, partial).unwrap();
            assert!(trae(&dir, &mut state, now, Vec::new).is_empty());
            std::fs::write(&path, &first).unwrap();
            assert!(
                trae(&dir, &mut state, now, Vec::new).is_empty(),
                "same-task rewrites must not turn a known headless run into a working session when its writer is temporarily absent"
            );
            std::fs::write(&path, task("2026-10-31T17:00:00.000001")).unwrap();
            let rows = trae(&dir, &mut state, now, Vec::new);
            assert_eq!(
                rows.len(),
                1,
                "a new task without writer evidence remains unknown and counts"
            );
            assert_eq!(rows[0].2, "start");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_live_trae_headless_writer_stays_excluded_after_a_torn_rewrite_and_exit() {
        use std::{
            io::{BufRead, BufReader, Write},
            process::{Child, Command, Stdio},
        };
        struct Guard(Child);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("tool.json");
        // A fixture process with the installed CLI's argv shape. It writes no
        // prompts or credentials and never launches a native CLI or model call.
        let program = temp.path().join("trae-cli");
        std::fs::write(
            &program,
            r#"import json,pathlib,sys
path=pathlib.Path(sys.argv[sys.argv.index('--trajectory-file')+1])
data={'start_time':'2026-10-31T16:30:09.094402','end_time':'','llm_interactions':[]}
path.write_text(json.dumps(data))
print('ready',flush=True)
if sys.stdin.readline().strip()=='truncate':
    path.write_text('{')
    print('partial',flush=True)
if sys.stdin.readline().strip()=='restore':
    path.write_text(json.dumps(data))
    print('restored',flush=True)
"#,
        )
        .unwrap();
        let mut child = Guard(
            Command::new("python3")
                .arg(&program)
                .arg("run")
                .arg("--trajectory-file")
                .arg(&path)
                .current_dir(temp.path())
                .env_remove("AIEYES_INTERNAL")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut output = BufReader::new(child.0.stdout.take().unwrap());
        let mut next = |expected: &str| {
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), expected);
        };
        next("ready");
        let now = crate::models::now() as u64;
        let (mut state, rows) = refresh("trae", &dir, State::default(), now).unwrap();
        assert!(
            rows.is_empty(),
            "the live fixture process must be classified headless"
        );
        writeln!(child.0.stdin.as_mut().unwrap(), "truncate").unwrap();
        next("partial");
        (state, _) = refresh("trae", &dir, state, now).unwrap();
        writeln!(child.0.stdin.as_mut().unwrap(), "restore").unwrap();
        next("restored");
        assert!(child.0.wait().unwrap().success());
        let (mut state, rows) = refresh("trae", &dir, state, now).unwrap();
        assert!(
            rows.is_empty(),
            "an observed headless run must remain excluded after its writer exits during the rewrite window"
        );
        std::fs::remove_file(&path).unwrap();
        (state, _) = refresh("trae", &dir, state, now).unwrap();
        assert!(
            state.trae.is_empty(),
            "a deleted trajectory clears its writer verdict"
        );
        std::fs::write(
            &path,
            json!({"start_time":"2026-10-31T16:30:09.094402","end_time":"","llm_interactions":[]})
                .to_string(),
        )
        .unwrap();
        let (_, rows) = refresh("trae", &dir, state, now).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "a recreated path without writer evidence must not inherit the deleted run's classification"
        );
        assert_eq!(rows[0].2, "start");
    }

    #[test]
    fn trae_rejects_non_trajectories_and_torn_json() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("task.json");
        for value in [
            json!({}),
            json!([]),
            json!({"start_time":""}),
            json!({"start_time":"2026-10-31T01:00:00", "llm_interactions":{}}),
            json!({"start_time":"PRIVATE_INVALID", "llm_interactions":[]}),
            json!({"start_time":"2026-10-31T01:00:00", "end_time":"invalid", "llm_interactions":[]}),
        ] {
            std::fs::write(&file, value.to_string()).unwrap();
            assert!(trae_task(&file).is_none());
        }
        let mut valid = json!({"task":"PRIVATE_".repeat(10000),"start_time":"2026-10-31T01:00:00.000001", "end_time":"", "llm_interactions":[], "success":false});
        std::fs::write(&file, valid.to_string()).unwrap();
        assert!(!trae_task(&file).unwrap().1);
        let mut text = serde_json::to_string_pretty(&valid).unwrap();
        text.pop();
        std::fs::write(&file, text).unwrap();
        assert!(trae_task(&file).is_none());
        valid["end_time"] = json!("2026-10-31T01:00:00.000000");
        std::fs::write(&file, valid.to_string()).unwrap();
        assert!(
            !trae_task(&file).unwrap().1,
            "Previous task end must not complete the next task"
        );
        let file = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        file.set_len(64 * 1024 * 1024 + 1).unwrap();
        assert!(trae_task(&temp.path().join("task.json")).is_none());
    }

    #[test]
    fn trae_writer_resolves_the_full_project_path_and_internal_verdict() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        for dir in [&a, &b] {
            std::fs::create_dir_all(dir.join("trajectories")).unwrap();
        }
        let path = a.join("trajectories/result.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::write(b.join("trajectories/result.json"), "{}").unwrap();
        let argv = |file: &str| {
            vec![
                "trae-cli".into(),
                "run".into(),
                "--trajectory-file".into(),
                file.into(),
            ]
        };
        assert_eq!(
            trae_writer(
                &path,
                &[(1, argv("trajectories/result.json"), b.clone(), false)]
            ),
            None
        );
        assert_eq!(
            trae_writer(
                &path,
                &[(
                    1,
                    argv(b.join("trajectories/result.json").to_str().unwrap()),
                    a.clone(),
                    false
                )]
            ),
            None
        );
        assert_eq!(
            trae_writer(
                &path,
                &[(1, argv("./trajectories/result.json"), a.clone(), false)]
            ),
            Some(crate::session_events::HEADLESS)
        );
        let interactive = vec![
            "trae-cli".into(),
            "interactive".into(),
            format!("--trajectory-file={}", path.display()),
        ];
        assert_eq!(
            trae_writer(&path, &[(1, interactive, a.clone(), true)]),
            Some(crate::session_events::INTERNAL)
        );
        let at = chrono::NaiveDateTime::parse_from_str("20261031_012437", "%Y%m%d_%H%M%S")
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .unwrap()
            .timestamp() as u64;
        let default = a.join("trajectories/trajectory_20261031_012437.json");
        assert_eq!(
            trae_writer(
                &default,
                &[(at, vec!["trae-cli".into(), "run".into()], b, false)]
            ),
            None
        );
        assert_eq!(
            trae_writer(
                &default,
                &[(at, vec!["trae-cli".into(), "run".into()], a, false)]
            ),
            Some(crate::session_events::HEADLESS)
        );
    }

    #[test]
    fn trae_fingerprint_notices_older_file_rewrites_and_reused_tasks() {
        use std::{
            fs::FileTimes,
            time::{Duration, SystemTime},
        };
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        std::fs::create_dir(&dir).unwrap();
        let old = dir.join("old.json");
        let new = dir.join("new.json");
        let data =
            |task| json!({"start_time": task,"llm_interactions":[], "end_time":""}).to_string();
        std::fs::write(&old, data("2026-10-31T01:00:00")).unwrap();
        std::fs::write(&new, data("2026-10-31T02:00:00")).unwrap();
        let times = FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120));
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(times)
            .unwrap();
        let before = trae_fingerprint(&dir);
        std::fs::write(&old, data("2026-10-31T01:00:01.123456")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(times)
            .unwrap();
        assert_ne!(before, trae_fingerprint(&dir));
        let mut state = State::default();
        let process = (
            1,
            vec![
                "trae-cli".into(),
                "run".into(),
                "-t".into(),
                new.display().to_string(),
            ],
            temp.path().to_owned(),
            false,
        );
        let now = crate::models::now() as u64;
        assert!(
            !trae(&dir, &mut state, now, || vec![process])
                .iter()
                .any(|r| r.0 == "new")
        );
        assert!(
            !trae(&dir, &mut state, now, Vec::new)
                .iter()
                .any(|r| r.0 == "new")
        );
        std::fs::write(&new, data("2026-10-31T03:00:00")).unwrap();
        assert!(
            trae(&dir, &mut state, now, Vec::new)
                .iter()
                .any(|r| r.0 == "new")
        );
    }

    #[test]
    fn trae_ide_processes_are_edition_specific_and_exclude_helpers() {
        for (provider, executable) in [
            ("trae", "/Applications/Trae.app/Contents/MacOS/Electron"),
            (
                "trae-cn",
                "/Applications/Trae CN.app/Contents/MacOS/Electron",
            ),
            ("trae", "C:\\Apps\\Trae\\Trae.exe"),
            ("trae-cn", "/opt/trae-cn/trae-cn"),
        ] {
            assert!(trae_ide_process(provider, Path::new(executable), &[]));
            assert!(!trae_ide_process(
                if provider == "trae" {
                    "trae-cn"
                } else {
                    "trae"
                },
                Path::new(executable),
                &[]
            ));
            assert!(!trae_ide_process(
                provider,
                Path::new(executable),
                &["--type=renderer".into()]
            ));
        }
        for executable in [
            "/usr/bin/trae-cli",
            "/Applications/Trae.app/Contents/Frameworks/Trae Helper.app/Contents/MacOS/Trae Helper",
            "/Applications/Other.app/Contents/MacOS/Electron",
        ] {
            assert!(!trae_ide_process("trae", Path::new(executable), &[]));
        }
    }

    #[test]
    fn cline_counts_interactive_sessions_and_their_subagents_only() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sessions.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        // Subset of the `sessions` table written by Cline CLI 3.0.74.
        db.execute_batch("CREATE TABLE sessions(session_id TEXT PRIMARY KEY,source TEXT NOT NULL,status TEXT NOT NULL,interactive INTEGER NOT NULL,parent_session_id TEXT,is_subagent INTEGER NOT NULL DEFAULT 0,prompt TEXT,updated_at TEXT NOT NULL)").unwrap();
        let now = crate::models::now() as u64;
        let at = |ago: u64| {
            chrono::DateTime::from_timestamp((now - ago) as i64, 0)
                .unwrap()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        };
        for (id, interactive, status, parent, ago) in [
            ("tui", 1, "completed", None, 60),
            ("json", 0, "completed", None, 30),
            ("child", 0, "running", Some("tui"), 10),
            ("old", 1, "completed", None, 2 * 86400),
        ] {
            db.execute(
                "INSERT INTO sessions VALUES(?1,'cli',?2,?3,?4,?5,'PRIVATE',?6)",
                rusqlite::params![id, status, interactive, parent, parent.is_some(), at(ago)],
            )
            .unwrap();
        }
        let (_, rows) = refresh("cline", &path, State::default(), now).unwrap();
        let rows: Vec<_> = rows.iter().map(|r| (&*r.0, &*r.2, &*r.4)).collect();
        assert_eq!(rows, [("child", "start", "tui"), ("tui", "complete", "")]);
    }

    #[test]
    fn roo_tasks_fold_subtasks_and_have_no_headless_mode() {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("tasks");
        for (id, extra) in [
            ("root", json!({"status":"active"})),
            ("sub", json!({"parentTaskId":"root","status":"completed"})),
        ] {
            std::fs::create_dir_all(tasks.join(id)).unwrap();
            let mut item = json!({"id":id,"number":1,"ts":crate::models::now()*1000,"task":"PRIVATE","tokensIn":1,"tokensOut":1,"totalCost":0});
            item.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            std::fs::write(tasks.join(id).join("history_item.json"), item.to_string()).unwrap();
        }
        let dir = roo_directory(temp.path().to_str().unwrap()).unwrap();
        let (_, rows) =
            refresh("roo", &dir, State::default(), crate::models::now() as u64).unwrap();
        let mut rows: Vec<_> = rows
            .iter()
            .map(|r| (r.0.clone(), r.2.clone(), r.4.clone()))
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            [
                ("root".into(), "start".into(), String::new()),
                ("sub".into(), "complete".into(), "root".into())
            ]
        );
        assert!(roo_fingerprint(&dir).starts_with("roo:2:"));
    }

    #[test]
    fn kilo_run_sessions_are_headless_and_other_sessions_count() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("kilo.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        // Columns as written by Kilo CLI 7.8.18 (OpenCode schema).
        db.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT,permission TEXT);CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)").unwrap();
        let run = r#"[{"permission":"question","pattern":"*","action":"deny"},{"permission":"plan_enter","pattern":"*","action":"deny"}]"#;
        db.execute(
            "INSERT INTO session VALUES('ses_run',NULL,?1),('ses_tui',NULL,NULL),('ses_child','ses_tui','[]')",
            [run],
        )
        .unwrap();
        let now = crate::models::now() as u64;
        let data =
            json!({"role":"assistant","finish":"stop","time":{"completed":now*1000}}).to_string();
        for (i, session) in ["ses_run", "ses_tui", "ses_child"].iter().enumerate() {
            db.execute(
                "INSERT INTO message VALUES(?1,?2,?3,?3,?4)",
                rusqlite::params![format!("m{i}"), session, now * 1000 + i as u64, &data],
            )
            .unwrap();
        }
        let (_, rows) = refresh("kilo", &path, State::default(), now).unwrap();
        let mut sessions: Vec<_> = rows.iter().map(|r| (&*r.0, &*r.4)).collect();
        sessions.sort();
        assert_eq!(sessions, [("ses_child", "ses_tui"), ("ses_tui", "")]);
        assert!(
            crate::opencode::database(temp.path().to_str().unwrap()).is_some_and(|p| p == path)
        );
    }

    #[test]
    fn workbuddy_lists_recent_user_sessions_and_skips_automation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("workbuddy.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(include_str!("../tests/fixtures/workbuddy/workbuddy.sql"))
            .unwrap();
        let now = crate::models::now() as u64;
        let (_, rows) = refresh("workbuddy", &path, State::default(), now).unwrap();
        let mut seen: Vec<_> = rows.iter().map(|r| (r.0.as_str(), r.2.as_str())).collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("earlier-user", "complete"),
                ("user-chat", "complete"),
                ("user-live", "start"),
                ("user-waiting", "PermissionRequest")
            ]
        );
        assert_eq!(workbuddy_phase("Completed"), "complete");
        assert_eq!(workbuddy_phase("tool-executing"), "tool");
        assert_eq!(workbuddy_phase("future"), "unknown");
    }
}
