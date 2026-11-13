use super::*;
use crate::{
    models::{Settings, Source, now},
    sessions::{ControlProcess, LiveSession, Phase, Snapshot, SourceHealth},
};
use prost::Message;
use rusqlite::params;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::mpsc,
    time::Duration,
};

const NATIVE: &str = "11111111-2222-4333-8444-555555555555";
const CHILD: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

fn fixture() -> (tempfile::TempDir, Store, Source) {
    #[cfg(unix)]
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    #[cfg(not(unix))]
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("data")).unwrap();
    let root = dir.path().canonicalize().unwrap().join("codex");
    std::fs::create_dir(&root).unwrap();
    let source = Source {
        id: "fixture-source".into(),
        provider: "codex".into(),
        path: root.to_string_lossy().into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    (dir, store, source)
}
fn enabled(store: &Store, paired: bool) -> Config {
    let c = Config {
        enabled: true,
        app_id: "cli_fixture".into(),
        secret_ref: "fixture-reference".into(),
        epoch: "fixture-epoch".into(),
        revision: 1,
        owner: paired.then(owner),
        allow_terminal: false,
    };
    config::write(&store.db, &c).unwrap();
    c
}
fn owner() -> Owner {
    Owner {
        open_id: "ou_owner".into(),
        chat_id: "oc_owner".into(),
        tenant_key: "tenant".into(),
    }
}
fn live(source: &Source, phase: Phase) -> LiveSession {
    LiveSession {
        id: "fixture-session".into(),
        source_id: source.id.clone(),
        provider: source.provider.clone(),
        native_session: NATIVE.into(),
        machine_id: source.host_id.clone().unwrap_or_else(|| "local".into()),
        machine: "Fixture".into(),
        turn_id: "turn-one".into(),
        turn_started_at: (now() * 1000 - 10_000) as u64,
        updated_at: now() as u64,
        event_at: (now() * 1000) as u64,
        phase,
        ..Default::default()
    }
}
fn observe(store: &Store, s: LiveSession) -> Session {
    let source = SourceHealth {
        source_id: s.source_id.clone(),
        machine_id: s.machine_id.clone(),
        state: "ready".into(),
        checked_at: now() as u64,
        ..Default::default()
    };
    db::observe(
        store,
        &Snapshot {
            sessions: vec![s.clone()],
            sources: vec![source],
            ..Default::default()
        },
    )
    .unwrap();
    db::session(&store.db, &configuration(&store.db).unwrap().epoch, &s.id)
        .unwrap()
        .unwrap()
}
fn incoming(id: &str, text: &str, parent: &str) -> Value {
    json!({"header":{"app_id":"cli_fixture","event_type":"im.message.receive_v1","tenant_key":"tenant"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_owner"},"tenant_key":"tenant"},"message":{"message_id":id,"create_time":(now()*1000).to_string(),"chat_type":"p2p","chat_id":"oc_owner","message_type":"text","content":json!({"text":text}).to_string(),"parent_id":parent}}})
}
fn ingest(store: &Store, event: Value) {
    db::ingest(store, &configuration(&store.db).unwrap().epoch, &event).unwrap();
    db::process_inbox(store).unwrap();
}
fn count(store: &Store, table: &str) -> i64 {
    store
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn save_rows(path: &Path, rows: &[Value]) {
    std::fs::write(
        path,
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
}
fn records(target: &Session) -> Vec<Value> {
    let at = target.session.turn_started_at + 1000;
    vec![
        json!({"type":"session_meta","payload":{"id":NATIVE,"cwd":"/fixture"}}),
        json!({"timestamp":at,"type":"turn_context","payload":{"turn_id":"turn-one","cwd":"/fixture"}}),
        json!({"timestamp":at+1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Continue precisely"}]}}),
        json!({"timestamp":at+2,"type":"response_item","payload":{"type":"reasoning","summary":[{"text":"PRIVATE REASONING"}]}}),
        json!({"timestamp":at+3,"type":"response_item","payload":{"type":"function_call_output","output":"PRIVATE TOOL RESULT"}}),
        json!({"timestamp":at+4,"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"PRIVATE PROGRESS"}]}}),
        json!({"timestamp":at+5,"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final","content":[{"type":"output_text","text":"Final answer"}]}}),
    ]
}

#[test]
fn disabled_channel_collects_no_content_and_returns_no_credentials() {
    let (_dir, store, source) = fixture();
    let s = live(&source, Phase::NeedsInput);
    db::observe(
        &store,
        &Snapshot {
            sessions: vec![s],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(count(&store, "channel_sessions"), 0);
    assert_eq!(count(&store, "channel_events"), 0);
    db::ingest(&store, "", &incoming("message", "/pair test", "")).unwrap();
    assert_eq!(count(&store, "channel_inbox"), 0);
    let status = config::status(&store).unwrap();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["connection"]["state"], "disabled");
    assert!(!status.to_string().contains("secretRef"));
}

#[test]
fn credentials_live_only_in_keyring_and_edits_require_revision() {
    let (_dir, store, _) = fixture();
    let params = json!({"appId":"cli_fixture","appSecret":"private-fixture-secret","enabled":true,"revision":0,"allowTerminal":true});
    let status = config::save(&store, &params).unwrap();
    let c = configuration(&store.db).unwrap();
    assert_eq!(
        crate::credentials::read_channel(&c.secret_ref).unwrap(),
        "private-fixture-secret"
    );
    assert_eq!(status["hasSecret"], true);
    assert!(!status.to_string().contains("private-fixture-secret"));
    assert!(
        !store
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [config::KEY], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
            .contains("private-fixture-secret")
    );
    assert!(config::save(&store, &params).is_err());
    let next = config::save(
        &store,
        &json!({"appId":"cli_fixture","appSecret":"","enabled":false,"revision":c.revision}),
    )
    .unwrap();
    assert_eq!(next["hasSecret"], true);
    assert_eq!(configuration(&store.db).unwrap().secret_ref, c.secret_ref);
    crate::credentials::delete_channel(&c.secret_ref).unwrap();
}

#[test]
fn pairing_is_single_owner_bounded_and_publishes_current_questions() {
    let (_dir, store, source) = fixture();
    enabled(&store, false);
    let row = observe(&store, live(&source, Phase::NeedsInput));
    assert_eq!(count(&store, "channel_events"), 0);
    let pair = config::begin_pairing(&store).unwrap();
    ingest(
        &store,
        incoming("pair", pair["message"].as_str().unwrap(), ""),
    );
    let mut c = configuration(&store.db).unwrap();
    assert_eq!(c.owner, Some(owner()));
    assert_eq!(count(&store, "channel_events"), 1);
    assert!(config::begin_pairing(&store).is_err());
    assert!(
        !config::accept_pair(
            &store.db,
            &mut c,
            "x",
            Owner {
                open_id: "attacker".into(),
                ..owner()
            }
        )
        .unwrap()
    );
    assert_eq!(
        db::session(&store.db, &c.epoch, &row.key)
            .unwrap()
            .unwrap()
            .session
            .id,
        row.session.id
    );
    assert!(config::unpair(&store, &json!({"revision":0})).is_err());
    config::unpair(&store, &json!({"revision":c.revision})).unwrap();
    assert!(configuration(&store.db).unwrap().owner.is_none());
}

#[test]
fn pair_attempts_and_expiration_are_enforced() {
    let (_dir, store, _) = fixture();
    let mut c = enabled(&store, false);
    let pair = config::begin_pairing(&store).unwrap();
    for _ in 0..5 {
        assert!(!config::accept_pair(&store.db, &mut c, "wrong", owner()).unwrap());
    }
    assert!(
        !config::accept_pair(&store.db, &mut c, pair["code"].as_str().unwrap(), owner()).unwrap()
    );
    let pair = config::begin_pairing(&store).unwrap();
    store
        .db
        .execute(
            "UPDATE kv SET value=json_set(value,'$.expires_at',?1) WHERE key=?2",
            params![now() - 1, config::PAIR_KEY],
        )
        .unwrap();
    assert!(
        !config::accept_pair(&store.db, &mut c, pair["code"].as_str().unwrap(), owner()).unwrap()
    );
}

#[test]
fn ingress_rejects_foreign_sender_app_group_type_and_stale_messages() {
    let (_dir, store, _) = fixture();
    let c = enabled(&store, true);
    let good = incoming("valid", "/help", "");
    let mutations = [
        ("/header/app_id", json!("cli_other")),
        ("/header/event_type", json!("unrelated")),
        ("/event/sender/sender_id/open_id", json!("ou_other")),
        ("/event/sender/tenant_key", json!("other")),
        ("/event/sender/sender_type", json!("app")),
        ("/event/message/chat_type", json!("group")),
        ("/event/message/chat_id", json!("oc_other")),
        ("/event/message/message_type", json!("image")),
        (
            "/event/message/create_time",
            json!(((now() - 301) * 1000).to_string()),
        ),
        (
            "/event/message/content",
            json!(json!({"text":"bad\u{1b}[31m"}).to_string()),
        ),
        (
            "/event/message/content",
            json!(json!({"text":"x".repeat(db::MAX_INPUT+1)}).to_string()),
        ),
    ];
    for (path, value) in mutations {
        let mut event = good.clone();
        *event.pointer_mut(path).unwrap() = value;
        db::ingest(&store, &c.epoch, &event).unwrap();
    }
    assert_eq!(count(&store, "channel_inbox"), 0);
    db::ingest(&store, "old-epoch", &good).unwrap();
    assert_eq!(count(&store, "channel_inbox"), 0);
    ingest(&store, good.clone());
    ingest(&store, good);
    assert_eq!(count(&store, "channel_inbox"), 1);
    assert_eq!(count(&store, "channel_outbox"), 1);
    let raw: String = store
        .db
        .query_row("SELECT payload FROM channel_inbox", [], |r| r.get(0))
        .unwrap();
    assert_eq!(raw, "{}");
}

#[test]
fn quote_routes_to_bound_session_and_unknown_quotes_never_use_selection() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let a = observe(&store, live(&source, Phase::Complete));
    let mut next = live(&source, Phase::Complete);
    next.id = "second".into();
    next.native_session = CHILD.into();
    let b = observe(&store, next);
    ingest(&store, incoming("select", &format!("/use {}", b.key), ""));
    db::outbox(
        &store.db,
        &c,
        "",
        Some((&a.session.id, &a.state_key)),
        "Question",
    )
    .unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET sent_id='quoted-a' WHERE body='Question'",
            [],
        )
        .unwrap();
    ingest(&store, incoming("reply", "Reply to A", "quoted-a"));
    ingest(
        &store,
        incoming("unknown", "Must not send to B", "unrecognized"),
    );
    let commands = db::command_rows(&store.db, &c.epoch, None).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].session_id, a.session.id);
    assert_eq!(commands[0].expected, a.state_key);
    ingest(
        &store,
        incoming("direct", &format!("/send {} Follow B", b.key), ""),
    );
    assert_eq!(
        db::command_rows(&store.db, &c.epoch, None).unwrap()[0].session_id,
        b.session.id
    );
}

#[test]
fn changed_turn_blocks_before_submission_and_cannot_be_retried() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let mut s = live(&source, Phase::Complete);
    let row = observe(&store, s.clone());
    ingest(
        &store,
        incoming("command", &format!("/send {} Hello", row.key), ""),
    );
    s.turn_id = "new-turn".into();
    observe(&store, s);
    control::process_command(&store).unwrap();
    let command = &db::command_rows(&store.db, &c.epoch, None).unwrap()[0];
    assert_eq!(command.status, "blocked");
    assert_eq!(
        command.reason,
        "会话或问题已改变，请回复最新通知或发送新指令"
    );
    assert!(db::retry(&store, &json!({"id":command.id})).is_err());
    db::cancel(&store, &json!({"id":command.id})).unwrap();
    assert_eq!(
        db::command_rows(&store.db, &c.epoch, None).unwrap()[0].status,
        "cancelled"
    );
}

#[test]
fn restart_does_not_replay_dispatching_and_uncertain_cannot_retry() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let row = observe(&store, live(&source, Phase::Complete));
    ingest(
        &store,
        incoming("command", &format!("/send {} Hello", row.key), ""),
    );
    store
        .db
        .execute("UPDATE channel_commands SET status='dispatching'", [])
        .unwrap();
    control::recover(&store).unwrap();
    let command = &db::command_rows(&store.db, &c.epoch, None).unwrap()[0];
    assert_eq!(command.status, "uncertain");
    assert!(db::retry(&store, &json!({"id":command.id})).is_err());
    assert!(db::cancel(&store, &json!({"id":command.id})).is_err());
    let out = count(&store, "channel_outbox");
    control::recover(&store).unwrap();
    assert_eq!(count(&store, "channel_outbox"), out);
}

#[test]
fn submitted_commands_time_out_when_target_or_source_is_unavailable() {
    for unavailable in ["session", "source", "paused"] {
        let (_dir, store, source) = fixture();
        let c = enabled(&store, true);
        let row = observe(&store, live(&source, Phase::Complete));
        ingest(
            &store,
            incoming("command", &format!("/send {} Hello", row.key), ""),
        );
        store
            .db
            .execute(
                "UPDATE channel_commands SET status='submitted',attempted_at=?1",
                [now()],
            )
            .unwrap();
        if unavailable == "session" {
            store
                .db
                .execute("DELETE FROM channel_sessions", [])
                .unwrap();
        } else {
            let mut settings = store.settings().unwrap();
            if unavailable == "source" {
                settings.sources.clear();
            } else {
                settings.sources[0].enabled = false;
            }
            store.save_settings(&settings).unwrap();
        }
        control::confirm(&store).unwrap();
        assert_eq!(
            db::command_rows(&store.db, &c.epoch, None).unwrap()[0].status,
            "submitted"
        );
        store
            .db
            .execute("UPDATE channel_commands SET attempted_at=?1", [now() - 61])
            .unwrap();
        control::confirm(&store).unwrap();
        let command = &db::command_rows(&store.db, &c.epoch, None).unwrap()[0];
        assert_eq!(command.status, "uncertain", "{unavailable}");
        assert!(db::retry(&store, &json!({"id":command.id})).is_err());
        let out = count(&store, "channel_outbox");
        control::confirm(&store).unwrap();
        assert_eq!(count(&store, "channel_outbox"), out);
    }
}

#[test]
fn confirmation_checks_oldest_before_limiting_and_counts_pending_receipts() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let row = observe(&store, live(&source, Phase::Complete));
    // A backlog larger than the review page must not hide its oldest submissions.
    for index in 0..105 {
        store.db.execute("INSERT INTO channel_commands(id,epoch,session_id,expected,body,reply_to,at,status,attempted_at) VALUES(?1,?2,?3,?4,'Unconfirmed','',?5,'submitted',?5)",params![format!("c{index:03}"),c.epoch,row.session.id,row.state_key,now()-200+index]).unwrap();
    }
    control::confirm(&store).unwrap();
    let uncertain = db::command_rows(&store.db, &c.epoch, Some("uncertain")).unwrap();
    assert_eq!(
        uncertain.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["c004", "c003", "c002", "c001", "c000"]
    );
    ingest(
        &store,
        incoming("overflow", &format!("/send {} Extra", row.key), ""),
    );
    assert_eq!(count(&store, "channel_commands"), 105);
    control::confirm(&store).unwrap();
    ingest(
        &store,
        incoming("accepted", &format!("/send {} Extra", row.key), ""),
    );
    assert_eq!(count(&store, "channel_commands"), 106);
}

#[test]
fn confirmation_requires_matching_input_after_submission() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let mut s = live(&source, Phase::Complete);
    let path = Path::new(&source.path).join("parent.jsonl");
    s.workspace = Some("/fixture".into());
    s.path = path.to_string_lossy().into();
    let row = observe(&store, s);
    save_rows(&path, &records(&row));
    ingest(
        &store,
        incoming(
            "first",
            &format!("/send {} Continue precisely", row.key),
            "",
        ),
    );
    store
        .db
        .execute(
            "UPDATE channel_commands SET status='submitted',attempted_at=?1",
            [now() - 20],
        )
        .unwrap();
    control::confirm(&store).unwrap();
    assert_eq!(
        db::command_rows(&store.db, &c.epoch, None).unwrap()[0].status,
        "confirmed"
    );
    ingest(
        &store,
        incoming(
            "second",
            &format!("/send {} Continue precisely", row.key),
            "",
        ),
    );
    store
        .db
        .execute(
            "UPDATE channel_commands SET status='submitted',attempted_at=?1 WHERE status='queued'",
            [now()],
        )
        .unwrap();
    control::confirm(&store).unwrap();
    assert_eq!(
        db::command_rows(&store.db, &c.epoch, None).unwrap()[0].status,
        "submitted"
    );
}

#[test]
fn work_alias_keeps_source_machine_and_remote_process_identity() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let mut s = live(&source, Phase::Complete);
    s.provider = "chatgpt-work".into();
    let mut row = observe(&store, s);
    assert!(control::source(&store.settings().unwrap(), &row).is_ok());
    row.session.source_id = "another".into();
    assert!(control::source(&store.settings().unwrap(), &row).is_err());
    row.session.source_id = source.id;
    row.session.machine_id = "another-host".into();
    assert!(control::source(&store.settings().unwrap(), &row).is_err());
    row.session.id = "remote-session".into();
    row.session.control_process = Some(ControlProcess {
        pid: 333,
        start: 444,
    });
    row.session.pid = None;
    row.session.process_start = None;
    let wire = serde_json::to_value(&row.session).unwrap();
    assert!(wire.get("pid").is_none());
    assert!(wire.get("controlProcess").is_none());
    row = observe(&store, row.session);
    assert_eq!(
        row.process,
        Some(ControlProcess {
            pid: 333,
            start: 444
        })
    );
}

