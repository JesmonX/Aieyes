//! Associate live CLI processes with native sessions, never with a provider as
//! a whole. Exact PID/session/open-file evidence wins. The fallback requires a
//! unique workspace + data-directory + launch-time match; ambiguous sessions
//! remain unknown. argv and environment values never leave this module.
use crate::{
    models::Source,
    session_events::{self, Activity},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct Context {
    pub(crate) path: PathBuf,
    pub(crate) session: String,
    pub(crate) workspace: Option<PathBuf>,
    created: u64,
}
#[derive(Clone)]
struct Process {
    pid: u32,
    start: u64,
    provider: String,
    classification: Option<&'static str>,
    selected_session: Option<String>,
    workspace: Option<PathBuf>,
    root: PathBuf,
    root_observed: bool,
    internal: bool,
    files: HashSet<PathBuf>,
    observed: Instant,
}
#[derive(Default)]
pub(crate) struct Snapshot {
    processes: Vec<Process>,
}
#[derive(Clone, Copy)]
pub(crate) struct Evidence {
    pub(crate) pid: u32,
    pub(crate) start: u64,
    pub(crate) classification: Option<&'static str>,
}
fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| dunce::simplified(path).to_owned())
}
fn absolute(value: &str) -> Option<PathBuf> {
    (!value.is_empty() && value.len() <= 4096 && Path::new(value).is_absolute())
        .then(|| canonical(Path::new(value)))
}
fn stamp(value: &Value) -> u64 {
    crate::models::timestamp(value)
        .map(|v| {
            let v = v.max(0) as u64;
            if v > 10_000_000_000 { v / 1000 } else { v }
        })
        .unwrap_or(0)
}
fn metadata(value: &Value, context: &mut Context) {
    let value = if value["type"] == "session_meta" {
        &value["payload"]
    } else {
        value
    };
    context.workspace = [
        "cwd",
        "workingDirectory",
        "working_directory",
        "workspace",
        "directory",
    ]
    .iter()
    .find_map(|key| value[*key].as_str().and_then(absolute))
    .or_else(|| context.workspace.clone());
    if context.created == 0 {
        context.created = ["createdAt", "created_at", "timestamp", "time"]
            .iter()
            .map(|key| stamp(&value[*key]))
            .find(|v| *v > 0)
            .unwrap_or(0);
    }
}
/// A bounded header or native index supplies the workspace, not the directory
/// name encoding (which loses dashes and can alias unrelated projects).
pub(crate) fn context(provider: &str, key: &str, session: &str) -> Context {
    let (file, database_session) = key
        .rsplit_once('#')
        .filter(|(p, _)| Path::new(p).is_file())
        .map_or((key, None), |(p, id)| (p, Some(id)));
    let path = canonical(Path::new(file));
    let mut result = Context {
        path: path.clone(),
        session: if session.is_empty() {
            key.into()
        } else {
            session.into()
        },
        ..Default::default()
    };
    if let Some(id) = database_session {
        result.session = id.into();
        if matches!(provider, "opencode" | "kilo")
            && let Ok(db) = crate::sqlite_ro::open_ro(&path)
        {
            for table in ["session_v2", "session"] {
                if let Ok((directory, created)) = db.query_row(
                    &format!("SELECT directory,time_created FROM {table} WHERE id=?1"),
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
                ) {
                    result.workspace = absolute(&directory);
                    result.created = if created > 10_000_000_000 {
                        created / 1000
                    } else {
                        created
                    };
                    break;
                }
            }
        }
        return result;
    }
    if provider == "kimi" {
        let root = path.ancestors().find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("session_"))
        });
        if let Some(root) = root {
            result.session = root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Ok(file) = std::fs::File::open(root.join("state.json")) {
                let mut bytes = Vec::new();
                if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_ok()
                    && bytes.len() <= 1024 * 1024
                    && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
                    && value["id"].as_str() == Some(&result.session)
                {
                    metadata(&value, &mut result);
                }
            }
        }
    }
    if result.workspace.is_none()
        && let Ok(file) = std::fs::File::open(&path)
    {
        let mut reader = BufReader::new(file);
        for _ in 0..4 {
            let mut line = Vec::new();
            if reader
                .by_ref()
                .take(65537)
                .read_until(b'\n', &mut line)
                .is_err()
                || line.len() > 65536
                || line.is_empty()
            {
                break;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                metadata(&value, &mut result);
            }
            if result.workspace.is_some() {
                break;
            }
        }
    }
    result
}

