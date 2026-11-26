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
#[cfg(test)]
fn codex_async_question_receipt_and_current_final_answer_are_readable() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let row = observe(&store, live(&source, Phase::NeedsInput));
    let at = row.session.turn_started_at + 1000;
    let mut values = vec![
        json!({"timestamp":at,"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-one"}}),
        json!({"timestamp":at+1,"type":"response_item","payload":{"type":"function_call","name":"functions.request_user_input_async","call_id":"ask","arguments":json!({"questions":[{"title":"测试问题","options":["收到","没有"]}]}).to_string()}}),
        json!({"timestamp":at+2,"type":"response_item","payload":{"type":"function_call_output","call_id":"ask","output":"{\"accepted\":true}"}}),
        json!({"timestamp":at+3,"type":"response_item","payload":{"type":"function_call_output","call_id":"other","output":"PRIVATE TOOL RESULT"}}),
    ];
    assert_eq!(
        content::parse(values.clone(), &row).question,
        "1: 测试问题\n  1. 收到\n  2. 没有"
    );
    let mut answered = values.clone();
    answered.push(json!({"timestamp":at+4,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",json!([{"answer":"收到","questionItemId":json!(["request_user_input_async","ask",0]).to_string()}]))}]}}));
    assert!(content::parse(answered, &row).question.is_empty());
    values.push(json!({"timestamp":at+4,"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Current final"}]}}));
    assert_eq!(content::parse(values.clone(), &row).text, "Current final");
    values.push(json!({"timestamp":at+5,"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-one","last_agent_message":"Completion final"}}));
    values.push(json!({"timestamp":at+6,"type":"event_msg","payload":{"type":"task_complete","turn_id":"other-turn","last_agent_message":"PRIVATE OTHER TURN"}}));
    assert_eq!(
        content::parse(values.clone(), &row).text,
        "Completion final"
    );
    values[2]["payload"]["output"] = json!("{\"accepted\":false}");
    assert!(content::parse(values, &row).question.is_empty());
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
fn claude_question_cards_and_hook_answers_preserve_original_input() {
    let (_dir, store, mut source) = fixture();
    source.provider = "claude".into();
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let c = enabled(&store, true);
    let mut s = live(&source, Phase::NeedsInput);
    s.input_request = "question-tool".into();
    s.process_start = Some(1);
    s.pid = Some(1);
    let mut target = observe(&store, s);
    let input = json!({"questions":[{"question":"Which system?","header":"System","options":[{"label":"Linux","description":"One"},{"label":"macOS","description":"Two"}],"multiSelect":false}]});
    let at = target.session.turn_started_at + 1000;
    let rows = vec![
        json!({"timestamp":at,"type":"assistant","sessionId":NATIVE,"message":{"role":"assistant","content":[{"type":"tool_use","name":"AskUserQuestion","id":"question-tool","input":input}]}}),
    ];
    let excerpt = content::parse(rows.clone(), &target);
    assert!(excerpt.question.contains("Which system?"));
    assert!(excerpt.claude_request.is_some());
    let request = claude::request(&target, &excerpt).unwrap();
    assert_eq!(request["method"], "aieyes/claudeQuestion");
    assert_eq!(
        request["params"]["questions"][0]["options"][1]["label"],
        "macOS"
    );
    let answer = claude::updated_input(&input, "2").unwrap();
    let result = &answer["hookSpecificOutput"];
    assert_eq!(result["hookEventName"], "PermissionRequest");
    assert_eq!(result["decision"]["behavior"], "allow");
    assert_eq!(
        result["decision"]["updatedInput"]["questions"],
        input["questions"]
    );
    assert_eq!(
        result["decision"]["updatedInput"]["answers"]["Which system?"],
        "macOS"
    );
    assert!(result["decision"].get("updatedPermissions").is_none());
    let card = cards::build("Question", "orange", "Claude", "Question", "Reply");
    db::outbox_card(
        &store.db,
        &c,
        "",
        Some((&target.session.id, &target.state_key)),
        "Question",
        &card,
    )
    .unwrap();
    actions::attach(&store.db, &request).unwrap();
    let payload: String = store
        .db
        .query_row("SELECT payload FROM channel_cards", [], |r| r.get(0))
        .unwrap();
    assert!(payload.contains("macOS"));
    assert!(payload.contains("button"));
    let path = Path::new(&source.path).join("claude.jsonl");
    save_rows(&path, &rows);
    target.path = path.to_string_lossy().into();
    target.log_paths = vec![];
    store
        .db
        .execute(
            "INSERT INTO kv(key,value) VALUES(?1,?2)",
            params![
                format!("channel.prompt:{}", target.session.id),
                json!({"expected":target.state_key,"hash":codex::fingerprint(&request)})
                    .to_string()
            ],
        )
        .unwrap();
    assert_eq!(
        claude::answer(&store, &source, &target, "1").status,
        "blocked"
    );
    let pid = std::process::id();
    let mut system = sysinfo::System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    let start = system
        .process(sysinfo::Pid::from_u32(pid))
        .unwrap()
        .start_time();
    target.process = Some(ControlProcess { pid, start });
    let mut hook_input = input.clone();
    hook_input["additionalField"] = json!("must survive");
    store.db.execute("INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat) VALUES('request',?1,?2,?3,?4,'question-tool',?5,?6,?7,?8,?9)",params![c.epoch,source.id,NATIVE,claude::question_hash(&input),hook_input.to_string(),pid,start,now(),now()-6]).unwrap();
    assert_eq!(
        claude::answer(&store, &source, &target, "2").status,
        "blocked"
    );
    store
        .db
        .execute(
            "UPDATE channel_claude_requests SET heartbeat=?1,call_id='other'",
            [now()],
        )
        .unwrap();
    assert_eq!(
        claude::answer(&store, &source, &target, "2").status,
        "blocked"
    );
    store
        .db
        .execute(
            "UPDATE channel_claude_requests SET call_id='question-tool',process_start=?1",
            [start + 1],
        )
        .unwrap();
    assert_eq!(
        claude::answer(&store, &source, &target, "2").status,
        "blocked"
    );
    store
        .db
        .execute(
            "UPDATE channel_claude_requests SET process_start=?1",
            [start],
        )
        .unwrap();
    let outcome = claude::answer(&store, &source, &target, "2");
    assert_eq!(outcome.status, "submitted");
    assert_eq!(outcome.receipt["adapter"], "claudeHook");
    let response: String = store
        .db
        .query_row("SELECT response FROM channel_claude_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(
        response["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Which system?"],
        "macOS"
    );
    assert_eq!(
        response["hookSpecificOutput"]["decision"]["updatedInput"]["additionalField"],
        "must survive"
    );
    assert_eq!(
        claude::answer(&store, &source, &target, "1").status,
        "blocked"
    );
    let mut answered = rows;
    answered.push(json!({"timestamp":at+1,"type":"user","sessionId":NATIVE,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"question-tool","content":"Answered"}]}}));
    let excerpt = content::parse(answered, &target);
    assert!(excerpt.claude_request.is_none());
    assert!(excerpt.question.is_empty());
    assert_eq!(excerpt.claude_answered_call, "question-tool");
}

#[test]
fn claude_hook_requires_all_questions_and_supports_multiselect_without_approval_changes() {
    let input = json!({"questions":[{"question":"First","options":[{"label":"A"},{"label":"B"}],"multiSelect":true},{"question":"Second","options":[{"label":"X"},{"label":"Y"}],"multiSelect":false}],"extra":"retained"});
    assert!(claude::updated_input(&input, "1=1").is_err());
    let answer = claude::updated_input(&input, "1=1,2\n2=2").unwrap();
    let result = &answer["hookSpecificOutput"]["decision"]["updatedInput"];
    assert_eq!(result["answers"]["First"], "A, B");
    assert_eq!(result["answers"]["Second"], "Y");
    assert_eq!(result["extra"], "retained");
}

#[test]
fn claude_question_hook_install_is_narrow_idempotent_and_preserves_foreign_settings() {
    let (_dir, store, mut source) = fixture();
    source.provider = "claude".into();
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let original = json!({"env":{"SOME_SETTING":"keep"},"permissions":{"defaultMode":"default"},"hooks":{"PreToolUse":[{"matcher":"*","hooks":[{"type":"command","command":"foreign observer"}]}],"PermissionRequest":[{"matcher":"*","hooks":[{"type":"http","url":"http://127.0.0.1:23333/permission","timeout":600}]}]}});
    let path = Path::new(&source.path).join("settings.json");
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    claude::install(&store).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        original
    );
    enabled(&store, true);
    claude::install(&store).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let installed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(installed["env"], original["env"]);
    assert_eq!(installed["permissions"], original["permissions"]);
    assert_eq!(
        installed["hooks"]["PreToolUse"],
        original["hooks"]["PreToolUse"]
    );
    assert_eq!(
        installed["hooks"]["PermissionRequest"][0],
        original["hooks"]["PermissionRequest"][0]
    );
    let own = &installed["hooks"]["PermissionRequest"][1];
    assert_eq!(own["matcher"], "AskUserQuestion");
    assert!(
        own["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--aieyes-claude-question")
    );
    assert_eq!(own["hooks"][0]["timeout"], 86400);
    claude::install(&store).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut implicit = json!({"questions":[{"question":"Choose","options":[{"label":"A"}]}]});
    let hash = claude::question_hash(&implicit);
    implicit["questions"][0]["multiSelect"] = json!(false);
    assert_eq!(claude::question_hash(&implicit), hash);
}

