//! Real process sampling -> native transcript -> public session snapshot.
//! Client stand-ins use no model or network and keep argv/environment private.
#[cfg(unix)]
mod unix {
    use aieyes_core::{
        models::{Settings, Source},
        sessions::SessionMonitor,
    };
    use serde_json::json;
    use std::{
        fs,
        path::Path,
        process::{Child, Command, Stdio},
        time::SystemTime,
    };
    struct Agent(Child);
    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn launch(root: &Path, workspace: &Path, print: bool, internal: bool) -> Agent {
        let cli = root.join("pi");
        if !cli.exists() {
            // macOS hides the environment of its platform-signed /bin/sh.
            // Park our own unsigned CLI on stdin so process sampling exercises
            // the same argv/environment surface as Node/Rust agent clients.
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_aieyes-core"), &cli).unwrap();
        }
        let mut command = Command::new(cli);
        command
            .arg("--data-dir")
            .arg(workspace.join("parked-core"))
            .current_dir(workspace)
            .env("PI_CODING_AGENT_DIR", root.join("data"))
            .env("AIEYES_AUTO_SCAN", "off")
            .env_remove("AIEYES_INTERNAL");
        if print {
            command.args(["-p", "PRIVATE_ARGV_CONTENT"]);
        }
        if internal {
            command.env("AIEYES_INTERNAL", "1");
        }
        Agent(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    fn transcript(root: &Path, id: &str, workspace: &Path) {
        let stamp = chrono::Utc::now().to_rfc3339();
        let header = json!({"type":"session","id":id,"cwd":workspace,"timestamp":stamp});
        let start = json!({"type":"message","id":format!("turn-{id}"),"timestamp":stamp,"message":{"role":"user","content":"PRIVATE_TRANSCRIPT_CONTENT"}});
        fs::write(
            root.join("data/sessions").join(format!("{id}.jsonl")),
            format!("{header}\n{start}\n"),
        )
        .unwrap();
    }
    #[test]
    fn native_process_argv_filters_only_its_own_session_and_retains_verdict_after_exit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("data/sessions")).unwrap();
        let workspaces: Vec<_> = ["headless", "interactive", "internal", "unknown"]
            .into_iter()
            .map(|id| {
                let p = root.join(id);
                fs::create_dir(&p).unwrap();
                p
            })
            .collect();
        let headless = launch(root, &workspaces[0], true, false);
        let _interactive = launch(root, &workspaces[1], false, false);
        let _internal = launch(root, &workspaces[2], false, true);
        for (id, path) in ["headless", "interactive", "internal", "unknown"]
            .iter()
            .zip(&workspaces)
        {
            transcript(root, id, path);
        }
        let settings = Settings {
            sources: vec![Source {
                id: "pi".into(),
                provider: "pi".into(),
                path: root.join("data").to_string_lossy().into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut monitor = SessionMonitor::default();
        let first = monitor.read(&settings, SystemTime::now());
        let mut visible: Vec<_> = first
            .sessions
            .iter()
            .map(|s| {
                Path::new(&s.path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        visible.sort();
        assert_eq!(visible, ["interactive", "unknown"]);
        assert_eq!(first.active_count(), 2);
        let wire = serde_json::to_string(&first).unwrap();
        assert!(!wire.contains("PRIVATE_"));
        assert!(!wire.contains("AIEYES_INTERNAL"));
        drop(headless);
        let after = monitor.read(&settings, SystemTime::now());
        assert_eq!(
            after.sessions.len(),
            2,
            "an observed headless run stays excluded after its process exits"
        );
        let late_workspace = root.join("late-flush");
        fs::create_dir(&late_workspace).unwrap();
        let late = launch(root, &late_workspace, true, false);
        monitor.read(&settings, SystemTime::now());
        drop(late);
        transcript(root, "late-flush", &late_workspace);
        assert_eq!(
            monitor.read(&settings, SystemTime::now()).sessions.len(),
            2,
            "a transcript first flushed at exit retains its observed launch mode"
        );
    }
}