fn root(provider: &str, argv: &[String], env: &HashMap<String, String>) -> (PathBuf, bool) {
    let args = session_events::cli_args(provider, argv);
    if matches!(provider, "pi" | "omp")
        && let Some(path) = args
            .as_ref()
            .and_then(|a| a.value(&["--session-dir"]))
            .and_then(absolute)
    {
        return (path, true);
    }
    if let Some(path) = args
        .as_ref()
        .and_then(|a| a.value(&["--config-dir"]))
        .and_then(absolute)
    {
        return (path, true);
    }
    let variable = match provider {
        "codex" => "CODEX_HOME",
        "claude" => "CLAUDE_CONFIG_DIR",
        "kimi" => "KIMI_CODE_HOME",
        "codebuddy" => "CODEBUDDY_CONFIG_DIR",
        "qoder" => "QODER_CONFIG_DIR",
        "qoder-cn" => "QODERCN_CONFIG_DIR",
        "grok" => "GROK_HOME",
        "deepseek-harness" => "DSH_HOME",
        "pi" | "omp" => "PI_CODING_AGENT_DIR",
        _ => "",
    };
    if let Some(path) = env.get(variable).and_then(|v| absolute(v)) {
        return (path, true);
    }
    if matches!(provider, "opencode" | "kilo")
        && let Some(path) = env.get("XDG_DATA_HOME").and_then(|v| absolute(v))
    {
        return (path.join(provider), true);
    }
    let default = crate::contracts::provider(provider).map_or("", |p| p.default_path);
    (canonical(&crate::store::expand(default)), false)
}
impl Process {
    fn retain_launch_evidence(&mut self, previous: &Self) {
        // Node's process.title overwrites the kernel argv on macOS/Linux.
        // Preserve only normalized evidence, never the original prompt/argv.
        // This is scoped to a process lifetime, so an interactive resume in a
        // new process can replace the old session's headless classification.
        if self.pid != previous.pid
            || self.start != previous.start
            || self.provider != previous.provider
        {
            return;
        }
        self.internal |= previous.internal;
        if previous.classification == Some(session_events::HEADLESS)
            || self.classification.is_none()
        {
            self.classification = previous.classification;
        }
        if !self.root_observed && previous.root_observed {
            self.root.clone_from(&previous.root);
            self.root_observed = true;
        }
        if self.selected_session.is_none() {
            self.selected_session.clone_from(&previous.selected_session);
        }
        if self.workspace.is_none() {
            self.workspace.clone_from(&previous.workspace);
        }
    }
}
impl Snapshot {
    pub(crate) fn refresh(&mut self, sources: &[Source]) {
        let mut current = Self::read(sources, self);
        // Some clients flush their transcript while exiting. Keep normalized
        // evidence briefly so that the first complete file can still be tied
        // to its launch; the same uniqueness checks apply to these candidates.
        for previous in &self.processes {
            if current.processes.len() >= 256 {
                break;
            }
            if previous.observed.elapsed() <= Duration::from_secs(15)
                && sources.iter().any(|s| s.provider == previous.provider)
                && !current.processes.iter().any(|p| {
                    p.pid == previous.pid
                        && p.start == previous.start
                        && p.provider == previous.provider
                })
            {
                current.processes.push(previous.clone());
            }
        }
        *self = current;
    }
    fn read(sources: &[Source], previous: &Self) -> Self {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let providers: HashSet<&str> = sources.iter().map(|s| s.provider.as_str()).collect();
        if providers.is_empty() {
            return Self::default();
        }
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
        let ids: Vec<_> = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                let argv: Vec<_> = process
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect();
                (!argv
                    .iter()
                    .any(|s| s.contains(crate::session_hooks::MARKER))
                    && (providers
                        .iter()
                        .any(|p| session_events::is_program(p, &argv))
                        || previous.processes.iter().any(|p| {
                            p.pid == pid.as_u32()
                                && p.start == process.start_time()
                                && providers.contains(p.provider.as_str())
                        })))
                .then_some(*pid)
            })
            .take(256)
            .collect();
        if ids.is_empty() {
            return Self::default();
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&ids),
            true,
            ProcessRefreshKind::nothing()
                .with_cwd(UpdateKind::Always)
                .with_environ(UpdateKind::Always),
        );
        // These native logs lack a workspace header. Query only their writers;
        // Pi/Kimi and database sessions use the cheaper native metadata match.
        let writers: Vec<_> = ids
            .iter()
            .filter(|id| {
                system.process(**id).is_some_and(|p| {
                    let argv: Vec<_> = p
                        .cmd()
                        .iter()
                        .map(|s| s.to_string_lossy().into_owned())
                        .collect();
                    ["deepseek-harness", "grok"].iter().any(|provider| {
                        providers.contains(provider) && session_events::is_program(provider, &argv)
                    })
                })
            })
            .map(|id| id.as_u32())
            .collect();
        let files = if writers.is_empty() {
            HashMap::new()
        } else {
            open_files(&writers)
        };
        let mut processes = Vec::new();
        for id in ids {
            let Some(p) = system.process(id) else {
                continue;
            };
            let argv: Vec<_> = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            let env: HashMap<String, String> = p
                .environ()
                .iter()
                .filter_map(|s| {
                    let value = s.to_str()?;
                    let (key, value) = value.split_once('=')?;
                    matches!(
                        key,
                        "AIEYES_INTERNAL"
                            | "CODEX_HOME"
                            | "CLAUDE_CONFIG_DIR"
                            | "KIMI_CODE_HOME"
                            | "CODEBUDDY_CONFIG_DIR"
                            | "QODER_CONFIG_DIR"
                            | "GROK_HOME"
                            | "DSH_HOME"
                            | "PI_CODING_AGENT_DIR"
                            | "XDG_DATA_HOME"
                    )
                    .then(|| (key.into(), value.into()))
                })
                .collect();
            for provider in &providers {
                let prior = previous.processes.iter().find(|old| {
                    old.pid == id.as_u32()
                        && old.start == p.start_time()
                        && old.provider == *provider
                });
                let args = session_events::cli_args(provider, &argv);
                if args.is_none() && prior.is_none() {
                    continue;
                }
                // A TUI's shared server is not the writer/client of every session.
                if args.as_ref().is_some_and(|args| {
                    (*provider == "opencode"
                        && args.positional.first() == Some(&"serve")
                        && args.has(&["--service"]))
                        || (*provider == "kilo" && args.positional.first() == Some(&"serve"))
                }) {
                    continue;
                }
                let workspace = args
                    .as_ref()
                    .and_then(|a| {
                        a.value(&["--cwd", "--directory", "--workdir", "--work-dir", "-C"])
                    })
                    .and_then(absolute)
                    .or_else(|| p.cwd().map(canonical));
                let (root, root_observed) = root(provider, &argv, &env);
                let mut classification =
                    session_events::headless_argv(provider, &argv).or_else(|| {
                        (args.is_some() && matches!(*provider, "pi" | "omp"))
                            .then_some(session_events::INTERACTIVE)
                    });
                // Rewritten argv can consume environment entries up to the old
                // argc. Such a title carries no proof of interactive mode.
                if classification != Some(session_events::HEADLESS)
                    && argv.iter().skip(1).any(|arg| {
                        arg.split_once('=').is_some_and(|(key, _)| {
                            !key.is_empty()
                                && key.bytes().all(|c| {
                                    c.is_ascii_uppercase() || c == b'_' || c.is_ascii_digit()
                                })
                        })
                    })
                {
                    classification = None;
                }
                let mut process = Process {
                    pid: id.as_u32(),
                    start: p.start_time(),
                    provider: (*provider).into(),
                    classification,
                    selected_session: args
                        .as_ref()
                        .and_then(|a| a.value(&["--session", "--session-id", "--resume", "-S"]))
                        .map(str::to_string),
                    root,
                    root_observed,
                    workspace,
                    internal: env.get("AIEYES_INTERNAL").is_some_and(|v| !v.is_empty()),
                    files: files.get(&id.as_u32()).cloned().unwrap_or_default(),
                    observed: Instant::now(),
                };
                if let Some(previous) = prior {
                    process.retain_launch_evidence(previous);
                }
                processes.push(process);
            }
        }
        Self { processes }
    }
    pub(crate) fn associate(
        &self,
        provider: &str,
        sessions: &[(String, Context, Activity)],
    ) -> HashMap<String, Evidence> {
        let mut matches: Vec<(usize, usize, u8)> = Vec::new();
        for (si, (_, context, activity)) in sessions.iter().enumerate() {
            for (pi, p) in self
                .processes
                .iter()
                .enumerate()
                .filter(|(_, p)| p.provider == provider)
            {
                let exact_pid = activity.pid == p.pid
                    && activity.process_start > 0
                    && activity.process_start == p.start;
                let transcript = !sessions[si].0.contains('#')
                    && (p.files.contains(&context.path)
                        || provider == "deepseek-harness"
                            && context.path.parent().is_some_and(|parent| {
                                p.files.contains(&parent.join("session.lock"))
                            }));
                let in_root = context.path.starts_with(&p.root);
                let explicit = p.selected_session.as_deref().is_some_and(|value| {
                    value == context.session
                        || value == activity.session
                        || absolute(value).as_ref() == Some(&context.path)
                });
                let same_workspace =
                    context.workspace.is_some() && context.workspace == p.workspace;
                let started =
                    context.created >= p.start && context.created <= p.start.saturating_add(120);
                let turn = activity.turn_started_at() / 1000;
                let resumed = activity.phase.is_some_and(|p| p.active())
                    && turn >= p.start
                    && turn <= p.start.saturating_add(120);
                let strength = if exact_pid || transcript {
                    3
                } else if in_root && explicit {
                    2
                } else if in_root && same_workspace && (started || resumed) {
                    1
                } else {
                    continue;
                };
                matches.push((si, pi, strength));
            }
        }
        let mut result = HashMap::new();
        for (si, (key, _, _)) in sessions.iter().enumerate() {
            let best = matches
                .iter()
                .filter(|(s, _, _)| *s == si)
                .map(|(_, _, strength)| *strength)
                .max();
            let candidates: Vec<_> = matches
                .iter()
                .filter(|(s, _, strength)| *s == si && Some(*strength) == best)
                .collect();
            if candidates.len() != 1 {
                continue;
            }
            let (_, pi, strength) = *candidates[0];
            if strength == 1 {
                let native_sessions: HashSet<_> = matches
                    .iter()
                    .filter(|(_, p, _)| *p == pi)
                    .map(|(s, _, _)| &sessions[*s].1.session)
                    .collect();
                if native_sessions.len() != 1 {
                    continue;
                }
            }
            let p = &self.processes[pi];
            let classification = if p.internal {
                Some(session_events::INTERNAL)
            } else {
                p.classification
            };
            result.insert(
                key.clone(),
                Evidence {
                    pid: p.pid,
                    start: p.start,
                    classification,
                },
            );
        }
        result
    }
}

