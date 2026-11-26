//! OpenCode's server keeps pending questions (forms) and permission requests behind an
//! authenticated loopback API. Replies address the original request identifier, and every reply
//! re-reads the pending requests first, so nothing is sent to a request that was not shown.
//! The background service registers its address and password in `service.json`. A server started
//! by hand (`opencode serve` without `--service`) is found through its loopback listener and is
//! only usable when Aieyes has the same `OPENCODE_PASSWORD`. A `--standalone` process is served
//! by neither and keeps the terminal path.
use super::{Session, codex, configuration, content, control, control::Outcome, db::MAX_INPUT};
use crate::session_interaction::contract::{Approval, Decision, FileChange};
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

pub(crate) const FORM: &str = "aieyes/opencodeForm";
pub(crate) const PERMISSION: &str = "aieyes/opencodePermission";
const UNSUPPORTED: &str = "aieyes/opencodeUnsupported";

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

/// What the model is told when a question is dismissed from the popup (already URL-encoded).
const DISMISSED: &str = "The%20user%20dismissed%20this%20question%20without%20answering.";

/// A loopback server and the password of its `opencode` user.
#[derive(Clone, PartialEq)]
struct Endpoint {
    url: String,
    password: String,
}

struct Client {
    http: reqwest::blocking::Client,
    url: String,
    password: String,
}

/// The origin of a plain loopback address. The password must never leave this machine.
fn loopback(address: &str) -> Option<String> {
    let url = reqwest::Url::parse(address).ok()?;
    (url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.origin().ascii_serialization())
}

/// The background service registered by the installation that owns this source's database.
fn registered(source: &Source) -> Option<Endpoint> {
    let bytes = std::fs::read(registration(source)?).ok()?;
    if bytes.len() > 65536 {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    Some(Endpoint {
        url: loopback(value["url"].as_str().unwrap_or(""))?,
        password: value["password"]
            .as_str()
            .filter(|s| !s.is_empty())?
            .to_owned(),
    })
}

/// Pids of this user's `opencode … serve` processes in `ps -xo pid=,command=` output.
fn serve_pids(listing: &str) -> Vec<u32> {
    listing
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse::<u32>().ok()?;
            let words: Vec<&str> = words.collect();
            // The binary itself, or a runtime followed by the script path.
            let program = words
                .iter()
                .take(2)
                .position(|w| w.rsplit('/').next() == Some("opencode"))?;
            words[program + 1..]
                .contains(&"serve")
                .then_some(pid)
                .filter(|pid| *pid != std::process::id())
        })
        .take(8)
        .collect()
}

/// Loopback origins of the TCP listeners in `lsof -Fn` output. A listener on every interface is
/// reached through 127.0.0.1 as well; other addresses are ignored.
fn loopback_listeners(listing: &str) -> Vec<String> {
    let mut found = Vec::new();
    for name in listing.lines().filter_map(|line| line.strip_prefix('n')) {
        let Some((host, port)) = name.rsplit_once(':') else {
            continue;
        };
        let host = match host {
            "127.0.0.1" | "*" => "127.0.0.1",
            "[::1]" => "[::1]",
            _ => continue,
        };
        if let Ok(port) = port.parse::<u16>()
            && port != 0
        {
            let origin = format!("http://{host}:{port}");
            if !found.contains(&origin) {
                found.push(origin);
            }
        }
    }
    found
}

