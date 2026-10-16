use super::*;

fn fixture() -> (tempfile::TempDir, Store, Settings, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().canonicalize().unwrap().join("codex");
    std::fs::create_dir(&home).unwrap();
    let source = Source {
        id: "s".into(),
        account_id: "a".into(),
        path: home.to_string_lossy().into(),
        auto_valuation: Some(crate::automatic::AutoValuationConfig {
            enabled: true,
            consent_version: 1,
            mode: "mixed".into(),
        }),
        ..Default::default()
    };
    let connection = Connection {
        id: "fixture".into(),
        source_id: "s".into(),
        name: "Fixture API".into(),
        base_url: "https://example.test/v1".into(),
        model: "fixture-model".into(),
        revision: "one".into(),
        prices: vec![ModelPrice {
            id: "fixture-model".into(),
            input: Some(0.000020),
            output: Some(0.000040),
            cache_read: Some(0.000002),
            ..Default::default()
        }],
        ..Default::default()
    };
    let settings = Settings {
        sources: vec![source],
        accounts: vec![Account {
            id: "a".into(),
            name: "Subscription".into(),
            ..Default::default()
        }],
        codex_apis: vec![connection],
        ..Default::default()
    };
    let store = Store::open(&temporary.path().join("data")).unwrap();
    store.save_settings(&settings).unwrap();
    let root = api_root(&settings.sources[0]).unwrap();
    private_dir(&root).unwrap();
    (temporary, store, settings, root)
}

fn request(id: &str, stamp: i64, status: &str) -> Value {
    json!({"id":id,"connectionId":"fixture","sourceId":"s","timestamp":stamp,"endedAt":stamp+1,"sessionId":"session","turnId":"api-turn","model":"fixture-model","status":status,"responseId":"response-one","usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":10}},"prices":[{"id":"fixture-model","input":0.000020,"output":0.000040,"cacheRead":0.000002}]})
}

fn event(id: &str, stamp: i64, turn: &str, category: &str) -> UsageEvent {
    serde_json::from_value(json!({"id":id,"sourceId":"s","accountId":"a","provider":"codex","sessionId":"session","turnId":turn,"model":"fixture-model","timestamp":stamp,"tokens":{"input":90,"output":20,"cacheRead":10},"attribution":"event","billing":{"category":category,"evidence":"codex-request-v4"},"intervalStart":stamp-1})).unwrap()
}

#[test]
fn mixed_estimates_exclude_api_and_do_not_accept_unknown_intervals() {
    let (_temporary, store, settings, root) = fixture();
    let stamp = now();
    let prices = vec![ModelPrice {
        id: "fixture-model".into(),
        input: Some(0.000001),
        output: Some(0.000002),
        cache_read: Some(0.0000001),
        ..Default::default()
    }];
    store
        .put_event(
            &event("sub", stamp - 8, "sub-turn", "subscription"),
            &prices,
            &settings,
        )
        .unwrap();
    // Even stale subscription metadata must yield to the request's actual route.
    store
        .put_event(
            &event("api", stamp - 5, "api-turn", "subscription"),
            &prices,
            &settings,
        )
        .unwrap();
    gateway::save(&root, &request("request-one", stamp - 6, "completed")).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    let mut sample = crate::estimates::EstimateCommon {
        account_key: "codex:a".into(),
        source_ids: vec!["s".into()],
        started_at: stamp - 10,
        checkpoint_at: stamp,
        ..Default::default()
    };
    store.calculate_usage(&mut sample, None).unwrap();
    assert_eq!(sample.calculation_status, "ready");
    assert_eq!(sample.total_tokens, 120);
    assert_eq!(sample.excluded_api_tokens, 120);
    assert!(
        sample.cost < 0.001,
        "third-party prices must not price subscription samples"
    );
    store
        .put_event(
            &event("unknown", stamp - 2, "unmanaged-turn", "unknown"),
            &prices,
            &settings,
        )
        .unwrap();
    store.calculate_usage(&mut sample, None).unwrap();
    assert_eq!(sample.calculation_status, "unknownBilling");
}

