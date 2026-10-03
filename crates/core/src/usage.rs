use crate::models::*;
use serde_json::Value;

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
        id: hash(&format!(
            "{}:{}:{identity}",
            source.provider,
            if source.account_id.is_empty() {
                format!("source:{}", source.id)
            } else {
                source.account_id.clone()
            }
        )),
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: source.provider.clone(),
        session_id: state.session_id.clone(),
        timestamp: stamp,
        model,
        tokens,
        attribution: attribution.into(),
    }
}

pub fn parse(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    match source.provider.as_str() {
        "codex" => parse_codex(source, state, v),
        "claude" => parse_claude(source, state, v),
        _ => parse_normalized(source, state, v),
    }
}

fn parse_codex(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    let payload = &v["payload"];
    match v["type"].as_str()? {
        "session_meta" => {
            if let Some(id) = payload["id"].as_str().or(payload["session_id"].as_str()) {
                state.session_id = id.into();
            }
            None
        }
        "turn_context" => {
            if let Some(model) = payload["model"].as_str() {
                state.model = model.into();
            }
            None
        }
        "event_msg" if payload["type"] == "token_count" => {
            let stamp = timestamp(&v["timestamp"])?;
            let cumulative = codex_tokens(&payload["info"]["total_token_usage"])?;
            if state.last_cumulative.as_ref() == Some(&cumulative) {
                return None;
            }
            let (tokens, attribution) = match &state.last_cumulative {
                None => (cumulative.clone(), "session-summary"),
                Some(prev) => {
                    if let Some(delta) = cumulative.checked_delta(prev) {
                        (delta, "event")
                    } else {
                        state.cumulative_segment += 1;
                        (
                            codex_tokens(&payload["info"]["last_token_usage"])?,
                            "counter-reset",
                        )
                    }
                }
            };
            state.last_cumulative = Some(cumulative.clone());
            state.last_timestamp = stamp;
            if tokens.total() == 0 {
                return None;
            }
            let identity = format!(
                "{}:{}:{}:{}",
                state.session_id,
                state.cumulative_segment,
                stamp,
                serde_json::to_string(&cumulative).ok()?
            );
            Some(event(
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
            ))
        }
        _ => None,
    }
}

fn parse_claude(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    if v["type"] != "assistant" || v["isApiErrorMessage"] == true {
        return None;
    }
    let msg = &v["message"];
    let u = &msg["usage"];
    if !u.is_object() {
        return None;
    }
    let model = msg["model"].as_str()?;
    if model.starts_with('<') {
        return None;
    }
    let stamp = timestamp(&v["timestamp"])?;
    if let Some(id) = v["sessionId"].as_str() {
        state.session_id = id.into();
    }
    let tokens = Tokens {
        input: count(u, "input_tokens"),
        output: count(u, "output_tokens"),
        cache_read: count(u, "cache_read_input_tokens"),
        cache_write: count(u, "cache_creation_input_tokens"),
        reasoning: 0,
    };
    if tokens.total() == 0 {
        return None;
    }
    let id = msg["id"].as_str().or(v["uuid"].as_str())?;
    let identity = format!(
        "{}:{}:{}",
        state.session_id,
        v["requestId"].as_str().unwrap_or(""),
        id
    );
    Some(event(
        source,
        state,
        stamp,
        model.into(),
        tokens,
        identity,
        "event",
    ))
}

fn parse_normalized(source: &Source, state: &mut ParseState, v: &Value) -> Option<UsageEvent> {
    let tokens: Tokens = serde_json::from_value(v.get("tokens")?.clone()).ok()?;
    let stamp = timestamp(&v["timestamp"])?;
    let id = v["id"].as_str()?;
    state.session_id = v["sessionId"].as_str().unwrap_or("import").into();
    let model = v["model"].as_str()?.to_string();
    Some(event(
        source,
        state,
        stamp,
        model,
        tokens,
        format!("{}:{id}", state.session_id),
        "import",
    ))
}

pub fn codex_quota(source: &Source, value: &Value, stamp: i64, origin: &str) -> QuotaSnapshot {
    let mut q = QuotaSnapshot {
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: source.provider.clone(),
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
                    Some(300) => "5h".into(),
                    Some(10080) => "7d".into(),
                    Some(n) if n % 60 == 0 => format!("{}h", n / 60),
                    Some(n) => format!("{n}m"),
                    None => key.into(),
                };
                q.windows.push(QuotaWindow {
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
