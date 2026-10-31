use crate::contracts::{FIVE_HOUR_MINUTES, WEEK_MINUTES};
use crate::models::*;
use serde_json::Value;

pub const PARSER_VERSION: u32 = 6;
pub fn parser_version(provider: &str) -> u32 {
    if provider == "codex" {
        return 10;
    }
    if provider == "codebuddy" {
        return 11;
    }
    if provider == "kimi" {
        return 9;
    }
    if crate::opencode::family(provider) {
        return 8;
    }
    if provider == "qoder" {
        return 11;
    }
    if provider == "deepseek-harness" {
        return 9;
    }
    if matches!(provider, "pi" | "omp") {
        return 8;
    }
    if provider == "claude" {
        return 7;
    }
    if provider == "workbuddy" {
        return 7;
    }
    if provider == "grok" {
        8
    } else {
        PARSER_VERSION
    }
}

fn count(v: &Value, name: &str) -> u64 {
    v.get(name).and_then(Value::as_u64).unwrap_or(0)
}

pub fn codex_tokens(v: &Value) -> Option<Tokens> {
    let all_input = v.get("input_tokens")?.as_u64()?;
    let cache_read = count(v, "cached_input_tokens");
    let cache_write = count(v, "cache_write_input_tokens");
    Some(Tokens {
        input: all_input
            .checked_sub(cache_read)?
            .checked_sub(cache_write)?,
        output: count(v, "output_tokens"),
        cache_read,
        cache_write,
        reasoning: count(v, "reasoning_output_tokens"),
    })
}

/// ChatGPT Work shares Codex metering; only the displayed agent differs.
fn codex_agent(state: &ParseState) -> String {
    match state.originator.as_str() {
        crate::session_events::CODEX_WORK_ORIGINATOR => "chatgpt-work".into(),
        "" => String::new(),
        _ => "codex".into(),
    }
}

fn event(
    source: &Source,
    state: &ParseState,
    stamp: i64,
    model: String,
    tokens: Tokens,
    identity: String,
    attribution: &str,
) -> UsageEvent {
    UsageEvent {
        usage_version: 0,
        turn_id: state.turn_id.clone(),
        id: event_id(&source.provider, &source.id, &source.account_id, &identity),
        import_identity: Some(identity),
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: crate::providers::canonical(&source.provider).into(),
        session_id: state.session_id.clone(),
        timestamp: stamp,
        model,
        tokens,
        attribution: attribution.into(),
        billing: BillingEvidence {
            category: "unknown".into(),
            evidence: state.model_provider.clone(),
        },
        native_credits: None,
        raw_tokens: None,
        inheritance: None,
        interval_start: None,
        interval_evidence: String::new(),
        agent: codex_agent(state),
    }
}

pub fn event_id(provider: &str, source_id: &str, account_id: &str, identity: &str) -> String {
    let provider = crate::providers::canonical(provider);
    let namespace = if account_id.is_empty() {
        format!("source:{source_id}")
    } else {
        account_id.to_string()
    };
    hash(&format!("{provider}:{namespace}:{identity}"))
}

pub fn parse(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    state.parser_version = parser_version(&source.provider);
    state.counter_correction = None;
    match source.provider.as_str() {
        "codex" => parse_codex(source, state, v),
        "qoder" | "pi" | "omp" if v["tokens"].is_object() => parse_normalized(source, state, v),
        "claude" | "qoder" => parse_claude(source, state, v),
        "pi" | "omp" => crate::pi_history::parse(source, state, v),
        "kimi" | "codebuddy" | "copilot" => crate::native_agents::parse(source, state, v),
        "deepseek-harness" if v["type"].is_string() => {
            crate::native_agents::parse_dsh(source, state, v)
        }
        _ => parse_normalized(source, state, v),
    }
}

