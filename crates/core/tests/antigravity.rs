use aieyes_core::{antigravity, import, models::*, store::Store, usage};
use rusqlite::Connection;
use serde_json::{Value, json};

fn varint(mut n: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while n >= 128 {
        bytes.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    bytes.push(n as u8);
    bytes
}
fn number(field: u64, value: u64) -> Vec<u8> {
    [varint(field << 3), varint(value)].concat()
}
fn bytes(field: u64, value: &[u8]) -> Vec<u8> {
    [
        varint((field << 3) | 2),
        varint(value.len() as u64),
        value.to_vec(),
    ]
    .concat()
}
fn step(start: u64, end: u64) -> Vec<u8> {
    [
        bytes(1, &number(1, start)),
        bytes(8, &number(1, end)),
        bytes(
            9,
            &[
                number(2, 100),
                number(3, 20),
                number(4, 30),
                number(5, 50),
                number(9, 10),
                bytes(7, b"call"),
            ]
            .concat(),
        ),
        bytes(
            20,
            b"private conversation text must never leave the collector",
        ),
    ]
    .concat()
}
fn source(path: &std::path::Path) -> Source {
    Source {
        id: "source".into(),
        name: "Antigravity · 本机".into(),
        provider: "antigravity".into(),
        account_id: "account".into(),
        path: path.to_string_lossy().into(),
        ..Default::default()
    }
}

#[test]
fn partial_databases_keep_tokens_retry_and_repair_model_without_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("conversations");
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("valid.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE trajectory_meta(cascade_id TEXT); INSERT INTO trajectory_meta VALUES('partial-session'); CREATE TABLE gen_metadata(data BLOB); CREATE TABLE steps(idx INTEGER,status INTEGER,metadata BLOB);").unwrap();
    let stamp = now() as u64 - 100;
    let row = |id: &str, numeric: u64| {
        [
            bytes(1, &number(1, stamp)),
            bytes(8, &number(1, stamp + 10)),
            bytes(
                9,
                &[number(1, numeric), number(2, 100), bytes(7, id.as_bytes())].concat(),
            ),
        ]
        .concat()
    };
    db.execute(
        "INSERT INTO gen_metadata VALUES(?1)",
        [[bytes(1, &bytes(19, b"known")), number(2, 0)].concat()],
    )
    .unwrap();
    for (index, id, numeric) in [(0, "known", 10), (1, "recoverable", 10), (2, "unknown", 99)] {
        db.execute(
            "INSERT INTO steps VALUES(?1,3,?2)",
            rusqlite::params![index, row(id, numeric)],
        )
        .unwrap();
    }
    db.execute("INSERT INTO steps VALUES(3,3,?1)", [vec![255u8]])
        .unwrap();
    std::fs::write(dir.join("broken.db"), b"not sqlite").unwrap();
    let read = antigravity::read_database_detailed(&path).unwrap();
    assert_eq!(read.events.len(), 3);
    assert_eq!(read.events[1]["model"], "known");
    assert_eq!(read.events[2]["model"], "antigravity-unknown-99");
    assert_eq!(read.issues.len(), 2);
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/remote_history.py"
        ))
        .arg(&dir)
        .arg("antigravity")
        .output()
        .unwrap();
    let remote: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(remote["partial"], true);
    assert_eq!(remote["failedFiles"], 1);
    assert_eq!(remote["files"][0]["events"], json!(read.events));
    let store = Store::open(&temp.path().join("store")).unwrap();
    let source = source(temp.path());
    let settings = Settings::default();
    store
        .save_prices(&[
            price("known"),
            price("repaired"),
            price("antigravity-unknown-99"),
        ])
        .unwrap();
    let first = antigravity::scan(&store, &settings, &source).unwrap();
    assert_eq!(first["newEvents"], 3);
    assert_eq!(first["partial"], true);
    assert_eq!(first["failedFiles"], 1);
    let second = antigravity::scan(&store, &settings, &source).unwrap();
    assert_eq!(second["newEvents"], 0);
    assert_eq!(second["partial"], true);
    let mut engine = aieyes_core::Engine::open(&temp.path().join("store")).unwrap();
    engine
        .store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let public = engine
        .call("sources.scan", json!({"sourceId":source.id}))
        .unwrap();
    assert_eq!(public[0]["partial"], true);
    assert!(public[0]["issues"].as_array().unwrap().len() >= 2);
    assert!(public[0].get("error").is_none());

    assert_eq!(
        store
            .dashboard(&Filter::default())
            .unwrap()
            .summary
            .priced_tokens,
        200
    );
    db.execute(
        "INSERT INTO gen_metadata VALUES(?1)",
        [[bytes(1, &bytes(19, b"repaired")), number(2, 2)].concat()],
    )
    .unwrap();
    db.execute("DELETE FROM steps WHERE idx=3", []).unwrap();
    std::fs::remove_file(dir.join("broken.db")).unwrap();
    let repaired = antigravity::scan(&store, &settings, &source).unwrap();
    assert_eq!(repaired["newEvents"], 0);
    assert_eq!(repaired["partial"], false);
    let dashboard = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(dashboard.summary.total, 300);
    assert_eq!(dashboard.summary.priced_tokens, 300);
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE model='repaired'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    // A late copy with less metadata must not undo the repaired model.
    let event = usage::parse(&source, &mut ParseState::default(), &read.events[2]).unwrap();
    store
        .put_event(&event, &store.prices().unwrap(), &settings)
        .unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE model='repaired'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}
