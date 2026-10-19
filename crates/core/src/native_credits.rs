//! Resolve native parent credit mirrors without changing token accounting.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS native_credit_aliases(scope TEXT NOT NULL,alias TEXT NOT NULL,event_id TEXT NOT NULL,PRIMARY KEY(scope,alias));
      CREATE INDEX IF NOT EXISTS native_credit_alias_event ON native_credit_aliases(event_id);
      CREATE INDEX IF NOT EXISTS native_credit_mirror ON events(json_extract(payload,'$.nativeCredits.mirrorOf')) WHERE provider='codebuddy';
      CREATE INDEX IF NOT EXISTS native_credit_resolved ON events(json_extract(payload,'$.nativeCredits.mirroredBy')) WHERE provider='codebuddy';
      CREATE TRIGGER IF NOT EXISTS native_credit_deleted AFTER DELETE ON events BEGIN
        DELETE FROM native_credit_aliases WHERE event_id=OLD.id;
        UPDATE events SET payload=json_remove(payload,'$.nativeCredits.mirroredBy') WHERE provider='codebuddy' AND json_extract(payload,'$.nativeCredits.mirroredBy')=OLD.id;
      END;")?;
    // A source gaining or losing a child changes the parent's projected day,
    // which may differ from the day invalidated by the ordinary membership trigger.
    for (name, action, rows) in [
        ("insert", "INSERT", "NEW.event_id"),
        ("delete", "DELETE", "OLD.event_id"),
        ("update", "UPDATE", "OLD.event_id,NEW.event_id"),
    ] {
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS native_credit_membership_{name} AFTER {action} ON event_sources BEGIN
          INSERT INTO usage_day_revision SELECT ((stamp-CASE WHEN stamp<0 THEN 86399 ELSE 0 END)/86400),1 FROM events WHERE provider='codebuddy' AND json_extract(payload,'$.nativeCredits.mirroredBy') IN ({rows}) GROUP BY ((stamp-CASE WHEN stamp<0 THEN 86399 ELSE 0 END)/86400) ON CONFLICT(bucket) DO UPDATE SET revision=revision+1;
        END;"))?;
    }
    Ok(())
}
fn scope(e: &UsageEvent) -> String {
    hash(&format!(
        "{}:{}",
        e.provider,
        if e.account_id.is_empty() {
            format!("source:{}", e.source_id)
        } else {
            format!("account:{}", e.account_id)
        }
    ))
}
pub(crate) fn rekey(store: &Store, old: &str, new: &str) -> Result<()> {
    store.db.execute(
        "UPDATE native_credit_aliases SET event_id=?1 WHERE event_id=?2",
        params![new, old],
    )?;
    store.db.execute("UPDATE events SET payload=json_set(payload,'$.nativeCredits.mirroredBy',?1) WHERE provider='codebuddy' AND json_extract(payload,'$.nativeCredits.mirroredBy')=?2",params![new,old])?;
    Ok(())
}
pub(crate) fn resolve(store: &Store, e: &mut UsageEvent) -> Result<()> {
    if e.provider != "codebuddy" {
        return Ok(());
    }
    let scope = scope(e);
    let Some(c) = &mut e.native_credits else {
        return Ok(());
    };
    c.mirrored_by = None;
    if let Some(alias) = &c.mirror_of {
        c.mirrored_by=store.db.query_row("SELECT a.event_id FROM native_credit_aliases a JOIN events e ON e.id=a.event_id WHERE a.scope=?1 AND a.alias=?2 AND json_extract(e.payload,'$.nativeCredits.credits')>0 AND json_extract(e.payload,'$.nativeCredits.mirrorOf') IS NULL",params![scope,alias],|r|r.get(0)).optional()?;
    }
    Ok(())
}
pub(crate) fn persisted(store: &Store, e: &UsageEvent) -> Result<()> {
    if e.provider != "codebuddy" {
        return Ok(());
    }
    let Some(c) = e
        .native_credits
        .as_ref()
        .filter(|c| c.valid() && c.mirror_of.is_none() && c.credits > 0.0)
    else {
        return Ok(());
    };
    let scope = scope(e);
    for alias in &c.aliases {
        store.db.execute("INSERT INTO native_credit_aliases VALUES(?1,?2,?3) ON CONFLICT(scope,alias) DO UPDATE SET event_id=excluded.event_id WHERE event_id IS NOT excluded.event_id",params![scope,alias,e.id])?;
        store.db.execute("UPDATE events SET payload=json_set(payload,'$.nativeCredits.mirroredBy',?1) WHERE provider='codebuddy' AND account_id=?2 AND (?2!='' OR json_extract(payload,'$.sourceId')=?3) AND json_extract(payload,'$.nativeCredits.mirrorOf')=?4 AND json_extract(payload,'$.nativeCredits.mirroredBy') IS NOT ?1",params![e.id,e.account_id,e.source_id,alias])?;
    }
    Ok(())
}
pub(crate) fn prepare_view(
    store: &Store,
    e: &mut UsageEvent,
    source: Option<&str>,
) -> Result<bool> {
    if let (Some(source), Some(c)) = (source, e.native_credits.as_mut())
        && let Some(target) = &c.mirrored_by
    {
        let visible: bool = store.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM event_sources WHERE event_id=?1 AND source_id=?2)",
            params![target, source],
            |r| r.get(0),
        )?;
        if !visible {
            c.mirrored_by = None;
        }
    }
    Ok(e.tokens.total() > 0
        || e.native_credits
            .as_ref()
            .is_some_and(|c| c.valid() && c.mirrored_by.is_none()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{usage, usage_cache};
    use serde_json::{Value, json};
    fn fixture() -> (tempfile::TempDir, Store, Settings) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let settings = Settings {
            sources: ["a", "b"]
                .into_iter()
                .map(|id| Source {
                    id: id.into(),
                    provider: "codebuddy".into(),
                    account_id: "account".into(),
                    path: "/fixture".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        (root, store, settings)
    }
    fn parent(stamp: i64) -> Value {
        json!({"id":"parent-credit","type":"credit-usage","sessionId":"parent","timestamp":stamp*1000,"providerData":{"conversationRequestId":"parent-turn","source":"team_member","stageId":"team:child-row","credit":0.5,"content":"PRIVATE"}})
    }
    fn child(stamp: i64) -> Value {
        json!({"id":"child-row","type":"model-usage","sessionId":"child","timestamp":stamp*1000,"providerData":{"conversationRequestId":"child-turn","messageId":"backend-message","model":"fixture","rawUsage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":30},"credit":0.5}},"content":"PRIVATE"})
    }
    fn save(store: &Store, settings: &Settings, source: usize, row: &Value) -> UsageEvent {
        let e = usage::parse(&settings.sources[source], &mut ParseState::default(), row).unwrap();
        store.put_event(&e, &[], settings).unwrap();
        e
    }
    fn value(store: &Store, filter: Filter) -> f64 {
        let (cached, pending) = usage_cache::read(store, &filter, false).unwrap();
        let raw = store.dashboard_usage_reference(&filter).unwrap();
        assert_eq!(
            serde_json::to_value(&cached.summary).unwrap(),
            serde_json::to_value(&raw.summary).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&cached.models).unwrap(),
            serde_json::to_value(&raw.models).unwrap()
        );
        assert_eq!(cached.model_options, raw.model_options);
        assert_eq!(
            serde_json::to_value(&cached.trend_days).unwrap(),
            serde_json::to_value(&raw.trend_days).unwrap()
        );
        usage_cache::persist(store, pending).unwrap();
        cached
            .summary
            .native_credits
            .get("codebuddy")
            .map(|c| c.credits)
            .unwrap_or(0.0)
    }
    #[test]
    fn codebuddy_main_outputs_and_generation_stages_are_unique_while_parent_mirrors_follow_children()
     {
        for child_first in [false, true] {
            let (_root, store, settings) = fixture();
            let stamp = now() - 10;
            let main = child(stamp);
            if child_first {
                save(&store, &settings, 0, &main);
            }
            let mirrored = save(&store, &settings, 0, &parent(stamp));
            let generated = json!({"id":"generated","type":"credit-usage","sessionId":"parent","timestamp":stamp*1000,"providerData":{"source":"generation_tool","stageId":"image:one","conversationRequestId":"parent-turn","credit":0.2}});
            save(&store, &settings, 0, &generated);
            save(&store, &settings, 0, &generated);
            if !child_first {
                assert!((value(&store, Filter::default()) - 0.7).abs() < 1e-12);
            }
            save(&store, &settings, 0, &main);
            let mut output = main.clone();
            output["type"] = json!("function_call");
            output["id"] = json!("child-tool-output");
            output["providerData"]
                .as_object_mut()
                .unwrap()
                .remove("model");
            save(&store, &settings, 0, &output);
            assert!((value(&store, Filter::default()) - 0.7).abs() < 1e-12);
            let dashboard = store.dashboard(&Filter::default()).unwrap();
            assert_eq!(dashboard.summary.total, 120);
            assert_eq!(dashboard.summary.events, 2);
            let raw: String = store
                .db
                .query_row(
                    "SELECT payload FROM events WHERE id=?1",
                    [mirrored.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(!raw.contains("PRIVATE"));
            assert!(
                serde_json::from_str::<UsageEvent>(&raw)
                    .unwrap()
                    .native_credits
                    .unwrap()
                    .mirrored_by
                    .is_some()
            );
            assert!(
                (value(
                    &store,
                    Filter {
                        model: Some("unknown".into()),
                        ..Default::default()
                    }
                ) - 0.2)
                    .abs()
                    < 1e-12
            );
        }
    }
    #[test]
    fn source_views_keep_uncovered_parent_credits_and_membership_changes_invalidate_the_parent_day()
    {
        let (_root, store, settings) = fixture();
        let today = chrono::Local::now().date_naive();
        let yesterday = usage_cache::boundary(today - chrono::Duration::days(1)) + 3600;
        save(&store, &settings, 0, &parent(now() - 10));
        assert_eq!(value(&store, Filter::default()), 0.5);
        let main = child(yesterday);
        save(&store, &settings, 1, &main);
        assert_eq!(value(&store, Filter::default()), 0.0);
        let a = Filter {
            source_id: Some("a".into()),
            ..Default::default()
        };
        assert_eq!(value(&store, a.clone()), 0.5);
        assert_eq!(value(&store, a.clone()), 0.5);
        let all = Filter {
            date_range: Some(DateRange::All),
            ..Default::default()
        };
        assert_eq!(value(&store, all.clone()), 0.5);
        let child = save(&store, &settings, 0, &main);
        assert_eq!(value(&store, a.clone()), 0.0);
        assert_eq!(
            value(
                &store,
                Filter {
                    source_id: Some("a".into()),
                    ..all
                }
            ),
            0.5
        );
        store
            .db
            .execute(
                "UPDATE event_sources SET source_id='c' WHERE event_id=?1 AND source_id='a'",
                [&child.id],
            )
            .unwrap();
        assert_eq!(value(&store, a.clone()), 0.5);
        save(&store, &settings, 0, &main);
        assert_eq!(value(&store, a.clone()), 0.0);
        store
            .db
            .execute(
                "DELETE FROM event_sources WHERE event_id=?1 AND source_id='a'",
                [&child.id],
            )
            .unwrap();
        assert_eq!(value(&store, a), 0.5);
    }
    #[test]
    fn deleting_child_restores_parent_and_rekeying_keeps_the_link_valid() {
        let (root, store, settings) = fixture();
        let stamp = now() - 10;
        save(&store, &settings, 0, &parent(stamp));
        let mut main = usage::parse(
            &settings.sources[0],
            &mut ParseState::default(),
            &child(stamp),
        )
        .unwrap();
        let canonical = main.id.clone();
        main.id = "legacy-child".into();
        store
            .merge_usage_event(main.clone(), "a", &[], &settings)
            .unwrap();
        assert_eq!(value(&store, Filter::default()), 0.5);
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert_eq!(value(&store, Filter::default()), 0.5);
        assert!(
            store
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM native_credit_aliases WHERE event_id=?1)",
                    [&canonical],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
        );
        store
            .db
            .execute("DELETE FROM event_sources WHERE event_id=?1", [&canonical])
            .unwrap();
        store
            .db
            .execute("DELETE FROM events WHERE id=?1", [&canonical])
            .unwrap();
        assert_eq!(value(&store, Filter::default()), 0.5);
        assert_eq!(
            store.dashboard(&Filter::default()).unwrap().summary.total,
            0
        );
    }
}
