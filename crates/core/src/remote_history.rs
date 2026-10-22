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
            usage::parser_version(&source.provider),
            precise,
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
    store.ensure_codex_evidence_scope(settings, source)?;
    let host = settings
        .hosts
        .iter()
        .find(|h| Some(&h.id) == source.host_id.as_ref() && h.enabled)
        .context("主机不存在或已暂停")?;
    crate::history_counting::prepare_source(&store, settings, source)?;
    let config = format!(
        "{}:{}",
        configuration(source, host, settings.precise_counting),
        settings.counting_generation
    );
    store.db.execute(
        "DELETE FROM remote_cursors WHERE source_id=?1 AND configuration<>?2",
        params![source.id, config],
    )?;
    let mut cursors = if full {
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
    if matches!(source.provider.as_str(), "opencode" | "zcode") {
        for (path, cursor) in &mut cursors {
            let pending = |after: i64| -> Result<Vec<i64>> {
                Ok(store.db.prepare("SELECT row_id FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id>?3 ORDER BY row_id LIMIT 257")?
                    .query_map(params![source.id,path,after],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
            };
            let mut rows = pending(cursor["pendingAfter"].as_i64().unwrap_or(0))?;
            if rows.is_empty() {
                rows = pending(0)?;
            }
            cursor["pendingMore"] = json!(rows.len() > 256);
            rows.truncate(256);
            cursor["pending"] = json!(rows);
        }
    }
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
    let qoder_state_cursors = if source.provider == "qoder" {
        crate::qoder_state::remote_cursors(&store, source)?
    } else {
        Value::Null
    };
    let request = json!({"protocol":2,"path":source.path,"provider":source.provider,"cursors":cursors,"routeCursor":route_cursor,"preciseCounting":settings.precise_counting,"countingGeneration":settings.counting_generation,"qoderStateCursors":qoder_state_cursors,"qoderStateReset":full});
    let prices = store.prices()?;
    if source.provider == "codex" {
        store.prepare_history_corrections()?;
    }
    let mut count = 0;
    let mut completed = None;
    let mut started = false;
    let mut qoder_issues = Vec::new();
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
                    format!(
                        "{}:{}",
                        configuration(current_source, current_host, current.precise_counting),
                        current.counting_generation
                    ) == config,
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
            Some("qoderState") => {
                let report =
                    crate::qoder_state::apply_remote_frame(&store, settings, source, &frame)?;
                qoder_issues.extend(report.issues);
            }
            Some("done") => {
                if source.provider == "qoder" {
                    let report = crate::qoder_state::reconcile(&store, source, &prices, settings)?;
                    qoder_issues.extend(report.issues);
                }
                completed = Some(frame);
            }
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
    if !qoder_issues.is_empty() {
        result["partial"] = json!(true);
        result
            .as_object_mut()
            .unwrap()
            .entry("issues")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .extend(qoder_issues);
    }
    if matches!(source.provider.as_str(), "opencode" | "zcode") {
        let states = store
            .db
            .prepare("SELECT state FROM remote_cursors WHERE source_id=?1 AND configuration=?2")?
            .query_map(params![source.id, config], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut malformed = 0;
        for raw in states {
            let state: ParseState = serde_json::from_str(&raw)?;
            malformed += state.database_checkpoint.map_or(0, |c| c.malformed);
        }
        result["malformedRecords"] = json!(malformed);
        result["pendingRecords"] = json!(store.db.query_row("SELECT count(*) FROM native_pending p JOIN remote_cursors c ON c.source_id=p.source_id AND c.path=p.path WHERE p.source_id=?1 AND c.configuration=?2 AND p.invalid=0",params![source.id,config],|r|r.get::<_,u64>(0))?);
        if malformed > 0 {
            result["partial"] = json!(true);
            if let Some(issues) = result["issues"].as_array_mut() {
                issues.push(json!({"code":"invalidRecord","message":"部分原生统计格式无效，将在记录变化后重试；已保留其他计量。"}));
            }
        }
    }
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
            session_id: if source.provider == "kimi" {
                crate::native_agents::session_id(Path::new(path))
            } else {
                format!("{}:{}", source.id, path)
            },
            precise_counting: Some(settings.precise_counting),
            counting_generation: settings.counting_generation,
            ..Default::default()
        });
    let mut count = 0;
    if let Some(rows) = frame["nativeRows"].as_array() {
        ensure!(
            matches!(source.provider.as_str(), "opencode" | "zcode"),
            "原生统计来源类型不一致"
        );
        ensure!(rows.len() <= 256, "原生统计批次过大");
        let mut checkpoint = state.database_checkpoint.take().unwrap_or_default();
        if frame["nativeReset"] == true {
            store.db.execute(
                "DELETE FROM native_pending WHERE source_id=?1 AND path=?2",
                params![source.id, path],
            )?;
            checkpoint.malformed = 0;
        }
        if let Some(gone) = frame["gone"].as_array() {
            ensure!(gone.len() <= 256, "原生统计重试批次过大");
            for rowid in gone {
                let rowid = rowid.as_i64().context("原生统计行号无效")?;
                let invalid: Option<bool> = store.db.query_row("DELETE FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3 RETURNING invalid",params![source.id,path,rowid],|r|r.get(0)).optional()?;
                checkpoint.malformed = checkpoint
                    .malformed
                    .saturating_sub(u64::from(invalid == Some(true)));
            }
        }
        for row in rows {
            let rowid = row["rowid"]
                .as_i64()
                .filter(|n| *n > 0)
                .context("原生统计行号无效")?;
            let record = if row["data"]["invalidMetadata"] == true {
                Err(anyhow::anyhow!("原生统计元数据无效"))
            } else if source.provider == "opencode" {
                crate::opencode::record(source, row["id"].as_str().unwrap_or(""), &row["data"])
            } else {
                crate::zcode::record(source, row["id"].as_str().unwrap_or(""), &row["data"])
            };
            let invalid = record.is_err();
            let pending = invalid || matches!(record, Ok(crate::native_db::Record::Pending));
            if let Ok(crate::native_db::Record::Event(event)) = record
                && store.put_event(&event, prices, settings)?
            {
                count += 1;
            }
            let was_invalid: Option<bool> = store.db.query_row("SELECT invalid FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3",params![source.id,path,rowid],|r|r.get(0)).optional()?;
            checkpoint.malformed = checkpoint
                .malformed
                .saturating_sub(u64::from(was_invalid == Some(true)))
                + u64::from(invalid);
            if pending {
                store.db.execute("INSERT INTO native_pending(source_id,path,row_id,invalid) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,path,row_id) DO UPDATE SET invalid=excluded.invalid",params![source.id,path,rowid,invalid])?;
            } else {
                store.db.execute(
                    "DELETE FROM native_pending WHERE source_id=?1 AND path=?2 AND row_id=?3",
                    params![source.id, path, rowid],
                )?;
            }
        }
        state.database_checkpoint = Some(checkpoint);
    }
    for row in frame["events"].as_array().context("远程批次缺少事件")? {
        if source.provider == "deepseek-harness" {
            crate::harness_history::prepare(store, source, row)?;
        }
        if source.provider == "opencode" && row["opencode"].is_object() {
            let row = &row["opencode"];
            let event = crate::opencode::event(
                source,
                row["id"].as_str().context("OpenCode 请求标识缺失")?,
                row["session"].as_str().context("OpenCode 会话标识缺失")?,
                &row["data"],
            )?;
            if store.put_event(&event, prices, settings)? {
                count += 1;
            }
            continue;
        }
        if source.provider == "grok" && row.get("method").is_some() {
            for event in crate::native_agents::grok_events(source, row) {
                if store.put_event(&event, prices, settings)? {
                    count += 1;
                }
            }
            continue;
        }
        if source.provider == "kimi" {
            state.inherited_usage = row["_aieyesInherited"] == true;
        }
        let parsed = usage::parse(source, &mut state, row);
        store.record_codex_history(source, &state, row, parsed.as_ref())?;
        if settings.precise_counting
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
            if source.provider == "deepseek-harness" {
                crate::harness_history::put(store, &event, row, prices, settings)?
            } else if matches!(source.provider.as_str(), "pi" | "omp") {
                crate::pi_history::put(store, &event, &state, prices, settings)?
            } else {
                store.put_event(&event, prices, settings)?
            }
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
    fn native_batches_retry_invalid_and_pending_rows_without_changing_frozen_attribution() {
        for provider in ["opencode", "zcode"] {
            let root = tempfile::tempdir().unwrap();
            let store = Store::open(root.path()).unwrap();
            let source = Source {
                id: "native".into(),
                provider: provider.into(),
                path: "/fixture".into(),
                ..Default::default()
            };
            store
                .save_settings(&Settings {
                    sources: vec![source.clone()],
                    ..Default::default()
                })
                .unwrap();
            let settings = store.settings().unwrap();
            let prices = vec![ModelPrice {
                id: "fixture".into(),
                input: Some(1.0),
                output: Some(2.0),
                cache_read: Some(0.5),
                ..Default::default()
            }];
            let native = |rowid: i64, complete: bool, invalid: bool| {
                let input = if invalid { -1 } else { 10 };
                let data = if provider == "opencode" {
                    json!({"session":"session","role":"assistant","model":"fixture","provider":"backend","completed":if complete { json!(1792296856613_u64) } else { Value::Null },"tokens":{"input":input,"output":2,"reasoning":1,"cache":{"read":3,"write":0}}})
                } else {
                    json!({"id":format!("r{rowid}"),"session":"session","turn":"turn","status":if complete { "completed" } else { "running" },"model":"fixture","completed":if complete { 1792296856613_u64 } else { 0 },"input":input,"output":3,"read":3,"write":0,"reasoning":1,"providerTotal":16,"computedTotal":16})
                };
                json!({"rowid":rowid,"id":format!("r{rowid}"),"data":data})
            };
            let first = json!({"path":"native.db","nativeReset":true,"nativeRows":[native(1,true,false),native(2,false,false),native(3,true,true)],"events":[],"cursor":{"offset":3}});
            assert_eq!(
                apply_batch(&store, &settings, &source, "config", &prices, &first).unwrap(),
                1
            );
            assert_eq!(
                store
                    .db
                    .query_row("SELECT count(*) FROM native_pending", [], |r| r
                        .get::<_, u64>(0))
                    .unwrap(),
                2
            );
            let frozen = store
                .db
                .query_row("SELECT cost FROM events", [], |r| r.get::<_, f64>(0))
                .unwrap();
            assert!(frozen > 0.0);
            let state = || {
                let raw: String = store
                    .db
                    .query_row("SELECT state FROM remote_cursors", [], |r| r.get(0))
                    .unwrap();
                serde_json::from_str::<ParseState>(&raw)
                    .unwrap()
                    .database_checkpoint
                    .unwrap()
                    .malformed
            };
            assert_eq!(state(), 1);
            let second = json!({"path":"native.db","nativeRows":[native(2,true,false)],"gone":[3],"events":[],"cursor":{"offset":3}});
            assert_eq!(
                apply_batch(&store, &settings, &source, "config", &prices, &second).unwrap(),
                1
            );
            assert_eq!(state(), 0);
            assert_eq!(
                store
                    .db
                    .query_row("SELECT count(*) FROM native_pending", [], |r| r
                        .get::<_, u64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                apply_batch(&store, &settings, &source, "config", &prices, &second).unwrap(),
                0
            );
            let mut changed = settings.clone();
            changed.sources[0].account_id = "new-account".into();
            changed.accounts.push(Account {
                id: "new-account".into(),
                name: "Test account".into(),
                provider: provider.into(),
                ..Default::default()
            });
            store.save_settings(&changed).unwrap();
            let changed = store.settings().unwrap();
            let new_source = &changed.sources[0];
            let replay = json!({"path":"native.db","nativeRows":[native(1,true,false)],"events":[],"cursor":{"offset":3}});
            let new_prices = [ModelPrice {
                id: "fixture".into(),
                input: Some(100.0),
                output: Some(200.0),
                ..Default::default()
            }];
            assert_eq!(
                apply_batch(&store, &changed, new_source, "config", &new_prices, &replay).unwrap(),
                0
            );
            let totals: (u64, f64, u64) = store
                .db
                .query_row(
                    "SELECT count(*),sum(cost),sum(account_id='') FROM events",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(totals, (2, frozen * 2.0, 2));
            let mut moved = changed.clone();
            moved.sources[0].path = "/another".into();
            store.save_settings(&moved).unwrap();
            assert!(apply_batch(&store, &changed, new_source, "config", &prices, &replay).is_err());
        }
    }
    #[test]
    fn harness_projected_batches_keep_native_identity_seed_boundary_and_replay_safety() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "harness".into(),
            provider: "deepseek-harness".into(),
            path: "/fixture".into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let settings = store.settings().unwrap();
        let prices = store.prices().unwrap();
        let usage = |seq: u64, input: u64| json!({"type":"assistant/message","seq":seq,"time":1792296856613_u64,"data":{"turn":2,"message":{"source":{"model":"fixture"}},"usage":{"inputTokens":input,"outputTokens":2}}});
        let first = json!({"kind":"batch","path":"session.v4.jsonl.zstd","reset":true,"events":[{"type":"session","version":4,"id":"native-session","isSeeded":true},usage(1,999),{"type":"session/end-seed"},usage(10,11)],"cursor":null});
        let second = json!({"kind":"batch","path":"session.v4.jsonl.zstd","reset":true,"events":[{"type":"session","version":4,"id":"native-session","isSeeded":false},usage(11,22)],"cursor":{"harness":"fingerprint"}});
        assert_eq!(
            apply_batch(&store, &settings, &source, "config", &prices, &first).unwrap(),
            1
        );
        assert_eq!(
            apply_batch(&store, &settings, &source, "config", &prices, &second).unwrap(),
            1
        );
        assert_eq!(
            apply_batch(&store, &settings, &source, "config", &prices, &second).unwrap(),
            0
        );
        let mut query = store.db.prepare("SELECT payload FROM events").unwrap();
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str::<Value>(&r.unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r["sessionId"] == "native-session"));
        assert_eq!(
            rows.iter()
                .filter(|r| r["inheritance"]["inherited"] == true && r["tokens"]["input"] == 0)
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .map(|r| r["tokens"]["input"].as_u64().unwrap())
                .sum::<u64>(),
            33
        );
        let mut settings = settings;
        for (mode, total) in [(false, 1038), (true, 37)] {
            settings.precise_counting = mode;
            store.save_settings(&settings).unwrap();
            settings = store.settings().unwrap();
            crate::history_counting::prepare_source(&store, &settings, &source).unwrap();
            apply_batch(&store, &settings, &source, "config", &prices, &first).unwrap();
            apply_batch(&store, &settings, &source, "config", &prices, &second).unwrap();
            assert_eq!(
                store
                    .dashboard(&Filter {
                        date_range: Some(DateRange::All),
                        ..Default::default()
                    })
                    .unwrap()
                    .summary
                    .total,
                total
            );
        }
    }
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
            settings.precise_counting = precise;
            store.save_settings(&settings).unwrap();
            if previous.precise_counting != precise {
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
