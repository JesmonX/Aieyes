//! Provider-neutral lifecycle reduction. No prompts, tool arguments or output are retained.
use crate::sessions::Phase;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Event {
    pub kind: String,
    pub session: String,
    pub turn: String,
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
pub fn hook(provider: &str, kind: &str, input: &Value, at: u64) -> Event {
    Event {
        kind: kind.into(),
        at,
        session: field(input, &["session_id", "conversationId"]),
        turn: field(input, &["turn_id", "turnId"]),
        call: field(
            input,
            &["tool_use_id", "toolUseId", "agent_id", "elicitation_id"],
        ),
        tool: if provider == "antigravity" {
            field(&input["toolCall"], &["name"])
        } else {
            field(input, &["tool_name"])
        },
        fully_idle: input["fullyIdle"].as_bool(),
        failed: kind == "StopFailure"
            || (kind == "Stop"
                && (input["terminationReason"] == "error"
                    || input["error"].as_str().is_some_and(|s| !s.is_empty()))),
        interrupted: input["terminationReason"]
            .as_str()
            .is_some_and(|s| s.contains("cancel") || s.contains("interrupt")),
        notification: field(input, &["notification_type"]),
        ..Event::default()
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
/// Read-only Claude recovery when hooks were not present at session start.
pub fn claude(row: &Value) -> Vec<Event> {
    let session = field(row, &["sessionId"]);
    let at = row["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis().max(0) as u64)
        .unwrap_or(0);
    let base = Event {
        session,
        at,
        ..Event::default()
    };
    let content = row["message"]["content"].as_array();
    let mut out = Vec::new();
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
    pub turn: String,
    pub at: u64,
    pub pid: u32,
    pub process_start: u64,
    waiting: HashSet<String>,
    tools: HashSet<String>,
    background: HashSet<String>,
    generation: u64,
    turn_started: u64,
    pending_stop: bool,
    external_background: bool,
}
impl Activity {
    pub fn background_active(&self) -> bool {
        self.external_background || !self.background.is_empty()
    }
    pub fn apply(&mut self, e: Event) {
        if !e.session.is_empty() {
            self.session = e.session.clone();
        }
        if e.kind == "identity" {
            return;
        }
        if e.at > 0 && e.at < self.turn_started {
            return;
        }
        if e.pid > 0 {
            self.pid = e.pid;
            self.process_start = e.process_start;
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
                    "thinking" => self.phase = Some(Phase::Thinking),
                    "tool" | "PreToolUse" => {
                        if !e.call.is_empty() {
                            self.tools.insert(e.call.clone());
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
                    "output" | "PostToolUse" | "PostToolUseFailure" | "ElicitationResult" => {
                        self.waiting.remove(&e.call);
                        self.waiting.remove("notification");
                        self.tools.remove(&e.call);
                        // An individual tool failure is recoverable. Only a terminal event can fail a turn.
                        self.phase = Some(if self.tools.is_empty() {
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
                        }
                    }
                    "failed" | "StopFailure" => {
                        self.phase = Some(Phase::Failed);
                        self.waiting.clear();
                        self.tools.clear();
                    }
                    "interrupted" | "SessionEnd" => {
                        self.phase = Some(Phase::Interrupted);
                        self.waiting.clear();
                        self.tools.clear();
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
