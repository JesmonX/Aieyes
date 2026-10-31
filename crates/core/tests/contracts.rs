use aieyes_core::{Engine, contracts, models::*, monitoring};
use serde_json::{Value, json};

#[test]
fn manifest_matches_routing_and_read_methods_do_not_invalidate() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let manifest = engine.call("methods.manifest", json!({})).unwrap();
    let settings = engine.call("settings.get", json!({})).unwrap();
    let hello = engine.call("hello", json!({})).unwrap();
    assert_eq!(hello["contractHash"], contracts::CONTRACT_HASH);
    assert!(hello["chatgptWorkInstalled"].is_boolean());
    assert_eq!(engine.call("settings.get", json!({})).unwrap(), settings);
    for key in ["accounts", "sources"] {
        assert!(
            settings[key]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["provider"] != "chatgpt-work")
        );
    }
    assert_eq!(manifest, serde_json::to_value(contracts::METHODS).unwrap());
    assert_eq!(
        contracts::method("accounts.create").unwrap().lane,
        contracts::Lane::Configuration
    );
    assert_eq!(
        contracts::method("accounts.delete").unwrap().lane,
        contracts::Lane::Accounts
    );
    for name in [
        "settings.get",
        "wakeups.list",
        "wakeups.history",
        "creditEstimates.get",
        "quotaEstimates.list",
    ] {
        assert!(contracts::method(name).unwrap().invalidates.is_empty());
    }
}

#[test]
fn dashboard_projection_keeps_persisted_reasoning_tokens_intact() {
    let tokens = Tokens {
        input: 100,
        reasoning: 99,
        ..Default::default()
    };
    let persisted = serde_json::to_value(&tokens).unwrap();
    assert_eq!(persisted["reasoning"], 99);
    assert_eq!(
        serde_json::from_value::<Tokens>(persisted)
            .unwrap()
            .reasoning,
        99
    );
    let aggregate = Aggregate {
        tokens,
        total: 100,
        ..Default::default()
    };
    let dashboard = Dashboard {
        summary: aggregate.clone(),
        models: vec![aggregate.clone()],
        days: vec![aggregate],
        price_updated_at: Some(123),
        ..Default::default()
    };
    let wire = dashboard.wire_value().unwrap();
    assert!(wire.get("days").is_none() && wire.get("priceUpdatedAt").is_none());
    assert!(wire["summary"]["tokens"].get("reasoning").is_none());
    assert!(wire["models"][0]["tokens"].get("reasoning").is_none());
    assert_eq!(dashboard.summary.tokens.reasoning, 99);
    assert_eq!(wire["summary"]["total"], 100);
}

#[test]
fn core_and_native_share_filesystem_and_device_selection_fixtures() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/monitor-selection.json"
    ))
    .unwrap();
    for row in fixture["filesystems"].as_array().unwrap() {
        assert_eq!(
            monitoring::recommended_filesystem(row),
            row["expected"].as_bool().unwrap(),
            "{row}"
        );
    }
    for case in fixture["devices"].as_array().unwrap() {
        let mut sample = json!({case["group"].as_str().unwrap():case["rows"]});
        let host = Host {
            devices: serde_json::from_value(case["tokens"].clone()).unwrap(),
            ..Default::default()
        };
        monitoring::filter_devices(&mut sample, &host);
        let ids: Vec<_> = sample[case["group"].as_str().unwrap()]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].clone())
            .collect();
        assert_eq!(json!(ids), case["expected"], "{}", case["name"]);
    }
}
