use aieyes_core::{Engine, models::*};
use serde_json::json;

#[test]
fn references_are_persisted_preserve_manual_choices_and_only_fill_missing_costs() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let prices = vec![
        ModelPrice {
            id: "anthropic/claude-sonnet-4.5".into(),
            name: "Anthropic: Claude Sonnet 4.5".into(),
            input: Some(0.000003),
            output: Some(0.000015),
            ..Default::default()
        },
        ModelPrice {
            id: "anthropic/claude-sonnet-latest".into(),
            name: "Claude Sonnet Latest".into(),
            input: Some(0.000003),
            ..Default::default()
        },
        ModelPrice {
            id: "anthropic/claude-sonnet-4.5:batch".into(),
            name: "Claude Sonnet 4.5".into(),
            input: Some(0.0000015),
            ..Default::default()
        },
    ];
    engine.store.save_prices(&prices).unwrap();
    engine
        .store
        .db
        .execute(
            "INSERT INTO kv(key,value) VALUES('openrouter-models',?1)",
            [serde_json::to_string(&prices).unwrap()],
        )
        .unwrap();
    let custom = ModelPrice {
        id: "deployment".into(),
        name: "Claude Sonnet 4.5".into(),
        ..Default::default()
    };
    engine.store.save_prices(&[custom]).unwrap();
    let mut settings = engine.store.settings().unwrap();
    settings
        .model_mappings
        .insert("manually-mapped".into(), prices[2].id.clone());
    engine.store.save_settings(&settings).unwrap();
    for (id, model, frozen) in [
        ("missing", "relay/claude-4-5-sonnet", false),
        ("priced", "relay/claude-4-5-sonnet", true),
        ("latest", "claude-sonnet", false),
        ("unknown", "claude-sonnet-9.9", false),
        ("manual", "manually-mapped", false),
    ] {
        let e: UsageEvent = serde_json::from_value(json!({
            "id":id,"model":model,"provider":"custom","sourceId":"source",
            "accountId":"account","sessionId":"session","timestamp":now(),
            "tokens":{"input":100},"attribution":"exact"
        }))
        .unwrap();
        let rates = if frozen {
            vec![ModelPrice {
                id: model.into(),
                input: Some(1.0),
                ..Default::default()
            }]
        } else {
            vec![]
        };
        engine.store.put_event(&e, &rates, &settings).unwrap();
    }
    let result = engine.call("prices.reference", json!({})).unwrap();
    assert_eq!(result["matched"], 3);
    assert_eq!(result["unmatched"], json!(["claude-sonnet-9.9"]));
    let mappings = &result["settings"]["modelMappings"];
    assert_eq!(mappings["relay/claude-4-5-sonnet"], prices[0].id);
    assert_eq!(mappings["deployment"], prices[0].id);
    assert_eq!(mappings["claude-sonnet"], prices[1].id);
    assert_eq!(mappings["manually-mapped"], prices[2].id);
    for (id, cost) in [("missing", 0.0003), ("priced", 100.0)] {
        let actual: f64 = engine
            .store
            .db
            .query_row("SELECT cost FROM events WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert!((actual - cost).abs() < 1e-12, "{id}: {actual}");
    }
    assert_eq!(
        engine.call("prices.reference", json!({})).unwrap()["matched"],
        0
    );
    drop(engine);
    let mut engine = Engine::open(root.path()).unwrap();
    assert_eq!(
        engine.call("settings.get", json!({})).unwrap()["modelMappings"],
        *mappings
    );
}

#[test]
fn cached_legacy_prices_and_name_only_references_work_without_network() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    assert!(
        engine
            .call("prices.reference", json!({"model":"alias"}))
            .is_err()
    );
    engine
        .store
        .save_prices(&[ModelPrice {
            id: "openai/gpt-4.1".into(),
            input: Some(0.000002),
            ..Default::default()
        }])
        .unwrap();
    let r = engine
        .call(
            "prices.reference",
            json!({"model":"internal-deployment","name":"OpenAI: GPT-4.1"}),
        )
        .unwrap();
    assert_eq!(
        r["settings"]["modelMappings"]["internal-deployment"],
        "openai/gpt-4.1"
    );
    assert_eq!(r["matched"], 1);
    assert_eq!(
        engine
            .call("prices.reference", json!({"model":""}))
            .unwrap_err()
            .to_string(),
        "请输入模型 ID"
    );
}