#[test]
fn button_callbacks_are_owner_scoped_current_and_submit_once() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let target = observe(&store, live(&source, Phase::NeedsInput));
    let request = codex::async_request(
        &target,
        &content::Excerpt {
            async_request: Some(
                json!({"callId":"ask","questions":[{"title":"Choose","options":["One","Two"]}]}),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let card = cards::build("等待输入", "orange", "context", "Choose", "Quote to answer");
    db::outbox_card(
        &store.db,
        &c,
        "",
        Some((&target.session.id, &target.state_key)),
        "Choose",
        &card,
    )
    .unwrap();
    actions::attach(&store.db, &request).unwrap();
    let id: String = store
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
            "UPDATE channel_outbox SET sent_id='om_card' WHERE id=?1",
            [&id],
        )
        .unwrap();
    store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2)",params![format!("channel.prompt:{}",target.session.id),json!({"expected":target.state_key,"hash":codex::fingerprint(&request),"approval":false,"outboxId":id}).to_string()]).unwrap();
    let event = json!({"header":{"event_id":"click","app_id":"cli_fixture","event_type":"card.action.trigger","create_time":(now()*1_000_000).to_string(),"tenant_key":"tenant"},"event":{"host":"im_message","operator":{"open_id":"ou_owner","tenant_key":"tenant"},"context":{"open_chat_id":"oc_owner","open_message_id":"om_card"},"action":{"tag":"button","value":{"action":"aieyes.answer","outboxId":id,"questionId":"1","option":0}}}});
    for (path, value) in [
        ("/header/app_id", json!("other")),
        ("/event/operator/open_id", json!("other")),
        ("/event/operator/tenant_key", json!("other")),
        ("/event/context/open_chat_id", json!("other")),
        ("/event/context/open_message_id", json!("other")),
        ("/event/action/value/option", json!(99)),
        ("/event/action/value/option", json!(-1)),
        ("/event/action/value/questionId", json!("other")),
        (
            "/header/create_time",
            json!(((now() - 301) * 1_000_000).to_string()),
        ),
    ] {
        let mut bad = event.clone();
        *bad.pointer_mut(path).unwrap() = value;
        actions::ingest(&store, &c.epoch, &bad).unwrap();
        assert_eq!(count(&store, "channel_commands"), 0);
    }
    actions::ingest(&store, "other-epoch", &event).unwrap();
    assert_eq!(count(&store, "channel_commands"), 0);
    let response = actions::ingest(&store, &c.epoch, &event).unwrap();
    assert_eq!(response["toast"]["type"], "success");
    let buttons = response["card"]["data"]["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["tag"] == "button")
        .collect::<Vec<_>>();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["type"], "primary");
    assert_eq!(buttons[1]["disabled"], true);
    actions::ingest(&store, &c.epoch, &event).unwrap();
    let mut repeated = event.clone();
    repeated["header"]["event_id"] = json!("other-click");
    repeated["event"]["action"]["value"]["option"] = json!(1);
    actions::ingest(&store, &c.epoch, &repeated).unwrap();
    assert_eq!(count(&store, "channel_commands"), 1);
    let result: (String, String) = store
        .db
        .query_row("SELECT body,reply_to FROM channel_commands", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(result, ("1".into(), "om_card".into()));
    let mut s = target.session;
    s.phase = Phase::Complete;
    observe(&store, s);
    assert_eq!(
        actions::ingest(&store, &c.epoch, &repeated).unwrap()["toast"]["type"],
        "info"
    );
}

#[test]
fn multi_question_buttons_wait_for_all_choices_and_never_render_approvals() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let target = observe(&store, live(&source, Phase::NeedsInput));
    let request=codex::async_request(&target,&content::Excerpt{async_request:Some(json!({"callId":"ask","questions":[{"title":"First","options":["A","B"]},{"title":"Second","options":["X","Y"]}]})),..Default::default()}).unwrap();
    let card = cards::build("Question", "orange", "context", "Question", "Footer");
    db::outbox_card(
        &store.db,
        &c,
        "",
        Some((&target.session.id, &target.state_key)),
        "Question",
        &card,
    )
    .unwrap();
    actions::attach(&store.db, &request).unwrap();
    let id: String = store
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
            "UPDATE channel_outbox SET sent_id='om_card' WHERE id=?1",
            [&id],
        )
        .unwrap();
    store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2)",params![format!("channel.prompt:{}",target.session.id),json!({"expected":target.state_key,"hash":codex::fingerprint(&request),"approval":false}).to_string()]).unwrap();
    let mut event = json!({"header":{"event_id":"first","app_id":"cli_fixture","event_type":"card.action.trigger","create_time":(now()*1_000_000).to_string()},"event":{"host":"im_message","operator":{"open_id":"ou_owner","tenant_key":"tenant"},"context":{"open_chat_id":"oc_owner","open_message_id":"om_card"},"action":{"tag":"button","value":{"action":"aieyes.answer","outboxId":id,"questionId":"1","option":1}}}});
    actions::ingest(&store, &c.epoch, &event).unwrap();
    assert_eq!(count(&store, "channel_commands"), 0);
    event["header"]["event_id"] = json!("second");
    event["event"]["action"]["value"]["questionId"] = json!("2");
    event["event"]["action"]["value"]["option"] = json!(0);
    actions::ingest(&store, &c.epoch, &event).unwrap();
    let body: String = store
        .db
        .query_row("SELECT body FROM channel_commands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "1=2\n2=1");
    db::outbox_card(
        &store.db,
        &c,
        "",
        Some((&target.session.id, &target.state_key)),
        "Approval",
        &card,
    )
    .unwrap();
    let mut approval = request;
    approval["method"] = json!("item/commandExecution/requestApproval");
    actions::attach(&store.db, &approval).unwrap();
    assert_eq!(count(&store, "channel_card_actions"), 1);
}

