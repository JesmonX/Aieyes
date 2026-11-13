use super::db;
use crate::{credentials, models::now, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const KEY: &str = "channel.feishu.v1";
pub(super) const PAIR_KEY: &str = "channel.feishu.pair.v1";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Config {
    pub enabled: bool,
    pub app_id: String,
    pub secret_ref: String,
    pub epoch: String,
    pub revision: u64,
    pub owner: Option<Owner>,
    pub allow_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Owner {
    pub open_id: String,
    pub chat_id: String,
    pub tenant_key: String,
}

#[derive(Serialize, Deserialize)]
struct Pairing {
    hash: String,
    expires_at: i64,
    attempts: u8,
    epoch: String,
}

pub(crate) fn random_id() -> Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("无法生成配对标识"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn configuration(db: &Connection) -> Result<Config> {
    let value: Option<String> = db
        .query_row("SELECT value FROM kv WHERE key=?1", [KEY], |r| r.get(0))
        .optional()?;
    value
        .map(|v| serde_json::from_str(&v).context("无法读取聊天渠道配置"))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(super) fn write(db: &Connection, config: &Config) -> Result<()> {
    db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![KEY, serde_json::to_string(config)?])?;
    Ok(())
}

pub(super) fn status(store: &Store) -> Result<Value> {
    let c = configuration(&store.db)?;
    let connection: Option<String> = store
        .db
        .query_row(
            "SELECT value FROM kv WHERE key='channel.connection'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut connection: Value = connection
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(json!({"state":"stopped"}));
    if connection["epoch"].as_str() != Some(&c.epoch)
        || connection["at"].as_i64().is_none_or(|at| now() - at > 45)
    {
        connection = json!({"state":"stopped"});
    }
    if !c.enabled {
        connection = json!({"state":"disabled"});
    }
    let pending: i64 = store.db.query_row("SELECT count(*) FROM channel_commands WHERE epoch=?1 AND status IN ('queued','blocked','automation','dispatching','submitted','uncertain')", [&c.epoch], |r| r.get(0))?;
    let undelivered: i64 = store.db.query_row(
        "SELECT count(*) FROM channel_outbox WHERE epoch=?1 AND sent_id IS NULL",
        [&c.epoch],
        |r| r.get(0),
    )?;
    let delivery_error = recent_error(store, &c.epoch, "channel.deliveryError")?;
    let worker_error = recent_error(store, &c.epoch, "channel.workerError")?;
    Ok(json!({
        "enabled":c.enabled,"appId":c.app_id,"hasSecret":!c.secret_ref.is_empty(),
        "revision":c.revision,"paired":c.owner.is_some(),"ownerOpenId":c.owner.as_ref().map(|o| &o.open_id),
        "allowTerminal":c.allow_terminal,"connection":connection,"pending":pending,
        "deliveryError":delivery_error,"workerError":worker_error,"undelivered":undelivered
    }))
}

fn recent_error(store: &Store, epoch: &str, key: &str) -> Result<Value> {
    let raw: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    let value: Value = raw
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(Value::Null);
    Ok(if value["epoch"].as_str() == Some(epoch) {
        value
    } else {
        Value::Null
    })
}

pub(super) fn worker_error(store: &Store, message: &str) -> Result<()> {
    let c = configuration(&store.db)?;
    store.db.execute("INSERT INTO kv(key,value) VALUES('channel.workerError',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!({"epoch":c.epoch,"message":message,"at":now()}).to_string()])?;
    Ok(())
}

fn expected_revision(c: &Config, params: &Value) -> Result<()> {
    ensure!(
        params["revision"].as_u64() == Some(c.revision),
        "聊天渠道配置已变更，请刷新后重试"
    );
    Ok(())
}

pub(super) fn save(store: &Store, input: &Value) -> Result<Value> {
    let _lock = db::config_lock(store)?;
    let mut c = configuration(&store.db)?;
    expected_revision(&c, input)?;
    let app_id = input["appId"].as_str().context("请输入飞书 App ID")?.trim();
    ensure!(
        app_id.starts_with("cli_")
            && (5..=128).contains(&app_id.len())
            && app_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "飞书 App ID 格式无效"
    );
    let enabled = input["enabled"].as_bool().context("缺少聊天渠道开关")?;
    let secret = input["appSecret"].as_str().filter(|s| !s.is_empty());
    ensure!(
        secret.is_some() || app_id == c.app_id && !c.secret_ref.is_empty(),
        "请输入飞书 App Secret"
    );
    let old_ref = c.secret_ref.clone();
    let old_epoch = c.epoch.clone();
    let changed_app = app_id != c.app_id;
    let next_ref = if let Some(secret) = secret {
        ensure!(
            secret.len() <= 4096 && !secret.contains(['\n', '\r', '\0']),
            "飞书 App Secret 格式无效"
        );
        let reference = format!("feishu-{}", random_id()?);
        credentials::save_channel(&reference, secret)?;
        reference
    } else {
        old_ref.clone()
    };
    c.secret_ref = next_ref.clone();
    c.app_id = app_id.to_owned();
    c.enabled = enabled;
    c.allow_terminal = input["allowTerminal"].as_bool().unwrap_or(false);
    c.revision += 1;
    // Every edit invalidates transport credentials and commands from the old configuration.
    c.epoch = random_id()?;
    if changed_app {
        c.owner = None;
    }
    let saved = (|| -> Result<()> {
        let tx = store.db.unchecked_transaction()?;
        write(&tx, &c)?;
        tx.execute("DELETE FROM kv WHERE key=?1", [PAIR_KEY])?;
        db::invalidate(&tx, &old_epoch)?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = saved {
        if next_ref != old_ref {
            let _ = credentials::delete_channel(&next_ref);
        }
        return Err(error);
    }
    if old_ref != next_ref && !old_ref.is_empty() {
        let _ = credentials::delete_channel(&old_ref);
    }
    crate::runtime::signal(&store.root).wake();
    status(store)
}

pub(super) fn begin_pairing(store: &Store) -> Result<Value> {
    let _lock = db::config_lock(store)?;
    let c = configuration(&store.db)?;
    ensure!(
        c.enabled && !c.secret_ref.is_empty(),
        "请先保存并启用飞书渠道"
    );
    ensure!(c.owner.is_none(), "已配对，请先解除配对");
    let code = format!("aieyes-{}", &random_id()?[..12]);
    let expires_at = now() + 600;
    let pair = Pairing {
        hash: crate::models::hash(&code),
        expires_at,
        attempts: 0,
        epoch: c.epoch,
    };
    store.db.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![PAIR_KEY,serde_json::to_string(&pair)?])?;
    Ok(json!({"code":code,"message":format!("/pair {code}"),"expiresAt":expires_at}))
}

pub(super) fn unpair(store: &Store, input: &Value) -> Result<Value> {
    let _lock = db::config_lock(store)?;
    let mut c = configuration(&store.db)?;
    expected_revision(&c, input)?;
    let tx = store.db.unchecked_transaction()?;
    db::invalidate(&tx, &c.epoch)?;
    c.owner = None;
    c.epoch = random_id()?;
    c.revision += 1;
    write(&tx, &c)?;
    tx.execute("DELETE FROM kv WHERE key=?1", [PAIR_KEY])?;
    tx.commit()?;
    crate::runtime::signal(&store.root).wake();
    status(store)
}

/// Called inside the inbox transaction. No account can replace an existing binding.
pub(super) fn accept_pair(
    db: &Connection,
    c: &mut Config,
    code: &str,
    owner: Owner,
) -> Result<bool> {
    if !c.enabled || c.owner.is_some() {
        return Ok(false);
    }
    let raw: Option<String> = db
        .query_row("SELECT value FROM kv WHERE key=?1", [PAIR_KEY], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(mut pair) = raw.and_then(|s| serde_json::from_str::<Pairing>(&s).ok()) else {
        return Ok(false);
    };
    if pair.epoch != c.epoch || pair.expires_at < now() || pair.attempts >= 5 {
        return Ok(false);
    }
    pair.attempts += 1;
    if crate::models::hash(code) != pair.hash {
        db.execute(
            "UPDATE kv SET value=?1 WHERE key=?2",
            params![serde_json::to_string(&pair)?, PAIR_KEY],
        )?;
        return Ok(false);
    }
    c.owner = Some(owner);
    c.revision += 1;
    write(db, c)?;
    db.execute("DELETE FROM kv WHERE key=?1", [PAIR_KEY])?;
    Ok(true)
}

pub(super) fn connection(
    db: &Connection,
    epoch: &str,
    state: &str,
    error: Option<&str>,
) -> Result<()> {
    db.execute("INSERT INTO kv(key,value) VALUES('channel.connection',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json!({"epoch":epoch,"state":state,"at":now(),"error":error}).to_string()])?;
    Ok(())
}