#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
fn scan_listeners() -> Vec<String> {
    use std::process::Command;
    let run = |mut command: Command| {
        command.env("LC_ALL", "C");
        crate::process::run(command, vec![], Duration::from_secs(3))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    let mut ps = Command::new("ps");
    ps.args(["-xo", "pid=,command="]);
    let pids = serve_pids(&run(ps));
    if pids.is_empty() {
        return Vec::new();
    }
    let pids = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut lsof = Command::new("lsof");
    lsof.args(["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN", "-Fn"]);
    loopback_listeners(&run(lsof))
}

#[cfg(not(unix))]
#[cfg_attr(test, allow(dead_code))]
fn scan_listeners() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
thread_local! {
    /// Stand-in for the process scan: hand-started servers visible to the current test.
    static TEST_SERVERS: std::cell::RefCell<Vec<Endpoint>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn set_discovered_for_test(servers: &[(&str, &str)]) {
    TEST_SERVERS.with(|cell| {
        *cell.borrow_mut() = servers
            .iter()
            .map(|(url, password)| Endpoint {
                url: (*url).into(),
                password: (*password).into(),
            })
            .collect();
    });
}

/// Servers started by hand write no registration. Their listeners are read from this user's
/// own `opencode serve` processes, and only loopback ones are used. Such a server takes its
/// password from `OPENCODE_PASSWORD` (or invents one that only its terminal shows), so it is
/// reachable only when Aieyes was started with the same variable; no other process's
/// environment is read.
fn discovered() -> Vec<Endpoint> {
    #[cfg(test)]
    {
        TEST_SERVERS.with(|cell| cell.borrow().clone())
    }
    #[cfg(not(test))]
    {
        static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<String>)>> =
            std::sync::Mutex::new(None);
        let Some(password) = std::env::var("OPENCODE_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty())
        else {
            return Vec::new();
        };
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = cache
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(15));
        if !fresh {
            *cache = Some((std::time::Instant::now(), scan_listeners()));
        }
        cache
            .as_ref()
            .map(|(_, urls)| urls.clone())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|url| loopback(&url))
            .map(|url| Endpoint {
                url,
                password: password.clone(),
            })
            .collect()
    }
}

/// Every server that may host a session of this source: the registered service first.
fn endpoints(source: &Source) -> Vec<Endpoint> {
    if source.provider != "opencode" || source.host_id.is_some() {
        return Vec::new();
    }
    let mut all: Vec<Endpoint> = registered(source).into_iter().collect();
    for endpoint in discovered() {
        if !all.iter().any(|known| known.url == endpoint.url) {
            all.push(endpoint);
        }
    }
    all
}

/// Form and permission identifiers start with the creation time, so the part after the prefix
/// orders requests of both kinds oldest first.
fn age_key(row: &Value) -> &str {
    let id = row["id"].as_str().unwrap_or("");
    id.split_once('_').map_or(id, |(_, rest)| rest)
}

/// Why the server refused a reply. Its own message says which field or value was wrong.
fn refusal(status: u16, value: &Value) -> Outcome {
    if status == 401 {
        return Outcome::blocked("OpenCode 服务口令已变化，请重启该服务或 Aieyes 后重试");
    }
    let message: String = value["message"]
        .as_str()
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let message = content::clip(message.trim(), 200);
    if message.is_empty() {
        return Outcome::blocked("原会话拒绝了指令，请核对原窗口");
    }
    Outcome::blocked(&format!("原会话拒绝了指令：{}", message))
}

