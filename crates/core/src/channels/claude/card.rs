//! The approval card of a held `PermissionRequest` and the hook reply for each of its decisions.
//! The reply side rebuilds the card from the stored request, so a decision's native payload
//! (permission updates suggested by the CLI itself) never comes from a client.
use super::{PendingClaude, Profile, profile};
use crate::{
    channels::content,
    session_interaction::contract::{Approval, Decision, FileChange, MAX_REASON},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};

/// Top-level reply flag: the helper fills `updatedInput` with the tool input it was given.
/// Tools that need user interaction (`ExitPlanMode`) ignore an allow without `updatedInput`.
pub(super) const ECHO: &str = "aieyesEchoInput";
const MAX_SUGGESTIONS: usize = 3;

/// Claude Code drops a hook's allow for a tool that asks for user interaction of its own and
/// keeps its terminal dialog open. Plans and questions are answered through `updatedInput`;
/// for the rest the hook input does not say which tools are flagged, and only MCP tools and a
/// few built-in setup dialogs can be. An allow for an MCP tool is therefore reported as
/// submitted, never as confirmed by the helper alone.
pub(super) fn allow_may_be_discarded(tool_name: &str) -> bool {
    tool_name.starts_with("mcp__")
}

fn bounded(value: &Value, limit: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.contains(['\n', '\r']))
}

