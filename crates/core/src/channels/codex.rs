use super::{Session, content, control::Outcome, socket};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tungstenite::Message;

pub(super) struct Client {
    socket: socket::Socket,
    sequence: u64,
    pending: Vec<Value>,
}
impl Client {
    #[cfg(unix)]
    pub fn open(root: &std::path::Path) -> Result<Self> {
        let path = root.join("app-server-control/app-server-control.sock");
        let mut client = Self {
            socket: socket::unix(&path)?,
            sequence: 0,
            pending: Vec::new(),
        };
        client.call("initialize",json!({"clientInfo":{"name":"aieyes","title":"Aieyes","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
        client.send(json!({"method":"initialized"}))?;
        Ok(client)
    }
    #[cfg(not(unix))]
    pub fn open(_root: &std::path::Path) -> Result<Self> {
        anyhow::bail!("此平台没有可连接的 Codex Unix 控制接口")
    }

    fn send(&mut self, value: Value) -> Result<()> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|_| anyhow::anyhow!("会话控制连接已断开"))
    }
    fn next(&mut self) -> Result<Option<Value>> {
        match self.socket.read() {
            Ok(Message::Text(text)) => Ok(serde_json::from_str(&text).ok()),
            Ok(Message::Ping(_)) => {
                self.socket.flush()?;
                Ok(None)
            }
            Ok(Message::Close(_)) => anyhow::bail!("会话控制连接已关闭"),
            Ok(_) => Ok(None),
            Err(error) if socket::timed_out(&error) => Ok(None),
            Err(_) => anyhow::bail!("会话控制连接已断开"),
        }
    }
    fn capture(&mut self, value: &Value) {
        if value.get("id").is_some() && value["method"].is_string() && self.pending.len() < 32 {
            self.pending.push(value.clone());
        }
        if value["method"] == "serverRequest/resolved" {
            self.pending
                .retain(|r| r["id"] != value["params"]["requestId"]);
        }
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        let id = format!("aieyes-{}", self.sequence);
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if let Some(v) = self.next()? {
                if v["id"] == id {
                    ensure!(v.get("error").is_none(), "原会话控制接口拒绝此请求");
                    return v.get("result").cloned().context("原会话返回无效结果");
                }
                self.capture(&v);
            }
        }
        anyhow::bail!("原会话控制接口响应超时")
    }
    pub fn attach(&mut self, target: &Session, only_loaded: bool) -> Result<Value> {
        ensure!(valid_id(&target.session.native_session), "原生会话标识无效");
        self.verify(target)?;
        if only_loaded {
            let loaded = self.call("thread/loaded/list", json!({}))?;
            ensure!(
                loaded["data"].as_array().is_some_and(|ids| ids
                    .iter()
                    .any(|v| v.as_str() == Some(&target.session.native_session))),
                "原会话未在控制服务中运行"
            );
        }
        let response = self.call(
            "thread/resume",
            json!({"threadId":target.session.native_session,"excludeTurns":true}),
        )?;
        ensure!(
            response["thread"]["id"].as_str() == Some(&target.session.native_session),
            "原会话身份不符"
        );
        Ok(response["thread"].clone())
    }
    fn verify(&mut self, target: &Session) -> Result<Value> {
        ensure!(!target.session.turn_id.is_empty(), "原会话缺少轮次标识");
        let thread = self.call(
            "thread/read",
            json!({"threadId":target.session.native_session,"includeTurns":false}),
        )?["thread"]
            .clone();
        ensure!(
            thread["id"].as_str() == Some(&target.session.native_session),
            "原会话身份不符"
        );
        if let Some(workspace) = &target.session.workspace {
            ensure!(
                thread["cwd"].as_str() == Some(workspace),
                "原会话工作目录已改变或无法核对"
            );
        }
        let turns = self.call("thread/turns/list",json!({"threadId":target.session.native_session,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"}))?;
        ensure!(
            turns["data"][0]["id"].as_str() == Some(&target.session.turn_id),
            "原会话已进入其他轮次，请发送新指令"
        );
        Ok(thread)
    }
    pub fn pending(&mut self, target: &Session) -> Result<Option<Value>> {
        let until = Instant::now() + Duration::from_millis(450);
        while Instant::now() < until {
            if let Some(v) = self.next()? {
                self.capture(&v);
            }
        }
        Ok(self
            .pending
            .iter()
            .rev()
            .find(|r| {
                r["params"]["threadId"].as_str() == Some(&target.session.native_session)
                    && (target.session.turn_id.is_empty()
                        || r["params"]["turnId"].as_str() == Some(&target.session.turn_id))
            })
            .cloned())
    }
    pub fn followup(&mut self, target: &Session, text: &str) -> Outcome {
        match self.verify(target) {
            Ok(thread) if thread["status"]["type"] == "idle" => {}
            Ok(_) => return Outcome::blocked("原会话状态已改变，指令已保留"),
            Err(error) => return Outcome::blocked(&error.to_string()),
        }
        // The turn response itself is evidence of a started turn; a timeout is not a rejection.
        match self.call("turn/start",json!({"threadId":target.session.native_session,"input":[{"type":"text","text":text,"text_elements":[]}]})) {
            Ok(result) if result["turn"]["id"].as_str().is_some_and(|id|!id.is_empty() && id!=target.session.turn_id) && matches!(result["turn"]["status"].as_str(),Some("inProgress"|"completed")) && result["turn"]["error"].is_null() => Outcome::confirmed("原会话已开始执行",json!({"turnId":result["turn"]["id"],"adapter":"codex"})),
            Ok(_) => Outcome::uncertain("原会话返回的执行结果不完整，请核对原窗口"),
            Err(error) if error.to_string()=="原会话控制接口拒绝此请求" => Outcome::blocked("原会话拒绝了指令，请核对原窗口"),
            Err(_) => Outcome::uncertain("指令可能已提交，连接中断；请核对原会话，系统不会重发"),
        }
    }
    pub fn answer(&mut self, target: &Session, request: &Value, text: &str) -> Outcome {
        let result = match answer(request, text) {
            Ok(v) => v,
            Err(e) => return Outcome::blocked(&e.to_string()),
        };
        if let Err(error) = self.verify(target) {
            return Outcome::blocked(&error.to_string());
        }
        if !self
            .pending
            .iter()
            .any(|r| r["id"] == request["id"] && fingerprint(r) == fingerprint(request))
        {
            return Outcome::blocked("原会话的问题已结束或改变，请回复最新通知");
        }
        if self
            .send(json!({"id":request["id"],"result":result}))
            .is_err()
        {
            return Outcome::uncertain("回答可能已提交，请核对原会话");
        }
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            match self.next() {
                Ok(Some(v))
                    if v["method"] == "serverRequest/resolved"
                        && v["params"]["requestId"] == request["id"]
                        && v["params"]["threadId"] == request["params"]["threadId"] =>
                {
                    return Outcome::confirmed(
                        "原会话已接收回答",
                        json!({"adapter":"codex","requestResolved":true}),
                    );
                }
                Ok(Some(v)) => self.capture(&v),
                Ok(None) => {}
                Err(_) => break,
            }
        }
        Outcome::submitted(
            "回答已提交，等待原会话确认",
            json!({"adapter":"codex","answer":true}),
        )
    }
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub(super) fn fingerprint(request: &Value) -> String {
    let p = &request["params"];
    let params: serde_json::Map<String, Value> = [
        "threadId",
        "turnId",
        "itemId",
        "questions",
        "command",
        "reason",
        "availableDecisions",
        "cwd",
        "grantRoot",
        "commandActions",
        "permissions",
        "changes",
        "networkApprovalContext",
        "proposedExecpolicyAmendment",
    ]
    .into_iter()
    .map(|k| (k.to_owned(), p[k].clone()))
    .collect();
    crate::models::hash(&json!({"method":request["method"],"params":params}).to_string())
}
pub(super) fn describe(request: &Value, language: &str) -> String {
    let p = &request["params"];
    let tr = |text: &str| crate::localization::translate(language, text).into_owned();
    match request["method"].as_str().unwrap_or("") {
        "item/tool/requestUserInput" | "tool/requestUserInput" => {
            let mut text = content::questions(&p["questions"]);
            if p["questions"].as_array().is_some_and(|q| q.len() > 1) {
                text.push_str(&format!("\n\n{}", tr("请按「问题编号=答案」逐行回复。")));
            }
            text
        }
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let operation = p["command"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    p["command"].as_array().map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                })
                .unwrap_or_else(|| tr("文件修改"));
            format!(
                "{}\n{}\n\n{}",
                p["reason"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| tr("原会话请求确认")),
                content::clip(&operation, 6000),
                tr("引用此消息回复 /approve 允许本次操作，/decline 拒绝。")
            )
        }
        _ => tr("原会话正在等待输入。此类请求需要回到原窗口处理。"),
    }
}

pub(super) fn answer(request: &Value, text: &str) -> Result<Value> {
    let p = &request["params"];
    match request["method"].as_str().unwrap_or("") {
        "item/tool/requestUserInput" | "tool/requestUserInput" => {
            let questions = p["questions"].as_array().context("原会话问题已不可用")?;
            ensure!(!questions.is_empty(), "原会话问题已不可用");
            let mut answers = serde_json::Map::new();
            for q in questions {
                let id = q["id"].as_str().context("原会话问题缺少标识")?;
                let response = if questions.len() == 1 {
                    text.trim()
                } else {
                    let matches = text
                        .lines()
                        .filter_map(|line| line.split_once('='))
                        .filter(|(key, _)| key.trim() == id)
                        .map(|(_, value)| value.trim())
                        .collect::<Vec<_>>();
                    ensure!(matches.len() == 1, "请按「问题编号=答案」逐行回答所有问题");
                    matches[0]
                };
                ensure!(!response.is_empty(), "问题回答不能为空");
                let selected = response
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0)
                    .and_then(|n| q["options"].as_array().and_then(|a| a.get(n - 1)))
                    .and_then(|o| o["label"].as_str())
                    .unwrap_or(response);
                answers.insert(id.into(), json!({"answers":[selected]}));
            }
            Ok(json!({"answers":answers}))
        }
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = match text.trim() {
                "/approve" => "accept",
                "/decline" => "decline",
                "/cancel-request" => "cancel",
                _ => anyhow::bail!("此操作需要明确回复 /approve 或 /decline"),
            };
            if let Some(allowed) = p["availableDecisions"].as_array() {
                ensure!(
                    allowed.iter().any(|v| v.as_str() == Some(decision)),
                    "原会话不允许此审批选项"
                );
            }
            Ok(json!({"decision":decision}))
        }
        _ => anyhow::bail!("此类输入需要回到原窗口处理，指令已保留"),
    }
}
