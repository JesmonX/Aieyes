//! Runs the real interaction hook helper the way Claude Code starts it: as a child of a process
//! named `claude`, with the hook input on stdin, against a store the desktop app has enabled.
#![cfg(unix)]
use aieyes_core::{
    models::{Settings, Source},
    store::Store,
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const NATIVE: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

struct Bench {
    dir: tempfile::TempDir,
    db: Connection,
    transcript: PathBuf,
}

impl Bench {
    fn new(approvals: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let home = dir.path().join("claude-home");
        fs::create_dir_all(home.join("projects/p")).unwrap();
        let store = Store::open(&root).unwrap();
        store
            .save_settings(&Settings {
                sources: vec![Source {
                    id: "claude-src".into(),
                    provider: "claude".into(),
                    path: home.to_string_lossy().into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        let set = |key: &str, value: Value| {
            store
                .db
                .execute(
                    "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    rusqlite::params![key, value.to_string()],
                )
                .unwrap();
        };
        set(
            "desktop.interaction.v1",
            json!({"enabled": true, "claudeQuestions": true, "claudeApprovals": approvals}),
        );
        // The desktop app refreshes this every few seconds; the test outlives no hold.
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        set("interaction.heartbeat", json!({"at": at + 600}));
        drop(store);
        let transcript = home.join(format!("projects/p/{NATIVE}.jsonl"));
        fs::write(
            &transcript,
            format!("{}\n", json!({"type":"user","message":{"content":"go"}})),
        )
        .unwrap();
        // A shell under the agent's name stands in for the agent process.
        let shell = ["/bin/dash", "/bin/sh"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .unwrap();
        fs::copy(shell, dir.path().join("claude")).unwrap();
        let db = Connection::open(root.join("aieyes.sqlite")).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        Self {
            dir,
            db,
            transcript,
        }
    }

    fn append(&self, record: Value) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .unwrap();
        writeln!(file, "{record}").unwrap();
    }

    fn observer_event(&self, event: Value) {
        let path = self.dir.path().join("claude-home/.aieyes/session-events");
        fs::create_dir_all(&path).unwrap();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.join(format!("{NATIVE}.jsonl")))
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }

    fn hook_input(&self, event: &str, extra: Value) -> Value {
        let mut input = json!({
            "session_id": NATIVE,
            "transcript_path": self.transcript,
            "cwd": "/work/project",
            "permission_mode": "default",
            "hook_event_name": event,
        });
        for (key, value) in extra.as_object().unwrap() {
            input[key] = value.clone();
        }
        input
    }

    /// Start the helper under the stand-in agent. `flags` are the agent's own arguments.
    fn start(&self, flags: &[&str], input: &Value) -> Child {
        self.start_text(flags, &input.to_string())
    }

    fn start_text(&self, flags: &[&str], input: &str) -> Child {
        let dir = self.dir.path();
        fs::write(dir.join("input.json"), input).unwrap();
        let _ = fs::remove_file(dir.join("output.json"));
        fs::write(
            dir.join("hook.sh"),
            format!(
                "'{}' --aieyes-claude-question '{}' claude-src < '{}' > '{}'\nexit $?\n",
                env!("CARGO_BIN_EXE_aieyes-core"),
                dir.join("data").display(),
                dir.join("input.json").display(),
                dir.join("output.json").display(),
            ),
        )
        .unwrap();
        Command::new(dir.join("claude"))
            .arg(dir.join("hook.sh"))
            .args(flags)
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn output(&self, mut child: Child) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("the hook helper did not exit");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        fs::read_to_string(self.dir.path().join("output.json")).unwrap_or_default()
    }

    /// `(id, kind, tool, call id, input, meta)` of the request the helper registered.
    fn row(&self) -> Option<(String, String, String, String, Value, Value)> {
        self.db
            .query_row(
                "SELECT id,kind,tool_name,call_id,input,meta FROM channel_claude_requests WHERE consumed=0 AND response IS NULL",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        serde_json::from_str(&r.get::<_, String>(4)?).unwrap(),
                        serde_json::from_str(&r.get::<_, String>(5)?).unwrap(),
                    ))
                },
            )
            .optional()
            .unwrap()
    }

    fn held(&self) -> (String, String, String, String, Value, Value) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(row) = self.row() {
                return row;
            }
            assert!(Instant::now() < deadline, "the helper registered nothing");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn respond(&self, id: &str, response: &str) {
        let changed = self
            .db
            .execute(
                "UPDATE channel_claude_requests SET response=?1 WHERE id=?2 AND response IS NULL",
                rusqlite::params![response, id],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    fn rows(&self) -> Vec<(String, String, Option<String>, i64)> {
        let mut stmt = self
            .db
            .prepare("SELECT input,meta,response,consumed FROM channel_claude_requests")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn tool_use(id: &str, name: &str, input: &Value) -> Value {
    json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}})
}

#[test]
fn an_approval_is_held_answered_and_erased() {
    let bench = Bench::new(true);
    let tool_input = json!({"command": "git init", "description": "Create a repository"});
    bench.append(tool_use("toolu_1", "Bash", &tool_input));
    let input = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": tool_input, "permission_suggestions": [
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "git init *"}], "behavior": "allow", "destination": "localSettings"},
            {"type": "setMode", "mode": "bypassPermissions", "destination": "session"}
        ]}),
    );
    let child = bench.start(&[], &input);
    let (id, kind, tool, call, stored, meta) = bench.held();
    assert_eq!((kind.as_str(), tool.as_str()), ("approval", "Bash"));
    // The hook input names no tool call; the transcript does.
    assert_eq!(call, "toolu_1");
    assert_eq!(stored, tool_input);
    assert_eq!(meta["provider"], "claude");
    assert_eq!(meta["cwd"], "/work/project");
    assert_eq!(meta["truncated"], false);
    assert_eq!(meta["suggestions"].as_array().unwrap().len(), 1);
    assert_eq!(meta["suggestions"][0]["destination"], "localSettings");

    let reply = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {
        "behavior": "allow", "updatedPermissions": meta["suggestions"]}}});
    bench.respond(&id, &reply.to_string());
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert_eq!(printed, reply);
    // Delivered: identity stays for the receipt, the command and the answer do not.
    assert_eq!(
        bench.rows(),
        [("{}".to_owned(), "{}".to_owned(), Some(String::new()), 1)]
    );
}

