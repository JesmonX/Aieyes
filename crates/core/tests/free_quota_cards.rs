use aieyes_core::{Engine, models::*};
use serde_json::{Value, json};

/// Codex-shaped quota output for a custom command, with the given plan.
fn command(plan: &str) -> String {
    let reset = chrono::Utc::now().timestamp() + 18_000;
    let body = json!({"rateLimits":{"planType":plan,"primary":{"usedPercent":20,"windowDurationMins":300,"resetsAt":reset}}});
    if cfg!(windows) {
        format!("Write-Output '{body}'")
    } else {
        format!("echo '{body}'")
    }
}

fn engine() -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    let engine = Engine::open(dir.path()).unwrap();
    let mut settings = Settings::default();
    for (id, plan) in [("free", "free"), ("plus", "plus")] {
        settings.accounts.push(Account {
            id: id.into(),
            name: id.into(),
            provider: "codex".into(),
            quota_enabled: true,
            ..Default::default()
        });
        settings.sources.push(Source {
            id: id.into(),
            name: id.into(),
            provider: "codex".into(),
            account_id: id.into(),
            path: logs.to_string_lossy().into(),
            quota_command: command(plan),
            ..Default::default()
        });
    }
    engine.store.save_settings(&settings).unwrap();
    (dir, engine)
}

fn flags(rows: &Value) -> Vec<(String, bool)> {
    let mut flags: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            (
                q["accountId"].as_str().unwrap().to_owned(),
                q["freePlan"] == true,
            )
        })
        .collect();
    flags.sort();
    flags
}

#[test]
fn setting_defaults_off_and_round_trips_through_a_patch() {
    assert!(!Settings::default().hide_free_quota_cards);
    assert!(
        serde_json::from_value::<Settings>(json!({"hideFreeQuotaCards":true}))
            .unwrap()
            .hide_free_quota_cards
    );
    let (_dir, mut engine) = engine();
    let base = engine.call("settings.get", json!({})).unwrap();
    assert_eq!(base["hideFreeQuotaCards"], false);
    let mut next = base.clone();
    next["hideFreeQuotaCards"] = json!(true);
    engine
        .call("settings.patch", json!({"base":base,"settings":next}))
        .unwrap();
    assert!(engine.store.settings().unwrap().hide_free_quota_cards);
    assert_eq!(
        engine.call("settings.get", json!({})).unwrap()["hideFreeQuotaCards"],
        true
    );
}

#[test]
fn refreshed_rows_and_the_dashboard_flag_free_plans_without_storing_the_flag() {
    let (_dir, mut engine) = engine();
    let expected = vec![("free".to_owned(), true), ("plus".to_owned(), false)];
    let streamed = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let sink = streamed.clone();
    let rows = engine
        .call_with_progress(
            "quotas.refresh",
            json!({}),
            Box::new(move |p| {
                if p["kind"] == "quotas.refresh" {
                    sink.lock().unwrap().push(p["row"].clone());
                }
            }),
        )
        .unwrap();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|q| q["error"].is_null()),
        "{rows}"
    );
    assert_eq!(flags(&rows), expected);
    assert_eq!(
        flags(&Value::Array(streamed.lock().unwrap().clone())),
        expected
    );
    let dashboard = engine.call("dashboard.status", json!({})).unwrap();
    assert_eq!(flags(&dashboard["quotas"]), expected);
    // Stored payloads stay as the provider reported them.
    let stored: Vec<String> = engine
        .store
        .db
        .prepare("SELECT payload FROM quotas")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|p| !p.contains("freePlan")));
}