#[test]
fn excerpts_ignore_reasoning_tools_children_other_turns_and_partial_jsonl() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let mut row = observe(&store, live(&source, Phase::Complete));
    row.session.workspace = Some("/fixture".into());
    let path = Path::new(&source.path).join("parent.jsonl");
    let child = Path::new(&source.path).join("child.jsonl");
    let mut values = records(&row);
    save_rows(&path, &values);
    let mut child_values = values.clone();
    child_values[0]["payload"]["id"] = json!(CHILD);
    child_values.last_mut().unwrap()["payload"]["content"][0]["text"] = json!("CHILD PRIVATE");
    save_rows(&child, &child_values);
    row.path = path.to_string_lossy().into();
    row.log_paths = vec![child.to_string_lossy().into()];
    let excerpt = content::local(&source, &row);
    assert_eq!(excerpt.text, "Final answer");
    assert_eq!(excerpt.user_text, "Continue precisely");
    assert!(!serde_json::to_string(&excerpt).unwrap().contains("PRIVATE"));
    content::verify_codex_record(&source, &row).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"type\":")
        .unwrap();
    assert_eq!(content::local(&source, &row).text, "Final answer");
    assert!(content::verify_codex_record(&source, &row).is_err());
    values.push(json!({"type":"turn_context","timestamp":now()*1000,"payload":{"turn_id":"turn-two","cwd":"/fixture"}}));
    values.push(json!({"type":"response_item","timestamp":now()*1000+1,"payload":{"type":"message","role":"assistant","phase":"final","content":[{"type":"output_text","text":"NEXT PRIVATE"}]}}));
    save_rows(&path, &values);
    assert_eq!(content::local(&source, &row).text, "Final answer");
    assert!(content::verify_codex_record(&source, &row).is_err());
    row.path = child.to_string_lossy().into();
    assert!(content::local(&source, &row).text.is_empty());
}

