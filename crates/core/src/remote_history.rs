use crate::{file_lock::FileLock, models::*, ssh, store::Store, usage};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS remote_cursors(source_id TEXT NOT NULL,path TEXT NOT NULL,configuration TEXT NOT NULL,cursor TEXT NOT NULL,state TEXT NOT NULL,PRIMARY KEY(source_id,path));")?;
    Ok(())
}
fn configuration(source: &Source, host: &Host) -> String {
    hash(
        &serde_json::to_string(&(
            usage::PARSER_VERSION,
            &source.provider,
            &source.path,
            &source.account_id,
            &source.codex_home_id,
            &source.host_id,
            &host.target,
            &host.port,
            &host.username,
            &host.shell,
            &host.pre_command,
            &host.identity_file,
        ))
        .unwrap(),
    )
}
fn source_lock(root: &Path, source: &Source) -> Result<FileLock> {
    let path = root.join(format!("history-{}.lock", hash(&source.id)));
    let start = Instant::now();
    loop {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match FileLock::try_exclusive(file) {
            Ok(lock) => return Ok(lock),
            Err(std::fs::TryLockError::WouldBlock) if start.elapsed() < Duration::from_secs(45) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) fn scan(root: &Path, settings: &Settings, source: &Source) -> Result<Value> {
    let _lock = source_lock(root, source)?;
    let store = Store::open(root)?;
    let host = settings
        .hosts
        .iter()
        .find(|h| Some(&h.id) == source.host_id.as_ref() && h.enabled)
        .context("主机不存在或已暂停")?;
    let config = configuration(source, host);
    store.db.execute(
        "DELETE FROM remote_cursors WHERE source_id=?1 AND configuration<>?2",
        params![source.id, config],
    )?;
    let mut q = store.db.prepare(
        "SELECT path,cursor FROM remote_cursors WHERE source_id=?1 AND configuration=?2",
    )?;
    let cursors = q
        .query_map(params![source.id, config], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .map(|r| -> Result<_> {
            let (path, cursor) = r?;
            Ok((path, serde_json::from_str::<Value>(&cursor)?))
        })
        .collect::<Result<serde_json::Map<String, Value>>>()?;
    drop(q);
    let request =
        json!({"protocol":2,"path":source.path,"provider":source.provider,"cursors":cursors});
    let prices = store.prices()?;
    let mut count = 0;
    let mut completed = None;
    let mut started = false;
    ssh::history_stream(host, request, |line| {
        // Login profiles can print banners before our protocol starts.
        let frame: Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(_) => {
                ensure!(!started, "远程统计协议包含损坏的批次");
                return Ok(());
            }
        };
        if frame["protocol"] != 2 {
            ensure!(!started, "远程统计协议包含未知数据");
            return Ok(());
        }
        started = true;
        ensure!(completed.is_none(), "远程统计协议在结束后仍返回数据");
        match frame["kind"].as_str() {
            Some("batch") => {
                let current = store.settings()?;
                let current_source = current
                    .sources
                    .iter()
                    .find(|s| s.id == source.id && s.enabled)
                    .context("来源配置已改变，同步已停止")?;
                let current_host = current
                    .hosts
                    .iter()
                    .find(|h| h.id == host.id && h.enabled)
                    .context("主机配置已改变，同步已停止")?;
                ensure!(
                    configuration(current_source, current_host) == config,
                    "来源配置已改变，同步已停止"
                );
                count += apply_batch(&store, settings, source, &config, &prices, &frame)?;
            }
            Some("done") => completed = Some(frame),
            Some("error") => {
                anyhow::bail!("{}", frame["error"].as_str().unwrap_or("远程记录读取失败"))
            }
            _ => anyhow::bail!("远程统计协议格式不正确"),
        }
        Ok(())
    })?;
    let mut result = completed.context("远程记录传输未完成；已提交批次将在下次同步时续传")?;
    result.as_object_mut().unwrap().remove("kind");
    result.as_object_mut().unwrap().remove("protocol");
    result["newEvents"] = json!(count);
    result["updatedAt"] = json!(now());
    store.source_status(&source.id, &result)?;
    Ok(result)
}

fn apply_batch(
    store: &Store,
    settings: &Settings,
    source: &Source,
    config: &str,
    prices: &[ModelPrice],
    frame: &Value,
) -> Result<u64> {
    let path = frame["path"].as_str().context("远程批次缺少文件路径")?;
    let tx = store.db.unchecked_transaction()?;
    let saved: Option<String> = if frame["reset"] == true {
        None
    } else {
        store.db.query_row("SELECT state FROM remote_cursors WHERE source_id=?1 AND path=?2 AND configuration=?3",params![source.id,path,config],|r|r.get(0)).optional()?
    };
    let mut state = saved
        .map(|s| serde_json::from_str::<ParseState>(&s))
        .transpose()?
        .unwrap_or_else(|| ParseState {
            session_id: format!("{}:{}", source.id, path),
            ..Default::default()
        });
    let mut count = 0;
    for row in frame["events"].as_array().context("远程批次缺少事件")? {
        if let Some(event) = usage::parse(source, &mut state, row)
            && store.put_event(&event, prices, settings)?
        {
            count += 1;
        }
        if source.provider == "codex"
            && settings.quota_enabled(source)
            && row["payload"]["rate_limits"].is_object()
            && let Some(stamp) = timestamp(&row["timestamp"])
        {
            store.quota(&usage::codex_quota(
                source,
                &row["payload"]["rate_limits"],
                stamp,
                "log",
            ))?;
        }
    }
    if frame["cursor"].is_object() {
        store.db.execute("INSERT INTO remote_cursors VALUES(?1,?2,?3,?4,?5) ON CONFLICT(source_id,path) DO UPDATE SET configuration=excluded.configuration,cursor=excluded.cursor,state=excluded.state",params![source.id,path,config,serde_json::to_string(&frame["cursor"])?,serde_json::to_string(&state)?])?;
    }
    tx.commit()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_a_sources_host_invalidates_its_cursor_configuration() {
        let mut source = Source {
            host_id: Some("first".into()),
            ..Default::default()
        };
        let host = Host::default();
        let before = configuration(&source, &host);
        source.host_id = Some("second".into());
        assert_ne!(before, configuration(&source, &host));
    }

    #[test]
    fn cumulative_parser_and_cursor_commit_together_and_replay_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "s".into(),
            account_id: "a".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let token = |n| json!({"type":"event_msg","timestamp":now(),"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":n}}}});
        let first = json!({"path":"/fixture","reset":true,"events":[{"type":"session_meta","payload":{"id":"session"}},{"type":"turn_context","payload":{"model":"fixture"}},token(100)],"cursor":{"offset":100}});
        apply_batch(&store, &settings, &source, "v1", &[], &first).unwrap();
        let second = json!({"path":"/fixture","events":[token(150)],"cursor":{"offset":200}});
        apply_batch(&store, &settings, &source, "v1", &[], &second).unwrap();
        apply_batch(&store, &settings, &source, "v1", &[], &second).unwrap();
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            150
        );
        store.db.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON remote_cursors BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let failed = json!({"path":"/fixture","events":[token(200)],"cursor":{"offset":300}});
        assert!(apply_batch(&store, &settings, &source, "v1", &[], &failed).is_err());
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            150
        );
        let cursor: String = store
            .db
            .query_row("SELECT cursor FROM remote_cursors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&cursor).unwrap()["offset"],
            200
        );
    }
}
