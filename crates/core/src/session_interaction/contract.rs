//! Wire contract shared by every interaction adapter and both pet UIs (see docs/session-interaction.md).
//! Adapters translate their native request into these types; `respond` validates the reply against
//! the card that is pending right now, so a UI can only pick what the original session offered.
use crate::channels::{content, opencode};
use anyhow::{Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};

pub(crate) const MAX_INPUT: usize = 8192;
pub(crate) const MAX_ANSWER: usize = 2000;
pub(crate) const MAX_REASON: usize = 2000;
const MAX_PLAN: usize = 12_000;
const MAX_DIFF: usize = 6000;
const DIFF_BUDGET: usize = 24_000;
const MAX_CHANGES: usize = 40;
const MAX_PATHS: usize = 50;

/// What the calling client can do with a result. Absent flags mean "cannot".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// The client runs on the user's desktop session and can deliver a `terminalJob` itself.
    pub local_terminal: bool,
}

impl Capabilities {
    pub fn from_params(params: &Value) -> Self {
        Self {
            local_terminal: params["capabilities"]["localTerminal"] == true,
        }
    }
}

/// One button of an approval card. `id` is what the client sends back as `action`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Decision {
    pub id: String,
    /// Localised for the app language.
    pub label: String,
    /// Catalogue key (the zh-CN source string) for clients that localise themselves.
    pub label_key: String,
    /// `primary` | `secondary` | `danger`.
    pub style: &'static str,
    /// `allow` | `deny` | `cancel`.
    pub behavior: &'static str,
    /// `once` | `turn` | `session` | `always`.
    pub scope: &'static str,
    /// The client may send `reason` (feedback for the agent) with this decision.
    pub accepts_reason: bool,
    /// The decision also stops the agent's current turn.
    pub interrupts: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Adapter-private payload for the native reply. Never leaves the core.
    #[serde(skip)]
    pub native: Value,
}

// The full vocabulary is defined here once; an adapter uses the entries its agent supports.
#[allow(dead_code)]
impl Decision {
    pub fn new(
        id: &str,
        label_key: &'static str,
        style: &'static str,
        behavior: &'static str,
        scope: &'static str,
        lang: &str,
    ) -> Self {
        Self {
            id: id.into(),
            label: crate::localization::translate(lang, label_key).into_owned(),
            label_key: label_key.into(),
            style,
            behavior,
            scope,
            accepts_reason: false,
            interrupts: false,
            description: String::new(),
            native: Value::Null,
        }
    }
    /// `approve`: allow this one request.
    pub fn approve(lang: &str) -> Self {
        Self::new("approve", "允许本次", "primary", "allow", "once", lang)
    }
    /// `approveSession`: allow this kind of request until the session ends.
    pub fn approve_session(lang: &str) -> Self {
        Self::new(
            "approveSession",
            "本会话内允许",
            "secondary",
            "allow",
            "session",
            lang,
        )
    }
    /// `approveAlways`: persist a permission rule.
    pub fn approve_always(lang: &str) -> Self {
        Self::new(
            "approveAlways",
            "始终允许",
            "secondary",
            "allow",
            "always",
            lang,
        )
    }
    /// `decline`: refuse this one request.
    pub fn decline(lang: &str) -> Self {
        Self::new("decline", "拒绝", "danger", "deny", "once", lang)
    }
    /// `declineInterrupt`: refuse and stop the turn.
    pub fn decline_interrupt(lang: &str) -> Self {
        let mut d = Self::new(
            "declineInterrupt",
            "拒绝并中断",
            "danger",
            "deny",
            "once",
            lang,
        );
        d.interrupts = true;
        d
    }
    /// `cancel`: withdraw the request without answering it.
    pub fn cancel(lang: &str) -> Self {
        let mut d = Self::new("cancel", "取消", "secondary", "cancel", "once", lang);
        d.interrupts = true;
        d
    }
    pub fn with_reason(mut self) -> Self {
        self.accepts_reason = true;
        self
    }
    pub fn with_native(mut self, native: Value) -> Self {
        self.native = native;
        self
    }
    pub fn with_description(mut self, text: &str) -> Self {
        self.description = content::clip(text, 300);
        self
    }
}