#[test]
fn observer_does_not_handoff_child_transcripts() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let mut s = live(&source, Phase::Complete);
    s.subagent_count = 1;
    s.log_paths = vec!["parent".into(), "child".into()];
    s.usage_members = vec![
        crate::session_usage::UsageMember {
            source_id: source.id.clone(),
            native_session: NATIVE.into(),
            log_paths: vec!["parent".into()],
        },
        crate::session_usage::UsageMember {
            source_id: source.id,
            native_session: CHILD.into(),
            log_paths: vec!["child".into()],
        },
    ];
    assert_eq!(observe(&store, s).log_paths, vec!["parent"]);
}

fn question() -> Value {
    json!({"id":7,"method":"item/tool/requestUserInput","params":{"threadId":NATIVE,"turnId":"turn-one","itemId":"item","questions":[{"id":"choice","question":"Choose","options":[{"label":"One"},{"label":"Two"}]}]}})
}

#[test]
fn approval_commands_require_the_specific_delivered_request_notification() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let row = observe(&store, live(&source, Phase::NeedsInput));
    ingest(&store, incoming("select", &format!("/use {}", row.key), ""));
    db::outbox_raw(
        &store.db,
        &c,
        "",
        Some((&row.session.id, &row.state_key)),
        "Approval details",
    )
    .unwrap();
    let outgoing: String = store
        .db
        .query_row(
            "SELECT id FROM channel_outbox WHERE rowid=last_insert_rowid()",
            [],
            |r| r.get(0),
        )
        .unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET sent_id='om_approval' WHERE id=?1",
            [&outgoing],
        )
        .unwrap();
    store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2)", params![format!("channel.prompt:{}",row.session.id),json!({"hash":"request","expected":row.state_key,"approval":true,"outboxId":outgoing}).to_string()]).unwrap();
    db::outbox_raw(
        &store.db,
        &c,
        "",
        Some((&row.session.id, &row.state_key)),
        "Status receipt",
    )
    .unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET sent_id='om_receipt' WHERE rowid=last_insert_rowid()",
            [],
        )
        .unwrap();
    for (id, body, parent) in [
        ("bare", "/approve".into(), ""),
        ("send", format!("/send {} /approve", row.key), "om_approval"),
        ("receipt", "/approve".into(), "om_receipt"),
    ] {
        ingest(&store, incoming(id, &body, parent));
    }
    assert_eq!(count(&store, "channel_commands"), 0);
    ingest(&store, incoming("approve", "/approve", "om_approval"));
    assert_eq!(count(&store, "channel_commands"), 1);
}