#[test]
fn only_delivered_approvals_stop_waiting_before_the_tool_returns() {
    let bench = Bench::new(true);
    let mut engine = aieyes_core::Engine::open(&bench.dir.path().join("data")).unwrap();
    let session_id = aieyes_core::models::hash(&format!("local:claude:{NATIVE}"));
    let at = chrono::Utc::now().timestamp_millis() - 5_000;
    let event = |kind: &str, call: &str, at: i64| {
        bench.observer_event(json!({"kind":kind,"call":call,"at":at,
            "session":NATIVE,"classification":"interactive","turn":"turn","tool":"Bash"}));
    };
    let inspect = |engine: &mut aieyes_core::Engine| -> (Value, Value) {
        let snapshot = engine.call("sessions.list", json!({})).unwrap();
        let session = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == session_id)
            .unwrap_or_else(|| panic!("fixture session missing: {snapshot}"));
        let card = engine
            .call(
                "sessions.interaction.inspect",
                json!({"sessionId":session_id}),
            )
            .unwrap();
        (session["phase"].clone(), card["kind"].clone())
    };
    event("UserPromptSubmit", "", at);
    // Claude 2.1.332 omits the call ID on this hook; the preceding tool identifies it.
    event("PreToolUse", "toolu_delivered", at + 1);
    event("PermissionRequest", "", at + 2);
    assert_eq!(
        inspect(&mut engine),
        (json!("needsInput"), json!("approval"))
    );

    let tool_input = json!({"command":"echo test"});
    bench.append(tool_use("toolu_delivered", "Bash", &tool_input));
    let input = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name":"Bash","tool_input":tool_input}),
    );
    let child = bench.start(&[], &input);
    let (id, _, _, call, _, _) = bench.held();
    assert_eq!(call, "toolu_delivered");
    bench.respond(
        &id,
        &json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest",
        "decision":{"behavior":"deny","interrupt":true}}})
        .to_string(),
    );
    assert!(!bench.output(child).is_empty());
    // Neither PostToolUse nor Stop was written. The same session should still drop the card.
    assert_eq!(inspect(&mut engine), (json!("tool"), json!("none")));

    // A later same-call permission prompt in the terminal remains a real wait.
    let later = chrono::Utc::now().timestamp_millis() + 1;
    event("PermissionRequest", "toolu_delivered", later);
    assert_eq!(
        inspect(&mut engine),
        (json!("needsInput"), json!("approval"))
    );
    let child = bench.start(&[], &input);
    let (id, ..) = bench.held();
    bench.respond(&id, "__aieyes_passthrough__");
    assert!(bench.output(child).is_empty());
    assert_eq!(
        inspect(&mut engine),
        (json!("needsInput"), json!("approval"))
    );
}

