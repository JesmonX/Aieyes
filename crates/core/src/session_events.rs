//! Provider-neutral lifecycle reduction. No prompts, tool arguments or output are retained.
use crate::sessions::Phase;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Event {
    pub kind: String,
    pub session: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_source: String,
    /// `interactive`, `headless` or `internal` from process argv, hook payload
    /// or `AIEYES_INTERNAL`. Empty means no signal.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub classification: String,
    /// Codex `session_meta.originator`; only `codex_work_desktop` changes the
    /// displayed provider. Headless rules still read `session_source` only.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub originator: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent_session: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent_session_path: String,
    pub turn: String,
    pub record: String,
    pub parent: String,
    pub call: String,
    pub tool: String,
    pub at: u64,
    pub pid: u32,
    pub process_start: u64,
    pub fully_idle: Option<bool>,
    pub failed: bool,
    pub interrupted: bool,
    pub notification: String,
}
fn field(v: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|k| v[*k].as_str())
        .unwrap_or("")
        .chars()
        .take(256)
        .collect()
}
pub const HEADLESS: &str = "headless";
pub const INTERACTIVE: &str = "interactive";
pub const INTERNAL: &str = "internal";
pub const UNKNOWN: &str = "unknown";
/// Session source of processes Aieyes spawns itself (`AIEYES_INTERNAL=1`).
pub const INTERNAL_SOURCE: &str = "aieyes-internal";

/// Executable basenames and npm package directories that identify a provider CLI.
fn programs(provider: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match provider {
        "claude" => (&["claude"], &["@anthropic-ai/claude-code"]),
        "codex" => (&["codex"], &["@openai/codex"]),
        "deepseek-harness" => (&["dsh"], &["@deepseek-ai/dsh"]),
        "kimi" => (
            &["kimi", "kimi-cli", "kimi-code"],
            &["@moonshot-ai/kimi-code"],
        ),
        "cursor" => (&["cursor-agent", "agent"], &[]),
        "opencode" => (&["opencode"], &["opencode-ai", "@opencode/cli"]),
        "codebuddy" => (&["codebuddy"], &["@tencent-ai/codebuddy-code"]),
        "antigravity" => (&["agy"], &[]),
        "qoder" => (&["qodercli", "qoder"], &["@qoder-ai/qodercli"]),
        "qoder-cn" => (
            &["qoderclicn", "qodercn", "qoder-cn"],
            &["@qoder-ai/qoderclicn"],
        ),
        "zcode" => (&["zcode"], &[]),
        "grok" => (&["grok"], &[]),
        "pi" => (
            &["pi"],
            &[
                "@mariozechner/pi-coding-agent",
                "@earendil-works/pi-coding-agent",
            ],
        ),
        "omp" => (&["omp"], &["@oh-my-pi/pi-coding-agent"]),
        "copilot" => (&["copilot"], &["@github/copilot"]),
        "kiro" => (&["kiro-cli"], &[]),
        "cline" => (&["cline"], &["cline", "@cline/cli"]),
        "kilo" => (&["kilo", "kilocode"], &["@kilocode/cli"]),
        // The `trae-agent` console script, or `python -m trae_agent.cli`.
        "trae" | "trae-cn" => (&["trae-cli", "trae_agent.cli"], &[]),
        _ => (&[], &[]),
    }
}
fn program_index(provider: &str, argv: &[String]) -> Option<usize> {
    let (names, packages) = programs(provider);
    // Interpreters (node, bun, python, uv) precede the script within a few tokens.
    argv.iter().take(4).position(|token| {
        let token = token.replace('\\', "/");
        let base = token.rsplit('/').next().unwrap_or("").to_lowercase();
        let base = [".exe", ".cmd", ".js", ".mjs", ".cjs"]
            .iter()
            .find_map(|x| base.strip_suffix(x))
            .unwrap_or(&base);
        names.contains(&base)
            || packages
                .iter()
                .any(|p| token.contains(&format!("node_modules/{p}/")))
    })
}
/// Whether `argv` runs the provider's own CLI (its rule table applies).
pub(crate) fn is_program(provider: &str, argv: &[String]) -> bool {
    program_index(provider, argv).is_some()
}