#[test]
fn multiselect_buttons_toggle_until_submit_and_cards_show_option_previews() {
    let (_dir, store, source) = fixture();
    let c = enabled(&store, true);
    let target = observe(&store, live(&source, Phase::NeedsInput));
    let ask = |input: &Value| {
        claude::request(
            &target,
            &content::Excerpt {
                claude_request: Some(json!({"callId":"question-tool","input":input})),
                ..Default::default()
            },
        )
        .unwrap()
    };
    // Deliver a question card and make it the session's current prompt.
    let publish = |request: &Value, message: &str| -> String {
        let card = cards::build("Question", "orange", "context", "Question", "Footer");
        db::outbox_card(
            &store.db,
            &c,
            "",
            Some((&target.session.id, &target.state_key)),
            "Question",
            &card,
        )
        .unwrap();
        actions::attach(&store.db, request).unwrap();
        let id: String = store
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
                "UPDATE channel_outbox SET sent_id=?1 WHERE id=?2",
                params![message, id],
            )
            .unwrap();
        store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("channel.prompt:{}",target.session.id),json!({"expected":target.state_key,"hash":codex::fingerprint(request),"approval":false}).to_string()]).unwrap();
        id
    };
    let mut clicks = 0;
    let mut click = |message: &str, value: Value| {
        clicks += 1;
        let event = json!({"header":{"event_id":format!("click-{clicks}"),"app_id":"cli_fixture","event_type":"card.action.trigger","create_time":(now()*1_000_000).to_string()},"event":{"host":"im_message","operator":{"open_id":"ou_owner","tenant_key":"tenant"},"context":{"open_chat_id":"oc_owner","open_message_id":message},"action":{"tag":"button","value":value}}});
        actions::ingest(&store, &c.epoch, &event).unwrap()
    };
    let buttons = |card: &Value| {
        card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["tag"] == "button")
            .cloned()
            .collect::<Vec<_>>()
    };

    let input = json!({"questions":[
        {"question":"Style?","options":[{"label":"Comprehension","preview":"```python\nsquares = [x * x for x in range(10)]\n```"},{"label":"Loop","preview":"+--+\n|  |\n+--+"}],"multiSelect":false},
        {"question":"Features?","options":[{"label":"A"},{"label":"B"},{"label":"C"}],"multiSelect":true}]});
    let request = ask(&input);
    let id = publish(&request, "om_card");
    let payload: String = store
        .db
        .query_row(
            "SELECT payload FROM channel_cards WHERE outbox_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let card: Value = serde_json::from_str(&payload).unwrap();
    let text = card["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["tag"] == "markdown")
        .map(|e| e["content"].as_str().unwrap())
        .collect::<Vec<_>>();
    // A code preview keeps its own fence; an unfenced mockup is fenced to stay monospace.
    assert!(
        text.contains(
            &"**1. Comprehension**\n```python\nsquares = [x * x for x in range(10)]\n```"
        )
    );
    assert!(text.contains(&"**2. Loop**\n```\n+--+\n|  |\n+--+\n```"));
    assert!(text.contains(&format!("**2: Features?**{}", "（可多选）").as_str()));
    let shown = buttons(&card);
    assert_eq!(shown.len(), 6);
    assert_eq!(shown[5]["behaviors"][0]["value"]["action"], "aieyes.submit");

    let answer = |q: &str, option: usize| json!({"action":"aieyes.answer","outboxId":id,"questionId":q,"option":option});
    let submit = json!({"action":"aieyes.submit","outboxId":id});
    assert_eq!(click("om_card", submit.clone())["toast"]["type"], "warning");
    click("om_card", answer("2", 2));
    click("om_card", answer("2", 0));
    click("om_card", answer("2", 2));
    let toggled = click("om_card", answer("2", 1));
    let shown = buttons(&toggled["card"]["data"]);
    assert_eq!(
        shown.iter().map(|b| &b["type"]).collect::<Vec<_>>(),
        [
            "default", "default", "primary", "primary", "default", "primary"
        ]
    );
    assert_eq!(shown[2]["text"]["content"], "✓ A");
    assert_eq!(click("om_card", submit.clone())["toast"]["type"], "warning");
    // Completing every question does not submit while a multi-select can still change.
    click("om_card", answer("1", 0));
    assert_eq!(count(&store, "channel_commands"), 0);
    let done = click("om_card", submit.clone());
    assert_eq!(done["toast"]["type"], "success");
    assert!(
        buttons(&done["card"]["data"])
            .iter()
            .all(|b| b["disabled"] == true)
    );
    let body: String = store
        .db
        .query_row("SELECT body FROM channel_commands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "1=1\n2=1,2");
    let answers = &claude::updated_input(&input, &body).unwrap()["hookSpecificOutput"]["decision"]
        ["updatedInput"]["answers"];
    assert_eq!(answers["Style?"], "Comprehension");
    assert_eq!(answers["Features?"], "A, B");
    assert_eq!(click("om_card", answer("2", 2))["toast"]["type"], "info");
    assert_eq!(click("om_card", submit)["toast"]["type"], "info");
    assert_eq!(count(&store, "channel_commands"), 1);

    // One multi-select question replies without the `id=` prefix; previews share a bounded budget.
    let large = "x".repeat(6000);
    let single = json!({"questions":[{"question":"Pick","options":[{"label":"A","preview":large},{"label":"B","preview":large},{"label":"C","preview":large},{"label":"D","preview":large}],"multiSelect":true}]});
    let request = ask(&single);
    assert!(
        request["params"]["questions"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["preview"].as_str().unwrap().len() <= 2000)
    );
    let id = publish(&request, "om_second");
    click(
        "om_second",
        json!({"action":"aieyes.answer","outboxId":id,"questionId":"1","option":3}),
    );
    click(
        "om_second",
        json!({"action":"aieyes.answer","outboxId":id,"questionId":"1","option":0}),
    );
    let done = click("om_second", json!({"action":"aieyes.submit","outboxId":id}));
    assert_eq!(buttons(&done["card"]["data"]).len(), 5);
    let body: String = store
        .db
        .query_row(
            "SELECT body FROM channel_commands ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "1,4");
    assert_eq!(
        claude::updated_input(&single, &body).unwrap()["hookSpecificOutput"]["decision"]["updatedInput"]
            ["answers"]["Pick"],
        "A, D"
    );
    // A request without a multi-select question has no submit action.
    let plain =
        ask(&json!({"questions":[{"question":"One","options":[{"label":"A"},{"label":"B"}]}]}));
    let id = publish(&plain, "om_third");
    click("om_third", json!({"action":"aieyes.submit","outboxId":id}));
    assert_eq!(count(&store, "channel_commands"), 2);
}

#[test]
fn callback_ack_encodes_response_bytes_like_the_official_sdk() {
    use base64::Engine;
    let response = json!({"toast":{"type":"success","content":"已选择"}});
    let bytes = protocol::Frame::default().acknowledge_response(200, Some(&response));
    let frame = protocol::Frame::decode(bytes.as_slice()).unwrap();
    let envelope: Value = serde_json::from_slice(frame.payload.as_ref().unwrap()).unwrap();
    assert_eq!(envelope["code"], 200);
    let data = base64::engine::general_purpose::STANDARD
        .decode(envelope["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&data).unwrap(), response);
}

#[test]
#[cfg(test)]
fn card_styles_separate_status_context_body_and_footer_without_changing_text() {
    let card = cards::build(
        "等待输入",
        cards::phase_color(Phase::NeedsInput),
        "s123 · Codex · <workspace>",
        "**问题**\n\n1. 继续\n2. 停止\n\n```rust\nlet x = 1 < 2;\n```",
        "引用此消息回复",
    );
    assert_eq!(card["schema"], "2.0");
    assert_eq!(card["header"]["template"], "orange");
    let elements = card["body"]["elements"].as_array().unwrap();
    assert_eq!(elements[0]["text"]["tag"], "plain_text");
    assert_eq!(elements[0]["text"]["content"], "s123 · Codex · <workspace>");
    assert_eq!(elements[0]["text"]["text_size"], "notation");
    assert_eq!(elements[0]["text"]["text_color"], "grey");
    assert_eq!(elements[1]["tag"], "markdown");
    assert!(
        elements[1]["content"]
            .as_str()
            .unwrap()
            .contains("let x = 1 < 2;")
    );
    assert_eq!(elements[1]["text_size"], "normal");
    assert_eq!(elements[3]["text"]["text_size"], "notation");
    assert_eq!(cards::phase_color(Phase::Complete), "green");
    assert_eq!(cards::phase_color(Phase::Failed), "red");
    assert_eq!(cards::command_color("confirmed"), "green");
    let long = cards::build("完成", "green", "context", &"\\\"".repeat(16000), "提示");
    assert!(long.to_string().len() <= 24000);
    assert!(long.to_string().contains("内容已截断"));
}

#[test]
fn card_delivery_retries_identically_keeps_quote_routing_and_cleans_up_payload() {
    let (_dir, store, _) = fixture();
    let c = enabled(&store, true);
    let card = cards::build("已确认", "green", "c123", "原会话已接收回答", "");
    db::outbox_card(
        &store.db,
        &c,
        "om_parent",
        Some(("session", "state")),
        "Plain audit text",
        &card,
    )
    .unwrap();
    let outbox_id: String = store
        .db
        .query_row(
            "SELECT id FROM channel_outbox WHERE rowid=last_insert_rowid()",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let saved: String = store
        .db
        .query_row(
            "SELECT payload FROM channel_cards WHERE outbox_id=?1",
            [&outbox_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), card);
    let (url, received, server) = http_receiver(vec![500, 200]);
    let mut api = feishu::test_api(url);
    feishu::deliver(&store, &c, &mut api).unwrap();
    store
        .db
        .execute("UPDATE channel_outbox SET next_attempt=0", [])
        .unwrap();
    feishu::deliver(&store, &c, &mut api).unwrap();
    let first = received.recv().unwrap();
    let second = received.recv().unwrap();
    assert_eq!(first, second);
    assert!(first.0.contains("/om_parent/reply"));
    assert_eq!(first.1["msg_type"], "interactive");
    assert_eq!(
        serde_json::from_str::<Value>(first.1["content"].as_str().unwrap()).unwrap(),
        card
    );
    assert_eq!(first.1["reply_in_thread"], false);
    assert_eq!(config::status(&store).unwrap()["undelivered"], 0);
    server.join().unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET at=?1",
            [now() - db::RETENTION - 1],
        )
        .unwrap();
    db::cleanup(&store.db).unwrap();
    assert_eq!(count(&store, "channel_cards"), 0);
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
                "turn/steer" => json!({"turnId":"turn-one"}),
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
    let link = Path::new(&source.path).join("app-server-control/app-server-control.sock");
    let resolved = Path::new(&source.path).join("daemon.sock");
    std::fs::rename(&link, &resolved).unwrap();
    std::os::unix::fs::symlink(&resolved, &link).unwrap();
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
fn question_reference_survives_parallel_activity_but_changes_for_a_new_request() {
    let (_dir, store, source) = fixture();
    enabled(&store, true);
    let mut s = live(&source, Phase::NeedsInput);
    s.input_request = "ask-one".into();
    let first = observe(&store, s.clone());
    s.event_at += 1000;
    s.updated_at += 1;
    let next = observe(&store, s.clone());
    assert_eq!(first.state_key, next.state_key);
    assert_eq!(count(&store, "channel_events"), 1);
    s.input_request = "ask-two".into();
    assert_ne!(first.state_key, observe(&store, s).state_key);
    assert_eq!(count(&store, "channel_events"), 2);
}

#[test]
#[cfg(unix)]
#[cfg(test)]
fn async_answer_uses_the_original_active_turn_with_option_text_and_call_identity() {
    for changed in [false, true] {
        let (_dir, store, source) = fixture();
        enabled(&store, true);
        let row = observe(&store, live(&source, Phase::NeedsInput));
        let excerpt = content::Excerpt {
            async_request: Some(
                json!({"callId":"ask","questions":[{"title":"测试问题","options":["收到","没有"]}]}),
            ),
            ..Default::default()
        };
        let request = codex::async_request(&row, &excerpt).unwrap();
        let (received, server) = codex_server(Path::new(&source.path), changed, true);
        let mut client = codex::Client::open(Path::new(&source.path)).unwrap();
        let outcome = client.answer_async(&row, &request, "1");
        assert_eq!(
            outcome.status,
            if changed { "blocked" } else { "confirmed" }
        );
        drop(client);
        server.join().unwrap();
        let sent = received.try_iter().collect::<Vec<_>>();
        assert!(
            !sent
                .iter()
                .any(|v| matches!(v["method"].as_str(), Some("thread/resume" | "turn/start")))
        );
        let steer = sent.iter().find(|v| v["method"] == "turn/steer");
        if changed {
            assert!(steer.is_none());
            continue;
        }
        let p = &steer.unwrap()["params"];
        assert_eq!(p["expectedTurnId"], "turn-one");
        assert_eq!(p["threadId"], NATIVE);
        let text = p["input"][0]["text"].as_str().unwrap();
        let body = text
            .strip_prefix("<send_user_message_question_reply>\n")
            .unwrap()
            .strip_suffix("\n</send_user_message_question_reply>")
            .unwrap();
        let answer: Value = serde_json::from_str(body).unwrap();
        assert_eq!(answer[0]["answer"], "收到");
        assert_eq!(
            answer[0]["questionItemId"],
            json!(["request_user_input_async", "ask", 0]).to_string()
        );
    }
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

/// A stand-in for OpenCode's background service: one session with pending requests.
fn opencode_service(dir: &Path, state: Value) -> (Source, mpsc::Receiver<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut state = state;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                if stream.read_exact(&mut b).is_err() {
                    break;
                }
                head.push(b[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let mut request = head.lines().next().unwrap_or("").split(' ');
            let method = request.next().unwrap_or("");
            let path = request.next().unwrap_or("").split('?').next().unwrap();
            let header = |name: &str| {
                head.lines().find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case(name)
                        .then(|| value.trim().to_owned())
                })
            };
            let mut bytes = vec![
                0;
                header("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            ];
            stream.read_exact(&mut bytes).unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            // "opencode:secret"
            let authorized =
                header("authorization").as_deref() == Some("Basic b3BlbmNvZGU6c2VjcmV0");
            let (code, reply) = match (method, path.strip_prefix("/api/session/ses_fixture")) {
                _ if !authorized => (401, Value::Null),
                ("GET", Some("")) => (200, json!({"data":{"id":"ses_fixture"}})),
                ("GET", Some("/form")) => (200, json!({"data":state["forms"]})),
                ("GET", Some("/permission")) => (200, json!({"data":state["permissions"]})),
                ("GET", Some("/message")) => (200, json!({"data":state["messages"]})),
                ("POST", Some("/prompt")) => (200, json!({"data":{"id":"msg_new"}})),
                ("POST", Some(rest)) if rest.ends_with("/reply") => {
                    let kind = if rest.starts_with("/form/") {
                        "forms"
                    } else {
                        "permissions"
                    };
                    let id = rest.split('/').nth(2).unwrap_or("");
                    let rows = state[kind].as_array_mut().unwrap();
                    let before = rows.len();
                    rows.retain(|r| r["id"] != id);
                    (if rows.len() < before { 204 } else { 409 }, Value::Null)
                }
                _ => (404, json!({"_tag":"SessionNotFoundError"})),
            };
            if method == "POST" {
                sender.send((path.to_owned(), body)).unwrap();
            }
            let body = if reply.is_null() {
                String::new()
            } else {
                reply.to_string()
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let home = dir.join("opencode-home");
    let data = home.join("share/opencode");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("opencode.db"), b"").unwrap();
    std::fs::create_dir_all(home.join("state/opencode")).unwrap();
    std::fs::write(
        home.join("state/opencode/service.json"),
        json!({"url":url,"password":"secret","pid":1}).to_string(),
    )
    .unwrap();
    let source = Source {
        id: "opencode-source".into(),
        provider: "opencode".into(),
        path: data.to_string_lossy().into(),
        ..Default::default()
    };
    (source, receiver)
}

fn opencode_fixture(
    state: Value,
) -> (
    tempfile::TempDir,
    Store,
    Source,
    mpsc::Receiver<(String, Value)>,
) {
    let (dir, store, _) = fixture();
    let (source, calls) = opencode_service(dir.path(), state);
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    (dir, store, source, calls)
}

fn opencode_live(source: &Source, phase: Phase) -> LiveSession {
    let mut s = live(source, phase);
    s.native_session = "ses_fixture".into();
    s
}

/// Mark the newest notification as delivered and return its outbox id and card.
fn delivered(store: &Store, message: &str) -> (String, String) {
    let id: String = store
        .db
        .query_row(
            "SELECT id FROM channel_outbox ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    store
        .db
        .execute(
            "UPDATE channel_outbox SET sent_id=?1 WHERE id=?2",
            params![message, id],
        )
        .unwrap();
    let card = store
        .db
        .query_row(
            "SELECT payload FROM channel_cards WHERE outbox_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    (id, card)
}

fn command_status(store: &Store, reply_to: &str) -> String {
    control::process_command(store).unwrap();
    store
        .db
        .query_row(
            "SELECT status FROM channel_commands WHERE reply_to=?1",
            [reply_to],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn opencode_forms_become_cards_and_answers_reach_the_original_form() {
    let form = json!({"id":"frm_one","sessionID":"ses_fixture","title":"Questions","metadata":{"kind":"question"},"fields":[
        {"key":"q0","title":"Style","description":"Which style?","type":"string","options":[{"value":"comprehension","label":"Comprehension","description":"One line"},{"value":"loop","label":"Loop"}],"custom":true},
        {"key":"q1","title":"Features","description":"Which features?","type":"multiselect","options":[{"value":"A","label":"A"},{"value":"B","label":"B"},{"value":"C","label":"C"}]},
        {"key":"internal","type":"string","hidden":true}]});
    let (_dir, store, source, calls) = opencode_fixture(json!({"forms":[form],"permissions":[]}));
    let c = enabled(&store, true);
    let mut s = opencode_live(&source, Phase::NeedsInput);
    s.input_request = "question".into();
    let target = observe(&store, s);
    control::process_event(&store).unwrap();
    let (id, card) = delivered(&store, "om_form");
    assert!(card.contains("Which style?") && card.contains("Comprehension"));
    assert!(card.contains("Which features?") && card.contains("aieyes.submit"));
    assert!(!card.contains("internal"));
    // The service accepts partial answers; the adapter must not.
    for (message, body) in [
        ("partial", "1=2"),
        ("unlisted", "1=2\n2=Z"),
        ("approve", "1=2\n2=/approve"),
    ] {
        ingest(&store, incoming(message, body, "om_form"));
        assert_eq!(command_status(&store, message), "blocked");
    }
    assert!(calls.try_recv().is_err());
    let click = |event: &str, value: Value| json!({"header":{"event_id":event,"app_id":"cli_fixture","event_type":"card.action.trigger","create_time":(now()*1_000_000).to_string(),"tenant_key":"tenant"},"event":{"host":"im_message","operator":{"open_id":"ou_owner","tenant_key":"tenant"},"context":{"open_chat_id":"oc_owner","open_message_id":"om_form"},"action":{"tag":"button","value":value}}});
    for (event, value) in [
        (
            "style",
            json!({"action":"aieyes.answer","outboxId":id,"questionId":"1","option":1}),
        ),
        (
            "first",
            json!({"action":"aieyes.answer","outboxId":id,"questionId":"2","option":0}),
        ),
        (
            "third",
            json!({"action":"aieyes.answer","outboxId":id,"questionId":"2","option":2}),
        ),
        ("submit", json!({"action":"aieyes.submit","outboxId":id})),
    ] {
        actions::ingest(&store, &c.epoch, &click(event, value)).unwrap();
    }
    assert_eq!(command_status(&store, "om_form"), "confirmed");
    assert_eq!(
        calls.try_recv().unwrap(),
        (
            "/api/session/ses_fixture/form/frm_one/reply".into(),
            json!({"answer":{"q0":"loop","q1":["A","C"]}})
        )
    );
    // The form is settled: a late answer to the same notification is not replayed elsewhere.
    ingest(&store, incoming("late", "1=1\n2=2", "om_form"));
    assert_eq!(command_status(&store, "late"), "blocked");
    assert!(calls.try_recv().is_err());
    assert_eq!(target.session.native_session, "ses_fixture");
}

#[test]
fn opencode_answers_are_typed_and_unsupported_fields_stay_in_the_original_window() {
    let request =
        |questions: Value| json!({"method":"aieyes/opencodeForm","params":{"questions":questions}});
    let fields = json!([
        {"id":"1","key":"name","kind":"string","options":[]},
        {"id":"2","key":"sure","kind":"boolean","options":[{"label":"Yes","value":true},{"label":"No","value":false}]},
        {"id":"3","key":"count","kind":"integer","options":[]},
        {"id":"4","key":"ratio","kind":"number","options":[]},
        {"id":"5","key":"tags","kind":"multiselect","options":[{"label":"Red","value":"r"}],"custom":true}]);
    assert_eq!(
        opencode::answer(
            &request(fields.clone()),
            "1=Free text\n2=no\n3=4\n4=0.5\n5=1, custom"
        )
        .unwrap(),
        json!({"answer":{"name":"Free text","sure":false,"count":4,"ratio":0.5,"tags":["r","custom"]}})
    );
    assert!(opencode::answer(&request(fields.clone()), "1=x\n2=maybe\n3=4\n4=0.5\n5=1").is_err());
    assert!(opencode::answer(&request(fields), "1=x\n2=1\n3=four\n4=0.5\n5=1").is_err());
    let permission = json!({"method":"aieyes/opencodePermission","params":{}});
    assert_eq!(
        opencode::answer(&permission, "/decline").unwrap(),
        json!({"decision":"reject"})
    );
    assert!(opencode::answer(&permission, "always").is_err());
    // A browser hand-off cannot be answered from chat.
    let external = json!({"id":"frm_two","fields":[{"key":"login","type":"external","url":"https://example.com"}]});
    let (_dir, store, source, calls) =
        opencode_fixture(json!({"forms":[external],"permissions":[]}));
    enabled(&store, true);
    let mut s = opencode_live(&source, Phase::NeedsInput);
    s.input_request = "question".into();
    observe(&store, s);
    control::process_event(&store).unwrap();
    let (_, card) = delivered(&store, "om_external");
    assert!(!card.contains("button"));
    ingest(&store, incoming("answer", "done", "om_external"));
    assert_eq!(command_status(&store, "answer"), "blocked");
    assert!(calls.try_recv().is_err());
}

#[test]
fn opencode_permission_requests_are_polled_once_and_need_an_explicit_decision() {
    let permission = json!({"id":"per_one","sessionID":"ses_fixture","action":"shell","resources":["rm -rf build"]});
    let (_dir, store, source, calls) =
        opencode_fixture(json!({"forms":[],"permissions":[permission]}));
    enabled(&store, true);
    observe(&store, opencode_live(&source, Phase::Tool));
    assert_eq!(count(&store, "channel_events"), 0);
    opencode::poll(&store).unwrap();
    assert_eq!(count(&store, "channel_events"), 1);
    control::process_event(&store).unwrap();
    let (_, card) = delivered(&store, "om_permission");
    assert!(card.contains("rm -rf build") && card.contains("/approve"));
    assert!(!card.contains("button"));
    opencode::poll(&store).unwrap();
    assert_eq!(count(&store, "channel_events"), 0);
    ingest(&store, incoming("vague", "yes", "om_permission"));
    assert_eq!(command_status(&store, "vague"), "blocked");
    assert!(calls.try_recv().is_err());
    ingest(&store, incoming("approve", "/approve", "om_permission"));
    assert_eq!(command_status(&store, "approve"), "confirmed");
    assert_eq!(
        calls.try_recv().unwrap(),
        (
            "/api/session/ses_fixture/permission/per_one/reply".into(),
            json!({"decision":"once"})
        )
    );
    opencode::poll(&store).unwrap();
    assert_eq!(count(&store, "channel_events"), 0);
}

#[test]
fn opencode_final_reply_and_follow_up_use_the_service_only_for_its_own_sessions() {
    let messages = json!([
        {"type":"idle","outcome":"succeeded"},
        {"type":"assistant","finish":"stop","content":[{"type":"reasoning","text":"PRIVATE REASONING"},{"type":"text","text":"Final answer"}]},
        {"type":"assistant","finish":"tool-calls","content":[{"type":"text","text":"PRIVATE PROGRESS"},{"type":"tool","name":"read"}]},
        {"type":"user","text":"Do it","time":{"created":1}},
        {"type":"assistant","finish":"stop","content":[{"type":"text","text":"PREVIOUS TURN"}]}]);
    let (dir, store, source, calls) =
        opencode_fixture(json!({"forms":[],"permissions":[],"messages":messages}));
    enabled(&store, true);
    observe(&store, opencode_live(&source, Phase::Working));
    let done = observe(&store, opencode_live(&source, Phase::Complete));
    control::process_event(&store).unwrap();
    let (_, card) = delivered(&store, "om_done");
    assert!(card.contains("Final answer"));
    assert!(!card.contains("PRIVATE") && !card.contains("PREVIOUS"));
    ingest(&store, incoming("next", "Next step", "om_done"));
    assert_eq!(command_status(&store, "next"), "confirmed");
    assert_eq!(
        calls.try_recv().unwrap(),
        (
            "/api/session/ses_fixture/prompt".into(),
            json!({"text":"Next step"})
        )
    );
    // A standalone process is unknown to the service and keeps the terminal path.
    let mut other = opencode_live(&source, Phase::Complete);
    other.id = "standalone".into();
    other.native_session = "ses_standalone".into();
    let other = observe(&store, other);
    ingest(
        &store,
        incoming("standalone", &format!("/send {} Hello", other.key), ""),
    );
    assert_eq!(command_status(&store, "standalone"), "blocked");
    // The password is only ever sent to a loopback service.
    std::fs::write(
        dir.path().join("opencode-home/state/opencode/service.json"),
        json!({"url":"http://192.0.2.1:4096","password":"secret"}).to_string(),
    )
    .unwrap();
    ingest(
        &store,
        incoming("remote", &format!("/send {} Hello", done.key), ""),
    );
    assert_eq!(command_status(&store, "remote"), "blocked");
    assert!(calls.try_recv().is_err());
}

/// A local session of an agent whose record is read by `readers`.
fn reader_session(
    store: &Store,
    provider: &str,
    path: &Path,
    native: &str,
    phase: Phase,
) -> (Source, Session) {
    let source = Source {
        id: format!("{provider}-source"),
        provider: provider.into(),
        path: path.to_string_lossy().into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let mut s = live(&source, phase);
    s.id = format!("{provider}-{native}-{}", phase.key());
    s.native_session = native.into();
    (source, observe(store, s))
}

#[test]
fn kiro_pending_approval_last_question_and_final_reply_are_readable() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let home = dir.path().join("kiro");
    let folder = home.join("sessions/workspace/sess_fixture");
    std::fs::create_dir_all(&folder).unwrap();
    let row =
        |payload: Value| json!({"id":"r","timestamp":"2026-11-16T08:00:00.000Z","payload":payload});
    let mut rows = vec![
        row(json!({"type":"user","content":"Old input"})),
        row(json!({"type":"assistant","operationType":"Say","content":"PREVIOUS TURN"})),
        row(json!({"type":"turn_end","stopReason":"end_turn"})),
        row(json!({"type":"user","content":"Clean the build"})),
        row(json!({"type":"turn_start","turnId":"t2"})),
        row(json!({"type":"assistant","operationType":"Reasoning","content":"PRIVATE REASONING"})),
        row(json!({"type":"assistant","operationType":"Say","content":"PRIVATE PROGRESS"})),
        row(json!({"type":"tool_call","toolCallId":"call-1","toolName":"shell"})),
        row(
            json!({"type":"pending_interaction","interactionType":"tool_approval","toolCallId":"call-0","question":"Old approval","options":[{"optionId":"allow","name":"Yes","kind":"allow_once"}]}),
        ),
        row(
            json!({"type":"interaction_resolved","toolCallId":"call-0","outcome":"selected","selectedOption":"allow"}),
        ),
        row(
            json!({"type":"pending_interaction","interactionType":"tool_approval","toolCallId":"call-1","question":"Run rm -rf target?","options":[{"optionId":"allow","name":"Yes","kind":"allow_once"},{"optionId":"reject","name":"No","kind":"reject_once"}],"_meta":{"kiro":{"toolId":"shell"}}}),
        ),
    ];
    save_rows(&folder.join("messages.jsonl"), &rows);
    let (source, waiting) =
        reader_session(&store, "kiro", &home, "sess_fixture", Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.question.contains("Run rm -rf target?"));
    assert!(excerpt.question.contains("1. Yes") && excerpt.question.contains("2. No"));
    assert!(!excerpt.question.contains("Old approval"));
    assert_eq!(excerpt.user_text, "Clean the build");
    assert!(excerpt.user_at > 1_700_000_000_000);
    // The notification carries the approval instead of the generic placeholder.
    control::process_event(&store).unwrap();
    let (_, card) = delivered(&store, "om_kiro");
    assert!(card.contains("Run rm -rf target?") && !card.contains("PRIVATE"));
    // Once resolved, a waiting turn's question is the model's last message.
    rows.push(row(
        json!({"type":"interaction_resolved","toolCallId":"call-1","outcome":"selected","selectedOption":"allow"}),
    ));
    rows.push(row(
        json!({"type":"assistant","operationType":"Say","content":"Which branch should I use?"}),
    ));
    rows.push(row(json!({"type":"turn_end","stopReason":"end_turn"})));
    save_rows(&folder.join("messages.jsonl"), &rows);
    assert_eq!(
        readers::excerpt(&source, &waiting).unwrap().question,
        "Which branch should I use?"
    );
    let (source, done) = reader_session(&store, "kiro", &home, "sess_fixture", Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "Which branch should I use?");
    assert!(excerpt.question.is_empty());
    // Identifiers that could leave the source directory are never joined into a path.
    let (source, outside) = reader_session(&store, "kiro", &home, "../workspace", Phase::Complete);
    assert!(readers::excerpt(&source, &outside).is_none());
}

#[test]
fn antigravity_pending_call_and_final_reply_are_readable() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("agy");
    let folder = root
        .join("brain")
        .join(NATIVE)
        .join(".system_generated/logs");
    std::fs::create_dir_all(&folder).unwrap();
    let row = |kind: &str, extra: Value| {
        let mut v = json!({"type":kind,"status":"DONE","created_at":"2026-11-16T08:00:00Z"});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let mut rows = vec![
        row("USER_INPUT", json!({"content":"Old input"})),
        row("PLANNER_RESPONSE", json!({"content":"PREVIOUS TURN"})),
        row("USER_INPUT", json!({"content":"Ship it"})),
        row(
            "PLANNER_RESPONSE",
            json!({"thinking":"PRIVATE REASONING","tool_calls":[
                {"name":"view_file","args":{"AbsolutePath":"/fixture/a"}},
                {"name":"run_command","args":{"CommandLine":"git push origin main","Cwd":"/fixture"}}]}),
        ),
        row("GENERIC", json!({"content":"PRIVATE TOOL RESULT"})),
    ];
    save_rows(&folder.join("transcript.jsonl"), &rows);
    let (source, waiting) = reader_session(&store, "antigravity", &root, NATIVE, Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert_eq!(excerpt.question, "run_command\ngit push origin main");
    assert_eq!(excerpt.user_text, "Ship it");
    // A question tool lists its options.
    rows.push(row("GENERIC", json!({"content":"PRIVATE TOOL RESULT"})));
    rows.push(row(
        "PLANNER_RESPONSE",
        json!({"tool_calls":[{"name":"ask_question","args":{"questions":[{"question":"Which target?","options":["Staging","Production"]}]}}]}),
    ));
    save_rows(&folder.join("transcript.jsonl"), &rows);
    let question = readers::excerpt(&source, &waiting).unwrap().question;
    assert!(question.contains("Which target?") && question.contains("2. Production"));
    rows.push(row("GENERIC", json!({"content":"PRIVATE TOOL RESULT"})));
    rows.push(row("PLANNER_RESPONSE", json!({"content":"Final answer"})));
    save_rows(&folder.join("transcript.jsonl"), &rows);
    let (source, done) = reader_session(&store, "antigravity", &root, NATIVE, Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "Final answer");
    assert!(excerpt.question.is_empty());
}

#[test]
fn harness_compressed_log_exposes_pending_question_approval_and_final_reply() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("dsh");
    // The harness names a session after its folder.
    let native = format!("session-{NATIVE}");
    let folder = root.join("sessions/fixture").join(&native);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("session.v4.jsonl.zstd");
    let save = |rows: &[Value]| {
        let text = rows.iter().map(|r| format!("{r}\n")).collect::<String>();
        std::fs::write(&path, zstd::encode_all(text.as_bytes(), 0).unwrap()).unwrap();
    };
    let arguments = json!({"questions":[{"id":"q1","header":"Scope","question":"Which scope?","options":[{"label":"Small","description":"One file"},{"label":"Large","description":"Whole crate"}]}]}).to_string();
    let mut rows = vec![
        json!({"type":"session","id":native}),
        json!({"type":"turn/start","time":1,"data":{"turn":1}}),
        json!({"type":"user/message","time":2,"data":{"content":[{"type":"text","text":"Old input"}]}}),
        json!({"type":"assistant/message","time":3,"data":{"turn":1,"message":{"content":[{"type":"text","text":"PREVIOUS TURN"}]}}}),
        json!({"type":"turn/end","time":4,"data":{"turn":1,"reason":{"kind":"completed"}}}),
        json!({"type":"turn/start","time":5,"data":{"turn":2}}),
        json!({"type":"user/message","time":6,"data":{"content":[{"type":"text","text":"Refactor it"}]}}),
        json!({"type":"assistant/message","time":7,"data":{"turn":2,"message":{"content":[{"type":"reasoning","text":"PRIVATE REASONING"},{"type":"text","text":"PRIVATE PROGRESS"}]}}}),
        json!({"type":"tool/call","time":8,"data":{"turn":2,"step":0,"callId":"ask","name":"ask_user_question","arguments":arguments}}),
    ];
    save(&rows);
    let source = |phase| {
        let (source, mut target) =
            reader_session(&store, "deepseek-harness", &root, &native, phase);
        target.path = path.to_string_lossy().into();
        (source, target)
    };
    let (dsh, waiting) = source(Phase::NeedsInput);
    let excerpt = readers::excerpt(&dsh, &waiting).unwrap();
    assert!(excerpt.question.contains("Which scope?") && excerpt.question.contains("2. Large"));
    assert!(!excerpt.question.contains("PRIVATE"));
    assert_eq!(
        (excerpt.user_text.as_str(), excerpt.user_at),
        ("Refactor it", 6)
    );
    rows.push(json!({"type":"tool/result","time":9,"data":{"turn":2,"step":0,"message":{"role":"tool","id":"m","toolCallId":"ask","content":[]}}}));
    rows.push(json!({"type":"tool/call","time":10,"data":{"turn":2,"step":1,"callId":"write","name":"write","arguments":"{\"path\":\"/fixture/a\"}"}}));
    rows.push(json!({"type":"approval/asked","time":11,"data":{"id":"a1","toolName":"write","callId":"write","reason":"outside workspace"}}));
    save(&rows);
    let question = readers::excerpt(&dsh, &waiting).unwrap().question;
    assert!(question.starts_with("write\noutside workspace\n") && question.contains("/fixture/a"));
    assert!(!question.contains("Which scope?"));
    rows.push(
        json!({"type":"approval/decided","time":12,"data":{"id":"a1","outcome":"allowed-once"}}),
    );
    rows.push(json!({"type":"assistant/message","time":13,"data":{"turn":2,"message":{"content":[{"type":"text","text":"Final answer"}]}}}));
    rows.push(json!({"type":"turn/end","time":14,"data":{"turn":2,"reason":{"kind":"completed"}}}));
    save(&rows);
    let (dsh, done) = source(Phase::Complete);
    let excerpt = readers::excerpt(&dsh, &done).unwrap();
    assert_eq!(excerpt.text, "Final answer");
    assert!(excerpt.question.is_empty());
    // Another session's log is never read under this session's name.
    rows[0] = json!({"type":"session","id":CHILD});
    save(&rows);
    assert!(readers::excerpt(&dsh, &done).is_none());
}

#[test]
fn qoder_desktop_pending_question_and_final_reply_are_readable() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let folder = dir.path().join("qoder-desktop");
    std::fs::create_dir_all(&folder).unwrap();
    let db = rusqlite::Connection::open(folder.join("main.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE chat_session_messages(session_id TEXT, message_id TEXT, turn_id TEXT, sequence INTEGER, payload_json TEXT, status TEXT, created_at INTEGER, role TEXT)").unwrap();
    let insert = |session: &str, sequence: i64, role: &str, payload: Value| {
        db.execute(
            "INSERT INTO chat_session_messages VALUES(?1,?2,'t',?3,?4,'completed',?5,?6)",
            params![
                session,
                format!("m{sequence}"),
                sequence,
                payload.to_string(),
                1_794_000_000_000 + sequence,
                role
            ],
        )
        .unwrap();
    };
    insert(
        NATIVE,
        1,
        "user",
        json!({"parts":[{"type":"text","id":"u","text":"Plan the release"}]}),
    );
    insert(
        CHILD,
        2,
        "assistant",
        json!({"parts":[{"type":"text","id":"x","text":"OTHER SESSION"}]}),
    );
    let question = json!({"questions":[{"question":"Which channel?","options":[{"label":"Beta","description":"Testers"},{"label":"Stable","description":"Everyone"}]}]});
    insert(
        NATIVE,
        3,
        "assistant",
        json!({"parts":[
        {"type":"thinking","id":"r","text":"PRIVATE REASONING"},
        {"type":"tool","id":"p1","tool":{"id":"c0","name":"AskUserQuestion","input":{"questions":[{"question":"ANSWERED","options":[]}]},"status":"completed","completedAt":5}},
        {"type":"tool","id":"p2","tool":{"id":"c1","name":"AskUserQuestion","input":question,"status":"running"}}]}),
    );
    let (source, waiting) = reader_session(&store, "qoder", &folder, NATIVE, Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.question.contains("Which channel?") && excerpt.question.contains("2. Stable"));
    assert!(!excerpt.question.contains("ANSWERED"));
    assert_eq!(excerpt.user_text, "Plan the release");
    insert(
        NATIVE,
        4,
        "assistant",
        json!({"finalTextId":"f","parts":[
        {"type":"text","id":"s","parentToolUseId":"task","text":"PRIVATE SUBAGENT"},
        {"type":"text","id":"f","parentToolUseId":null,"text":"Final answer"},
        {"type":"tool","id":"p3","tool":{"id":"c2","name":"Bash","status":"completed","completedAt":9}}]}),
    );
    let (source, done) = reader_session(&store, "qoder", &folder, NATIVE, Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "Final answer");
    assert!(excerpt.question.is_empty());
}

#[test]
fn qoder_cli_shares_the_question_hook_and_desktop_sources_are_left_alone() {
    let (dir, store, _) = fixture();
    let root = dir.path().canonicalize().unwrap();
    let cli = root.join("qoder-cli");
    std::fs::create_dir_all(cli.join("projects/fixture")).unwrap();
    let desktop = root.join("qoder-desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    rusqlite::Connection::open(desktop.join("main.sqlite"))
        .unwrap()
        .execute_batch("CREATE TABLE chat_session_messages(session_id TEXT)")
        .unwrap();
    // A folder without transcripts is not a CLI configuration directory.
    let other = root.join("qoder-other");
    std::fs::create_dir_all(&other).unwrap();
    let source = |id: &str, provider: &str, path: &Path| Source {
        id: id.into(),
        provider: provider.into(),
        path: path.to_string_lossy().into(),
        ..Default::default()
    };
    let sources = vec![
        source("qoder-cli", "qoder", &cli),
        source("qoder-cli-again", "qoder", &cli),
        source("qoder-desktop", "qoder", &desktop),
        source("qoder-other", "qoder-cn", &other),
    ];
    store
        .save_settings(&Settings {
            sources: sources.clone(),
            ..Default::default()
        })
        .unwrap();
    enabled(&store, true);
    claude::install(&store).unwrap();
    let installed: Value =
        serde_json::from_slice(&std::fs::read(cli.join("settings.json")).unwrap()).unwrap();
    let rows = installed["hooks"]["PermissionRequest"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["matcher"], "AskUserQuestion");
    let command = rows[0]["hooks"][0]["command"].as_str().unwrap();
    // The first source of a shared directory owns the hook.
    assert!(
        command.contains("--aieyes-claude-question")
            && command.trim_end_matches(['\'', '"']).ends_with("qoder-cli")
    );
    assert!(!desktop.join("settings.json").exists() && !desktop.join(".aieyes").exists());
    assert!(!other.join("settings.json").exists());

    // The question becomes a card with buttons and the reply waits for the hook.
    let input = json!({"questions":[{"question":"Which system?","header":"System","options":[{"label":"Linux","description":"One"},{"label":"macOS","description":"Two"}],"multiSelect":false}]});
    let path = cli.join("projects/fixture").join(format!("{NATIVE}.jsonl"));
    let mut s = live(&sources[0], Phase::NeedsInput);
    s.input_request = "question-tool".into();
    s.path = path.to_string_lossy().into();
    let at = s.turn_started_at + 1000;
    save_rows(
        &path,
        &[
            json!({"timestamp":at,"type":"assistant","sessionId":NATIVE,"message":{"role":"assistant","content":[{"type":"tool_use","name":"AskUserQuestion","id":"question-tool","input":input}]}}),
        ],
    );
    let mut target = observe(&store, s);
    control::process_event(&store).unwrap();
    let (_, card) = delivered(&store, "om_qoder");
    assert!(card.contains("Which system?") && card.contains("macOS") && card.contains("button"));
    ingest(&store, incoming("early", "2", "om_qoder"));
    // No hook is waiting yet: nothing is typed into a terminal instead.
    assert_eq!(command_status(&store, "early"), "blocked");
    let pid = std::process::id();
    let mut system = sysinfo::System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    let start = system
        .process(sysinfo::Pid::from_u32(pid))
        .unwrap()
        .start_time();
    target.process = Some(ControlProcess { pid, start });
    let c = configuration(&store.db).unwrap();
    store.db.execute("INSERT INTO channel_claude_requests(id,epoch,source_id,native,question_hash,call_id,input,pid,process_start,at,heartbeat) VALUES('request',?1,?2,?3,?4,'question-tool',?5,?6,?7,?8,?8)",params![c.epoch,sources[0].id,NATIVE,claude::question_hash(&input),input.to_string(),pid,start,now()]).unwrap();
    let outcome = claude::answer(&store, &sources[0], &target, "2");
    assert_eq!(outcome.status, "submitted");
    let response: String = store
        .db
        .query_row("SELECT response FROM channel_claude_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(
        response["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Which system?"],
        "macOS"
    );
}

/// Rows as written by Kimi Code 2.4.1 (`interaction.request` / `interaction.resolved`).
#[test]
fn kimi_pending_approval_and_question_are_readable_until_resolved() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("kimi");
    let folder = format!("session_{NATIVE}");
    let agent = root
        .join("sessions/wd_fixture")
        .join(&folder)
        .join("agents/main");
    std::fs::create_dir_all(&agent).unwrap();
    let path = agent.join("wire.jsonl");
    let native = format!("{folder}:main");
    let part = |kind: &str, text: &str| json!({"type":"context.append_loop_event","agentId":"main","event":{"type":"content.part","turnId":"0","part":{"type":kind,(kind):text,"text":text}},"time":3});
    let mut rows = vec![
        json!({"type":"metadata","protocol_version":"1.5","created_at":1}),
        json!({"type":"turn.prompt","agentId":"main","input":[{"type":"text","text":"Remove the folder"}],"turnId":0,"time":1_795_631_203_441_i64}),
        part("think", "PRIVATE REASONING"),
        part("text", "PRIVATE PROGRESS"),
        json!({"type":"context.append_loop_event","agentId":"main","event":{"type":"tool.call","toolCallId":"call-0","name":"Bash"},"time":4}),
        json!({"type":"interaction.request","agentId":"main","id":"approval_0","kind":"approval","toolCallId":"call-0","request":{"id":"approval_0","sessionId":folder,"toolName":"Bash","action":"Running: ls","display":{"kind":"command","command":"ls","cwd":"/fixture"}}}),
        json!({"type":"interaction.resolved","id":"approval_0","response":{"decision":"approved"}}),
        json!({"type":"interaction.request","agentId":"main","id":"approval_1","kind":"approval","toolCallId":"call-1","request":{"id":"approval_1","sessionId":folder,"toolName":"Bash","action":"Running: rm -rf probe_dir","display":{"kind":"command","command":"rm -rf probe_dir","cwd":"/fixture","language":"bash"}}}),
    ];
    save_rows(&path, &rows);
    let session = |native: &str, phase| {
        let (source, mut target) = reader_session(&store, "kimi", &root, native, phase);
        target.path = path.to_string_lossy().into();
        (source, target)
    };
    let (source, waiting) = session(&native, Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let approval = excerpt.structured_approval.unwrap();
    assert_eq!(approval["toolName"], "Bash");
    assert_eq!(approval["command"], "rm -rf probe_dir");
    assert_eq!(approval["reason"], "Running: rm -rf probe_dir");
    assert_eq!(approval["kind"], "command");
    assert_eq!(approval["cwd"], "/fixture");
    assert!(excerpt.question.contains("rm -rf probe_dir") && !excerpt.question.contains("PRIVATE"));
    assert_eq!(excerpt.user_text, "Remove the folder");
    assert_eq!(excerpt.user_at, 1_795_631_203_441);
    // A file write shows its path and content; an edit shows both sides.
    rows.push(
        json!({"type":"interaction.resolved","id":"approval_1","response":{"decision":"rejected"}}),
    );
    rows.push(json!({"type":"interaction.request","id":"approval_2","kind":"approval","request":{"sessionId":folder,"toolName":"Edit","action":"Editing hello.txt","display":{"kind":"file_io","operation":"edit","path":"/fixture/hello.txt","before":"hi","after":"hello"}}}));
    save_rows(&path, &rows);
    let approval = readers::excerpt(&source, &waiting)
        .unwrap()
        .structured_approval
        .unwrap();
    assert_eq!(approval["kind"], "fileChange");
    assert_eq!(approval["command"], "/fixture/hello.txt");
    assert_eq!(approval["details"], "- hi\n+ hello");
    // A request naming another session is never shown as this session's prompt.
    rows.push(
        json!({"type":"interaction.resolved","id":"approval_2","response":{"decision":"approved"}}),
    );
    rows.push(json!({"type":"interaction.request","id":"approval_3","kind":"approval","request":{"sessionId":"session_other","toolName":"Bash","action":"Running: FOREIGN","display":{"kind":"command","command":"FOREIGN"}}}));
    save_rows(&path, &rows);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.structured_approval.is_none() && !excerpt.question.contains("FOREIGN"));
    rows.pop();
    rows.push(json!({"type":"interaction.request","id":"question_1","kind":"question","toolCallId":"call-3","request":{"turnId":0,"toolCallId":"call-3","questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"Warm"},{"label":"Blue"}]},{"question":"Which fruits?","header":"Fruit","multiSelect":true,"options":[{"label":"Apple"},{"label":"Cherry"}]}]}}));
    save_rows(&path, &rows);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let questions = excerpt.structured_questions.unwrap();
    assert_eq!(questions[0]["question"], "Which color?");
    assert_eq!(
        questions[0]["options"][0],
        json!({"label":"Red","description":"Warm"})
    );
    assert_eq!(questions[1]["multiSelect"], true);
    assert!(excerpt.question.contains("2. Blue"));
    // Once resolved, the turn's last text is the reply.
    rows.push(json!({"type":"interaction.resolved","id":"question_1","response":{}}));
    rows.push(part("text", "All done"));
    rows.push(json!({"type":"turn.ended","reason":"completed","turnId":0}));
    save_rows(&path, &rows);
    let (source, done) = session(&native, Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "All done");
    assert!(excerpt.question.is_empty() && excerpt.structured_questions.is_none());
    // Another agent of the same session, or another session, does not read this record.
    let (source, other) = session(&format!("{folder}:coder"), Phase::NeedsInput);
    assert!(readers::excerpt(&source, &other).is_none());
    let (source, other) = session("session_other:main", Phase::NeedsInput);
    assert!(readers::excerpt(&source, &other).is_none());
}

/// Rows as written by Grok CLI 1.0.92: `events.jsonl` names the tool, `updates.jsonl` the call.
#[test]
fn grok_pending_permission_and_question_are_read_from_both_records() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("grok");
    let folder = root.join("sessions/%2Ffixture").join(NATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let ts = "2026-11-25T18:29:29.067Z";
    let event = |kind: &str, tool: &str| json!({"ts":ts,"type":kind,"tool_name":tool});
    let update = |update: Value| json!({"timestamp":1_795_631_363,"method":"session/update","params":{"sessionId":NATIVE,"update":update}});
    let call = |id: &str, tool: &str, kind: &str, input: Value| {
        update(
            json!({"sessionUpdate":"tool_call","toolCallId":id,"title":tool,"rawInput":input,"_meta":{"x.ai/tool":{"name":tool,"kind":kind}}}),
        )
    };
    let mut events = vec![
        json!({"ts":ts,"type":"turn_started","session_id":NATIVE,"turn_number":0}),
        event("tool_started", "list_dir"),
        event("permission_requested", "list_dir"),
        json!({"ts":ts,"type":"permission_resolved","tool_name":"list_dir","decision":"allow","wait_ms":0}),
        json!({"ts":ts,"type":"tool_completed","tool_name":"list_dir","tool_call_id":"call-0"}),
        event("tool_started", "run_terminal_command"),
        event("permission_requested", "run_terminal_command"),
    ];
    let mut updates = vec![
        update(
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Remove the folder"}}),
        ),
        update(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"PRIVATE PROGRESS"}}),
        ),
        call(
            "call-0",
            "list_dir",
            "list",
            json!({"target_directory":"/fixture"}),
        ),
        update(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-0","kind":"other","rawInput":{"variant":"ListDir"}}),
        ),
        update(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-0","status":"completed"}),
        ),
        call(
            "call-1",
            "run_terminal_command",
            "execute",
            json!({"command":"rm -rf probe_dir","description":"Remove it"}),
        ),
    ];
    let save = |events: &[Value], updates: &[Value]| {
        save_rows(&folder.join("events.jsonl"), events);
        save_rows(&folder.join("updates.jsonl"), updates);
    };
    save(&events, &updates);
    let session = |phase| {
        let (source, mut target) = reader_session(&store, "grok", &root, NATIVE, phase);
        target.path = folder.join("events.jsonl").to_string_lossy().into();
        (source, target)
    };
    let (source, waiting) = session(Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let approval = excerpt.structured_approval.unwrap();
    assert_eq!(approval["toolName"], "run_terminal_command");
    assert_eq!(approval["command"], "rm -rf probe_dir");
    assert_eq!(approval["reason"], "Remove it");
    assert_eq!(approval["kind"], "command");
    assert_eq!(excerpt.user_text, "Remove the folder");
    assert!(!excerpt.question.contains("PRIVATE"));
    // A parallel call that is allowed automatically and finishes leaves this dialog open.
    let mut parallel = events.clone();
    parallel.extend([
        event("tool_started", "list_dir"),
        event("permission_requested", "list_dir"),
        json!({"ts":ts,"type":"permission_resolved","tool_name":"list_dir","decision":"allow","wait_ms":0}),
        json!({"ts":ts,"type":"tool_completed","tool_name":"list_dir","tool_call_id":"call-9"}),
    ]);
    save(&parallel, &updates);
    let approval = readers::excerpt(&source, &waiting)
        .unwrap()
        .structured_approval
        .expect("the open dialog is still shown");
    assert_eq!(approval["toolName"], "run_terminal_command");
    assert_eq!(approval["command"], "rm -rf probe_dir");
    // Its own answer closes it.
    parallel.push(json!({"ts":ts,"type":"permission_resolved","tool_name":"run_terminal_command","decision":"allow"}));
    save(&parallel, &updates);
    assert!(
        readers::excerpt(&source, &waiting)
            .unwrap()
            .structured_approval
            .is_none()
    );
    save(&events, &updates);
    // Two unfinished calls of the tool: the permission row cannot say which, so no command.
    updates.push(call(
        "call-2",
        "run_terminal_command",
        "execute",
        json!({"command":"OTHER"}),
    ));
    save(&events, &updates);
    let approval = readers::excerpt(&source, &waiting)
        .unwrap()
        .structured_approval
        .unwrap();
    assert_eq!(approval["toolName"], "run_terminal_command");
    assert_eq!(approval["command"], "");
    // The question tool is allowed automatically; its unfinished call carries the options.
    events.push(json!({"ts":ts,"type":"permission_resolved","tool_name":"run_terminal_command","decision":"deny"}));
    events.push(event("tool_started", "ask_user_question"));
    events.push(event("permission_requested", "ask_user_question"));
    events.push(json!({"ts":ts,"type":"permission_resolved","tool_name":"ask_user_question","decision":"allow","wait_ms":0}));
    for id in ["call-1", "call-2"] {
        updates.push(update(
            json!({"sessionUpdate":"tool_call_update","toolCallId":id,"status":"failed"}),
        ));
    }
    updates.push(call("call-3", "ask_user_question", "ask_user", json!({"questions":[{"question":"Which color?","options":[{"label":"Red","description":"Pick red."},{"label":"Blue"}]},{"question":"Which fruits?","options":[{"label":"Apple"}],"multi_select":true}]})));
    save(&events, &updates);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.structured_approval.is_none());
    let questions = excerpt.structured_questions.unwrap();
    assert_eq!(questions[0]["options"][0]["description"], "Pick red.");
    assert_eq!(questions[0]["multiSelect"], false);
    assert_eq!(questions[1]["multiSelect"], true);
    updates.push(update(
        json!({"sessionUpdate":"tool_call_update","toolCallId":"call-3","status":"completed"}),
    ));
    updates.push(update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"All "}}),
    ));
    updates.push(update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}}),
    ));
    save(&events, &updates);
    let (source, done) = session(Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "All done");
    assert!(excerpt.question.is_empty());
    // A record of another session is refused.
    let (source, mut other) =
        reader_session(&store, "grok", &root, "other-session", Phase::NeedsInput);
    other.path = folder.join("events.jsonl").to_string_lossy().into();
    assert!(readers::excerpt(&source, &other).is_none());
}

/// Rows as written by Copilot CLI 1.0.105.
#[test]
fn copilot_pending_permission_and_ask_user_form_are_readable() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("copilot");
    let folder = root.join("session-state").join(NATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("events.jsonl");
    let row = |kind: &str, data: Value| json!({"type":kind,"data":data,"timestamp":"2026-11-25T18:56:26.573Z"});
    let mut rows = vec![
        row("session.start", json!({"sessionId":NATIVE})),
        row("user.message", json!({"content":"Remove the folder"})),
        row(
            "assistant.message",
            json!({"content":"PRIVATE PROGRESS","toolRequests":[{"toolCallId":"call-1","name":"bash"}]}),
        ),
        row(
            "tool.execution_start",
            json!({"toolCallId":"call-1","toolName":"bash","arguments":{"command":"rm -rf probe_dir"}}),
        ),
        row(
            "permission.requested",
            json!({"requestId":"r0","permissionRequest":{"kind":"shell","fullCommandText":"OLD"}}),
        ),
        row(
            "permission.completed",
            json!({"requestId":"r0","result":{"kind":"approved"}}),
        ),
        row(
            "permission.requested",
            json!({"requestId":"r1","permissionRequest":{"kind":"shell","toolCallId":"call-1","fullCommandText":"rm -rf probe_dir","intention":"Remove probe_dir directory"},"promptRequest":{"kind":"commands"}}),
        ),
    ];
    save_rows(&path, &rows);
    let session = |phase| {
        let (source, mut target) = reader_session(&store, "copilot", &root, NATIVE, phase);
        target.path = path.to_string_lossy().into();
        (source, target)
    };
    let (source, waiting) = session(Phase::NeedsInput);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let approval = excerpt.structured_approval.unwrap();
    assert_eq!(approval["toolName"], "shell");
    assert_eq!(approval["command"], "rm -rf probe_dir");
    assert_eq!(approval["reason"], "Remove probe_dir directory");
    assert_eq!(approval["kind"], "command");
    assert!(!excerpt.question.contains("OLD") && !excerpt.question.contains("PRIVATE"));
    rows.push(row(
        "permission.completed",
        json!({"requestId":"r1","result":{"kind":"approved"}}),
    ));
    rows.push(row(
        "tool.execution_complete",
        json!({"toolCallId":"call-1"}),
    ));
    rows.push(row("tool.execution_start", json!({"toolCallId":"call-2","toolName":"ask_user","arguments":{"message":"Which color would you like?","requestedSchema":{"properties":{"color":{"type":"string","title":"Color","enum":["red","green","blue"]}}}}})));
    save_rows(&path, &rows);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let questions = excerpt.structured_questions.unwrap();
    assert_eq!(questions[0]["id"], "color");
    assert_eq!(questions[0]["question"], "Which color would you like?");
    assert_eq!(questions[0]["options"][2]["label"], "blue");
    // A free-text field is not a choice: only the message is shown.
    rows.push(row(
        "tool.execution_complete",
        json!({"toolCallId":"call-2"}),
    ));
    rows.push(row("tool.execution_start", json!({"toolCallId":"call-3","toolName":"ask_user","arguments":{"message":"What name?","requestedSchema":{"properties":{"name":{"type":"string"}}}}})));
    save_rows(&path, &rows);
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.structured_questions.is_none());
    assert_eq!(excerpt.question, "What name?");
    rows.push(row(
        "tool.execution_complete",
        json!({"toolCallId":"call-3"}),
    ));
    rows.push(row(
        "assistant.message",
        json!({"content":"All done","toolRequests":[]}),
    ));
    save_rows(&path, &rows);
    let (source, done) = session(Phase::Complete);
    let excerpt = readers::excerpt(&source, &done).unwrap();
    assert_eq!(excerpt.text, "All done");
    assert!(excerpt.question.is_empty());
}

/// A parsed object is sorted by key and option labels are strings: the form is still shown
/// in the order Copilot wrote it, with number and boolean choices as written.
#[test]
fn copilot_form_keeps_its_field_order_and_shows_non_string_choices() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    let root = dir.path().join("copilot");
    let folder = root.join("session-state").join(NATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("events.jsonl");
    let head = format!(
        r#"{{"type":"session.start","data":{{"sessionId":"{NATIVE}"}},"timestamp":"2026-11-25T18:56:26.573Z"}}
{{"type":"user.message","data":{{"content":"Ask"}},"timestamp":"2026-11-25T18:56:26.573Z"}}
"#
    );
    let ask = |call: &str, properties: &str| {
        format!(
            r#"{{"type":"tool.execution_start","data":{{"toolCallId":"{call}","toolName":"ask_user","arguments":{{"message":"Set up","requestedSchema":{{"properties":{properties}}}}}}},"timestamp":"2026-11-25T18:56:27.573Z"}}
"#
        )
    };
    let form = r#"{"zeta":{"type":"integer","title":"Replicas","enum":[1,2,3]},"alpha":{"type":"boolean","enum":[true,false]},"mid":{"type":"number","enum":[1.5]}}"#;
    std::fs::write(&path, format!("{head}{}", ask("call-1", form))).unwrap();
    let (source, mut waiting) = reader_session(&store, "copilot", &root, NATIVE, Phase::NeedsInput);
    waiting.path = path.to_string_lossy().into();
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    let questions = excerpt.structured_questions.unwrap();
    let ids: Vec<&str> = questions
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["zeta", "alpha", "mid"]);
    assert_eq!(
        questions[0]["options"],
        json!([{"label":"1"},{"label":"2"},{"label":"3"}])
    );
    assert_eq!(
        questions[1]["options"],
        json!([{"label":"true"},{"label":"false"}])
    );
    assert_eq!(questions[2]["options"], json!([{"label":"1.5"}]));
    assert!(excerpt.question.contains("1. 1") && excerpt.question.starts_with("Set up\n"));

    // A choice that is not a plain value cannot be shown as one: only the message is.
    let odd = r#"{"zeta":{"enum":[{"const":1}]},"alpha":{"enum":["a"]}}"#;
    std::fs::write(
        &path,
        format!("{head}{}{}", ask("call-1", form), ask("call-2", odd)),
    )
    .unwrap();
    let excerpt = readers::excerpt(&source, &waiting).unwrap();
    assert!(excerpt.structured_questions.is_none());
    assert_eq!(excerpt.question, "Set up");
}

/// Rows as written by CodeBuddy Code 2.187.0; WorkBuddy uses the same transcript family.
#[test]
fn codebuddy_and_workbuddy_pending_question_and_single_approval_are_readable() {
    let (dir, store, _) = fixture();
    enabled(&store, true);
    for provider in ["codebuddy", "workbuddy"] {
        let root = dir.path().join(provider);
        let project = root.join("projects/fixture-project");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{NATIVE}.jsonl"));
        let arguments = json!({"questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"Warm"},{"label":"Blue"}],"multiSelect":false},{"question":"Which fruits?","header":"Fruit","options":[{"label":"Apple"}],"multiSelect":true}]}).to_string();
        let mut rows = vec![
            json!({"id":"u1","timestamp":1_795_632_421_755_i64,"type":"message","role":"user","content":[{"type":"input_text","text":"Ask me"}],"sessionId":NATIVE}),
            json!({"id":"a0","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"PRIVATE PROGRESS"}],"sessionId":NATIVE}),
            json!({"id":"c0","type":"function_call","callId":"call-0","name":"Read","arguments":"{}","sessionId":NATIVE}),
            json!({"id":"r0","type":"function_call_result","callId":"call-0","sessionId":NATIVE}),
            json!({"id":"c1","type":"function_call","callId":"call-1","name":"AskUserQuestion","arguments":arguments,"sessionId":NATIVE}),
        ];
        save_rows(&path, &rows);
        let session = |phase, kind: &str| {
            let (source, mut target) = reader_session(&store, provider, &root, NATIVE, phase);
            // WorkBuddy sessions come from its database and carry no transcript path.
            if provider == "codebuddy" {
                target.path = path.to_string_lossy().into();
            }
            target.session.input_kind = kind.into();
            (source, target)
        };
        let (source, waiting) = session(Phase::NeedsInput, "question");
        let excerpt = readers::excerpt(&source, &waiting).unwrap();
        let questions = excerpt.structured_questions.unwrap();
        assert_eq!(questions[0]["id"], "Color");
        assert_eq!(
            questions[0]["options"][0],
            json!({"label":"Red","description":"Warm"})
        );
        assert_eq!(questions[1]["multiSelect"], true);
        assert_eq!(excerpt.user_text, "Ask me");
        assert!(!excerpt.question.contains("PRIVATE"));
        // One unanswered call while the session waits on a permission is that permission.
        rows.push(
            json!({"id":"r1","type":"function_call_result","callId":"call-1","sessionId":NATIVE}),
        );
        rows.push(json!({"id":"c2","type":"function_call","callId":"call-2","name":"Bash","arguments":"{\"command\":\"rm -rf probe_dir\",\"description\":\"Remove it\"}","sessionId":NATIVE}));
        save_rows(&path, &rows);
        let (source, waiting) = session(Phase::NeedsInput, "approval");
        let approval = readers::excerpt(&source, &waiting)
            .unwrap()
            .structured_approval
            .unwrap();
        assert_eq!(approval["toolName"], "Bash");
        assert_eq!(approval["command"], "rm -rf probe_dir");
        assert_eq!(approval["kind"], "command");
        // With two calls in flight the transcript does not say which one is being asked.
        rows.push(json!({"id":"c3","type":"function_call","callId":"call-3","name":"Bash","arguments":"{\"command\":\"OTHER\"}","sessionId":NATIVE}));
        // A subagent's call is not this session's.
        rows.push(json!({"id":"c4","type":"function_call","callId":"call-4","name":"AskUserQuestion","arguments":arguments,"isSidechain":true,"sessionId":NATIVE}));
        save_rows(&path, &rows);
        let excerpt = readers::excerpt(&source, &waiting).unwrap();
        assert!(excerpt.structured_approval.is_none() && excerpt.structured_questions.is_none());
        assert!(excerpt.question.is_empty());
        for call in ["call-2", "call-3"] {
            rows.push(
                json!({"id":call,"type":"function_call_output","callId":call,"sessionId":NATIVE}),
            );
        }
        rows.push(json!({"id":"a1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"All done"}],"sessionId":NATIVE}));
        save_rows(&path, &rows);
        let (source, done) = session(Phase::Complete, "");
        let excerpt = readers::excerpt(&source, &done).unwrap();
        assert_eq!(excerpt.text, "All done");
        assert!(excerpt.question.is_empty());
    }
}

/// With the terminal adapter on, a chat reply is still never typed into an agent's own
/// approval dialog or option menu.
#[test]
fn chat_reply_is_never_typed_into_an_open_approval_or_option_menu() {
    let (dir, store, _) = fixture();
    let mut c = enabled(&store, true);
    c.allow_terminal = true;
    config::write(&store.db, &c).unwrap();
    let root = dir.path().join("copilot");
    let folder = root.join("session-state").join(NATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("events.jsonl");
    let row = |kind: &str, data: Value| json!({"type":kind,"data":data,"timestamp":"2026-11-25T18:56:26.573Z"});
    save_rows(
        &path,
        &[
            row("session.start", json!({"sessionId":NATIVE})),
            row("user.message", json!({"content":"Remove the folder"})),
            row(
                "permission.requested",
                json!({"requestId":"r1","permissionRequest":{"kind":"shell","fullCommandText":"rm -rf probe_dir"}}),
            ),
        ],
    );
    let source = Source {
        id: "copilot-source".into(),
        provider: "copilot".into(),
        path: root.to_string_lossy().into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let reason = |id: &str| -> String {
        store
            .db
            .query_row(
                "SELECT reason FROM channel_commands WHERE reply_to=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    // The record shows an open permission although the session's kind was not reported.
    let mut s = live(&source, Phase::NeedsInput);
    s.path = path.to_string_lossy().into();
    s.pid = Some(std::process::id());
    s.process_start = Some(1);
    let target = observe(&store, s.clone());
    ingest(
        &store,
        incoming("typed", &format!("/send {} y", target.key), ""),
    );
    assert_eq!(command_status(&store, "typed"), "blocked");
    assert!(
        reason("typed").contains("原会话正在等待选择或确认，终端输入无法可靠送达，请在原窗口处理")
    );
    // A question tool whose options no reader lists opens a menu all the same.
    std::fs::remove_file(&path).unwrap();
    s.input_kind = "question".into();
    s.input_request = "ask".into();
    let target = observe(&store, s);
    ingest(
        &store,
        incoming("menu", &format!("/send {} 1", target.key), ""),
    );
    assert_eq!(command_status(&store, "menu"), "blocked");
    assert!(
        reason("menu").contains("原会话正在等待选择或确认，终端输入无法可靠送达，请在原窗口处理")
    );
}