#[test]
fn a_large_plan_is_shown_shortened_and_approved_in_full() {
    let bench = Bench::new(true);
    // Larger than the 64 KiB the helper used to accept.
    let plan = format!("# Plan\n\n{}", "- a step that matters\n".repeat(12_000));
    let tool_input = json!({"plan": plan});
    bench.append(tool_use("toolu_plan", "ExitPlanMode", &tool_input));
    let input = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name": "ExitPlanMode", "tool_input": tool_input}),
    );
    let child = bench.start(&[], &input);
    let (id, kind, tool, call, stored, meta) = bench.held();
    assert_eq!((kind.as_str(), tool.as_str()), ("approval", "ExitPlanMode"));
    assert_eq!(call, "toolu_plan");
    assert_eq!(meta["truncated"], true);
    let shown = stored["plan"].as_str().unwrap();
    assert!(shown.len() <= 16 * 1024 && plan.starts_with(shown));

    // What the card's "approve plan" decision stores; the helper adds the input it holds.
    bench.respond(
        &id,
        &json!({"aieyesEchoInput": true, "hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow"}}})
        .to_string(),
    );
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert!(printed.get("aieyesEchoInput").is_none());
    assert_eq!(
        printed["hookSpecificOutput"]["decision"]["behavior"],
        "allow"
    );
    assert_eq!(
        printed["hookSpecificOutput"]["decision"]["updatedInput"],
        tool_input
    );
}

/// Hook stdin as Claude Code 2.1.332 sent it in a live session (ids and paths replaced): no
/// `tool_use_id`, and an `ExitPlanMode` input that is empty because the plan file was missing.
#[test]
fn inputs_captured_from_claude_code_are_held_and_answered_in_its_reply_shape() {
    let bench = Bench::new(true);
    let captured = |extra: Value| {
        let mut input = bench.hook_input("PermissionRequest", extra);
        input["prompt_id"] = json!("4ce46cd3-37fb-476e-8a66-2917050783f8");
        input["effort"] = json!({"level": "medium"});
        input
    };

    // An empty plan: the allow must still carry `updatedInput`, here `{}`, and the mode.
    let mut input = captured(json!({"tool_name": "ExitPlanMode", "tool_input": {}}));
    input["permission_mode"] = json!("plan");
    bench.append(tool_use("toolu_exit", "ExitPlanMode", &json!({})));
    let child = bench.start(&[], &input);
    let (id, kind, tool, call, stored, meta) = bench.held();
    assert_eq!((kind.as_str(), tool.as_str()), ("approval", "ExitPlanMode"));
    assert_eq!(call, "toolu_exit");
    assert_eq!(stored, json!({}));
    // Plan mode sends no suggestions.
    assert_eq!(meta["suggestions"], json!([]));
    let mode = json!([{"type": "setMode", "mode": "default", "destination": "session"}]);
    bench.respond(
        &id,
        &json!({"aieyesEchoInput": true, "hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow", "updatedPermissions": mode}}})
        .to_string(),
    );
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert_eq!(
        printed,
        json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow", "updatedInput": {}, "updatedPermissions": mode}}})
    );

    // A question: registered without waiting for the session monitor, answered by its text.
    let questions = json!({"questions": [{"question": "Which color do you prefer?", "header": "Color",
        "options": [{"label": "Red", "description": ""}, {"label": "Blue", "description": ""}],
        "multiSelect": false}]});
    bench.append(tool_use("toolu_ask", "AskUserQuestion", &questions));
    let input = captured(json!({"tool_name": "AskUserQuestion", "tool_input": questions}));
    let child = bench.start(&[], &input);
    let (id, kind, tool, _, stored, _) = bench.held();
    assert_eq!(
        (kind.as_str(), tool.as_str()),
        ("question", "AskUserQuestion")
    );
    assert_eq!(stored, questions);
    let mut answered = questions.clone();
    answered["answers"] = json!({"Which color do you prefer?": "Blue"});
    let reply = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
        "decision": {"behavior": "allow", "updatedInput": answered}}});
    bench.respond(&id, &reply.to_string());
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert_eq!(printed, reply);

    // A command with the always-allow rule the CLI suggested for it.
    let command = json!({"command": "python3 -c \"open(chr(98)+chr(46)+chr(116)+chr(120)+chr(116),chr(119))\"",
        "description": "Create empty b.txt file via Python"});
    let rule = json!({"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": command["command"]}],
        "behavior": "allow", "destination": "localSettings"});
    bench.append(tool_use("toolu_bash", "Bash", &command));
    let input = captured(
        json!({"tool_name": "Bash", "tool_input": command, "permission_suggestions": [rule]}),
    );
    let child = bench.start(&[], &input);
    let (id, .., meta) = bench.held();
    assert_eq!(meta["suggestions"], json!([rule]));
    bench.respond(
        &id,
        &json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "deny", "message": "use make instead"}}})
        .to_string(),
    );
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert_eq!(
        printed["hookSpecificOutput"]["decision"],
        json!({"behavior": "deny", "message": "use make instead"})
    );
}

