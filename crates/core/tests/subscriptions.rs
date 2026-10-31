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

fn kimi_wallet_body() -> Value {
    json!({"usages":{"limit_5h":{"used_ratio":0.25}},"boosterWallet":{
        "balance":{"type":"BOOSTER","amount":"1234000000","amountLeft":"1000000000"},
        "monthlyChargeLimitEnabled":true,"monthlyChargeLimit":{"priceInCents":"1500","currency":"USD"},
        "monthlyUsed":{"priceInCents":123,"currency":"USD"}},"ignored":"PRIVATE_RESPONSE"})
}

#[test]
fn kimi_wallet_uses_exact_money_units_separately_from_subscription_windows() {
    for mode in ["cn", "global"] {
        let q = parse("kimi", mode, kimi_wallet_body()).unwrap();
        let wallet = q.extra_usage.as_ref().unwrap();
        assert_eq!(wallet.currency, "USD");
        assert_eq!(wallet.total, "12.34");
        assert_eq!(wallet.balance.as_deref(), Some("10.00"));
        assert_eq!(wallet.monthly_limit_enabled, Some(true));
        assert_eq!(wallet.monthly_limit.as_deref(), Some("15.00"));
        assert_eq!(wallet.monthly_used.as_deref(), Some("1.23"));
        assert_eq!(q.windows.len(), 1);
        assert_eq!(q.windows[0].used_percent, 25.);
        assert!(q.balances.is_empty() && q.credits.is_none() && q.is_available.is_none());
        assert!(q.metadata_error.is_none());
        assert!(!serde_json::to_string(&q).unwrap().contains("PRIVATE"));
    }
}

#[test]
fn kimi_wallet_preserves_zero_debt_and_subcent_amounts_without_float_rounding() {
    for (raw, expected) in [
        (json!(0), "0.00"),
        (json!(1), "0.00000001"),
        (json!(1.0), "0.00000001"),
        (json!(-1), "-0.00000001"),
        (json!("9007199254740993"), "90071992.54740993"),
    ] {
        let mut body = kimi_wallet_body();
        body["usages"] = json!({});
        body["boosterWallet"]["balance"]["amountLeft"] = raw;
        body["boosterWallet"]["monthlyChargeLimit"]["priceInCents"] = json!(0);
        body["boosterWallet"]["monthlyUsed"]["priceInCents"] = json!(0);
        let q = parse("kimi", "cn", body).unwrap();
        let wallet = q.extra_usage.unwrap();
        assert_eq!(wallet.balance.as_deref(), Some(expected));
        assert_eq!(wallet.monthly_limit.as_deref(), Some("0.00"));
        assert_eq!(wallet.monthly_used.as_deref(), Some("0.00"));
        assert!(q.windows.is_empty() && q.is_available.is_none());
    }
}

#[test]
fn kimi_wallet_missing_or_invalid_metadata_never_becomes_a_zero_balance() {
    let mut body = kimi_wallet_body();
    body["boosterWallet"]["balance"]
        .as_object_mut()
        .unwrap()
        .remove("amountLeft");
    body["boosterWallet"]
        .as_object_mut()
        .unwrap()
        .remove("monthlyChargeLimitEnabled");
    body["boosterWallet"]
        .as_object_mut()
        .unwrap()
        .remove("monthlyUsed");
    let q = parse("kimi", "cn", body).unwrap();
    let wallet = q.extra_usage.unwrap();
    assert!(
        wallet.balance.is_none()
            && wallet.monthly_limit_enabled.is_none()
            && wallet.monthly_used.is_none()
    );
    assert!(q.metadata_error.is_some());
    assert_eq!(q.windows.len(), 1);
    for invalid in [
        json!({}),
        json!({"balance":{"type":"OTHER","amount":100}}),
        json!({"balance":{"type":"BOOSTER","amount":"NaN"}}),
        json!({"balance":{"type":"BOOSTER","amount":-1}}),
    ] {
        let mut body = kimi_wallet_body();
        body["boosterWallet"] = invalid;
        let q = parse("kimi", "cn", body).unwrap();
        assert!(q.extra_usage.is_none());
        assert!(q.metadata_error.is_some());
        assert_eq!(q.windows.len(), 1);
    }
    for invalid in [
        json!("NaN"),
        json!("1e8"),
        json!(1.5),
        json!(9_007_199_254_740_992.0),
        json!({"private":"PRIVATE"}),
    ] {
        let mut body = kimi_wallet_body();
        body["boosterWallet"]["balance"]["amountLeft"] = invalid;
        let q = parse("kimi", "cn", body).unwrap();
        assert!(q.extra_usage.unwrap().balance.is_none());
        assert!(q.metadata_error.is_some());
    }
    let mut body = kimi_wallet_body();
    body["boosterWallet"]["monthlyUsed"]["currency"] = json!("CNY");
    let q = parse("kimi", "cn", body).unwrap();
    assert!(q.extra_usage.is_none() && q.metadata_error.is_some());
    assert_eq!(q.windows.len(), 1);
    let mut body = kimi_wallet_body();
    body["usages"] = json!({"limit_5h":{"used_ratio":"invalid"}});
    let q = parse("kimi", "cn", body).unwrap();
    assert!(q.windows.is_empty() && q.extra_usage.is_some() && q.metadata_error.is_some());
}

