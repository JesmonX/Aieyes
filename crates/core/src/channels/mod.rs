//! Opt-in, personal Feishu channel. Only this module may persist conversation excerpts.
//! The ordinary lifecycle observer and outbound webhooks remain metadata-only.
mod codex;
mod config;
mod content;
mod control;
mod db;
mod feishu;
mod protocol;
mod service;
mod socket;

use crate::{Engine, sessions::Snapshot, store::Store};
use anyhow::Result;
use serde_json::{Value, json};

pub(crate) use config::{Config, Owner, configuration, random_id};
pub(crate) use db::{Command, Session};
pub use service::Service;

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db::setup(db)
}

/// Called by the existing session lane. No network or transcript reads on this path.
pub fn observe(store: &Store, snapshot: &Snapshot) -> Result<()> {
    let result = db::observe(store, snapshot);
    if result.is_err() {
        let _ = config::worker_error(store, "无法更新渠道会话，请检查本机数据库");
    }
    result
}

impl Engine {
    pub(crate) fn channel_call(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "channels.start" => {
                if self.channel_service.is_none() {
                    self.channel_service = Service::start(&self.store.root)?;
                }
                Ok(json!({"started":self.channel_service.is_some()}))
            }
            "channels.dispatch" => {
                if self.channel_service.is_none() {
                    self.channel_service = Service::start(&self.store.root)?;
                }
                crate::runtime::signal(&self.store.root).wake();
                Ok(json!({"scheduled":true}))
            }
            "channels.status" => config::status(&self.store),
            "channels.configure" => config::save(&self.store, &params),
            "channels.pair" => config::begin_pairing(&self.store),
            "channels.unpair" => config::unpair(&self.store, &params),
            "channels.sessions" => db::session_list(&self.store),
            "channels.commands" => db::command_list(&self.store),
            "channels.retry" => db::retry(&self.store, &params),
            "channels.cancel" => db::cancel(&self.store, &params),
            "channels.automation.take" => control::take_automation(&self.store),
            "channels.automation.finish" => control::finish_automation(&self.store, &params),
            _ => anyhow::bail!("未知的聊天渠道操作"),
        }
    }
}

#[cfg(test)]
mod tests;
