use aieyes_core::{
    Engine,
    models::*,
    subscriptions::{self, SubscriptionConfig},
};
use serde_json::{Value, json};

fn parse(provider: &str, mode: &str, data: Value) -> anyhow::Result<QuotaSnapshot> {
    subscriptions::parse(
        &Source {
            provider: provider.into(),
            id: "source".into(),
            account_id: "account".into(),
            ..Default::default()
        },
        &SubscriptionConfig {
            mode: mode.into(),
            workspace_id: "team_123".into(),
            ..Default::default()
        },
        &data,
    )
}

#[test]
fn zai_credit_windows_preserve_units_resets_and_observed_zero() {
    let q = parse("zcode", "global", json!({"success":true,"data":{"planName":"Pro","limits":[
        {"type":"CREDIT_LIMIT","unit":3,"number":5,"percentage":0,"usage":12000,"currentValue":3000,"nextResetTime":(now()+600)*1000},
        {"type":"CREDIT_LIMIT","unit":6,"number":1,"percentage":0},
        {"type":"FUTURE_LIMIT"}
    ]}})).unwrap();
    assert_eq!(q.plan.as_deref(), Some("Pro"));
    assert_eq!(q.windows.len(), 2);
    assert_eq!(q.windows[0].used_percent, 25.);
    assert_eq!(q.windows[0].window_minutes, Some(300));
    assert_eq!(q.windows[1].window_minutes, Some(10080));
    assert_eq!(q.windows[1].used_percent, 0.);
    assert!(q.windows[0].resets_at.unwrap() < now() + 1000);
    assert!(
        parse(
            "zcode",
            "global",
            json!({"success":false,"data":{"limits":[]}})
        )
        .is_err()
    );
}

#[test]
fn qoder_sums_shared_credits_and_zero_quota_has_no_headroom() {
    let q = parse("qoder", "global", json!({"totalQuota":{"quotaSummary":{"usedValue":250,"limitValue":1000}},"sharedQuota":{"quotaSummary":{"usedValue":500,"limitValue":2000}},"nextResetAt":"2026-11-01T00:00:00Z"})).unwrap();
    assert_eq!(q.windows[0].used_percent, 25.);
    assert!(q.windows[0].resets_at.is_some());
    let q = parse(
        "qoder",
        "global",
        json!({"totalQuota":{"quotaSummary":{"usedValue":0,"limitValue":0}}}),
    )
    .unwrap();
    assert_eq!(q.windows[0].used_percent, 100.);
}

#[test]
fn cursor_keeps_usage_pools_separate_and_does_not_scale_fractional_percent() {
    let q = parse("cursor", "session", json!({"membershipType":"ultra","individualUsage":{"plan":{"autoPercentUsed":0.36,"apiPercentUsed":50.5}}})).unwrap();
    assert_eq!(q.windows.len(), 2);
    assert_eq!(q.windows[0].used_percent, 0.36);
    assert_eq!(q.windows[1].used_percent, 50.5);
    assert!(q.windows[0].resets_at.is_none());
    let q = parse(
        "cursor",
        "session",
        json!({"individualUsage":{"overall":{"used":10,"limit":100}}}),
    )
    .unwrap();
    assert_eq!(q.windows[0].id, "personal-cap");
    assert_eq!(q.windows[0].used_percent, 10.);
}

#[test]
fn grok_subscription_and_api_balance_are_distinct() {
    let q = parse("grok", "subscription", json!({"config":{"subscriptionTier":"premium","creditUsagePercent":0,"currentPeriod":{"start":"2026-10-01T00:00:00Z","end":"2026-11-01T00:00:00Z"}}})).unwrap();
    assert_eq!(q.windows[0].used_percent, 0.);
    assert_eq!(q.windows[0].window_minutes, Some(44640));
    assert!(q.balances.is_empty());
    let q = parse("grok", "api", json!({"total":{"val":"-125"}})).unwrap();
    assert_eq!(q.balances[0].total, "-1.25");
    assert!(q.windows.is_empty());
    assert!(
        parse(
            "grok",
            "subscription",
            json!({"config":{"currentPeriod":{"end":"2026-11-01T00:00:00Z"}}})
        )
        .is_err()
    );
}