#[test]
fn kimi_wallet_absence_and_inactive_zero_funding_remain_absent() {
    for wallet in [
        Value::Null,
        json!({"balance":{"type":"BOOSTER","amount":0,"amountLeft":0}}),
    ] {
        let mut body = kimi_wallet_body();
        body["boosterWallet"] = wallet;
        let q = parse("kimi", "cn", body).unwrap();
        assert!(q.extra_usage.is_none() && q.metadata_error.is_none());
    }
    for currency in ["CNY", "EUR", "JPY", "usd"] {
        let mut body = kimi_wallet_body();
        body["boosterWallet"]["monthlyChargeLimitEnabled"] = json!(false);
        body["boosterWallet"]["monthlyChargeLimit"]["currency"] = json!(currency);
        body["boosterWallet"]["monthlyUsed"]["currency"] = json!(currency);
        let q = parse("kimi", "cn", body).unwrap();
        assert_eq!(
            q.extra_usage.as_ref().unwrap().monthly_limit_enabled,
            Some(false)
        );
        assert_eq!(
            q.extra_usage.unwrap().currency,
            currency.to_ascii_uppercase()
        );
    }
}

#[test]
fn kimi_wallet_only_snapshots_survive_cache_dashboard_and_failed_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let store = aieyes_core::store::Store::open(temp.path()).unwrap();
    let settings = Settings {
        accounts: vec![Account {
            id: "account".into(),
            provider: "kimi".into(),
            name: "Kimi wallet".into(),
            quota_enabled: true,
            ..Default::default()
        }],
        sources: vec![Source {
            id: "source".into(),
            account_id: "account".into(),
            provider: "kimi".into(),
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    let mut body = kimi_wallet_body();
    body["usages"] = json!({});
    let q = parse("kimi", "cn", body).unwrap();
    store.quota(&q).unwrap();
    let first = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(first.quotas.len(), 1);
    assert_eq!(first.quotas[0].extra_usage, q.extra_usage);
    store
        .quota(&QuotaSnapshot {
            provider: "kimi".into(),
            account_id: "account".into(),
            source_id: "source".into(),
            error: Some("query failed".into()),
            updated_at: q.updated_at + 1,
            ..Default::default()
        })
        .unwrap();
    let failed = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(failed.quotas[0].extra_usage, q.extra_usage);
    assert_eq!(failed.quotas[0].updated_at, q.updated_at);
    assert_eq!(failed.quotas[0].error.as_deref(), Some("query failed"));
    let mut legacy = serde_json::to_value(&q).unwrap();
    legacy.as_object_mut().unwrap().remove("extraUsage");
    assert!(
        serde_json::from_value::<QuotaSnapshot>(legacy)
            .unwrap()
            .extra_usage
            .is_none()
    );
}

#[test]
fn qoder_native_zero_account_quota_does_not_report_unused_capacity_or_session_credits() {
    let v = json!({"userId":"fixture-user", "userType":"personal_standard", "isQuotaExceeded":true,
        "userQuota":{"total":0,"used":0,"remaining":0,"percentage":0,"unit":"credits"},
        "session":{"total_credits":999}});
    let q = parse("qoder", "cli", v.clone()).unwrap();
    assert_eq!(q.windows.len(), 1);
    assert_eq!(q.windows[0].used_percent, 100.);
    assert!(q.windows[0].name.contains("剩余 0"));
    assert_eq!(q.is_available, Some(false));
    assert!(!q.identity.as_ref().unwrap().key.contains("fixture-user"));
    assert!(q.balances.is_empty());
    let multiple = json!({"userId":"fixture-user", "userType":"pro", "userQuota":{"total":100,"used":25,"remaining":75,"unit":"credits"},"addOnQuota":{"total":20,"used":5,"remaining":15,"unit":"credits"},"orgResourcePackage":{"cap":500,"used":300,"remaining":200,"unit":"credits","available":false}});
    let q = parse("qoder", "cli", multiple).unwrap();
    assert_eq!(q.windows.len(), 3);
    assert_eq!(q.windows[0].used_percent, 25.);
    assert_eq!(q.windows[2].used_percent, 60.);
    assert_eq!(q.is_available, Some(true));
    for body in [
        json!({"session":{"total_credits":0}}),
        json!({"userId":"fixture-user"}),
        json!({"userId":"fixture-user","userQuota":{"total":10,"used":0,"unit":"credits"}}),
        json!({"userId":"fixture-user","userQuota":{"total":10,"used":0,"remaining":10,"unit":"tokens"}}),
    ] {
        assert!(parse("qoder", "cli", body).is_err());
    }
}

#[test]
fn harness_official_backend_balance_preserves_decimal_strings_and_provider_identity() {
    let body = json!({"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00000001","granted_balance":"10.00000001","topped_up_balance":"100.00"}]});
    let q = parse("deepseek-harness", "api", body.clone()).unwrap();
    assert_eq!(q.provider, "deepseek-harness");
    assert_eq!(q.account_id, "account");
    assert_eq!(q.is_available, Some(true));
    assert!(q.windows.is_empty());
    assert_eq!(q.balances[0].total, "110.00000001");
    assert_eq!(q.balances[0].granted, "10.00000001");
    let mut invalid = body.clone();
    invalid["balance_infos"][0]["total_balance"] = json!("NaN");
    assert!(parse("deepseek-harness", "api", invalid).is_err());
    let mut empty = body.clone();
    empty["balance_infos"] = json!([]);
    assert!(parse("deepseek-harness", "api", empty).is_err());
    let mut unavailable = body;
    unavailable["is_available"] = json!(false);
    assert_eq!(
        parse("deepseek-harness", "api", unavailable)
            .unwrap()
            .is_available,
        Some(false)
    );
    assert!(parse("deepseek-harness", "self-hosted", json!({"remaining":100})).is_err());
}