#[test]
fn ledger_cursor_imports_late_completion_and_summary_deduplicates_responses() {
    let (_temporary, store, settings, root) = fixture();
    let stamp = now();
    let mut pending = request("slow", stamp - 900, "pending");
    pending["usage"] = Value::Null;
    gateway::save(&root, &pending).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    gateway::save(&root, &request("recent", stamp - 5, "completed")).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    gateway::save(&root, &request("slow", stamp - 900, "completed")).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT status FROM api_requests WHERE id='slow'", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "completed"
    );
    let summary = store.billing_summary(&Filter::default()).unwrap();
    assert_eq!(summary["apiTokens"], 120);
    assert_eq!(summary["incompleteRequests"], 0);
    assert!((summary["apiCost"].as_f64().unwrap() - 0.00262).abs() < 1e-9);
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let mut missing = request("no-usage", stamp, "incomplete");
    missing["responseId"] = Value::Null;
    missing["usage"] = Value::Null;
    gateway::save(&root, &missing).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    assert_eq!(
        store.billing_summary(&Filter::default()).unwrap()["incompleteRequests"],
        1,
        "ledger changes invalidate the dashboard cache"
    );
    assert!(
        !store
            .api_interval_complete(&["s".into()], stamp - 1, stamp + 2)
            .unwrap()
    );
}

