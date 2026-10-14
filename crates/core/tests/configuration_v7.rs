use aieyes_core::{Engine, models::*};
use serde_json::json;

#[test]
fn source_availability_matches_shared_client_fixtures() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/source-availability.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let settings: Settings = serde_json::from_value(json!({
            "sources": fixture["sources"], "agents": case["agents"], "hosts": case["hosts"]
        }))
        .unwrap();
        let enabled: Vec<_> = settings
            .sources
            .iter()
            .filter(|source| settings.source_enabled(source))
            .map(|source| &source.id)
            .collect();
        assert_eq!(json!(enabled), case["expectedEnabled"], "{}", case["name"]);
        engine
            .store
            .db
            .execute(
                "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
                [json!(settings).to_string()],
            )
            .unwrap();
        let dashboard = engine.call("dashboard", json!({})).unwrap();
        let effective: Vec<_> = dashboard["sources"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|source| source["enabled"] == true)
            .map(|source| &source["id"])
            .collect();
        assert_eq!(
            &json!(effective),
            case.get("expectedNormalizedEnabled")
                .unwrap_or(&case["expectedEnabled"]),
            "dashboard: {}",
            case["name"]
        );
    }
}

#[test]
fn account_targets_use_committed_agent_machine_host_and_archive_rules() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(root.path()).unwrap();
    let original: Settings = serde_json::from_value(json!({
        "version":7,
        "agents":[{"provider":"claude","enabled":true,"machineIds":["local","online","paused","missing"]}],
        "sources":[
            {"id":"local","provider":"claude","accountId":"fixture","enabled":true},
            {"id":"remote","provider":"claude","accountId":"fixture","hostId":"online","enabled":true},
            {"id":"paused","provider":"claude","accountId":"fixture","hostId":"paused","enabled":true},
            {"id":"missing","provider":"claude","accountId":"fixture","hostId":"missing","enabled":true},
            {"id":"disabled","provider":"claude","accountId":"fixture","enabled":false}
        ],
        "accounts":[{"id":"fixture","provider":"claude","quotaEnabled":true}],
        "hosts":[{"id":"online","enabled":true},{"id":"paused","enabled":false}]
    })).unwrap();
    let mut cases = vec![("active", original.clone(), vec!["local", "remote"])];
    let mut paused = original.clone();
    paused.agents[0].enabled = false;
    cases.push(("paused agent", paused, vec![]));
    let mut machines = original.clone();
    machines.agents[0].machine_ids = vec!["online".into()];
    cases.push(("deselected local", machines, vec!["remote"]));
    let mut local = original.clone();
    local.local_device_enabled = false;
    cases.push(("paused local collection", local, vec!["remote"]));
    let mut duplicate = original.clone();
    let mut conflicting = duplicate.agents[0].clone();
    conflicting.enabled = false;
    duplicate.agents.push(conflicting);
    cases.push(("conflicting agent", duplicate, vec![]));
    let mut host = original.clone();
    host.hosts[0].enabled = false;
    cases.push(("paused host", host, vec!["local"]));
    let mut archived = original;
    archived.accounts[0].archived = true;
    cases.push(("archived account", archived, vec![]));
    for (name, settings, expected) in cases {
        engine
            .store
            .db
            .execute(
                "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
                [json!(settings).to_string()],
            )
            .unwrap();
        let targets = engine.call("accounts.targets", json!({})).unwrap();
        let actual: Vec<_> = targets
            .as_array()
            .unwrap()
            .iter()
            .map(|target| target["sourceId"].as_str().unwrap())
            .collect();
        assert_eq!(actual, expected, "{name}");
        assert!(
            targets
                .as_array()
                .unwrap()
                .iter()
                .all(|target| target["canWake"] == true),
            "{name}: {targets}"
        );
        let drafts = engine
            .call("accounts.targets", json!({"includeUnavailable":true}))
            .unwrap();
        if name != "archived account" {
            let rows = drafts.as_array().unwrap();
            assert_eq!(
                rows.len(),
                5,
                "{name}: paused sources remain visible for drafts"
            );
            for row in rows {
                let id = row["sourceId"].as_str().unwrap();
                assert_eq!(row["available"], expected.contains(&id), "{name}: {row}");
                assert_eq!(row["canDraft"], id != "missing", "{name}: {row}");
                if !expected.contains(&id) {
                    assert_eq!(row["canWake"], false);
                }
            }
        } else {
            assert!(drafts.as_array().unwrap().is_empty());
        }
    }
}

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
    assert_eq!(settings.version, 10);
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