#[test]
fn kimi_subscription_ratios_and_reset_times_remain_separate_from_local_tokens() {
    let q = parse(
        "kimi",
        "cn",
        json!({"goods_version":2,"usages":{
            "limit_5h":{"used_ratio":0.3,"reset_time":"2026-10-18T18:00:00Z"},
            "limit_7d":{"used_ratio":"0","reset_time":"2026-10-25T00:00:00Z"},
            "limit_month_total":{"used_ratio":0.4},
            "limit_month_code":{"used_ratio":0.25}
        }}),
    )
    .unwrap();
    assert_eq!(q.windows.len(), 4);
    assert_eq!(q.windows[0].used_percent, 30.);
    assert_eq!(q.windows[1].used_percent, 0.);
    assert_eq!(q.windows[2].used_percent, 40.);
    assert_eq!(q.windows[3].used_percent, 25.);
    assert_eq!(
        q.windows[0].resets_at,
        timestamp(&json!("2026-10-18T18:00:00Z"))
    );
    assert!(q.windows[2].window_minutes.is_none());
    assert!(q.balances.is_empty());
    assert!(q.plan.is_none());
    for invalid in [
        json!({}),
        json!({"usages":{}}),
        json!({"usages":{"limit_5h":{"used_ratio":-1}}}),
        json!({"usages":{"limit_5h":{"used_ratio":"NaN"}}}),
    ] {
        assert!(parse("kimi", "cn", invalid).is_err());
    }
    let config = SubscriptionConfig {
        mode: "global".into(),
        ..Default::default()
    };
    assert_eq!(
        subscriptions::endpoint("kimi", &config).unwrap(),
        "https://api.kimi.ai/coding/v1/usages"
    );
    assert!(
        subscriptions::endpoint(
            "kimi",
            &SubscriptionConfig {
                mode: "custom".into(),
                ..Default::default()
            }
        )
        .is_err()
    );
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
    let q = parse(
        "cursor",
        "api2",
        json!({"planUsage":{"autoPercentUsed":0.36,"apiPercentUsed":50.5}}),
    )
    .unwrap();
    assert_eq!(q.windows.len(), 2);
    assert_eq!(q.windows[0].used_percent, 0.36);
    assert_eq!(q.windows[1].used_percent, 50.5);
    assert!(q.windows[0].resets_at.is_none());
    let q = parse(
        "cursor",
        "api2",
        json!({"planUsage":{"totalSpend":1000,"limit":2000}}),
    )
    .unwrap();
    assert_eq!(q.windows[0].id, "plan");
    assert_eq!(q.windows[0].used_percent, 50.);
}

