//! Immutable Harness generations share requests even when migration renumbers rows.
use crate::{models::*, store::Store, usage};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn generation(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    if matches!(name, "session.jsonl" | "session.jsonl.zstd") {
        return Some(0);
    }
    let raw = name
        .strip_prefix("session.v")?
        .strip_suffix(".jsonl.zstd")
        .or_else(|| name.strip_prefix("session.v")?.strip_suffix(".jsonl"))?;
    let version: u64 = raw.parse().ok()?;
    (version > 0 && version.to_string() == raw).then_some(version)
}
pub(crate) fn paths(root: &Path) -> Result<Vec<PathBuf>> {
    let explicit = root.is_file();
    ensure!(
        !explicit || !root.is_symlink(),
        "请选择 Harness 会话的实际文件"
    );
    let folder = if root.join("sessions").is_dir() {
        root.join("sessions")
    } else {
        root.to_path_buf()
    };
    let mut selected = BTreeMap::<PathBuf, BTreeMap<u64, Vec<PathBuf>>>::new();
    for entry in walkdir::WalkDir::new(folder).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(version) = generation(entry.path()) else {
            continue;
        };
        ensure!(version <= 4, "Harness 会话版本高于 v4，需要更新适配器");
        let parent = entry.path().parent().unwrap_or(entry.path()).to_path_buf();
        selected
            .entry(parent)
            .or_default()
            .entry(version)
            .or_default()
            .push(entry.into_path());
    }
    selected
        .into_values()
        .map(|mut versions| {
            let (version, mut files) = versions.pop_last().unwrap();
            ensure!(
                explicit || files.len() == 1,
                "Harness 同一会话存在两种编码，请选择明确的文件"
            );
            ensure!(
                matches!(version, 0 | 4),
                "Harness v1–v3 统计适配尚未接入，请保留原始记录"
            );
            Ok(files.remove(0))
        })
        .collect()
}

pub(crate) fn put(
    store: &Store,
    event: &UsageEvent,
    row: &Value,
    prices: &[ModelPrice],
    settings: &Settings,
) -> Result<bool> {
    if row["harnessVersion"] != 0
        && let Some(seq) = row["seq"].as_u64()
    {
        let identity = format!("{}:seq:{seq}", event.session_id);
        let old_id = usage::event_id(
            &event.provider,
            &event.source_id,
            &event.account_id,
            &identity,
        );
        if old_id != event.id {
            let old: Option<(String, Option<String>)> = store
                .db
                .query_row(
                    "SELECT payload,price FROM events WHERE id=?1",
                    [&old_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((raw, price)) = old {
                let mut prior: UsageEvent = serde_json::from_str(&raw)?;
                ensure!(
                    prior.session_id == event.session_id
                        && prior.timestamp == event.timestamp
                        && prior.model == event.model
                        && prior.turn_id == event.turn_id,
                    "Harness 旧请求身份与当前记录不一致，保留原始计量"
                );
                prior.id = event.id.clone();
                prior.import_identity = event.import_identity.clone();
                let exists: bool = store.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1)",
                    [&event.id],
                    |r| r.get(0),
                )?;
                if exists {
                    let frozen: Vec<ModelPrice> = price
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?
                        .into_iter()
                        .collect();
                    store.merge_usage_event(prior.clone(), &prior.source_id, &frozen, settings)?;
                } else {
                    store.db.execute("INSERT INTO events SELECT ?1,provider,account_id,model,stamp,?2,cost,priced_tokens,price FROM events WHERE id=?3",params![event.id,serde_json::to_string(&prior)?,old_id])?;
                }
                store.db.execute("INSERT OR IGNORE INTO event_sources SELECT ?1,source_id FROM event_sources WHERE event_id=?2",params![event.id,old_id])?;
                store
                    .db
                    .execute("DELETE FROM event_sources WHERE event_id=?1", [&old_id])?;
                store
                    .db
                    .execute("DELETE FROM events WHERE id=?1", [&old_id])?;
            }
        }
    }
    store.put_event(event, prices, settings)
}

