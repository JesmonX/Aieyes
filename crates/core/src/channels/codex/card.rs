//! Cards for the requests a Codex app-server sends its clients (shapes from codex-cli 0.172.0,
//! `codex app-server generate-json-schema --experimental`). Every decision keeps the exact
//! native reply it stands for: listed decision objects are echoed back unchanged, never built
//! from what a client sent. A request kind that cannot be shown faithfully gets a card with no
//! decisions, which leaves it to the original window.
use crate::channels::{claude::elicitation, content};
use crate::session_interaction::contract::{Approval, Decision, FileChange};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

pub(crate) const COMMAND: &str = "item/commandExecution/requestApproval";
pub(crate) const FILE_CHANGE: &str = "item/fileChange/requestApproval";
pub(crate) const PERMISSIONS: &str = "item/permissions/requestApproval";
pub(crate) const ELICITATION: &str = "mcpServer/elicitation/request";
/// Requests of the deprecated conversation API; they name the thread `conversationId`.
pub(crate) const LEGACY_EXEC: &str = "execCommandApproval";
pub(crate) const LEGACY_PATCH: &str = "applyPatchApproval";
const MAX_PATCH: usize = 256 * 1024;

fn method(request: &Value) -> &str {
    request["method"].as_str().unwrap_or("")
}

/// The thread a request belongs to, whichever API generation sent it.
pub(crate) fn thread_id(request: &Value) -> Option<&str> {
    let p = &request["params"];
    p["threadId"]
        .as_str()
        .or_else(|| p["conversationId"].as_str())
}

/// Requests that carry no turn id of their own; the caller verifies the thread's latest turn.
pub(crate) fn turn_optional(request: &Value) -> bool {
    matches!(method(request), ELICITATION | LEGACY_EXEC | LEGACY_PATCH)
}

fn schema_fields(request: &Value) -> Option<&Map<String, Value>> {
    request["params"]["requestedSchema"]["properties"].as_object()
}

/// A form-mode elicitation that asks for nothing but consent: an MCP tool call approval, or a
/// server's plain confirmation.
fn consent(request: &Value) -> bool {
    method(request) == ELICITATION
        && request["params"]["mode"] == "form"
        && schema_fields(request).is_some_and(Map::is_empty)
}

fn tool_call(request: &Value) -> bool {
    request["params"]["_meta"]["codex_approval_kind"] == "mcp_tool_call"
}

/// Whether the request is shown as an approval card (with or without decisions).
pub(crate) fn is_approval(request: &Value) -> bool {
    matches!(
        method(request),
        COMMAND | FILE_CHANGE | PERMISSIONS | LEGACY_EXEC | LEGACY_PATCH
    ) || consent(request)
}

/// A blocking `request_user_input` question.
pub(crate) fn is_user_input(request: &Value) -> bool {
    matches!(
        method(request),
        "item/tool/requestUserInput" | "tool/requestUserInput"
    )
}

/// Whether the request is an MCP elicitation that asks the user to fill something in.
pub(crate) fn is_form(request: &Value) -> bool {
    method(request) == ELICITATION && !consent(request)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}

fn words(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        value.as_array().map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
    })
}