/// A file touched by the request, for diff rendering.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileChange {
    pub path: String,
    /// `add` | `update` | `delete` | `move` | `unknown`.
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub move_path: String,
    /// Unified diff, when the agent supplies one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub diff: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub old_text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub new_text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Plan {
    /// Markdown as the agent wrote it (not JSON-escaped).
    pub markdown: String,
    pub truncated: bool,
}

/// Approval card. The first four fields are the original contract and are always present.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Approval {
    pub tool_name: String,
    pub reason: String,
    pub command: String,
    pub details: String,
    /// `command` | `fileChange` | `plan` | `permissions` | `mcpTool` | `tool` | `generic`.
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FileChange>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    pub decisions: Vec<Decision>,
}

fn clip_flag(text: &str, limit: usize) -> (String, bool) {
    (content::clip(text, limit), text.len() > limit)
}

impl Approval {
    pub fn new(tool_name: &str, kind: &str) -> Self {
        Self {
            tool_name: tool_name.into(),
            kind: kind.into(),
            ..Default::default()
        }
    }
    /// A card for a request the core can show but not answer.
    pub fn readonly(tool_name: &str, reason: &str) -> Self {
        let mut card = Self::new(tool_name, "generic");
        card.reason = reason.into();
        card
    }
    pub fn set_plan(&mut self, markdown: &str) {
        let (markdown, truncated) = clip_flag(markdown, MAX_PLAN);
        self.plan = Some(Plan {
            markdown,
            truncated,
        });
    }
    /// Bound every field so one oversized request cannot stall the popup.
    pub fn bounded(mut self) -> Self {
        self.reason = content::clip(&self.reason, 800);
        self.command = content::clip(&self.command, 4000);
        self.details = content::clip(&self.details, 2000);
        self.title = content::clip(&self.title, 200);
        self.cwd = content::clip(&self.cwd, 1000);
        self.changes.truncate(MAX_CHANGES);
        let mut budget = DIFF_BUDGET;
        for change in &mut self.changes {
            change.path = content::clip(&change.path, 1000);
            change.move_path = content::clip(&change.move_path, 1000);
            for field in [&mut change.diff, &mut change.old_text, &mut change.new_text] {
                let limit = MAX_DIFF.min(budget);
                if field.len() > limit {
                    change.truncated = true;
                    *field = if limit < 64 {
                        String::new()
                    } else {
                        content::clip(field, limit)
                    };
                }
                budget = budget.saturating_sub(field.len());
            }
        }
        self.paths.truncate(MAX_PATHS);
        for path in &mut self.paths {
            *path = content::clip(path, 1000);
        }
        self
    }
}

/// Whether a follow-up message can be sent to a session that is not waiting for input.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FollowUp {
    pub available: bool,
    /// `codex` | `opencode` | `tmux` | `macosTerminal` | `` when unavailable.
    pub adapter: String,
    /// `newTurn` (native API starts the next turn) | `queue` (held until the running turn ends)
    /// | `terminal` (typed into the original terminal).
    pub mode: String,
    /// Why it is unavailable, localised; empty when available.
    pub reason: String,
    /// The `interrupt` action can stop the session's running turn.
    pub can_interrupt: bool,
}

impl FollowUp {
    pub fn available(adapter: &str, mode: &str) -> Self {
        Self {
            available: true,
            adapter: adapter.into(),
            mode: mode.into(),
            reason: String::new(),
            can_interrupt: false,
        }
    }
    pub fn unavailable(reason: String) -> Self {
        Self {
            reason,
            ..Default::default()
        }
    }
}

/// What the client asked for, decoded from `action` / `decisionId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verb {
    Answer,
    FollowUp,
    /// Withdraw a form or elicitation; on an approval card this is the `cancel` decision.
    Cancel,
    /// Decline to fill a form (elicitation "decline").
    Skip,
    Decide(String),
}