#[test]
fn an_mcp_form_is_held_in_field_order_and_its_reply_is_printed() {
    let bench = Bench::new(false);
    bench.append(tool_use("toolu_mcp", "mcp__ops__deploy", &json!({})));
    // Written by hand: the order of `properties` is the order the form is shown in, and
    // serialising a parsed value would sort it.
    let input = format!(
        r#"{{"session_id":"{NATIVE}","transcript_path":{},"cwd":"/work","hook_event_name":"Elicitation",
            "mcp_server_name":"ops","message":"Deploy where?","mode":"form","elicitation_id":"el-7",
            "requested_schema":{{"type":"object","required":["zone"],"properties":{{
                "zone":{{"type":"string","enum":["eu","us"]}},
                "count":{{"type":"integer","minimum":1}},
                "announce":{{"type":"boolean"}}}}}}}}"#,
        json!(bench.transcript)
    );
    let child = bench.start_text(&[], &input);
    let (id, kind, tool, call, stored, _) = bench.held();
    assert_eq!(
        (kind.as_str(), tool.as_str(), call.as_str()),
        ("elicitation", "Elicitation", "el-7")
    );
    assert_eq!(stored["server"], "ops");
    assert_eq!(stored["message"], "Deploy where?");
    assert_eq!(stored["order"], json!(["zone", "count", "announce"]));

    let reply = json!({"hookSpecificOutput": {"hookEventName": "Elicitation", "action": "accept",
        "content": {"zone": "eu", "count": 2}}});
    bench.respond(&id, &reply.to_string());
    let printed: Value = serde_json::from_str(&bench.output(child)).unwrap();
    assert_eq!(printed, reply);

    // A form that opens a link stays in the terminal.
    let url = bench.hook_input(
        "Elicitation",
        json!({"mcp_server_name": "ops", "message": "Sign in", "mode": "url", "url": "https://example.com/auth"}),
    );
    assert_eq!(bench.output(bench.start(&[], &url)), "");
    assert!(bench.row().is_none());
}

#[test]
fn a_request_answered_in_the_terminal_releases_the_helper() {
    let bench = Bench::new(true);
    let tool_input = json!({"command": "cargo test"});
    bench.append(tool_use("toolu_9", "Bash", &tool_input));
    let input = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": tool_input}),
    );
    let child = bench.start(&[], &input);
    let (_, _, _, call, _, _) = bench.held();
    assert_eq!(call, "toolu_9");
    // Claude Code does not signal the hook; it just writes the tool result.
    bench.append(json!({"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"toolu_9","content":"ok"}]}}));
    assert_eq!(bench.output(child), "");
    assert!(bench.rows().is_empty());

    // Handed back from the card: the helper leaves without a decision.
    bench.append(tool_use("toolu_10", "Bash", &tool_input));
    let child = bench.start(&[], &input);
    let (id, ..) = bench.held();
    bench.respond(&id, "__aieyes_passthrough__");
    assert_eq!(bench.output(child), "");
}

#[test]
fn requests_the_helper_must_not_take_are_left_to_the_agent() {
    let bench = Bench::new(false);
    let approval = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
    );
    // Tool approvals are a separate opt-in.
    assert_eq!(bench.output(bench.start(&[], &approval)), "");
    assert!(bench.row().is_none());

    let bench = Bench::new(true);
    let approval = bench.hook_input(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
    );
    // A headless run has nobody at the terminal to fall back to.
    assert_eq!(bench.output(bench.start(&["-p"], &approval)), "");
    // Another session's transcript, an unknown event and malformed input bind nothing.
    let mut foreign = approval.clone();
    foreign["transcript_path"] = json!(bench.dir.path().join("elsewhere.jsonl"));
    fs::write(bench.dir.path().join("elsewhere.jsonl"), "").unwrap();
    assert_eq!(bench.output(bench.start(&[], &foreign)), "");
    let mut renamed = approval.clone();
    renamed["session_id"] = json!("11111111-2222-3333-4444-555555555555");
    assert_eq!(bench.output(bench.start(&[], &renamed)), "");
    let mut other = approval.clone();
    other["hook_event_name"] = json!("PreToolUse");
    assert_eq!(bench.output(bench.start(&[], &other)), "");
    assert_eq!(bench.output(bench.start_text(&[], "{not json")), "");
    assert!(bench.rows().is_empty());
}
