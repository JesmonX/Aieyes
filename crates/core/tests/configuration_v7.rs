use aieyes_core::{Engine, models::*};
use serde_json::json;

#[test]
fn legacy_profile_migrates_once_and_removed_connections_do_not_resurrect() {
    let mut settings = Settings {
        version: 6,
        sources: vec![Source {
            id: "s".into(),
            path: "/fixture".into(),
            codex_home_id: Some("home".into()),
            ..Default::default()
        }],
        accounts: vec![Account {
            id: "a".into(),
            quota_profile_id: Some("home:p".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    settings.migrate();
    assert_eq!(settings.version, 7);
    assert_eq!(settings.accounts[0].connections.len(), 1);
    let first = json!(settings);
    settings.migrate();
    assert_eq!(json!(settings), first);
    settings.accounts[0].connections.clear();
    settings.migrate();
    assert!(settings.accounts[0].connections.is_empty());
    assert!(settings.accounts[0].quota_profile_id.is_none());
}

#[test]
fn conflicting_agents_are_visible_and_repairable_and_wrong_home_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let mut settings = Settings {
        sources: vec![Source {
            id: "s".into(),
            path: "/fixture".into(),
            codex_home_id: Some("home".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    settings.migrate();
    let mut duplicate = settings.agents[0].clone();
    duplicate.enabled = !duplicate.enabled;
    settings.agents.push(duplicate);
    engine
        .store
        .db
        .execute(
            "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
            [json!(settings).to_string()],
        )
        .unwrap();
    let observed = engine.call("settings.get", json!({})).unwrap();
    assert!(
        !observed["configurationIssues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let saved = engine
        .call(
            "agents.set",
            json!({"provider":"codex","enabled":true,"machineIds":["local","local"]}),
        )
        .unwrap();
    assert!(saved["configurationIssues"].as_array().unwrap().is_empty());
    assert_eq!(
        saved["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["provider"] == "codex")
            .count(),
        1
    );
    let mut invalid = engine.store.settings().unwrap();
    invalid.accounts.push(Account {
        id: "a".into(),
        connections: vec![AccountConnection {
            source_id: "s".into(),
            profile_id: Some("wrong:p".into()),
        }],
        ..Default::default()
    });
    assert!(
        engine
            .store
            .save_settings(&invalid)
            .unwrap_err()
            .to_string()
            .contains("home")
    );
    assert_eq!(engine.call("settings.get", json!({})).unwrap(), saved);
}