fn price(model: &str) -> ModelPrice {
    ModelPrice {
        id: model.into(),
        input: Some(0.01),
        output: Some(0.02),
        cache_read: Some(0.001),
        cache_write: Some(0.02),
        ..Default::default()
    }
}

#[test]
fn native_and_ssh_match_wal_incremental_and_standard_import() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("conversations");
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("conversation.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE trajectory_meta(cascade_id TEXT); INSERT INTO trajectory_meta VALUES('session');
        CREATE TABLE gen_metadata(data BLOB); CREATE TABLE steps(idx INTEGER,status INTEGER,metadata BLOB);").unwrap();
    let metadata = [
        bytes(1, &bytes(19, b"gemini-test")),
        bytes(2, &[varint(0), varint(1)].concat()),
    ]
    .concat();
    db.execute("INSERT INTO gen_metadata VALUES(?1)", [&metadata])
        .unwrap();
    let start = now() as u64 - 100;
    db.execute(
        "INSERT INTO steps VALUES(0,3,?1)",
        [step(start, start + 10)],
    )
    .unwrap();
    db.execute("INSERT INTO steps VALUES(1,1,?1)", [step(start + 20, 0)])
        .unwrap();
    let rows = antigravity::read_database(&path).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["tokens"],
        json!({"input":100,"output":20,"cacheRead":50,"cacheWrite":30,"reasoning":10})
    );
    assert_eq!(rows[0]["model"], "gemini-test");
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/remote_history.py"
        ))
        .arg(format!("{}/", dir.display()))
        .arg("antigravity")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(remote["files"][0]["events"], json!(rows));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private conversation"));

    let store = Store::open(&temp.path().join("store")).unwrap();
    let s = source(temp.path());
    let settings = Settings {
        sources: vec![s.clone()],
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    store.save_prices(&[price("gemini-test")]).unwrap();
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &s).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &s).unwrap()["newEvents"],
        0
    );
    let d = store.dashboard(&Filter::default()).unwrap();
    assert_eq!(d.summary.total, 200);
    assert!((d.summary.cost - 2.05).abs() < 1e-10);
    let finished = step(start + 20, start + 30);
    // The same stable call ID with updated counters is not counted twice.
    db.execute(
        "UPDATE steps SET status=3,metadata=?1 WHERE idx=1",
        [finished],
    )
    .unwrap();
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &s).unwrap()["newEvents"],
        0
    );
    let standard = temp.path().join("standard");
    std::fs::create_dir(&standard).unwrap();
    std::fs::write(standard.join("copy.jsonl"), format!("{}\n", rows[0])).unwrap();
    let copy = Source {
        id: "copy".into(),
        path: standard.to_string_lossy().into(),
        ..s.clone()
    };
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &copy).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.total,
        200
    );
    // A new call with unpacked indices and the alternate model field is imported once.
    db.execute(
        "INSERT INTO gen_metadata VALUES(?1)",
        [[bytes(3, &bytes(28, b"claude-test")), number(2, 2)].concat()],
    )
    .unwrap();
    let metadata = [
        bytes(1, &number(1, start + 40)),
        bytes(8, &number(1, start + 50)),
        bytes(9, &[number(2, 10), bytes(7, b"second-call")].concat()),
    ]
    .concat();
    db.execute("INSERT INTO steps VALUES(2,3,?1)", [metadata])
        .unwrap();
    assert_eq!(
        antigravity::read_database(&path).unwrap().last().unwrap()["model"],
        "claude-test"
    );
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &s).unwrap()["newEvents"],
        1
    );
    assert_eq!(
        import::scan(&store, &store.settings().unwrap(), &s).unwrap()["newEvents"],
        0
    );
    assert_eq!(
        store.dashboard(&Filter::default()).unwrap().summary.total,
        210
    );
}

