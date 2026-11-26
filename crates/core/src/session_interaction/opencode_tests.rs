//! OpenCode through the popup contract, against a stand-in server that serves the responses
//! captured from OpenCode v2.1.0 (`crates/core/tests/fixtures/opencode-v2.1.0`).
use super::*;
use crate::{
    models::{Appearance, Settings},
    sessions::SourceHealth,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{Arc, Mutex},
};

const NATIVE: &str = "ses_dec4df878ffe9FgvEm07re0zcf";

fn fixture(name: &str) -> Value {
    let raw = match name {
        "permission-shell" => {
            include_str!("../../tests/fixtures/opencode-v2.1.0/permission-shell.json")
        }
        "permission-edit" => {
            include_str!("../../tests/fixtures/opencode-v2.1.0/permission-edit.json")
        }
        "permissions-parallel" => {
            include_str!("../../tests/fixtures/opencode-v2.1.0/permissions-parallel.json")
        }
        "form-question" => include_str!("../../tests/fixtures/opencode-v2.1.0/form-question.json"),
        "form-fields" => include_str!("../../tests/fixtures/opencode-v2.1.0/form-fields.json"),
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str::<Value>(raw).unwrap()["data"].clone()
}

/// What the stand-in server holds and what it was asked to do.
#[derive(Default)]
struct Served {
    forms: Vec<Value>,
    permissions: Vec<Value>,
    /// A refusal for the next reply, as `(status, body)`.
    refuse: Option<(u16, Value)>,
    running: bool,
    /// The server does not know the session at all.
    stranger: bool,
    /// `(method, path with query, body)` of every request that changes something.
    calls: Vec<(String, String, Value)>,
}

type Shared = Arc<Mutex<Served>>;

/// One session behind Basic auth `opencode:secret`, with the paths and status codes of v2.1.0.
fn serve(state: Served) -> (String, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let shared = Arc::new(Mutex::new(state));
    let served = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                if stream.read_exact(&mut b).is_err() {
                    break;
                }
                head.push(b[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let mut line = head.lines().next().unwrap_or("").split(' ');
            let method = line.next().unwrap_or("").to_owned();
            let full = line.next().unwrap_or("").to_owned();
            let path = full.split('?').next().unwrap_or("");
            let header = |name: &str| {
                head.lines().find_map(|l| {
                    let (key, value) = l.split_once(':')?;
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
            let _ = stream.read_exact(&mut bytes);
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let mut state = served.lock().unwrap();
            let authorized =
                header("authorization").as_deref() == Some("Basic b3BlbmNvZGU6c2VjcmV0");
            let rest = path
                .strip_prefix(&format!("/api/session/{NATIVE}"))
                .filter(|_| !state.stranger);
            if method != "GET" && authorized {
                state.calls.push((method.clone(), full.clone(), body));
            }
            let remove = |rows: &mut Vec<Value>, id: &str| {
                let before = rows.len();
                rows.retain(|r| r["id"] != id);
                rows.len() < before
            };
            let (code, reply) = match (method.as_str(), rest) {
                _ if !authorized => (
                    401,
                    json!({"_tag":"UnauthorizedError","message":"Authentication required"}),
                ),
                ("GET", _) if path == "/api/session/active" => (
                    200,
                    if state.running && !state.stranger {
                        json!({"data":{NATIVE:{"type":"running"}}})
                    } else {
                        json!({"data":{}})
                    },
                ),
                ("GET", Some("")) => (200, json!({"data":{"id":NATIVE}})),
                ("GET", Some("/form")) => (200, json!({"data":state.forms})),
                ("GET", Some("/permission")) => (200, json!({"data":state.permissions})),
                ("GET", Some("/message")) => (200, json!({"data":[]})),
                ("POST", Some("/prompt")) => (200, json!({"data":{"id":"msg_new"}})),
                ("POST", Some("/interrupt")) => {
                    let was = std::mem::take(&mut state.running);
                    state.permissions.clear();
                    (200, json!({"interrupted":was}))
                }
                ("POST", Some(rest)) if rest.ends_with("/reply") => {
                    let id = rest.split('/').nth(2).unwrap_or("").to_owned();
                    if let Some(refusal) = state.refuse.take() {
                        refusal
                    } else if rest.starts_with("/form/") {
                        if remove(&mut state.forms, &id) {
                            (204, Value::Null)
                        } else {
                            (409, json!({"_tag":"FormAlreadySettledError"}))
                        }
                    } else if remove(&mut state.permissions, &id) {
                        (204, Value::Null)
                    } else {
                        (404, json!({"_tag":"PermissionNotFoundError"}))
                    }
                }
                ("DELETE", Some(rest)) if rest.starts_with("/form/") => {
                    let id = rest.split('/').nth(2).unwrap_or("").to_owned();
                    if remove(&mut state.forms, &id) {
                        (204, Value::Null)
                    } else {
                        (409, json!({"_tag":"FormAlreadySettledError"}))
                    }
                }
                _ => (404, json!({"_tag":"SessionNotFoundError"})),
            };
            drop(state);
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
    (url, shared)
}

struct World {
    _dir: tempfile::TempDir,
    store: Store,
    source: Source,
    live: LiveSession,
    server: Shared,
}

impl World {
    fn inspect(&self, extra: Value) -> Value {
        let mut params = json!({"sessionId": self.live.id});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        call(&self.store, "sessions.interaction.inspect", params).unwrap()
    }
    fn respond(&self, card: &Value, extra: Value) -> Value {
        let mut params = json!({
            "sessionId": card["sessionId"],
            "stateKey": card["stateKey"],
            "promptHash": card["promptHash"],
        });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        call(&self.store, "sessions.interaction.respond", params).unwrap()
    }
    fn calls(&self) -> Vec<(String, String, Value)> {
        std::mem::take(&mut self.server.lock().unwrap().calls)
    }
    fn last_call(&self) -> (String, String, Value) {
        self.calls().pop().expect("the server received a request")
    }
}

fn source_at(dir: &Path) -> Source {
    let share = dir.join("xdg/share/opencode");
    std::fs::create_dir_all(&share).unwrap();
    std::fs::create_dir_all(dir.join("xdg/state/opencode")).unwrap();
    std::fs::write(share.join("opencode.db"), b"").unwrap();
    Source {
        id: "opencode-src".into(),
        provider: "opencode".into(),
        path: share.to_string_lossy().into(),
        ..Default::default()
    }
}

/// A store that has observed one local OpenCode session of `source`.
fn observed(
    dir: &Path,
    source: &Source,
    native: &str,
    phase: Phase,
    language: &str,
) -> (Store, LiveSession) {
    let store = Store::open(&dir.join("data")).unwrap();
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            appearance: Appearance {
                language: language.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    let live = LiveSession {
        id: crate::models::hash(&format!("local:opencode:{native}")),
        source: "opencode".into(),
        source_id: source.id.clone(),
        provider: "opencode".into(),
        machine_id: "local".into(),
        machine: r"本机".into(),
        turn_id: "turn-1".into(),
        phase,
        updated_at: now() as u64,
        event_at: (now() * 1000) as u64,
        native_session: native.into(),
        ..Default::default()
    };
    observe(
        &store,
        &Snapshot {
            sessions: vec![live.clone()],
            sources: vec![SourceHealth {
                source_id: source.id.clone(),
                machine_id: "local".into(),
                state: "ready".into(),
                checked_at: now() as u64,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    (store, live)
}

/// A session of a registered background service. `registered: false` leaves out `service.json`,
/// as a server started by hand does.
fn world_with(state: Served, phase: Phase, language: &str, registered: bool) -> (World, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = source_at(dir.path());
    let (url, server) = serve(state);
    if registered {
        std::fs::write(
            dir.path().join("xdg/state/opencode/service.json"),
            json!({"id":"svc","version":"2.1.0","url":url,"pid":1,"password":"secret"}).to_string(),
        )
        .unwrap();
    }
    let (store, live) = observed(dir.path(), &source, NATIVE, phase, language);
    (
        World {
            _dir: dir,
            store,
            source,
            live,
            server,
        },
        url,
    )
}

fn world(state: Served, phase: Phase) -> World {
    world_with(state, phase, "zh-CN", true).0
}

fn permissions(name: &str) -> Served {
    Served {
        permissions: fixture(name).as_array().unwrap().clone(),
        running: true,
        ..Default::default()
    }
}

fn forms(name: &str) -> Served {
    Served {
        forms: fixture(name).as_array().unwrap().clone(),
        running: true,
        ..Default::default()
    }
}

fn decision_ids(card: &Value) -> Vec<&str> {
    card["approval"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["id"].as_str())
        .collect()
}

#[test]
fn a_real_shell_permission_can_be_allowed_always_with_the_saved_patterns_shown() {
    let w = world(permissions("permission-shell"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "approval");
    assert_eq!(card["adapter"], "opencode");
    assert_eq!(card["inputKind"], "approval");
    assert_eq!(card["requestId"], "per_213b2cf6a001atszzmoQHG52Wk");
    assert_eq!(card["pendingCount"], 1);
    let approval = &card["approval"];
    assert_eq!(approval["toolName"], "Shell");
    assert_eq!(approval["kind"], "command");
    assert_eq!(approval["command"], "echo probe > out.txt\nls -la");
    assert_eq!(approval["reason"], r"请求执行命令");
    assert_eq!(decision_ids(&card), ["approve", "approveAlways", "decline"]);
    let always = &approval["decisions"][1];
    assert_eq!(always["scope"], "always");
    assert_eq!(always["behavior"], "allow");
    assert_eq!(always["description"], "echo *\nls *");
    assert_eq!(approval["decisions"][2]["acceptsReason"], true);
    assert_eq!(approval["decisions"][0]["acceptsReason"], false);

    // Feedback belongs to a rejection; nothing is sent when it is attached to an approval.
    let refused = w.respond(&card, json!({"action":"approveAlways","reason":"sure"}));
    assert_eq!(refused["status"], "blocked");
    // A decision the server does not know is never forwarded.
    let unknown = w.respond(&card, json!({"action":"approveSession"}));
    assert_eq!(unknown["status"], "blocked");
    assert!(w.calls().is_empty());

    let done = w.respond(
        &card,
        json!({"action":"decide","decisionId":"approveAlways"}),
    );
    assert_eq!(done["status"], "confirmed", "{done}");
    assert_eq!(
        done["receipt"]["requestId"],
        "per_213b2cf6a001atszzmoQHG52Wk"
    );
    let (method, path, body) = w.last_call();
    assert_eq!(method, "POST");
    assert_eq!(
        path,
        format!("/api/session/{NATIVE}/permission/per_213b2cf6a001atszzmoQHG52Wk/reply")
    );
    assert_eq!(body, json!({"decision":"always"}));
}

#[test]
fn a_real_edit_permission_shows_the_patch_and_a_rejection_carries_the_reason() {
    let w = world(permissions("permission-edit"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    let approval = &card["approval"];
    assert_eq!(approval["toolName"], "Edit");
    assert_eq!(approval["kind"], "fileChange");
    assert_eq!(approval["title"], "Edit · a.txt");
    assert_eq!(approval["reason"], r"请求修改文件");
    assert_eq!(approval["paths"], json!(["a.txt"]));
    let change = &approval["changes"][0];
    assert_eq!(change["path"], "a.txt");
    assert_eq!(change["kind"], "update");
    assert!(
        change["diff"]
            .as_str()
            .unwrap()
            .contains("-hello\n+goodbye\n")
    );
    assert_eq!(change["truncated"], false);
    // "Always" would allow every edit; the pattern is on the button's description.
    assert_eq!(approval["decisions"][1]["description"], "*");

    let done = w.respond(
        &card,
        json!({"action":"decline","reason":"  Do not edit a.txt; write b.txt instead.  "}),
    );
    assert_eq!(done["status"], "confirmed", "{done}");
    assert_eq!(
        w.last_call().2,
        json!({"decision":"reject","message":"Do not edit a.txt; write b.txt instead."})
    );
    // The request is gone: a second reply reaches nothing.
    let again = w.respond(&card, json!({"action":"approve"}));
    assert_eq!(again["status"], "blocked");
    assert!(w.calls().is_empty());
}

#[test]
fn parallel_permissions_are_counted_and_each_reply_reaches_the_request_it_names() {
    let w = world(permissions("permissions-parallel"), Phase::NeedsInput);
    let (first, second) = (
        "per_213b7ee470014WOOaCW00XFocx",
        "per_213b7ee49001wRHGbaQiBtkskg",
    );
    let card = w.inspect(json!({}));
    assert_eq!(card["pendingCount"], 2);
    assert_eq!(card["requestId"], first);
    assert_eq!(card["approval"]["command"], "mkdir -p d1");
    assert_eq!(
        card["pendingRequests"],
        json!([
            {"requestId": first, "kind": "approval", "title": "Shell · mkdir -p d1"},
            {"requestId": second, "kind": "approval", "title": "Shell · touch t1.txt"},
        ])
    );

    // Step to the second request.
    let next = w.inspect(json!({"requestId": second}));
    assert_eq!(next["requestId"], second);
    assert_eq!(next["approval"]["command"], "touch t1.txt");
    assert_eq!(next["pendingCount"], 2);
    assert_ne!(next["promptHash"], card["promptHash"]);
    // An id that is not pending falls back to the oldest request.
    assert_eq!(
        w.inspect(json!({"requestId": "per_gone"}))["requestId"],
        first
    );

    // The second card's hash without its id would be the first request: refused.
    let unnamed = w.respond(&next, json!({"action":"approve"}));
    assert_eq!(unnamed["status"], "blocked");
    // The first card's hash with the second id: refused as well.
    let crossed = w.respond(&card, json!({"action":"approve","requestId": second}));
    assert_eq!(crossed["status"], "blocked");
    let gone = w.respond(&card, json!({"action":"approve","requestId":"per_gone"}));
    assert_eq!(gone["status"], "blocked");
    assert!(w.calls().is_empty());

    let done = w.respond(&next, json!({"action":"decline","requestId": second}));
    assert_eq!(done["status"], "confirmed", "{done}");
    let (_, path, body) = w.last_call();
    assert!(path.ends_with(&format!("/permission/{second}/reply")));
    assert_eq!(body, json!({"decision":"reject"}));

    let left = w.inspect(json!({}));
    assert_eq!(left["pendingCount"], 1);
    assert_eq!(left["requestId"], first);
    assert_eq!(left["promptHash"], card["promptHash"]);
}

#[test]
fn a_real_question_form_keeps_headers_allows_partial_answers_and_can_be_dismissed() {
    let w = world(forms("form-question"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "questions");
    assert_eq!(card["inputKind"], "question");
    assert_eq!(card["requestId"], "frm_213b700bb001BxQGhiv6V84cTy");
    assert_eq!(
        card["form"],
        json!({"title":"Questions","description":"","source":"tool","serverName":"","url":"",
               "canCancel":true,"canSkip":false})
    );
    let q = &card["questions"];
    assert_eq!(q[0]["header"], "Color");
    assert_eq!(q[0]["question"], "Which color do you prefer?");
    assert_eq!(q[0]["kind"], "string");
    assert_eq!(q[0]["allowCustom"], true);
    assert_eq!(q[0]["required"], false);
    assert_eq!(q[0]["options"][0]["description"], "Vibrant red");
    assert_eq!(q[1]["header"], "Toppings");
    assert_eq!(q[1]["multiSelect"], true);

    // There is no separate "decline" for an OpenCode form.
    assert_eq!(
        w.respond(&card, json!({"action":"skip"}))["status"],
        "blocked"
    );
    assert!(w.calls().is_empty());

    // The original form accepts a partial answer; unanswered questions are left out.
    let done = w.respond(&card, json!({"answers":{"1":"Teal (custom)","2":[]}}));
    assert_eq!(done["status"], "confirmed", "{done}");
    let (method, path, body) = w.last_call();
    assert_eq!(method, "POST");
    assert!(path.ends_with("/form/frm_213b700bb001BxQGhiv6V84cTy/reply"));
    assert_eq!(body, json!({"answer":{"q0":"Teal (custom)"}}));

    let w = world(forms("form-question"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    let done = w.respond(
        &card,
        json!({"answers":{"q0":"red","q1":["Cheese","Olive"]}}),
    );
    assert_eq!(done["status"], "confirmed", "{done}");
    assert_eq!(
        w.last_call().2,
        json!({"answer":{"q0":"Red","q1":["Cheese","Olive"]}})
    );

    let w = world(forms("form-question"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    let done = w.respond(&card, json!({"action":"cancel"}));
    assert_eq!(done["status"], "confirmed", "{done}");
    let (method, path, body) = w.last_call();
    assert_eq!(method, "DELETE");
    assert!(path.starts_with(&format!(
        "/api/session/{NATIVE}/form/frm_213b700bb001BxQGhiv6V84cTy?message=The%20user"
    )));
    assert!(body.is_null());
    // Already dismissed: nothing is pending any more.
    assert_eq!(
        w.respond(&card, json!({"action":"cancel"}))["status"],
        "blocked"
    );
}

#[test]
fn form_fields_carry_constraints_and_localised_booleans_and_are_checked_before_sending() {
    let w = world(forms("form-fields"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    assert_eq!(card["form"]["title"], "Deploy settings");
    assert_eq!(card["form"]["source"], "agent");
    let q = card["questions"].as_array().unwrap();
    let by_key = |key: &str| q.iter().find(|row| row["key"] == key).unwrap();
    let env = by_key("env");
    assert_eq!(env["header"], "Environment");
    assert_eq!(env["required"], true);
    assert_eq!(env["default"], "staging");
    assert_eq!(env["allowCustom"], false);
    assert_eq!(env["options"][0]["value"], "staging");
    let note = by_key("note");
    // A title without a description is the question itself.
    assert_eq!(
        (&note["header"], &note["question"]),
        (&json!(""), &json!("Note"))
    );
    assert_eq!(note["required"], false);
    assert_eq!(note["placeholder"], "lowercase");
    assert_eq!(
        note["constraints"],
        json!({"minLength":2,"maxLength":10,"pattern":"^[a-z]+$"})
    );
    let replicas = by_key("replicas");
    assert_eq!(replicas["kind"], "integer");
    assert_eq!(replicas["default"], 2);
    assert_eq!(replicas["constraints"], json!({"minimum":1,"maximum":5}));
    assert_eq!(by_key("ratio")["question"], "ratio");
    let confirm = by_key("confirm");
    assert_eq!(confirm["kind"], "boolean");
    assert_eq!(confirm["default"], false);
    assert_eq!(
        confirm["options"],
        json!([{"label":r"是","value":true},{"label":r"否","value":false}])
    );
    assert_eq!(
        by_key("regions")["constraints"],
        json!({"minItems":1,"maxItems":2})
    );

    let blocked = |answers: Value| {
        let out = w.respond(&card, json!({"answers": answers}));
        assert_eq!(out["status"], "blocked", "{out}");
        out["message"].as_str().unwrap().to_owned()
    };
    // A required field is missing.
    assert_eq!(
        blocked(json!({"replicas":2,"confirm":true})),
        r"请回答所有问题后再提交"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":9,"confirm":true})),
        r"数值超出允许范围"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":"2.5","confirm":true})),
        r"请输入有效的整数"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":2,"confirm":true,"ratio":"1.5"})),
        r"数值超出允许范围"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":2,"confirm":true,"note":"x"})),
        r"文本长度不符合要求"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":2,"confirm":true,"regions":["us","eu","ap"]})),
        r"选择的数量不符合要求"
    );
    assert_eq!(
        blocked(json!({"env":"moon","replicas":2,"confirm":true})),
        r"回答不在可选项中，请重新选择"
    );
    assert_eq!(
        blocked(json!({"env":"staging","replicas":2,"confirm":"maybe"})),
        r"回答不在可选项中，请重新选择"
    );
    assert!(w.calls().is_empty());

    // What only the server can check comes back in its own words.
    w.server.lock().unwrap().refuse = Some((
        400,
        json!({"_tag":"FormInvalidAnswerError","id":"frm_213b5fb81001m3qWHdVLE04dS9",
               "message":"Form field does not match pattern: note"}),
    ));
    let refused = w.respond(
        &card,
        json!({"answers":{"env":"Staging","replicas":2,"confirm":true,"note":"ABC"}}),
    );
    assert_eq!(refused["status"], "blocked");
    assert_eq!(
        refused["message"],
        r"原会话拒绝了指令：Form field does not match pattern: note"
    );
    w.calls();

    w.server.lock().unwrap().refuse = Some((
        401,
        json!({"_tag":"UnauthorizedError","message":"Authentication required"}),
    ));
    let unauthorized = w.respond(
        &card,
        json!({"answers":{"env":"Staging","replicas":2,"confirm":true}}),
    );
    assert_eq!(unauthorized["status"], "blocked");
    assert!(
        unauthorized["message"]
            .as_str()
            .unwrap()
            .contains(r"口令已变化")
    );
    w.calls();

    // Labels become the server's values; booleans and numbers are typed; optional gaps are left out.
    let done = w.respond(
        &card,
        json!({"answers":{"env":"Production","replicas":"3","ratio":0.5,"confirm":r"否",
                          "regions":["US","eu"],"note":""}}),
    );
    assert_eq!(done["status"], "confirmed", "{done}");
    assert_eq!(
        w.last_call().2,
        json!({"answer":{"env":"production","replicas":3,"ratio":0.5,"confirm":false,
                         "regions":["us","eu"]}})
    );
}

#[test]
fn boolean_labels_follow_the_app_language_and_refusals_are_translated() {
    let (w, _) = world_with(forms("form-fields"), Phase::NeedsInput, "en", true);
    let card = w.inspect(json!({}));
    let confirm = card["questions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == "confirm")
        .unwrap();
    assert_eq!(
        confirm["options"],
        json!([{"label":"Yes","value":true},{"label":"No","value":false}])
    );
    w.server.lock().unwrap().refuse = Some((
        400,
        json!({"_tag":"FormInvalidAnswerError","message":"Unknown form field: zz"}),
    ));
    let refused = w.respond(
        &card,
        json!({"answers":{"env":"staging","replicas":2,"confirm":"Yes"}}),
    );
    assert_eq!(refused["status"], "blocked");
    let message = refused["message"].as_str().unwrap();
    assert!(message.ends_with("Unknown form field: zz"), "{message}");
    assert!(!message.contains(r"原会话"), "{message}");
    assert_eq!(
        w.last_call().2,
        json!({"answer":{"env":"staging","replicas":2,"confirm":true}})
    );

    let (w, _) = world_with(
        permissions("permission-shell"),
        Phase::NeedsInput,
        "en",
        true,
    );
    let card = w.inspect(json!({}));
    assert_eq!(card["approval"]["decisions"][1]["label"], "Always allow");
    assert_ne!(card["approval"]["reason"], r"请求执行命令");
}

#[test]
fn a_running_session_queues_a_follow_up_and_can_be_interrupted() {
    let w = world(
        Served {
            running: true,
            ..Default::default()
        },
        Phase::Tool,
    );
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "followup");
    assert_eq!(card["adapter"], "opencode");
    assert_eq!(card["canRespond"], true);
    assert_eq!(
        card["followUp"],
        json!({"available":true,"adapter":"opencode","mode":"queue","reason":"","canInterrupt":true})
    );

    let queued = w.respond(
        &card,
        json!({"action":"followup","text":"  then run the tests  "}),
    );
    assert_eq!(queued["status"], "confirmed", "{queued}");
    assert_eq!(queued["receipt"]["delivery"], "queue");
    let (method, path, body) = w.last_call();
    assert_eq!(
        (method.as_str(), path),
        ("POST", format!("/api/session/{NATIVE}/prompt"))
    );
    // Never steered into the running turn.
    assert_eq!(
        body,
        json!({"text":"then run the tests","delivery":"queue"})
    );

    assert_eq!(
        w.respond(&card, json!({"action":"followup","text":"   "}))["status"],
        "blocked"
    );
    // A button that is not an answer has nothing to act on.
    assert_eq!(
        w.respond(&card, json!({"action":"approve"}))["status"],
        "blocked"
    );
    assert!(w.calls().is_empty());

    let stopped = w.respond(&card, json!({"action":"interrupt"}));
    assert_eq!(stopped["status"], "confirmed", "{stopped}");
    let (method, path, _) = w.last_call();
    assert_eq!(
        (method.as_str(), path),
        ("POST", format!("/api/session/{NATIVE}/interrupt"))
    );
    // Nothing is running any more.
    let idle = w.respond(&card, json!({"action":"interrupt"}));
    assert_eq!(idle["status"], "blocked");
    assert_eq!(idle["message"], r"原会话当前没有正在执行的任务");
}

#[test]
fn a_finished_session_takes_a_plain_follow_up_and_offers_no_interrupt() {
    let w = world(Served::default(), Phase::Complete);
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "followup");
    assert_eq!(card["followUp"]["mode"], "newTurn");
    assert_eq!(card["followUp"]["canInterrupt"], false);
    let sent = w.respond(&card, json!({"action":"followup","text":"next step"}));
    assert_eq!(sent["status"], "confirmed", "{sent}");
    assert_eq!(w.last_call().2, json!({"text":"next step"}));
    let stopped = w.respond(&card, json!({"action":"interrupt"}));
    assert_eq!(stopped["status"], "blocked");
    assert!(w.calls().is_empty());
}

#[test]
fn a_waiting_session_can_be_interrupted_but_takes_no_follow_up() {
    let w = world(permissions("permissions-parallel"), Phase::NeedsInput);
    let card = w.inspect(json!({}));
    assert_eq!(card["followUp"]["available"], false);
    assert_eq!(card["followUp"]["canInterrupt"], true);
    let follow = w.respond(&card, json!({"action":"followup","text":"hello"}));
    assert_eq!(follow["status"], "blocked");
    // Text is not a decision: it never reaches a permission request.
    let text = w.respond(&card, json!({"text":"yes"}));
    assert_eq!(text["status"], "blocked");
    assert!(w.calls().is_empty());
    let stopped = w.respond(&card, json!({"action":"interrupt"}));
    assert_eq!(stopped["status"], "confirmed", "{stopped}");
    assert!(w.last_call().1.ends_with("/interrupt"));
    assert!(w.server.lock().unwrap().permissions.is_empty());
}

#[test]
fn interrupt_is_refused_for_sessions_no_opencode_server_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("data")).unwrap();
    let home = dir.path().join("kimi");
    std::fs::create_dir_all(&home).unwrap();
    let source = Source {
        id: "kimi-src".into(),
        provider: "kimi".into(),
        path: home.to_string_lossy().into(),
        ..Default::default()
    };
    store
        .save_settings(&Settings {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .unwrap();
    let live = LiveSession {
        id: "kimi-live".into(),
        source: "kimi".into(),
        source_id: source.id.clone(),
        provider: "kimi".into(),
        machine_id: "local".into(),
        turn_id: "turn-1".into(),
        phase: Phase::Working,
        updated_at: now() as u64,
        native_session: "native".into(),
        ..Default::default()
    };
    observe(
        &store,
        &Snapshot {
            sessions: vec![live.clone()],
            sources: vec![SourceHealth {
                source_id: source.id.clone(),
                machine_id: "local".into(),
                state: "ready".into(),
                checked_at: now() as u64,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let card = call(
        &store,
        "sessions.interaction.inspect",
        json!({"sessionId": live.id}),
    )
    .unwrap();
    assert_eq!(card["followUp"]["canInterrupt"], false);
    let out = call(
        &store,
        "sessions.interaction.respond",
        json!({"sessionId": live.id, "stateKey": card["stateKey"], "action": "interrupt"}),
    )
    .unwrap();
    assert_eq!(out["status"], "blocked");
    assert_eq!(out["message"], r"此会话不支持在弹窗中断，请回到原窗口处理");
}

#[test]
fn a_hand_started_server_is_used_only_with_its_password_and_only_for_its_own_sessions() {
    let (w, url) = world_with(
        permissions("permission-shell"),
        Phase::NeedsInput,
        "zh-CN",
        false,
    );
    // No registration and nothing discovered: the session is not reachable.
    opencode::set_discovered_for_test(&[]);
    assert_eq!(opencode::probe_pending_id(&w.source, NATIVE), None);
    assert_ne!(w.inspect(json!({}))["adapter"], "opencode");

    // Found, but Aieyes holds a different password: still not reachable, and nothing is sent twice.
    opencode::set_discovered_for_test(&[(&url, "other")]);
    assert_eq!(opencode::probe_pending_id(&w.source, NATIVE), None);
    assert_ne!(w.inspect(json!({}))["adapter"], "opencode");

    // A second server that does not know the session is skipped.
    let (stranger, _) = serve(Served {
        stranger: true,
        ..Default::default()
    });
    opencode::set_discovered_for_test(&[(&stranger, "secret"), (&url, "secret")]);
    assert_eq!(
        opencode::probe_pending_id(&w.source, NATIVE),
        Some(("permission".into(), "per_213b2cf6a001atszzmoQHG52Wk".into()))
    );
    assert_eq!(opencode::probe_pending_id(&w.source, "ses_other"), None);
    let card = w.inspect(json!({}));
    assert_eq!(card["adapter"], "opencode");
    assert_eq!(card["kind"], "approval");
    let done = w.respond(&card, json!({"action":"approve"}));
    assert_eq!(done["status"], "confirmed", "{done}");
    assert_eq!(w.last_call().2, json!({"decision":"once"}));
    opencode::set_discovered_for_test(&[]);
}

/// Every server reads the same database and so shows every session; only the process that
/// runs a session holds its pending requests and its turn.
#[test]
fn among_servers_that_show_a_session_the_one_that_runs_it_is_used() {
    let (w, running) = world_with(
        permissions("permissions-parallel"),
        Phase::NeedsInput,
        "zh-CN",
        false,
    );
    // The registered service knows the session but is not running it.
    let (service, idle) = serve(Served::default());
    std::fs::write(
        w._dir.path().join("xdg/state/opencode/service.json"),
        json!({"id":"svc","version":"2.1.0","url":service,"pid":1,"password":"secret"}).to_string(),
    )
    .unwrap();
    opencode::set_discovered_for_test(&[(&running, "secret")]);
    let ids: Vec<String> = fixture("permissions-parallel")
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        opencode::probe_pending_id(&w.source, NATIVE),
        Some(("permission".into(), ids[0].clone()))
    );
    let card = w.inspect(json!({"requestId": ids[1]}));
    assert_eq!(card["adapter"], "opencode");
    assert_eq!(card["requestId"], ids[1].as_str());
    assert_eq!(card["pendingCount"], 2);
    let done = w.respond(&card, json!({"action":"approve","requestId":ids[1]}));
    assert_eq!(done["status"], "confirmed", "{done}");
    assert!(w.last_call().1.contains(&ids[1]));
    assert!(idle.lock().unwrap().calls.is_empty());

    // With nothing pending, the running turn still names the process to stop.
    w.server.lock().unwrap().permissions.clear();
    let rephase = |phase: Phase| {
        let mut live = w.live.clone();
        live.phase = phase;
        observe(
            &w.store,
            &Snapshot {
                sessions: vec![live],
                sources: vec![SourceHealth {
                    source_id: w.source.id.clone(),
                    machine_id: "local".into(),
                    state: "ready".into(),
                    checked_at: now() as u64,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    };
    rephase(Phase::Working);
    let card = w.inspect(json!({}));
    let out = w.respond(&card, json!({"action":"interrupt"}));
    assert_eq!(out["status"], "confirmed", "{out}");
    assert!(w.last_call().1.ends_with("/interrupt"));
    assert!(idle.lock().unwrap().calls.is_empty());

    // An idle session has neither: the follow-up goes to the registered service, and an
    // interrupt reports that nothing is running.
    rephase(Phase::Complete);
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "followup");
    let out = w.respond(&card, json!({"action":"followup","text":"next"}));
    assert_eq!(out["status"], "confirmed", "{out}");
    let sent = std::mem::take(&mut idle.lock().unwrap().calls);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.ends_with("/prompt"));
    assert!(w.calls().is_empty());
    opencode::set_discovered_for_test(&[]);
}

#[test]
fn the_oldest_request_is_shown_first_whatever_its_type() {
    // The edit permission (213b4c…) was asked before the form (213b5f…).
    let mut state = permissions("permission-edit");
    state.forms = fixture("form-fields").as_array().unwrap().clone();
    let w = world(state, Phase::NeedsInput);
    assert_eq!(
        opencode::probe_pending_id(&w.source, NATIVE),
        Some(("permission".into(), "per_213b4cf76001B93qRzq0S6iUGz".into()))
    );
    let card = w.inspect(json!({}));
    assert_eq!(card["kind"], "approval");
    assert_eq!(card["pendingCount"], 2);
    assert_eq!(card["pendingRequests"][1]["kind"], "questions");
    assert_eq!(card["pendingRequests"][1]["title"], "Deploy settings");
    // A decision id sent to the form, or answers sent to the permission, go nowhere.
    let form = w.inspect(json!({"requestId":"frm_213b5fb81001m3qWHdVLE04dS9"}));
    assert_eq!(form["kind"], "questions");
    let wrong = w.respond(
        &form,
        json!({"action":"approve","requestId":"frm_213b5fb81001m3qWHdVLE04dS9"}),
    );
    assert_eq!(wrong["status"], "blocked");
    let wrong = w.respond(&card, json!({"answers":{"env":"staging"}}));
    assert_eq!(wrong["status"], "blocked");
    assert!(w.calls().is_empty());
}

/// Manual check against a real `opencode serve` (see docs/manual-verification.md). Set
/// `AIEYES_OPENCODE_LIVE` to `url|password|sessionId|phase` and optionally
/// `AIEYES_OPENCODE_LIVE_REPLY` to the extra respond parameters as JSON. Prints the card and
/// the outcome; nothing is sent without the second variable.
#[test]
#[ignore = "needs a running OpenCode server"]
fn live_server_card_and_reply() {
    let Ok(spec) = std::env::var("AIEYES_OPENCODE_LIVE") else {
        return;
    };
    let parts: Vec<&str> = spec.split('|').collect();
    let [url, password, native, phase] = parts.as_slice() else {
        panic!("AIEYES_OPENCODE_LIVE must be url|password|sessionId|phase");
    };
    let phase = match *phase {
        "needsInput" => Phase::NeedsInput,
        "tool" => Phase::Tool,
        "complete" => Phase::Complete,
        other => panic!("unknown phase {other}"),
    };
    let dir = tempfile::tempdir().unwrap();
    let source = source_at(dir.path());
    let (store, live) = observed(dir.path(), &source, native, phase, "en");
    opencode::set_discovered_for_test(&[(url, password)]);
    let w = World {
        _dir: dir,
        store,
        source,
        live,
        server: Shared::default(),
    };
    println!("probe: {:?}", opencode::probe_pending_id(&w.source, native));
    let card = w.inspect(json!({}));
    println!("card: {}", serde_json::to_string_pretty(&card).unwrap());
    if let Ok(reply) = std::env::var("AIEYES_OPENCODE_LIVE_REPLY") {
        let extra: Value = serde_json::from_str(&reply).unwrap();
        println!("outcome: {}", w.respond(&card, extra));
    }
}