#[test]
fn configuration_invalidation_keeps_uncertain_results_for_local_review() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let row = observe(&store, live(&source, Phase::Complete));
    ingest(
        &store,
        incoming("first", &format!("/send {} first", row.key), ""),
    );
    ingest(
        &store,
        incoming("second", &format!("/send {} second", row.key), ""),
    );
    store
        .db
        .execute(
            "UPDATE channel_commands SET status='submitted' WHERE body='first'",
            [],
        )
        .unwrap();
    config::unpair(&store, &json!({"revision":c.revision})).unwrap();
    let rows = db::command_list(&store).unwrap();
    assert_eq!(rows["commands"].as_array().unwrap().len(), 2);
    assert_eq!(rows["commands"][0]["status"], "cancelled");
    assert_eq!(rows["commands"][1]["status"], "uncertain");
    assert_eq!(count(&store, "channel_outbox"), 0);
}

#[test]
fn question_identity_ignores_delivery_timers_and_approvals_are_single_use() {
    let request = question();
    let mut replay = request.clone();
    replay["id"] = json!(123);
    replay["params"]["autoResolutionMs"] = json!(10000);
    assert_eq!(codex::fingerprint(&request), codex::fingerprint(&replay));
    replay["params"]["questions"][0]["question"] = json!("Changed");
    assert_ne!(codex::fingerprint(&request), codex::fingerprint(&replay));
    assert_eq!(
        codex::answer(&request, "2").unwrap()["answers"]["choice"]["answers"],
        json!(["Two"])
    );
    let approval = json!({"method":"item/commandExecution/requestApproval","params":{"availableDecisions":["accept","acceptForSession","decline","cancel"]}});
    assert!(codex::answer(&approval, "yes").is_err());
    assert!(codex::answer(&approval, "acceptForSession").is_err());
    assert_eq!(
        codex::answer(&approval, "/approve").unwrap(),
        json!({"decision":"accept"})
    );
    let mut multi = request.clone();
    multi["params"]["questions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"detail","question":"Describe"}));
    assert!(codex::answer(&multi, "2").is_err());
    assert_eq!(
        codex::answer(&multi, "choice=2\ndetail=Keep the old session").unwrap()["answers"]["detail"]
            ["answers"],
        json!(["Keep the old session"])
    );
}

#[test]
fn protobuf_fragment_reassembly_and_ack_match_feishu_envelope() {
    let mut a = protocol::Assembler::default();
    let frame = |seq, payload: &[u8]| protocol::Frame {
        method: 1,
        seq_id: 99,
        log_id: 11,
        service: 7,
        headers: vec![
            protocol::Header {
                key: "type".into(),
                value: "event".into(),
            },
            protocol::Header {
                key: "message_id".into(),
                value: "event-one".into(),
            },
            protocol::Header {
                key: "sum".into(),
                value: "2".into(),
            },
            protocol::Header {
                key: "seq".into(),
                value: format!("{seq}"),
            },
        ],
        payload: Some(payload.to_vec()),
        ..Default::default()
    };
    let second = frame(1, b"true}");
    assert!(a.payload(&second).unwrap().is_none());
    assert!(a.payload(&second).unwrap().is_none());
    assert_eq!(
        a.payload(&frame(0, b"{\"ok\":")).unwrap().unwrap(),
        b"{\"ok\":true}"
    );
    let decoded = protocol::Frame::decode(second.acknowledge(200).as_slice()).unwrap();
    assert_eq!(decoded.seq_id, 99);
    assert_eq!(decoded.service, 7);
    assert_eq!(decoded.header("biz_rt"), "0");
    assert_eq!(
        serde_json::from_slice::<Value>(decoded.payload.as_ref().unwrap()).unwrap()["code"],
        200
    );
    assert!(a.payload(&frame(2, b"invalid")).is_err());
    assert!(
        a.payload(&frame(0, &vec![0; protocol::MAX_PAYLOAD + 1]))
            .is_err()
    );
    a.payload(&frame(0, b"old")).unwrap();
    assert!(a.payload(&frame(0, b"conflict")).is_err());
}

fn http_receiver(
    codes: Vec<u16>,
) -> (
    String,
    mpsc::Receiver<(String, Value)>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        for code in codes {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                stream.read_exact(&mut b).unwrap();
                head.push(b[0]);
                assert!(head.len() < 16384);
            }
            let head = String::from_utf8(head).unwrap();
            let size = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap();
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).unwrap();
            sender
                .send((
                    head.lines().next().unwrap().to_string(),
                    serde_json::from_slice(&bytes).unwrap(),
                ))
                .unwrap();
            let body = if code == 200 {
                json!({"code":0,"data":{"message_id":"om_sent"}}).to_string()
            } else {
                "{}".into()
            };
            write!(stream,"HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    (format!("http://{address}"), receiver, handle)
}

#[test]
fn feishu_send_retries_same_uuid_and_preserves_quote_route() {
    let (_dir, store, _) = fixture();
    let c = enabled(&store, true);
    let (url, received, server) = http_receiver(vec![500, 200]);
    let mut api = feishu::test_api(url);
    db::outbox(
        &store.db,
        &c,
        "om_parent",
        Some(("session", "state")),
        "Literal <message>\nHello",
    )
    .unwrap();
    feishu::deliver(&store, &c, &mut api).unwrap();
    assert_eq!(config::status(&store).unwrap()["undelivered"], 1);
    assert!(!config::status(&store).unwrap()["deliveryError"].is_null());
    store
        .db
        .execute("UPDATE channel_outbox SET next_attempt=0", [])
        .unwrap();
    feishu::deliver(&store, &c, &mut api).unwrap();
    let first = received.recv().unwrap();
    let second = received.recv().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.0,
        "POST /open-apis/im/v1/messages/om_parent/reply HTTP/1.1"
    );
    assert_eq!(
        serde_json::from_str::<Value>(first.1["content"].as_str().unwrap()).unwrap()["text"],
        "Literal <message>\nHello"
    );
    assert_eq!(first.1["reply_in_thread"], false);
    assert_eq!(config::status(&store).unwrap()["undelivered"], 0);
    server.join().unwrap();
}

