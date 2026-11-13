//! Feishu's Pbbp2 envelope, matching larksuite/oapi-sdk-go/ws. No SDK process is required.
use anyhow::{Result, ensure};
use prost::Message;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(super) const MAX_PAYLOAD: usize = 1024 * 1024;

#[derive(Clone, PartialEq, Message)]
pub(super) struct Header {
    #[prost(string, required, tag = "1")]
    pub key: String,
    #[prost(string, required, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct Frame {
    #[prost(uint64, required, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    pub log_id: u64,
    #[prost(int32, required, tag = "3")]
    pub service: i32,
    #[prost(int32, required, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}
impl Frame {
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map_or("", |h| h.value.as_str())
    }
    pub fn ping(service: i32) -> Self {
        Self {
            service,
            headers: vec![Header {
                key: "type".into(),
                value: "ping".into(),
            }],
            ..Default::default()
        }
    }
    pub fn acknowledge(mut self, code: u16) -> Vec<u8> {
        self.headers.retain(|h| h.key != "biz_rt");
        self.headers.push(Header {
            key: "biz_rt".into(),
            value: "0".into(),
        });
        self.payload = Some(
            serde_json::json!({"code":code,"headers":{},"data":null})
                .to_string()
                .into_bytes(),
        );
        self.payload_encoding = None;
        self.encode_to_vec()
    }
}

struct Parts {
    started: Instant,
    size: usize,
    chunks: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
pub(super) struct Assembler {
    parts: HashMap<String, Parts>,
}
impl Assembler {
    pub fn payload(&mut self, frame: &Frame) -> Result<Option<Vec<u8>>> {
        self.parts
            .retain(|_, p| p.started.elapsed() < Duration::from_secs(5));
        let payload = frame.payload.as_deref().unwrap_or_default();
        ensure!(payload.len() <= MAX_PAYLOAD, "飞书事件超过大小限制");
        let sum = if frame.header("sum").is_empty() {
            1
        } else {
            frame.header("sum").parse::<usize>()?
        };
        let seq = if frame.header("seq").is_empty() {
            0
        } else {
            frame.header("seq").parse::<usize>()?
        };
        ensure!((1..=64).contains(&sum) && seq < sum, "飞书事件分片无效");
        if sum == 1 {
            return Ok(Some(payload.to_vec()));
        }
        let key = frame.header("message_id");
        ensure!(!key.is_empty() && key.len() <= 256, "飞书事件缺少消息标识");
        ensure!(
            self.parts.contains_key(key) || self.parts.len() < 32,
            "飞书事件分片过多"
        );
        let part = self.parts.entry(key.into()).or_insert_with(|| Parts {
            started: Instant::now(),
            size: 0,
            chunks: vec![None; sum],
        });
        ensure!(part.chunks.len() == sum, "飞书事件分片数量不一致");
        if let Some(old) = &part.chunks[seq] {
            ensure!(old == payload, "飞书事件分片内容不一致");
        } else {
            ensure!(
                part.size + payload.len() <= MAX_PAYLOAD,
                "飞书事件超过大小限制"
            );
            part.size += payload.len();
            part.chunks[seq] = Some(payload.to_vec());
        }
        if part.chunks.iter().any(Option::is_none) {
            return Ok(None);
        }
        let part = self.parts.remove(key).unwrap();
        Ok(Some(part.chunks.into_iter().flatten().flatten().collect()))
    }
}