/// A copy without null members, so a grant only names what was asked for.
fn without_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), without_nulls(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Readable lines and plain paths of a requested permission profile.
fn permission_lines(profile: &Value, lang: &str) -> (Vec<String>, Vec<String>) {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let (mut lines, mut paths) = (Vec::new(), Vec::new());
    if profile["network"]["enabled"] == true {
        lines.push(tr("网络访问"));
    }
    let fs = &profile["fileSystem"];
    let mut add = |access: &str, path: String, plain: bool| {
        if !path.is_empty() {
            lines.push(format!("{access}: {path}"));
            if plain && !paths.contains(&path) {
                paths.push(path);
            }
        }
    };
    if let Some(entries) = fs["entries"].as_array().filter(|e| !e.is_empty()) {
        for entry in entries {
            let access = entry["access"].as_str().unwrap_or("?");
            let target = &entry["path"];
            match target["type"].as_str() {
                Some("path") => add(access, target["path"].as_str().unwrap_or("").into(), true),
                Some("glob_pattern") => add(
                    access,
                    target["pattern"].as_str().unwrap_or("").into(),
                    false,
                ),
                Some("special") => {
                    let special = &target["value"];
                    let mut name = format!("<{}>", special["kind"].as_str().unwrap_or("?"));
                    for key in ["path", "subpath"] {
                        if let Some(part) = text(&special[key]) {
                            name.push_str(&format!(" {part}"));
                        }
                    }
                    add(access, name, false);
                }
                _ => add(access, target.to_string(), false),
            }
        }
    } else {
        for access in ["read", "write"] {
            for path in fs[access].as_array().into_iter().flatten() {
                add(access, path.as_str().unwrap_or("").into(), true);
            }
        }
    }
    (lines, paths)
}

fn push(card: &mut Approval, decision: Decision) {
    let mut decision = decision;
    let same = |d: &Decision, id: &str| d.id == id || d.id.starts_with(&format!("{id}."));
    let seen = card
        .decisions
        .iter()
        .filter(|d| same(d, &decision.id))
        .count();
    if seen > 0 {
        decision.id = format!("{}.{}", decision.id, seen + 1);
    }
    card.decisions.push(decision);
}

/// Codex `cancel`: refuse and stop the turn at once.
fn stop(lang: &str) -> Decision {
    let label = Decision::decline_interrupt(lang);
    let mut d = Decision::cancel(lang);
    d.label = label.label;
    d.label_key = label.label_key;
    d.style = "danger";
    d
}

fn printable(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1000 && !value.chars().any(char::is_control)
}

/// One entry of `availableDecisions` as a decision that echoes it back.
fn listed(entry: &Value, lang: &str) -> Option<Decision> {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let native = json!({"decision": entry});
    if let Some(name) = entry.as_str() {
        return Some(
            match name {
                "accept" => Decision::approve(lang),
                "acceptForSession" => Decision::approve_session(lang),
                "decline" => Decision::decline(lang),
                "cancel" => stop(lang),
                _ => return None,
            }
            .with_native(native),
        );
    }
    let object = entry.as_object().filter(|o| o.len() == 1)?;
    if let Some(amendment) = object.get("acceptWithExecpolicyAmendment") {
        let prefix = amendment["execpolicy_amendment"].as_array()?;
        let parts = prefix
            .iter()
            .map(|part| part.as_str().filter(|s| printable(s)))
            .collect::<Option<Vec<_>>>()
            .filter(|parts| !parts.is_empty())?;
        return Some(
            Decision::approve_always(lang)
                .with_description(&format!(
                    "{}: {}",
                    tr("今后不再询问以此开头的命令"),
                    parts.join(" ")
                ))
                .with_native(native),
        );
    }
    if let Some(amendment) = object.get("applyNetworkPolicyAmendment") {
        let rule = &amendment["network_policy_amendment"];
        let host = rule["host"].as_str().filter(|s| printable(s))?;
        let decision = match rule["action"].as_str()? {
            "allow" => Decision::new(
                "approveHost",
                "始终允许访问此主机",
                "secondary",
                "allow",
                "always",
                lang,
            ),
            "deny" => Decision::new(
                "declineHost",
                "始终拒绝访问此主机",
                "danger",
                "deny",
                "always",
                lang,
            ),
            _ => return None,
        };
        return Some(decision.with_description(host).with_native(native));
    }
    None
}

fn command_card(request: &Value, lang: &str) -> Approval {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let mut card = Approval::new("Command", "command");
    card.command = words(&p["command"]).unwrap_or_default();
    card.reason = text(&p["reason"])
        .map(str::to_owned)
        .unwrap_or_else(|| tr("请求批准命令执行"));
    let mut details = Vec::new();
    let network = &p["networkApprovalContext"];
    if let Some(host) = text(&network["host"]) {
        details.push(format!(
            "{}: {}://{host}",
            tr("网络访问"),
            network["protocol"].as_str().unwrap_or("?")
        ));
    }
    let (lines, paths) = permission_lines(&p["additionalPermissions"], lang);
    details.extend(lines);
    card.paths = paths;
    if let Some(cwd) = text(&p["cwd"]) {
        card.cwd = cwd.to_owned();
        if details.is_empty() {
            details.push(cwd.to_owned());
        }
    }
    card.details = details.join("\n");
    // Input for a running terminal and managed checkpoints carry rules this card cannot show.
    if !matches!(p["kind"].as_str(), None | Some("command")) {
        return card;
    }
    match p["availableDecisions"].as_array() {
        Some(all) => {
            for entry in all {
                if let Some(decision) = listed(entry, lang) {
                    push(&mut card, decision);
                }
            }
            // The server takes a plain refusal even when the prompt does not list one
            // (0.172.0); without it the only way to say no would also stop the turn.
            if !card.decisions.is_empty() && !card.decisions.iter().any(|d| d.id == "decline") {
                let at = card
                    .decisions
                    .iter()
                    .position(|d| d.behavior != "allow")
                    .unwrap_or(card.decisions.len());
                card.decisions.insert(
                    at,
                    Decision::decline(lang).with_native(json!({"decision": "decline"})),
                );
            }
        }
        // Older servers list nothing and take the two basic decisions.
        None => {
            for name in ["accept", "decline"] {
                card.decisions.extend(listed(&json!(name), lang));
            }
        }
    }
    card
}

fn file_card(request: &Value, changes: Vec<FileChange>, lang: &str) -> Approval {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let mut card = Approval::new("FileChange", "fileChange");
    card.command = if changes.is_empty() {
        tr("文件修改")
    } else {
        changes
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    card.changes = changes;
    card.reason = text(&p["reason"])
        .map(str::to_owned)
        .unwrap_or_else(|| tr("请求批准文件修改"));
    if let Some(cwd) = text(&p["cwd"]) {
        card.cwd = cwd.to_owned();
    }
    if let Some(root) = text(&p["grantRoot"]) {
        card.details = format!("{}: {root}", tr("请求本会话内可写入此目录"));
        card.paths.push(root.to_owned());
    }
    card
}

/// `{path: {type: add|delete, content} | {type: update, unified_diff, move_path}}`.
fn legacy_changes(map: &Value) -> Vec<FileChange> {
    map.as_object()
        .into_iter()
        .flatten()
        .map(|(path, change)| {
            let mut out = FileChange {
                path: path.clone(),
                kind: "unknown".into(),
                ..Default::default()
            };
            let body = |key: &str| change[key].as_str().unwrap_or("").to_owned();
            match change["type"].as_str() {
                Some("add") => {
                    out.kind = "add".into();
                    out.new_text = body("content");
                }
                Some("delete") => {
                    out.kind = "delete".into();
                    out.old_text = body("content");
                }
                Some("update") => {
                    out.move_path = body("move_path");
                    out.kind = if out.move_path.is_empty() {
                        "update"
                    } else {
                        "move"
                    }
                    .into();
                    out.diff = body("unified_diff");
                }
                _ => {}
            }
            out
        })
        .collect()
}

/// The `changes` of a `fileChange` thread item: `[{path, kind: {type, move_path?}, diff}]`.
/// `diff` is a unified diff only for an update; for an added or deleted file it is the file's
/// content, which is shown as new or old text like the other sources of a change do.
pub(crate) fn item_changes(item: &Value) -> Vec<FileChange> {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = text(&change["path"])?;
            let move_path = change["kind"]["move_path"].as_str().unwrap_or("");
            let kind = match change["kind"]["type"].as_str() {
                Some("update") if !move_path.is_empty() => "move",
                Some(kind @ ("add" | "delete" | "update")) => kind,
                _ => "unknown",
            };
            let body = change["diff"].as_str().unwrap_or("").to_owned();
            let mut out = FileChange {
                path: path.to_owned(),
                kind: kind.into(),
                move_path: move_path.to_owned(),
                ..Default::default()
            };
            match kind {
                "add" => out.new_text = body,
                "delete" => out.old_text = body,
                _ => out.diff = body,
            }
            Some(out)
        })
        .collect()
}