#[test]
fn feishu_notification_targets_owner_chat_and_expired_retries_stop() {
    let (_dir, store, _) = fixture();
    let c = enabled(&store, true);
    let (url, received, server) = http_receiver(vec![200]);
    let mut api = feishu::test_api(url);
    db::outbox(&store.db, &c, "", None, "Notice").unwrap();
    feishu::deliver(&store, &c, &mut api).unwrap();
    let (route, body) = received.recv().unwrap();
    assert!(route.contains("receive_id_type=chat_id"));
    assert_eq!(body["receive_id"], "oc_owner");
    server.join().unwrap();
    db::outbox(&store.db, &c, "", None, "Expired").unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET at=?1 WHERE sent_id IS NULL",
            [now() - 3301],
        )
        .unwrap();
    feishu::deliver(&store, &c, &mut api).unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT attempts FROM channel_outbox WHERE sent_id IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[cfg(unix)]
fn codex_server(
    root: &Path,
    changed: bool,
    pending: bool,
) -> (mpsc::Receiver<Value>, std::thread::JoinHandle<()>) {
    use std::os::unix::net::UnixListener;
    let socket = root.join("app-server-control");
    std::fs::create_dir_all(&socket).unwrap();
    let listener = UnixListener::bind(socket.join("app-server-control.sock")).unwrap();
    let (sender, received) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        while let Ok(message) = ws.read() {
            let Ok(text) = message.to_text() else {
                continue;
            };
            let value: Value = serde_json::from_str(text).unwrap();
            sender.send(value.clone()).unwrap();
            let Some(method) = value["method"].as_str() else {
                if value["id"] == 7 {
                    ws.send(tungstenite::Message::Text(json!({"method":"serverRequest/resolved","params":{"threadId":NATIVE,"requestId":7}}).to_string().into())).unwrap();
                }
                continue;
            };
            if method == "initialized" {
                continue;
            }
            let thread = json!({"id":NATIVE,"cwd":"/fixture","status":{"type":if pending{"active"}else{"idle"},"activeFlags":if pending{vec!["waitingOnUserInput"]}else{vec![]}}});
            let result = match method {
                "initialize" => json!({}),
                "thread/read" => json!({"thread":thread}),
                "thread/turns/list" => {
                    json!({"data":[{"id":if changed{"turn-new"}else{"turn-one"}}]})
                }
                "thread/loaded/list" => json!({"data":[NATIVE]}),
                "thread/resume" => json!({"thread":thread}),
                "turn/start" => {
                    json!({"turn":{"id":"turn-new","status":"inProgress","error":null}})
                }
                other => panic!("Unexpected method {other}"),
            };
            ws.send(tungstenite::Message::Text(
                json!({"id":value["id"],"result":result}).to_string().into(),
            ))
            .unwrap();
            if method == "thread/resume" && pending {
                ws.send(tungstenite::Message::Text(question().to_string().into()))
                    .unwrap();
            }
        }
    });
    (received, server)
}