#[test]
fn cursor_api2_on_demand_and_team_pool_meters() {
    let q = parse(
        "cursor",
        "api2",
        json!({"billingCycleEnd":"4102444800000","planUsage":{"totalPercentUsed":12},
            "spendLimitUsage":{"limitType":"user","individualUsed":250,"individualLimit":1000}}),
    )
    .unwrap();
    assert_eq!(q.windows[0].id, "plan");
    assert_eq!(q.windows[0].resets_at, Some(4102444800));
    assert_eq!(q.meters[0].id, "on-demand");
    assert_eq!(q.meters[0].unit, "usd");
    assert_eq!(q.meters[0].used, 2.5);
    assert_eq!(q.meters[0].remaining, Some(7.5));
    // Members of one team share a meter id, so the pool is shown once.
    let team = |t| {
        parse(
            "cursor",
            "api2",
            json!({"spendLimitUsage":{"limitType":"team","teamId":t,"pooledUsed":100,"pooledLimit":500}}),
        )
        .unwrap()
    };
    assert_eq!(team(json!(42)).meters[0].id, "team-pool:42");
    assert_eq!(team(json!(42)).meters[0].id, team(json!("42")).meters[0].id);
    assert!(
        subscriptions::endpoint(
            "cursor",
            &SubscriptionConfig {
                mode: "session".into(),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn moonshot_key_in_kimi_config_reads_open_platform_balance() {
    let body = json!({"code":0,"status":true,"data":{"available_balance":12.5,"voucher_balance":0,"cash_balance":12.5}});
    let q = parse("kimi", "moonshot-cn", body.clone()).unwrap();
    assert_eq!(q.balances[0].currency, "CNY");
    assert_eq!(q.balances[0].total, "12.50");
    assert_eq!(
        parse("kimi", "moonshot-global", body).unwrap().balances[0].currency,
        "USD"
    );
    assert!(parse("kimi", "moonshot-cn", json!({"code":1,"status":false})).is_err());
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
        ("cursor", "api2"),
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
    assert!(subscriptions::local_secret("grok", "subscription", &ambiguous).is_ok());
    assert_eq!(
        subscriptions::login_url("zcode", "cn").unwrap(),
        "https://bigmodel.cn/coding-plan/personal/usage"
    );
    assert!(subscriptions::login_url("https://evil.invalid", "global").is_err());
}

#[test]
fn grok_with_several_logins_uses_the_most_recently_used_valid_entry() {
    let auth = json!({
        "https://auth.x.ai::old":{"key":"fixture-old","last_used":"2026-01-01T00:00:00Z"},
        "https://auth.x.ai::new":{"key":"fixture-new","last_used":"2026-10-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z"},
        "https://auth.x.ai::gone":{"key":"fixture-gone","last_used":"2026-10-20T00:00:00Z","expires_at":"2020-01-01T00:00:00Z"}
    });
    assert_eq!(
        subscriptions::local_secret("grok", "subscription", &auth).unwrap(),
        "fixture-new"
    );
    // Equal recency is still ambiguous: the manual token field decides.
    let tie = json!({
        "https://auth.x.ai::a":{"key":"a","last_used":1700000000},
        "https://auth.x.ai::b":{"key":"b","last_used":1700000000}
    });
    assert_eq!(
        subscriptions::local_secret("grok", "subscription", &tie).unwrap(),
        "b"
    );
}

#[test]
fn grok_current_native_expiry_selects_a_valid_login_without_last_used() {
    let auth = json!({
        "https://auth.x.ai::old":{"key":"old-native","expires_at":"2098-01-01T00:00:00Z"},
        "https://auth.x.ai::new":{"key":"new-native","expires_at":"2099-01-01T00:00:00Z"}
    });
    assert_eq!(
        subscriptions::local_secret("grok", "subscription", &auth).unwrap(),
        "new-native"
    );
}

#[test]
fn zcode_quota_parses_the_same_body_for_a_login_jwt_or_coding_plan_key() {
    let body = json!({"code":200,"success":true,"data":{"planName":"Pro","limits":[
        {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":40,"nextResetTime":4102444800000_i64},
        {"type":"TIME_LIMIT","unit":5,"number":1,"usage":100,"currentValue":10}
    ]}});
    for mode in ["global", "cn"] {
        let q = parse("zcode", mode, body.clone()).unwrap();
        assert_eq!(q.plan.as_deref(), Some("Pro"));
        assert_eq!(q.windows.len(), 2);
        assert_eq!(q.windows[0].used_percent, 40.0);
        assert_eq!(q.windows[0].window_minutes, Some(300));
    }
    assert!(
        subscriptions::endpoint(
            "zcode",
            &SubscriptionConfig {
                mode: "eu".into(),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn deepseek_account_summary_reads_wallets_and_asks_to_relogin_in_dsh() {
    let config = SubscriptionConfig {
        mode: "account".into(),
        ..Default::default()
    };
    assert_eq!(
        subscriptions::endpoint("deepseek-harness", &config).unwrap(),
        "https://platform.deepseek.com/api/v0/users/get_user_summary"
    );
    // The account endpoint belongs to dsh logins only, never to the API-key provider.
    assert!(subscriptions::endpoint("deepseek", &config).is_err());
    let q = parse(
        "deepseek-harness",
        "account",
        json!({"code":0,"data":{"biz_code":0,"biz_data":{
            "email":"PRIVATE@example.test",
            "normal_wallets":[{"currency":"CNY","balance":"12.50","token_estimation":"1"}],
            "bonus_wallets":[{"currency":"CNY","balance":"2.25"}]
        }}}),
    )
    .unwrap();
    assert_eq!(q.balances.len(), 1);
    assert_eq!(q.balances[0].currency, "CNY");
    assert_eq!(q.balances[0].total, "14.75");
    assert_eq!(q.balances[0].granted, "2.25");
    assert_eq!(q.balances[0].topped_up, "12.5");
    assert!(!serde_json::to_string(&q).unwrap().contains("PRIVATE"));
    let expired = parse(
        "deepseek-harness",
        "account",
        json!({"code":0,"data":{"biz_code":40003}}),
    );
    assert!(
        expired
            .unwrap_err()
            .to_string()
            .contains("在 dsh 中重新登录")
    );
    assert!(parse("deepseek-harness", "account", json!({"code":0,"data":{"biz_code":0,"biz_data":{"normal_wallets":[{"currency":"EUR","balance":"1"}]}}})).is_err());
}