fn parse_codex(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    let payload = &v["payload"];
    match v["type"].as_str()? {
        "event_msg" if payload["type"] == "task_started" => {
            state.task_started_at = timestamp(&v["timestamp"]);
            state.turn_id = payload["turn_id"].as_str().unwrap_or("").into();
            None
        }
        "event_msg" if payload["type"] == "task_complete" || payload["type"] == "turn_aborted" => {
            state.task_started_at = None;
            None
        }
        "session_meta" => {
            if let Some(id) = payload["id"].as_str().or(payload["session_id"].as_str())
                && id != state.session_id
            {
                *state = ParseState {
                    session_id: id.into(),
                    parser_version: parser_version(&source.provider),
                    precise_counting: state.precise_counting,
                    counting_generation: state.counting_generation,
                    ..Default::default()
                };
            }
            state.session_started_at =
                timestamp(&payload["timestamp"]).or_else(|| timestamp(&v["timestamp"]));
            state.model_provider = payload["model_provider"].as_str().unwrap_or("").into();
            if let Some(originator) = payload["originator"].as_str().filter(|s| !s.is_empty()) {
                state.originator = originator.into();
            }
            if let Some(id) = payload["id"].as_str().or(payload["session_id"].as_str()) {
                state.session_id = id.into();
            }
            if payload["history_base"].is_object() && state.last_cumulative.is_none() {
                state.inherited_history = Some(crate::codex_history::InheritedHistory {
                    parent_thread: payload["history_base"]["thread_id"]
                        .as_str()
                        .unwrap_or("")
                        .into(),
                    end_byte_offset: payload["history_base"]["end_byte_offset"]
                        .as_u64()
                        .unwrap_or(0),
                    issue: if state.precise_counting.unwrap_or(true) {
                        "继承历史尚未解析".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                });
                state.first_inherited_count = true;
            }
            None
        }
        "turn_context" => {
            state.turn_id = payload["turn_id"].as_str().unwrap_or("").into();
            if let Some(model) = payload["model"].as_str() {
                state.model = model.into();
            }
            None
        }
        "event_msg" if payload["type"] == "token_count" => {
            let stamp = timestamp(&v["timestamp"])?;
            let cumulative = codex_tokens(&payload["info"]["total_token_usage"])?;
            let inherited = state
                .first_inherited_count
                .then(|| state.inherited_history.clone())
                .flatten();
            if state.last_cumulative.as_ref() == Some(&cumulative) && inherited.is_none() {
                return None;
            }
            let mut interval_start = (state.last_timestamp > 0)
                .then_some(state.last_timestamp)
                .or(state.session_started_at);
            let precise = state.precise_counting.unwrap_or(true);
            let (tokens, attribution) = if inherited.is_some() && !precise {
                (cumulative.clone(), "session-summary")
            } else if inherited.as_ref().is_some_and(|h| h.tokens.is_none()) {
                // Keep a recoverable, zero-valued tombstone instead of charging an
                // inherited counter whose parent has not arrived yet.
                (Tokens::default(), "unresolved-inheritance")
            } else {
                match &state.last_cumulative {
                    None => (cumulative.clone(), "session-summary"),
                    Some(prev) => {
                        if let Some(delta) = cumulative.checked_delta(prev) {
                            (delta, "event")
                        } else {
                            if inherited.is_none() {
                                state.cumulative_segment += 1;
                            }
                            (
                                codex_tokens(&payload["info"]["last_token_usage"])?,
                                "counter-reset",
                            )
                        }
                    }
                }
            };
            // A task boundary is only stronger evidence when this whole delta is
            // exactly the last request. Never relocate an accumulated summary.
            let task_boundary = state.task_started_at.filter(|start| {
                *start <= stamp
                    && interval_start.is_none_or(|old| *start > old)
                    && codex_tokens(&payload["info"]["last_token_usage"]).as_ref() == Some(&tokens)
            });
            if let Some(start) = task_boundary {
                interval_start = Some(start);
            }
            state.last_cumulative = Some(cumulative.clone());
            state.last_timestamp = stamp;
            state.first_inherited_count = false;
            if tokens.total() == 0 && inherited.is_none() {
                return None;
            }
            let identity = format!(
                "{}:{}:{}:{}",
                state.session_id,
                state.cumulative_segment,
                stamp,
                serde_json::to_string(&cumulative).ok()?
            );
            let mut parsed = event(
                source,
                state,
                stamp,
                if state.model.is_empty() {
                    "unknown".into()
                } else {
                    state.model.clone()
                },
                tokens,
                identity,
                attribution,
            );
            let limits = &payload["rate_limits"];
            // A cumulative catch-up can cross providers or turns. Only the last
            // request's complete delta may inherit its response's quota evidence.
            let request_complete =
                codex_tokens(&payload["info"]["last_token_usage"]).as_ref() == Some(&parsed.tokens);
            if request_complete {
                parsed.billing.evidence = "codex-request-v4".into();
            }
            if request_complete
                && limits["plan_type"]
                    .as_str()
                    .is_some_and(|p| !["api", "free", ""].contains(&p))
                && (limits["primary"].is_object() || limits["secondary"].is_object())
            {
                parsed.billing = BillingEvidence {
                    category: "subscription".into(),
                    evidence: "codex-response-rate-limits".into(),
                };
            }
            parsed.interval_start = interval_start;
            if task_boundary.is_some() {
                parsed.interval_evidence = "task-request-v2".into();
            }
            if let Some(history) = inherited {
                if precise && history.tokens.is_some() {
                    parsed.attribution = "inherited-counter".into();
                }
                state.counter_correction = Some(crate::codex_history::CounterCorrection {
                    event: parsed.clone(),
                    history,
                });
            }
            (parsed.tokens.total() > 0).then_some(parsed)
        }
        _ => None,
    }
}

fn parse_claude(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    if v["type"] != "assistant" || v["isApiErrorMessage"] == true {
        return None;
    }
    let msg = &v["message"];
    let model = msg["model"].as_str()?;
    if model.starts_with('<') {
        return None;
    }
    if source.provider == "qoder" {
        if let Some(id) = v["sessionId"].as_str() {
            if state.session_id != id {
                state.qoder_main_model.clear();
            }
            state.session_id = id.into();
        }
        if state.qoder_main_model.is_empty() && v["isSidechain"] != true {
            state.qoder_main_model = model.into();
        }
    }
    let u = &msg["usage"];
    if !u.is_object() {
        return None;
    }
    let stamp = timestamp(&v["timestamp"])?;
    if source.provider != "qoder"
        && let Some(id) = v["sessionId"].as_str()
    {
        state.session_id = id.into();
    }
    let raw_tokens = Tokens {
        input: count(u, "input_tokens"),
        output: count(u, "output_tokens"),
        cache_read: count(u, "cache_read_input_tokens"),
        cache_write: count(u, "cache_creation_input_tokens"),
        reasoning: 0,
    };
    // Qoder's first-party counters use an inclusive prompt count. Custom
    // Anthropic-compatible providers keep their original usage semantics.
    let mut tokens = raw_tokens.clone();
    if source.provider == "qoder" && v["modelSource"].as_str() != Some("custom") {
        tokens.input = tokens
            .input
            .checked_sub(tokens.cache_read)?
            .checked_sub(tokens.cache_write)?;
    }
    let native_credits = if source.provider == "qoder" {
        u["credits"].as_f64().and_then(|credits| {
            let value = NativeCredits {
                credits,
                original_credits: u["original_credits"].as_f64(),
                billable: u["billable"].as_bool(),
                ..Default::default()
            };
            value.valid().then_some(value)
        })
    } else {
        None
    };
    if tokens.total() == 0 && native_credits.is_none() {
        return None;
    }
    let id = msg["id"].as_str().or(v["uuid"].as_str())?;
    let identity = format!(
        "{}:{}:{}",
        state.session_id,
        v["requestId"].as_str().unwrap_or(""),
        id
    );
    let mut parsed = event(
        source,
        state,
        stamp,
        if source.provider == "qoder"
            && v["isSidechain"] == true
            && !state.qoder_main_model.is_empty()
        {
            state.qoder_main_model.clone()
        } else {
            model.into()
        },
        tokens,
        identity,
        "event",
    );
    parsed.native_credits = native_credits;
    // Native response IDs survive history copies; the original timestamp disambiguates
    // clients that reuse short response IDs across independent requests.
    parsed.inheritance = Some(UsageInheritance {
        origin: format!(
            "response:{}:{}:{}",
            v["requestId"].as_str().unwrap_or(""),
            id,
            v["timestamp"]
        ),
        inherited: false,
    });
    if source.provider == "qoder" {
        parsed.raw_tokens = Some(raw_tokens);
    }
    Some(parsed)
}

pub(crate) fn parse_normalized(
    source: &Source,
    state: &mut ParseState,
    v: &Value,
) -> Option<UsageEvent> {
    let tokens: Tokens = serde_json::from_value(v.get("tokens")?.clone()).ok()?;
    let stamp = timestamp(&v["timestamp"])?;
    let id = v["id"].as_str()?;
    let session = v["sessionId"].as_str().unwrap_or("import");
    if source.provider == "qoder" && state.session_id != session {
        state.qoder_main_model.clear();
    }
    state.session_id = session.into();
    let model = v["model"].as_str()?.to_string();
    if source.provider == "qoder" && state.qoder_main_model.is_empty() {
        state.qoder_main_model = model.clone();
    }
    let mut parsed = event(
        source,
        state,
        stamp,
        model,
        tokens,
        format!("{}:{id}", state.session_id),
        "import",
    );
    // This marker belongs only to a Cline legacy task's canonical cumulative
    // row. Version 3 removes an ambiguous child aggregate when independent,
    // explicitly related child records appear after an earlier import.
    if source.provider == "cline"
        && id == format!("task:{session}")
        && let Some(version @ (2 | 3)) = v["clineTaskUsageVersion"].as_u64()
    {
        parsed.usage_version = version as u32;
    }
    parsed.interval_start = timestamp(&v["intervalStart"]);
    parsed.interval_evidence = v["intervalEvidence"].as_str().unwrap_or("").into();
    // Standard exports may explicitly name the original request. Absent that
    // evidence, session relationships or matching counters prove nothing.
    parsed.inheritance = serde_json::from_value::<UsageInheritance>(v["inheritance"].clone())
        .ok()
        .filter(|proof| !proof.origin.trim().is_empty() && proof.origin.len() <= 4096);
    if matches!(
        source.provider.as_str(),
        "qoder" | "codebuddy" | "workbuddy"
    ) {
        parsed.native_credits = serde_json::from_value::<NativeCredits>(v["nativeCredits"].clone())
            .ok()
            .filter(NativeCredits::valid);
        if let Some(c) = &mut parsed.native_credits {
            c.aliases.clear();
            c.mirror_of = None;
            c.mirrored_by = None;
        }
        if source.provider == "workbuddy" && parsed.native_credits.is_some() {
            parsed.usage_version = parser_version(&source.provider);
            parsed.attribution = "workbuddy-credit".into();
        }
    }
    if let Some(category @ ("api" | "subscription")) = v["billing"]["category"].as_str() {
        parsed.billing = BillingEvidence {
            category: category.into(),
            evidence: "normalized-record".into(),
        };
    }
    Some(parsed)
}

pub fn codex_quota(source: &Source, value: &Value, stamp: i64, origin: &str) -> QuotaSnapshot {
    let mut q = QuotaSnapshot {
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: crate::providers::canonical(&source.provider).into(),
        name: source.name.clone(),
        updated_at: stamp,
        origin: origin.into(),
        ..Default::default()
    };
    let maps = value.get("rateLimitsByLimitId").and_then(Value::as_object);
    let fallback = value.get("rateLimits").unwrap_or(value);
    let buckets: Vec<(&str, &Value)> = if let Some(m) = maps.filter(|m| !m.is_empty()) {
        m.iter().map(|(k, v)| (k.as_str(), v)).collect()
    } else {
        vec![("codex", fallback)]
    };
    for (name, bucket) in buckets {
        q.plan = q.plan.or_else(|| {
            bucket["planType"]
                .as_str()
                .or(bucket["plan_type"].as_str())
                .map(str::to_string)
        });
        for key in ["primary", "secondary"] {
            let w = &bucket[key];
            if let Some(used) = w["usedPercent"].as_f64().or(w["used_percent"].as_f64()) {
                let duration = w["windowDurationMins"]
                    .as_i64()
                    .or(w["window_minutes"].as_i64());
                let label = match duration {
                    Some(FIVE_HOUR_MINUTES) => "5h".into(),
                    Some(WEEK_MINUTES) => "7d".into(),
                    Some(n) if n % 60 == 0 => format!("{}h", n / 60),
                    Some(n) => format!("{n}m"),
                    None => key.into(),
                };
                q.windows.push(QuotaWindow {
                    id: format!("{name}:{key}"),
                    group_id: name.into(),
                    group_name: name.into(),
                    name: if name == "codex" {
                        label
                    } else {
                        format!("{name} · {label}")
                    },
                    used_percent: used,
                    window_minutes: duration,
                    resets_at: w["resetsAt"].as_i64().or(w["resets_at"].as_i64()),
                });
            }
        }
    }
    // Credits are shared balances, never a sum of model-specific buckets.
    let primary = value.get("rateLimits").unwrap_or(value);
    let credit = primary
        .get("credits")
        .filter(|v| v.is_object())
        .or_else(|| {
            maps.and_then(|m| m.get("codex"))
                .and_then(|v| v.get("credits"))
                .filter(|v| v.is_object())
        });
    q.credits = credit.and_then(|v| serde_json::from_value(v.clone()).ok());
    q.credits_updated_at = q.credits.as_ref().map(|_| stamp);
    q.credits_origin = q.credits.as_ref().map(|_| origin.into());
    q.bank_reset = value
        .get("rateLimitResetCredits")
        .filter(|v| v.is_object())
        .cloned();
    q.bank_updated_at = q.bank_reset.as_ref().map(|_| stamp);
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source() -> Source {
        Source {
            id: "s1".into(),
            account_id: "a1".into(),
            ..Default::default()
        }
    }
    fn line(input: u64, cache: u64, output: u64) -> Value {
        json!({"type":"event_msg","timestamp":"2026-10-03T01:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":cache,"output_tokens":output},"last_token_usage":{"input_tokens":input,"cached_input_tokens":cache,"output_tokens":output}}}})
    }
    #[test]
    fn cumulative_usage_and_duplicates() {
        let mut s = ParseState {
            session_id: "thread".into(),
            model: "model".into(),
            ..Default::default()
        };
        let a = parse(&source(), &mut s, &line(100, 60, 20)).unwrap();
        assert_eq!(a.tokens.total(), 120);
        assert_eq!(a.tokens.input, 40);
        assert!(parse(&source(), &mut s, &line(100, 60, 20)).is_none());
        let b = parse(&source(), &mut s, &line(250, 160, 50)).unwrap();
        assert_eq!(b.tokens.total(), 180);
        assert_eq!(b.tokens.cache_read, 100);
    }
    #[test]
    fn claude_cache_is_additive() {
        let s = Source {
            provider: "claude".into(),
            ..source()
        };
        let v = json!({"type":"assistant","timestamp":"2026-10-03T01:00:00Z","sessionId":"c","message":{"id":"m","model":"claude-x","usage":{"input_tokens":20,"output_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":30}}});
        let e = parse(&s, &mut ParseState::default(), &v).unwrap();
        assert_eq!(e.tokens.total(), 160);
        assert_eq!(e.tokens.all_input(), 150);
    }
    #[test]
    fn qoder_first_party_cache_is_split_but_custom_usage_is_unchanged() {
        let qoder = Source {
            provider: "qoder".into(),
            ..source()
        };
        let mut v = json!({"type":"assistant","timestamp":"2026-10-03T01:00:00Z","sessionId":"c","message":{"id":"m","model":"dfmodel","usage":{"input_tokens":120,"output_tokens":10,"cache_read_input_tokens":80,"cache_creation_input_tokens":20}}});
        let e = parse(&qoder, &mut ParseState::default(), &v).unwrap();
        assert_eq!(
            e.tokens,
            Tokens {
                input: 20,
                output: 10,
                cache_read: 80,
                cache_write: 20,
                reasoning: 0,
            }
        );
        assert_eq!(e.raw_tokens.unwrap().input, 120);
        v["modelSource"] = json!("custom");
        let e = parse(&qoder, &mut ParseState::default(), &v).unwrap();
        assert_eq!((e.tokens.input, e.tokens.total()), (120, 230));
        assert_eq!(e.raw_tokens.unwrap().input, 120);
    }

    #[test]
    fn bank_count_is_not_detail_count() {
        let q = codex_quota(
            &source(),
            &json!({"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":123}},"rateLimitResetCredits":{"availableCount":5,"credits":[]}}),
            1,
            "live",
        );
        assert_eq!(q.windows[0].name, "5h");
        assert_eq!(q.bank_reset.unwrap()["availableCount"], 5);
    }
}