#[test]
#[cfg(unix)]
fn existing_codex_thread_is_verified_and_reused_without_policy_overrides() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let row = observe(&store, live(&source, Phase::Complete));
    let (received, server) = codex_server(Path::new(&source.path), false, false);
    let mut client = codex::Client::open(Path::new(&source.path)).unwrap();
    client.attach(&row, false).unwrap();
    let outcome = client.followup(&row, "Keep this thread");
    assert_eq!(outcome.status, "confirmed");
    drop(client);
    server.join().unwrap();
    let sent = received.try_iter().collect::<Vec<_>>();
    let start = sent.iter().find(|v| v["method"] == "turn/start").unwrap();
    assert_eq!(start["params"]["threadId"], NATIVE);
    assert_eq!(start["params"].as_object().unwrap().len(), 2);
    let resume = sent
        .iter()
        .find(|v| v["method"] == "thread/resume")
        .unwrap();
    assert_eq!(
        resume["params"],
        json!({"threadId":NATIVE,"excludeTurns":true})
    );
}

#[test]
#[cfg(unix)]
fn native_latest_turn_rejects_stale_cached_target_before_resuming() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let row = observe(&store, live(&source, Phase::Complete));
    let (received, server) = codex_server(Path::new(&source.path), true, false);
    let mut client = codex::Client::open(Path::new(&source.path)).unwrap();
    assert!(client.attach(&row, false).is_err());
    drop(client);
    server.join().unwrap();
    assert!(
        received
            .try_iter()
            .all(|v| v["method"] != "thread/resume" && v["method"] != "turn/start")
    );
}

