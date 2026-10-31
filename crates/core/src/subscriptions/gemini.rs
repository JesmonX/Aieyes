//! Gemini CLI OAuth quotas kept by Pi/OMP. The installed OMP 18.12.2 adapter
//! and google-gemini/gemini-cli's code_assist types define this protocol.
//! Access tokens are read only: no OAuth refresh, onboarding, CLI or model call.
//! Gemini CLI and Antigravity are separate products, even for the same Google ID.
use crate::{
    models::*,
    native_quota_transport::{Request, Response},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const USER: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const LOAD: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const QUOTA: &str = "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const REJECTED: &str = "上游登录已失效，等待原客户端刷新";
const FORMAT: &str = "额度响应格式无效";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Profile {
    /// Google's opaque userinfo ID, often 21 digits: never parse it as u64.
    pub subject: String,
    pub project: String,
    pub email: String,
    pub plan: Option<String>,
    pub meters: Vec<QuotaMeter>,
    pub metadata_error: Option<String>,
}
impl Profile {
    /// Same subject/project across Pi and OMP is one Gemini CLI quota target.
    /// Never collides with Antigravity's `google:<subject hash>` identity.
    pub(crate) fn upstream_identity(&self) -> String {
        format!("google-gemini-cli:{}", hash(&self.subject_key()))
    }
    pub(crate) fn subject_key(&self) -> String {
        json!([self.subject, self.project]).to_string()
    }
}

fn public_string(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
fn project(value: &Value) -> Option<String> {
    public_string(value, 256).filter(|s| !s.chars().any(char::is_whitespace))
}
fn credential(auth: &Value) -> Result<(&str, Option<String>)> {
    let token = auth["access"].as_str().context("登录凭据格式无效")?;
    ensure!(
        !token.is_empty()
            && token.len() <= 32768
            && !token.chars().any(|c| c.is_whitespace() || c.is_control()),
        "登录凭据格式无效"
    );
    if !auth["expires"].is_null() {
        let expires = auth["expires"].as_i64().context("登录凭据格式无效")?;
        let expires = if expires > 10_000_000_000 {
            expires / 1000
        } else {
            expires
        };
        ensure!(expires > now(), REJECTED);
    }
    let selected = project(&auth["projectId"]);
    ensure!(
        auth["projectId"].is_null() || selected.is_some(),
        "登录凭据格式无效"
    );
    Ok((token, selected))
}
fn request(url: &str, token: &str, body: Option<Value>) -> Request {
    let mut request = Request::new(if body.is_some() { "POST" } else { "GET" }, url)
        .timeouts(5, 15)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("User-Agent", concat!("Aieyes/", env!("CARGO_PKG_VERSION")));
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/json")
            .body(body.to_string());
    }
    request
}
fn send_json(
    request: Request,
    send: &mut impl FnMut(Request) -> Result<Response>,
) -> Result<Value> {
    // Both owning-host transports already bound response size and reject redirects.
    // Never echo arbitrary transport/server text that could contain a credential.
    let token = request.headers["Authorization"]
        .strip_prefix("Bearer ")
        .unwrap_or_default()
        .to_owned();
    let response = send(request).map_err(|_| anyhow::anyhow!("额度查询连接失败"))?;
    ensure!(!matches!(response.status, 401 | 403), REJECTED);
    ensure!(
        (200..300).contains(&response.status),
        "额度查询失败（HTTP {}）",
        response.status
    );
    ensure!(response.data.is_object(), FORMAT);
    ensure!(
        !contains_token(&response.data, &token),
        "额度响应包含私有字段，已丢弃"
    );
    Ok(response.data)
}

fn contains_token(value: &Value, token: &str) -> bool {
    match value {
        Value::String(value) => !token.is_empty() && value.contains(token),
        Value::Array(values) => values.iter().any(|value| contains_token(value, token)),
        Value::Object(values) => values.iter().any(|(key, value)| {
            !token.is_empty() && key.contains(token) || contains_token(value, token)
        }),
        _ => false,
    }
}

/// `creditAmount` is an int64 count of credits, not a currency or a request
/// limit. Keep the reported remainder without inventing its total or usage.
fn credits(value: &Value) -> (Vec<QuotaMeter>, bool) {
    let Some(credits) = [&value["paidTier"], &value["currentTier"]]
        .into_iter()
        .map(|tier| &tier["availableCredits"])
        .find(|credits| !credits.is_null())
    else {
        return (vec![], false);
    };
    let Some(credits) = credits.as_array().filter(|credits| credits.len() <= 64) else {
        return (vec![], true);
    };
    let mut invalid = false;
    let mut meters = BTreeMap::new();
    for credit in credits {
        let kind = public_string(&credit["creditType"], 128).filter(|kind| {
            kind.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        });
        let amount = credit["creditAmount"]
            .as_i64()
            .or_else(|| credit["creditAmount"].as_str()?.parse::<i64>().ok())
            .filter(|amount| *amount >= 0);
        let (Some(kind), Some(amount)) = (kind, amount) else {
            invalid = true;
            continue;
        };
        let meter = QuotaMeter {
            id: format!("gemini-credits:{}", kind.to_ascii_lowercase()),
            group: "Gemini".into(),
            label: if kind == "GOOGLE_ONE_AI" {
                "AI Credits".into()
            } else {
                kind
            },
            used_known: Some(false),
            remaining: Some(amount as f64),
            unit: "credits".into(),
            ..Default::default()
        };
        // A repeated pool is not another balance. Retain the first and flag
        // conflicting copies instead of summing a duplicated response.
        if let Some(previous) = meters.get(&meter.id) {
            invalid |= previous != &meter;
        } else {
            meters.insert(meter.id.clone(), meter);
        }
    }
    (meters.into_values().collect(), invalid)
}

/// Resolve server subject and project using the already-granted token. A known
/// native project permits a quota read if optional tier metadata is unavailable.
/// A conflicting server project never silently changes the selected account.
pub(crate) fn probe_using(
    auth: &Value,
    mut send: impl FnMut(Request) -> Result<Response>,
) -> Result<Profile> {
    let (token, selected) = credential(auth)?;
    let user = send_json(request(USER, token, None), &mut send)?;
    let subject = public_string(&user["id"], 256)
        .or_else(|| {
            user["id"]
                .as_u64()
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
        })
        .filter(|v| !v.chars().any(char::is_whitespace))
        .context("上游账户身份格式不支持")?;
    let email = public_string(&user["email"], 256).unwrap_or_default();
    let mut body = json!({"metadata":{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}});
    if let Some(selected) = &selected {
        body["cloudaicompanionProject"] = json!(selected);
    }
    let load = send_json(request(LOAD, token, Some(body)), &mut send);
    let (value, mut metadata_error) = match load {
        Ok(value) => (value, None),
        Err(error) if selected.is_some() && error.to_string() != REJECTED => {
            (Value::Null, Some(error.to_string()))
        }
        Err(error) => return Err(error),
    };
    let reported = project(&value["cloudaicompanionProject"])
        .or_else(|| project(&value["cloudaicompanionProject"]["id"]));
    ensure!(
        selected
            .as_ref()
            .zip(reported.as_ref())
            .is_none_or(|(a, b)| a == b),
        "原生登录账户已变化，正在重新识别"
    );
    let project = selected.or(reported).context("原生账户身份待确认")?;
    let plan = [&value["paidTier"], &value["currentTier"]]
        .into_iter()
        .find_map(|tier| {
            public_string(&tier["name"], 256).or_else(|| public_string(&tier["id"], 256))
        });
    let (meters, invalid) = credits(&value);
    if invalid {
        metadata_error = Some(FORMAT.into());
    }
    Ok(Profile {
        subject,
        project,
        email,
        plan,
        meters,
        metadata_error,
    })
}

/// Local and SSH use exactly the same parser/request chain. `expected` is the
/// previously discovered product-scoped upstream identity, not a client label.
pub(crate) fn read_using(
    source: &Source,
    auth: &Value,
    expected: Option<&str>,
    mut send: impl FnMut(Request) -> Result<Response>,
) -> Result<QuotaSnapshot> {
    let profile = probe_using(auth, &mut send)?;
    let identity = profile.upstream_identity();
    ensure!(
        expected.is_none_or(|value| value == identity),
        "查询期间原生账户身份已变化"
    );
    let (token, _) = credential(auth)?;
    let quota = send_json(
        request(QUOTA, token, Some(json!({"project":profile.project}))),
        &mut send,
    )
    .and_then(|value| parse(source, &value));
    let mut q = match quota {
        Ok(quota) => quota,
        Err(error) if !profile.meters.is_empty() && error.to_string() != REJECTED => {
            let mut quota = snapshot(source);
            quota.metadata_error = Some(error.to_string());
            quota
        }
        Err(error) => return Err(error),
    };
    q.meters = profile.meters;
    q.plan = profile.plan.clone();
    q.metadata_error = q.metadata_error.or(profile.metadata_error);
    q.identity = Some(AccountIdentity {
        key: identity,
        email: profile.email,
        subscription: profile.plan,
        checked_at: q.updated_at,
        subscription_checked_at: Some(q.updated_at),
        ..Default::default()
    });
    Ok(q)
}

fn snapshot(source: &Source) -> QuotaSnapshot {
    let at = now();
    QuotaSnapshot {
        provider: source.provider.clone(),
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        name: source.name.clone(),
        updated_at: at,
        snapshot_at: Some(at),
        origin: "live".into(),
        ..Default::default()
    }
}

fn parse(source: &Source, value: &Value) -> Result<QuotaSnapshot> {
    let buckets = value["buckets"].as_array().context(FORMAT)?;
    ensure!(buckets.len() <= 512, FORMAT);
    let mut q = snapshot(source);
    let mut occurrences = BTreeMap::<(String, String), usize>::new();
    let mut invalid = false;
    for bucket in buckets {
        let Some(remaining) = bucket["remainingFraction"]
            .as_f64()
            .filter(|n| n.is_finite())
        else {
            // remainingAmount has no universal unit/denominator. Do not turn a
            // missing fraction into 0% usage, invent a token price or a request cap.
            invalid = true;
            continue;
        };
        let model = public_string(&bucket["modelId"], 256).unwrap_or_else(|| "Gemini".into());
        let token_type = public_string(&bucket["tokenType"], 128).unwrap_or_else(|| "Quota".into());
        let reset = bucket["resetTime"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp());
        if !bucket["resetTime"].is_null() && reset.is_none() {
            invalid = true;
        }
        let count = occurrences
            .entry((model.clone(), token_type.clone()))
            .or_default();
        let id = format!(
            "gemini:{}",
            hash(&json!([model, token_type, count]).to_string())
        );
        *count += 1;
        q.windows.push(QuotaWindow {
            id,
            group_id: format!("gemini:{}", hash(&model)),
            group_name: model,
            name: token_type,
            used_percent: 100.0 * (1.0 - remaining.clamp(0.0, 1.0)),
            window_minutes: None,
            resets_at: reset,
        });
    }
    ensure!(!q.windows.is_empty(), FORMAT);
    if invalid {
        q.metadata_error = Some(FORMAT.into());
    }
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn auth() -> Value {
        json!({"access":"PRIVATE-ACCESS","refresh":"PRIVATE-REFRESH","expires":4102444800000_i64,"projectId":"project-one"})
    }
    fn source() -> Source {
        Source {
            provider: "pi".into(),
            id: "native-pi".into(),
            account_id: "account".into(),
            ..Default::default()
        }
    }
    fn fixture() -> Value {
        // Synthetic responses shaped by the official Code Assist types and
        // OMP 18.12.2's installed adapter; no authenticated account was used.
        serde_json::from_str(include_str!("../../tests/fixtures/gemini-cli-usage.json")).unwrap()
    }
    fn user() -> Value {
        fixture()["userinfo"].clone()
    }
    fn load() -> Value {
        fixture()["freeAssist"].clone()
    }
    fn quota() -> Value {
        fixture()["quota"].clone()
    }
    fn run(
        auth: &Value,
        expected: Option<&str>,
        replies: Vec<(u16, Value)>,
    ) -> (Result<QuotaSnapshot>, Vec<Request>) {
        let mut replies: VecDeque<_> = replies.into();
        let mut requests = vec![];
        let result = read_using(&source(), auth, expected, |request| {
            requests.push(request);
            let (status, data) = replies.pop_front().expect("unexpected request");
            Ok(Response { status, data })
        });
        (result, requests)
    }
    #[test]
    fn official_chain_preserves_large_subject_project_and_each_model_token_bucket() {
        let (result, requests) = run(
            &auth(),
            None,
            vec![(200, user()), (200, load()), (200, quota())],
        );
        let q = result.unwrap();
        assert_eq!(
            requests.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
            [USER, LOAD, QUOTA]
        );
        assert_eq!(
            requests
                .iter()
                .map(|r| r.method.as_str())
                .collect::<Vec<_>>(),
            ["GET", "POST", "POST"]
        );
        let body: Value = serde_json::from_str(requests[1].body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"cloudaicompanionProject":"project-one","metadata":{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}})
        );
        assert_eq!(
            requests[2].body.as_deref(),
            Some("{\"project\":\"project-one\"}")
        );
        for r in &requests {
            assert_eq!(
                r.headers.get("Authorization").map(String::as_str),
                Some("Bearer PRIVATE-ACCESS")
            );
            assert!(
                !serde_json::to_string(r)
                    .unwrap()
                    .contains("PRIVATE-REFRESH")
            );
            assert!(
                !r.body
                    .as_ref()
                    .is_some_and(|v| v.contains("PRIVATE-ACCESS"))
            );
        }
        assert_eq!(q.windows.len(), 3);
        assert_eq!(q.windows[0].used_percent, 75.0);
        assert_eq!(q.windows[0].resets_at, Some(1893553445));
        assert_ne!(q.windows[0].id, q.windows[1].id);
        assert_eq!(q.windows[0].group_id, q.windows[1].group_id);
        assert_ne!(q.windows[0].group_id, q.windows[2].group_id);
        assert!(q.windows.iter().all(|w| w.window_minutes.is_none()));
        assert!(q.meters.is_empty());
        assert_eq!(q.plan.as_deref(), Some("Free"));
        assert_eq!(q.provider, "pi");
        assert_eq!(q.snapshot_at, Some(q.updated_at));
        assert!(!serde_json::to_string(&q).unwrap().contains("PRIVATE"));
    }
    #[test]
    fn product_identity_uses_server_id_and_project_not_email_or_client() {
        let fetch = |email: &str, project: &str| {
            probe_using(&json!({"access":"token"}), |r| {
                Ok(Response {
                    status: 200,
                    data: if r.url == USER {
                        json!({"id":"123456789012345678901","email":email})
                    } else {
                        json!({"cloudaicompanionProject":project})
                    },
                })
            })
            .unwrap()
        };
        let a = fetch("before@example.test", "p");
        let b = fetch("after@example.test", "p");
        let c = fetch("before@example.test", "q");
        assert_eq!(a.subject, "123456789012345678901");
        assert_eq!(a.upstream_identity(), b.upstream_identity());
        assert_ne!(a.upstream_identity(), c.upstream_identity());
        assert_ne!(
            a.upstream_identity(),
            format!("google:{}", hash(&a.subject))
        );
        let mut no_id = user();
        no_id.as_object_mut().unwrap().remove("id");
        assert!(run(&auth(), None, vec![(200, no_id)]).0.is_err());
    }
    #[test]
    fn identity_or_selected_project_changes_stop_before_quota() {
        let (result, requests) = run(
            &auth(),
            Some("another-account"),
            vec![(200, user()), (200, load())],
        );
        assert!(result.is_err());
        assert_eq!(requests.len(), 2);
        let (result, requests) = run(
            &auth(),
            None,
            vec![
                (200, user()),
                (200, json!({"cloudaicompanionProject":"another-project"})),
            ],
        );
        assert!(result.is_err());
        assert_eq!(requests.len(), 2);
    }
    #[test]
    fn expired_or_malformed_credentials_do_not_request_or_refresh() {
        for expires in [json!(1), json!(1000), json!("bad")] {
            let mut auth = auth();
            auth["expires"] = expires;
            let (result, requests) = run(&auth, None, vec![]);
            assert!(result.is_err());
            assert!(requests.is_empty());
        }
        for key in ["access", "projectId"] {
            let mut auth = auth();
            auth[key] = json!("bad\nvalue");
            assert!(run(&auth, None, vec![]).0.is_err());
        }
    }
    #[test]
    fn rejected_and_rate_limited_calls_never_retry_or_echo_server_data() {
        for status in [401, 403, 429, 500] {
            let (result, requests) = run(
                &auth(),
                None,
                vec![
                    (200, user()),
                    (200, load()),
                    (status, json!({"error":"PRIVATE-REFRESH"})),
                ],
            );
            let message = result.unwrap_err().to_string();
            assert!(!message.contains("PRIVATE"));
            assert_eq!(requests.len(), 3);
            if matches!(status, 401 | 403) {
                assert_eq!(message, REJECTED)
            } else {
                assert!(message.contains(&status.to_string()))
            }
        }
        let (result, requests) = run(&auth(), None, vec![(200, user()), (401, json!({}))]);
        assert_eq!(result.unwrap_err().to_string(), REJECTED);
        assert_eq!(requests.len(), 2);
    }
    #[test]
    fn known_project_keeps_valid_quota_when_tier_is_unavailable_but_never_onboards() {
        let (result, requests) = run(
            &auth(),
            None,
            vec![(200, user()), (503, json!({})), (200, quota())],
        );
        let q = result.unwrap();
        assert_eq!(q.windows.len(), 3);
        assert!(q.plan.is_none());
        assert!(q.metadata_error.is_some());
        assert_eq!(requests.len(), 3);
        let mut auth = auth();
        auth.as_object_mut().unwrap().remove("projectId");
        assert!(
            run(
                &auth,
                None,
                vec![
                    (200, user()),
                    (200, json!({"allowedTiers":[{"id":"free-tier"}]}))
                ]
            )
            .0
            .is_err()
        );
    }
    #[test]
    fn malformed_buckets_keep_valid_siblings_without_fabricating_empty_usage() {
        let mut value = quota();
        value["buckets"].as_array_mut().unwrap().extend([
            json!({"modelId":"unsupported","remainingAmount":"100"}),
            json!({"remainingFraction":"bad"}),
        ]);
        let q = parse(&source(), &value).unwrap();
        assert_eq!(q.windows.len(), 3);
        assert!(q.metadata_error.is_some());
        for value in [
            json!({}),
            json!({"buckets":[]}),
            json!({"buckets":[{"remainingAmount":"100"}]}),
        ] {
            assert!(parse(&source(), &value).is_err());
        }
        let a = parse(&source(), &quota()).unwrap();
        let mut changed = quota();
        changed["buckets"][0]["resetTime"] = json!("2030-01-03T03:04:05Z");
        assert_eq!(
            a.windows[0].id,
            parse(&source(), &changed).unwrap().windows[0].id
        );
    }

    #[test]
    fn paid_tier_credits_are_reported_without_inventing_usage_total_or_money() {
        let (result, _) = run(
            &auth(),
            None,
            vec![
                (200, user()),
                (200, fixture()["paidAssist"].clone()),
                (200, quota()),
            ],
        );
        let q = result.unwrap();
        assert_eq!(q.plan.as_deref(), Some("Google AI Pro"));
        assert_eq!(q.windows.len(), 3);
        assert_eq!(q.meters.len(), 1);
        let credit = &q.meters[0];
        assert_eq!(credit.id, "gemini-credits:google_one_ai");
        assert_eq!(credit.remaining, Some(1234.0));
        assert_eq!(credit.used_known, Some(false));
        assert_eq!(serde_json::to_value(credit).unwrap()["usedKnown"], false);
        assert_eq!(credit.total, None);
        assert_eq!(credit.overage, None);
        assert_eq!(credit.resets_at, None);
        assert_eq!(credit.unit, "credits");
        assert!(!credit.estimated);
        assert!(q.metadata_error.is_none());

        let mut value = load();
        value["paidTier"] = json!({});
        let (result, _) = run(
            &auth(),
            None,
            vec![(200, user()), (200, value), (200, quota())],
        );
        assert_eq!(result.unwrap().plan.as_deref(), Some("Free"));
    }

    #[test]
    fn malformed_credit_siblings_and_duplicates_never_inflate_a_measured_pool() {
        let mut value = fixture()["paidAssist"].clone();
        value["paidTier"]["availableCredits"]
            .as_array_mut()
            .unwrap()
            .extend([
                json!({"creditType":"GOOGLE_ONE_AI","creditAmount":"1234"}),
                json!({"creditType":"GOOGLE_ONE_AI","creditAmount":"4000"}),
                json!({"creditType":"INVALID_AMOUNT","creditAmount":"NaN"}),
                json!({"creditType":"NEGATIVE","creditAmount":"-1"}),
                json!({"creditType":"NOT_INTEGER","creditAmount":"0.5"}),
                json!({"creditType":"ZERO","creditAmount":"0"}),
            ]);
        let (result, _) = run(
            &auth(),
            None,
            vec![(200, user()), (200, value), (200, quota())],
        );
        let q = result.unwrap();
        assert_eq!(q.meters.len(), 2);
        assert_eq!(q.meters[0].remaining, Some(1234.0));
        assert_eq!(q.meters[1].remaining, Some(0.0));
        assert_eq!(q.metadata_error.as_deref(), Some(FORMAT));
        assert_eq!(q.windows.len(), 3);
    }

    #[test]
    fn measured_credit_only_snapshot_keeps_a_failed_quota_read_explicit() {
        for (status, value) in [(200, json!({"buckets":[]})), (503, json!({}))] {
            let (result, requests) = run(
                &auth(),
                None,
                vec![
                    (200, user()),
                    (200, fixture()["paidAssist"].clone()),
                    (status, value),
                ],
            );
            let q = result.unwrap();
            assert!(q.windows.is_empty());
            assert_eq!(q.meters[0].remaining, Some(1234.0));
            assert!(q.metadata_error.is_some());
            assert_eq!(requests.len(), 3);
        }
        for status in [401, 403] {
            let (result, _) = run(
                &auth(),
                None,
                vec![
                    (200, user()),
                    (200, fixture()["paidAssist"].clone()),
                    (status, json!({})),
                ],
            );
            assert_eq!(result.unwrap_err().to_string(), REJECTED);
        }
    }

    #[test]
    fn successful_responses_cannot_echo_the_access_token_into_public_fields() {
        for mut value in [user(), load(), quota()] {
            value["echo"] = json!({"nested":["prefix PRIVATE-ACCESS suffix"]});
            let result = send_json(request(USER, "PRIVATE-ACCESS", None), &mut |_| {
                Ok(Response {
                    status: 200,
                    data: value.clone(),
                })
            });
            let error = result.unwrap_err().to_string();
            assert_eq!(error, "额度响应包含私有字段，已丢弃");
            assert!(!error.contains("PRIVATE"));
        }
        assert!(contains_token(
            &json!({"PRIVATE-ACCESS":"safe"}),
            "PRIVATE-ACCESS"
        ));
    }

    #[test]
    fn int64_credit_boundary_remains_finite_and_remaining_only() {
        let (meters, invalid) = credits(
            &json!({"paidTier":{"availableCredits":[{"creditType":"GOOGLE_ONE_AI","creditAmount":i64::MAX.to_string()}]}}),
        );
        assert!(!invalid);
        assert_eq!(meters.len(), 1);
        assert!(meters[0].remaining.unwrap().is_finite());
        assert_eq!(meters[0].used_known, Some(false));
        assert_eq!(meters[0].total, None);
    }
}