pub(crate) fn prepare(store: &Store, source: &Source, row: &Value) -> Result<()> {
    if row["type"] == "session" && row["harnessVersion"] == 0 {
        let prefix = format!("{}:seq:", row["id"].as_str().unwrap_or(""));
        // Partial index: no growing per-session scan across unrelated history,
        // and other agents do not acquire additional indexed payload columns.
        store.db.execute_batch("CREATE INDEX IF NOT EXISTS harness_legacy_identity ON events(account_id,json_extract(payload,'$.sessionId'),json_extract(payload,'$.importIdentity')) WHERE provider='deepseek-harness'")?;
        let upper = format!("{};", prefix.trim_end_matches(':'));
        let unresolved:bool=store.db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE provider='deepseek-harness' AND account_id=?1 AND json_extract(payload,'$.sessionId')=?3 AND json_extract(payload,'$.importIdentity')>=?4 AND json_extract(payload,'$.importIdentity')<?5 AND (?1!='' OR json_extract(payload,'$.sourceId')=?2))",params![source.account_id,source.id,row["id"].as_str().unwrap_or(""),prefix,upper],|r|r.get(0))?;
        ensure!(
            !unresolved,
            "此 Harness 会话仍有旧版 v4 行号身份，请先重扫原 v4 来源后再导入 v0，避免重复计量"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{import, native_agents};
    use serde_json::json;
    fn row(seq: u64) -> Value {
        json!({"type":"assistant/message","seq":seq,"time":now()*1000,"data":{"turn":1,"step":1,"message":{"source":{"kind":"model","model":"fixture"},"content":"PRIVATE"},"usage":{"inputTokens":10,"outputTokens":2,"cacheReadTokens":3,"reasoningTokens":1}}})
    }
    #[test]
    fn v0_seed_prefix_and_modern_generation_share_one_request_despite_renumbering() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("app")).unwrap();
        let folder = root.path().join("client/sessions/project/session");
        std::fs::create_dir_all(&folder).unwrap();
        let source = Source {
            id: "harness".into(),
            provider: "deepseek-harness".into(),
            path: root.path().join("client").to_string_lossy().into(),
            ..Default::default()
        };
        store
            .save_settings(&Settings {
                sources: vec![source.clone()],
                ..Default::default()
            })
            .unwrap();
        let settings = store.settings().unwrap();
        let request = row(2);
        let mut inherited = request.clone();
        inherited["seq"] = json!(0);
        inherited["data"]["usage"]["inputTokens"] = json!(999);
        let legacy = format!(
            "{}\n{}\n{}\n{}\n",
            json!({"type":"session","version":0,"id":"session","seedLength":2}),
            inherited,
            json!({"type":"permission/preset","seq":1}),
            request
        );
        let old = folder.join("session.jsonl.zstd");
        std::fs::write(
            &old,
            zstd::stream::encode_all(legacy.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        assert_eq!(
            import::scan(&store, &settings, &source).unwrap()["newEvents"],
            1
        );
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            15
        );
        let mut modern = request;
        modern["seq"] = json!(12);
        let data = format!(
            "{}\n{}\n",
            json!({"type":"session","version":4,"id":"session","isSeeded":false}),
            modern
        );
        std::fs::write(folder.join("session.v4.jsonl"), data).unwrap();
        // Obsolete encodings do not conflict with the selected higher generation.
        std::fs::write(folder.join("session.jsonl"), legacy).unwrap();
        assert_eq!(
            paths(&folder).unwrap(),
            vec![folder.join("session.v4.jsonl")]
        );
        assert_eq!(
            import::rescan(&store, &settings, &source).unwrap()["newEvents"],
            0
        );
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            15
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            1
        );
        let mut found = 0;
        native_agents::read_dsh(&old, |r| {
            found += usize::from(r["type"] == "assistant/message");
            Ok(())
        })
        .unwrap();
        assert_eq!(found, 1);
        std::fs::write(folder.join("session.v5.jsonl"), "{}\n").unwrap();
        assert!(paths(&folder).is_err());
        let malformed = folder.join("session.v4.jsonl");
        std::fs::write(
            &malformed,
            format!(
                "{}\n[]\n",
                json!({"type":"session","version":4,"id":"session"})
            ),
        )
        .unwrap();
        assert!(native_agents::read_dsh(&malformed, |_| Ok(())).is_err());
    }
    #[test]
    fn previous_v4_sequence_identity_migrates_without_losing_source_membership() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let source = Source {
            id: "a".into(),
            provider: "deepseek-harness".into(),
            path: "/fixture".into(),
            account_id: "account".into(),
            ..Default::default()
        };
        let mut b = source.clone();
        b.id = "b".into();
        let settings = Settings {
            sources: vec![source.clone(), b.clone()],
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        let mut state = ParseState {
            session_id: "session".into(),
            ..Default::default()
        };
        let raw = row(12);
        let current = native_agents::parse_dsh(&source, &mut state, &raw).unwrap();
        let mut old = current.clone();
        old.import_identity = Some("session:seq:12".into());
        old.id = usage::event_id(
            &old.provider,
            &old.source_id,
            &old.account_id,
            old.import_identity.as_ref().unwrap(),
        );
        let prices = vec![ModelPrice {
            id: "fixture".into(),
            input: Some(1.0),
            output: Some(2.0),
            cache_read: Some(0.5),
            ..Default::default()
        }];
        store.put_event(&old, &prices, &settings).unwrap();
        let mut copy = old.clone();
        copy.source_id = b.id;
        store.put_event(&copy, &prices, &settings).unwrap();
        let cost: f64 = store
            .db
            .query_row("SELECT cost FROM events WHERE id=?1", [&old.id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(cost > 0.0);
        let header = json!({"type":"session","id":"session","harnessVersion":0});
        assert!(prepare(&store, &source, &header).is_err());
        let tx = store.write_transaction().unwrap();
        assert!(!put(&store, &current, &raw, &[], &settings).unwrap());
        tx.commit().unwrap();
        prepare(&store, &source, &header).unwrap();
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            1
        );
        for id in ["a", "b"] {
            assert_eq!(
                store
                    .dashboard(&Filter {
                        source_id: Some(id.into()),
                        ..Default::default()
                    })
                    .unwrap()
                    .summary
                    .total,
                15
            );
        }
        let payload: String = store
            .db
            .query_row(
                "SELECT payload FROM events WHERE id=?1",
                [&current.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!payload.contains("PRIVATE"));
        assert_eq!(
            store
                .db
                .query_row("SELECT cost FROM events WHERE id=?1", [&current.id], |r| {
                    r.get::<_, f64>(0)
                })
                .unwrap(),
            cost
        );
    }
}