#[test]
#[cfg(unix)]
fn native_pending_question_is_answered_and_resolved() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let row = observe(&store, live(&source, Phase::NeedsInput));
    let (received, server) = codex_server(Path::new(&source.path), false, true);
    let mut client = codex::Client::open(Path::new(&source.path)).unwrap();
    client.attach(&row, true).unwrap();
    let request = client.pending(&row).unwrap().unwrap();
    assert_eq!(client.answer(&row, &request, "2").status, "confirmed");
    drop(client);
    server.join().unwrap();
    let answer = received
        .try_iter()
        .find(|v| v["id"] == 7 && v.get("method").is_none())
        .unwrap();
    assert_eq!(
        answer["result"]["answers"]["choice"]["answers"],
        json!(["Two"])
    );
}

#[test]
fn service_lock_allows_one_consumer_and_disabled_start_stays_offline() {
    let (_dir, store, _) = fixture();
    let service = Service::start(&store.root).unwrap().unwrap();
    assert!(Service::start(&store.root).unwrap().is_none());
    assert_eq!(
        config::status(&store).unwrap()["connection"]["state"],
        "disabled"
    );
    service.stop();
    drop(service);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(next) = Service::start(&store.root).unwrap() {
            next.stop();
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminal_names_never_accept_shell_foreground() {
    for provider in [
        "claude",
        "kimi",
        "pi",
        "omp",
        "opencode",
        "qoder",
        "codebuddy",
        "gemini",
    ] {
        for shell in [
            "sh",
            "bash",
            "zsh",
            "fish",
            "node",
            "python",
            "powershell.exe",
        ] {
            assert!(!control::agent_foreground(provider, shell));
        }
    }
    assert!(control::agent_foreground("claude", "/usr/bin/claude"));
}