/// Parse only CLI option tokens. Prompt text and values such as a model named
/// `acp` must never become flags or subcommands themselves.
pub(crate) struct CliArgs<'a> {
    options: Vec<(&'a str, Option<&'a str>)>,
    pub(crate) positional: Vec<&'a str>,
}
impl<'a> CliArgs<'a> {
    pub(crate) fn value(&self, flags: &[&str]) -> Option<&'a str> {
        self.options
            .iter()
            .find_map(|(key, value)| flags.contains(key).then_some(*value).flatten())
    }
    pub(crate) fn has(&self, flags: &[&str]) -> bool {
        self.options.iter().any(|(key, _)| flags.contains(key))
    }
}
pub(crate) fn cli_args<'a>(provider: &str, argv: &'a [String]) -> Option<CliArgs<'a>> {
    let args = &argv[program_index(provider, argv)? + 1..];
    let mut parsed = CliArgs {
        options: Vec::new(),
        positional: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        if arg == "--" {
            parsed
                .positional
                .extend(args[index..].iter().map(String::as_str));
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            parsed.positional.push(arg);
            continue;
        }
        if let Some((key, value)) = arg.split_once('=') {
            parsed.options.push((key, Some(value)));
            continue;
        }
        let takes_value = matches!(
            arg,
            "--model"
                | "-m"
                | "--provider"
                | "--api-key"
                | "--config"
                | "--config-file"
                | "--config-dir"
                | "--cwd"
                | "--directory"
                | "--workdir"
                | "--work-dir"
                | "-C"
                | "--profile"
                | "--output-format"
                | "--input-format"
                | "--format"
                | "--mode"
                | "--session"
                | "--session-id"
                | "--session-dir"
                | "--resume"
                | "-S"
                | "--log-level"
                | "--log-file"
                | "--agent"
                | "--agent-file"
                | "--add-dir"
                | "--tools"
                | "--allowed-tools"
                | "--allowedTools"
                | "--system-prompt"
                | "--append-system-prompt"
                | "--prompt"
                | "--prompt-interactive"
                | "--target"
                | "--print-timeout"
                | "--json-schema"
                | "--max-turns"
                | "--setting-sources"
                | "--thinking"
                | "--theme"
                | "--title"
                | "--hostname"
                | "--port"
                | "--trajectory-file"
        ) || (arg == "-p" && !matches!(provider, "pi" | "omp"))
            || (arg == "-o" && crate::qoder_family::is_qoder(provider))
            || (arg == "-i" && provider == "antigravity")
            || (arg == "-t" && matches!(provider, "trae" | "trae-cn"));
        let value = takes_value
            .then(|| args.get(index).map(String::as_str))
            .flatten();
        if value.is_some() {
            index += 1;
        }
        parsed.options.push((arg, value));
    }
    Some(parsed)
}
/// The headless rule table (research doc (c)). `None` means the argv carries no
/// signal; the session is then counted as `unknown`.
pub fn headless_argv(provider: &str, argv: &[String]) -> Option<&'static str> {
    let args = cli_args(provider, argv)?;
    let has = |flags: &[&str]| args.has(flags);
    let value = |flag: &str| args.value(&[flag]);
    let command = args.positional.first().copied();
    let verdict = |headless: bool| Some(if headless { HEADLESS } else { INTERACTIVE });
    match provider {
        "claude" => verdict(has(&["-p", "--print"])),
        "codebuddy" => verdict(has(&["-p", "--print", "--acp"])),
        "codex" => matches!(command, Some("exec" | "e" | "mcp-server")).then_some(HEADLESS),
        "deepseek-harness" => {
            let profile = value("--profile").or(command);
            match profile {
                Some("headless" | "sdk" | "acp") => Some(HEADLESS),
                Some("tui" | "web" | "desktop") => Some(INTERACTIVE),
                _ => None,
            }
        }
        "kimi" => verdict(
            has(&["-p", "--prompt"])
                || matches!(command, Some("acp" | "web" | "server" | "rc" | "remote")),
        ),
        "cursor" => verdict(has(&["-p", "--print"])),
        "opencode" => match command {
            Some("run" | "acp" | "api") => Some(HEADLESS),
            // The TUI's background service hosts TUI sessions; the TUI client classifies them.
            Some("serve") if has(&["--service"]) => None,
            Some("serve") => Some(HEADLESS),
            None => Some(INTERACTIVE),
            _ => None,
        },
        "antigravity" => {
            if has(&["-i", "--prompt-interactive"]) {
                Some(INTERACTIVE)
            } else if has(&[
                "-p",
                "--print",
                "--prompt",
                "--output-format",
                "--input-format",
                "--print-timeout",
                "--json-schema",
            ]) || matches!(command, Some("remote-control" | "mic-serve"))
            {
                Some(HEADLESS)
            } else {
                None
            }
        }
        "qoder" | "qoder-cn" => verdict(has(&[
            "-p",
            "--print",
            "-o",
            "--output-format",
            "--remote-control",
        ])),
        "zcode" => (has(&["-p", "--prompt", "--target"]) || command == Some("app-server"))
            .then_some(HEADLESS),
        "grok" => has(&["-p", "--single", "--output-format", "--json-schema"]).then_some(HEADLESS),
        "pi" | "omp" => (has(&["-p", "--print"])
            || matches!(value("--mode"), Some("json" | "rpc")))
        .then_some(HEADLESS),
        "copilot" => has(&["-p", "--prompt", "--acp"]).then_some(HEADLESS),
        // kiro-cli 2.36: `--output-format stream-json` implies `--no-interactive`.
        "kiro" => (command == Some("chat")
            && (has(&["--no-interactive", "--non-interactive"])
                || value("--output-format") == Some("stream-json")))
        .then_some(HEADLESS),
        "cline" => {
            if has(&["-i", "--tui"]) {
                Some(INTERACTIVE)
            } else if has(&["--json", "-z", "--zen", "--oneshot", "-y", "--yolo"])
                || command.is_some()
            {
                // A positional prompt without -i, or `schedule`, runs unattended.
                Some(HEADLESS)
            } else {
                None
            }
        }
        "kilo" => {
            if matches!(command, Some("run" | "acp"))
                || has(&["--auto"])
                || value("--format") == Some("json")
            {
                Some(HEADLESS)
            } else {
                None
            }
        }
        "trae" | "trae-cn" => match command {
            Some("run") => Some(HEADLESS),
            Some("interactive") => Some(INTERACTIVE),
            _ => args
                .options
                .iter()
                .any(|(a, _)| a.starts_with("--docker-") || *a == "--must-patch")
                .then_some(HEADLESS),
        },
        _ => None,
    }
}
/// Creation class of a session: `internal` (Aieyes spawned it) and `headless`
/// are not monitored; `unknown` (no signal) is counted.
pub fn classify(provider: &str, activity: &Activity) -> &'static str {
    let source = activity.session_source.as_str();
    if source == INTERNAL_SOURCE || activity.classification == INTERNAL {
        return INTERNAL;
    }
    // Explicit log fields outrank process argv.
    match provider {
        "claude"
            if matches!(
                source,
                "sdk-cli" | "sdk-ts" | "sdk-py" | "mcp" | "claude-code-github-action"
            ) =>
        {
            return HEADLESS;
        }
        "claude" if !source.is_empty() => return INTERACTIVE,
        "codex"
            if activity.parent_session.is_empty() && !matches!(source, "" | "cli" | "vscode") =>
        {
            return HEADLESS;
        }
        "codex" if matches!(source, "cli" | "vscode") => return INTERACTIVE,
        _ => {}
    }
    match activity.classification.as_str() {
        HEADLESS => HEADLESS,
        INTERACTIVE => INTERACTIVE,
        _ => UNKNOWN,
    }
}