impl Client {
    /// `fast` is for the session monitor, which must not stall on a slow server.
    fn connect(endpoint: &Endpoint, fast: bool) -> Option<Self> {
        let (connect, total) = if fast { (200, 350) } else { (1000, 4000) };
        Some(Self {
            http: reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_millis(connect))
                .timeout(Duration::from_millis(total))
                .build()
                .ok()?,
            url: endpoint.url.clone(),
            password: endpoint.password.clone(),
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

    /// Whether this server can show the session. Every server on a machine reads the same
    /// database, so this does not say which process runs it (see `client_with`).
    fn owns(&self, native: &str) -> bool {
        id(native, "ses_")
            && self
                .call(Method::GET, &format!("/api/session/{native}"), None)
                .is_ok_and(|(status, value)| {
                    status == 200 && value["data"]["id"].as_str() == Some(native)
                })
    }

    /// Whether this process is running a turn of the session right now.
    fn running(&self, native: &str) -> bool {
        self.call(Method::GET, "/api/session/active", None)
            .is_ok_and(|(status, value)| status == 200 && value["data"][native].is_object())
    }

    fn rows(&self, native: &str, kind: &str) -> Result<Vec<Value>> {
        ensure!(id(native, "ses_"), "原生会话标识无效");
        let (status, value) =
            self.call(Method::GET, &format!("/api/session/{native}/{kind}"), None)?;
        ensure!(status == 200, "无法读取 OpenCode 的待处理请求");
        Ok(value["data"].as_array().cloned().unwrap_or_default())
    }

    /// Pending forms and permission requests, oldest first, as `(kind, row)`.
    fn queue(&self, native: &str) -> Result<Vec<(&'static str, Value)>> {
        let mut all: Vec<(&'static str, Value)> = self
            .rows(native, "form")?
            .into_iter()
            .map(|row| ("form", row))
            .collect();
        all.extend(
            self.rows(native, "permission")?
                .into_iter()
                .map(|row| ("permission", row)),
        );
        all.sort_by(|a, b| age_key(&a.1).cmp(age_key(&b.1)));
        Ok(all)
    }

    fn permission(&self, target: &Session) -> Result<Option<Value>> {
        Ok(self
            .rows(&target.session.native_session, "permission")?
            .first()
            .map(|row| permission_request(target, row)))
    }

    /// Every pending request, oldest first. Each knows its place in the queue.
    fn pending_all(&self, target: &Session, lang: &str) -> Result<Vec<Value>> {
        let queue = self.queue(&target.session.native_session)?;
        let count = queue.len();
        Ok(queue
            .iter()
            .enumerate()
            .map(|(index, (kind, row))| {
                let mut request = if *kind == "form" {
                    form_request(target, row, lang)
                } else {
                    permission_request(target, row)
                };
                request["params"]["pendingCount"] = json!(count);
                request["params"]["position"] = json!(index + 1);
                request
            })
            .collect())
    }

    /// The oldest pending request, as the chat channel describes and answers it.
    fn pending(&self, target: &Session) -> Result<Option<Value>> {
        Ok(self.pending_all(target, "en")?.into_iter().next())
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
            Ok((status, value)) if status < 500 => refusal(status, &value),
            _ => Outcome::uncertain("回答可能已提交，请核对原会话"),
        }
    }

    /// `queue` holds the instruction until the running turn ends instead of steering it.
    fn followup(&self, target: &Session, text: &str, queue: bool) -> Outcome {
        let path = format!("/api/session/{}/prompt", target.session.native_session);
        let mut body = json!({"text":text});
        if queue {
            body["delivery"] = json!("queue");
        }
        match self.call(Method::POST, &path, Some(&body)) {
            Ok((200, value)) if value["data"].is_object() => {
                let mut receipt = json!({"adapter":"opencode","messageId":value["data"]["id"]});
                if queue {
                    receipt["delivery"] = json!("queue");
                    Outcome::confirmed("指令已排队，将在本轮结束后执行", receipt)
                } else {
                    Outcome::confirmed("原会话已接收指令", receipt)
                }
            }
            Ok((status, value)) if status < 500 => refusal(status, &value),
            _ => Outcome::uncertain("指令可能已提交，连接中断；请核对原会话，系统不会重发"),
        }
    }

    /// Stops the running turn. Requests still waiting for an answer are dropped by the server.
    fn interrupt(&self, target: &Session) -> Outcome {
        let path = format!("/api/session/{}/interrupt", target.session.native_session);
        match self.call(Method::POST, &path, None) {
            Ok((200, value)) if value["interrupted"] == true => {
                Outcome::confirmed("已中断原会话", json!({"adapter":"opencode"}))
            }
            Ok((200, _)) => Outcome::blocked("原会话当前没有正在执行的任务"),
            Ok((status, value)) if status < 500 => refusal(status, &value),
            _ => Outcome::uncertain("中断请求可能已提交，请核对原会话"),
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

/// A form as question rows. `title` is the short header of a field that also has a description.
/// Constraints are passed on for the card and checked again before a reply is sent.
fn form_request(target: &Session, form: &Value, lang: &str) -> Value {
    let tr = |text: &str| crate::localization::translate(lang, text).into_owned();
    let mut supported = id(form["id"].as_str().unwrap_or(""), "frm_");
    let mut link = "";
    let text = |field: &Value, key: &str| -> Option<String> {
        field[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
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
            if kind == "external" && link.is_empty() {
                link = field["url"].as_str().unwrap_or("");
            }
            let options = if kind == "boolean" {
                json!([{"label":tr("是"),"value":true},{"label":tr("否"),"value":false}])
            } else {
                field["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| json!({"label":o["label"],"description":o["description"],"value":o["value"]}))
                    .collect()
            };
            let (header, question) = match (text(field, "title"), text(field, "description")) {
                (Some(title), Some(description)) => (title, description),
                (title, description) => (
                    String::new(),
                    description
                        .or(title)
                        .or_else(|| text(field, "key"))
                        .unwrap_or_default(),
                ),
            };
            let mut row = json!({
                "id": (index + 1).to_string(),
                "key": field["key"],
                "kind": kind,
                "header": header,
                "question": question,
                "options": options,
                "multiSelect": kind == "multiselect",
                "custom": field["custom"] == true,
                // The server refuses a reply that leaves out a required field; others may stay empty.
                "required": field["required"] == true,
            });
            if let Some(placeholder) = text(field, "placeholder") {
                row["placeholder"] = json!(placeholder);
            }
            if matches!(
                field["default"],
                Value::String(_) | Value::Bool(_) | Value::Number(_)
            ) {
                row["default"] = field["default"].clone();
            }
            let constraints: serde_json::Map<String, Value> = [
                "minimum",
                "maximum",
                "minLength",
                "maxLength",
                "minItems",
                "maxItems",
            ]
            .into_iter()
            .filter(|key| field[*key].is_number())
            .chain(
                ["pattern", "format"]
                    .into_iter()
                    .filter(|key| field[*key].as_str().is_some_and(|s| !s.is_empty())),
            )
            .map(|key| (key.to_owned(), field[key].clone()))
            .collect();
            if !constraints.is_empty() {
                row["constraints"] = Value::Object(constraints);
            }
            row
        })
        .collect::<Vec<_>>();
    let answerable = supported && !questions.is_empty();
    json!({
        "method": if answerable { FORM } else { UNSUPPORTED },
        "params": {
            "threadId": target.session.native_session,
            "turnId": target.session.turn_id,
            "itemId": form["id"],
            "questions": questions,
            "form": {
                "title": form["title"].as_str().unwrap_or(""),
                // The question tool asks on the model's behalf; other forms come from the agent itself.
                "source": if form["metadata"]["kind"] == "question" { "tool" } else { "agent" },
                "url": link,
                // Dismissing is the one way out besides answering; there is no separate "decline".
                "canCancel": answerable,
                "canSkip": false,
            },
        }
    })
}

fn permission_request(target: &Session, row: &Value) -> Value {
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let resources = strings(&row["resources"]).join("\n");
    let action = row["action"].as_str().unwrap_or("");
    let message = row["message"].as_str().filter(|s| !s.is_empty());
    let reason = match message {
        Some(message) => format!("{action}: {message}"),
        None => action.to_owned(),
    };
    // Edits carry the patch of each file they would write.
    let changes = row["metadata"]["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| {
            Some(json!({
                "path": file["file"].as_str().filter(|s| !s.is_empty())?,
                "status": file["status"].as_str().unwrap_or(""),
                "patch": file["patch"].as_str().unwrap_or(""),
            }))
        })
        .collect::<Vec<_>>();
    let supported = id(row["id"].as_str().unwrap_or(""), "per");
    json!({
        "method": if supported { PERMISSION } else { UNSUPPORTED },
        "params": {
            "threadId": target.session.native_session,
            "turnId": target.session.turn_id,
            "itemId": row["id"],
            "command": resources,
            "reason": reason,
            "kind": action,
            "message": message.unwrap_or(""),
            "changes": changes,
            // The patterns "always" would save as allowed.
            "permissions": strings(&row["save"]),
        }
    })
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

/// The server that runs this session: the registered service or a hand-started one.
/// Pending forms and permissions and the running turn live in the memory of one process,
/// while every server can read the session itself. Among the servers that show the session,
/// the one that lists the `wanted` request is used, then one with any pending request, then
/// one that reports the session running. An idle session has none of these; it goes to the
/// first server, the registered service when there is one.
fn client_with(source: &Source, native: &str, fast: bool, wanted: Option<&str>) -> Option<Client> {
    let mut visible: Vec<Client> = endpoints(source)
        .iter()
        .filter_map(|endpoint| Client::connect(endpoint, fast))
        .filter(|client| client.owns(native))
        .collect();
    if visible.len() > 1 {
        let queues: Vec<Vec<String>> = visible
            .iter()
            .map(|client| {
                client
                    .queue(native)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|(_, row)| row["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .collect();
        let pick = wanted
            .and_then(|id| {
                queues
                    .iter()
                    .position(|queue| queue.iter().any(|q| q == id))
            })
            .or_else(|| queues.iter().position(|queue| !queue.is_empty()))
            .or_else(|| visible.iter().position(|client| client.running(native)));
        if let Some(index) = pick {
            return Some(visible.swap_remove(index));
        }
    }
    visible.into_iter().next()
}

fn client(source: &Source, target: &Session) -> Option<Client> {
    client_with(source, &target.session.native_session, false, None)
}

/// Whether a reachable server owns this session, so replies and follow-ups can reach it.
pub(crate) fn hosted(source: &Source, target: &Session) -> bool {
    client(source, target).is_some()
}

/// The approval card for a permission request: what kind of tool asks, the commands or files it
/// names, the patch of an edit, and the decisions `permission_body` can reply with.
pub(crate) fn approval_card(request: &Value, lang: &str) -> Approval {
    let tr = |text: &str| crate::localization::translate(lang, text).into_owned();
    let p = &request["params"];
    let action = p["kind"].as_str().unwrap_or("");
    let resources: Vec<String> = p["command"]
        .as_str()
        .unwrap_or("")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let (tool, kind, fallback) = match action {
        "shell" | "bash" => ("Shell", "command", "请求执行命令"),
        "edit" => ("Edit", "fileChange", "请求修改文件"),
        "" => ("Permission", "permissions", "原会话请求确认"),
        _ => (action, "tool", "原会话请求确认"),
    };
    let mut card = Approval::new(tool, kind);
    card.reason = match p["message"].as_str().filter(|s| !s.is_empty()) {
        Some(message) => message.to_owned(),
        None => tr(fallback),
    };
    card.command = resources.join("\n");
    card.changes = p["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|file| FileChange {
            path: file["path"].as_str().unwrap_or("").to_owned(),
            kind: match file["status"].as_str().unwrap_or("") {
                "added" => "add",
                "modified" => "update",
                "deleted" => "delete",
                _ => "unknown",
            }
            .into(),
            diff: file["patch"].as_str().unwrap_or("").to_owned(),
            ..Default::default()
        })
        .collect();
    if kind == "fileChange" {
        if let [only] = resources.as_slice() {
            let name = only.rsplit(['/', '\\']).next().unwrap_or(only);
            card.title = format!("{tool} · {name}");
        }
        card.paths = resources;
    } else if kind != "command" {
        card.paths = resources;
    }
    let saved: Vec<&str> = p["permissions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|pattern| !pattern.is_empty())
        .collect();
    card.decisions = vec![Decision::approve(lang).with_native(json!({"decision": "once"}))];
    if !saved.is_empty() {
        // The server saves exactly these patterns; the card shows them before the user agrees.
        card.decisions.push(
            Decision::approve_always(lang)
                .with_native(json!({"decision": "always"}))
                .with_description(&saved.join("\n")),
        );
    }
    card.decisions.push(
        Decision::decline(lang)
            .with_reason()
            .with_native(json!({"decision": "reject"})),
    );
    card
}

/// The oldest pending question or permission request, as the chat channel expects it.
pub(crate) fn request(source: &Source, target: &Session) -> Option<Value> {
    client(source, target)?.pending(target).ok().flatten()
}

/// Every pending request of the session, oldest first, with labels in the app language.
/// `None` when no reachable server owns the session.
pub(crate) fn requests(source: &Source, target: &Session, lang: &str) -> Option<Vec<Value>> {
    client(source, target)?.pending_all(target, lang).ok()
}

/// The request a card or reply is about: the named one, otherwise the oldest.
pub(crate) fn select<'a>(requests: &'a [Value], wanted: Option<&str>) -> Option<&'a Value> {
    match wanted {
        Some(wanted) => requests
            .iter()
            .find(|request| request["params"]["itemId"].as_str() == Some(wanted)),
        None => requests.first(),
    }
}

/// Fast loopback probe for the session monitor so pending in-memory permissions/forms show up
/// as NeedsInput. Reports the oldest request, the one the card shows first.
pub(crate) fn probe_pending_id(source: &Source, native: &str) -> Option<(String, String)> {
    let client = client_with(source, native, true, None)?;
    let (kind, row) = client.queue(native).ok()?.into_iter().next()?;
    let prefix = if kind == "form" { "frm_" } else { "per" };
    row["id"]
        .as_str()
        .filter(|value| id(value, prefix))
        .map(|value| (kind.to_owned(), value.to_owned()))
}

/// `None` when the source is not an OpenCode session reachable through its server.
pub(crate) fn excerpt(source: &Source, target: &Session) -> Option<content::Excerpt> {
    Some(client(source, target)?.excerpt(target))
}

/// A reply from the pet popup, already checked for shape and size.
pub(crate) struct Reply<'a> {
    /// The fingerprint of the request the card showed.
    pub hash: Option<&'a str>,
    /// `answer`, `cancel` or a decision id.
    pub action: &'a str,
    pub answers: Option<&'a Value>,
    pub text: Option<&'a str>,
    pub reason: Option<&'a str>,
    /// The request the card showed; the oldest one when absent.
    pub request_id: Option<&'a str>,
    pub lang: &'a str,
}

/// Structured desktop reply for OpenCode forms, permissions, and follow-up text.
/// `None` when no reachable server owns the session.
pub(crate) fn execute_structured(
    source: &Source,
    target: &Session,
    reply: &Reply,
) -> Option<Outcome> {
    let client = client_with(
        source,
        &target.session.native_session,
        false,
        reply.request_id,
    )?;
    let pending = match client.pending_all(target, reply.lang) {
        Ok(value) => value,
        Err(error) => return Some(Outcome::blocked(&error.to_string())),
    };
    if pending.is_empty() {
        if reply.action != "answer" {
            return Some(Outcome::blocked("当前没有待处理的审批请求"));
        }
        if target.session.phase == Phase::NeedsInput {
            return Some(Outcome::blocked("原会话没有可回答的请求，请核对原窗口"));
        }
        let body = reply.text.unwrap_or("").trim();
        if body.is_empty() {
            return Some(Outcome::blocked("输入内容不能为空"));
        }
        // A turn that may still be running is never steered: the instruction waits for its end.
        return Some(client.followup(target, body, !target.session.phase.terminal()));
    }
    let Some(request) = select(&pending, reply.request_id) else {
        return Some(Outcome::blocked("原请求已结束或已被处理，请刷新后再操作"));
    };
    if reply.hash != Some(&codex::fingerprint(request)) {
        return Some(Outcome::blocked("问题已改变或尚未同步，请刷新后再操作"));
    }
    let item = request["params"]["itemId"].as_str().unwrap_or("");
    let native = &target.session.native_session;
    let receipt = json!({"adapter":"opencode","requestId":item});
    let form = request["method"] == FORM;
    if form && reply.action == "cancel" {
        let path = format!("/api/session/{native}/form/{item}?message={DISMISSED}");
        return Some(match client.call(Method::DELETE, &path, None) {
            Ok((200 | 204, _)) => Outcome::confirmed("已取消原会话的提问", receipt),
            Ok((404 | 409, _)) => Outcome::blocked("原会话的问题已结束或改变，请刷新后再操作"),
            Ok((status, value)) if status < 500 => refusal(status, &value),
            _ => Outcome::uncertain("取消请求可能已提交，请核对原会话"),
        });
    }
    let body = match structured_body(request, reply) {
        Ok(value) => value,
        Err(error) => return Some(Outcome::blocked(&error.to_string())),
    };
    let kind = if form { "form" } else { "permission" };
    let path = format!("/api/session/{native}/{kind}/{item}/reply");
    Some(match client.call(Method::POST, &path, Some(&body)) {
        Ok((200 | 204, _)) => Outcome::confirmed("原会话已接收回答", receipt),
        Ok((404 | 409, _)) => Outcome::blocked("原会话的问题已结束或改变，请刷新后再操作"),
        Ok((status, value)) if status < 500 => refusal(status, &value),
        _ => Outcome::uncertain("回答可能已提交，请核对原会话"),
    })
}

/// Stop the running turn of a session a reachable server owns. `None` when none does.
pub(crate) fn interrupt(source: &Source, target: &Session) -> Option<Outcome> {
    Some(client(source, target)?.interrupt(target))
}

/// The reply to a permission request: one of the decisions its card offers, and the user's
/// reason when a rejection carries one (the agent reads it as the tool's error).
fn permission_body(request: &Value, reply: &Reply) -> Result<Value> {
    let wanted = match reply.action {
        "accept" => "approve",
        "deny" => "decline",
        other => other,
    };
    let card = approval_card(request, reply.lang);
    let decision = card
        .decisions
        .iter()
        .find(|decision| decision.id == wanted)
        .context("不支持的审批操作")?;
    let mut body = decision.native.clone();
    let reason = reply.reason.map(str::trim).filter(|r| !r.is_empty());
    if let Some(reason) = reason {
        ensure!(decision.accepts_reason, "此审批选项不支持附加说明");
        body["message"] = json!(reason);
    }
    Ok(body)
}

fn unanswered(raw: Option<&Value>) -> bool {
    match raw {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => false,
    }
}

fn structured_body(request: &Value, reply: &Reply) -> Result<Value> {
    match request["method"].as_str().unwrap_or("") {
        PERMISSION => permission_body(request, reply),
        FORM => {
            ensure!(
                reply.action == "answer",
                "此请求不支持取消或跳过，请在原窗口处理"
            );
            let Some(map) = reply.answers.and_then(Value::as_object) else {
                return answer(request, reply.text.unwrap_or(""));
            };
            let questions = request["params"]["questions"]
                .as_array()
                .context("原会话问题已不可用")?;
            ensure!(!questions.is_empty(), "原会话问题已不可用");
            let mut out = serde_json::Map::new();
            for q in questions {
                let id = q["id"].as_str().context("原会话问题缺少标识")?;
                let key = q["key"].as_str().context("原会话问题缺少标识")?;
                let raw = map.get(id).or_else(|| map.get(key));
                if unanswered(raw) {
                    // An optional field left empty is omitted, as the original form allows.
                    ensure!(q["required"] != true, "请回答所有问题后再提交");
                    continue;
                }
                let raw = raw.unwrap_or(&Value::Null);
                let limit = |name: &str| q["constraints"][name].as_f64();
                let options = q["options"].as_array().map_or(&[][..], Vec::as_slice);
                let pick_scalar = |s: &str| -> Result<Value> {
                    if let Some(option) = options.iter().find(|o| {
                        o["label"]
                            .as_str()
                            .is_some_and(|l| l.eq_ignore_ascii_case(s))
                            || match &o["value"] {
                                Value::String(value) => value == s,
                                Value::Bool(value) => value.to_string() == s,
                                _ => false,
                            }
                    }) {
                        return Ok(option["value"].clone());
                    }
                    ensure!(
                        options.is_empty() || q["custom"] == true,
                        "回答不在可选项中，请重新选择"
                    );
                    Ok(json!(s))
                };
                let in_range = |n: f64| {
                    limit("minimum").is_none_or(|min| n >= min)
                        && limit("maximum").is_none_or(|max| n <= max)
                };
                let val = match q["kind"].as_str().unwrap_or("") {
                    "multiselect" => {
                        let items: Vec<String> = if let Some(arr) = raw.as_array() {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .collect()
                        } else if let Some(s) = raw.as_str() {
                            s.split([',', '，'])
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .collect()
                        } else {
                            Vec::new()
                        };
                        ensure!(!items.is_empty(), "问题回答不能为空");
                        let count = items.len() as f64;
                        ensure!(
                            limit("minItems").is_none_or(|min| count >= min)
                                && limit("maxItems").is_none_or(|max| count <= max),
                            "选择的数量不符合要求"
                        );
                        Value::Array(
                            items
                                .iter()
                                .map(|s| pick_scalar(s))
                                .collect::<Result<_>>()?,
                        )
                    }
                    "boolean" => {
                        if let Some(b) = raw.as_bool() {
                            json!(b)
                        } else {
                            let s = raw.as_str().unwrap_or("").trim();
                            ensure!(!s.is_empty(), "问题回答不能为空");
                            let picked = pick_scalar(s)?;
                            ensure!(picked.is_boolean(), "回答不在可选项中，请重新选择");
                            picked
                        }
                    }
                    "integer" => {
                        let s = raw
                            .as_i64()
                            .map(|n| n.to_string())
                            .or_else(|| raw.as_str().map(|s| s.trim().to_owned()))
                            .unwrap_or_default();
                        let n = s.parse::<i64>().ok().context("请输入有效的整数")?;
                        ensure!(in_range(n as f64), "数值超出允许范围");
                        json!(n)
                    }
                    "number" => {
                        let s = raw
                            .as_f64()
                            .map(|n| n.to_string())
                            .or_else(|| raw.as_str().map(|s| s.trim().to_owned()))
                            .unwrap_or_default();
                        let n = s
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .context("请输入有效的数字")?;
                        ensure!(in_range(n), "数值超出允许范围");
                        json!(n)
                    }
                    "string" => {
                        let s = raw.as_str().unwrap_or("").trim();
                        ensure!(!s.is_empty(), "问题回答不能为空");
                        let picked = pick_scalar(s)?;
                        // Listed options are the server's own values; only typed text has a length.
                        if options.is_empty() {
                            let length = s.chars().count() as f64;
                            ensure!(
                                limit("minLength").is_none_or(|min| length >= min)
                                    && limit("maxLength").is_none_or(|max| length <= max),
                                "文本长度不符合要求"
                            );
                        }
                        picked
                    }
                    _ => bail!("此类输入需要回到原窗口处理"),
                };
                out.insert(key.into(), val);
            }
            // An empty reply would record every question as unanswered; dismissing says that honestly.
            ensure!(!out.is_empty(), "请至少回答一个问题后再提交");
            Ok(json!({"answer":out}))
        }
        _ => bail!("此类输入需要回到原窗口处理"),
    }
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
        Ok(None) => client.followup(target, text, false),
        Err(error) => Outcome::blocked(&error.to_string()),
    })
}

/// A pending permission request lives only in the server's memory, so the observer sees a
/// running tool. Ask the server about mid-turn sessions and notify once per request.
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
        let Some(request) = client(&source, &target)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_users_opencode_serve_processes_and_loopback_listeners_are_considered() {
        let ps = "\
 6749 /Users/me/.opencode/bin/opencode serve --hostname 127.0.0.1 --port 48731
 7001 opencode serve
 7002 node /usr/local/lib/node_modules/opencode-ai/bin/opencode serve --port 5000
 7003 /Users/me/.opencode/bin/opencode
 7004 /Users/me/.opencode/bin/opencode run serve-the-docs
 7005 /bin/zsh -c opencode serve
 7006 vim opencode-notes serve
 nope opencode serve
";
        assert_eq!(serve_pids(ps), [6749, 7001, 7002]);
        assert!(serve_pids(&format!("{} opencode serve", std::process::id())).is_empty());

        let lsof = "p6749\nf7\nn127.0.0.1:48731\np7001\nf9\nn*:4096\nf10\nn[::1]:4097\n\
                    f11\nn192.168.1.20:5000\nf12\nn127.0.0.1:0\nf13\nn127.0.0.1:48731\nnjunk\n";
        assert_eq!(
            loopback_listeners(lsof),
            [
                "http://127.0.0.1:48731",
                "http://127.0.0.1:4096",
                "http://[::1]:4097"
            ]
        );
        assert!(loopback_listeners("").is_empty());
    }

    /// Manual check: with `opencode serve` running as this user, prints what the scan finds.
    #[test]
    #[ignore = "needs a running OpenCode server"]
    fn live_scan_lists_hand_started_servers() {
        let found = scan_listeners();
        println!("listeners: {found:?}");
        if let Ok(expected) = std::env::var("AIEYES_OPENCODE_LIVE_URL") {
            assert!(found.contains(&expected), "{found:?}");
        }
    }

    #[test]
    fn the_password_only_goes_to_plain_loopback_addresses() {
        assert_eq!(
            loopback("http://127.0.0.1:4096/").as_deref(),
            Some("http://127.0.0.1:4096")
        );
        assert!(loopback("http://localhost:4096").is_some());
        assert!(loopback("http://[::1]:4096").is_some());
        for refused in [
            "https://127.0.0.1:4096",
            "http://192.168.1.2:4096",
            "http://example.com",
            "http://user@127.0.0.1:4096",
            "http://opencode:pw@127.0.0.1:4096",
            "http://127.0.0.1.example.com:4096",
            "",
        ] {
            assert!(loopback(refused).is_none(), "{refused}");
        }
    }

    #[test]
    fn requests_are_ordered_by_the_time_in_their_identifier() {
        let mut rows = [
            json!({"id":"per_213b7ee49001wRHGbaQiBtkskg"}),
            json!({"id":"frm_213b700bb001BxQGhiv6V84cTy"}),
            json!({"id":"per_213b2cf6a001atszzmoQHG52Wk"}),
        ];
        rows.sort_by(|a, b| age_key(a).cmp(age_key(b)));
        let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert_eq!(
            ids,
            [
                "per_213b2cf6a001atszzmoQHG52Wk",
                "frm_213b700bb001BxQGhiv6V84cTy",
                "per_213b7ee49001wRHGbaQiBtkskg"
            ]
        );
    }

    #[test]
    fn server_refusals_keep_the_servers_wording_on_one_bounded_line() {
        let out = refusal(
            400,
            &json!({"_tag":"InvalidRequestError","message":"Expected Permission.Reply\n  at [\"decision\"]"}),
        );
        assert_eq!(out.status, "blocked");
        assert_eq!(
            out.reason,
            "原会话拒绝了指令：Expected Permission.Reply   at [\"decision\"]"
        );
        let long = refusal(400, &json!({"message":"x".repeat(5000)}));
        assert!(long.reason.len() < 300);
        assert_eq!(
            refusal(400, &Value::Null).reason,
            "原会话拒绝了指令，请核对原窗口"
        );
        assert!(
            refusal(401, &json!({"message":"Authentication required"}))
                .reason
                .contains("口令")
        );
    }
}