impl Verb {
    /// The decision id this verb names on an approval card.
    pub fn decision_id(&self) -> Option<&str> {
        match self {
            Self::Decide(id) => Some(id),
            Self::Cancel => Some("cancel"),
            _ => None,
        }
    }
    /// Text meant as an answer or instruction, as opposed to a button.
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Answer | Self::FollowUp)
    }
}

pub(crate) fn valid_decision_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    id.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// `action` is a verb (`answer`, `followup`, `cancel`, `skip`, `decide`) or a decision id.
/// `accept` and `deny` are the original aliases of `approve` and `decline`.
pub(crate) fn parse_verb(params: &Value) -> Result<Verb> {
    let action = params["action"].as_str().unwrap_or("answer");
    let explicit = params["decisionId"].as_str().filter(|s| !s.is_empty());
    let decide = |id: &str| -> Result<Verb> {
        let id = match id {
            "accept" => "approve",
            "deny" => "decline",
            other => other,
        };
        ensure!(valid_decision_id(id), "不支持的审批操作");
        Ok(if id == "cancel" {
            Verb::Cancel
        } else {
            Verb::Decide(id.to_owned())
        })
    };
    match (action, explicit) {
        ("decide", Some(id)) => decide(id),
        ("decide", None) => bail!("不支持的审批操作"),
        (_, Some(_)) => bail!("不支持的审批操作"),
        ("" | "answer", None) => Ok(Verb::Answer),
        ("followup", None) => Ok(Verb::FollowUp),
        ("skip", None) => Ok(Verb::Skip),
        (other, None) => decide(other),
    }
}

fn clean(text: &str, limit: usize) -> bool {
    text.len() <= limit
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}

pub(crate) fn validate_text(text: Option<&str>) -> Result<()> {
    ensure!(
        text.is_none_or(|t| clean(t, MAX_INPUT)),
        "输入内容无效或过长"
    );
    Ok(())
}

pub(crate) fn validate_reason(reason: Option<&str>) -> Result<()> {
    ensure!(
        reason.is_none_or(|t| clean(t, MAX_REASON)),
        "输入内容无效或过长"
    );
    Ok(())
}

/// Shape and size only: an object of question id to a string, a list of strings, a boolean or a number.
pub(crate) fn validate_answers(answers: &Value) -> Result<()> {
    if answers.is_null() {
        return Ok(());
    }
    let map = answers.as_object();
    ensure!(
        map.is_some_and(|m| !m.is_empty() && m.len() <= 32),
        "输入内容无效或过长"
    );
    let mut total = 0usize;
    for (key, value) in map.into_iter().flatten() {
        ensure!(
            !key.is_empty() && clean(key, MAX_ANSWER),
            "输入内容无效或过长"
        );
        total += key.len();
        match value {
            Value::String(s) => {
                ensure!(clean(s, MAX_ANSWER), "输入内容无效或过长");
                total += s.len();
            }
            Value::Array(items) => {
                ensure!(items.len() <= 50, "输入内容无效或过长");
                for item in items {
                    let s = item.as_str().filter(|s| clean(s, MAX_ANSWER));
                    ensure!(s.is_some(), "输入内容无效或过长");
                    total += s.map_or(0, str::len);
                }
            }
            Value::Bool(_) | Value::Number(_) => total += 8,
            _ => bail!("输入内容无效或过长"),
        }
    }
    ensure!(total <= MAX_INPUT, "输入内容无效或过长");
    Ok(())
}

/// The offered decision named by the client; anything the original session did not offer is refused.
pub(crate) fn pick_decision<'a>(
    card: &'a Approval,
    verb: &Verb,
    reason: Option<&str>,
) -> Result<&'a Decision> {
    let id = verb
        .decision_id()
        .ok_or_else(|| anyhow::anyhow!("不支持的审批操作"))?;
    let decision = card
        .decisions
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| anyhow::anyhow!("原会话不允许此审批选项"))?;
    ensure!(
        decision.accepts_reason || reason.is_none_or(|r| r.trim().is_empty()),
        "此审批选项不支持附加说明"
    );
    Ok(decision)
}