pub fn hook(provider: &str, kind: &str, input: &Value, at: u64) -> Event {
    if provider == "cursor" {
        let terminal = |status: &str| match status {
            "completed" => "complete",
            "error" => "failed",
            "aborted" | "window_close" | "user_close" => "interrupted",
            _ => "unknown",
        };
        let kind = match kind {
            "sessionStart" => "SessionStart",
            "beforeSubmitPrompt" => "start",
            "preToolUse" => "PreToolUse",
            "postToolUse" => "PostToolUse",
            "postToolUseFailure" => "PostToolUseFailure",
            "afterAgentThought" => "thinking",
            "stop" => terminal(input["status"].as_str().unwrap_or("")),
            "sessionEnd" => terminal(input["reason"].as_str().unwrap_or("")),
            _ => "unknown",
        };
        let tool = field(input, &["tool_name"]);
        return Event {
            kind: kind.into(),
            at,
            // Cloud background agents are not local conversations.
            classification: if input["is_background_agent"] == true {
                HEADLESS.into()
            } else {
                String::new()
            },
            session: field(input, &["conversation_id", "session_id"]),
            turn: field(input, &["generation_id"]),
            call: field(input, &["tool_use_id", "tool_call_id"]),
            tool: if tool == "AskQuestion" {
                "ask_question".into()
            } else {
                tool
            },
            ..Event::default()
        };
    }
    let mut event = Event {
        kind: kind.into(),
        at,
        session_source: if provider == "claude" {
            field(input, &["entrypoint"])
        } else {
            String::new()
        },
        session: field(input, &["session_id", "conversationId", "sessionId"]),
        parent_session: field(input, &["parent_session_id", "parentSessionId"]),
        turn: field(input, &["turn_id", "turnId", "prompt_id"]),
        call: field(
            input,
            &["tool_use_id", "toolUseId", "agent_id", "elicitation_id"],
        ),
        tool: if provider == "antigravity" {
            field(&input["toolCall"], &["name"])
        } else {
            field(input, &["tool_name"])
        },
        fully_idle: input["fullyIdle"].as_bool().or_else(|| {
            (provider == "codebuddy")
                .then(|| input["background_tasks"].as_array().map(Vec::is_empty))
                .flatten()
        }),
        failed: kind == "StopFailure"
            || (kind == "Stop"
                && (input["terminationReason"] == "error"
                    || input["error"].as_str().is_some_and(|s| !s.is_empty()))),
        interrupted: input["terminationReason"]
            .as_str()
            .is_some_and(|s| s.contains("cancel") || s.contains("interrupt")),
        notification: field(input, &["notification_type"]),
        ..Event::default()
    };
    if matches!(provider, "codebuddy" | "claude")
        && !matches!(kind, "SubagentStart" | "SubagentStop")
        && let Some(agent) = input["agent_id"].as_str().filter(|s| !s.is_empty())
    {
        event.parent_session = event.session.clone();
        event.session = format!("{}:{agent}", event.session);
    }
    if kind == "PostToolUseFailure" && input["is_interrupt"] == true {
        event.kind = "interrupted".into();
        event.interrupted = true;
    }
    event
}

/// ChatGPT Work threads share Codex logs, account and quota pool.
pub const CODEX_WORK_ORIGINATOR: &str = "codex_work_desktop";

/// Provider shown for a session. Identity and parent hashes keep the log provider.
pub fn display_provider<'a>(provider: &'a str, activity: &Activity) -> &'a str {
    if provider == "codex" && activity.originator == CODEX_WORK_ORIGINATOR {
        "chatgpt-work"
    } else {
        provider
    }
}

