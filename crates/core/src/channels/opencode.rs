//! OpenCode's background service keeps pending questions (forms) and permission requests
//! behind an authenticated loopback API. Replies address the original request identifier.
//! A `--standalone` process is not served by it and keeps the terminal path.
use super::{Session, codex, configuration, content, control, control::Outcome, db::MAX_INPUT};
use crate::{
    models::{Source, now},
    sessions::Phase,
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::Method;
use rusqlite::params;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

pub(super) const FORM: &str = "aieyes/opencodeForm";
pub(super) const PERMISSION: &str = "aieyes/opencodePermission";
const UNSUPPORTED: &str = "aieyes/opencodeUnsupported";

struct Client {
    http: reqwest::blocking::Client,
    url: String,
    password: String,
}

/// The service registration of the installation that owns this source's database.
fn registration(source: &Source) -> Option<PathBuf> {
    if source.provider != "opencode" || source.host_id.is_some() {
        return None;
    }
    let database = crate::opencode::database(&source.path)?;
    let folder = database.parent()?;
    let mut candidates = Vec::new();
    // XDG default layout: <base>/share/opencode beside <base>/state/opencode.
    if folder.file_name().is_some_and(|n| n == "opencode")
        && let Some(share) = folder.parent()
        && share.file_name().is_some_and(|n| n == "share")
        && let Some(base) = share.parent()
    {
        candidates.push(base.join("state/opencode/service.json"));
    }
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        candidates.push(PathBuf::from(state).join("opencode/service.json"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Identifiers are interpolated into request paths.
fn id(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Client {
    fn open(source: &Source) -> Result<Self> {
        let path = registration(source).context("未找到 OpenCode 后台服务")?;
        let bytes = std::fs::read(path)?;
        ensure!(bytes.len() <= 65536, "OpenCode 后台服务登记无效");
        let value: Value = serde_json::from_slice(&bytes)?;
        let url = reqwest::Url::parse(value["url"].as_str().unwrap_or(""))
            .ok()
            .context("OpenCode 后台服务登记无效")?;
        // The service password must never leave this machine.
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none(),
            "OpenCode 后台服务登记无效"
        );
        let password = value["password"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("OpenCode 后台服务登记无效")?;
        Ok(Self {
            http: reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(4))
                .build()?,
            url: url.origin().ascii_serialization(),
            password: password.into(),
        })
    }

    fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<(u16, Value)> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.url))
            .basic_auth("opencode", Some(&self.password));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        let bytes = response.bytes()?;
        ensure!(bytes.len() <= 8 << 20, "OpenCode 后台服务响应过大");
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }

    /// `false` when this service does not own the session (for example a standalone process).
    fn owns(&self, target: &Session) -> Result<bool> {
        let native = &target.session.native_session;
        ensure!(id(native, "ses_"), "原生会话标识无效");
        let (status, value) = self.call(Method::GET, &format!("/api/session/{native}"), None)?;
        Ok(status == 200 && value["data"]["id"].as_str() == Some(native))
    }

    fn permission(&self, target: &Session) -> Result<Option<Value>> {
        let native = &target.session.native_session;
        ensure!(id(native, "ses_"), "原生会话标识无效");
        let (status, value) = self.call(
            Method::GET,
            &format!("/api/session/{native}/permission"),
            None,
        )?;
        ensure!(status == 200, "无法读取 OpenCode 的待处理请求");
        Ok(value["data"]
            .as_array()
            .and_then(|rows| rows.first())
            .map(|row| permission_request(target, row)))
    }

    fn pending(&self, target: &Session) -> Result<Option<Value>> {
        let native = &target.session.native_session;
        ensure!(id(native, "ses_"), "原生会话标识无效");
        let (status, value) =
            self.call(Method::GET, &format!("/api/session/{native}/form"), None)?;
        ensure!(status == 200, "无法读取 OpenCode 的待处理请求");
        if let Some(form) = value["data"].as_array().and_then(|rows| rows.first()) {
            return Ok(Some(form_request(target, form)));
        }
        self.permission(target)
    }

    fn answer(&self, target: &Session, request: &Value, text: &str) -> Outcome {
        let body = match answer(request, text) {
            Ok(body) => body,
            Err(error) => return Outcome::blocked(&error.to_string()),
        };
        let item = request["params"]["itemId"].as_str().unwrap_or("");
        let kind = if request["method"] == FORM {
            "form"
        } else {
            "permission"
        };
        let path = format!(
            "/api/session/{}/{kind}/{item}/reply",
            target.session.native_session
        );
        match self.call(Method::POST, &path, Some(&body)) {
            Ok((200 | 204, _)) => Outcome::confirmed(
                "原会话已接收回答",
                json!({"adapter":"opencode","requestId":item}),
            ),
            Ok((404 | 409, _)) => Outcome::blocked("原会话的问题已结束或改变，请回复最新通知"),
            Ok((status, _)) if status < 500 => Outcome::blocked("原会话拒绝了指令，请核对原窗口"),
            _ => Outcome::uncertain("回答可能已提交，请核对原会话"),
        }
    }

    fn followup(&self, target: &Session, text: &str) -> Outcome {
        let path = format!("/api/session/{}/prompt", target.session.native_session);
        match self.call(Method::POST, &path, Some(&json!({"text":text}))) {
            Ok((200, value)) if value["data"].is_object() => Outcome::confirmed(
                "原会话已接收指令",
                json!({"adapter":"opencode","messageId":value["data"]["id"]}),
            ),
            Ok((status, _)) if status < 500 => Outcome::blocked("原会话拒绝了指令，请核对原窗口"),
            _ => Outcome::uncertain("指令可能已提交，连接中断；请核对原会话，系统不会重发"),
        }
    }

    /// The newest turn only: its user input and, when it ended normally, the final text.
    fn excerpt(&self, target: &Session) -> content::Excerpt {
        let mut out = content::Excerpt::default();
        let path = format!(
            "/api/session/{}/message?limit=40&order=desc",
            target.session.native_session
        );
        let Ok((200, value)) = self.call(Method::GET, &path, None) else {
            return out;
        };
        let mut newest = true;
        for message in value["data"].as_array().into_iter().flatten() {
            match message["type"].as_str() {
                Some("user") => {
                    out.user_text =
                        content::clip(message["text"].as_str().unwrap_or(""), MAX_INPUT);
                    out.user_at = message["time"]["created"].as_i64().unwrap_or(0);
                    break;
                }
                Some("assistant") => {
                    if newest && message["finish"] == "stop" {
                        out.text = content::clip(&content::text_blocks(&message["content"]), 10000);
                    }
                    newest = false;
                }
                _ => {}
            }
        }
        out
    }
}

fn form_request(target: &Session, form: &Value) -> Value {
    let mut supported = id(form["id"].as_str().unwrap_or(""), "frm_");
    let questions = form["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|field| field["hidden"] != true)
        .enumerate()
        .map(|(index, field)| {
            let kind = field["type"].as_str().unwrap_or("");
            supported &= matches!(
                kind,
                "string" | "number" | "integer" | "boolean" | "multiselect"
            ) && field["key"].as_str().is_some_and(|k| !k.is_empty())
                && field["when"].as_array().is_none_or(Vec::is_empty);
            let options = if kind == "boolean" {
                json!([{"label":"Yes","value":true},{"label":"No","value":false}])
            } else {
                field["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| json!({"label":o["label"],"description":o["description"],"value":o["value"]}))
                    .collect()
            };
            let question = ["description", "title", "key"]
                .iter()
                .find_map(|k| field[*k].as_str().filter(|s| !s.is_empty()))
                .unwrap_or("");
            json!({"id":(index+1).to_string(),"key":field["key"],"kind":kind,"question":question,"options":options,"multiSelect":kind=="multiselect","custom":field["custom"]==true})
        })
        .collect::<Vec<_>>();
    json!({"method":if supported && !questions.is_empty(){FORM}else{UNSUPPORTED},"params":{"threadId":target.session.native_session,"turnId":target.session.turn_id,"itemId":form["id"],"questions":questions}})
}

fn permission_request(target: &Session, row: &Value) -> Value {
    let resources = row["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let action = row["action"].as_str().unwrap_or("");
    let reason = match row["message"].as_str().filter(|s| !s.is_empty()) {
        Some(message) => format!("{action}: {message}"),
        None => action.to_owned(),
    };
    let supported = id(row["id"].as_str().unwrap_or(""), "per");
    json!({"method":if supported{PERMISSION}else{UNSUPPORTED},"params":{"threadId":target.session.native_session,"turnId":target.session.turn_id,"itemId":row["id"],"command":resources,"reason":reason}})
}

/// The service accepts incomplete answers, so every visible field is checked here.
pub(super) fn answer(request: &Value, text: &str) -> Result<Value> {
    let p = &request["params"];
    match request["method"].as_str().unwrap_or("") {
        FORM => {
            let questions = p["questions"].as_array().context("原会话问题已不可用")?;
            ensure!(!questions.is_empty(), "原会话问题已不可用");
            let mut answer = serde_json::Map::new();
            for q in questions {
                let number = q["id"].as_str().context("原会话问题缺少标识")?;
                let key = q["key"].as_str().context("原会话问题缺少标识")?;
                let reply = if questions.len() == 1 {
                    text.trim()
                } else {
                    let matches = text
                        .lines()
                        .filter_map(|line| line.split_once('='))
                        .filter(|(k, _)| k.trim() == number)
                        .map(|(_, v)| v.trim())
                        .collect::<Vec<_>>();
                    ensure!(matches.len() == 1, "请按「问题编号=答案」逐行回答所有问题");
                    matches[0]
                };
                ensure!(!reply.is_empty(), "问题回答不能为空");
                let options = q["options"].as_array().map_or(&[][..], Vec::as_slice);
                // A number selects an option; other text is an option or an allowed custom value.
                let pick = |s: &str| -> Result<Value> {
                    let option = s
                        .parse::<usize>()
                        .ok()
                        .filter(|n| *n > 0)
                        .and_then(|n| options.get(n - 1))
                        .or_else(|| {
                            options.iter().find(|o| {
                                o["label"]
                                    .as_str()
                                    .is_some_and(|l| l.eq_ignore_ascii_case(s))
                                    || o["value"].as_str() == Some(s)
                            })
                        });
                    if let Some(option) = option {
                        return Ok(option["value"].clone());
                    }
                    ensure!(
                        options.is_empty() || q["custom"] == true,
                        "回答不在可选项中，请回复选项编号"
                    );
                    Ok(json!(s))
                };
                let value = match q["kind"].as_str().unwrap_or("") {
                    "multiselect" => Value::Array(
                        reply
                            .split([',', '，'])
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(pick)
                            .collect::<Result<_>>()?,
                    ),
                    "string" | "boolean" => pick(reply)?,
                    "integer" => json!(reply.parse::<i64>().ok().context("请回复一个数字")?),
                    "number" => json!(
                        reply
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .context("请回复一个数字")?
                    ),
                    _ => bail!("此类输入需要回到原窗口处理，指令已保留"),
                };
                answer.insert(key.into(), value);
            }
            Ok(json!({"answer":answer}))
        }
        PERMISSION => Ok(json!({"decision":match text.trim() {
            "/approve" => "once",
            "/decline" => "reject",
            _ => bail!("此操作需要明确回复 /approve 或 /decline"),
        }})),
        _ => bail!("此类输入需要回到原窗口处理，指令已保留"),
    }
}

fn client(source: &Source, target: &Session) -> Option<Client> {
    let client = Client::open(source).ok()?;
    client.owns(target).unwrap_or(false).then_some(client)
}

/// The pending question or permission request, as the card and reply code expect it.
pub(super) fn request(source: &Source, target: &Session) -> Option<Value> {
    client(source, target)?.pending(target).ok().flatten()
}

/// `None` when the source is not an OpenCode session reachable through the service.
pub(super) fn excerpt(source: &Source, target: &Session) -> Option<content::Excerpt> {
    Some(client(source, target)?.excerpt(target))
}

/// `None` leaves the command to the terminal adapters.
pub(super) fn execute(
    source: &Source,
    target: &Session,
    text: &str,
    saved: Option<&str>,
) -> Option<Outcome> {
    let client = client(source, target)?;
    let approval = matches!(text.trim(), "/approve" | "/decline" | "/cancel-request");
    Some(match client.pending(target) {
        Ok(Some(request)) if saved == Some(&codex::fingerprint(&request)) => {
            client.answer(target, &request, text)
        }
        Ok(Some(_)) => Outcome::blocked("问题已改变或尚未同步，请回复最新的问题通知"),
        Ok(None) if target.session.phase == Phase::NeedsInput => {
            Outcome::blocked("原会话没有可回答的请求，请核对原窗口")
        }
        Ok(None) if approval => Outcome::blocked("当前没有审批请求，请回复具体问题通知"),
        Ok(None) if target.session.phase.active() => {
            Outcome::blocked("原会话仍在执行，请等待本轮结束后重试")
        }
        Ok(None) => client.followup(target, text),
        Err(error) => Outcome::blocked(&error.to_string()),
    })
}

/// A pending permission request lives only in the service's memory, so the observer sees a
/// running tool. Ask the service about mid-turn sessions and notify once per request.
pub(super) fn poll(store: &Store) -> Result<()> {
    let c = configuration(&store.db)?;
    if !c.enabled || c.owner.is_none() {
        return Ok(());
    }
    let settings = store.settings()?;
    let rows = store
        .db
        .prepare("SELECT payload FROM channel_sessions WHERE epoch=?1 AND observed_at>?2")?
        .query_map(params![c.epoch, now() - 20], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for raw in rows {
        let Ok(mut target) = serde_json::from_str::<Session>(&raw) else {
            continue;
        };
        if target.session.provider != "opencode"
            || !matches!(
                target.session.phase,
                Phase::Working | Phase::Thinking | Phase::Tool
            )
        {
            continue;
        }
        let Ok(source) = control::source(&settings, &target) else {
            continue;
        };
        let Some(request) = Client::open(&source)
            .ok()
            .and_then(|client| client.permission(&target).ok().flatten())
            .filter(|request| request["method"] == PERMISSION)
        else {
            continue;
        };
        let hash = codex::fingerprint(&request);
        if control::saved_prompt(store, &target)?.as_deref() == Some(&hash) {
            continue;
        }
        // Same state key as the observed turn: a reply stays bound to this turn and request.
        target.session.phase = Phase::NeedsInput;
        store.db.execute(
            "INSERT OR IGNORE INTO channel_events VALUES(?1,?2,?3,?4,?5)",
            params![
                crate::models::hash(&format!("{}:{}:{hash}", c.epoch, target.session.id)),
                c.epoch,
                target.session.id,
                serde_json::to_string(&target)?,
                now()
            ],
        )?;
    }
    Ok(())
}