/// The files of an `apply_patch` envelope (`*** Begin Patch` … `*** End Patch`), each with its
/// own hunk lines as the diff. Text around the envelope is ignored.
pub(crate) fn patch_changes(patch: &str) -> Vec<FileChange> {
    let Some(start) = patch.find("*** Begin Patch") else {
        return Vec::new();
    };
    let mut changes: Vec<FileChange> = Vec::new();
    for line in patch[start..].lines().skip(1) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("*** End Patch") {
            break;
        }
        let header = [
            ("*** Add File: ", "add"),
            ("*** Update File: ", "update"),
            ("*** Delete File: ", "delete"),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| Some((line.strip_prefix(prefix)?.trim(), kind)));
        if let Some((path, kind)) = header {
            changes.push(FileChange {
                path: path.to_owned(),
                kind: kind.into(),
                ..Default::default()
            });
        } else if let Some(change) = changes.last_mut() {
            if let Some(target) = line.strip_prefix("*** Move to: ") {
                change.move_path = target.trim().to_owned();
                change.kind = "move".into();
            } else if change.diff.len() < MAX_PATCH {
                change.diff.push_str(line);
                change.diff.push('\n');
            } else {
                change.truncated = true;
            }
        }
    }
    changes.retain(|c| !c.path.is_empty());
    changes
}

/// The patch text of the tool call `item_id` in a rollout: `function_call` records keep it in
/// their JSON `arguments`, `custom_tool_call` records in `input`. Only the call with exactly
/// this id is read; nothing is inferred from neighbouring records.
pub(crate) fn call_patch(records: &[Value], item_id: &str) -> Option<String> {
    fn find(value: &Value) -> Option<&str> {
        match value {
            Value::String(s) => s.contains("*** Begin Patch").then_some(s.as_str()),
            Value::Array(items) => items.iter().find_map(find),
            Value::Object(map) => map.values().find_map(find),
            _ => None,
        }
    }
    let payload = records
        .iter()
        .rev()
        .map(|record| &record["payload"])
        .find(|p| {
            p["call_id"].as_str() == Some(item_id)
                && matches!(
                    p["type"].as_str(),
                    Some("function_call" | "custom_tool_call")
                )
        })?;
    let arguments = payload["arguments"]
        .as_str()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let patch = arguments
        .as_ref()
        .and_then(find)
        .or_else(|| find(&payload["input"]))
        .or_else(|| find(&payload["arguments"]))?;
    Some(content::clip(patch, MAX_PATCH))
}

fn permissions_card(request: &Value, lang: &str) -> Approval {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let mut card = Approval::new("Permissions", "permissions");
    let (lines, paths) = permission_lines(&p["permissions"], lang);
    card.command = lines.join("\n");
    card.paths = paths;
    card.reason = text(&p["reason"])
        .map(str::to_owned)
        .unwrap_or_else(|| tr("请求额外权限"));
    if let Some(cwd) = text(&p["cwd"]) {
        card.cwd = cwd.to_owned();
    }
    let granted = without_nulls(&p["permissions"]);
    // A profile this card cannot spell out is not granted from here.
    if !granted.is_object() || card.command.is_empty() {
        return card;
    }
    let mut turn = Decision::new("approve", "本轮内允许", "primary", "allow", "turn", lang);
    turn.native = json!({"permissions": granted, "scope": "turn"});
    card.decisions.push(turn);
    card.decisions.push(
        Decision::approve_session(lang)
            .with_native(json!({"permissions": granted, "scope": "session"})),
    );
    card.decisions
        .push(Decision::decline(lang).with_native(json!({"permissions": {}, "scope": "turn"})));
    card
}