pub fn codex(row: &Value) -> Option<Event> {
    let p = &row["payload"];
    let kind = match (row["type"].as_str()?, p["type"].as_str().unwrap_or("")) {
        ("session_meta", _) => "identity",
        ("event_msg", "task_started") => "start",
        ("event_msg", "task_complete") => "complete",
        ("event_msg", "turn_aborted") => "interrupted",
        ("event_msg", "error") => "failed",
        ("response_item", "reasoning") => "thinking",
        ("response_item", "function_call" | "custom_tool_call") => "tool",
        ("response_item", "function_call_output" | "custom_tool_call_output") => "output",
        _ => return None,
    };
    Some(Event {
        kind: kind.into(),
        session: if kind == "identity" {
            field(p, &["id"])
        } else {
            String::new()
        },
        session_source: if kind == "identity" {
            field(p, &["source"])
        } else {
            String::new()
        },
        originator: if kind == "identity" {
            field(p, &["originator"])
        } else {
            String::new()
        },
        parent_session: if kind == "identity" {
            field(
                &p["source"]["subagent"]["thread_spawn"],
                &["parent_thread_id"],
            )
        } else {
            String::new()
        },
        turn: field(p, &["turn_id"]),
        call: field(p, &["call_id"]),
        tool: field(p, &["name"]),
        at: row["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis().max(0) as u64)
            .unwrap_or(0),
        ..Event::default()
    })
}

/// Native lifecycle records. Only identifiers and state cross this boundary.
pub fn native(provider: &str, row: &Value) -> Option<Event> {
    if provider == "deepseek-harness" && row["_aieyesInherited"] == true {
        return None;
    }
    let mut event = Event {
        parent_session: field(row, &["parentSessionId", "parent_session_id"]),
        ..Default::default()
    };
    let kind = match provider {
        "kimi" => {
            event.at = row["time"].as_u64().unwrap_or(0);
            event.turn = row["turnId"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default();
            match row["type"].as_str() {
                Some("interaction.request" | "interaction.resolved") => {
                    event.call = field(row, &["id", "interactionId"]);
                    event.tool = field(row, &["toolName"]);
                    if row["type"] == "interaction.request" {
                        "Elicitation"
                    } else {
                        "ElicitationResult"
                    }
                }
                Some("turn.prompt") => "start",
                Some("llm.request") => "thinking",
                Some("turn.ended") => match row["reason"].as_str() {
                    Some("completed") => "complete",
                    Some("interrupted" | "cancelled" | "canceled") => "interrupted",
                    Some("error" | "failed") => "failed",
                    Some("blocked") => "PermissionRequest",
                    _ => return None,
                },
                Some("context.append_loop_event") => match row["event"]["type"].as_str() {
                    Some("tool.call") => "tool",
                    Some("tool.result") => "output",
                    _ => return None,
                },
                _ => {
                    event.at = row["timestamp"]
                        .as_f64()
                        .map(|v| (v * 1000.) as u64)
                        .unwrap_or(0);
                    match row["message"]["type"].as_str()? {
                        "TurnBegin" => "start",
                        "StepBegin" => "thinking",
                        "ToolCall" => "tool",
                        "ToolResult" => "output",
                        "TurnEnd" => "complete",
                        "TurnInterrupted" => "interrupted",
                        _ => return None,
                    }
                }
            }
        }
        "grok" => {
            event.at = row["ts"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis().max(0) as u64)
                .unwrap_or(0);
            event.session = field(row, &["session_id"]);
            event.turn = row["turn_number"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default();
            match row["type"].as_str()? {
                "turn_started" => "start",
                "phase_changed" if row["phase"] == "waiting_for_model" => "thinking",
                "phase_changed" if row["phase"] == "tool_execution" => "tool_phase",
                "tool_started" => "tool",
                "tool_completed" | "tool_finished" | "tool_failed" => "output",
                "permission_requested"
                | "permission_resolved"
                | "permission_response"
                | "permission_responded" => {
                    event.call = field(
                        row,
                        &["permission_id", "request_id", "tool_call_id", "call_id"],
                    );
                    if row["type"] == "permission_requested" {
                        "PermissionRequest"
                    } else {
                        "ElicitationResult"
                    }
                }
                "turn_ended" => match row["outcome"].as_str()? {
                    "completed" => "complete",
                    "cancelled" | "interrupted" => "interrupted",
                    "failed" | "error" => "failed",
                    _ => return None,
                },
                _ => return None,
            }
        }
        "codebuddy" => {
            event.at = row["timestamp"].as_u64().unwrap_or(0);
            event.session = field(row, &["sessionId"]);
            event.turn = field(&row["providerData"], &["conversationRequestId"]);
            event.record = field(row, &["id"]);
            event.parent = field(row, &["parentId"]);
            if row["isSidechain"] == true
                && let Some(agent) = row["agentId"].as_str().filter(|s| !s.is_empty())
            {
                event.parent_session = event.session.clone();
                event.session = format!("{}:{agent}", event.session);
            }
            if row["type"] == "message" && row["role"] == "user" && event.turn.is_empty() {
                event.turn = event.record.clone();
            }
            match (row["type"].as_str()?, row["role"].as_str()) {
                ("message", Some("user")) => "start",
                ("reasoning", _) => "thinking",
                ("function_call", _) => "tool",
                ("message" | "function_call_output" | "function_call_result", _)
                    if row["status"] == "incomplete"
                        && row["providerData"]["skipRun"] == true
                        && row["providerData"]["error"]["message"] == "Interrupted by user" =>
                {
                    "interrupted"
                }
                ("function_call_output" | "function_call_result", _) => "output",
                ("message", Some("assistant"))
                    if row["status"] == "incomplete"
                        && row["providerData"]["skipRun"] == true
                        && row["providerData"]["error"].is_object() =>
                {
                    // Native cancellation persists this fixed diagnostic,
                    // then emits a generic Stop without an outcome flag.
                    if row["providerData"]["error"]["message"] == "Interrupted by user" {
                        "interrupted"
                    } else if row["providerData"]["error"]["isRetryable"] == false {
                        "failed"
                    } else {
                        "unknown"
                    }
                }
                ("message", Some("assistant")) if row["status"] == "completed" => {
                    // Current CodeBuddy persists commentary as a completed
                    // message before its following tool call. Only the final
                    // message carries this model step's settled usage.
                    let p = &row["providerData"];
                    if p["messageId"].is_string() && p["rawUsage"].is_null() && p["usage"].is_null()
                    {
                        "thinking"
                    } else {
                        "complete"
                    }
                }
                _ if !event.record.is_empty() => "identity",
                _ => return None,
            }
        }
        // Copilot CLI `session-state/<id>/events.jsonl`. `assistant.turn_end` closes one
        // model call, not the user's turn; a reply without tool requests does.
        "copilot" => {
            let data = &row["data"];
            event.at = row["timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis().max(0) as u64)
                .unwrap_or(0);
            event.turn = field(data, &["interactionId"]);
            match row["type"].as_str()? {
                "session.start" | "session.resume" => {
                    event.session = field(data, &["sessionId"]);
                    "identity"
                }
                // `copilot -p` writes this fixed system prompt (observed in 1.0.105).
                "system.message"
                    if data["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("You are running in non-interactive mode")) =>
                {
                    event.classification = HEADLESS.into();
                    "identity"
                }
                "user.message" => "start",
                "assistant.turn_start" => "thinking",
                "assistant.message"
                    if data["toolRequests"].as_array().is_none_or(|a| a.is_empty()) =>
                {
                    "complete"
                }
                "tool.execution_start" => "tool",
                "tool.execution_complete" => "output",
                "abort" | "session.abort" | "assistant.abort" => "interrupted",
                "session.error" => "failed",
                _ => return None,
            }
        }
        "deepseek-harness" => {
            event.at = row["time"].as_u64().unwrap_or(0);
            event.turn = row["data"]["turn"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default();
            match row["type"].as_str()? {
                "session" => {
                    event.session = field(row, &["id"]);
                    "identity"
                }
                "turn/start" => "start",
                "step/start" => "thinking",
                "tool/call" => "tool",
                "tool/result" => "output",
                "turn/end" => match row["data"]["reason"]["kind"].as_str() {
                    Some("completed") => "complete",
                    Some("cancelled" | "interrupted" | "aborted") => "interrupted",
                    Some("failed" | "error" | "max-tokens") => "failed",
                    Some("blocked") => "PermissionRequest",
                    _ => return None,
                },
                _ => return None,
            }
        }
        _ => return None,
    };
    if kind == "tool" || kind == "output" {
        let p = if provider == "kimi" {
            &row["event"]
        } else if matches!(provider, "deepseek-harness" | "copilot") {
            &row["data"]
        } else {
            row
        };
        event.call = field(
            p,
            &["toolCallId", "tool_call_id", "callId", "call_id", "id"],
        );
        event.tool = field(p, &["name", "tool_name", "toolName"]);
    }
    event.kind = if provider == "grok" && kind == "tool" && event.call.is_empty() {
        "anonymous_tool"
    } else {
        kind
    }
    .into();
    Some(event)
}
/// Pi-family transcripts preserve tool calls inside assistant content arrays.
pub fn pi(row: &Value) -> Vec<Event> {
    let at = row["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis().max(0) as u64)
        .unwrap_or(0);
    let base = Event {
        at,
        record: field(row, &["id"]),
        parent: field(row, &["parentId"]),
        parent_session: field(row, &["parentSessionId", "parent_session_id"]),
        parent_session_path: if row["type"] == "session" {
            field(row, &["parentSession"])
        } else {
            String::new()
        },
        ..Default::default()
    };
    if row["type"] == "session" {
        return vec![Event {
            kind: "identity".into(),
            session: field(row, &["id"]),
            ..base
        }];
    }
    if row["type"] == "custom" && row["customType"] == "tool_execution_start" {
        return vec![Event {
            kind: "tool".into(),
            call: field(&row["data"], &["toolCallId"]),
            tool: field(&row["data"], &["toolName"]),
            ..base
        }];
    }
    if row["type"] == "custom" && row["customType"] == "session_exit" {
        let data = &row["data"];
        let kind = match data["kind"].as_str() {
            Some("fatal") => "failed",
            Some("signal") => "interrupted",
            Some("normal")
                if data["pendingToolCalls"]
                    .as_array()
                    .is_none_or(Vec::is_empty) =>
            {
                "complete"
            }
            _ => "interrupted",
        };
        return vec![Event {
            kind: kind.into(),
            ..base
        }];
    }
    if row["type"] != "message" {
        return vec![Event {
            kind: "identity".into(),
            ..base
        }];
    }
    let message = &row["message"];
    match message["role"].as_str() {
        Some("user") => vec![Event {
            kind: "start".into(),
            turn: base.record.clone(),
            ..base
        }],
        Some("toolResult") => vec![Event {
            kind: "output".into(),
            call: field(message, &["toolCallId"]),
            tool: field(message, &["toolName"]),
            ..base
        }],
        Some("assistant") => {
            let mut events = Vec::new();
            for part in message["content"].as_array().into_iter().flatten() {
                if part["type"] == "toolCall" {
                    events.push(Event {
                        kind: "tool".into(),
                        call: field(part, &["id"]),
                        tool: field(part, &["name"]),
                        ..base.clone()
                    });
                }
            }
            let kind = match message["stopReason"].as_str() {
                Some("stop") => Some("complete"),
                Some("error" | "length") => Some("failed"),
                Some("aborted") => Some("interrupted"),
                Some("toolUse") => None,
                _ => Some("unknown"),
            };
            if let Some(kind) = kind {
                events.push(Event {
                    kind: kind.into(),
                    ..base
                });
            }
            events
        }
        _ => vec![Event {
            kind: "identity".into(),
            ..base
        }],
    }
}

/// Read-only Claude recovery when hooks were not present at session start.
pub fn claude(row: &Value) -> Vec<Event> {
    let session = field(row, &["sessionId"]);
    let at = row["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis().max(0) as u64)
        .unwrap_or(0);
    let mut base = Event {
        session,
        session_source: field(row, &["entrypoint"]),
        at,
        parent_session: field(row, &["parentSessionId", "parent_session_id"]),
        ..Event::default()
    };
    if row["isSidechain"] == true
        && let Some(agent) = row["agentId"].as_str().filter(|s| !s.is_empty())
    {
        base.parent_session = base.session.clone();
        base.session = format!("{}:{agent}", base.session);
    }
    let content = row["message"]["content"].as_array();
    let mut out = Vec::new();
    if !base.session_source.is_empty() {
        out.push(Event {
            kind: "identity".into(),
            ..base.clone()
        });
    }
    match row["type"].as_str() {
        Some("user") => {
            let results: Vec<_> = content
                .into_iter()
                .flatten()
                .filter(|v| v["type"] == "tool_result")
                .collect();
            if results.is_empty() && row["isMeta"] != true {
                out.push(Event {
                    kind: "start".into(),
                    turn: field(row, &["uuid"]),
                    ..base
                });
            } else {
                for r in results {
                    out.push(Event {
                        kind: "output".into(),
                        call: field(r, &["tool_use_id"]),
                        ..base.clone()
                    });
                }
            }
        }
        Some("assistant") => {
            if row["isApiErrorMessage"] == true {
                return vec![Event {
                    kind: "failed".into(),
                    ..base
                }];
            }
            for part in content.into_iter().flatten() {
                let kind = match part["type"].as_str() {
                    Some("thinking") => "thinking",
                    Some("tool_use") => "tool",
                    _ => continue,
                };
                out.push(Event {
                    kind: kind.into(),
                    tool: field(part, &["name"]),
                    call: field(part, &["id"]),
                    ..base.clone()
                });
            }
            if row["message"]["stop_reason"] == "end_turn" {
                out.push(Event {
                    kind: "complete".into(),
                    ..base
                });
            }
        }
        _ => {}
    }
    out
}

#[derive(Default, Clone)]
pub struct Activity {
    pub phase: Option<Phase>,
    pub session: String,
    pub session_source: String,
    pub classification: String,
    pub originator: String,
    pub parent_session: String,
    pub parent_session_path: String,
    pub turn: String,
    pub at: u64,
    pub pid: u32,
    pub process_start: u64,
    waiting: HashSet<String>,
    tools: HashSet<String>,
    anonymous_tools: u16,
    background: HashSet<String>,
    generation: u64,
    turn_started: u64,
    pending_stop: bool,
    external_background: bool,
    record_turns: HashMap<String, String>,
    record_order: VecDeque<String>,
}
impl Activity {
    pub fn turn_started_at(&self) -> u64 {
        self.turn_started
    }
    pub fn background_active(&self) -> bool {
        self.external_background || !self.background.is_empty()
    }
    pub fn apply(&mut self, mut e: Event) {
        // Older CodeBuddy prompts have no conversationRequestId. Model steps
        // acquire one later; the native parent chain ties them to the prompt,
        // including late events from an earlier turn.
        if !matches!(e.kind.as_str(), "start" | "UserPromptSubmit")
            && let Some(turn) = self.record_turns.get(&e.parent)
        {
            e.turn = turn.clone();
        }
        if !e.record.is_empty() && !e.turn.is_empty() {
            if !self.record_turns.contains_key(&e.record) {
                self.record_order.push_back(e.record.clone());
            }
            self.record_turns.insert(e.record.clone(), e.turn.clone());
            while self.record_order.len() > 256 {
                if let Some(old) = self.record_order.pop_front() {
                    self.record_turns.remove(&old);
                }
            }
        }
        if !e.session.is_empty() {
            self.session = e.session.clone();
        }
        if !e.parent_session.is_empty() {
            self.parent_session = e.parent_session.clone();
        }
        if !e.parent_session_path.is_empty() {
            self.parent_session_path = e.parent_session_path.clone();
        }
        if !e.session_source.is_empty() {
            self.session_source = e.session_source.clone();
        }
        if !e.originator.is_empty() {
            self.originator = e.originator.clone();
        }
        // Events without a signal keep the last one. An Aieyes-internal run stays
        // internal; otherwise the newest process (e.g. an interactive resume) wins.
        if !e.classification.is_empty() && self.classification != INTERNAL {
            self.classification = e.classification.clone();
        }
        if e.pid > 0 {
            self.pid = e.pid;
            self.process_start = e.process_start;
        }
        if e.kind == "identity" {
            return;
        }
        if e.at > 0 && e.at < self.at.max(self.turn_started) {
            return;
        }
        let starting = matches!(e.kind.as_str(), "start" | "UserPromptSubmit");
        if starting {
            if !e.turn.is_empty() && self.turn == e.turn {
                return;
            }
            self.generation += 1;
            self.turn = if e.turn.is_empty() {
                format!("{}-{}", e.at, self.generation)
            } else {
                e.turn.clone()
            };
            self.turn_started = e.at;
            self.waiting.clear();
            self.tools.clear();
            self.anonymous_tools = 0;
            self.background.clear();
            self.pending_stop = false;
            self.external_background = false;
            self.phase = Some(Phase::Working);
        } else {
            if !e.turn.is_empty() && !self.turn.is_empty() && self.turn != e.turn {
                return;
            }
            if e.kind == "SessionStart" {
                self.phase = None;
                self.waiting.clear();
                self.tools.clear();
                self.anonymous_tools = 0;
                self.background.clear();
                self.turn.clear();
                self.turn_started = e.at;
                self.pending_stop = false;
                self.external_background = false;
            } else if e.kind == "PreInvocation" {
                // Each invocation may belong to the same AGY turn. A model call is not a new user turn.
                if self.phase.is_none_or(Phase::terminal) {
                    self.generation += 1;
                    self.turn = format!("{}-{}", e.at, self.generation);
                    self.turn_started = e.at;
                    self.phase = Some(Phase::Working);
                }
            } else if !self.phase.is_some_and(Phase::terminal) {
                match e.kind.as_str() {
                    "unknown" => self.phase = Some(Phase::Unknown),
                    "thinking" => self.phase = Some(Phase::Thinking),
                    "tool_phase" => self.phase = Some(Phase::Tool),
                    "tool" | "anonymous_tool" | "PreToolUse" => {
                        if !e.call.is_empty() {
                            self.tools.insert(e.call.clone());
                        } else if e.kind == "anonymous_tool" {
                            self.anonymous_tools = self.anonymous_tools.saturating_add(1).min(256);
                        }
                        let tool = e.tool.rsplit(['.', ':']).next().unwrap_or("");
                        if matches!(
                            tool,
                            "request_user_input"
                                | "AskUserQuestion"
                                | "ask_question"
                                | "ask_permission"
                        ) {
                            self.waiting.insert(e.call.clone());
                        }
                        self.phase = Some(Phase::Tool);
                    }
                    "PermissionRequest" | "Elicitation" => {
                        self.waiting.insert(e.call.clone());
                        self.phase = Some(Phase::NeedsInput);
                    }
                    "Notification"
                        if matches!(
                            e.notification.as_str(),
                            "permission_prompt" | "elicitation_dialog" | "idle_prompt"
                        ) =>
                    {
                        self.waiting.insert("notification".into());
                    }
                    "output" | "PostToolUse" | "PostToolUseFailure" | "PermissionDenied"
                    | "ElicitationResult" => {
                        self.waiting.remove(&e.call);
                        // CodeBuddy's permission hook may omit the call ID;
                        // its subsequent tool result still answers that prompt.
                        if e.kind != "ElicitationResult" {
                            self.waiting.remove("");
                        }
                        self.waiting.remove("notification");
                        if e.kind != "ElicitationResult" && !self.tools.remove(&e.call) {
                            // Grok only assigns a call ID at completion.
                            self.anonymous_tools = self.anonymous_tools.saturating_sub(1);
                        }
                        // An individual tool failure is recoverable. Only a terminal event can fail a turn.
                        self.phase = Some(if self.tools.is_empty() && self.anonymous_tools == 0 {
                            Phase::Working
                        } else {
                            Phase::Tool
                        });
                    }
                    "SubagentStart" => {
                        self.background.insert(e.call.clone());
                        self.phase = Some(Phase::Working);
                    }
                    "SubagentStop" => {
                        self.background.remove(&e.call);
                        if let Some(idle) = e.fully_idle {
                            self.external_background = !idle;
                        }
                        if self.pending_stop && !self.background_active() {
                            self.phase = Some(Phase::Complete);
                            self.pending_stop = false;
                        }
                    }
                    "PostInvocation" => { /* Never mark the whole turn complete here. */ }
                    "Stop" | "complete" => {
                        if let Some(idle) = e.fully_idle {
                            self.external_background = !idle;
                        }
                        if e.fully_idle == Some(false) || !self.background.is_empty() {
                            self.pending_stop = true;
                            self.phase = Some(Phase::Working);
                        } else {
                            self.phase = Some(if e.failed {
                                Phase::Failed
                            } else if e.interrupted {
                                Phase::Interrupted
                            } else {
                                Phase::Complete
                            });
                            self.waiting.clear();
                            self.tools.clear();
                            self.anonymous_tools = 0;
                        }
                    }
                    "failed" | "StopFailure" => {
                        self.phase = Some(Phase::Failed);
                        self.waiting.clear();
                        self.tools.clear();
                        self.anonymous_tools = 0;
                    }
                    "interrupted" | "SessionEnd" => {
                        self.phase = Some(Phase::Interrupted);
                        self.waiting.clear();
                        self.tools.clear();
                        self.anonymous_tools = 0;
                    }
                    _ => {}
                }
            }
        }
        if !self.waiting.is_empty() {
            self.phase = Some(Phase::NeedsInput);
        }
        self.at = self.at.max(e.at);
    }
}

/// AGY step enums from its metadata schema. Completed tools alone cannot finish a turn.
pub fn agy_steps(rows: &[(i64, i64, i64)]) -> (Option<Phase>, String) {
    let start = rows
        .iter()
        .filter(|(_, kind, _)| *kind == 14)
        .map(|(idx, _, _)| *idx)
        .max()
        .unwrap_or(-1);
    let rows: Vec<_> = rows.iter().filter(|(idx, _, _)| *idx >= start).collect();
    let phase = if rows
        .iter()
        .any(|(_, kind, status)| *status == 9 || (*kind == 138 && matches!(status, 1 | 2 | 8 | 11)))
    {
        Some(Phase::NeedsInput)
    } else if rows
        .iter()
        .any(|(_, _, status)| matches!(status, 1 | 2 | 8 | 11))
    {
        Some(
            if rows.iter().any(|(_, kind, status)| {
                !matches!(kind, 14 | 15) && matches!(status, 1 | 2 | 8 | 11)
            }) {
                Phase::Tool
            } else {
                Phase::Thinking
            },
        )
    } else {
        rows.last().map(|(_, kind, status)| match (kind, status) {
            (_, 6 | 12) => Phase::Interrupted,
            (2 | 17, 7) => Phase::Failed,
            (2, 3) => Phase::Complete,
            _ => Phase::Unknown,
        })
    };
    (phase, format!("step-{start}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headless_argv_table_matches_fixture() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../../../scripts/fixtures/headless-argv.json"))
                .unwrap();
        assert!(cases.len() > 60);
        for case in cases {
            let argv: Vec<String> = serde_json::from_value(case["argv"].clone()).unwrap();
            let provider = case["provider"].as_str().unwrap();
            assert_eq!(
                headless_argv(provider, &argv),
                case["expected"].as_str(),
                "{provider} {argv:?}"
            );
        }
    }
    #[test]
    fn classification_is_sticky_for_internal_runs_and_unknown_without_signal() {
        let tagged = |classification: &str| Event {
            kind: "start".into(),
            classification: classification.into(),
            ..Event::default()
        };
        let mut a = Activity::default();
        assert_eq!(classify("kimi", &a), UNKNOWN);
        a.apply(tagged(HEADLESS));
        a.apply(tagged(""));
        assert_eq!(classify("kimi", &a), HEADLESS);
        // An interactive resume of the same session replaces a headless verdict.
        a.apply(tagged(INTERACTIVE));
        assert_eq!(classify("kimi", &a), INTERACTIVE);
        a.apply(tagged(INTERNAL));
        a.apply(tagged(INTERACTIVE));
        assert_eq!(classify("kimi", &a), INTERNAL);
        // Explicit log fields outrank argv.
        let mut claude = Activity {
            classification: INTERACTIVE.into(),
            session_source: "sdk-cli".into(),
            ..Activity::default()
        };
        assert_eq!(classify("claude", &claude), HEADLESS);
        claude.session_source = INTERNAL_SOURCE.into();
        assert_eq!(classify("codex", &claude), INTERNAL);
        let cursor = hook(
            "cursor",
            "sessionStart",
            &serde_json::json!({"conversation_id":"c","is_background_agent":true}),
            1,
        );
        assert_eq!(cursor.classification, HEADLESS);
        let local = hook(
            "cursor",
            "sessionStart",
            &serde_json::json!({"conversation_id":"c","is_background_agent":false}),
            1,
        );
        assert!(local.classification.is_empty());
    }
    fn event(kind: &str, call: &str, at: u64) -> Event {
        Event {
            kind: kind.into(),
            call: call.into(),
            at,
            ..Event::default()
        }
    }
    #[test]
    fn parallel_wait_and_old_turns_are_safe() {
        let mut a = Activity::default();
        a.apply(event("start", "", 100));
        a.apply(event("PermissionRequest", "ask", 101));
        a.apply(event("PostToolUse", "other", 102));
        assert_eq!(a.phase, Some(Phase::NeedsInput));
        a.apply(event("PostToolUse", "ask", 103));
        assert_eq!(a.phase, Some(Phase::Working));
        a.apply(event("start", "", 200));
        a.apply(event("Stop", "", 150));
        assert_eq!(a.phase, Some(Phase::Working));
        a.apply(event("PostToolUseFailure", "tool", 201));
        assert_ne!(a.phase, Some(Phase::Failed));
    }
    #[test]
    fn codebuddy_parent_chain_binds_legacy_prompts_without_finishing_a_new_turn() {
        let mut activity = Activity::default();
        let mut apply = |row: Value| activity.apply(native("codebuddy", &row).unwrap());
        apply(
            serde_json::json!({"id":"user-1","type":"message","role":"user","timestamp":100,"content":"PRIVATE_PROMPT"}),
        );
        apply(
            serde_json::json!({"id":"reason-1","parentId":"user-1","type":"reasoning","timestamp":101,"providerData":{"conversationRequestId":"request-1"}}),
        );
        assert_eq!(activity.phase, Some(Phase::Thinking));
        assert_eq!(activity.turn, "user-1");
        activity.apply(
            native(
                "codebuddy",
                &serde_json::json!({"id":"user-2","type":"message","role":"user","timestamp":200}),
            )
            .unwrap(),
        );
        let stale = native("codebuddy", &serde_json::json!({"id":"answer-1","parentId":"reason-1","type":"message","role":"assistant","status":"completed","timestamp":201,"providerData":{"conversationRequestId":"request-1"}})).unwrap();
        activity.apply(stale);
        assert_eq!(activity.phase, Some(Phase::Working));
        let row = serde_json::json!({"id":"reason-2","parentId":"user-2","type":"reasoning","timestamp":202,"content":"PRIVATE_REASONING","providerData":{"conversationRequestId":"request-2"}});
        let event = native("codebuddy", &row).unwrap();
        assert!(!serde_json::to_string(&event).unwrap().contains("PRIVATE"));
        activity.apply(event);
        activity.apply(native("codebuddy", &serde_json::json!({"id":"answer-2","parentId":"reason-2","type":"message","role":"assistant","status":"completed","timestamp":203,"providerData":{"conversationRequestId":"request-2"}})).unwrap());
        assert_eq!(activity.phase, Some(Phase::Complete));
        assert_eq!(activity.turn, "user-2");
        let mut parent = "answer-2".to_string();
        for n in 0..300 {
            let id = format!("metadata-{n}");
            activity.apply(native("codebuddy", &serde_json::json!({"id":id,"parentId":parent,"type":"credit-usage","timestamp":204+n})).unwrap());
            parent = id;
        }
        assert_eq!(activity.record_turns.len(), 256);
        assert_eq!(activity.record_order.len(), 256);
        assert_eq!(
            activity.record_turns.get(&parent).map(String::as_str),
            Some("user-2")
        );
    }
    #[test]
    fn agy_model_return_and_background_work_do_not_finish_turn() {
        let mut a = Activity::default();
        a.apply(event("PreInvocation", "", 1));
        a.apply(event("PostInvocation", "", 2));
        assert_eq!(a.phase, Some(Phase::Working));
        a.apply(Event {
            fully_idle: Some(false),
            ..event("Stop", "", 3)
        });
        assert_eq!(a.phase, Some(Phase::Working));
        a.apply(Event {
            fully_idle: Some(true),
            ..event("Stop", "", 4)
        });
        assert_eq!(a.phase, Some(Phase::Complete));
        a.apply(event("PreInvocation", "", 5));
        assert_eq!(a.phase, Some(Phase::Working));
        assert_eq!(agy_steps(&[(0, 14, 3), (1, 21, 3)]).0, Some(Phase::Unknown));
        assert_eq!(
            agy_steps(&[(0, 14, 3), (1, 21, 2), (2, 2, 3)]).0,
            Some(Phase::Tool)
        );
    }
    #[test]
    fn hook_metadata_drops_all_content() {
        let e = hook(
            "claude",
            "PreToolUse",
            &serde_json::json!({"session_id":"s","tool_name":"Bash","tool_input":{"command":"SECRET"},"prompt":"SECRET","tool_response":"SECRET"}),
            1,
        );
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"));
    }
}