/// A permission update the CLI suggested, rebuilt from the fields that are understood.
/// Only updates that widen what is allowed in the way the terminal dialog itself offers are
/// kept: allow rules, the accept-edits mode for the session and extra working directories.
pub(super) fn suggestion(raw: &Value) -> Option<Value> {
    let destination = raw["destination"].as_str().filter(|d| {
        matches!(
            *d,
            "session" | "localSettings" | "projectSettings" | "userSettings"
        )
    })?;
    match raw["type"].as_str()? {
        "addRules" if raw["behavior"] == "allow" => {
            let rules = raw["rules"]
                .as_array()
                .filter(|r| (1..=8).contains(&r.len()))?;
            let rules = rules
                .iter()
                .map(|rule| {
                    let mut out = json!({"toolName": bounded(&rule["toolName"], 200)?});
                    // Qoder sends `""` where Claude Code omits the key: a rule for the whole tool.
                    if !rule["ruleContent"].is_null() && rule["ruleContent"] != "" {
                        out["ruleContent"] = json!(bounded(&rule["ruleContent"], 1000)?);
                    }
                    Some(out)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(
                json!({"type": "addRules", "rules": rules, "behavior": "allow", "destination": destination}),
            )
        }
        "setMode" if raw["mode"] == "acceptEdits" && destination == "session" => {
            Some(json!({"type": "setMode", "mode": "acceptEdits", "destination": "session"}))
        }
        "addDirectories" => {
            let dirs = raw["directories"]
                .as_array()
                .filter(|d| (1..=8).contains(&d.len()))?
                .iter()
                .map(|d| bounded(d, 1000))
                .collect::<Option<Vec<_>>>()?;
            Some(json!({"type": "addDirectories", "directories": dirs, "destination": destination}))
        }
        _ => None,
    }
}

/// The suggestions worth storing with a request.
pub(super) fn suggestions(raw: &Value) -> Vec<Value> {
    raw.as_array()
        .into_iter()
        .flatten()
        .filter_map(suggestion)
        .take(MAX_SUGGESTIONS)
        .collect()
}

fn suggested(update: &Value, lang: &str) -> Option<Decision> {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let update = suggestion(update)?;
    let session = update["destination"] == "session";
    let place = tr(match update["destination"].as_str()? {
        "session" => "仅本会话",
        "localSettings" => "本项目（本地设置）",
        "projectSettings" => "本项目（共享设置）",
        _ => "所有项目（用户设置）",
    });
    let (decision, what) = match update["type"].as_str()? {
        "addRules" => (
            if session {
                Decision::approve_session(lang)
            } else {
                Decision::approve_always(lang)
            },
            update["rules"]
                .as_array()?
                .iter()
                .map(|rule| match rule["ruleContent"].as_str() {
                    Some(content) => {
                        format!("{}({content})", rule["toolName"].as_str().unwrap_or(""))
                    }
                    None => rule["toolName"].as_str().unwrap_or("").to_owned(),
                })
                .collect::<Vec<_>>()
                .join(", "),
        ),
        "setMode" => (
            Decision::new(
                "approveAcceptEdits",
                "本会话自动接受编辑",
                "secondary",
                "allow",
                "session",
                lang,
            ),
            String::new(),
        ),
        _ => (
            Decision::new(
                "approveDirectories",
                "允许并添加工作目录",
                "secondary",
                "allow",
                if session { "session" } else { "always" },
                lang,
            ),
            update["directories"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ),
    };
    let description = if what.is_empty() {
        place
    } else {
        format!("{what} · {place}")
    };
    Some(
        decision
            .with_description(&description)
            .with_native(json!({"updatedPermissions": [update]})),
    )
}

fn decisions(req: &PendingClaude, profile: Profile, lang: &str) -> Vec<Decision> {
    let mut out = Vec::new();
    if req.tool_name == "ExitPlanMode" {
        // Claude Code leaves plan mode for the mode the session was in before, which may be one
        // that asks for nothing (auto, bypassPermissions). Its terminal dialog names that mode;
        // a card button cannot, so every approval from the card names the mode it leaves in.
        let mode =
            |mode: &str| json!([{"type": "setMode", "mode": mode, "destination": "session"}]);
        let mut approve = Decision::new("approve", "批准计划", "primary", "allow", "once", lang)
            .with_native(json!({"echoInput": true}));
        if profile.suggestions {
            approve = approve
                .with_description(&crate::localization::translate(
                    lang,
                    "后续操作仍需逐项确认",
                ))
                .with_native(json!({"echoInput": true, "updatedPermissions": mode("default")}));
        }
        out.push(approve);
        if profile.suggestions {
            out.push(
                Decision::new(
                    "approveAcceptEdits",
                    "批准并自动接受编辑",
                    "secondary",
                    "allow",
                    "session",
                    lang,
                )
                .with_native(json!({"echoInput": true, "updatedPermissions": mode("acceptEdits")})),
            );
        }
        let mut revise = Decision::decline(lang).with_reason();
        revise.label = crate::localization::translate(lang, "继续修改计划").into_owned();
        revise.label_key = "继续修改计划".into();
        out.push(revise);
    } else {
        out.push(Decision::approve(lang));
        if profile.suggestions {
            for update in req.meta["suggestions"].as_array().into_iter().flatten() {
                let Some(mut decision) = suggested(update, lang) else {
                    continue;
                };
                let taken = out
                    .iter()
                    .filter(|d| d.id.starts_with(&decision.id))
                    .count();
                if taken > 0 {
                    decision.id = format!("{}.{}", decision.id, taken + 1);
                }
                out.push(decision);
            }
        }
        out.push(Decision::decline(lang).with_reason());
    }
    if profile.interrupt {
        out.push(Decision::decline_interrupt(lang));
    }
    out
}

fn lines(input: &Value) -> String {
    let Some(map) = input.as_object() else {
        return match input {
            Value::Null => String::new(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
    };
    map.iter()
        .map(|(key, value)| match value {
            Value::String(s) => format!("{key}: {s}"),
            other => format!("{key}: {other}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The approval card for a pending hook request. Every decision offered here is answered by
/// `reply`.
pub(crate) fn approval_card(req: &PendingClaude, lang: &str) -> Approval {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let profile = profile(req.provider()).unwrap_or(Profile::CLAUDE);
    let input = &req.input;
    let text = |key: &str| input[key].as_str().unwrap_or("");
    let path = ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|k| input[*k].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("");
    let mcp = req
        .tool_name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"));
    let mut card = Approval::new(
        &req.tool_name,
        match req.tool_name.as_str() {
            "ExitPlanMode" => "plan",
            "Bash" => "command",
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "fileChange",
            _ if mcp.is_some() => "mcpTool",
            _ => "tool",
        },
    );
    card.cwd = req.meta["cwd"].as_str().unwrap_or("").to_owned();
    card.command = input["command"]
        .as_str()
        .or_else(|| (!path.is_empty()).then_some(path))
        .or_else(|| input["url"].as_str())
        .unwrap_or("")
        .to_owned();
    card.reason = input["description"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} · {}", req.tool_name, tr("请求执行权限")));
    match card.kind.as_str() {
        "plan" => {
            card.title = tr("计划审批");
            // Claude Code sends an empty input when the plan file was never written; its own
            // dialog then only asks whether to leave plan mode.
            if text("plan").trim().is_empty() {
                card.reason = tr("请求退出计划模式，未附计划内容");
            } else {
                card.set_plan(text("plan"));
                card.reason = tr("请求批准计划");
            }
            // Permissions the plan asks for in advance.
            card.details = input["allowedPrompts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    Some(format!(
                        "{}: {}",
                        p["tool"].as_str()?,
                        p["prompt"].as_str()?
                    ))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        "fileChange" => {
            let edits = match input["edits"].as_array() {
                Some(edits) => edits.iter().collect::<Vec<_>>(),
                None => vec![input],
            };
            let kind = match (req.tool_name.as_str(), req.meta["exists"].as_bool()) {
                ("Write", Some(true)) => "update",
                ("Write", _) => "add",
                _ => "update",
            };
            card.changes = edits
                .into_iter()
                .map(|edit| FileChange {
                    path: path.to_owned(),
                    kind: kind.into(),
                    old_text: edit["old_string"].as_str().unwrap_or("").to_owned(),
                    new_text: edit["new_string"]
                        .as_str()
                        .or_else(|| edit["content"].as_str())
                        .or_else(|| edit["new_source"].as_str())
                        .unwrap_or("")
                        .to_owned(),
                    // The helper already shortened what it stored.
                    truncated: req.meta["truncated"] == true,
                    ..Default::default()
                })
                .collect();
            if !path.is_empty() {
                card.paths = vec![path.to_owned()];
            }
            card.title = format!(
                "{} · {}",
                req.tool_name,
                path.rsplit(['/', '\\']).next().unwrap_or(path)
            );
            // The original summary field; cards that render `changes` can ignore it.
            card.details = card
                .changes
                .first()
                .map(|c| content::clip(&c.new_text, 1200))
                .unwrap_or_default();
            if input["replace_all"] == true
                || input["edits"]
                    .as_array()
                    .is_some_and(|edits| edits.iter().any(|e| e["replace_all"] == true))
            {
                card.reason = format!("{} · {}", card.reason, tr("替换文件中所有匹配内容"));
            }
        }
        "mcpTool" => {
            if let Some((server, tool)) = mcp {
                card.title = format!("{server} · {tool}");
            }
            card.details = lines(input);
        }
        _ if card.command.is_empty() => card.details = lines(input),
        _ => {}
    }
    card.decisions = decisions(req, profile, lang);
    card.bounded()
}

/// The hook output for a decision of `approval_card`.
pub(super) fn reply(decision: &Decision, reason: Option<&str>) -> Result<Value> {
    let body = match decision.behavior {
        "allow" => {
            let mut body = json!({"behavior": "allow"});
            if decision.native["updatedPermissions"].is_array() {
                body["updatedPermissions"] = decision.native["updatedPermissions"].clone();
            }
            body
        }
        "deny" => {
            let mut body = json!({"behavior": "deny"});
            if let Some(message) = reason.map(str::trim).filter(|s| !s.is_empty()) {
                body["message"] = json!(content::clip(message, MAX_REASON));
            }
            if decision.interrupts {
                body["interrupt"] = json!(true);
            }
            body
        }
        _ => bail!("不支持的审批操作"),
    };
    let mut out =
        json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": body}});
    if decision.native["echoInput"] == true {
        out[ECHO] = json!(true);
    }
    Ok(out)
}

/// Fills in what `reply` left for the helper, which is the only side holding the full input.
pub(super) fn finish(mut response: Value, tool_input: &Value) -> Value {
    if response
        .as_object_mut()
        .and_then(|map| map.remove(ECHO))
        .is_some_and(|flag| flag == true)
    {
        response["hookSpecificOutput"]["decision"]["updatedInput"] = tool_input.clone();
    }
    response
}

/// A copy of a tool input that is safe to keep in the request table: long strings are cut.
/// Returns whether anything was cut.
pub(super) fn display_copy(input: &Value, budget: &mut usize) -> (Value, bool) {
    const FIELD: usize = 16 * 1024;
    match input {
        Value::String(s) => {
            let limit = FIELD.min(*budget);
            *budget = budget.saturating_sub(s.len().min(limit));
            if s.len() <= limit {
                (input.clone(), false)
            } else {
                let mut end = limit;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                (json!(&s[..end]), true)
            }
        }
        Value::Array(items) => {
            let mut cut = items.len() > 64;
            let items = items
                .iter()
                .take(64)
                .map(|item| {
                    let (item, c) = display_copy(item, budget);
                    cut |= c;
                    item
                })
                .collect();
            (Value::Array(items), cut)
        }
        Value::Object(map) => {
            let mut cut = map.len() > 64;
            let map = map
                .iter()
                .take(64)
                .map(|(key, item)| {
                    let (item, c) = display_copy(item, budget);
                    cut |= c;
                    (key.clone(), item)
                })
                .collect();
            (Value::Object(map), cut)
        }
        other => (other.clone(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(tool: &str, input: Value, meta: Value) -> PendingClaude {
        PendingClaude {
            id: "r".into(),
            kind: "approval".into(),
            tool_name: tool.into(),
            call_id: "c".into(),
            question_hash: "h".into(),
            input,
            meta,
            pid: 1,
            process_start: 1,
            at: 0,
        }
    }

    fn ids(card: &Approval) -> Vec<&str> {
        card.decisions.iter().map(|d| d.id.as_str()).collect()
    }

    #[test]
    fn a_plan_is_approved_with_its_input_and_revised_with_feedback() {
        let req = pending(
            "ExitPlanMode",
            json!({"plan": "# Plan\n\n1. Add tests", "allowedPrompts": [{"tool": "Bash", "prompt": "run tests"}]}),
            json!({}),
        );
        let card = approval_card(&req, "en");
        assert_eq!(card.kind, "plan");
        assert_eq!(
            card.plan.as_ref().unwrap().markdown,
            "# Plan\n\n1. Add tests"
        );
        assert_eq!(card.details, "Bash: run tests");
        assert_eq!(
            ids(&card),
            [
                "approve",
                "approveAcceptEdits",
                "decline",
                "declineInterrupt"
            ]
        );
        assert_ne!(card.decisions[0].label, "批准计划");
        assert!(card.decisions[2].accepts_reason && !card.decisions[0].accepts_reason);

        // A bare allow is ignored by Claude Code for this tool; the input has to travel back.
        // The mode is named too: without it the session returns to the mode it had before the
        // plan, which the card cannot show (seen with 2.1.332: an auto-mode session went back
        // to auto and ran the next command unasked).
        let full = json!({"plan": "the full plan"});
        let approve = finish(reply(&card.decisions[0], None).unwrap(), &full);
        assert_eq!(
            approve,
            json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                "decision": {"behavior": "allow", "updatedInput": {"plan": "the full plan"},
                    "updatedPermissions": [{"type": "setMode", "mode": "default", "destination": "session"}]}}})
        );
        assert!(!card.decisions[0].description.is_empty());
        let auto = finish(reply(&card.decisions[1], None).unwrap(), &full);
        assert_eq!(
            auto["hookSpecificOutput"]["decision"]["updatedPermissions"],
            json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}])
        );
        assert_eq!(auto["hookSpecificOutput"]["decision"]["updatedInput"], full);
        let revise = reply(&card.decisions[2], Some(" keep the old API ")).unwrap();
        assert_eq!(
            revise["hookSpecificOutput"]["decision"],
            json!({"behavior": "deny", "message": "keep the old API"})
        );
        let stop = reply(&card.decisions[3], None).unwrap();
        assert_eq!(
            stop["hookSpecificOutput"]["decision"],
            json!({"behavior": "deny", "interrupt": true})
        );
        assert!(stop.get(ECHO).is_none() && revise.get(ECHO).is_none());
    }

    /// No decision of any card may put a session into a mode that skips permission checks.
    #[test]
    fn no_decision_switches_to_a_mode_without_permission_checks() {
        let modes = |card: &Approval| {
            card.decisions
                .iter()
                .flat_map(|d| {
                    let reply = reply(d, None).unwrap();
                    reply["hookSpecificOutput"]["decision"]["updatedPermissions"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                })
                .filter(|update| update["type"] == "setMode")
                .map(|update| update["mode"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let hostile = json!([
            {"type": "setMode", "mode": "bypassPermissions", "destination": "session"},
            {"type": "setMode", "mode": "auto", "destination": "session"},
            {"type": "setMode", "mode": "dontAsk", "destination": "session"},
            {"type": "setMode", "mode": "acceptEdits", "destination": "userSettings"},
            {"type": "setMode", "mode": "plan", "destination": "session"},
            {"type": "setMode", "mode": "acceptEdits", "destination": "session"}
        ]);
        assert_eq!(suggestions(&hostile).len(), 1);
        for provider in ["claude", "qoder", "qoder-cn", "codebuddy", "zcode"] {
            for tool in ["ExitPlanMode", "Bash", "Edit", "mcp__a__b"] {
                // Suggestions as the CLI sent them, not filtered by the helper.
                let mut req = pending(
                    tool,
                    json!({"plan": "p"}),
                    json!({"suggestions": hostile, "provider": provider}),
                );
                req.meta["provider"] = json!(provider);
                let card = approval_card(&req, "en");
                let modes = modes(&card);
                assert!(
                    modes.iter().all(|m| m == "default" || m == "acceptEdits"),
                    "{provider} {tool}: {modes:?}"
                );
                // Every plan approval from Claude Code or Qoder names its mode.
                if tool == "ExitPlanMode" && profile(provider).unwrap().suggestions {
                    assert_eq!(modes, ["default", "acceptEdits"]);
                }
            }
        }
    }

    /// Claude Code 2.1.332 sends `{}` when the model never wrote the plan file.
    #[test]
    fn a_plan_request_without_text_says_so_and_still_echoes_its_input() {
        let card = approval_card(&pending("ExitPlanMode", json!({}), json!({})), "zh-CN");
        assert_eq!(card.kind, "plan");
        assert!(card.plan.is_none());
        assert_eq!(card.reason, "请求退出计划模式，未附计划内容");
        let approve = finish(reply(&card.decisions[0], None).unwrap(), &json!({}));
        assert_eq!(
            approve["hookSpecificOutput"]["decision"]["updatedInput"],
            json!({})
        );
    }

    #[test]
    fn only_understood_suggestions_become_decisions() {
        let raw = json!([
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "git init *"}], "behavior": "allow", "destination": "localSettings"},
            {"type": "setMode", "mode": "bypassPermissions", "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Bash"}], "behavior": "deny", "destination": "session"},
            {"type": "removeRules", "rules": [{"toolName": "Bash"}], "behavior": "deny", "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "a\nb"}], "behavior": "allow", "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Read", "extra": 1}], "behavior": "allow", "destination": "session", "extra": true},
            {"type": "setMode", "mode": "acceptEdits", "destination": "session"},
            {"type": "addDirectories", "directories": ["/srv/data"], "destination": "elsewhere"},
            {"type": "addDirectories", "directories": ["/srv/data"], "destination": "userSettings"}
        ]);
        let kept = suggestions(&raw);
        assert_eq!(kept.len(), 3);
        // Unknown fields are not passed back to the CLI.
        assert_eq!(
            kept[1],
            json!({"type": "addRules", "rules": [{"toolName": "Read"}], "behavior": "allow", "destination": "session"})
        );

        let req = pending(
            "Bash",
            json!({"command": "git init", "description": "Init"}),
            json!({"suggestions": kept, "cwd": "/work"}),
        );
        let card = approval_card(&req, "zh-CN");
        assert_eq!(
            ids(&card),
            [
                "approve",
                "approveAlways",
                "approveSession",
                "approveAcceptEdits",
                "decline",
                "declineInterrupt"
            ]
        );
        assert_eq!(card.cwd, "/work");
        assert_eq!(
            card.decisions[1].description,
            "Bash(git init *) · 本项目（本地设置）"
        );
        assert_eq!(card.decisions[1].scope, "always");
        assert_eq!(card.decisions[2].scope, "session");
        let always = reply(&card.decisions[1], None).unwrap();
        assert_eq!(
            always["hookSpecificOutput"]["decision"],
            json!({"behavior": "allow", "updatedPermissions": [
                {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "git init *"}], "behavior": "allow", "destination": "localSettings"}
            ]})
        );
        assert_eq!(
            reply(&card.decisions[0], None).unwrap()["hookSpecificOutput"]["decision"],
            json!({"behavior": "allow"})
        );

        // Suggestions are ignored for agents whose hook does not take permission updates.
        let mut other = req.clone();
        other.meta["provider"] = json!("codebuddy");
        assert_eq!(ids(&approval_card(&other, "en")), ["approve", "decline"]);
        // ZCode accepts `interrupt` in its schema but does not act on it.
        other.meta["provider"] = json!("zcode");
        assert_eq!(ids(&approval_card(&other, "en")), ["approve", "decline"]);
        // Two rule suggestions for the same scope stay distinguishable.
        let mut twice = req.clone();
        twice.meta["suggestions"] = json!([
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "a *"}], "behavior": "allow", "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "b *"}], "behavior": "allow", "destination": "session"}
        ]);
        assert_eq!(
            ids(&approval_card(&twice, "en"))[1..3],
            ["approveSession", "approveSession.2"]
        );
    }

    /// Qoder sends an empty `ruleContent` for a rule that covers the whole tool.
    #[test]
    fn a_tool_wide_rule_with_empty_content_is_still_an_always_allow_option() {
        let raw = json!([
            {"type": "addRules", "rules": [{"toolName": "mcp__x__y", "ruleContent": ""}], "behavior": "allow", "destination": "localSettings"}
        ]);
        let kept = suggestions(&raw);
        assert_eq!(
            kept,
            [
                json!({"type": "addRules", "rules": [{"toolName": "mcp__x__y"}], "behavior": "allow", "destination": "localSettings"})
            ]
        );
        let mut req = pending("mcp__x__y", json!({}), json!({"suggestions": kept}));
        req.meta["provider"] = json!("qoder");
        let card = approval_card(&req, "en");
        assert_eq!(ids(&card)[..2], ["approve", "approveAlways"]);
        // Content that is present must still be well formed.
        assert!(
            suggestions(&json!([
                {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": 7}], "behavior": "allow", "destination": "session"}
            ]))
            .is_empty()
        );
    }

    #[test]
    fn an_allow_the_agent_may_discard_is_never_confirmed_by_the_helper() {
        assert!(allow_may_be_discarded("mcp__ops__deploy"));
        assert!(!allow_may_be_discarded("Bash"));
        assert!(!allow_may_be_discarded("ExitPlanMode"));
    }

    #[test]
    fn edits_and_mcp_tools_are_presented_by_what_they_touch() {
        let edit = pending(
            "Edit",
            json!({"file_path": "/work/src/a.rs", "old_string": "a", "new_string": "b", "replace_all": true}),
            json!({"truncated": true}),
        );
        let card = approval_card(&edit, "en");
        assert_eq!(card.kind, "fileChange");
        assert_eq!(card.title, "Edit · a.rs");
        assert_eq!(card.paths, ["/work/src/a.rs"]);
        assert_eq!(
            (
                card.changes[0].kind.as_str(),
                card.changes[0].old_text.as_str(),
                card.changes[0].new_text.as_str(),
                card.changes[0].truncated
            ),
            ("update", "a", "b", true)
        );
        assert!(card.reason.contains(" · "));

        let write = |exists: Value| {
            approval_card(
                &pending(
                    "Write",
                    json!({"file_path": "/work/new.txt", "content": "hi"}),
                    json!({"exists": exists}),
                ),
                "en",
            )
            .changes[0]
                .kind
                .clone()
        };
        assert_eq!(write(json!(false)), "add");
        assert_eq!(write(json!(true)), "update");
        assert_eq!(write(Value::Null), "add");

        let multi = pending(
            "MultiEdit",
            json!({"file_path": "/work/a.rs", "edits": [
                {"old_string": "1", "new_string": "2"}, {"old_string": "3", "new_string": "4"}]}),
            json!({}),
        );
        assert_eq!(approval_card(&multi, "en").changes.len(), 2);

        let mcp = pending(
            "mcp__github__create_issue",
            json!({"title": "Bug", "labels": ["a"], "draft": false}),
            json!({}),
        );
        let card = approval_card(&mcp, "en");
        assert_eq!(card.kind, "mcpTool");
        assert_eq!(card.title, "github · create_issue");
        assert_eq!(card.details, "draft: false\nlabels: [\"a\"]\ntitle: Bug");
        assert_eq!(card.command, "");
    }

    #[test]
    fn stored_copies_are_cut_and_flagged() {
        let mut budget = 256 * 1024;
        let small = json!({"command": "ls", "n": 1});
        assert_eq!(display_copy(&small, &mut budget), (small.clone(), false));
        let big = json!({"file_path": "/a", "content": "é".repeat(40_000)});
        let (copy, cut) = display_copy(&big, &mut budget);
        assert!(cut);
        assert_eq!(copy["file_path"], "/a");
        assert!(copy["content"].as_str().unwrap().len() <= 16 * 1024);
        let mut budget = 10;
        let (copy, cut) = display_copy(&json!(["0123456789abcdef", "more"]), &mut budget);
        assert!(cut);
        assert_eq!(copy, json!(["0123456789", ""]));
    }
}