#[test]
fn opencode_go_has_three_independent_windows_console_uses_microcents() {
    let q = parse("opencode", "go", json!({"usage":{"rolling":{"percent":0.5,"resetInSec":60},"weekly":{"percent":99},"monthly":{"percent":0}}})).unwrap();
    assert_eq!(q.windows.len(), 3);
    assert_eq!(q.windows[0].used_percent, 0.5);
    assert_eq!(q.windows[2].window_minutes, None);
    let q = parse("opencode", "console", json!({"access":{"endsAt":"2026-11-01T00:00:00Z","meters":{"fiveHour":{"usedMicroCents":"250000","limitMicroCents":"1000000"},"month":{"usedMicroCents":0,"limitMicroCents":100}}}})).unwrap();
    assert_eq!(q.windows[0].used_percent, 25.);
    assert!(q.windows[0].resets_at.is_none());
    assert!(q.windows[1].resets_at.is_some());
    assert!(
        parse("opencode", "console", json!({"access":null}))
            .unwrap()
            .windows
            .is_empty()
    );
}

#[test]
fn missing_invalid_and_unknown_shapes_never_mean_zero_usage() {
    for (p, mode) in [
        ("zcode", "global"),
        ("qoder", "global"),
        ("cursor", "session"),
        ("grok", "subscription"),
        ("grok", "api"),
        ("opencode", "go"),
        ("opencode", "console"),
    ] {
        assert!(parse(p, mode, json!({})).is_err(), "{p}");
        assert!(
            parse(p, mode, json!({"error":"private-server-message"})).is_err(),
            "{p}"
        );
    }
    for bad in [json!(-1), json!("NaN"), json!(null), json!(true)] {
        assert!(
            parse(
                "opencode",
                "go",
                json!({"usage":{"rolling":{"percent":bad}}})
            )
            .is_err()
        );
    }
}

#[test]
fn regions_and_scope_ids_are_explicit_and_failed_setup_does_not_create_accounts() {
    let global = SubscriptionConfig {
        mode: "global".into(),
        ..Default::default()
    };
    let china = SubscriptionConfig {
        mode: "cn".into(),
        ..Default::default()
    };
    assert!(
        subscriptions::endpoint("zcode", &global)
            .unwrap()
            .starts_with("https://api.z.ai/")
    );
    assert!(
        subscriptions::endpoint("zcode", &china)
            .unwrap()
            .starts_with("https://open.bigmodel.cn/")
    );
    for id in [
        "",
        "../elsewhere",
        "team\r\nAuthorization: other",
        "https://example.com",
    ] {
        assert!(
            subscriptions::endpoint(
                "grok",
                &SubscriptionConfig {
                    mode: "api".into(),
                    workspace_id: id.into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(dir.path()).unwrap();
    engine
        .call(
            "agents.set",
            json!({"provider":"zcode","enabled":true,"machineIds":[]}),
        )
        .unwrap();
    let before = engine.call("settings.get", json!({})).unwrap();
    assert!(engine.call("subscriptions.connect",json!({"provider":"zcode","secret":"fixture-secret","subscription":{"mode":"invalid"}})).is_err());
    assert_eq!(before, engine.call("settings.get", json!({})).unwrap());
    assert!(!before.to_string().contains("fixture-secret"));
}

#[test]
fn explicit_native_login_import_is_scoped_and_rejects_expiry_or_ambiguity() {
    assert_eq!(subscriptions::local_secret("opencode", "go", &json!({"opencode-go":{"type":"api","key":"fixture-go"},"openai":{"type":"oauth","access":"must-not-use"}})).unwrap(), "fixture-go");
    assert!(
        subscriptions::local_secret(
            "opencode",
            "go",
            &json!({"openai":{"type":"api","key":"other"}})
        )
        .is_err()
    );
    assert!(subscriptions::local_secret("opencode", "console", &json!({})).is_err());
    let auth = json!({"https://auth.x.ai::client":{"key":"fixture-grok","expires_at":"2099-01-01T00:00:00Z"}});
    assert_eq!(
        subscriptions::local_secret("grok", "subscription", &auth).unwrap(),
        "fixture-grok"
    );
    let mut expired = auth.clone();
    expired["https://auth.x.ai::client"]["expires_at"] = json!("2020-01-01T00:00:00Z");
    assert!(subscriptions::local_secret("grok", "subscription", &expired).is_err());
    let mut ambiguous = auth;
    ambiguous["https://auth.x.ai::other"] = json!({"key":"other"});
    assert!(subscriptions::local_secret("grok", "subscription", &ambiguous).is_err());
    assert_eq!(
        subscriptions::login_url("zcode", "cn").unwrap(),
        "https://bigmodel.cn/coding-plan/personal/usage"
    );
    assert!(subscriptions::login_url("https://evil.invalid", "global").is_err());
}
