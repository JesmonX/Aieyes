use crate::{models::*, ssh, store::Store, usage};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS remote_cursors(source_id TEXT NOT NULL,path TEXT NOT NULL,configuration TEXT NOT NULL,cursor TEXT NOT NULL,state TEXT NOT NULL,PRIMARY KEY(source_id,path));")?;
    Ok(())
}
fn configuration(source: &Source, host: &Host, precise: bool) -> String {
    hash(
        &serde_json::to_string(&(
            usage::PARSER_VERSION,
            source.provider == "codex" && precise,
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

pub(crate) fn scan(root: &Path, settings: &Settings, source: &Source) -> Result<Value> {
    scan_mode(root, settings, source, false)
}

pub(crate) fn rescan(root: &Path, settings: &Settings, source: &Source) -> Result<Value> {
    scan_mode(root, settings, source, true)
}

fn scan_mode(root: &Path, settings: &Settings, source: &Source, full: bool) -> Result<Value> {
    let _lock = crate::file_lock::history(root, &source.id)?;
    let store = Store::open(root)?;
    let current = crate::import::current_settings(&store, settings, source)?;
    let settings = &current;
    let host = settings
        .hosts
        .iter()
        .find(|h| Some(&h.id) == source.host_id.as_ref() && h.enabled)
        .context("主机不存在或已暂停")?;
    let config = configuration(source, host, settings.codex_precise_counting);
    store.db.execute(
        "DELETE FROM remote_cursors WHERE source_id=?1 AND configuration<>?2",
        params![source.id, config],
    )?;
    let cursors = if full {
        serde_json::Map::new()
    } else {
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
        cursors
    };
    let route_key = format!("remote-route-cursor:{}:{config}", source.id);
    let route_cursor: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [&route_key], |r| {
            r.get(0)
        })
        .optional()?;
    let route_cursor: Value = route_cursor
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?
        .unwrap_or(Value::Null);
    let request = json!({"protocol":2,"path":source.path,"provider":source.provider,"cursors":cursors,"routeCursor":route_cursor,"codexPreciseCounting":settings.codex_precise_counting});
    let prices = store.prices()?;
    if source.provider == "codex" {
        store.prepare_history_corrections()?;
    }
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
            Some("routes") => {
                let tx = store.write_transaction()?;
                let current = crate::import::current_settings(&store, settings, source)?;
                let current_source = current
                    .sources
                    .iter()
                    .find(|s| s.id == source.id && s.enabled)
                    .context("来源配置已改变")?;
                let current_host = current
                    .hosts
                    .iter()
                    .find(|h| h.id == host.id && h.enabled)
                    .context("主机配置已改变")?;
                ensure!(
                    configuration(current_source, current_host, current.codex_precise_counting)
                        == config,
                    "来源配置已改变，同步已停止"
                );
                let routes: Vec<crate::codex_routes::Route> =
                    serde_json::from_value(frame["routes"].clone())?;
                store.put_codex_routes(&source.id, &routes)?;
                store.db.execute("INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![route_key,frame["cursor"].to_string()])?;
                tx.commit()?;
            }
            Some("batch") => {
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
    let tx = store.write_transaction()?;
    crate::import::current_settings(&store, settings, source)?;
    store.source_status(&source.id, &result)?;
    tx.commit()?;
    store.repair_history_estimates()?;
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
    let tx = store.write_transaction()?;
    let current = crate::import::current_settings(store, settings, source)?;
    let settings = &current;
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
            codex_precise_counting: Some(settings.codex_precise_counting),
            ..Default::default()
        });
    let mut count = 0;
    for row in frame["events"].as_array().context("远程批次缺少事件")? {
        let parsed = usage::parse(source, &mut state, row);
        if settings.codex_precise_counting
            && state.first_inherited_count
            && row["inheritedHistory"].is_object()
        {
            crate::codex_history::seed(
                &mut state,
                serde_json::from_value(row["inheritedHistory"].clone())?,
            );
        }
        let changed = if let Some(correction) = state.counter_correction.take() {
            store.put_counter_correction(&correction, prices, settings)?
                && correction.event.tokens.total() > 0
        } else if let Some(event) = parsed {
            store.put_event(&event, prices, settings)?
        } else {
            false
        };
        if changed {
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
    fn two_sources_commit_batches_while_configuration_and_quota_are_written() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let sources: Vec<_> = (0..2)
            .map(|i| Source {
                id: format!("s{i}"),
                account_id: "a".into(),
                provider: "custom".into(),
                path: "/fixture".into(),
                ..Default::default()
            })
            .collect();
        let settings = Settings {
            sources: sources.clone(),
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for source in &sources {
                let settings = &settings;
                let barrier = &barrier;
                let root = root.path();
                scope.spawn(move || {
                    let store = Store::open(root).unwrap();
                    barrier.wait();
                    for _ in 0..2 {
                        for n in 0..40 {
                            let event = json!({"id":format!("{}-{n}",source.id),"sessionId":source.id,"model":"fixture","timestamp":now(),"tokens":{"input":10}});
                            let frame = json!({"path":"/fixture","reset":n==0,"events":[event],"cursor":{"offset":n+1}});
                            apply_batch(&store, settings, source, "config", &[], &frame).unwrap();
                        }
                    }
                });
            }
            barrier.wait();
            for n in 0..40 {
                let mut next = settings.clone();
                next.refresh_seconds = 300 + n;
                store.save_settings(&next).unwrap();
                store
                    .quota(&QuotaSnapshot {
                        account_id: "a".into(),
                        provider: "custom".into(),
                        updated_at: now(),
                        ..Default::default()
                    })
                    .unwrap();
            }
        });
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            800
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM event_sources", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            80
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT COUNT(*) FROM remote_cursors WHERE json_extract(cursor,'$.offset')=40",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }
    #[test]
    fn changing_a_sources_host_invalidates_its_cursor_configuration() {
        let mut source = Source {
            host_id: Some("first".into()),
            ..Default::default()
        };
        let host = Host::default();
        let before = configuration(&source, &host, false);
        assert_ne!(before, configuration(&source, &host, true));
        source.host_id = Some("second".into());
        assert_ne!(before, configuration(&source, &host, false));
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

    #[test]
    fn remote_fork_replay_uses_saved_mode_and_rejects_stale_batches() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "fork".into(),
            provider: "codex".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        let mut settings = Settings {
            sources: vec![source.clone()],
            ..Default::default()
        };
        let stamp = now();
        let frame = json!({"path":"/fixture/child.jsonl","reset":true,"cursor":{"offset":100},"events":[
            {"type":"session_meta","payload":{"id":"child","history_base":{"thread_id":"parent","end_byte_offset":100}},"inheritedHistory":{"parentThread":"parent","endByteOffset":100,"tokens":{"input":900},"proof":"fixture"}},
            {"type":"event_msg","timestamp":stamp,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000},"last_token_usage":{"input_tokens":100}}}}
        ]});
        for (precise, expected) in [(false, 1000), (true, 100), (false, 1000), (true, 100)] {
            let previous = settings.clone();
            settings.codex_precise_counting = precise;
            store.save_settings(&settings).unwrap();
            if previous.codex_precise_counting != precise {
                assert!(apply_batch(&store, &previous, &source, "mode", &[], &frame).is_err());
            }
            apply_batch(&store, &settings, &source, "mode", &[], &frame).unwrap();
            apply_batch(&store, &settings, &source, "mode", &[], &frame).unwrap();
            assert_eq!(
                store.dashboard(&Filter::default()).unwrap().summary.total,
                expected
            );
        }
    }
}