fn value_line(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn consent_card(request: &Value, lang: &str) -> Approval {
    let p = &request["params"];
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let meta = &p["_meta"];
    let server = p["serverName"].as_str().unwrap_or("");
    let mut card = if tool_call(request) {
        let tool = meta["tool_name"].as_str().unwrap_or("");
        let mut card = Approval::new(if tool.is_empty() { "MCP" } else { tool }, "mcpTool");
        card.title = [server, tool]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let shown = meta["tool_params_display"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        let name = text(&row["display_name"]).or_else(|| text(&row["name"]))?;
                        Some(format!("{name}: {}", value_line(&row["value"])))
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|rows| !rows.is_empty());
        card.command = match (shown, meta["tool_params"].as_object()) {
            (Some(rows), _) => rows.join("\n"),
            (None, Some(params)) => params
                .iter()
                .map(|(key, value)| format!("{key}: {}", value_line(value)))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        card.details = meta["tool_description"].as_str().unwrap_or("").to_owned();
        card
    } else {
        let mut card = Approval::new(if server.is_empty() { "MCP" } else { server }, "generic");
        card.title = server.to_owned();
        card
    };
    card.reason = text(&p["message"])
        .map(str::to_owned)
        .unwrap_or_else(|| tr("原会话请求确认"));
    let accept = |persist: Option<&str>| {
        let mut reply = json!({"action": "accept", "content": {}});
        if let Some(persist) = persist {
            reply["_meta"] = json!({"persist": persist});
        }
        reply
    };
    let offers = |scope: &str| match &meta["persist"] {
        Value::String(s) => s == scope,
        Value::Array(all) => all.iter().any(|v| v == scope),
        _ => false,
    };
    card.decisions
        .push(Decision::approve(lang).with_native(accept(None)));
    if tool_call(request) && offers("session") {
        card.decisions
            .push(Decision::approve_session(lang).with_native(accept(Some("session"))));
    }
    if tool_call(request) && offers("always") {
        card.decisions
            .push(Decision::approve_always(lang).with_native(accept(Some("always"))));
    }
    card.decisions
        .push(Decision::decline(lang).with_native(json!({"action": "decline"})));
    card.decisions
        .push(stop(lang).with_native(json!({"action": "cancel"})));
    card
}

fn legacy_decisions(card: &mut Approval, lang: &str) {
    for (decision, native) in [
        (Decision::approve(lang), json!("approved")),
        (
            Decision::approve_session(lang),
            json!("approved_for_session"),
        ),
        (
            Decision::decline(lang).with_reason(),
            json!({"denied": {"rejection": ""}}),
        ),
        (stop(lang), json!("abort")),
    ] {
        card.decisions
            .push(decision.with_native(json!({"decision": native})));
    }
}

/// The approval card of a request for which `is_approval` holds. `changes` are the files of
/// a file-change request, which the request itself does not carry.
pub(crate) fn approval_card_with(
    request: &Value,
    changes: Vec<FileChange>,
    lang: &str,
) -> Approval {
    let p = &request["params"];
    match method(request) {
        FILE_CHANGE => {
            let mut card = file_card(request, changes, lang);
            // The request lists no decisions; these four are the whole reply type.
            for name in ["accept", "acceptForSession", "decline", "cancel"] {
                card.decisions.extend(listed(&json!(name), lang));
            }
            card
        }
        LEGACY_PATCH => {
            let mut card = file_card(request, legacy_changes(&p["fileChanges"]), lang);
            legacy_decisions(&mut card, lang);
            card
        }
        LEGACY_EXEC => {
            let mut card = command_card(request, lang);
            card.decisions.clear();
            legacy_decisions(&mut card, lang);
            card
        }
        PERMISSIONS => permissions_card(request, lang),
        ELICITATION => consent_card(request, lang),
        _ => command_card(request, lang),
    }
}

pub(crate) fn approval_card(request: &Value, lang: &str) -> Approval {
    approval_card_with(request, Vec::new(), lang)
}

/// The native reply for a decision of `approval_card`, with the user's reason where the
/// reply type has a place for one.
pub(crate) fn decision_reply(decision: &Decision, reason: Option<&str>) -> Result<Value> {
    let mut reply = decision.native.clone();
    if !reply.is_object() {
        bail!("不支持的审批操作");
    }
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        let slot = reply
            .get_mut("decision")
            .and_then(|d| d.get_mut("denied"))
            .and_then(|d| d.get_mut("rejection"))
            .filter(|_| decision.accepts_reason)
            .context("此审批选项不支持附加说明")?;
        *slot = json!(reason);
    }
    Ok(reply)
}

/// The reply for one of the three plain decisions of the text channel (`/approve`,
/// `/decline`, `/cancel-request`); scoped and persistent decisions are card-only.
pub(crate) fn plain_reply(request: &Value, id: &str) -> Result<Value> {
    let card = approval_card(request, "zh-CN");
    let decision = card
        .decisions
        .iter()
        .find(|d| d.id == id)
        .context("原会话不允许此审批选项")?;
    decision_reply(decision, None)
}

fn stored_form(request: &Value) -> Option<Value> {
    let p = &request["params"];
    if !is_form(request) || p["mode"] != "form" {
        return None;
    }
    // The server sends the schema with its keys sorted; required fields are shown first.
    let order = p["requestedSchema"]["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    elicitation::stored(
        &json!({
            "mode": "form",
            "message": p["message"],
            "mcp_server_name": p["serverName"],
            "requested_schema": without_nulls(&p["requestedSchema"]),
        }),
        order,
    )
}

/// The question card of an MCP form: flat schemas of primitive fields only. URL mode, device
/// verification, vendor forms and nested schemas return `None` and stay in the original window.
pub(crate) fn form_card(request: &Value, lang: &str) -> Option<Value> {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let stored = stored_form(request)?;
    let questions = elicitation::questions(&stored, lang)?;
    let p = &request["params"];
    let server = p["serverName"].as_str().unwrap_or("");
    Some(json!({"method": ELICITATION, "params": {
        "threadId": p["threadId"],
        "turnId": p["turnId"],
        "form": {
            "title": if server.is_empty() { tr("MCP 表单") } else { server.to_owned() },
            "description": stored["message"],
            "source": "mcp",
            "serverName": server,
            "canCancel": true,
            "canSkip": true,
        },
        "questions": questions,
    }}))
}

/// The reply to an MCP form: typed, schema-checked content, a decline (`skip`) or a cancel.
pub(crate) fn form_reply(
    request: &Value,
    action: &str,
    answers: Option<&Value>,
    text: Option<&str>,
) -> Result<Value> {
    let stored = stored_form(request).context("此类输入需要回到原窗口处理")?;
    Ok(match action {
        "cancel" => json!({"action": "cancel"}),
        "skip" => json!({"action": "decline"}),
        "answer" => {
            json!({"action": "accept", "content": elicitation::content(&stored, answers, text)?})
        }
        _ => bail!("当前请求不是审批，请刷新后再操作"),
    })
}

/// One line per part of a card for the text channel.
pub(crate) fn summary(request: &Value, lang: &str) -> Option<(String, String)> {
    let card = approval_card(request, lang);
    if card.decisions.is_empty() {
        return None;
    }
    let body = [card.title.as_str(), card.command.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Some((card.reason, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "01a21127-e634-7d10-a2ed-fff55187165f";
    const TURN: &str = "01a21127-e751-79b2-bb59-7dd6e912895c";

    fn ids(card: &Approval) -> Vec<&str> {
        card.decisions.iter().map(|d| d.id.as_str()).collect()
    }

    fn native<'a>(card: &'a Approval, id: &str) -> &'a Value {
        &card.decisions.iter().find(|d| d.id == id).unwrap().native
    }

    /// Captured from codex-cli 0.172.0 (policy `untrusted`, sandbox `read-only`).
    fn command() -> Value {
        json!({"method":"item/commandExecution/requestApproval","id":0,"params":{"kind":"command","threadId":THREAD,"turnId":TURN,"itemId":"call_a80e706e333b3846","startedAtMs":1795584160966u64,"environmentId":"local","command":"/bin/zsh -lc 'touch hello.txt && echo done'","cwd":"/tmp/codexprobe/work","commandActions":[{"type":"unknown","command":"touch hello.txt && echo done"}],"proposedExecpolicyAmendment":["touch","hello.txt"],"availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["touch","hello.txt"]}},"cancel"]}})
    }

    fn file_change() -> Value {
        json!({"method":"item/fileChange/requestApproval","id":0,"params":{"threadId":THREAD,"turnId":TURN,"itemId":"call_a58b273c67d1f3e3","startedAtMs":1795617995237u64,"reason":null,"grantRoot":null}})
    }

    fn permissions() -> Value {
        json!({"method":"item/permissions/requestApproval","id":0,"params":{"threadId":THREAD,"turnId":TURN,"itemId":"call_9aa64c6d457a3535","environmentId":"local","startedAtMs":1795586351255u64,"cwd":"/tmp/codexprobe/work","reason":"need to fetch data","permissions":{"network":{"enabled":true},"fileSystem":{"read":null,"write":["/tmp/probe-out"],"entries":[{"path":{"type":"path","path":"/tmp/probe-out"},"access":"write"}]}}}})
    }

    fn mcp_tool() -> Value {
        json!({"method":"mcpServer/elicitation/request","id":0,"params":{"threadId":THREAD,"turnId":TURN,"serverName":"probe","environmentId":null,"mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"],"tool_name":"delete_record","tool_annotations":{"readOnlyHint":false,"destructiveHint":true},"tool_description":"Delete a record by id (destructive).","tool_params":{"id":"42"},"tool_params_display":[{"name":"id","value":"42","display_name":"id"}]},"message":"Allow the probe MCP server to run tool \"delete_record\"?","requestedSchema":{"type":"object","properties":{}}}})
    }

    fn mcp_form() -> Value {
        json!({"method":"mcpServer/elicitation/request","id":0,"params":{"threadId":THREAD,"turnId":TURN,"serverName":"probe","environmentId":null,"mode":"form","_meta":null,"message":"Choose deployment settings","requestedSchema":{"type":"object","properties":{"comment":{"type":"string","title":"Comment","maxLength":40},"features":{"type":"array","title":"Features","minItems":1,"items":{"type":"string","enum":["cache","cdn","waf"]}},"notify":{"type":"boolean","title":"Notify team","default":true},"region":{"type":"string","title":"Region","description":"Where to deploy","enum":["us-east","eu-west"],"enumNames":["US East","EU West"]},"replicas":{"type":"integer","title":"Replicas","minimum":1,"maximum":5,"default":2}},"required":["region","replicas"]}}})
    }

    #[test]
    fn fingerprint_tells_similar_requests_apart() {
        let hash = super::super::fingerprint;
        let mut other = mcp_tool();
        other["params"]["_meta"]["tool_params"]["id"] = json!("43");
        assert_ne!(hash(&mcp_tool()), hash(&other));
        let mut form = mcp_form();
        form["params"]["requestedSchema"]["required"] = json!(["region"]);
        assert_ne!(hash(&mcp_form()), hash(&form));
        let legacy = json!({"method":"execCommandApproval","id":1,"params":{"conversationId":THREAD,"callId":"a","command":["ls"],"cwd":"/tmp"}});
        let mut next = legacy.clone();
        next["params"]["callId"] = json!("b");
        assert_ne!(hash(&legacy), hash(&next));
        // Delivery details are not part of the prompt.
        let mut same = command();
        same["id"] = json!(9);
        same["params"]["pendingCount"] = json!(3);
        same["params"]["startedAtMs"] = json!(1);
        assert_eq!(hash(&command()), hash(&same));
    }

    #[test]
    fn command_decisions_echo_listed_objects_and_always_offer_a_plain_refusal() {
        let card = approval_card(&command(), "en");
        assert_eq!(card.kind, "command");
        assert_eq!(card.command, "/bin/zsh -lc 'touch hello.txt && echo done'");
        assert_eq!(card.cwd, "/tmp/codexprobe/work");
        assert_eq!(
            ids(&card),
            ["approve", "approveAlways", "decline", "cancel"]
        );
        assert_eq!(native(&card, "approve"), &json!({"decision": "accept"}));
        // The amendment is the request's own object, not one rebuilt from its parts.
        assert_eq!(
            native(&card, "approveAlways"),
            &json!({"decision": command()["params"]["availableDecisions"][1]})
        );
        let always = &card.decisions[1];
        assert_eq!((always.behavior, always.scope), ("allow", "always"));
        assert!(always.description.ends_with(": touch hello.txt"));
        assert_eq!(native(&card, "decline"), &json!({"decision": "decline"}));
        let stop = &card.decisions[3];
        assert_eq!(stop.native, json!({"decision": "cancel"}));
        assert!(stop.interrupts && stop.label == "Deny and stop");

        let mut request = command();
        request["params"]["availableDecisions"] = json!([
            "accept",
            "acceptForSession",
            {"applyNetworkPolicyAmendment": {"network_policy_amendment": {"host": "example.com", "action": "allow"}}},
            {"applyNetworkPolicyAmendment": {"network_policy_amendment": {"host": "example.com", "action": "deny"}}},
            {"applyNetworkPolicyAmendment": {"network_policy_amendment": {"host": "other.example", "action": "allow"}}},
            {"applyNetworkPolicyAmendment": {"network_policy_amendment": {"host": "bad\u{7}host", "action": "allow"}}},
            {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": []}},
            {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": ["ls"]}, "extra": true},
            {"somethingNew": {}},
            "futureDecision",
            "decline",
            "cancel"
        ]);
        request["params"]["networkApprovalContext"] =
            json!({"host": "example.com", "protocol": "https"});
        let card = approval_card(&request, "en");
        assert_eq!(
            ids(&card),
            [
                "approve",
                "approveSession",
                "approveHost",
                "declineHost",
                "approveHost.2",
                "decline",
                "cancel"
            ]
        );
        assert_eq!(card.decisions[2].description, "example.com");
        assert_eq!(
            (card.decisions[3].behavior, card.decisions[3].scope),
            ("deny", "always")
        );
        assert_eq!(
            native(&card, "approveHost.2"),
            &json!({"decision": request["params"]["availableDecisions"][4]})
        );
        assert!(card.details.contains("https://example.com"));

        // Older servers list nothing; input for a terminal and managed checkpoints stay there.
        request["params"]
            .as_object_mut()
            .unwrap()
            .remove("availableDecisions");
        assert_eq!(ids(&approval_card(&request, "en")), ["approve", "decline"]);
        for kind in ["writeStdin", "humanCommand"] {
            request["params"]["kind"] = json!(kind);
            assert!(is_approval(&request));
            assert!(approval_card(&request, "en").decisions.is_empty());
            assert!(plain_reply(&request, "approve").is_err());
        }
    }

    #[test]
    fn file_change_card_shows_the_patch_of_exactly_that_call() {
        let request = file_change();
        let records = vec![
            json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\": \"apply_patch <<'EOF'\\n*** Begin Patch\\n*** Add File: other.md\\n+no\\n*** End Patch\\nEOF\"}","call_id":"call_other"}}),
            json!({"type":"response_item","payload":{"type":"function_call","status":"completed","name":"exec_command","arguments":"{\"cmd\": \"apply_patch <<'EOF'\\n*** Begin Patch\\n*** Add File: notes.md\\n+hello\\n*** Update File: src/a.rs\\n*** Move to: src/b.rs\\n@@ fn main\\n-old\\n+new\\n*** Delete File: gone.txt\\n*** End Patch\\nEOF\"}","call_id":"call_a58b273c67d1f3e3"}}),
        ];
        let patch = call_patch(&records, "call_a58b273c67d1f3e3").unwrap();
        let card = approval_card_with(&request, patch_changes(&patch), "en");
        assert_eq!(card.kind, "fileChange");
        assert_eq!(card.command, "notes.md\nsrc/a.rs\ngone.txt");
        let shown: Vec<_> = card
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.kind.as_str(), c.diff.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("notes.md", "add", "+hello\n"),
                ("src/a.rs", "move", "@@ fn main\n-old\n+new\n"),
                ("gone.txt", "delete", ""),
            ]
        );
        assert_eq!(card.changes[1].move_path, "src/b.rs");
        assert_eq!(
            ids(&card),
            ["approve", "approveSession", "decline", "cancel"]
        );
        assert_eq!(
            native(&card, "approveSession"),
            &json!({"decision": "acceptForSession"})
        );

        // A raw patch tool call keeps the text in `input`.
        let custom = vec![
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: x.py\n@@\n-a\n+b\n*** End Patch","call_id":"call_a58b273c67d1f3e3"}}),
        ];
        let changes = patch_changes(&call_patch(&custom, "call_a58b273c67d1f3e3").unwrap());
        assert_eq!(
            (changes[0].path.as_str(), changes[0].kind.as_str()),
            ("x.py", "update")
        );
        // No record for the call: the card says so rather than showing another call's files.
        assert!(call_patch(&records, "call_missing").is_none());
        let bare = approval_card(&request, "en");
        assert!(bare.changes.is_empty() && bare.command == "File changes");
        assert!(patch_changes("no patch here").is_empty());

        // The live `item/started` shape, when a connection does see it.
        let item = json!({"type":"fileChange","id":"call_a58b273c67d1f3e3","changes":[{"path":"/tmp/work/notes.md","kind":{"type":"add"},"diff":"hello\n"},{"path":"/tmp/work/a","kind":{"type":"update","move_path":"/tmp/work/b"},"diff":"@@\n"},{"path":"/tmp/work/old","kind":{"type":"delete"},"diff":"-not a diff line\n"}]});
        let changes = item_changes(&item);
        assert_eq!(changes[0].kind, "add");
        // An added or deleted file arrives as its content, not as a diff.
        assert_eq!(
            (changes[0].new_text.as_str(), changes[0].diff.as_str()),
            ("hello\n", "")
        );
        assert_eq!(
            (
                changes[2].kind.as_str(),
                changes[2].old_text.as_str(),
                changes[2].diff.as_str()
            ),
            ("delete", "-not a diff line\n", "")
        );
        assert_eq!(changes[1].diff, "@@\n");
        assert_eq!(
            (changes[1].kind.as_str(), changes[1].move_path.as_str()),
            ("move", "/tmp/work/b")
        );

        let mut rooted = request.clone();
        rooted["params"]["grantRoot"] = json!("/tmp/work");
        rooted["params"]["reason"] = json!("needs write access");
        let card = approval_card(&rooted, "en");
        assert_eq!(card.paths, ["/tmp/work"]);
        assert_eq!(card.reason, "needs write access");
    }

    #[test]
    fn permissions_card_grants_only_what_was_requested() {
        let card = approval_card(&permissions(), "en");
        assert_eq!(card.kind, "permissions");
        assert_eq!(card.reason, "need to fetch data");
        assert_eq!(card.command, "Network access\nwrite: /tmp/probe-out");
        assert_eq!(card.paths, ["/tmp/probe-out"]);
        assert_eq!(ids(&card), ["approve", "approveSession", "decline"]);
        let granted = json!({"network":{"enabled":true},"fileSystem":{"write":["/tmp/probe-out"],"entries":[{"path":{"type":"path","path":"/tmp/probe-out"},"access":"write"}]}});
        assert_eq!(
            native(&card, "approve"),
            &json!({"permissions": granted, "scope": "turn"})
        );
        assert_eq!(card.decisions[0].scope, "turn");
        assert_eq!(
            native(&card, "approveSession"),
            &json!({"permissions": granted, "scope": "session"})
        );
        assert_eq!(
            native(&card, "decline"),
            &json!({"permissions": {}, "scope": "turn"})
        );

        let mut special = permissions();
        special["params"]["permissions"] = json!({"network": null, "fileSystem": {"entries": [
            {"path": {"type": "glob_pattern", "pattern": "**/*.env"}, "access": "read"},
            {"path": {"type": "special", "value": {"kind": "project_roots", "subpath": "target"}}, "access": "write"}
        ]}});
        let card = approval_card(&special, "en");
        assert_eq!(
            card.command,
            "read: **/*.env\nwrite: <project_roots> target"
        );
        assert!(card.paths.is_empty());
        // Nothing readable to grant: no buttons.
        special["params"]["permissions"] = json!({});
        assert!(approval_card(&special, "en").decisions.is_empty());
    }

    #[test]
    fn mcp_tool_approval_is_a_card_and_a_real_form_is_questions() {
        let tool = mcp_tool();
        assert!(is_approval(&tool) && !is_form(&tool));
        let card = approval_card(&tool, "en");
        assert_eq!(card.kind, "mcpTool");
        assert_eq!(card.title, "probe · delete_record");
        assert_eq!(card.command, "id: 42");
        assert_eq!(card.details, "Delete a record by id (destructive).");
        assert_eq!(
            ids(&card),
            [
                "approve",
                "approveSession",
                "approveAlways",
                "decline",
                "cancel"
            ]
        );
        assert_eq!(
            native(&card, "approve"),
            &json!({"action": "accept", "content": {}})
        );
        assert_eq!(
            native(&card, "approveSession"),
            &json!({"action": "accept", "content": {}, "_meta": {"persist": "session"}})
        );
        assert_eq!(
            native(&card, "approveAlways"),
            &json!({"action": "accept", "content": {}, "_meta": {"persist": "always"}})
        );
        assert_eq!(native(&card, "decline"), &json!({"action": "decline"}));
        assert_eq!(native(&card, "cancel"), &json!({"action": "cancel"}));
        // Persistence is offered only as far as the request allows it.
        let mut once = tool.clone();
        once["params"]["_meta"]["persist"] = json!("session");
        assert_eq!(
            ids(&approval_card(&once, "en")),
            ["approve", "approveSession", "decline", "cancel"]
        );
        once["params"]["_meta"] = Value::Null;
        let plain = approval_card(&once, "en");
        assert_eq!(plain.kind, "generic");
        assert_eq!(ids(&plain), ["approve", "decline", "cancel"]);

        let form = mcp_form();
        assert!(is_form(&form) && !is_approval(&form));
        let card = form_card(&form, "en").unwrap();
        assert_eq!(card["params"]["form"]["serverName"], "probe");
        assert_eq!(
            card["params"]["form"]["description"],
            "Choose deployment settings"
        );
        let keys: Vec<_> = card["params"]["questions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| (q["key"].as_str().unwrap(), q["kind"].as_str().unwrap()))
            .collect();
        assert_eq!(
            keys,
            [
                ("region", "string"),
                ("replicas", "integer"),
                ("comment", "string"),
                ("features", "multiselect"),
                ("notify", "boolean"),
            ]
        );
        let answers = json!({"1": "EU West", "2": "3", "4": ["cache", "waf"], "5": false});
        assert_eq!(
            form_reply(&form, "answer", Some(&answers), None).unwrap(),
            json!({"action": "accept", "content": {"region": "eu-west", "replicas": 3, "features": ["cache", "waf"], "notify": false}})
        );
        assert_eq!(
            form_reply(&form, "skip", None, None).unwrap(),
            json!({"action": "decline"})
        );
        assert_eq!(
            form_reply(&form, "cancel", None, None).unwrap(),
            json!({"action": "cancel"})
        );
        for bad in [
            json!({"1": "Mars", "2": "3"}),
            json!({"1": "EU West", "2": "9"}),
            json!({"2": "3"}),
        ] {
            assert!(
                form_reply(&form, "answer", Some(&bad), None).is_err(),
                "{bad}"
            );
        }
        assert!(form_reply(&form, "approve", None, None).is_err());

        // Anything but a flat form stays in the original window.
        for (key, value) in [
            ("mode", json!("url")),
            ("mode", json!("openai/form")),
            ("mode", json!("openai/userVerification")),
            (
                "requestedSchema",
                json!({"type": "object", "properties": {"a": {"type": "object"}}}),
            ),
        ] {
            let mut other = form.clone();
            other["params"][key] = value;
            assert!(is_form(&other), "{key}");
            assert!(form_card(&other, "en").is_none(), "{key}");
            assert!(form_reply(&other, "cancel", None, None).is_err());
        }
    }

    #[test]
    fn legacy_approvals_use_the_review_decision_vocabulary() {
        let exec = json!({"method":"execCommandApproval","id":3,"params":{"conversationId":THREAD,"callId":"call-1","approvalId":null,"command":["git","push"],"cwd":"/repo","parsedCmd":[],"reason":"Needs network"}});
        assert_eq!(thread_id(&exec), Some(THREAD));
        assert!(turn_optional(&exec) && is_approval(&exec));
        let card = approval_card(&exec, "en");
        assert_eq!(
            (card.command.as_str(), card.cwd.as_str()),
            ("git push", "/repo")
        );
        assert_eq!(
            ids(&card),
            ["approve", "approveSession", "decline", "cancel"]
        );
        assert_eq!(native(&card, "approve"), &json!({"decision": "approved"}));
        assert_eq!(
            native(&card, "approveSession"),
            &json!({"decision": "approved_for_session"})
        );
        assert_eq!(native(&card, "cancel"), &json!({"decision": "abort"}));
        let decline = &card.decisions[2];
        assert!(decline.accepts_reason);
        assert_eq!(
            decision_reply(decline, Some(" not on main ")).unwrap(),
            json!({"decision": {"denied": {"rejection": "not on main"}}})
        );
        assert_eq!(
            decision_reply(decline, None).unwrap(),
            json!({"decision": {"denied": {"rejection": ""}}})
        );
        assert!(decision_reply(&card.decisions[0], Some("why")).is_err());

        let patch = json!({"method":"applyPatchApproval","id":4,"params":{"conversationId":THREAD,"callId":"call-2","fileChanges":{"/repo/new.txt":{"type":"add","content":"hi\n"},"/repo/old.txt":{"type":"delete","content":"bye\n"},"/repo/a.rs":{"type":"update","unified_diff":"@@\n-a\n+b\n","move_path":"/repo/b.rs"}},"reason":null,"grantRoot":"/repo"}});
        let card = approval_card(&patch, "en");
        assert_eq!(card.kind, "fileChange");
        let kinds: Vec<_> = card.changes.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, ["move", "add", "delete"]);
        assert_eq!(card.changes[1].new_text, "hi\n");
        assert_eq!(card.paths, ["/repo"]);
        assert_eq!(
            ids(&card),
            ["approve", "approveSession", "decline", "cancel"]
        );
    }

    #[test]
    fn text_channel_takes_only_the_three_plain_decisions() {
        assert_eq!(
            plain_reply(&command(), "approve").unwrap(),
            json!({"decision": "accept"})
        );
        assert_eq!(
            plain_reply(&command(), "decline").unwrap(),
            json!({"decision": "decline"})
        );
        assert_eq!(
            plain_reply(&permissions(), "decline").unwrap(),
            json!({"permissions": {}, "scope": "turn"})
        );
        assert_eq!(
            plain_reply(&mcp_tool(), "cancel").unwrap(),
            json!({"action": "cancel"})
        );
        assert!(plain_reply(&permissions(), "cancel").is_err());
        let (reason, body) = summary(&mcp_tool(), "en").unwrap();
        assert!(reason.starts_with("Allow the probe MCP server"));
        assert_eq!(body, "probe · delete_record\nid: 42");
    }
}