#[test]
fn missing_turn_and_cumulative_catchup_never_prove_subscription_usage() {
    let (_temporary, store, settings, root) = fixture();
    let stamp = now();
    let mut value = request("unmatched", stamp - 1, "completed");
    value["turnId"] = Value::Null;
    gateway::save(&root, &value).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    assert!(
        !store
            .api_interval_complete(&["s".into()], stamp - 3, stamp + 3)
            .unwrap()
    );
    assert_eq!(
        store.billing_summary(&Filter::default()).unwrap()["incompleteRequests"],
        1
    );
    let source = &settings.sources[0];
    let mut state = ParseState::default();
    crate::usage::parse(
        source,
        &mut state,
        &json!({"type":"session_meta","payload":{"id":"session","model_provider":"openai","timestamp":stamp-20}}),
    );
    crate::usage::parse(
        source,
        &mut state,
        &json!({"type":"turn_context","payload":{"turn_id":"continued","model":"fixture-model"}}),
    );
    let parsed=crate::usage::parse(source,&mut state,&json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count","rate_limits":{"plan_type":"plus","primary":{}},"info":{"total_token_usage":{"input_tokens":100,"output_tokens":20},"last_token_usage":{"input_tokens":10,"output_tokens":2}}}})).unwrap();
    assert_eq!(parsed.billing.category, "unknown");
    assert_eq!(parsed.turn_id, "continued");
}

#[test]
fn sse_reader_handles_chunk_boundaries_multiline_and_compaction() {
    let mut reader = gateway::UsageReader::default();
    let data=b"event: response.completed\r\ndata: {\"type\":\"response.completed\",\r\ndata: \"response\":{\"id\":\"r\",\"model\":\"m\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\r\n\r\n";
    for chunk in data.chunks(7) {
        reader.feed(chunk);
    }
    assert!(reader.complete);
    assert_eq!(reader.response_id.as_deref(), Some("r"));
    assert_eq!(
        usage::response_tokens(reader.usage.as_ref().unwrap())
            .unwrap()
            .total(),
        12
    );
    reader.value(&json!({"object":"response.compaction","id":"compact","usage":{"input_tokens":4,"output_tokens":1}}));
    assert!(reader.complete);
    assert_eq!(reader.response_id.as_deref(), Some("compact"));
    assert!(
        usage::response_tokens(
            &json!({"input_tokens":2,"output_tokens":1,"input_tokens_details":{"cached_tokens":3}})
        )
        .is_none()
    );
}

#[test]
fn default_switch_checks_revision_and_preserves_shared_auth_and_unrelated_config() {
    let (_temporary, _store, settings, root) = fixture();
    let source = &settings.sources[0];
    let home = home_from_root(&root).unwrap();
    std::fs::write(home.join("auth.json"), "fixture subscription credentials").unwrap();
    std::fs::write(
        home.join("config.toml"),
        "model='subscription-model'\n[projects.fixture]\ntrust_level='trusted'\n",
    )
    .unwrap();
    let c = &settings.codex_apis[0];
    atomic(
        &root.join("connections/fixture.json"),
        &PrivateConnection {
            connection: c.clone(),
            api_key: "fixture-key".into(),
            local_token: "local-token".into(),
            proxy_environment: Default::default(),
        },
    )
    .unwrap();
    let runtime = Runtime {
        port: 32123,
        runner: root.join("runner"),
    };
    atomic(&root.join("runtime.json"), &runtime).unwrap();
    install_profile(&root, c, &runtime).unwrap();
    let before = config_revision(&root).unwrap();
    local(
        source,
        "default",
        &json!({"id":"fixture","revision":c.revision,"configRevision":before}),
    )
    .unwrap();
    assert!(
        local(
            source,
            "default",
            &json!({"id":"subscription","configRevision":before})
        )
        .is_err()
    );
    let revision = config_revision(&root).unwrap();
    local(
        source,
        "default",
        &json!({"id":"subscription","configRevision":revision}),
    )
    .unwrap();
    assert_eq!(
        config(&root).unwrap()["model"].as_str(),
        Some("subscription-model")
    );
    assert_eq!(
        config(&root).unwrap()["projects"]["fixture"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        "fixture subscription credentials"
    );
    assert!(
        !std::fs::read_to_string(home.join(format!("{}.config.toml", provider(c))))
            .unwrap()
            .contains("fixture-key")
    );
    let mut next = config(&root).unwrap();
    next.get_mut("model_providers")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("unrelated".into(), toml::Value::Table(toml::Table::new()));
    let mut changed = c.clone();
    changed.revision = "two".into();
    select_api(&root, &mut next, &changed, &runtime).unwrap();
    assert!(next["model_providers"].get(provider(c)).is_none());
    assert!(next["model_providers"].get("unrelated").is_some());
    assert!(remove_provider_tables(&mut next, &c.id));
    assert!(next["model_providers"].get(provider(&changed)).is_none());
    assert!(next["model_providers"].get("unrelated").is_some());
    let private: toml::Table = toml::from_str("model_provider='custom'\n[model_providers.custom]\nbase_url='https://example.test/SECRET'\nexperimental_bearer_token='SECRET'\n[model_providers.custom.auth]\ncommand='/resolver/SECRET'\nargs=['SECRET']\n").unwrap();
    assert!(!auth_snapshot(&private).to_string().contains("SECRET"));
}

#[test]
fn legacy_consent_keeps_subscription_mode() {
    let config: crate::automatic::AutoValuationConfig =
        serde_json::from_value(json!({"enabled":true,"consentVersion":1})).unwrap();
    assert!(config.enabled);
    assert_ne!(config.mode, "mixed");
}

#[test]
fn copied_events_correlate_across_memberships_but_costs_follow_the_source_filter() {
    let (_temporary, store, mut settings, root) = fixture();
    let mut copy = settings.sources[0].clone();
    copy.id = "copy".into();
    copy.path += "/sessions";
    settings.sources.push(copy);
    store.save_settings(&settings).unwrap();
    let stamp = now();
    let mut e = event("shared", stamp, "api-turn", "subscription");
    store.put_event(&e, &[], &settings).unwrap();
    e.source_id = "copy".into();
    store.put_event(&e, &[], &settings).unwrap();
    gateway::save(&root, &request("request", stamp, "completed")).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    let summary = store
        .billing_summary(&Filter {
            source_id: Some("copy".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(summary["apiHistoryTokens"], 120);
    assert_eq!(summary["subscriptionTokens"], 0);
    assert_eq!(summary["apiCost"], 0.0);
    assert_eq!(store.effective_billing(&e).unwrap(), "api");
}

#[test]
fn request_evidence_rejects_cumulative_deltas_and_keeps_subscription_resumes() {
    let (_temporary, store, settings, root) = fixture();
    let stamp = now();
    gateway::save(&root, &request("request", stamp, "completed")).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    let mut e = event("catch-up", stamp, "api-turn", "subscription");
    e.attribution = "interval".into();
    e.billing.evidence = "codex-provider".into();
    store.put_event(&e, &[], &settings).unwrap();
    assert_eq!(store.effective_billing(&e).unwrap(), "unknown");
    let mut parse = ParseState::default();
    crate::usage::parse(
        &settings.sources[0],
        &mut parse,
        &json!({"type":"session_meta","payload":{"id":"session","model_provider":"aieyes_api_fixture_one"}}),
    );
    crate::usage::parse(
        &settings.sources[0],
        &mut parse,
        &json!({"type":"turn_context","payload":{"turn_id":"subscription-resume","model":"fixture-model"}}),
    );
    let resumed=crate::usage::parse(&settings.sources[0],&mut parse,&json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count","rate_limits":{"plan_type":"plus","primary":{"used_percent":1}},"info":{"total_token_usage":{"input_tokens":10,"output_tokens":2},"last_token_usage":{"input_tokens":10,"output_tokens":2}}}})).unwrap();
    assert_eq!(store.effective_billing(&resumed).unwrap(), "subscription");
}

#[test]
fn completed_replays_replace_partial_evidence_and_missing_prices_stay_unpriced() {
    let (_temporary, store, settings, root) = fixture();
    let stamp = now();
    let mut partial = request("partial", stamp - 1, "incomplete");
    partial["usage"] = Value::Null;
    gateway::save(&root, &partial).unwrap();
    let mut complete = request("complete", stamp, "completed");
    complete["prices"] = json!([]);
    gateway::save(&root, &complete).unwrap();
    usage::sync(&store, &settings, &settings.sources[0]).unwrap();
    let summary = store.billing_summary(&Filter::default()).unwrap();
    assert_eq!(summary["apiTokens"], 120);
    assert_eq!(summary["apiPricedTokens"], 0);
    assert_eq!(summary["apiCost"], 0.0);
    assert_eq!(summary["incompleteRequests"], 0);
    let e = event("replay", stamp, "api-turn", "subscription");
    store
        .put_event(&e, &settings.codex_apis[0].prices, &settings)
        .unwrap();
    assert_eq!(store.effective_billing(&e).unwrap(), "api");
    assert!(
        store
            .api_interval_complete(&["s".into()], stamp - 5, stamp + 5)
            .unwrap()
    );
    gateway::save(&root, &request("crashed", stamp, "pending")).unwrap();
    gateway::interrupt_pending(&root, "service-restarted").unwrap();
    assert_eq!(
        gateway::records(&root, -1).unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "crashed")
            .unwrap()["status"],
        "incomplete"
    );
}

#[test]
fn api_locations_and_revisions_are_checked_before_any_target_changes() {
    let (temporary, store, settings, _root) = fixture();
    let mut moved = settings.clone();
    moved.sources[0].path += "/elsewhere";
    assert!(
        store
            .save_settings(&moved)
            .unwrap_err()
            .to_string()
            .contains("API")
    );
    let mut removed = settings.clone();
    removed.sources.clear();
    assert!(store.save_settings(&removed).is_err());
    let mut engine = Engine::open(&temporary.path().join("data")).unwrap();
    for action in ["default", "remove"] {
        let error = engine
            .call(
                &format!("codexApi.{action}"),
                json!({"sourceId":"s","id":"fixture","revision":"stale"}),
            )
            .unwrap_err();
        assert!(error.to_string().contains(if action == "default" {
            "检查并确认切换"
        } else {
            "设置已变化"
        }));
    }
    let before = serde_json::to_value(store.settings().unwrap()).unwrap();
    engine.call("autoEstimates.configure",json!({"sourceId":"s","enabled":true,"confirmed":true,"mode":"subscription","baseConfig":before["sources"][0]["autoValuation"]})).unwrap();
    let mut legacy = store.settings().unwrap();
    legacy.sources[0]
        .auto_valuation
        .as_mut()
        .unwrap()
        .mode
        .clear();
    store.save_settings(&legacy).unwrap();
    engine.call("autoEstimates.configure",json!({"sourceId":"s","enabled":false,"baseConfig":{"enabled":true,"consentVersion":1}})).unwrap();
}

#[test]
fn rust_gateway_forwards_streams_and_preserves_interrupted_usage_without_secrets() {
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;
    let (_temporary, _store, settings, root) = fixture();
    for (index, (cut, encoded)) in [(false, false), (true, false), (false, true)]
        .into_iter()
        .enumerate()
    {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let mut connection = settings.codex_apis[0].clone();
        connection.base_url = format!("http://127.0.0.1:{port}/v1");
        connection.proxy.mode = "direct".into();
        atomic(
            &root.join("connections/fixture.json"),
            &PrivateConnection {
                connection,
                api_key: "fixture-upstream-secret".into(),
                local_token: "fixture-local-token".into(),
                proxy_environment: Default::default(),
            },
        )
        .unwrap();
        let upstream_worker = std::thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut lines = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if line.to_lowercase().starts_with("content-length:") {
                    length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
                }
                lines.push_str(&line.to_lowercase());
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            assert!(lines.contains("authorization: bearer fixture-upstream-secret"));
            assert!(!lines.contains("cookie:"));
            if encoded {
                assert!(lines.contains("content-encoding: fixture"));
                assert_eq!(body, b"opaque-compressed-request-fixture");
            }
            let data = format!(
                "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"r-{index}\",\"usage\":{{\"input_tokens\":100,\"output_tokens\":20}}}}}}\r\n\r\n"
            );
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nx-request-id: upstream-fixture\r\nConnection: close\r\n\r\n",data.len()+if cut {20}else{0}).unwrap();
            for bytes in data.as_bytes().chunks(7) {
                stream.write_all(bytes).unwrap();
            }
        });
        let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = gateway_listener.local_addr().unwrap();
        let service_root = root.clone();
        let worker = std::thread::spawn(move || {
            gateway::handle(&service_root, gateway_listener.accept().unwrap().0).unwrap();
        });
        let request = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .post(format!("http://{address}/providers/fixture/v1/responses"))
            .bearer_auth("fixture-local-token")
            .header("Cookie", "fixture-cookie")
            .header(
                "x-codex-turn-metadata",
                r#"{"session_id":"s","turn_id":"t"}"#,
            );
        let request = if encoded {
            request
                .header("Content-Encoding", "fixture")
                .body("opaque-compressed-request-fixture")
        } else {
            request.json(&json!({"model":"fixture-model","input":"PRIVATE-PROMPT"}))
        };
        let result = request.send().unwrap();
        assert_eq!(result.headers()["x-request-id"], "upstream-fixture");
        let _ = result.bytes();
        worker.join().unwrap();
        upstream_worker.join().unwrap();
        let records = gateway::records(&root, -1).unwrap();
        let record = records["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["responseId"] == format!("r-{index}"))
            .unwrap();
        assert_eq!(
            record["status"],
            if cut { "incomplete" } else { "completed" }
        );
        assert_eq!(record["usage"]["output_tokens"], 20);
        if encoded {
            assert_eq!(
                record["model"], "",
                "missing models stay unpriced instead of using the configured default"
            );
        }
        for secret in [
            "fixture-upstream-secret",
            "fixture-local-token",
            "PRIVATE-PROMPT",
            "fixture-cookie",
        ] {
            assert!(!records.to_string().contains(secret));
        }
    }
}