#[cfg(target_os = "linux")]
fn open_files(pids: &[u32]) -> HashMap<u32, HashSet<PathBuf>> {
    pids.iter()
        .map(|pid| {
            (
                *pid,
                std::fs::read_dir(format!("/proc/{pid}/fd"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .take(512)
                    .filter_map(|e| std::fs::read_link(e.path()).ok())
                    .filter(|p| p.is_absolute())
                    .map(|p| canonical(&p))
                    .collect(),
            )
        })
        .collect()
}
#[cfg(target_os = "macos")]
fn open_files(pids: &[u32]) -> HashMap<u32, HashSet<PathBuf>> {
    let mut command = std::process::Command::new("/usr/sbin/lsof");
    command.args([
        "-n",
        "-P",
        "-Fpn",
        "-p",
        &pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ]);
    let Ok(bytes) = crate::process::run(command, Vec::new(), std::time::Duration::from_secs(1))
    else {
        return HashMap::new();
    };
    let mut result: HashMap<u32, HashSet<PathBuf>> = HashMap::new();
    let mut pid = None;
    for line in String::from_utf8_lossy(&bytes).lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok().filter(|p| pids.contains(p));
        }
        if let Some(pid) = pid
            && let Some(path) = line.strip_prefix('n').and_then(absolute)
        {
            result.entry(pid).or_default().insert(path);
        }
    }
    result
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_files(_: &[u32]) -> HashMap<u32, HashSet<PathBuf>> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(pid: u32, workspace: &str, argv: &[&str]) -> Process {
        let argv: Vec<String> = argv.iter().map(|s| (*s).into()).collect();
        let args = session_events::cli_args("pi", &argv).unwrap();
        Process {
            pid,
            start: 100,
            provider: "pi".into(),
            classification: session_events::headless_argv("pi", &argv)
                .or(Some(session_events::INTERACTIVE)),
            selected_session: args
                .value(&["--session", "--session-id", "--resume", "-S"])
                .map(str::to_string),
            workspace: Some(workspace.into()),
            root: "/data/pi".into(),
            root_observed: true,
            internal: false,
            files: HashSet::new(),
            observed: Instant::now(),
        }
    }
    fn session(id: &str, workspace: &str) -> (String, Context, Activity) {
        let mut activity = Activity::default();
        activity.session = id.into();
        (
            format!("/data/pi/{id}.jsonl"),
            Context {
                path: format!("/data/pi/{id}.jsonl").into(),
                session: id.into(),
                workspace: Some(workspace.into()),
                created: 101,
            },
            activity,
        )
    }
    #[test]
    fn association_never_crosses_workspaces_roots_or_ambiguous_sessions() {
        let sessions = [session("a", "/project/a"), session("b", "/project/b")];
        let snapshot = Snapshot {
            processes: vec![
                process(1, "/project/a", &["pi", "-p", "fixture"]),
                process(2, "/project/b", &["pi"]),
            ],
        };
        let result = snapshot.associate("pi", &sessions);
        assert_eq!(
            result[&sessions[0].0].classification,
            Some(session_events::HEADLESS)
        );
        assert_eq!(
            result[&sessions[1].0].classification,
            Some(session_events::INTERACTIVE)
        );
        let ambiguous = [sessions[0].clone(), session("other-a", "/project/a")];
        assert!(snapshot.associate("pi", &ambiguous).is_empty());
        let mut wrong = snapshot;
        wrong.processes[0].root = "/different/data".into();
        assert!(
            !wrong
                .associate("pi", &sessions)
                .contains_key(&sessions[0].0)
        );
        wrong.processes[0].root = "/data/pi".into();
        wrong.processes.push(process(3, "/project/a", &["pi"]));
        assert!(
            !wrong
                .associate("pi", &sessions)
                .contains_key(&sessions[0].0)
        );
    }
    #[test]
    fn exact_session_and_pid_start_are_stronger_than_workspace_and_internal_wins() {
        let sessions = [session("a", "/project/a"), session("b", "/project/a")];
        let mut process = process(1, "/project/a", &["pi", "--session-id", "a", "-p"]);
        process.internal = true;
        let snapshot = Snapshot {
            processes: vec![process],
        };
        let result = snapshot.associate("pi", &sessions);
        assert_eq!(
            result[&sessions[0].0].classification,
            Some(session_events::INTERNAL)
        );
        assert!(!result.contains_key(&sessions[1].0));
        let mut wrong = session("unrelated", "/elsewhere");
        wrong.1.created = 1;
        wrong.2.pid = 1;
        wrong.2.process_start = 99;
        assert!(
            snapshot.associate("pi", &[wrong.clone()]).is_empty(),
            "reused PID is not evidence"
        );
        wrong.2.process_start = 100;
        assert_eq!(
            snapshot.associate("pi", &[wrong.clone()])[&wrong.0].classification,
            Some(session_events::INTERNAL)
        );
    }
    #[test]
    fn title_rewrite_keeps_launch_verdict_and_root_only_for_the_same_process_lifetime() {
        let mut launched = process(
            1,
            "/project/a",
            &["pi", "--session-id", "a", "-p", "PRIVATE_PROMPT"],
        );
        launched.internal = true;
        let mut rewritten = process(1, "/project/a", &["pi"]);
        rewritten.classification = None;
        rewritten.root = "/unknown-default".into();
        rewritten.root_observed = false;
        rewritten.retain_launch_evidence(&launched);
        assert_eq!(rewritten.classification, Some(session_events::HEADLESS));
        assert_eq!(rewritten.root, Path::new("/data/pi"));
        assert_eq!(rewritten.selected_session.as_deref(), Some("a"));
        assert!(rewritten.internal);
        let snapshot = Snapshot {
            processes: vec![rewritten],
        };
        let sessions = [session("a", "/project/a")];
        assert_eq!(
            snapshot.associate("pi", &sessions)[&sessions[0].0].classification,
            Some(session_events::INTERNAL)
        );
        let mut resumed = process(1, "/project/a", &["pi"]);
        resumed.start += 1;
        resumed.retain_launch_evidence(&launched);
        assert_eq!(resumed.classification, Some(session_events::INTERACTIVE));
        assert!(!resumed.internal);
    }
    #[test]
    fn harness_lock_is_exact_session_evidence_and_retired_launches_expire() {
        let mut writer = process(u32::MAX, "/workspace", &["pi", "-p"]);
        writer.provider = "deepseek-harness".into();
        writer
            .files
            .insert("/data/dsh/session-a/session.lock".into());
        let mut a = session("a", "/unknown");
        a.0 = "/data/dsh/session-a/session.v4.jsonl.zstd".into();
        a.1.path = a.0.clone().into();
        a.1.workspace = None;
        let mut b = a.clone();
        b.0 = "/data/dsh/session-b/session.v4.jsonl.zstd".into();
        b.1.path = b.0.clone().into();
        let mut snapshot = Snapshot {
            processes: vec![writer],
        };
        let source = Source {
            provider: "deepseek-harness".into(),
            ..Default::default()
        };
        snapshot.refresh(std::slice::from_ref(&source));
        let evidence = snapshot.associate("deepseek-harness", &[a.clone(), b.clone()]);
        assert_eq!(
            evidence[&a.0].classification,
            Some(session_events::HEADLESS)
        );
        assert!(!evidence.contains_key(&b.0));
        for process in &mut snapshot.processes {
            process.observed = Instant::now() - Duration::from_secs(16);
        }
        snapshot.refresh(&[source]);
        assert!(
            !snapshot
                .processes
                .iter()
                .any(|process| process.pid == u32::MAX)
        );
    }
}