fn option_matches(option: &Value, answer: &str) -> bool {
    let answer = answer.trim();
    option["label"]
        .as_str()
        .is_some_and(|label| label.trim().eq_ignore_ascii_case(answer))
        || match &option["value"] {
            Value::String(s) => s == answer,
            Value::Null => false,
            // Compared as JSON text: `Value == &str` would only match strings.
            other => {
                let text = other.to_string();
                text == answer
            }
        }
}

/// Check structured answers against the normalised questions of the pending card: only known
/// questions, every required one answered, one choice unless multi-select, and listed options
/// unless the question allows a custom answer.
pub(crate) fn check_answers(questions: &[Value], answers: &Value) -> Result<()> {
    let Some(map) = answers.as_object() else {
        return Ok(());
    };
    let mut used = 0usize;
    for q in questions {
        let found = ["id", "key", "question"]
            .iter()
            .filter_map(|k| q[*k].as_str().filter(|s| !s.is_empty()))
            .find_map(|k| map.get(k));
        let Some(value) = found else {
            ensure!(q["required"] == false, "请回答所有问题后再提交");
            continue;
        };
        used += 1;
        let picked: Vec<&str> = match value {
            Value::String(s) => vec![s.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            // Typed answers (boolean and number fields) are checked by the adapter.
            _ => continue,
        };
        let picked: Vec<&str> = picked
            .into_iter()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        ensure!(
            !picked.is_empty() || q["required"] == false,
            "问题回答不能为空"
        );
        if picked.is_empty() {
            // A blank answer to an optional question is the same as leaving it out.
            continue;
        }
        ensure!(
            q["multiSelect"] == true || value.is_string() || picked.len() <= 1,
            "此问题只能选择一个选项"
        );
        let options = q["options"].as_array().map_or(&[][..], Vec::as_slice);
        if !options.is_empty() && q["allowCustom"] != true && value.is_array() {
            ensure!(
                picked
                    .iter()
                    .all(|answer| options.iter().any(|o| option_matches(o, answer))),
                "回答不在可选项中，请重新选择"
            );
        } else if !options.is_empty() && q["allowCustom"] != true {
            // A single string may be one label or, for multi-select, a comma separated list.
            let whole = options.iter().any(|o| option_matches(o, picked[0]));
            let parts = q["multiSelect"] == true
                && picked[0]
                    .split([',', '，'])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .all(|part| options.iter().any(|o| option_matches(o, part)));
            ensure!(whole || parts, "回答不在可选项中，请重新选择");
        }
    }
    ensure!(used == map.len(), "回答与原问题不符，请刷新后再操作");
    Ok(())
}

/// Form-level metadata an adapter attaches as `params.form` (plan, elicitation, multi-field forms).
pub(crate) fn form_meta(request: &Value) -> Value {
    let form = &request["params"]["form"];
    if !form.is_object() {
        return Value::Null;
    }
    let text = |key: &str, limit: usize| content::clip(form[key].as_str().unwrap_or(""), limit);
    json!({
        "title": text("title", 200),
        "description": text("description", 1600),
        // `mcp` | `tool` | `agent`.
        "source": text("source", 40),
        "serverName": text("serverName", 200),
        "url": text("url", 2000),
        "canCancel": form["canCancel"] == true,
        "canSkip": form["canSkip"] == true,
    })
}

/// Requests waiting on this session, including the one shown. Adapters set `params.pendingCount`.
pub(crate) fn pending_count(request: &Value) -> u64 {
    request["params"]["pendingCount"]
        .as_u64()
        .unwrap_or(1)
        .max(1)
}

/// Adapter question rows to the card's `questions`. Rows may carry `header`, `description`,
/// `isSecret`/`secret`, `isOther`/`allowCustom`/`custom`, `required`, `placeholder`, `default`
/// and `constraints`; unknown keys are dropped.
pub(crate) fn normalize_questions(request: &Value) -> Vec<Value> {
    let method = request["method"].as_str().unwrap_or("");
    let allow_custom_default = matches!(
        method,
        "aieyes/claudeQuestion"
            | "aieyes/asyncQuestion"
            | "item/tool/requestUserInput"
            | "tool/requestUserInput"
    );
    request["params"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, q)| {
            let id = q["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| (i + 1).to_string());
            let options = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    let label = o["label"]
                        .as_str()
                        .or_else(|| o.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let mut item = json!({"label": content::clip(&label, 300)});
                    if let Some(desc) = o["description"].as_str().filter(|s| !s.is_empty()) {
                        item["description"] = json!(content::clip(desc, 600));
                    }
                    if let Some(prev) = o["preview"].as_str().filter(|s| !s.trim().is_empty()) {
                        item["preview"] = json!(content::clip(prev, 2500));
                    }
                    if !o["value"].is_null() {
                        item["value"] = o["value"].clone();
                    }
                    item
                })
                .collect::<Vec<_>>();
            let allow_custom = if options.is_empty() {
                true
            } else if let Some(flag) = q["isOther"]
                .as_bool()
                .or_else(|| q["allowCustom"].as_bool())
            {
                flag
            } else if method == opencode::FORM {
                q["custom"] == true
            } else {
                allow_custom_default
            };
            let mut item = json!({
                "id": id,
                "key": q["key"].as_str().unwrap_or(""),
                "header": content::clip(q["header"].as_str().unwrap_or(""), 200),
                "question": content::clip(q["question"].as_str().unwrap_or(""), 1600),
                "kind": q["kind"].as_str().unwrap_or("string"),
                "multiSelect": q["multiSelect"] == true,
                "allowCustom": allow_custom,
                "secret": q["isSecret"] == true || q["secret"] == true,
                "required": q["required"] != false,
                "options": options,
            });
            for (key, limit) in [("description", 1600), ("placeholder", 200)] {
                if let Some(text) = q[key].as_str().filter(|s| !s.is_empty()) {
                    item[key] = json!(content::clip(text, limit));
                }
            }
            if matches!(
                q["default"],
                Value::String(_) | Value::Bool(_) | Value::Number(_)
            ) {
                item["default"] = q["default"].clone();
            }
            if let Some(constraints) = q["constraints"].as_object() {
                let kept: serde_json::Map<String, Value> = constraints
                    .iter()
                    .filter(|(k, v)| {
                        matches!(
                            k.as_str(),
                            "minimum"
                                | "maximum"
                                | "minLength"
                                | "maxLength"
                                | "minItems"
                                | "maxItems"
                                | "pattern"
                                | "format"
                        ) && (v.is_number() || v.as_str().is_some_and(|s| s.len() <= 200))
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !kept.is_empty() {
                    item["constraints"] = Value::Object(kept);
                }
            }
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_keep_the_original_actions_and_refuse_ambiguous_ones() {
        let verb = |v: Value| parse_verb(&v);
        assert_eq!(verb(json!({})).unwrap(), Verb::Answer);
        assert_eq!(verb(json!({"action":"answer"})).unwrap(), Verb::Answer);
        assert_eq!(verb(json!({"action":"followup"})).unwrap(), Verb::FollowUp);
        assert_eq!(verb(json!({"action":"skip"})).unwrap(), Verb::Skip);
        assert_eq!(verb(json!({"action":"cancel"})).unwrap(), Verb::Cancel);
        for (alias, id) in [
            ("accept", "approve"),
            ("approve", "approve"),
            ("deny", "decline"),
        ] {
            assert_eq!(
                verb(json!({"action": alias})).unwrap(),
                Verb::Decide(id.into())
            );
        }
        assert_eq!(
            verb(json!({"action":"decide","decisionId":"approveSession"})).unwrap(),
            Verb::Decide("approveSession".into())
        );
        assert_eq!(
            verb(json!({"action":"decide","decisionId":"cancel"})).unwrap(),
            Verb::Cancel
        );
        assert!(verb(json!({"action":"decide"})).is_err());
        assert!(verb(json!({"action":"answer","decisionId":"approve"})).is_err());
        assert!(verb(json!({"action":"rm -rf /"})).is_err());
        assert!(verb(json!({"action":"9lives"})).is_err());
        assert_eq!(Verb::Cancel.decision_id(), Some("cancel"));
        assert_eq!(Verb::Answer.decision_id(), None);
    }

    #[test]
    fn decisions_are_limited_to_what_the_card_offers() {
        let mut card = Approval::new("Bash", "command");
        card.decisions = vec![
            Decision::approve("en"),
            Decision::approve_session("en").with_native(json!({"scope":"session"})),
            Decision::decline("en").with_reason(),
        ];
        let pick = |id: &str, reason: Option<&str>| {
            pick_decision(&card, &Verb::Decide(id.into()), reason).map(|d| d.id.clone())
        };
        assert_eq!(pick("approve", None).unwrap(), "approve");
        assert_eq!(
            pick("approveSession", Some("  ")).unwrap(),
            "approveSession"
        );
        assert_eq!(pick("decline", Some("use rg instead")).unwrap(), "decline");
        // Not offered by the original session.
        assert!(pick("approveAlways", None).is_err());
        assert!(pick_decision(&card, &Verb::Cancel, None).is_err());
        assert!(pick_decision(&card, &Verb::Answer, None).is_err());
        // Feedback is only sent where the agent will read it.
        assert!(pick("approve", Some("because")).is_err());

        let wire = serde_json::to_value(&card).unwrap();
        assert_eq!(wire["toolName"], "Bash");
        assert_eq!(wire["kind"], "command");
        assert_eq!(wire["decisions"][1]["id"], "approveSession");
        assert_eq!(wire["decisions"][1]["scope"], "session");
        assert_eq!(wire["decisions"][1]["behavior"], "allow");
        assert_eq!(wire["decisions"][1]["labelKey"], "本会话内允许");
        assert_eq!(wire["decisions"][2]["acceptsReason"], true);
        assert_eq!(wire["decisions"][0]["interrupts"], false);
        // Adapter payloads stay inside the core.
        assert!(wire["decisions"][1].get("native").is_none());
        assert!(wire.get("plan").is_none() && wire.get("changes").is_none());
    }

    #[test]
    fn standard_decisions_are_localised_and_typed() {
        assert_eq!(Decision::approve("en").label, "Allow once");
        assert_eq!(Decision::approve("zh-CN").label, "允许本次");
        assert_ne!(Decision::approve_session("en").label, "本会话内允许");
        assert_ne!(Decision::approve_always("en").label, "始终允许");
        let stop = Decision::decline_interrupt("en");
        assert!(stop.interrupts && stop.behavior == "deny");
        assert_ne!(stop.label, "拒绝并中断");
        let cancel = Decision::cancel("zh-CN");
        assert_eq!((cancel.id.as_str(), cancel.behavior), ("cancel", "cancel"));
    }

    #[test]
    fn text_reason_and_answers_are_bounded_and_free_of_control_characters() {
        assert!(validate_text(Some("line one\n\tline two")).is_ok());
        assert!(validate_text(Some("esc\u{1b}[2J")).is_err());
        assert!(validate_text(Some(&"x".repeat(MAX_INPUT + 1))).is_err());
        assert!(validate_reason(Some(&"x".repeat(MAX_REASON + 1))).is_err());
        assert!(validate_reason(Some("bell\u{7}")).is_err());

        assert!(validate_answers(&Value::Null).is_ok());
        assert!(validate_answers(&json!({"1":"A","2":["B","C"],"3":true,"4":3})).is_ok());
        assert!(validate_answers(&json!({})).is_err());
        assert!(validate_answers(&json!("A")).is_err());
        assert!(validate_answers(&json!({"1":null})).is_err());
        assert!(validate_answers(&json!({"1":{"nested":"x"}})).is_err());
        assert!(validate_answers(&json!({"1":["ok",7]})).is_err());
        assert!(validate_answers(&json!({"1":"ctl\u{0}"})).is_err());
        assert!(validate_answers(&json!({"1":"x".repeat(MAX_ANSWER + 1)})).is_err());
        let many: serde_json::Map<String, Value> = (0..5)
            .map(|i| (i.to_string(), json!("y".repeat(MAX_ANSWER))))
            .collect();
        assert!(validate_answers(&Value::Object(many)).is_err());
    }

    #[test]
    fn questions_keep_header_secret_and_custom_flags() {
        let request = json!({"method":"item/tool/requestUserInput","params":{"questions":[
            {"id":"token","header":"Auth","question":"API token?","isSecret":true,"isOther":true,"options":null},
            {"id":"mode","header":"Mode","question":"Pick","isOther":false,
             "options":[{"label":"Fast","description":"Less checking"},{"label":"Safe"}]},
            {"id":"open","question":"Pick or type","options":[{"label":"A"}]}
        ]}});
        let rows = normalize_questions(&request);
        assert_eq!(rows[0]["header"], "Auth");
        assert_eq!(rows[0]["secret"], true);
        assert_eq!(rows[0]["allowCustom"], true);
        assert_eq!(rows[1]["secret"], false);
        assert_eq!(rows[1]["allowCustom"], false);
        assert_eq!(rows[1]["options"][0]["description"], "Less checking");
        assert_eq!(rows[1]["required"], true);
        // Without an explicit flag the original default for the method applies.
        assert_eq!(rows[2]["allowCustom"], true);

        let form = json!({"method":opencode::FORM,"params":{
        "pendingCount":3,
        "form":{"title":"Deploy","canCancel":true,"source":"mcp","serverName":"ops"},
        "questions":[
            {"id":"1","key":"env","kind":"string","question":"Environment","required":false,
             "options":[{"label":"prod","value":"production"}],"custom":false,
             "constraints":{"maxLength":20,"pattern":"^[a-z]+$","x-internal":true},
             "placeholder":"staging","default":"staging"},
            {"id":"2","key":"note","kind":"string","question":"Note","options":[]}
        ]}});
        let rows = normalize_questions(&form);
        assert_eq!(rows[0]["allowCustom"], false);
        assert_eq!(rows[0]["required"], false);
        assert_eq!(
            rows[0]["constraints"],
            json!({"maxLength":20,"pattern":"^[a-z]+$"})
        );
        assert_eq!(rows[0]["placeholder"], "staging");
        assert_eq!(rows[0]["default"], "staging");
        assert_eq!(rows[1]["allowCustom"], true);
        assert_eq!(pending_count(&form), 3);
        assert_eq!(pending_count(&request), 1);
        let meta = form_meta(&form);
        assert_eq!(meta["title"], "Deploy");
        assert_eq!(meta["canCancel"], true);
        assert_eq!(meta["canSkip"], false);
        assert_eq!(meta["serverName"], "ops");
        assert!(form_meta(&request).is_null());
    }

    #[test]
    fn answers_must_match_the_pending_questions() {
        let request = json!({"method":"aieyes/readonlyQuestions","params":{"questions":[
            {"id":"1","question":"Target","options":[{"label":"macOS"},{"label":"Windows","value":"win"}]},
            {"id":"2","question":"Extras","multiSelect":true,"options":[{"label":"Docs"},{"label":"Tests"}]},
            {"id":"3","question":"Note","required":false}
        ]}});
        let questions = normalize_questions(&request);
        let check = |answers: Value| check_answers(&questions, &answers);
        assert!(check(json!({"1":"macos","2":["Docs","Tests"]})).is_ok());
        assert!(check(json!({"1":"win","2":"Docs, Tests","3":"free text"})).is_ok());
        assert!(check(json!({"Target":"macOS","Extras":["Docs"]})).is_ok());
        // Missing a required question.
        assert!(check(json!({"1":"macOS"})).is_err());
        // Not an offered option.
        assert!(check(json!({"1":"Linux","2":["Docs"]})).is_err());
        assert!(check(json!({"1":"macOS","2":["Docs","Deploy"]})).is_err());
        // Several choices for a single-select question.
        assert!(check(json!({"1":["macOS","Windows"],"2":["Docs"]})).is_err());
        // An answer for a question that is not on the card.
        assert!(check(json!({"1":"macOS","2":["Docs"],"9":"x"})).is_err());
        assert!(check(json!({"1":"  ","2":["Docs"]})).is_err());
        // Text replies are checked by the adapter.
        assert!(check(Value::Null).is_ok());
    }

    /// A blank answer to an optional choice field is the same as leaving the field out.
    #[test]
    fn blank_answer_to_an_optional_choice_is_treated_as_omitted() {
        let form = json!({"method":opencode::FORM,"params":{"questions":[
            {"id":"1","key":"env","kind":"string","question":"Environment","required":false,
             "options":[{"label":"prod","value":"production"}],"custom":false},
            {"id":"2","key":"zone","kind":"string","question":"Zone","required":true,
             "options":[{"label":"a"}],"custom":false}
        ]}});
        let questions = normalize_questions(&form);
        assert_eq!(questions[0]["required"], false);
        assert_eq!(questions[0]["allowCustom"], false);
        let check = |answers: Value| check_answers(&questions, &answers);
        assert!(check(json!({"1":"","2":"a"})).is_ok());
        assert!(check(json!({"1":"  ","2":"a"})).is_ok());
        assert!(check(json!({"1":[],"2":"a"})).is_ok());
        assert!(check(json!({"1":"prod","2":"a"})).is_ok());
        assert!(check(json!({"1":"staging","2":"a"})).is_err());
        // A required field still needs a value.
        assert!(check(json!({"1":"","2":""})).is_err());
    }

    #[test]
    fn approval_cards_are_bounded_and_flag_truncation() {
        let mut card = Approval::new("apply_patch", "fileChange");
        card.command = "c".repeat(9000);
        card.set_plan(&"p".repeat(MAX_PLAN + 10));
        card.changes = (0..6)
            .map(|i| FileChange {
                path: format!("src/file{i}.rs"),
                kind: "update".into(),
                diff: "d".repeat(MAX_DIFF + 500),
                ..Default::default()
            })
            .collect();
        card.paths = (0..80).map(|i| format!("p{i}")).collect();
        let card = card.bounded();
        assert!(card.command.len() <= 4000);
        assert!(card.plan.as_ref().is_some_and(|p| p.truncated));
        assert!(
            card.changes
                .iter()
                .all(|c| c.truncated && c.diff.len() <= MAX_DIFF)
        );
        assert!(card.changes.iter().map(|c| c.diff.len()).sum::<usize>() <= DIFF_BUDGET);
        assert!(card.changes[5].diff.is_empty());
        assert_eq!(card.paths.len(), MAX_PATHS);
        let small = FileChange {
            path: "a".into(),
            kind: "add".into(),
            new_text: "fn main() {}".into(),
            ..Default::default()
        };
        let wire = serde_json::to_value(&small).unwrap();
        assert_eq!(
            wire,
            json!({"path":"a","kind":"add","newText":"fn main() {}","truncated":false})
        );
    }

    #[test]
    fn only_an_explicit_capability_enables_local_terminal_jobs() {
        assert!(!Capabilities::from_params(&json!({})).local_terminal);
        assert!(
            !Capabilities::from_params(&json!({"capabilities":{"localTerminal":"yes"}}))
                .local_terminal
        );
        assert!(!Capabilities::from_params(&json!({"localTerminal":true})).local_terminal);
        assert!(
            Capabilities::from_params(&json!({"capabilities":{"localTerminal":true}}))
                .local_terminal
        );
    }
}
