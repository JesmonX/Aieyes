use super::{
    Config, config, configuration, db,
    protocol::{Assembler, Frame, MAX_PAYLOAD},
    socket,
};
use crate::{
    models::{ProxyConfig, now},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use prost::Message as _;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tungstenite::Message;

const BASE: &str = "https://open.feishu.cn";
pub(super) struct Api {
    client: reqwest::blocking::Client,
    app_id: String,
    secret: String,
    token: String,
    expires: Instant,
    base: String,
}
impl Api {
    pub fn new(config: &Config, proxy: &ProxyConfig) -> Result<Self> {
        Ok(Self {
            client: crate::network::client_with_policy(
                proxy,
                crate::network::ClientPolicy {
                    redirects: 0,
                    connect_timeout: Duration::from_secs(8),
                    timeout: Duration::from_secs(10),
                    https_only: true,
                    ..Default::default()
                },
            )?,
            app_id: config.app_id.clone(),
            secret: crate::credentials::read_channel(&config.secret_ref)?,
            token: String::new(),
            expires: Instant::now(),
            base: BASE.into(),
        })
    }
    fn decode(response: reqwest::blocking::Response) -> Result<Value> {
        ensure!(
            response.status().is_success(),
            "飞书请求失败（HTTP {}）",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        response
            .take(MAX_PAYLOAD as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_PAYLOAD, "飞书响应超过大小限制");
        serde_json::from_slice(&bytes).context("飞书响应格式无效")
    }
    pub fn endpoint(&self) -> Result<(String, u64, u64)> {
        let response = self
            .client
            .post(format!("{}/callback/ws/endpoint", self.base))
            .header("locale", "zh")
            .json(&json!({"AppID":self.app_id,"AppSecret":self.secret}))
            .send()
            .map_err(|_| anyhow::anyhow!("无法连接飞书，请检查网络和应用凭据"))?;
        let value = Self::decode(response)?;
        ensure!(
            value["code"] == 0,
            "飞书长连接未获授权，请检查 App ID、App Secret 和应用发布状态"
        );
        let url = value["data"]["URL"]
            .as_str()
            .context("飞书未返回长连接地址")?;
        let settings = &value["data"]["ClientConfig"];
        Ok((
            url.into(),
            settings["PingInterval"]
                .as_u64()
                .unwrap_or(120)
                .clamp(10, 300),
            settings["ReconnectInterval"]
                .as_u64()
                .unwrap_or(30)
                .clamp(5, 120),
        ))
    }
    fn token(&mut self) -> Result<()> {
        if !self.token.is_empty() && Instant::now() < self.expires {
            return Ok(());
        }
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.base
            ))
            .json(&json!({"app_id":self.app_id,"app_secret":self.secret}))
            .send()
            .map_err(|_| anyhow::anyhow!("无法获取飞书应用令牌"))?;
        let value = Self::decode(response)?;
        ensure!(value["code"] == 0, "飞书应用凭据未获授权");
        self.token = value["tenant_access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("飞书未返回应用令牌")?
            .into();
        self.expires = Instant::now()
            + Duration::from_secs(value["expire"].as_u64().unwrap_or(600).clamp(60, 7200) - 30);
        Ok(())
    }
    pub fn send(&mut self, id: &str, chat: &str, parent: &str, body: &str) -> Result<String> {
        self.token()?;
        let content = json!({"text":body}).to_string();
        let mut url = reqwest::Url::parse(&format!("{}/open-apis/im/v1/messages", self.base))?;
        let payload = if parent.is_empty() {
            url.query_pairs_mut()
                .append_pair("receive_id_type", "chat_id");
            json!({"receive_id":chat,"msg_type":"text","content":content,"uuid":id})
        } else {
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("飞书消息地址无效"))?
                .push(parent)
                .push("reply");
            json!({"msg_type":"text","content":content,"uuid":id,"reply_in_thread":false})
        };
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .json(&payload)
            .send()
            .map_err(|_| anyhow::anyhow!("飞书消息发送结果待确认，将使用同一消息标识重试"))?;
        let value = Self::decode(response)?;
        let code = value["code"].as_i64().unwrap_or(-1);
        if matches!(code, 99991661 | 99991663 | 99991668) {
            self.token.clear();
        }
        ensure!(
            code == 0,
            "飞书消息未送达（错误码 {code}），请检查机器人权限和应用可用范围"
        );
        value["data"]["message_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .context("飞书未确认消息标识")
    }
}

/// The stable UUID is persisted before I/O, and every retry uses the same Feishu idempotency key.
pub(super) fn deliver(store: &Store, config: &Config, api: &mut Api) -> Result<()> {
    let row: Option<(String,String,String,String,i64,i64)> = store.db.query_row("SELECT id,chat_id,reply_to,body,attempts,at FROM channel_outbox WHERE epoch=?1 AND sent_id IS NULL AND attempts<5 AND next_attempt<=?2 AND at>?3 ORDER BY at,rowid LIMIT 1",params![config.epoch,now(),now()-3300],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((id, chat, parent, body, attempts, _)) = row else {
        return Ok(());
    };
    let current = configuration(&store.db)?;
    if !current.enabled
        || current.epoch != config.epoch
        || current.owner.as_ref().is_none_or(|o| o.chat_id != chat)
    {
        return Ok(());
    }
    let changed = store.db.execute(
        "UPDATE channel_outbox SET attempts=attempts+1,next_attempt=?1 WHERE id=?2 AND attempts=?3",
        params![now() + 10 * (1 << attempts), id, attempts],
    )?;
    if changed != 1 {
        return Ok(());
    }
    match api.send(&id, &chat, &parent, &body) {
        Ok(sent) => {
            store.db.execute(
                "UPDATE channel_outbox SET sent_id=?1 WHERE id=?2",
                params![sent, id],
            )?;
            store
                .db
                .execute("DELETE FROM kv WHERE key='channel.deliveryError'", [])?;
        }
        Err(error) => {
            store.db.execute("INSERT INTO kv(key,value) VALUES('channel.deliveryError',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!({"epoch":config.epoch,"at":now(),"message":error.to_string()}).to_string()])?;
        }
    }
    Ok(())
}

fn payload(frame: &Frame, bytes: Vec<u8>) -> Result<Value> {
    let bytes = match frame.payload_encoding.as_deref().unwrap_or("") {
        "" | "json" => bytes,
        "gzip" => {
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice())
                .take(MAX_PAYLOAD as u64 + 1)
                .read_to_end(&mut decoded)?;
            ensure!(decoded.len() <= MAX_PAYLOAD, "飞书事件超过大小限制");
            decoded
        }
        _ => anyhow::bail!("不支持的飞书事件编码"),
    };
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn connection(
    store: &Store,
    c: &Config,
    proxy: &ProxyConfig,
    stopped: &Arc<AtomicBool>,
) -> Result<u64> {
    let api = Api::new(c, proxy)?;
    let (url, mut ping_seconds, retry) = api.endpoint()?;
    let parsed = reqwest::Url::parse(&url)?;
    let service = parsed
        .query_pairs()
        .find(|(key, _)| key == "service_id")
        .and_then(|(_, value)| value.parse::<i32>().ok())
        .context("飞书长连接缺少服务标识")?;
    let mut ws = socket::connect(&url, proxy)?;
    let mut assembler = Assembler::default();
    let mut ping = Instant::now() - Duration::from_secs(ping_seconds);
    let mut pong = Instant::now();
    let mut heartbeat = Instant::now();
    let mut checked = Instant::now();
    let policy = serde_json::to_string(proxy)?;
    config::connection(&store.db, &c.epoch, "connected", None)?;
    while !stopped.load(Ordering::Acquire) {
        if checked.elapsed() >= Duration::from_secs(1) {
            let current = configuration(&store.db)?;
            if !current.enabled
                || current.epoch != c.epoch
                || serde_json::to_string(&store.settings()?.proxy)? != policy
            {
                break;
            }
            checked = Instant::now();
        }
        if heartbeat.elapsed() >= Duration::from_secs(10) {
            config::connection(&store.db, &c.epoch, "connected", None)?;
            heartbeat = Instant::now();
        }
        if ping.elapsed() >= Duration::from_secs(ping_seconds) {
            ws.send(Message::Binary(Frame::ping(service).encode_to_vec().into()))?;
            ping = Instant::now();
        }
        ensure!(
            pong.elapsed() < Duration::from_secs(ping_seconds * 2 + 10),
            "飞书长连接心跳超时"
        );
        match ws.read() {
            Ok(Message::Binary(bytes)) => {
                let frame = Frame::decode(bytes.as_ref()).context("飞书长连接帧无效")?;
                if frame.method == 0 && frame.header("type") == "pong" {
                    pong = Instant::now();
                    if let Some(data) = &frame.payload
                        && let Ok(value) = serde_json::from_slice::<Value>(data)
                        && let Some(interval) = value["PingInterval"].as_u64()
                    {
                        ping_seconds = interval.clamp(10, 300);
                    }
                } else if frame.method == 1
                    && frame.header("type") == "event"
                    && let Some(bytes) = assembler.payload(&frame)?
                {
                    // ACK only after the message's deduplication key and payload are durable.
                    let accepted = payload(&frame, bytes)
                        .and_then(|event| db::ingest(store, &c.epoch, &event));
                    ws.send(Message::Binary(
                        frame
                            .acknowledge(if accepted.is_ok() { 200 } else { 500 })
                            .into(),
                    ))?;
                }
            }
            Ok(Message::Ping(_)) => {
                ws.flush()?;
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(error) if socket::timed_out(&error) => {}
            Err(_) => anyhow::bail!("飞书长连接已断开，正在重连"),
        }
    }
    let _ = ws.close(None);
    Ok(retry)
}

#[cfg(test)]
pub(super) fn test_api(base: String) -> Api {
    Api {
        client: reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        app_id: "cli_fixture".into(),
        secret: "fixture-secret".into(),
        token: "fixture-token".into(),
        expires: Instant::now() + Duration::from_secs(600),
        base,
    }
}
