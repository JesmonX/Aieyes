use aieyes_core::{Engine, models::*, quota, store::Store};
use serde_json::json;

fn query_source(provider: &str) -> Source {
    Source {
        id: "query".into(),
        account_id: "personal".into(),
        provider: provider.into(),
        ..Default::default()
    }
}

#[test]
fn agy_groups_have_independent_windows_and_reset_times() {
    let payload = json!({"status":"SUCCESS","command":{"name":"usage","data":{"groups":[
        {"name":"Gemini Models","buckets":[{"window":"weekly","remaining_fraction":0.75,"reset_time":"2026-10-08T03:05:12Z"},{"window":"5h","remaining_fraction":0.0}]},
        {"name":"Claude and GPT models","buckets":[{"window":"5h","remaining_fraction":1.0},{"window":"weekly","disabled":true,"remaining_fraction":0.0}]}
    ]}}});
    let q = quota::normalize(&query_source("agy"), &payload, "live").unwrap();
    assert_eq!(q.windows.len(), 3);
    assert_eq!(q.windows[0].used_percent, 25.0);
    assert_eq!(q.windows[0].window_minutes, Some(10080));
    assert_eq!(
        q.windows[0].resets_at,
        timestamp(&json!("2026-10-08T03:05:12Z"))
    );
    assert_eq!(q.windows[1].used_percent, 100.0);
    assert_eq!(q.windows[2].used_percent, 0.0);
    assert_ne!(q.windows[1].name, q.windows[2].name);
    let mut malformed = payload;
    malformed["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = json!(1.2);
    assert!(quota::normalize(&query_source("agy"), &malformed, "live").is_err());
    assert!(quota::normalize(&query_source("agy"), &json!({"status":"ERROR"}), "live").is_err());
}

#[test]
fn deepseek_balances_preserve_currency_precision_and_unavailability() {
    let payload = json!({"is_available":false,"balance_infos":[
        {"currency":"CNY","total_balance":"0.000001","granted_balance":"0.000001","topped_up_balance":"0.00"},
        {"currency":"USD","total_balance":"-0.12","granted_balance":"0.00","topped_up_balance":"-0.12"}
    ]});
    let q = quota::normalize(&query_source("deepseek"), &payload, "live").unwrap();
    assert!(q.windows.is_empty());
    assert_eq!(q.is_available, Some(false));
    assert_eq!(q.balances.len(), 2);
    assert_eq!(q.balances[0].total, "0.000001");
    assert_eq!(q.balances[1].currency, "USD");
    let mut malformed = payload;
    malformed["balance_infos"][0]["total_balance"] = json!("NaN");
    assert!(quota::normalize(&query_source("deepseek"), &malformed, "live").is_err());
}

#[test]
fn balances_are_persisted_and_last_success_survives_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let source = query_source("deepseek");
    store
        .save_settings(&Settings {
            version: 2,
            sources: vec![source.clone()],
            accounts: vec![Account {
                id: source.account_id.clone(),
                provider: source.provider.clone(),
                name: "DeepSeek".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let q = quota::normalize(&source, &json!({"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}), "live").unwrap();
    store.quota(&q).unwrap();
    store
        .quota(&QuotaSnapshot {
            account_id: source.account_id,
            provider: source.provider,
            error: Some("连接失败".into()),
            ..Default::default()
        })
        .unwrap();
    let d = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.quotas.len(), 1);
    assert_eq!(d.quotas[0].balances[0].total, "110.00");
    assert_eq!(d.quotas[0].updated_at, q.updated_at);
    assert!(d.quotas[0].error.is_some());
}

#[test]
fn query_only_sources_do_not_scan_credential_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(temp.path()).unwrap();
    let sources = vec![
        query_source("agy"),
        Source {
            id: "balance".into(),
            ..query_source("deepseek")
        },
    ];
    let accounts = sources
        .iter()
        .map(|s| Account {
            id: s.account_id.clone(),
            provider: s.provider.clone(),
            name: s.provider.clone(),
            ..Default::default()
        })
        .collect();
    engine
        .store
        .save_settings(&Settings {
            version: 2,
            sources,
            accounts,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(engine.call("sources.scan", json!({})).unwrap(), json!([]));
    let saved = engine
        .call(
            "credentials.save",
            json!({"sourceId":"../../outside", "apiKey":"fixture-key"}),
        )
        .unwrap();
    let path = std::path::Path::new(saved["path"].as_str().unwrap());
    assert_eq!(
        path.parent().unwrap(),
        temp.path().canonicalize().unwrap().join("credentials")
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "fixture-key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(!saved.to_string().contains("fixture-key"));
}