#[test]
fn legacy_migration_keeps_conflicting_accounts_ids_costs_and_wakeups() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let legacy = Source {
        provider: "agy".into(),
        name: "agy · 本机".into(),
        ..source(temp.path())
    };
    let other = Source {
        id: "other".into(),
        ..source(temp.path())
    };
    let mut settings = Settings {
        sources: vec![legacy.clone(), other],
        ..Default::default()
    };
    settings.migrate();
    let row = json!({"id":"call","sessionId":"session","timestamp":now(),"model":"gemini-test","tokens":{"input":100}});
    let mut event = usage::parse(&legacy, &mut ParseState::default(), &row).unwrap();
    event.provider = "agy".into();
    store
        .put_event(&event, &[price("gemini-test")], &settings)
        .unwrap();
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO kv VALUES('settings',?1)",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO kv VALUES('quotaOrder',?1)",
            [json!(["agy:account", "antigravity:account"]).to_string()],
        )
        .unwrap();
    for provider in ["agy", "antigravity"] {
        let payload = json!({"provider":provider,"accountId":"account","sourceId":"source","name":provider,"updatedAt":now(),"windows":[]});
        store
            .db
            .execute(
                "INSERT INTO quotas VALUES(?1,?2)",
                rusqlite::params![format!("{provider}:account"), payload.to_string()],
            )
            .unwrap();
    }
    let wake =
        json!({"task":{"id":"keep-task","sourceId":"source"},"deployment":{"source":legacy}});
    store
        .db
        .execute(
            "INSERT OR REPLACE INTO kv VALUES('wakeup:keep-task',?1)",
            [wake.to_string()],
        )
        .unwrap();
    store
        .db
        .execute("DELETE FROM kv WHERE key='antigravity.migrated.v1'", [])
        .unwrap();
    drop(store);
    let store = Store::open(temp.path()).unwrap();
    let settings = store.settings().unwrap();
    assert!(settings.sources.iter().all(|s| s.provider == "antigravity"));
    assert_ne!(
        settings.sources[0].account_id,
        settings.sources[1].account_id
    );
    assert_eq!(settings.sources[0].id, "source");
    assert_eq!(settings.sources[0].name, "Antigravity · 本机");
    let key = &settings.account_aliases["agy:account"];
    let (id, cost, account): (String, f64, String) = store
        .db
        .query_row("SELECT id,cost,account_id FROM events", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(id, event.id);
    assert_eq!(cost, 1.0);
    assert_eq!(key, &format!("antigravity:{account}"));
    let copy = Source {
        id: "new-copy".into(),
        ..settings.sources[0].clone()
    };
    for s in [&settings.sources[0], &copy] {
        let event = usage::parse(s, &mut ParseState::default(), &row).unwrap();
        assert!(!store.put_event(&event, &[], &settings).unwrap());
    }
    let raw: String = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='wakeup:keep-task'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let wake: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(wake["task"]["id"], "keep-task");
    assert_eq!(wake["deployment"]["source"]["accountId"], account);
    assert_eq!(
        store.quota_order().unwrap(),
        vec![key.clone(), "antigravity:account".into()]
    );
    let quotas: Vec<String> = store
        .db
        .prepare("SELECT payload FROM quotas ORDER BY account_key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(quotas.len(), 2);
    assert!(
        quotas
            .iter()
            .all(|raw| serde_json::from_str::<Value>(raw).unwrap()["provider"] == "antigravity")
    );
    let before = serde_json::to_value(&settings).unwrap();
    drop(store);
    assert_eq!(
        serde_json::to_value(Store::open(temp.path()).unwrap().settings().unwrap()).unwrap(),
        before
    );
}

#[test]
fn group_estimates_isolate_cost_capacity_and_reject_unknown_models() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let source = source(temp.path());
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    store
        .save_prices(&[price("gemini-test"), price("claude-test"), price("unknown")])
        .unwrap();
    let at = now() - 100;
    let mut q = QuotaSnapshot {
        provider: "antigravity".into(),
        account_id: "account".into(),
        source_id: source.id.clone(),
        origin: "live".into(),
        updated_at: at,
        ..Default::default()
    };
    for group in ["gemini", "claude-gpt"] {
        for minutes in [300, 10080] {
            q.windows.push(QuotaWindow {
                id: format!("{group}:{minutes}"),
                group_id: group.into(),
                name: format!("{group} {minutes}"),
                window_minutes: Some(minutes),
                used_percent: 10.0,
                resets_at: Some(at + minutes * 60),
                ..Default::default()
            });
        }
    }
    store.quota(&q).unwrap();
    let estimate = store
        .begin_estimate(&q, "gemini:300", vec![source.id.clone()], None)
        .unwrap();
    assert_eq!(estimate.group_id.as_deref(), Some("gemini"));
    assert!(
        store
            .begin_estimate(&q, "claude-gpt:300", vec![source.id.clone()], None)
            .is_err()
    );
    for (model, tokens) in [("gemini-test", 100), ("claude-test", 900)] {
        let e=usage::parse(&source,&mut ParseState::default(),&json!({"id":model,"timestamp":at+10,"intervalStart":at+1,"intervalEvidence":"antigravity-step-v1","model":model,"tokens":{"input":tokens}})).unwrap();
        store
            .put_event(&e, &store.prices().unwrap(), &store.settings().unwrap())
            .unwrap();
    }
    q.updated_at += 50;
    for (w, delta) in q.windows.iter_mut().zip([10.0, 2.0, 20.0, 2.0]) {
        w.used_percent += delta;
    }
    store.quota(&q).unwrap();
    let done = store.finish_estimate(&estimate.id).unwrap();
    assert_eq!(done.total_tokens, 100);
    assert_eq!(done.cost, 1.0);
    assert_eq!(done.five_hour_value, Some(10.0));
    assert_eq!(
        store
            .capacity_for_group("antigravity:account", "gemini", now())
            .unwrap()
            .ratio,
        Some(5.0)
    );
    assert_eq!(
        store
            .capacity_for_group("antigravity:account", "claude-gpt", now())
            .unwrap()
            .ratio,
        Some(10.0)
    );
    let e = store
        .begin_estimate(&q, "gemini:300", vec![source.id.clone()], None)
        .unwrap();
    let unknown=usage::parse(&source,&mut ParseState::default(),&json!({"id":"unknown","timestamp":q.updated_at+1,"model":"unknown","tokens":{"input":100}})).unwrap();
    store
        .put_event(
            &unknown,
            &store.prices().unwrap(),
            &store.settings().unwrap(),
        )
        .unwrap();
    q.updated_at += 10;
    q.windows[0].used_percent += 10.0;
    store.quota(&q).unwrap();
    let done = store.finish_estimate(&e.id).unwrap();
    assert_eq!(done.calculation_status, "unmappedGroup");
    assert_eq!(done.five_hour_value, None);
}
